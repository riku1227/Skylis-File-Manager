use super::{
    DeleteKind, Prepared, TransferKind, TransferOutcome, confirm_permanent_delete,
    prepare_deletion, report_delete_items, volume,
};
use crate::{
    file_operation_progress::{RecycleGuard, Subscription},
    operation_center::{JobIssue, JobWorker, PermanentDeleteReason},
};
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use windows::{
    Win32::{
        Foundation::{HWND, S_OK},
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            Ole::{OleInitialize, OleUninitialize},
        },
        UI::Shell::*,
    },
    core::{HRESULT, PCWSTR},
};

mod recycle;

struct Apartment;
impl Apartment {
    fn new() -> windows::core::Result<Self> {
        unsafe { OleInitialize(None)? };
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() }
    }
}

fn transfer_flags() -> FILEOPERATION_FLAGS {
    // Silence progress while retaining the Shell's collision and elevation prompts.
    FOFX_ADDUNDORECORD | FOF_NOCONFIRMMKDIR | FOF_SILENT
}

fn delete_flags(kind: DeleteKind) -> FILEOPERATION_FLAGS {
    let silent = FOF_SILENT | FOF_NOERRORUI | FOF_NOCONFIRMATION;
    match kind {
        DeleteKind::Recycle => {
            silent | FOFX_RECYCLEONDELETE | FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOF_WANTNUKEWARNING
        }
        DeleteKind::Permanent => silent,
    }
}

fn shell_item(path: &Path) -> anyhow::Result<IShellItem> {
    let wide = crate::native_shell::windows_path_wide(path)?;
    unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
        .with_context(|| format!("Shellが項目を開けません: {}", path.display()))
}

fn operation(hwnd: isize, flags: FILEOPERATION_FLAGS) -> windows::core::Result<IFileOperation> {
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOwnerWindow(HWND(hwnd as *mut _))?;
        operation.SetOperationFlags(flags)?;
        Ok(operation)
    }
}

pub(super) fn transfer(
    prepared: &Prepared,
    kind: TransferKind,
    worker: &JobWorker,
) -> anyhow::Result<TransferOutcome> {
    let _apartment = Apartment::new()?;
    let destination = shell_item(&prepared.destination)?;
    let mut sources = Vec::with_capacity(prepared.paths.len());
    for path in &prepared.paths {
        if worker.is_cancelled() {
            return Ok(TransferOutcome::Aborted);
        }
        sources.push((
            shell_item(path)?,
            volume::resolve(kind, path, &prepared.destination),
        ));
    }
    if kind == TransferKind::Automatic {
        worker.report_transfer_kind(sources.iter().map(|(_, kind)| *kind));
    }
    let Some(owner) = worker.native_owner()? else {
        return Ok(TransferOutcome::Aborted);
    };
    let operation = operation(owner.hwnd, transfer_flags())?;
    let sink = Subscription::new(&operation, worker, None)?;
    for (source, kind) in &sources {
        unsafe {
            match kind {
                TransferKind::Copy => {
                    operation.CopyItem(source, &destination, PCWSTR::null(), None)?
                }
                TransferKind::Move => {
                    operation.MoveItem(source, &destination, PCWSTR::null(), None)?
                }
                TransferKind::Automatic => unreachable!("volume policy resolves automatic intent"),
            }
        }
    }
    perform(&operation, &sink)
}

pub(super) fn delete(
    paths: &[PathBuf],
    kind: DeleteKind,
    worker: &mut JobWorker,
) -> anyhow::Result<TransferOutcome> {
    match kind {
        DeleteKind::Permanent => confirm_permanent(paths, PermanentDeleteReason::Explicit, worker),
        DeleteKind::Recycle => recycle_silently(paths, worker),
    }
}

fn confirm_permanent(
    paths: &[PathBuf],
    reason: PermanentDeleteReason,
    worker: &mut JobWorker,
) -> anyhow::Result<TransferOutcome> {
    confirm_permanent_delete(paths, reason, worker, |paths, worker| {
        let paths = prepare_deletion(paths)?;
        delete_batch(&paths, DeleteKind::Permanent, worker)
    })
}

fn delete_batch(
    paths: &[PathBuf],
    kind: DeleteKind,
    worker: &JobWorker,
) -> anyhow::Result<TransferOutcome> {
    let _apartment = Apartment::new()?;
    // Validate every Shell item before submitting a destructive batch.
    let items = paths
        .iter()
        .map(|path| shell_item(path))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let Some(owner) = worker.native_owner()? else {
        return Ok(TransferOutcome::Aborted);
    };
    let operation = operation(owner.hwnd, delete_flags(kind))?;
    let sink = Subscription::new(&operation, worker, None)?;
    for item in &items {
        unsafe { operation.DeleteItem(item, None)? };
    }
    perform(&operation, &sink)
}

fn recycle_silently(paths: &[PathBuf], worker: &mut JobWorker) -> anyhow::Result<TransferOutcome> {
    let mut candidates = Vec::new();
    let mut fallback = Vec::new();
    for path in paths {
        if worker.is_cancelled() {
            return Ok(TransferOutcome::Aborted);
        }
        if recycle::cannot_recycle(path) {
            fallback.push(path.clone());
        } else {
            candidates.push(path.as_path());
        }
    }
    let total_items = paths.len();
    let mut recycled_count = 0;
    if !candidates.is_empty() {
        let (outcome, recycled) =
            recycle_candidates(&candidates, total_items, &mut fallback, worker)?;
        recycled_count = recycled;
        if outcome == TransferOutcome::Aborted {
            return Ok(TransferOutcome::Aborted);
        }
    }
    if worker.is_cancelled() {
        return Ok(TransferOutcome::Aborted);
    }
    match confirm_permanent(&fallback, PermanentDeleteReason::RecycleUnavailable, worker)? {
        TransferOutcome::Aborted if !worker.is_cancelled() && recycled_count > 0 => {
            for path in &fallback {
                worker.issue(JobIssue {
                    item: Some(path.to_string_lossy().into_owned()),
                    message: "ゴミ箱へ移動できないため完全削除を見送りました".into(),
                });
            }
            Ok(TransferOutcome::CompletedWithIssues)
        }
        other => Ok(other),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecycleItemOutcome {
    Recycled,
    FallbackRoot,
    RefusedNested,
    Aborted,
    Failed,
}

#[allow(clippy::too_many_arguments)]
fn classify_recycle_item(
    worker_cancelled: bool,
    app_cancelled: bool,
    rejected: bool,
    rejected_root: bool,
    shell_cancelled: bool,
    any_aborted: bool,
    perform_hr: HRESULT,
    finish_hr: HRESULT,
) -> RecycleItemOutcome {
    if worker_cancelled || app_cancelled {
        return RecycleItemOutcome::Aborted;
    }
    if rejected {
        return if rejected_root {
            RecycleItemOutcome::FallbackRoot
        } else {
            RecycleItemOutcome::RefusedNested
        };
    }
    if shell_cancelled
        || any_aborted
        || matches!(
            perform_hr,
            COPYENGINE_E_CANCELLED | COPYENGINE_E_USER_CANCELLED
        )
        || matches!(
            finish_hr,
            COPYENGINE_E_CANCELLED | COPYENGINE_E_USER_CANCELLED
        )
    {
        return RecycleItemOutcome::Aborted;
    }
    if perform_hr.is_err() || finish_hr.is_err() {
        return RecycleItemOutcome::Failed;
    }
    RecycleItemOutcome::Recycled
}

fn recycle_candidates(
    paths: &[&Path],
    total_items: usize,
    fallback: &mut Vec<PathBuf>,
    worker: &JobWorker,
) -> anyhow::Result<(TransferOutcome, usize)> {
    let _apartment = Apartment::new()?;
    let flags = delete_flags(DeleteKind::Recycle);
    // A failed PreDeleteItem can abort all requests in a single IFileOperation.
    // Isolate top-level selections so a refusal cannot suppress later candidates.
    let Some(owner) = worker.native_owner()? else {
        return Ok((TransferOutcome::Aborted, 0));
    };
    let mut recycled_count = 0;
    for (index, path) in paths.iter().enumerate() {
        if worker.is_cancelled() {
            return Ok((TransferOutcome::Aborted, recycled_count));
        }
        report_delete_items(worker, total_items, index, Some(path));
        let result: anyhow::Result<RecycleItemOutcome> = (|| {
            let item = shell_item(path)?;
            let operation = operation(owner.hwnd, flags)?;
            let guard = RecycleGuard::new(item.clone());
            let sink = Subscription::new(&operation, worker, Some(guard.clone()))?;
            unsafe { operation.DeleteItem(&item, None)? };
            let result = unsafe { operation.PerformOperations() };
            let perform_hr = match &result {
                Ok(()) => S_OK,
                Err(error) => error.code(),
            };
            let any_aborted = unsafe { operation.GetAnyOperationsAborted() }
                .is_ok_and(|aborted| aborted.as_bool());
            let outcome = classify_recycle_item(
                worker.is_cancelled(),
                sink.app_cancelled(),
                guard.rejected(),
                guard.rejected_root(),
                sink.shell_cancelled(),
                any_aborted,
                perform_hr,
                sink.finish_hresult(),
            );
            if outcome == RecycleItemOutcome::Failed {
                result.context(
                    "ファイル操作が完了しませんでした。一部の項目は処理済みの可能性があります",
                )?;
                sink.finish_result()
                    .context("Windowsのファイル操作が失敗しました")?;
            }
            Ok(outcome)
        })();
        match result {
            Ok(RecycleItemOutcome::Aborted) => {
                return Ok((TransferOutcome::Aborted, recycled_count));
            }
            Ok(RecycleItemOutcome::FallbackRoot) => fallback.push(path.to_path_buf()),
            Ok(RecycleItemOutcome::RefusedNested) => {}
            Ok(RecycleItemOutcome::Recycled) => recycled_count += 1,
            Ok(RecycleItemOutcome::Failed) => unreachable!("failed outcomes return Err above"),
            Err(error) => worker.issue(JobIssue {
                item: Some(path.to_string_lossy().into_owned()),
                message: format!("{error:#}"),
            }),
        }
        report_delete_items(worker, total_items, index + 1, Some(path));
    }
    Ok((TransferOutcome::Completed, recycled_count))
}

fn perform(operation: &IFileOperation, sink: &Subscription) -> anyhow::Result<TransferOutcome> {
    operation_result(operation, sink, unsafe { operation.PerformOperations() })
}

fn operation_result(
    operation: &IFileOperation,
    sink: &Subscription,
    result: windows::core::Result<()>,
) -> anyhow::Result<TransferOutcome> {
    if sink.user_cancelled()
        || result.as_ref().is_err_and(|err| {
            matches!(
                err.code(),
                COPYENGINE_E_CANCELLED | COPYENGINE_E_USER_CANCELLED
            )
        })
        || unsafe { operation.GetAnyOperationsAborted()?.as_bool() }
    {
        return Ok(TransferOutcome::Aborted);
    }
    result.context("ファイル操作が完了しませんでした。一部の項目は処理済みの可能性があります")?;
    sink.finish_result()
        .context("Windowsのファイル操作が失敗しました")?;
    Ok(TransferOutcome::Completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrated_transfer_is_silent_and_preserves_collision_confirmation() {
        assert_eq!(
            transfer_flags(),
            FOF_SILENT | FOFX_ADDUNDORECORD | FOF_NOCONFIRMMKDIR
        );
        assert_eq!(transfer_flags().0 & FOF_NOCONFIRMATION.0, 0);
    }

    #[test]
    fn integrated_delete_preserves_the_recycle_guard_flags() {
        let recycle =
            FOFX_RECYCLEONDELETE | FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOF_WANTNUKEWARNING;
        let silent = FOF_SILENT | FOF_NOERRORUI | FOF_NOCONFIRMATION;
        assert_eq!(delete_flags(DeleteKind::Recycle), recycle | silent);
        assert_eq!(delete_flags(DeleteKind::Permanent), silent);
    }

    #[test]
    fn recycle_outcome_classification_preserves_refusals_over_collateral_cancellations() {
        for hr in [
            COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
            COPYENGINE_E_CANCELLED,
            COPYENGINE_E_USER_CANCELLED,
        ] {
            assert_eq!(
                classify_recycle_item(false, false, true, true, true, true, hr, hr),
                RecycleItemOutcome::FallbackRoot
            );
            assert_eq!(
                classify_recycle_item(false, false, true, false, true, true, hr, hr),
                RecycleItemOutcome::RefusedNested
            );
        }

        assert_eq!(
            classify_recycle_item(
                true,
                false,
                true,
                true,
                false,
                false,
                COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
                S_OK
            ),
            RecycleItemOutcome::Aborted
        );
        assert_eq!(
            classify_recycle_item(
                false,
                true,
                true,
                true,
                false,
                false,
                COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
                S_OK
            ),
            RecycleItemOutcome::Aborted
        );

        assert_eq!(
            classify_recycle_item(false, false, false, false, false, true, S_OK, S_OK),
            RecycleItemOutcome::Aborted
        );
        assert_eq!(
            classify_recycle_item(false, false, false, false, true, false, S_OK, S_OK),
            RecycleItemOutcome::Aborted
        );
        assert_eq!(
            classify_recycle_item(
                false,
                false,
                false,
                false,
                false,
                false,
                COPYENGINE_E_USER_CANCELLED,
                S_OK
            ),
            RecycleItemOutcome::Aborted
        );
        assert_eq!(
            classify_recycle_item(
                false,
                false,
                false,
                false,
                false,
                false,
                S_OK,
                COPYENGINE_E_CANCELLED
            ),
            RecycleItemOutcome::Aborted
        );

        let generic_error = HRESULT(0x8007_0005u32 as i32);
        assert_eq!(
            classify_recycle_item(
                false,
                false,
                false,
                false,
                false,
                false,
                generic_error,
                S_OK
            ),
            RecycleItemOutcome::Failed
        );
        assert_eq!(
            classify_recycle_item(
                false,
                false,
                false,
                false,
                false,
                false,
                S_OK,
                generic_error
            ),
            RecycleItemOutcome::Failed
        );

        assert_eq!(
            classify_recycle_item(false, false, false, false, false, false, S_OK, S_OK),
            RecycleItemOutcome::Recycled
        );
    }
}

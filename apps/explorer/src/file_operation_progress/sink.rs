use crate::{
    operation_center::{JobIssue, JobProgress, JobWorker, ProgressMeasure},
    settings_data::FileTransferEngine,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::S_OK,
        System::Com::CoTaskMemFree,
        UI::Shell::{
            COPYENGINE_E_CANCELLED, COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
            COPYENGINE_E_USER_CANCELLED, COPYENGINE_S_NOT_HANDLED, COPYENGINE_S_USER_IGNORED,
            IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl,
            IShellItem, SICHINT_CANONICAL, SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL,
            SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY, TSF_DELETE_RECYCLE_IF_POSSIBLE,
        },
    },
    core::{HRESULT, PCWSTR, Ref, Result, implement},
};

/// Advise and Unadvise live in the same STA as the native operation. No COM pointer escapes.
pub(crate) struct Subscription {
    operation: IFileOperation,
    cookie: u32,
    _sink: IFileOperationProgressSink,
    finish: Rc<Cell<HRESULT>>,
    app_cancelled: Rc<Cell<bool>>,
    shell_cancelled: Rc<Cell<bool>>,
}
impl Subscription {
    pub(crate) fn new(
        operation: &IFileOperation,
        worker: &JobWorker,
        engine: FileTransferEngine,
        recycle: Option<RecycleGuard>,
    ) -> Result<Self> {
        let finish = Rc::new(Cell::new(HRESULT(0)));
        let app_cancelled = Rc::new(Cell::new(false));
        let shell_cancelled = Rc::new(Cell::new(false));
        let sink: IFileOperationProgressSink = Sink {
            worker: worker.clone(),
            cancellable: engine == FileTransferEngine::OsIntegrated,
            recycle,
            finish: finish.clone(),
            app_cancelled: app_cancelled.clone(),
            shell_cancelled: shell_cancelled.clone(),
            progress: RefCell::new(JobProgress::default()),
        }
        .into();
        let cookie = unsafe { operation.Advise(&sink)? };
        Ok(Self {
            operation: operation.clone(),
            cookie,
            _sink: sink,
            finish,
            app_cancelled,
            shell_cancelled,
        })
    }

    pub(crate) fn app_cancelled(&self) -> bool {
        self.app_cancelled.get()
    }

    pub(crate) fn shell_cancelled(&self) -> bool {
        self.shell_cancelled.get()
    }

    pub(crate) fn finish_hresult(&self) -> HRESULT {
        self.finish.get()
    }

    pub(crate) fn user_cancelled(&self) -> bool {
        self.app_cancelled.get()
            || self.shell_cancelled.get()
            || matches!(
                self.finish.get(),
                COPYENGINE_E_CANCELLED | COPYENGINE_E_USER_CANCELLED
            )
    }

    pub(crate) fn finish_result(&self) -> Result<()> {
        self.finish.get().ok()
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        unsafe {
            let _ = self.operation.Unadvise(self.cookie);
        }
    }
}

/// A silent recycle call handles one selected root. Keep the refused Shell item
/// for PostDelete matching, and only offer fallback when the root itself was refused.
#[derive(Clone)]
pub(crate) struct RecycleGuard {
    root: IShellItem,
    state: Rc<RefCell<RecycleObservation>>,
}

#[derive(Default)]
struct RecycleObservation {
    rejected: Option<RejectedItem>,
    root_succeeded: bool,
}

struct RejectedItem {
    item: Option<IShellItem>,
    #[cfg_attr(not(test), allow(dead_code))]
    path: Option<String>,
    root: bool,
}

impl RecycleObservation {
    fn pre_delete(&mut self, flags: u32, rejected: impl FnOnce() -> RejectedItem) -> Result<()> {
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 != 0 {
            return Ok(());
        }
        if self.rejected.is_none() {
            self.rejected = Some(rejected());
        }
        Err(COPYENGINE_E_RECYCLE_BIN_NOT_FOUND.into())
    }

    fn post_delete(
        &mut self,
        result: HRESULT,
        newly_created: bool,
        root: bool,
    ) -> Option<&'static str> {
        if result == S_OK {
            if root {
                self.root_succeeded = true;
            }
            return (!newly_created).then_some("ゴミ箱に入らなかった可能性があります");
        }
        None
    }
}

impl RecycleGuard {
    pub(crate) fn new(root: IShellItem) -> Self {
        Self {
            root,
            state: Default::default(),
        }
    }

    pub(crate) fn rejected(&self) -> bool {
        self.state.borrow().rejected.is_some()
    }

    pub(crate) fn rejected_root(&self) -> bool {
        let state = self.state.borrow();
        !state.root_succeeded
            && state
                .rejected
                .as_ref()
                .is_some_and(|rejected| rejected.root)
    }
}

fn same_item(left: &IShellItem, right: &IShellItem) -> bool {
    left == right
        || unsafe {
            left.Compare(
                right,
                (SICHINT_CANONICAL.0 | SICHINT_TEST_FILESYSPATH_IF_NOT_EQUAL.0) as u32,
            )
        }
        .is_ok_and(|order| order == 0)
}

fn item_name(item: &IShellItem, filesystem_only: bool) -> Option<String> {
    // Shell and its allocated display name stay in the worker apartment.
    unsafe {
        let name = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .or_else(|error| {
                if filesystem_only {
                    Err(error)
                } else {
                    item.GetDisplayName(SIGDN_NORMALDISPLAY)
                }
            })
            .ok()?;
        let value = name.to_string().ok();
        CoTaskMemFree(Some(name.0.cast()));
        value.map(|mut name| {
            name.truncate(name.floor_char_boundary(32 * 1024));
            name
        })
    }
}

#[implement(IFileOperationProgressSink)]
struct Sink {
    worker: JobWorker,
    cancellable: bool,
    recycle: Option<RecycleGuard>,
    finish: Rc<Cell<HRESULT>>,
    app_cancelled: Rc<Cell<bool>>,
    shell_cancelled: Rc<Cell<bool>>,
    progress: RefCell<JobProgress>,
}

impl Sink_Impl {
    fn check_cancel(&self) -> Result<()> {
        if self.cancellable && self.worker.is_cancelled() {
            self.app_cancelled.set(true);
            return Err(COPYENGINE_E_USER_CANCELLED.into());
        }
        Ok(())
    }

    fn current(&self, item: Ref<'_, IShellItem>) {
        if self.recycle.is_none() {
            let name = item.as_ref().and_then(|item| item_name(item, false));
            let mut progress = self.progress.borrow_mut();
            progress.current_item = name;
            self.worker.report(progress.clone());
        }
    }

    fn item_result(&self, item: Option<&IShellItem>, result: HRESULT) {
        if matches!(result, COPYENGINE_E_CANCELLED | COPYENGINE_E_USER_CANCELLED) {
            self.shell_cancelled.set(true);
        } else if result.is_err()
            || matches!(result, COPYENGINE_S_USER_IGNORED | COPYENGINE_S_NOT_HANDLED)
        {
            let message = if result.is_err() {
                format!("Windowsのファイル操作に失敗しました ({result:?})")
            } else {
                "項目は処理されませんでした".into()
            };
            self.issue(item.and_then(|item| item_name(item, false)), message);
        }
    }

    fn issue(&self, item: Option<String>, message: String) {
        self.worker.issue(JobIssue { item, message });
    }
}
#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> Result<()> {
        self.check_cancel()
    }
    fn FinishOperations(&self, result: HRESULT) -> Result<()> {
        self.finish.set(result);
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, item: Ref<'_, IShellItem>, _: &PCWSTR) -> Result<()> {
        self.check_cancel()?;
        self.current(item);
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        self.item_result(item.as_ref(), result);
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> Result<()> {
        self.check_cancel()?;
        self.current(item);
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        self.item_result(item.as_ref(), result);
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> Result<()> {
        self.check_cancel()?;
        self.current(item);
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        self.item_result(item.as_ref(), result);
        Ok(())
    }
    fn PreDeleteItem(&self, flags: u32, item: Ref<'_, IShellItem>) -> Result<()> {
        self.check_cancel()?;
        if let Some(guard) = &self.recycle
            && flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0
        {
            let item = item.as_ref();
            let root = item.is_some_and(|item| same_item(item, &guard.root));
            let path = item.and_then(|item| item_name(item, true));
            let result = guard.state.borrow_mut().pre_delete(flags, || RejectedItem {
                item: item.cloned(),
                path: path.clone(),
                root,
            });
            if !root {
                self.issue(
                    path,
                    "ゴミ箱へ移動できない項目の削除を停止しました（親フォルダーは残っています）"
                        .into(),
                );
            }
            return result;
        }
        self.current(item);
        Ok(())
    }

    fn PostDeleteItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        result: HRESULT,
        newly_created: Ref<'_, IShellItem>,
    ) -> Result<()> {
        let item = item.as_ref();
        if let Some(guard) = &self.recycle {
            let any_rejected = guard.rejected();
            let refused_item = guard
                .state
                .borrow()
                .rejected
                .as_ref()
                .and_then(|rejected| rejected.item.clone());
            let refused = item
                .zip(refused_item.as_ref())
                .is_some_and(|(left, right)| same_item(left, right));
            let root = item.is_some_and(|item| same_item(item, &guard.root));
            if !refused {
                let warning = guard.state.borrow_mut().post_delete(
                    result,
                    newly_created.as_ref().is_some(),
                    root,
                );
                if let Some(warning) = warning {
                    self.issue(item.and_then(|item| item_name(item, true)), warning.into());
                }
            }
            if refused
                || (any_rejected
                    && matches!(
                        result,
                        COPYENGINE_E_CANCELLED
                            | COPYENGINE_E_USER_CANCELLED
                            | COPYENGINE_E_RECYCLE_BIN_NOT_FOUND
                    ))
            {
                return Ok(());
            }
        }
        self.item_result(item, result);
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        _: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, total: u32, done: u32) -> Result<()> {
        self.check_cancel()?;
        if self.recycle.is_none() {
            let mut progress = self.progress.borrow_mut();
            progress.measure = ProgressMeasure::Relative {
                work_done: done.min(total),
                work_total: total,
            };
            self.worker.report(progress.clone());
        }
        Ok(())
    }
    fn ResetTimer(&self) -> Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_delete_refuses_an_item_without_the_recycle_flag_and_records_it() {
        let mut state = RecycleObservation::default();
        state.pre_delete(0x80, || unreachable!()).unwrap();
        assert!(state.rejected.is_none());
        let error = state
            .pre_delete(0x40, || RejectedItem {
                item: None,
                path: Some(r"C:\example".into()),
                root: true,
            })
            .unwrap_err();
        assert_eq!(error.code(), COPYENGINE_E_RECYCLE_BIN_NOT_FOUND);
        let rejected = state.rejected.unwrap();
        assert_eq!(rejected.path.as_deref(), Some(r"C:\example"));
        assert!(rejected.root);
    }

    #[test]
    fn post_delete_warns_only_on_s_ok_without_a_recycled_item() {
        let mut state = RecycleObservation::default();
        for code in [
            COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
            COPYENGINE_S_USER_IGNORED,
            COPYENGINE_S_NOT_HANDLED,
        ] {
            assert!(state.post_delete(code, false, true).is_none());
            assert!(!state.root_succeeded);
        }
        assert!(state.post_delete(S_OK, false, true).is_some());
        assert!(state.root_succeeded);
        assert!(state.post_delete(S_OK, true, true).is_none());
    }
}

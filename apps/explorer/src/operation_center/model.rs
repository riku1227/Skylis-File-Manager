use crate::{file_operations::TransferKind, settings_data::FileTransferEngine};
use explorer_core::archive::{ArchiveFormat, ArchiveProgress};
use gpui::AnyWindowHandle;
use std::{path::PathBuf, time::Instant};

const ISSUE_LIMIT: usize = 256;
pub(super) const ISSUE_BYTES_LIMIT: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JobId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PromptId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobKind {
    Copy,
    Move,
    CopyOrMove,
    Trash,
    PermanentDelete,
    ArchiveCreate(ArchiveFormat),
    ArchiveExtract,
}

impl From<TransferKind> for JobKind {
    fn from(value: TransferKind) -> Self {
        match value {
            TransferKind::Copy => Self::Copy,
            TransferKind::Move => Self::Move,
            TransferKind::Automatic => Self::CopyOrMove,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobBackend {
    Builtin,
    OsDelegated { reason: FileTransferEngine },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Pausing and per-item issue results are produced by later I/O backends.
pub(crate) enum JobStatus {
    Queued,
    Running,
    Paused,
    WaitingForUser(PromptId),
    Cancelling,
    Completed,
    CompletedWithIssues,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Running
                | Self::Paused
                | Self::WaitingForUser(_)
                | Self::Cancelling
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // The archive backend exposes only optional known totals.
pub(crate) enum TotalState {
    Counting { lower_bound: u64 },
    Known(u64),
    Unknown,
}

impl TotalState {
    fn normalized(self, previous: Self, done: u64) -> Self {
        match (self, previous) {
            (Self::Known(total), Self::Known(old)) => Self::Known(total.max(old).max(done)),
            (_, Self::Known(old)) => Self::Known(old.max(done)),
            (Self::Known(total), Self::Counting { lower_bound }) => {
                Self::Known(total.max(lower_bound).max(done))
            }
            (Self::Known(total), _) => Self::Known(total.max(done)),
            (Self::Counting { lower_bound }, Self::Counting { lower_bound: old }) => {
                Self::Counting {
                    lower_bound: lower_bound.max(old).max(done),
                }
            }
            (Self::Counting { lower_bound }, _) => Self::Counting {
                lower_bound: lower_bound.max(done),
            },
            (Self::Unknown, Self::Counting { lower_bound }) => Self::Counting {
                lower_bound: lower_bound.max(done),
            },
            (Self::Unknown, _) => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ProgressMeasure {
    Measured {
        bytes_done: u64,
        bytes_total: TotalState,
        items_done: u64,
        items_total: TotalState,
        bytes_per_sec: Option<u64>,
        eta_seconds: Option<u64>,
    },
    Relative {
        work_done: u32,
        work_total: u32,
    },
    #[default]
    Indeterminate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct JobProgress {
    pub measure: ProgressMeasure,
    pub current_item: Option<String>,
    pub hydrating_cloud_likely: bool,
}

impl From<ArchiveProgress> for JobProgress {
    fn from(value: ArchiveProgress) -> Self {
        Self {
            measure: ProgressMeasure::Measured {
                bytes_done: value.processed_bytes,
                bytes_total: value
                    .total_bytes
                    .map_or(TotalState::Unknown, TotalState::Known),
                items_done: value.entries_done as u64,
                items_total: TotalState::Unknown,
                bytes_per_sec: None,
                eta_seconds: None,
            },
            current_item: value
                .current_source
                .map(|path| path.to_string_lossy().into_owned())
                .or_else(|| Some(value.current_name).filter(|name| !name.is_empty())),
            hydrating_cloud_likely: false,
        }
    }
}

impl JobProgress {
    /// Backends include skipped work in done. Normalize against the previous sample,
    /// including samples coalesced before the UI has seen them.
    pub(super) fn normalized(mut self, previous: &Self) -> Self {
        if self.measure == ProgressMeasure::Indeterminate {
            self.measure = previous.measure.clone();
        }
        if let ProgressMeasure::Measured {
            bytes_done,
            bytes_total,
            items_done,
            items_total,
            eta_seconds,
            ..
        } = &mut self.measure
        {
            let (old_bytes, old_bytes_total, old_items, old_items_total) = match previous.measure {
                ProgressMeasure::Measured {
                    bytes_done,
                    bytes_total,
                    items_done,
                    items_total,
                    ..
                } => (bytes_done, bytes_total, items_done, items_total),
                _ => (0, TotalState::Unknown, 0, TotalState::Unknown),
            };
            *bytes_done = (*bytes_done).max(old_bytes);
            *items_done = (*items_done).max(old_items);
            *bytes_total = bytes_total.normalized(old_bytes_total, *bytes_done);
            *items_total = items_total.normalized(old_items_total, *items_done);
            if self.hydrating_cloud_likely || !matches!(bytes_total, TotalState::Known(_)) {
                *eta_seconds = None;
            }
        }
        self
    }

    fn percent(&self) -> Option<u8> {
        fn fraction(done: u64, total: u64) -> Option<u8> {
            (total != 0).then(|| (u128::from(done) * 100 / u128::from(total)).min(99) as u8)
        }
        match self.measure {
            ProgressMeasure::Measured {
                bytes_done,
                bytes_total: TotalState::Known(total),
                ..
            } if total > 0 => fraction(bytes_done, total),
            ProgressMeasure::Measured {
                items_done,
                items_total: TotalState::Known(total),
                ..
            } => fraction(items_done, total),
            ProgressMeasure::Relative {
                work_done,
                work_total,
            } => fraction(work_done.into(), work_total.into()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct JobIssue {
    pub item: Option<String>,
    pub message: String,
}

pub(crate) struct JobSnapshot {
    pub id: JobId,
    pub kind: JobKind,
    pub backend: JobBackend,
    pub destination: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub origin: Option<AnyWindowHandle>,
    pub submitted_at: Instant,
    pub status: JobStatus,
    pub progress: JobProgress,
    pub percent: Option<u8>,
    pub cancellable: bool,
    pub unread_error: bool,
    pub issue_count: usize,
    issues: Vec<JobIssue>,
    issue_bytes: usize,
}

impl JobSnapshot {
    pub(super) fn new(
        id: JobId,
        kind: JobKind,
        backend: JobBackend,
        destination: Option<PathBuf>,
        origin: AnyWindowHandle,
        cancellable: bool,
        submitted_at: Instant,
    ) -> Self {
        Self {
            id,
            kind,
            backend,
            destination,
            output: None,
            origin: Some(origin),
            submitted_at,
            status: JobStatus::Queued,
            progress: JobProgress::default(),
            percent: None,
            cancellable,
            unread_error: false,
            issue_count: 0,
            issues: Vec::new(),
            issue_bytes: 0,
        }
    }

    pub(super) fn update(&mut self, progress: JobProgress) {
        self.progress = progress.normalized(&self.progress);
        self.percent = self.percent.max(self.progress.percent());
    }

    pub(super) fn finish(&mut self, status: JobStatus, error: Option<String>) {
        self.status = status;
        self.cancellable = false;
        if let Some(message) = error {
            self.add_issue(JobIssue {
                item: None,
                message,
            });
        }
        if matches!(
            status,
            JobStatus::Completed | JobStatus::CompletedWithIssues
        ) && self.percent.is_some()
        {
            self.percent = Some(100);
        }
        self.unread_error = matches!(status, JobStatus::Failed | JobStatus::CompletedWithIssues);
    }

    pub fn issues(&self) -> &[JobIssue] {
        &self.issues
    }

    pub(super) fn add_issue(&mut self, mut issue: JobIssue) {
        self.issue_count = self.issue_count.saturating_add(1);
        if self.issues.len() >= ISSUE_LIMIT || self.issue_bytes >= ISSUE_BYTES_LIMIT {
            return;
        }
        let remaining = ISSUE_BYTES_LIMIT - self.issue_bytes;
        let item_bytes = issue.item.as_ref().map_or(0, String::len);
        let total_bytes = item_bytes.saturating_add(issue.message.len());
        if total_bytes > remaining {
            let message_budget = remaining.min(issue.message.len()).min(256);
            if let Some(item) = &mut issue.item {
                let item_budget = remaining.saturating_sub(message_budget);
                truncate_with_ellipsis(item, item_budget);
                if item.is_empty() {
                    issue.item = None;
                }
            }
            let used_by_item = issue.item.as_ref().map_or(0, String::len);
            let message_budget = remaining.saturating_sub(used_by_item);
            truncate_with_ellipsis(&mut issue.message, message_budget);
            if issue.message.is_empty() && issue.item.is_none() {
                return;
            }
        }
        let bytes = issue
            .message
            .len()
            .saturating_add(issue.item.as_ref().map_or(0, String::len));
        self.issues.push(issue);
        self.issue_bytes += bytes;
    }
}

fn truncate_with_ellipsis(text: &mut String, budget: usize) {
    if text.len() <= budget {
        return;
    }
    if budget == 0 {
        text.clear();
        return;
    }
    if budget <= "…".len() {
        text.truncate(text.floor_char_boundary(budget));
        return;
    }
    text.truncate(text.floor_char_boundary(budget - "…".len()));
    text.push('…');
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Native Phase 0 operations continue using their existing OS prompts.
pub(crate) enum PromptKind {
    ConfirmPermanentDelete {
        representative: PathBuf,
        items: u64,
    },
    FileConflict {
        source: PathBuf,
        destination: PathBuf,
        allow_replace: bool,
        allow_apply_to_all: bool,
    },
    ItemError {
        item: PathBuf,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PromptRequest {
    pub job: JobId,
    pub id: PromptId,
    pub kind: PromptKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptAction {
    Cancel,
    Confirm,
    Skip,
    Replace,
    KeepBoth,
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PromptResponse {
    pub job: JobId,
    pub prompt: PromptId,
    pub action: PromptAction,
    pub apply_to_all: bool,
}

impl PromptKind {
    pub fn actions(&self) -> &'static [PromptAction] {
        use PromptAction::*;
        match self {
            Self::ConfirmPermanentDelete { .. } => &[Cancel, Confirm],
            Self::FileConflict {
                allow_replace: true,
                ..
            } => &[Skip, KeepBoth, Replace, Cancel],
            Self::FileConflict { .. } => &[Skip, KeepBoth, Cancel],
            Self::ItemError { .. } => &[Skip, Retry, Cancel],
        }
    }

    pub fn allow_apply_to_all(&self) -> bool {
        matches!(
            self,
            Self::FileConflict {
                allow_apply_to_all: true,
                ..
            }
        )
    }

    pub(super) fn accepts(&self, action: PromptAction, apply_to_all: bool) -> bool {
        self.actions().contains(&action)
            && (!apply_to_all || (action != PromptAction::Cancel && self.allow_apply_to_all()))
    }
}

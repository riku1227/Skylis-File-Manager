//! Opt-in submission accounting without a clock, sample history, or frame demand.
use crate::EntityId;

pub(super) struct FrameDiagnostics {
    pub view: EntityId,
    pub submissions: u64,
    // None: no attributed draw since the last present. False: HUD-only.
    // True: application work, which a later HUD/clean draw must not erase.
    pending_application: Option<bool>,
}

impl FrameDiagnostics {
    pub fn new(view: EntityId) -> Self {
        Self {
            view,
            submissions: 0,
            pending_application: None,
        }
    }

    pub fn record_draw(&mut self, diagnostic_only: bool, application_demand: bool) {
        if application_demand {
            self.record_application_demand();
        } else if diagnostic_only {
            self.pending_application.get_or_insert(false);
        }
        // A clean synchronous draw only rebuilds hit-testing state; it cannot
        // turn an already pending HUD-only submission into application work.
    }

    pub fn record_application_demand(&mut self) {
        self.pending_application = Some(true);
    }

    pub fn record_present(&mut self) {
        // No attributed draw means a native maintenance/OS submission. These
        // are real submissions even though the scene did not need rebuilding.
        if self.pending_application.take() != Some(false) {
            self.submissions = self.submissions.saturating_add(1);
        }
    }
}

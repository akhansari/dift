use crate::state::State;
use std::time::{Duration, Instant};

const ACTIVITY_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Activity {
    Refreshing,
    Rendering,
    Snapshot,
    Polling,
    Watching,
}

impl Activity {
    pub(super) fn current(state: &State, watching: bool, degraded: bool) -> Self {
        if state.collecting() {
            Self::Refreshing
        } else if state.rendering() {
            Self::Rendering
        } else if !watching {
            Self::Snapshot
        } else if degraded {
            Self::Polling
        } else {
            Self::Watching
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Refreshing => "Refreshing",
            Self::Rendering => "Rendering",
            Self::Snapshot => "Snapshot",
            Self::Polling => "Polling",
            Self::Watching => "Watching",
        }
    }
}

/// Show the first activity immediately, then suppress short-lived changes.
pub(super) struct ActivityStatus {
    displayed: Activity,
    pending: Option<(Activity, Instant)>,
}

impl ActivityStatus {
    pub(super) fn new(initial: Activity) -> Self {
        Self {
            displayed: initial,
            pending: None,
        }
    }

    pub(super) fn displayed(&self) -> Activity {
        self.displayed
    }

    /// Observe the current activity and report whether the label needs redrawing.
    pub(super) fn update(&mut self, current: Activity, now: Instant) -> bool {
        if current == self.displayed {
            self.pending = None;
            false
        } else {
            match self.pending {
                Some((pending, deadline)) if pending == current => {
                    if now >= deadline {
                        self.displayed = current;
                        self.pending = None;
                        true
                    } else {
                        false
                    }
                }
                _ => {
                    self.pending = Some((current, now + ACTIVITY_DELAY));
                    false
                }
            }
        }
    }

    pub(super) fn delay(&self, now: Instant) -> Option<Duration> {
        self.pending
            .map(|(_, deadline)| deadline.saturating_duration_since(now))
    }
}

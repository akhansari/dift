use super::{Effects, State};
use std::time::{Duration, Instant};

const TOAST_DURATION: Duration = Duration::from_secs(3);

pub(super) struct Toast {
    message: String,
    expires_at: Instant,
}

impl State {
    pub fn toast(&self) -> Option<&str> {
        self.toast.as_ref().map(|toast| toast.message.as_str())
    }

    pub(crate) fn show_toast(&mut self, message: String, now: Instant) {
        self.toast = Some(Toast {
            message,
            expires_at: now + TOAST_DURATION,
        });
    }

    pub fn toast_delay(&self, now: Instant) -> Option<Duration> {
        self.toast
            .as_ref()
            .map(|toast| toast.expires_at.saturating_duration_since(now))
    }

    /// Clear an expired notification without recollecting or rendering the diff.
    pub fn expire_toast(&mut self, now: Instant) -> Effects {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| now >= toast.expires_at)
        {
            self.toast = None;
            Effects::draw(false)
        } else {
            Effects::default()
        }
    }
}

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// A token that detects when a render request has been superseded.
#[derive(Debug, Clone)]
pub struct Cancellation {
    latest: Arc<AtomicU64>,
    generation: u64,
}

/// An error indicating that rendering was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Rendering was superseded by a newer request")
    }
}
impl std::error::Error for Cancelled {}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new(Arc::new(AtomicU64::new(0)), 0)
    }
}
impl Cancellation {
    pub fn new(latest: Arc<AtomicU64>, generation: u64) -> Self {
        Self { latest, generation }
    }
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.latest.load(Ordering::Relaxed) == self.generation {
            Ok(())
        } else {
            Err(Cancelled)
        }
    }
}

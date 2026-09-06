use crate::git::Repository;
use notify::{
    RecommendedWatcher, RecursiveMode, Watcher,
    event::{AccessKind, AccessMode, EventKind},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Coalesces requests without losing changes that arrive during collection.
pub(super) struct Refresh {
    last_scan: Instant,
    event_seen: Option<Instant>,
    event_burst: Option<Instant>,
    pending: bool,
}

impl Refresh {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            last_scan: now,
            event_seen: None,
            event_burst: None,
            pending: false,
        }
    }

    pub(super) fn observe(
        &mut self,
        now: Instant,
        dirty: bool,
        watching: bool,
        interval: Duration,
    ) {
        if dirty {
            self.event_seen = Some(now);
            self.event_burst.get_or_insert(now);
        }
        if watching
            && (self
                .event_seen
                .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(150))
                || self
                    .event_burst
                    .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(500))
                || now.duration_since(self.last_scan) >= interval)
        {
            self.request();
            self.event_seen = None;
            self.event_burst = None;
            self.last_scan = now;
        }
    }

    pub(super) fn request(&mut self) {
        self.pending = true;
    }

    pub(super) fn completed(&mut self, now: Instant) {
        self.last_scan = now;
    }

    pub(super) fn take_request(&mut self, collecting: bool) -> bool {
        !collecting && std::mem::take(&mut self.pending)
    }
}

fn relevant(event: &notify::Event, git_dir: &Path, common_dir: &Path) -> bool {
    // A rescan flag means any path may have changed, regardless of the event's
    // kind or paths. Close-after-write also catches mmap saves on Linux.
    if event.need_rescan() {
        return true;
    }
    if event.kind.is_access()
        && event.kind != EventKind::Access(AccessKind::Close(AccessMode::Write))
    {
        return false;
    }
    event.paths.is_empty()
        || event.paths.iter().any(|path| {
            let metadata = path
                .strip_prefix(git_dir)
                .ok()
                .or_else(|| path.strip_prefix(common_dir).ok());
            metadata.is_none_or(|p| {
                p.components().next().is_none_or(|part| {
                    matches!(
                        part.as_os_str().to_str(),
                        Some(
                            "HEAD"
                                | "index"
                                | "config"
                                | "config.worktree"
                                | "packed-refs"
                                | "refs"
                                | "reftable"
                                | "info"
                                | "MERGE_HEAD"
                                | "modules"
                        )
                    )
                })
            })
        })
}

pub(super) fn setup(
    repo: &Repository,
    dirty: Arc<AtomicBool>,
    degraded: Arc<AtomicBool>,
) -> Option<RecommendedWatcher> {
    let callback_degraded = Arc::clone(&degraded);
    let git_dir = repo.git_dir.clone();
    let common_dir = repo.common_dir.clone();
    let callback = move |result: notify::Result<notify::Event>| match result {
        Ok(event) if relevant(&event, &git_dir, &common_dir) => {
            dirty.store(true, Ordering::Relaxed);
        }
        Err(_) => {
            callback_degraded.store(true, Ordering::Relaxed);
            dirty.store(true, Ordering::Relaxed);
        }
        _ => {}
    };
    let result = notify::recommended_watcher(callback).and_then(|mut watcher| {
        watcher.watch(&repo.root, RecursiveMode::Recursive)?;
        if !repo.git_dir.starts_with(&repo.root) {
            watcher.watch(&repo.git_dir, RecursiveMode::Recursive)?;
        }
        if !repo.common_dir.starts_with(&repo.root) && !repo.common_dir.starts_with(&repo.git_dir) {
            watcher.watch(&repo.common_dir, RecursiveMode::Recursive)?;
        }
        Ok(watcher)
    });
    match result {
        Ok(watcher) => Some(watcher),
        Err(_) => {
            degraded.store(true, Ordering::Relaxed);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{Flag, ModifyKind};

    fn is_relevant(event: notify::Event) -> bool {
        relevant(&event, Path::new("repo/.git"), Path::new("repo/.git"))
    }

    #[test]
    fn closing_a_written_file_requests_a_refresh() {
        // Given
        let event = notify::Event::new(EventKind::Access(AccessKind::Close(AccessMode::Write)))
            .add_path("repo/file.txt".into());
        // When
        let refresh = is_relevant(event);
        // Then
        assert!(refresh);
    }

    #[test]
    fn reading_a_file_does_not_request_a_refresh() {
        // Given
        let event = notify::Event::new(EventKind::Access(AccessKind::Close(AccessMode::Read)))
            .add_path("repo/file.txt".into());
        // When
        let refresh = is_relevant(event);
        // Then
        assert!(!refresh);
    }

    #[test]
    fn rescan_overrides_both_access_and_metadata_filters() {
        // Given
        let event = notify::Event::new(EventKind::Access(AccessKind::Any))
            .add_path("repo/.git/objects/ignored".into())
            .set_flag(Flag::Rescan);
        // When
        let refresh = is_relevant(event);
        // Then
        assert!(refresh);
    }

    #[test]
    fn nested_submodule_metadata_requests_a_refresh() {
        // Given
        let event = notify::Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path("repo/.git/modules/outer/modules/inner/HEAD".into());
        // When
        let refresh = is_relevant(event);
        // Then
        assert!(refresh);
    }

    #[test]
    fn linked_worktree_observes_submodule_changes_in_the_common_directory() {
        // Given
        let event = notify::Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path("repo/.git/modules/module/HEAD".into());
        // When
        let refresh = relevant(
            &event,
            Path::new("repo/.git/worktrees/linked"),
            Path::new("repo/.git"),
        );
        // Then
        assert!(refresh);
    }

    #[test]
    fn object_database_writes_do_not_request_a_refresh() {
        // Given
        let event = notify::Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path("repo/.git/objects/ab/cd".into());
        // When
        let refresh = is_relevant(event);
        // Then
        assert!(!refresh);
    }

    #[test]
    fn a_change_during_collection_is_scanned_after_completion() {
        // Given
        let start = Instant::now();
        let mut refresh = Refresh::new(start);
        refresh.observe(start, true, true, Duration::from_secs(30));
        refresh.observe(
            start + Duration::from_millis(150),
            false,
            true,
            Duration::from_secs(30),
        );
        let during_collection = refresh.take_request(true);
        refresh.completed(start + Duration::from_millis(200));
        // When
        let after_collection = refresh.take_request(false);
        // Then
        assert!(!during_collection);
        assert!(after_collection);
        assert!(!refresh.take_request(false));
    }

    #[test]
    fn recent_events_wait_for_the_debounce_delay() {
        // Given
        let start = Instant::now();
        let mut refresh = Refresh::new(start);
        refresh.observe(start, true, true, Duration::from_secs(30));
        // When
        refresh.observe(
            start + Duration::from_millis(149),
            false,
            true,
            Duration::from_secs(30),
        );
        // Then
        assert!(!refresh.take_request(false));
    }

    #[test]
    fn continuous_events_are_collected_after_half_a_second() {
        // Given
        let start = Instant::now();
        let mut refresh = Refresh::new(start);
        refresh.observe(start, true, true, Duration::from_secs(30));
        // When
        refresh.observe(
            start + Duration::from_millis(500),
            true,
            true,
            Duration::from_secs(30),
        );
        // Then
        assert!(refresh.take_request(false));
    }

    #[test]
    fn a_periodic_scan_recovers_changes_without_events() {
        // Given
        let start = Instant::now();
        let mut refresh = Refresh::new(start);
        // When
        refresh.observe(
            start + Duration::from_secs(30),
            false,
            true,
            Duration::from_secs(30),
        );
        // Then
        assert!(refresh.take_request(false));
    }

    #[test]
    fn completion_postpones_the_next_periodic_scan() {
        // Given
        let start = Instant::now();
        let mut refresh = Refresh::new(start);
        refresh.completed(start + Duration::from_secs(20));
        // When
        refresh.observe(
            start + Duration::from_secs(30),
            false,
            true,
            Duration::from_secs(30),
        );
        // Then
        assert!(!refresh.take_request(false));
    }

    #[test]
    fn comparisons_do_not_collect_on_events_or_timers() {
        // Given
        let start = Instant::now();
        let mut refresh = Refresh::new(start);
        // When
        refresh.observe(
            start + Duration::from_secs(30),
            true,
            false,
            Duration::from_secs(30),
        );
        // Then
        assert!(!refresh.take_request(false));
    }

    #[test]
    fn manual_refresh_during_collection_remains_pending() {
        // Given
        let mut refresh = Refresh::new(Instant::now());
        refresh.request();
        let during_collection = refresh.take_request(true);
        // When
        let after_collection = refresh.take_request(false);
        // Then
        assert!(!during_collection);
        assert!(after_collection);
    }
}

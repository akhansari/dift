use crate::git::{Collector, Repository, WatchContext};
use notify::{
    RecommendedWatcher, RecursiveMode, Watcher,
    event::{AccessKind, AccessMode, EventKind},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, PathBuf},
    sync::mpsc,
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
        }
    }

    pub(super) fn periodic_due(&self, now: Instant, interval: Duration) -> bool {
        now.duration_since(self.last_scan) >= interval
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

fn observable(event: &notify::Event) -> bool {
    event.need_rescan()
        || !event.kind.is_access()
        || event.kind == EventKind::Access(AccessKind::Close(AccessMode::Write))
}

fn relevant_path(path: &Path, git_dir: &Path, common_dir: &Path) -> bool {
    let metadata = path
        .strip_prefix(git_dir)
        .ok()
        .or_else(|| path.strip_prefix(common_dir).ok());
    metadata.is_none_or(|path| {
        path.components().next().is_none_or(|part| {
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
}

#[cfg(test)]
fn relevant(event: &notify::Event, git_dir: &Path, common_dir: &Path) -> bool {
    event.need_rescan()
        || (observable(event)
            && (event.paths.is_empty()
                || event
                    .paths
                    .iter()
                    .any(|path| relevant_path(path, git_dir, common_dir))))
}

const MAX_EVENTS: usize = 1024;
const MAX_PATHS: usize = 4096;

#[derive(Default)]
pub(super) struct Batch {
    paths: BTreeSet<PathBuf>,
    force: bool,
    structure: bool,
}

impl Batch {
    pub(super) fn is_empty(&self) -> bool {
        !self.force && self.paths.is_empty()
    }

    fn add(&mut self, event: notify::Event) {
        self.force |= event.need_rescan() || event.paths.is_empty();
        self.structure |= !matches!(
            event.kind,
            EventKind::Access(_) | EventKind::Modify(notify::event::ModifyKind::Data(_))
        );
        if !self.force {
            for path in event.paths {
                self.paths.insert(path);
                if self.paths.len() > MAX_PATHS {
                    self.force = true;
                    break;
                }
            }
        }
        if self.force {
            self.paths.clear();
        }
    }
}

/// A bounded, non-blocking handoff from native callbacks to the collection worker.
pub(super) struct Inbox {
    events: mpsc::Receiver<notify::Event>,
    overflow: Arc<AtomicBool>,
}

struct EventSender {
    events: mpsc::SyncSender<notify::Event>,
    overflow: Arc<AtomicBool>,
}
impl EventSender {
    fn send(&self, event: notify::Event) {
        if matches!(
            self.events.try_send(event),
            Err(mpsc::TrySendError::Full(_))
        ) {
            self.overflow.store(true, Ordering::Release);
        }
    }
}
impl Inbox {
    fn channel() -> (EventSender, Self) {
        let (sender, events) = mpsc::sync_channel(MAX_EVENTS);
        let overflow = Arc::new(AtomicBool::new(false));
        (
            EventSender {
                events: sender,
                overflow: Arc::clone(&overflow),
            },
            Self { events, overflow },
        )
    }
    pub(super) fn drain_into(&self, batch: &mut Batch) -> bool {
        let mut dirty = self.overflow.swap(false, Ordering::AcqRel);
        if dirty {
            batch.force = true;
            batch.paths.clear();
        }
        for event in self.events.try_iter().take(MAX_EVENTS) {
            batch.add(event);
            dirty = true;
        }
        dirty
    }
}

pub(super) struct Watches {
    watcher: Option<RecommendedWatcher>,
    recursive: Vec<PathBuf>,
    external: BTreeSet<PathBuf>,
    degraded: Arc<AtomicBool>,
}

impl Watches {
    pub(super) fn new(
        repo: &Repository,
        degraded: Arc<AtomicBool>,
        enabled: bool,
    ) -> (Self, Inbox) {
        let (sender, inbox) = Inbox::channel();
        let callback_overflow = Arc::clone(&inbox.overflow);
        let callback_degraded = Arc::clone(&degraded);
        let callback = move |result: notify::Result<notify::Event>| match result {
            Ok(event) if observable(&event) => {
                sender.send(event);
            }
            Err(_) => {
                callback_degraded.store(true, Ordering::Relaxed);
                callback_overflow.store(true, Ordering::Release);
            }
            _ => {}
        };
        let watcher = enabled
            .then(|| notify::recommended_watcher(callback))
            .and_then(|result| match result {
                Ok(watcher) => Some(watcher),
                Err(_) => {
                    degraded.store(true, Ordering::Relaxed);
                    None
                }
            });
        let mut watches = Self {
            watcher,
            recursive: Vec::new(),
            external: BTreeSet::new(),
            degraded,
        };
        for path in [&repo.root, &repo.common_dir, &repo.git_dir] {
            if !watches.recursive.iter().any(|root| path.starts_with(root))
                && let Some(watcher) = &mut watches.watcher
            {
                match watcher.watch(path, RecursiveMode::Recursive) {
                    Ok(()) => watches.recursive.push(path.clone()),
                    Err(_) => {
                        watches.degraded.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        (watches, inbox)
    }

    pub(super) fn update_external(&mut self, context: &WatchContext) {
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        let desired: BTreeSet<_> = context
            .resources
            .iter()
            .filter(|path| !self.recursive.iter().any(|root| path.starts_with(root)))
            // Watching parents catches atomic replacement, including a previously missing file.
            .filter_map(|path| {
                path.ancestors()
                    .skip(1)
                    .find(|parent| parent.is_dir())
                    .map(Path::to_owned)
            })
            .collect();
        let mut registered = BTreeSet::new();
        for path in desired {
            if self.external.contains(&path)
                || watcher.watch(&path, RecursiveMode::NonRecursive).is_ok()
            {
                registered.insert(path);
            } else {
                self.degraded.store(true, Ordering::Relaxed);
            }
        }
        for path in self.external.difference(&registered) {
            let _ = watcher.unwatch(path);
        }
        self.external = registered;
    }
}

pub(super) struct EventFilter {
    context: WatchContext,
    matcher: Option<ignore::IncrementalIgnore>,
    checked_directories: BTreeSet<PathBuf>,
}

impl EventFilter {
    pub(super) fn invalidates_configuration(&self, batch: &Batch) -> bool {
        batch.force
            || batch.paths.iter().any(|path| {
                self.context.resource_changed(path)
                    || ((path.starts_with(&self.context.git_dir)
                        || path.starts_with(&self.context.common_dir))
                        && relevant_path(path, &self.context.git_dir, &self.context.common_dir))
            })
    }
    pub(super) fn new(context: WatchContext) -> Self {
        let matcher = Self::matcher(&context);
        Self {
            context,
            matcher,
            checked_directories: BTreeSet::new(),
        }
    }

    fn matcher(context: &WatchContext) -> Option<ignore::IncrementalIgnore> {
        let mut builder = ignore::WalkBuilder::new(&context.root);
        builder
            .hidden(false)
            .ignore(false)
            .parents(false)
            .git_global(false)
            .git_exclude(false)
            .git_ignore(true)
            .require_git(false)
            .ignore_case_insensitive(context.ignore_case)
            .current_dir(&context.root);
        for path in &context.excludes {
            match fs::metadata(path) {
                Ok(_) => {
                    if builder.add_ignore(path).is_some() {
                        return None;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return None,
            }
        }
        builder.build_matchers().pop()
    }

    /// Drop a batch only when every path is certainly ignored and untracked.
    pub(super) fn should_collect(&mut self, batch: &Batch, collector: &Collector) -> bool {
        if batch.force {
            return true;
        }
        // Directory moves can move or remove cached .gitignore files.
        if batch.structure {
            self.matcher = Self::matcher(&self.context);
            self.checked_directories.clear();
        }
        batch.paths.iter().any(|path| {
            let context = &self.context;
            if context.resource_changed(path) {
                return true;
            }
            if path.starts_with(&context.git_dir) || path.starts_with(&context.common_dir) {
                return relevant_path(path, &context.git_dir, &context.common_dir);
            }
            let Ok(relative) = path.strip_prefix(&context.root) else {
                // Parent watches also report siblings of external configuration files.
                return false;
            };
            if relative.as_os_str().is_empty()
                || relative
                    .components()
                    .any(|part| matches!(part, Component::ParentDir))
                || matches!(
                    relative.file_name().and_then(|name| name.to_str()),
                    Some(".gitignore" | ".gitattributes" | ".gitmodules" | ".git")
                )
                || collector.may_be_tracked(relative)
            {
                return true;
            }
            // ignore's hierarchical loader suppresses I/O errors. Check new directories
            // ourselves so an unreadable negation never silently hides a change.
            for directory in relative.ancestors().skip(1) {
                if self.checked_directories.insert(directory.to_owned()) {
                    let rule = context.root.join(directory).join(".gitignore");
                    let readable = match fs::symlink_metadata(&rule) {
                        Ok(metadata) if metadata.is_file() => fs::read_to_string(rule).is_ok(),
                        Ok(_) => true,
                        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                    };
                    if !readable {
                        self.matcher = None;
                    }
                }
            }
            let Some(matcher) = &mut self.matcher else {
                return true;
            };
            let kind = match fs::symlink_metadata(path) {
                Ok(metadata) => Some(metadata.is_dir()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return true,
            };
            let (file_match, file_error) =
                matcher.matched_with_errors(relative, kind.unwrap_or(false));
            let (dir_match, dir_error) = if kind.is_none() {
                matcher.matched_with_errors(relative, true)
            } else {
                (file_match.clone(), None)
            };
            if file_error.is_some() || dir_error.is_some() {
                // Errors are only reported while loading. Disable this generation, including cached matches.
                self.matcher = None;
                true
            } else {
                !file_match.is_ignore() || !dir_match.is_ignore()
            }
        })
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

#[cfg(test)]
mod filtering_tests {
    use super::*;
    use crate::{AppResult, ui::fixtures::Fixture};
    use notify::event::{DataChange, Flag, ModifyKind, RenameMode};

    fn changed(path: PathBuf) -> Batch {
        let mut batch = Batch::default();
        batch.add(
            notify::Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any))).add_path(path),
        );
        batch
    }

    #[test]
    fn ignored_untracked_output_does_not_request_collection() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        fixture.write("target/output", "ignored\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect =
            filter.should_collect(&changed(fixture.root.join("target/output")), &collector);
        // Then
        assert!(!collect);
        Ok(())
    }

    #[test]
    fn tracked_files_are_never_filtered_by_ignore_rules() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("*.txt\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect = filter.should_collect(&changed(fixture.root.join("tracked.txt")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn hidden_files_and_dot_ignore_rules_do_not_hide_git_changes() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        fixture.write(".ignore", "*\n")?;
        fixture.write(".secret", "visible\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect = filter.should_collect(&changed(fixture.root.join(".secret")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn nested_negation_keeps_an_untracked_file_visible() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("*.log\n")?;
        fixture.write("nested/.gitignore", "!keep.log\n")?;
        fixture.write("nested/keep.log", "visible\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect =
            filter.should_collect(&changed(fixture.root.join("nested/keep.log")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn global_excludes_are_resolved_relative_to_the_repository() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        fixture.write(".git/global-ignore", "/cache/\n")?;
        fixture.git(&["config", "core.excludesFile", ".git/global-ignore"])?;
        fixture.write("cache/output", "ignored\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect =
            filter.should_collect(&changed(fixture.root.join("cache/output")), &collector);
        // Then
        assert!(!collect);
        Ok(())
    }

    #[test]
    fn worktree_negation_overrides_repository_and_global_excludes() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("!keep.log\n")?;
        fixture.write(".git/global-ignore", "*.log\n")?;
        fixture.write(".git/info/exclude", "*.log\n")?;
        fixture.git(&["config", "core.excludesFile", ".git/global-ignore"])?;
        fixture.write("keep.log", "visible\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect = filter.should_collect(&changed(fixture.root.join("keep.log")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn deleted_ignored_files_do_not_require_metadata_to_be_filtered() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect =
            filter.should_collect(&changed(fixture.root.join("target/gone.o")), &collector);
        // Then
        assert!(!collect);
        Ok(())
    }

    #[test]
    fn removing_an_ignored_directory_with_tracked_descendants_collects() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        fixture.write("target/pinned.txt", "tracked\n")?;
        fixture.git(&["add", "-f", "target/pinned.txt"])?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        fs::remove_dir_all(fixture.root.join("target"))?;
        // When
        let collect = filter.should_collect(&changed(fixture.root.join("target")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn rename_from_a_visible_path_to_an_ignored_path_collects() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        let mut batch = Batch::default();
        batch.add(
            notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(fixture.root.join("visible.txt"))
                .add_path(fixture.root.join("target/hidden.txt")),
        );
        // When
        let collect = filter.should_collect(&batch, &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn rescan_flag_bypasses_ignore_matching() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        let mut batch = Batch::default();
        batch.add(
            notify::Event::new(EventKind::Other)
                .set_flag(Flag::Rescan)
                .add_path(fixture.root.join("target/output")),
        );
        // When
        let collect = filter.should_collect(&batch, &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn ignore_rule_changes_always_request_collection() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("*\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect =
            filter.should_collect(&changed(fixture.root.join("nested/.gitignore")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn ignore_loading_errors_keep_events_relevant() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("*.log\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        fs::create_dir_all(fixture.root.join("nested"))?;
        fs::write(fixture.root.join("nested/.gitignore"), [0xff, 0xfe])?;
        // When
        let collect =
            filter.should_collect(&changed(fixture.root.join("nested/output.log")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    fn overflow_with_ignored_events(sender: &EventSender, path: &Path) {
        for _ in 0..=MAX_EVENTS {
            sender.send(
                notify::Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
                    .add_path(path.to_owned()),
            );
        }
    }

    #[test]
    fn queue_overflow_requests_a_full_collection_even_for_ignored_paths() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        let (sender, inbox) = Inbox::channel();
        overflow_with_ignored_events(&sender, &fixture.root.join("target/output"));
        let mut batch = Batch::default();
        // When
        inbox.drain_into(&mut batch);
        let collect = filter.should_collect(&batch, &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn an_event_received_after_drain_survives_the_previous_collection() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        let (sender, inbox) = Inbox::channel();
        let mut consumed = Batch::default();
        inbox.drain_into(&mut consumed);
        sender.send(notify::Event::new(EventKind::Any).add_path(fixture.root.join("tracked.txt")));
        let mut pending = Batch::default();
        // When
        inbox.drain_into(&mut pending);
        let collect = filter.should_collect(&pending, &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn case_insensitive_tracking_preserves_ignored_file_events() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("*.txt\n")?;
        fixture.git(&["config", "core.ignoreCase", "true"])?;
        let collector = fixture.collector()?;
        let mut filter = EventFilter::new(collector.watch_context()?);
        // When
        let collect = filter.should_collect(&changed(fixture.root.join("TRACKED.TXT")), &collector);
        // Then
        assert!(collect);
        Ok(())
    }

    #[test]
    fn an_ignored_batch_does_not_postpone_periodic_reconciliation() {
        // Given
        let start = Instant::now();
        let mut refresh = Refresh::new(start);
        refresh.observe(
            start + Duration::from_secs(20),
            true,
            true,
            Duration::from_secs(30),
        );
        refresh.observe(
            start + Duration::from_secs(21),
            false,
            true,
            Duration::from_secs(30),
        );
        let _ignored_request = refresh.take_request(false);
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
}

use super::watch::{Batch, EventFilter, Refresh, Watches};
use crate::{
    git::{Collector, GitError, Limits, Mode, Repository},
    model::Snapshot,
};
use std::{
    collections::BTreeSet,
    io,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant},
};

pub(super) enum Update {
    Started,
    Finished(Result<Snapshot, GitError>),
}

pub(super) struct Worker {
    requests: mpsc::Sender<BTreeSet<PathBuf>>,
    pub updates: mpsc::Receiver<Update>,
    pub degraded: Arc<AtomicBool>,
}

impl Worker {
    pub(super) fn spawn(
        paths: Repository,
        mode: Mode,
        limits: Limits,
        interval: Duration,
    ) -> io::Result<Self> {
        let (requests, receive) = mpsc::channel();
        let (send, updates) = mpsc::channel();
        let degraded = Arc::new(AtomicBool::new(false));
        let watcher_degraded = Arc::clone(&degraded);
        thread::Builder::new()
            .name("git-collect".into())
            .spawn(move || {
                let watching = matches!(mode, Mode::Watch);
                // Register before the first snapshot, so startup changes stay pending.
                let (mut watches, inbox) = Watches::new(&paths, watcher_degraded, watching);
                let mut collector = None;
                let mut filter: Option<EventFilter> = None;
                let mut loaded = BTreeSet::new();
                let mut refresh = Refresh::new(Instant::now());
                let mut pending = Batch::default();
                loop {
                    let mut force = match receive.recv_timeout(Duration::from_millis(33)) {
                        Ok(request) => {
                            loaded = request;
                            true
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => false,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    for request in receive.try_iter() {
                        loaded = request;
                        force = true;
                    }
                    let dirty = inbox.drain_into(&mut pending);
                    let now = Instant::now();
                    let periodic = watching && refresh.periodic_due(now, interval);
                    refresh.observe(now, dirty, watching, interval);
                    let ready = refresh.take_request(false);
                    if !force && !ready {
                        continue;
                    }
                    let batch = std::mem::take(&mut pending);
                    if batch.is_empty() && !force && !periodic {
                        continue;
                    }
                    let result = (|| -> Result<Option<Snapshot>, GitError> {
                        if collector.is_none() {
                            collector = Some(Collector::new(paths.clone())?);
                        }
                        let Some(collector) = &mut collector else {
                            return Ok(None);
                        };
                        let full = force
                            || periodic
                            || filter
                                .as_ref()
                                .is_none_or(|filter| filter.invalidates_configuration(&batch));
                        if !full
                            && filter
                                .as_mut()
                                .is_some_and(|filter| !filter.should_collect(&batch, collector))
                        {
                            return Ok(None);
                        }
                        let _ = send.send(Update::Started);
                        if full {
                            collector.refresh_configuration(true)?;
                        }
                        if watching {
                            watches.update_external(&collector.watch_context()?);
                        }
                        let snapshot = collector.snapshot(&mode, limits, &loaded)?;
                        let context = collector.watch_context()?;
                        if watching {
                            watches.update_external(&context);
                        }
                        filter = Some(EventFilter::new(context));
                        Ok(Some(snapshot))
                    })();
                    let result = match result {
                        Ok(None) => continue, // Ignored events don't change the reconciliation clock or UI.
                        Ok(Some(snapshot)) => Ok(snapshot),
                        Err(error) => {
                            filter = None;
                            Err(error)
                        }
                    };
                    refresh.completed(Instant::now());
                    if send.send(Update::Finished(result)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            updates,
            degraded,
        })
    }

    pub(super) fn request(
        &self,
        loaded: BTreeSet<PathBuf>,
    ) -> Result<(), mpsc::SendError<BTreeSet<PathBuf>>> {
        self.requests.send(loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppResult, ui::fixtures::Fixture};

    fn next_snapshot(worker: &Worker) -> AppResult<Snapshot> {
        loop {
            if let Update::Finished(result) = worker.updates.recv_timeout(Duration::from_secs(4))? {
                return Ok(result?);
            }
        }
    }
    fn watching(fixture: &Fixture) -> AppResult<Worker> {
        let worker = Worker::spawn(
            fixture.repo()?,
            Mode::Watch,
            Limits::default(),
            Duration::from_secs(3600),
        )?;
        worker.request(BTreeSet::new())?;
        let _initial = next_snapshot(&worker)?;
        Ok(worker)
    }

    #[test]
    fn ignored_build_burst_does_not_start_a_collection() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        let worker = watching(&fixture)?;
        // When
        fixture.build_burst()?;
        let update = worker.updates.recv_timeout(Duration::from_millis(900));
        // Then
        assert!(matches!(update, Err(mpsc::RecvTimeoutError::Timeout)));
        Ok(())
    }

    #[test]
    fn a_tracked_edit_among_ignored_outputs_refreshes_content() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n*.txt\n")?;
        let worker = watching(&fixture)?;
        // When
        fixture.build_burst()?;
        fixture.write("tracked.txt", "edited during build\n")?;
        let snapshot = next_snapshot(&worker)?;
        // Then
        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(
            snapshot.changes[0].new_source.as_deref(),
            Some("edited during build\n")
        );
        Ok(())
    }

    #[test]
    fn manual_refresh_runs_even_without_filesystem_changes() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        let worker = watching(&fixture)?;
        // When
        worker.request(BTreeSet::new())?;
        let snapshot = next_snapshot(&worker)?;
        // Then
        assert!(snapshot.changes.is_empty());
        Ok(())
    }

    #[test]
    fn custom_excludes_inside_git_metadata_are_watched() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        fixture.write(".git/custom-ignore", "hidden.log\n")?;
        fixture.git(&["config", "core.excludesFile", ".git/custom-ignore"])?;
        fixture.write("hidden.log", "revealed\n")?;
        let worker = watching(&fixture)?;
        // When
        fixture.write(".git/custom-ignore", "")?;
        let snapshot = next_snapshot(&worker)?;
        // Then
        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(
            snapshot.changes[0].new_source.as_deref(),
            Some("revealed\n")
        );
        Ok(())
    }

    #[test]
    fn unrelated_siblings_of_external_config_do_not_start_collection() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("")?;
        let external = Fixture::new("")?;
        external.write("rules", "*.log\n")?;
        external.write("unrelated", "original\n")?;
        fixture.git(&[
            "config",
            "core.excludesFile",
            &external.root.join("rules").to_string_lossy(),
        ])?;
        let worker = watching(&fixture)?;
        // When
        external.write("unrelated", "unrelated change\n")?;
        let update = worker.updates.recv_timeout(Duration::from_millis(900));
        // Then
        assert!(matches!(update, Err(mpsc::RecvTimeoutError::Timeout)));
        Ok(())
    }

    #[test]
    fn changing_ignore_rules_reveals_previously_hidden_files() -> AppResult<()> {
        // Given
        let fixture = Fixture::new("target/\n")?;
        fixture.write("target/output", "revealed\n")?;
        let worker = watching(&fixture)?;
        // When
        fixture.write(".gitignore", "")?;
        let snapshot = next_snapshot(&worker)?;
        // Then
        assert!(snapshot.changes.iter().any(|change| change.path
            == std::path::Path::new("target/output")
            && change.new_source.as_deref() == Some("revealed\n")));
        Ok(())
    }
}

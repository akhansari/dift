use super::{Mutation, Request, Update, mutations::Mutations};
use crate::{
    git::{Collector, DISCARD_MODES, GitError, Limits, Mode, Repository, StageTarget},
    model::Snapshot,
    state::CollectionRequest,
    ui::watch::{Batch, EventFilter, Inbox, Refresh, Watches},
};
use std::{
    sync::{Arc, atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

/// Owns collection, attribution, and watch state on the Git worker thread.
pub(super) struct Runner {
    paths: Repository,
    mode: Mode,
    limits: Limits,
    interval: Duration,
    watching: bool,
    updates: mpsc::Sender<Update>,
    watches: Watches,
    inbox: Inbox,
    collector: Option<Collector>,
    filter: Option<EventFilter>,
    latest: CollectionRequest,
    refresh: Refresh,
    pending: Batch,
    coverage_refresh: bool,
    mutations: Mutations,
    #[cfg(test)]
    pub(super) initialized: Option<mpsc::Sender<()>>,
}

impl Runner {
    pub(super) fn new(
        paths: Repository,
        mode: Mode,
        limits: Limits,
        interval: Duration,
        degraded: Arc<AtomicBool>,
        updates: mpsc::Sender<Update>,
    ) -> Self {
        let watching = matches!(mode, Mode::Watch);
        // Bootstrap shallow watches without traversing the working tree.
        let (watches, inbox) = Watches::new(&paths, degraded, watching);
        Self {
            paths,
            mode,
            limits,
            interval,
            watching,
            updates,
            watches,
            inbox,
            collector: None,
            filter: None,
            latest: CollectionRequest::default(),
            refresh: Refresh::new(Instant::now()),
            pending: Batch::default(),
            coverage_refresh: false,
            mutations: Mutations::default(),
            #[cfg(test)]
            initialized: None,
        }
    }

    pub(super) fn run(&mut self, requests: mpsc::Receiver<Request>) {
        loop {
            let coverage_changed = std::mem::take(&mut self.coverage_refresh);
            let timeout = if coverage_changed {
                Duration::ZERO
            } else {
                Duration::from_millis(33)
            };
            let mut mutations = Vec::new();
            let mut force = match requests.recv_timeout(timeout) {
                Ok(request) => {
                    self.accept(request, &mut mutations);
                    true
                }
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            force |= coverage_changed;
            for request in requests.try_iter() {
                self.accept(request, &mut mutations);
                force = true;
            }
            let mut outcomes = Vec::with_capacity(mutations.len());
            for mutation in mutations {
                outcomes.push(self.execute(mutation));
            }
            let dirty = self.inbox.drain_into(&mut self.pending);
            let now = Instant::now();
            let periodic = self.watching && self.refresh.periodic_due(now, self.interval);
            self.refresh
                .observe(now, dirty, self.watching, self.interval);
            let ready = self.refresh.take_request(false);
            if !force && !ready {
                continue;
            }
            let batch = std::mem::take(&mut self.pending);
            if batch.is_empty() && !force && !periodic {
                continue;
            }
            let before = self.mutations.before_refresh(&self.paths);
            let (result, reconcile) = match self.collect(&batch, force, periodic) {
                // Ignored events don't change the reconciliation clock or UI.
                Ok(None) => continue,
                Ok(Some((snapshot, reconcile))) => (Ok(snapshot), reconcile),
                Err(error) => {
                    self.filter = None;
                    (Err(error), false)
                }
            };
            if let Some(update) = self.mutations.finish(&self.paths, &before, &result) {
                let _ = self.updates.send(update);
            }
            self.refresh.completed(Instant::now());
            if self
                .updates
                .send(Update::Finished(self.latest.generation, result))
                .is_err()
            {
                break;
            }
            for outcome in outcomes {
                let _ = self.updates.send(outcome);
            }
            if reconcile {
                self.reconcile(&batch, periodic);
            }
            // Signal only after startup coverage and its follow-up collection settle.
            #[cfg(test)]
            if !self.coverage_refresh
                && let Some(ready) = self.initialized.take()
            {
                let _ = ready.send(());
            }
        }
    }

    fn accept(&mut self, request: Request, mutations: &mut Vec<Mutation>) {
        match request {
            Request::Collect(request) => self.latest = request,
            Request::Mode(mode, request) => {
                self.mode = mode;
                self.latest = request;
            }
            Request::Toggle(target, request) => {
                self.latest = request;
                mutations.push(Mutation::Toggle(target));
            }
            Request::PrepareDiscard(target) => self.prepare_discard(&target),
            Request::Discard(preview, request) => {
                self.latest = request;
                mutations.push(Mutation::Discard(preview));
            }
        }
    }

    fn prepare_discard(&self, target: &StageTarget) {
        let options = DISCARD_MODES.map(|mode| {
            if self.watching {
                self.paths
                    .prepare_discard(target, mode)
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            } else {
                Err("Discard is unavailable in comparison mode".into())
            }
        });
        let _ = self.updates.send(Update::DiscardPrepared(options));
    }

    fn execute(&mut self, mutation: Mutation) -> Update {
        if !self.watching {
            let error = GitError::Command("Git changes are unavailable in comparison mode".into());
            return match mutation {
                Mutation::Toggle(_) => Update::MutationFinished(Err(error)),
                Mutation::Discard(_) => Update::DiscardFinished(Err(error)),
            };
        }
        // Observe external edits before the command changes their category or index.
        let guard = self.mutations.before_refresh(&self.paths);
        let head = self.paths.activity_head().ok();
        let before = self.snapshot();
        if let Some(update) = self.mutations.finish(&self.paths, &guard, &before) {
            let _ = self.updates.send(update);
        }
        if let Ok(snapshot) = &before {
            if let Some(observations) = &snapshot.activity {
                let _ = self
                    .updates
                    .send(Update::ObservedActivity(observations.clone()));
            }
        } else {
            // Without a pre-command observation, overlapping attribution is ambiguous.
            self.mutations.invalidate();
        }
        let (attribution, update) = match mutation {
            Mutation::Toggle(target) => {
                let report = self.paths.toggle_stage_tracked(&target);
                (report.attribution, Update::MutationFinished(report.outcome))
            }
            Mutation::Discard(preview) => {
                let report = self.paths.discard_tracked(&preview);
                (report.attribution, Update::DiscardFinished(report.outcome))
            }
        };
        self.mutations.record(
            attribution,
            before
                .as_ref()
                .ok()
                .and_then(|snapshot| snapshot.activity.as_ref()),
            head.as_deref(),
        );
        update
    }

    fn snapshot(&mut self) -> Result<Snapshot, GitError> {
        collector(&self.paths, &mut self.collector)?.snapshot_with_whitespace(
            &self.mode,
            self.limits,
            &self.latest.loaded,
            self.latest.whitespace,
        )
    }

    fn collect(
        &mut self,
        batch: &Batch,
        force: bool,
        periodic: bool,
    ) -> Result<Option<(Snapshot, bool)>, GitError> {
        let collector = collector(&self.paths, &mut self.collector)?;
        let full = force
            || periodic
            || self
                .filter
                .as_ref()
                .is_none_or(|filter| filter.invalidates_configuration(batch));
        if !full
            && self
                .filter
                .as_mut()
                .is_some_and(|filter| !filter.should_collect(batch, collector))
        {
            return Ok(None);
        }
        let _ = self.updates.send(Update::Started(self.latest.generation));
        if full {
            collector.refresh_configuration(true)?;
        }
        let snapshot = collector.snapshot_with_whitespace(
            &self.mode,
            self.limits,
            &self.latest.loaded,
            self.latest.whitespace,
        )?;
        self.filter = Some(EventFilter::new(collector.watch_context()?));
        Ok(Some((
            snapshot,
            self.watching && (full || batch.changes_directories()),
        )))
    }

    fn reconcile(&mut self, batch: &Batch, periodic: bool) {
        if let (Some(filter), Some(collector)) = (&mut self.filter, &self.collector) {
            // Publish first, then catch edits made before their directories were watched.
            self.coverage_refresh = self.watches.reconcile(filter, collector, batch, periodic);
        }
    }
}

/// Retry failed initialization, returning the initialized collector without a fallback state.
fn collector<'a>(
    repo: &Repository,
    slot: &'a mut Option<Collector>,
) -> Result<&'a mut Collector, GitError> {
    match slot {
        Some(collector) => Ok(collector),
        None => Ok(slot.insert(Collector::new(repo.clone())?)),
    }
}

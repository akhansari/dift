//! Commit history navigation and revision-specific views.
mod detail;
mod list;
mod worker;

use super::{Interaction, collect};
use crate::{
    AppResult,
    config::Config,
    git::{CommitInfo, Repository},
    input::{Action, Bindings, Editor, Scope},
    render::worker::RenderWorker,
    state::State,
};
use std::time::Instant;
use worker::Worker;

const PAGE: usize = 200;

pub(super) enum Screen {
    List,
    Detail(Box<DetailScreen>),
}

pub(super) enum DetailKind {
    Commit {
        commit: CommitInfo,
        presentation: crate::model::Presentation,
    },
    Comparison {
        base: CommitInfo,
        target: Box<CommitInfo>,
    },
}

pub(super) struct DetailScreen {
    kind: DetailKind,
    viewer: Detail,
    info: Option<InfoOverlay>,
}

struct Detail {
    state: State,
    interaction: Interaction,
    collecting: collect::Worker,
    rendering: RenderWorker,
}

#[derive(Default)]
struct InfoOverlay {
    scroll: u16,
    viewport: u16,
}

pub(super) struct History {
    repo: Repository,
    config: Config,
    list: HistoryList,
    screen: Screen,
    visible: bool,
}

#[derive(Default)]
struct HistoryList {
    worker: Option<Worker>,
    tip: Option<String>,
    branch: String,
    commits: Vec<CommitInfo>,
    cursor: usize,
    offset: usize,
    base: Option<CommitInfo>,
    query: String,
    search: Option<(Editor, String)>,
    search_deadline: Option<Instant>,
    more: bool,
    loading: bool,
    generation: u64,
    error: Option<String>,
    help: bool,
    help_scroll: usize,
}

impl History {
    pub fn new(repo: Repository, config: Config) -> Self {
        Self {
            repo,
            config,
            list: HistoryList::default(),
            screen: Screen::List,
            visible: false,
        }
    }

    pub fn toggle(&mut self) -> AppResult<()> {
        self.visible = !self.visible;
        if !self.visible {
            return Ok(());
        }
        if self.list.worker.is_none() {
            self.list.worker = Some(Worker::spawn(self.repo.clone())?);
            self.load(true, false)?;
        }
        if self.detail_state().is_some() {
            self.request_render()?;
        }
        Ok(())
    }

    pub fn tick(&mut self, now: Instant) -> AppResult<bool> {
        let list = self.poll_list(now)?;
        let detail = self.poll_detail(now)?;
        Ok(list || detail)
    }

    pub fn detail_state(&self) -> Option<&State> {
        match self.screen() {
            Some(Screen::Detail(screen)) => Some(&screen.viewer.state),
            _ => None,
        }
    }

    pub fn detail_mut(&mut self) -> Option<&mut State> {
        match (self.visible, &mut self.screen) {
            (true, Screen::Detail(screen)) => Some(&mut screen.viewer.state),
            _ => None,
        }
    }

    pub fn screen(&self) -> Option<&Screen> {
        self.visible.then_some(&self.screen)
    }
}

fn safe(value: &str) -> String {
    crate::text::escape_controls(value.chars())
}

fn safe_message(value: &str) -> String {
    value.split('\n').map(safe).collect::<Vec<_>>().join("\n")
}

fn shortcut(bindings: &Bindings, action: Action) -> String {
    bindings.labels_for(Scope::History, action)
}

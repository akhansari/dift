use super::{Interaction, collect, commit, editor};
use crate::{
    AppResult,
    git::DiscardOutcome,
    state::{Command, Effects, State},
};
use ratatui::{DefaultTerminal, layout::Size};
use std::{ops::ControlFlow, path::PathBuf, time::Instant};

#[derive(Default)]
enum Operation {
    #[default]
    Idle,
    Staging,
    PreparingDiscard,
    Discarding,
    Committing(commit::Session),
}

/// Serializes repository mutations and owns the active commit process.
pub(super) struct Commands {
    root: PathBuf,
    watching: bool,
    operation: Operation,
}

pub(super) struct Context<'a> {
    pub terminal: &'a mut DefaultTerminal,
    pub state: &'a mut State,
    pub collecting: &'a collect::Worker,
    pub interaction: &'a mut Interaction,
    pub size: Size,
    pub height: usize,
}

impl Commands {
    pub(super) fn new(root: PathBuf, watching: bool) -> Self {
        Self {
            root,
            watching,
            operation: Operation::Idle,
        }
    }

    fn idle(&self) -> bool {
        matches!(self.operation, Operation::Idle)
    }

    pub(super) fn update_commit(
        &mut self,
        terminal: &mut DefaultTerminal,
        state: &mut State,
        collecting: &collect::Worker,
    ) -> AppResult<bool> {
        if let Operation::Committing(session) = &mut self.operation
            && let Some(result) = session.update(terminal)?
        {
            self.operation = Operation::Idle;
            state.commit_finished(result);
            collecting.request(state.collection_request())?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn resize(&mut self, size: Size) -> AppResult<()> {
        if let Operation::Committing(session) = &mut self.operation {
            session.resize(size.width, size.height)?;
        }
        Ok(())
    }

    pub(super) fn update(
        &mut self,
        update: collect::Update,
        state: &mut State,
        collecting: &collect::Worker,
    ) -> AppResult<Effects> {
        match update {
            collect::Update::MutationFinished(result) => {
                self.operation = Operation::Idle;
                if let Err(error) = result {
                    state.show_toast(error.to_string(), Instant::now());
                }
            }
            collect::Update::DiscardPrepared(options) => {
                self.operation = Operation::Idle;
                state.discard_prepared(options);
            }
            collect::Update::DiscardFinished(result) => {
                self.operation = Operation::Idle;
                if matches!(result, Ok(DiscardOutcome::Stale)) {
                    if let Some(target) = state.discard_stale() {
                        collecting.prepare_discard(target)?;
                        self.operation = Operation::PreparingDiscard;
                    }
                } else {
                    state.close_discard();
                    state.show_toast(
                        match result {
                            Ok(DiscardOutcome::Discarded) => "Discarded changes".into(),
                            Ok(_) => "Nothing to discard".into(),
                            Err(error) => error.to_string(),
                        },
                        Instant::now(),
                    );
                }
            }
            collect::Update::ObservedActivity(observations) => {
                state.observe_activity(observations, Instant::now());
            }
            collect::Update::InternalActivity(paths, observations, external) => {
                state.internal_activity(&paths, observations);
                state.highlight_activity(&external, Instant::now());
            }
            collect::Update::Started(generation) => state.collection_started(generation),
            collect::Update::Finished(generation, result) => {
                let mut effects = state.collection_finished(generation, result, Instant::now());
                effects.redraw = true;
                return Ok(effects);
            }
        }
        Ok(Effects {
            redraw: true,
            ..Effects::default()
        })
    }

    pub(super) fn dispatch(
        &mut self,
        command: Command,
        context: Context<'_>,
    ) -> AppResult<ControlFlow<(), bool>> {
        let Context {
            terminal,
            state,
            collecting,
            interaction,
            size,
            height,
        } = context;
        let mut redraw = false;
        match command {
            Command::Quit => return Ok(ControlFlow::Break(())),
            Command::Collect => collecting.request(state.collection_request())?,
            Command::OpenCommit if self.idle() => {
                if self.watching {
                    state.begin_commit();
                    interaction.selection.clear();
                } else {
                    state.show_toast(
                        "Committing is unavailable in comparison mode".into(),
                        Instant::now(),
                    );
                }
                redraw = true;
            }
            Command::SubmitCommit
                if self.watching
                    && self.idle()
                    && size.width >= commit::MIN_WIDTH
                    && size.height >= commit::MIN_HEIGHT =>
            {
                if let Some(message) = state.commit_message() {
                    state.commit_started();
                    match commit::Session::start(&self.root, &message, size.width, size.height) {
                        Ok(session) => self.operation = Operation::Committing(session),
                        Err(error) => {
                            let error = format!("Cannot run git commit: {error}");
                            commit::show_failure(terminal, &error)?;
                            state.commit_finished(Err(error));
                            collecting.request(state.collection_request())?;
                        }
                    }
                    redraw = true;
                }
            }
            Command::OpenDiscard if self.idle() => {
                if !self.watching {
                    state.show_toast(
                        "Discard is unavailable in comparison mode".into(),
                        Instant::now(),
                    );
                } else if let Some(target) = state.change_target(height) {
                    collecting.prepare_discard(target.clone())?;
                    state.begin_discard(target);
                    interaction.selection.clear();
                    self.operation = Operation::PreparingDiscard;
                } else {
                    state.show_toast("Nothing to discard".into(), Instant::now());
                }
                redraw = true;
            }
            Command::ConfirmDiscard if self.idle() && size.width >= 40 && size.height >= 12 => {
                if let Some(preview) = state.take_discard_preview() {
                    collecting.discard(preview, state.collection_request())?;
                    self.operation = Operation::Discarding;
                }
            }
            Command::ToggleStage if self.idle() => {
                if !self.watching {
                    state.show_toast(
                        "Staging is unavailable in comparison mode".into(),
                        Instant::now(),
                    );
                } else if let Some(target) = state.stage_target(height) {
                    collecting.toggle(target, state.collection_request())?;
                    self.operation = Operation::Staging;
                } else {
                    state.show_toast("Nothing to stage or unstage".into(), Instant::now());
                }
                redraw = true;
            }
            Command::OpenEditor(position) => {
                let result = match editor::EditorCommand::prepare(
                    &self.root,
                    state.editor_target(height, position),
                    std::env::var_os("VISUAL"),
                    std::env::var_os("EDITOR"),
                ) {
                    Ok(command) => {
                        interaction.selection.clear();
                        let result = command.open(terminal)?;
                        if self.watching {
                            collecting.request(state.collection_request())?;
                        }
                        result
                    }
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    state.show_toast(error.to_string(), Instant::now());
                }
                redraw = true;
            }
            _ => {}
        }
        Ok(ControlFlow::Continue(redraw))
    }
}

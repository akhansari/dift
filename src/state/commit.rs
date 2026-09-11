use super::{Effects, State};
use crate::input::{Action, Bindings, Editor, MultilineEditor, Scope};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Default, PartialEq, Eq)]
pub(crate) enum CommitField {
    #[default]
    Subject,
    Body,
}

#[derive(Default, PartialEq, Eq)]
enum CommitPhase {
    #[default]
    Closed,
    Editing,
    Running,
}

pub(crate) struct CommitDialog {
    pub subject: Editor,
    pub body: MultilineEditor,
    pub field: CommitField,
    pub notice: Option<String>,
    phase: CommitPhase,
}

impl Default for CommitDialog {
    fn default() -> Self {
        Self {
            subject: Editor::new(String::new()),
            body: MultilineEditor::default(),
            field: CommitField::Subject,
            notice: None,
            phase: CommitPhase::Closed,
        }
    }
}

impl CommitDialog {
    pub(crate) fn running(&self) -> bool {
        self.phase == CommitPhase::Running
    }
}

impl State {
    pub(crate) fn commit_dialog(&self) -> Option<&CommitDialog> {
        (self.commit.phase != CommitPhase::Closed).then_some(&self.commit)
    }

    pub(crate) fn begin_commit(&mut self) {
        self.commit.phase = CommitPhase::Editing;
        self.commit.field = CommitField::Subject;
        self.commit.notice = None;
    }

    pub(crate) fn commit_message(&mut self) -> Option<String> {
        if self.commit.phase == CommitPhase::Closed {
            return None;
        }
        let subject = self.commit.subject.text().trim();
        if subject.is_empty() {
            self.commit.notice = Some("Enter a commit subject".into());
            self.commit.field = CommitField::Subject;
            return None;
        }
        let body = self.commit.body.editor().text().trim_matches('\n');
        Some(if body.trim().is_empty() {
            subject.to_owned()
        } else {
            format!("{subject}\n\n{body}")
        })
    }

    pub(crate) fn commit_started(&mut self) {
        self.commit.phase = CommitPhase::Running;
        self.commit.notice = None;
    }

    pub(crate) fn commit_finished(&mut self, result: Result<(), String>) {
        self.commit.phase = CommitPhase::Editing;
        match result {
            Ok(()) => {
                self.commit = CommitDialog::default();
            }
            Err(error) => self.commit.notice = Some(error),
        }
    }

    pub(super) fn commit_key(&mut self, key: KeyEvent, bindings: &Bindings) -> Effects {
        if self.commit.running() || key.kind == KeyEventKind::Release {
            return Effects::default();
        }
        if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
            return Effects {
                redraw: true,
                submit_commit: key.kind == KeyEventKind::Press,
                ..Effects::default()
            };
        }
        match key.code {
            KeyCode::Esc => return self.commit_action(Action::Cancel),
            KeyCode::Tab | KeyCode::BackTab => return self.commit_action(Action::Focus),
            KeyCode::Enter => return self.commit_action(Action::Accept),
            KeyCode::Up => return self.commit_action(Action::Up),
            KeyCode::Down => return self.commit_action(Action::Down),
            _ => {}
        }
        if let Some(action) = bindings.action(Scope::Input, key) {
            return self.commit_action(action);
        }
        if !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && let KeyCode::Char(ch) = key.code
        {
            return self.commit_paste(ch.encode_utf8(&mut [0; 4]));
        }
        Effects::default()
    }

    pub(super) fn commit_paste(&mut self, text: &str) -> Effects {
        if self.commit.running() {
            return Effects::default();
        }
        match self.commit.field {
            CommitField::Subject => {
                self.commit.subject.insert(text);
            }
            CommitField::Body => self.commit.body.insert(text),
        }
        self.commit.notice = None;
        Effects::draw(false)
    }

    pub(super) fn commit_action(&mut self, action: Action) -> Effects {
        if self.commit.running() {
            return Effects::default();
        }
        match action {
            Action::Cancel => self.commit.phase = CommitPhase::Closed,
            Action::Focus => {
                self.commit.field = match self.commit.field {
                    CommitField::Subject => CommitField::Body,
                    CommitField::Body => CommitField::Subject,
                };
            }
            Action::Accept => match self.commit.field {
                CommitField::Subject => self.commit.field = CommitField::Body,
                CommitField::Body => return self.commit_paste("\n"),
            },
            action => match self.commit.field {
                CommitField::Subject => {
                    self.commit.subject.edit(action);
                }
                CommitField::Body => self.commit.body.edit(action),
            },
        }
        Effects::draw(false)
    }
}

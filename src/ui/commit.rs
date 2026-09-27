mod process;

use super::{
    modal,
    terminal::{self, TerminalError},
};
use crate::{
    input::{Editor, fixed_key_label},
    render::Palette,
    state::{CommitField, State},
};
use process::Process;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        event::{self, KeyCode, KeyEventKind, KeyModifiers},
        execute,
        style::{Attribute, ResetColor, SetAttribute},
        terminal::{LeaveAlternateScreen, enable_raw_mode, size},
    },
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};
use unicode_width::UnicodeWidthStr;

pub(super) const MIN_WIDTH: u16 = 40;
pub(super) const MIN_HEIGHT: u16 = 14;

pub(super) struct Session {
    process: Process,
}

impl Session {
    pub(super) fn start(root: &Path, message: &str, columns: u16, rows: u16) -> io::Result<Self> {
        Process::start(root, message, columns, rows).map(|process| Self { process })
    }

    /// Returns a result once Git completes, handing off only when necessary.
    pub(super) fn update(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> Result<Option<Result<(), String>>, TerminalError> {
        let mut progress = self.process.poll().unwrap_or_else(|error| {
            Some(Err(format!("Cannot communicate with git commit: {error}")))
        });
        // Output collection may straddle the deadline. Recheck exit immediately
        // before handoff so an already completed commit stays inside dift.
        if progress.is_none() && self.process.handoff_due() {
            progress = self.process.poll().unwrap_or_else(|error| {
                Some(Err(format!("Cannot communicate with git commit: {error}")))
            });
        }
        if matches!(progress, Some(Ok(()))) {
            return Ok(progress);
        }
        if progress.is_some() || self.process.handoff_due() {
            return self.foreground(terminal, progress).map(Some);
        }
        Ok(None)
    }

    pub(super) fn resize(&mut self, columns: u16, rows: u16) -> io::Result<()> {
        self.process.resize(columns, rows)
    }

    fn foreground(
        &mut self,
        terminal: &mut DefaultTerminal,
        completed: Option<Result<(), String>>,
    ) -> Result<Result<(), String>, TerminalError> {
        // Discard keys queued while the dialog was busy before switching the
        // input owner from Crossterm to the raw relay on Unix.
        while event::poll(Duration::ZERO).map_err(TerminalError::Suspend)? {
            let _ = event::read().map_err(TerminalError::Suspend)?;
        }
        terminal::with_suspended(terminal, || {
            let result = (|| {
                enable_raw_mode()?;
                io::stdout().write_all(b"\r\n")?;
                self.process.show_output()?;
                let result = match completed {
                    Some(result) => result,
                    None => self.relay()?,
                };
                reset_display()?;
                if let Err(error) = &result {
                    acknowledge_failure(error)?;
                }
                Ok(result)
            })();
            // Attempt cleanup even when replay, relay, or acknowledgement failed.
            finish_handoff(result, reset_display())
        })
    }

    fn relay(&mut self) -> io::Result<Result<(), String>> {
        loop {
            let (columns, rows) = size()?;
            self.process.resize(columns, rows)?;
            let completed = self.process.poll()?;
            self.process.show_output()?;
            if let Some(result) = completed {
                return Ok(result);
            }
            self.process.read_input()?;
        }
    }
}

pub(super) fn show_failure(
    terminal: &mut DefaultTerminal,
    error: &str,
) -> Result<(), TerminalError> {
    terminal::with_suspended(terminal, || {
        enable_raw_mode()
            .and_then(|()| acknowledge_failure(error))
            .map_err(TerminalError::Handoff)
    })
}

fn finish_handoff<T>(result: io::Result<T>, reset: io::Result<()>) -> Result<T, TerminalError> {
    reset.map_err(TerminalError::Handoff)?;
    result.map_err(TerminalError::Handoff)
}

fn reset_display() -> io::Result<()> {
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
    // Child applications may have changed cursor/keypad and input modes.
    io::stdout().write_all(
        b"\x1b[?1l\x1b>\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?25h",
    )?;
    io::stdout().flush()
}

fn acknowledge_failure(error: &str) -> io::Result<()> {
    write!(
        io::stdout(),
        "\r\n{error}\r\nPress Enter to return to the commit message.\r\n"
    )?;
    io::stdout().flush()?;
    loop {
        if let event::Event::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
            && key.code == KeyCode::Enter
        {
            return Ok(());
        }
    }
}

pub(super) fn draw(frame: &mut Frame, state: &State, palette: &Palette) {
    let Some(dialog) = state.commit_dialog() else {
        return;
    };
    let area = frame.area();
    let controls = format!(
        "{} commit · {} switch field · {} close",
        fixed_key_label(KeyCode::Char('s'), KeyModifiers::CONTROL),
        fixed_key_label(KeyCode::Tab, KeyModifiers::NONE),
        fixed_key_label(KeyCode::Esc, KeyModifiers::NONE),
    );
    let width = [
        modal::widest(dialog.subject.text()) + 2,
        modal::widest(dialog.body.editor().text()) + 2,
        controls.width(),
        " Body (optional) ".width(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let style = palette.base().bg(palette.panel);
    let footer = if dialog.running() {
        Line::styled("Committing…", style.fg(palette.accent))
    } else if let Some(notice) = &dialog.notice {
        Line::styled(
            notice.escape_debug().to_string(),
            style.fg(palette.removed_emphasis),
        )
    } else {
        modal::hint(controls, palette)
    };
    let height = 5 + dialog.body.editor().text().split('\n').count().max(1);
    let modal = modal::Modal::commit(area, width, height, vec![footer]);
    let inner = modal.draw(frame, " Commit ", palette);
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT || inner.height < 6 {
        let message = if dialog.running() {
            "Committing…".to_owned()
        } else {
            format!(
                "Enlarge terminal.\n{} closes.",
                fixed_key_label(KeyCode::Esc, KeyModifiers::NONE)
            )
        };
        frame.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .style(style),
            inner,
        );
        return;
    }
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).split(inner);
    field(
        frame,
        rows[0],
        " Subject ",
        &dialog.subject,
        !dialog.running() && dialog.field == CommitField::Subject,
        palette,
    );
    field(
        frame,
        rows[1],
        " Body (optional) ",
        dialog.body.editor(),
        !dialog.running() && dialog.field == CommitField::Body,
        palette,
    );
}

fn field(
    frame: &mut Frame,
    rect: Rect,
    title: &'static str,
    editor: &Editor,
    focused: bool,
    palette: &Palette,
) {
    let style = palette.base().bg(palette.panel);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(style)
        .border_style(style.fg(if focused {
            palette.accent
        } else {
            palette.muted
        }));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.is_empty() {
        return;
    }
    let before = &editor.text()[..editor.cursor()];
    let row = before.bytes().filter(|byte| *byte == b'\n').count();
    let column = before.rsplit('\n').next().unwrap_or("").width();
    let top = row.saturating_sub(usize::from(inner.height) - 1);
    let left = column.saturating_sub(usize::from(inner.width) - 1);
    let lines: Vec<_> = editor
        .text()
        .split('\n')
        .skip(top)
        .take(usize::from(inner.height))
        .map(Line::raw)
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .style(style)
            .scroll((0, left.min(usize::from(u16::MAX)) as u16)),
        inner,
    );
    if focused {
        frame.set_cursor_position((
            inner.x + (column - left) as u16,
            inner.y + (row - top) as u16,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_failure_is_a_terminal_error_after_git_succeeds() {
        // Given
        let git_result: io::Result<Result<(), String>> = Ok(Ok(()));
        let cleanup = Err(io::Error::other("display reset failed"));
        // When
        let result = finish_handoff(git_result, cleanup);
        // Then
        assert!(matches!(result, Err(TerminalError::Handoff(_))));
    }

    #[test]
    fn git_failure_remains_a_git_result_after_cleanup() -> Result<(), TerminalError> {
        // Given
        let git_result = Ok(Err::<(), _>("Hook rejected the commit".to_owned()));
        // When
        let result = finish_handoff(git_result, Ok(()));
        // Then
        assert_eq!(result?, Err("Hook rejected the commit".to_owned()));
        Ok(())
    }
}

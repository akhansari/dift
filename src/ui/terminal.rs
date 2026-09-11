use ratatui::{
    DefaultTerminal,
    crossterm::{
        event::{
            DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        },
        execute,
        terminal::{EnterAlternateScreen, enable_raw_mode},
    },
};
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum TerminalError {
    #[error("Suspend dift terminal: {0}")]
    Suspend(#[source] io::Error),
    #[error("Terminal handoff: {0}")]
    Handoff(#[source] io::Error),
    #[error("Resume dift terminal: {0}")]
    Resume(#[source] io::Error),
}

/// Restore the application terminal even when the handed-off operation fails.
pub(super) fn with_suspended<T>(
    terminal: &mut DefaultTerminal,
    operation: impl FnOnce() -> Result<T, TerminalError>,
) -> Result<T, TerminalError> {
    suspend(terminal).map_err(TerminalError::Suspend)?;
    let result = operation();
    resume(terminal).map_err(TerminalError::Resume)?;
    result
}

fn suspend(terminal: &mut DefaultTerminal) -> io::Result<()> {
    execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste)?;
    terminal.show_cursor()?;
    ratatui::try_restore()
}

fn resume(terminal: &mut DefaultTerminal) -> io::Result<()> {
    // Restore modes directly so repeated launches do not accumulate panic hooks.
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    terminal.hide_cursor()?;
    // A fullscreen resize resets the back buffer without querying the cursor position.
    terminal.resize(terminal.size()?.into())
}

use super::*;
use crate::state::Command;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

fn draft(subject: &str, body: &str) -> State {
    let mut state = State::new(false);
    state.begin_commit();
    state.paste(subject, Instant::now());
    state.action(Action::Accept, 20, Instant::now());
    state.paste(body, Instant::now());
    state
}

fn key(state: &mut State, code: KeyCode) -> Effects {
    state.handle_key(
        KeyEvent::from(code),
        &Bindings::defaults(),
        20,
        Instant::now(),
    )
}

#[test]
fn subject_and_multiline_body_form_a_git_message() {
    // Given
    let mut state = draft(
        "  Explain the change  ",
        "First paragraph\r\n\r\nSecond paragraph",
    );
    // When
    let message = state.commit_message();
    // Then
    assert_eq!(
        message.as_deref(),
        Some("Explain the change\n\nFirst paragraph\n\nSecond paragraph")
    );
}

#[test]
fn blank_subject_is_rejected_even_with_a_body() {
    // Given
    let mut state = draft("   ", "A body alone");
    // When
    let message = state.commit_message();
    // Then
    assert_eq!(message, None);
    assert_eq!(
        state
            .commit_dialog()
            .and_then(|dialog| dialog.notice.as_deref()),
        Some("Enter a commit subject")
    );
}

#[test]
fn closing_and_reopening_restores_the_message() {
    // Given
    let mut state = draft("Retained subject", "Retained body");
    // When
    key(&mut state, KeyCode::Esc);
    state.begin_commit();
    // Then
    assert_eq!(
        state.commit_message().as_deref(),
        Some("Retained subject\n\nRetained body")
    );
}

#[test]
fn successful_commit_clears_the_draft() {
    // Given
    let mut state = draft("Old subject", "Old body");
    // When
    state.commit_finished(Ok(()));
    // Then
    assert!(state.commit_dialog().is_none());
    assert_eq!(state.toast(), None);
    state.begin_commit();
    assert_eq!(state.commit_message(), None);
    assert_eq!(
        state
            .commit_dialog()
            .map(|dialog| dialog.body.editor().text()),
        Some("")
    );
}

#[test]
fn unsuccessful_commit_preserves_the_message() {
    // Given
    let mut state = draft("Try again", "Keep this body");
    // When
    state.commit_finished(Err("Hook rejected the commit".into()));
    // Then
    assert_eq!(
        state.commit_message().as_deref(),
        Some("Try again\n\nKeep this body")
    );
    assert_eq!(
        state
            .commit_dialog()
            .and_then(|dialog| dialog.notice.as_deref()),
        Some("Hook rejected the commit")
    );
}

#[test]
fn letters_and_spaces_in_the_dialog_do_not_trigger_app_shortcuts() {
    // Given
    let mut state = draft("Message", "");
    // When
    let quit = key(&mut state, KeyCode::Char('q'));
    let stage = key(&mut state, KeyCode::Char(' '));
    let discard = key(&mut state, KeyCode::Char('D'));
    let commit = key(&mut state, KeyCode::Char('c'));
    // Then
    assert_eq!(quit.command, None);
    assert_eq!(stage.command, None);
    assert_eq!(discard.command, None);
    assert_eq!(commit.command, None);
    assert_eq!(state.commit_message().as_deref(), Some("Message\n\nq Dc"));
}

#[test]
fn enter_adds_a_body_line_without_submitting() {
    // Given
    let mut state = draft("Subject", "First");
    // When
    let effects = key(&mut state, KeyCode::Enter);
    state.paste("Second", Instant::now());
    // Then
    assert_eq!(effects.command, None);
    assert_eq!(
        state.commit_message().as_deref(),
        Some("Subject\n\nFirst\nSecond")
    );
}

#[test]
fn control_s_submits_from_the_subject_field() {
    // Given
    let mut state = draft("Subject", "Body");
    key(&mut state, KeyCode::Tab);
    // When
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(effects.command, Some(Command::SubmitCommit));
    assert!(!effects.render);
}

#[test]
fn repeated_control_s_does_not_submit() {
    // Given
    let mut state = draft("Subject", "");
    let key = KeyEvent::new_with_kind(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
        KeyEventKind::Repeat,
    );
    // When
    let effects = state.handle_key(key, &Bindings::defaults(), 20, Instant::now());
    // Then
    assert_eq!(effects.command, None);
}

#[test]
fn body_editing_preserves_unicode_and_restores_column_across_short_lines() {
    // Given
    let mut state = draft("Subject", "é界x\na\né界x");
    // When
    key(&mut state, KeyCode::Up);
    key(&mut state, KeyCode::Up);
    key(&mut state, KeyCode::Backspace);
    // Then
    assert_eq!(
        state.commit_message().as_deref(),
        Some("Subject\n\né界\na\né界x")
    );
}

#[test]
fn backspace_at_the_start_of_a_body_line_joins_lines() {
    // Given
    let mut state = draft("Subject", "First\nSecond");
    key(&mut state, KeyCode::Home);
    // When
    key(&mut state, KeyCode::Backspace);
    // Then
    assert_eq!(
        state.commit_message().as_deref(),
        Some("Subject\n\nFirstSecond")
    );
}

#[test]
fn commit_shortcut_can_be_rebound_without_affecting_text_input() -> AppResult<()> {
    // Given
    let config = Config::parse("keybindings { global { commit \"x\"; }; }")?;
    let mut state = State::new(false);
    // When
    let effects = state.handle_key(
        KeyEvent::from(KeyCode::Char('x')),
        &config.bindings,
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(effects.command, Some(Command::OpenCommit));
    assert_eq!(
        config
            .bindings
            .action(Scope::Input, KeyEvent::from(KeyCode::Char('x'))),
        None
    );
    Ok(())
}

#[test]
fn running_commit_ignores_edits_close_and_resubmission() {
    // Given
    let mut state = draft("Subject", "Body");
    state.commit_started();
    // When
    key(&mut state, KeyCode::Char('x'));
    key(&mut state, KeyCode::Esc);
    state.paste("unwanted paste", Instant::now());
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(effects.command, None);
    assert!(state.commit_dialog().is_some_and(|dialog| dialog.running()));
    assert_eq!(state.commit_message().as_deref(), Some("Subject\n\nBody"));
}

use super::*;
use crate::{
    AppResult,
    git::{ChangeTarget, DISCARD_MODES, DiscardMode},
    input::Bindings,
    state::Command,
    ui::fixtures::Fixture,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::sync::Arc;
use std::time::Instant;

fn ready() -> AppResult<State> {
    let fixture = Fixture::new("")?;
    fixture.write("tracked.txt", "edit")?;
    let repo = fixture.repo()?;
    let target = ChangeTarget::File("tracked.txt".into());
    let mut state = State::new(false);
    state.begin_discard(target.clone());
    state.discard_prepared(DISCARD_MODES.map(|mode| {
        repo.prepare_discard(&target, mode)
            .map(Arc::new)
            .map_err(|error| error.to_string())
    }));
    Ok(state)
}

#[test]
fn accepting_default_requests_unstaged_discard() -> AppResult<()> {
    // Given
    let mut state = ready()?;
    // When
    let effects = state.action(Action::Accept, 20, Instant::now());
    // Then
    assert_eq!(effects.command, Some(Command::ConfirmDiscard));
    assert_eq!(
        state
            .take_discard_preview()
            .ok_or("missing preview")?
            .mode(),
        DiscardMode::Unstaged
    );
    Ok(())
}

#[test]
fn accepting_all_requests_the_reviewed_mode() -> AppResult<()> {
    // Given
    let mut state = ready()?;
    state.action(Action::Down, 20, Instant::now());
    // When
    let effects = state.action(Action::Accept, 20, Instant::now());
    // Then
    assert_eq!(effects.command, Some(Command::ConfirmDiscard));
    assert_eq!(
        state
            .take_discard_preview()
            .ok_or("missing preview")?
            .mode(),
        DiscardMode::All
    );
    Ok(())
}

#[test]
fn opening_with_only_staged_changes_defaults_to_all() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    fixture.write("tracked.txt", "staged edit")?;
    fixture.git(&["add", "tracked.txt"])?;
    let repo = fixture.repo()?;
    let target = ChangeTarget::File("tracked.txt".into());
    let mut state = State::new(false);
    state.begin_discard(target.clone());
    // When
    state.discard_prepared(DISCARD_MODES.map(|mode| {
        repo.prepare_discard(&target, mode)
            .map(Arc::new)
            .map_err(|error| error.to_string())
    }));
    // Then
    assert_eq!(
        state
            .take_discard_preview()
            .ok_or("missing preview")?
            .mode(),
        DiscardMode::All
    );
    Ok(())
}

#[test]
fn opening_with_no_available_discard_defaults_to_cancel() {
    // Given
    let mut state = State::new(false);
    state.begin_discard(ChangeTarget::Subtree("".into()));
    state.discard_prepared([Err("conflict".into()), Err("conflict".into())]);
    // When
    let effects = state.action(Action::Accept, 20, Instant::now());
    // Then
    assert_eq!(effects.command, None);
    assert!(state.discard_menu().is_none());
}

#[test]
fn repeated_accept_cannot_execute_discard() -> AppResult<()> {
    // Given
    let mut state = ready()?;
    state.action(Action::Down, 20, Instant::now());
    let mut key = KeyEvent::from(KeyCode::Enter);
    key.kind = KeyEventKind::Repeat;
    // When
    let effects = state.handle_key(key, &Bindings::defaults(), 20, Instant::now());
    // Then
    assert_eq!(effects.command, None);
    assert!(state.discard_menu().is_some());
    Ok(())
}

#[test]
fn modal_suppresses_underlying_stage_action() -> AppResult<()> {
    // Given
    let mut state = ready()?;
    // When
    let effects = state.action(Action::ToggleStage, 20, Instant::now());
    // Then
    assert_eq!(effects.command, None);
    assert!(state.discard_menu().is_some());
    Ok(())
}

#[test]
fn escape_cancels_while_preview_is_preparing() {
    // Given
    let mut state = State::new(false);
    state.begin_discard(ChangeTarget::Subtree("".into()));
    // When
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert!(state.discard_menu().is_none());
    assert_eq!(effects.command, None);
}

#[test]
fn unavailable_option_cannot_execute() {
    // Given
    let mut state = State::new(false);
    state.begin_discard(ChangeTarget::Subtree("".into()));
    state.discard_prepared([Err("conflict".into()), Err("conflict".into())]);
    state.action(Action::Up, 20, Instant::now());
    // When
    let effects = state.action(Action::Accept, 20, Instant::now());
    // Then
    assert_eq!(effects.command, None);
    assert!(state.discard_menu().is_some());
}

#[test]
fn stale_preview_resets_confirmation_to_cancel() -> AppResult<()> {
    // Given
    let mut state = ready()?;
    state.action(Action::Down, 20, Instant::now());
    // When
    let target = state.discard_stale();
    let effects = state.action(Action::Accept, 20, Instant::now());
    // Then
    assert_eq!(target, Some(ChangeTarget::File("tracked.txt".into())));
    assert_eq!(effects.command, None);
    assert!(state.discard_menu().is_none());
    Ok(())
}

use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn working_tree_modal_shows_groups_and_keyboard_controls() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::ChangeFilter, 20, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 80, 24)?;
    // Then
    let text = text(&buffer);
    assert!(text.contains("Category"));
    assert!(text.contains("[ ] Unstaged/Untracked"));
    assert!(text.contains("[ ] Staged"));
    assert!(text.contains("[ ] U  Unmerged/conflict"));
    assert!(text.contains("ctrl-r reset selections"));
    assert!(text.contains("enter accept · esc cancel"));
    Ok(())
}

#[test]
fn comparison_modal_shows_only_status_group() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.set_comparison(true);
    state.action(Action::ChangeFilter, 20, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 80, 24)?;
    // Then
    let text = text(&buffer);
    assert!(text.contains("Status"));
    assert!(text.contains("[ ] ?  Untracked"));
    assert!(!text.contains("Category"));
    assert!(!text.contains("Unstaged/Untracked"));
    Ok(())
}

#[test]
fn checkbox_only_filter_explains_empty_tree_and_shows_summary() -> AppResult<()> {
    // Given
    let config = Config::default();
    let mut state = state_with("main.rs", 1);
    state.action(Action::ChangeFilter, 20, Instant::now());
    state.handle_key(
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        &config.bindings,
        20,
        Instant::now(),
    );
    state.handle_key(
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        &config.bindings,
        20,
        Instant::now(),
    );
    state.handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &config.bindings,
        20,
        Instant::now(),
    );
    // When
    let buffer = render(&state, &config, 100, 24)?;
    // Then
    let text = text(&buffer);
    assert!(text.contains("No matching files"));
    assert!(text.contains(" · Staged"));
    Ok(())
}

#[test]
fn small_terminal_explains_how_to_cancel_filters() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.action(Action::ChangeFilter, 20, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 40, 14)?;
    // Then
    assert!(text(&buffer).contains("esc cancels."));
    Ok(())
}

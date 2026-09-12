use super::*;
use crate::{AppResult, input::Bindings};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn changes() -> Snapshot {
    let mut snapshot = snapshot_with(&[
        ("src/mixed.rs", Category::Staged),
        ("src/mixed.rs", Category::Unstaged),
        ("src/new.rs", Category::Untracked),
        ("other.rs", Category::Staged),
    ]);
    snapshot.changes[0].status = 'A';
    snapshot.changes[2].status = '?';
    snapshot
}

fn key(state: &mut State, code: KeyCode, modifiers: KeyModifiers) -> Effects {
    state.handle_key(
        KeyEvent::new(code, modifiers),
        &Bindings::defaults(),
        20,
        Instant::now(),
    )
}

fn press(state: &mut State, code: KeyCode) -> Effects {
    key(state, code, KeyModifiers::NONE)
}

fn opened() -> State {
    let mut state = State::new(false);
    state.update_snapshot(changes());
    key(&mut state, KeyCode::Char('f'), KeyModifiers::CONTROL);
    state
}

fn staged() -> State {
    let mut state = opened();
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Char(' '));
    state
}

#[test]
fn default_filters_include_every_change() {
    // Given
    let state = opened();
    // When
    let selected = state.selected_changes();
    // Then
    assert_eq!(selected, [0, 1, 2, 3]);
    assert!(!state.change_filter().selected());
    assert!(state.filter_menu().is_some());
}

#[test]
fn category_and_status_must_match_the_same_change() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Tab);
    // When
    let effects = press(&mut state, KeyCode::Char(' '));
    // Then
    assert!(effects.render);
    assert_eq!(state.selected_changes(), [3]);
    assert_eq!(paths(&state), [Path::new(""), Path::new("other.rs")]);
}

#[test]
fn staged_filter_updates_badges_counts_and_directory_diff() -> AppResult<()> {
    // Given
    let mut state = opened();
    press(&mut state, KeyCode::Down);
    // When
    press(&mut state, KeyCode::Char(' '));
    // Then
    assert_eq!(state.selected_changes(), [0, 3]);
    assert_eq!(state.tree(false)[0].count, 2);
    let mixed = state
        .tree(false)
        .iter()
        .find(|row| row.path == Path::new("src/mixed.rs"))
        .ok_or("Missing mixed path")?;
    assert_eq!(
        mixed.badges,
        [TreeBadge {
            category: Category::Staged,
            status: 'A'
        }]
    );
    Ok(())
}

#[test]
fn selecting_both_categories_includes_all_changes() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Up);
    // When
    press(&mut state, KeyCode::Char(' '));
    // Then
    assert_eq!(state.selected_changes(), [0, 1, 2, 3]);
    assert_eq!(state.change_filter().summary(), "Both categories");
}

#[test]
fn selected_statuses_are_combined_with_or() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Tab);
    press(&mut state, KeyCode::Char(' '));
    press(&mut state, KeyCode::Down);
    // When
    press(&mut state, KeyCode::Char(' '));
    // Then
    assert_eq!(state.selected_changes(), [0, 3]);
    assert_eq!(state.change_filter().summary(), "Staged · M,A");
}

#[test]
fn all_statuses_allow_even_unknown_statuses() {
    // Given
    let mut state = opened();
    state.change_filter.statuses = [true; 8];
    let mut snapshot = changes();
    snapshot.changes[0].status = 'X';
    // When
    state.update_snapshot(snapshot);
    // Then
    assert_eq!(state.selected_changes(), [0, 1, 2, 3]);
    assert_eq!(state.change_filter().summary(), "All statuses");
}

#[test]
fn cancel_restores_selection_and_collapsed_directories() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(changes());
    state.action(Action::Down, 20, Instant::now());
    state.action(Action::Left, 20, Instant::now());
    key(&mut state, KeyCode::Char('f'), KeyModifiers::CONTROL);
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Char(' '));
    press(&mut state, KeyCode::Tab);
    press(&mut state, KeyCode::Char(' '));
    // When
    press(&mut state, KeyCode::Esc);
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert!(state.is_collapsed(Path::new("src")));
    assert!(!state.change_filter().selected());
    assert!(state.filter_menu().is_none());
    assert_eq!(state.selected_changes(), [0, 1, 2]);
}

#[test]
fn accepted_filters_survive_refresh() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Enter);
    // When
    state.update_snapshot(changes());
    // Then
    assert_eq!(state.selected_changes(), [0, 3]);
    assert!(state.filter_menu().is_none());
}

#[test]
fn text_filter_is_anded_with_checkbox_filters() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Enter);
    state.action(Action::Filter, 20, Instant::now());
    state.paste("src", Instant::now());
    // When
    state.action(Action::Accept, 20, Instant::now());
    // Then
    assert_eq!(state.selected_changes(), [0]);
}

#[test]
fn reset_checkboxes_preserves_text_filter() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Enter);
    state.action(Action::Filter, 20, Instant::now());
    state.paste("src", Instant::now());
    state.action(Action::Accept, 20, Instant::now());
    key(&mut state, KeyCode::Char('f'), KeyModifiers::CONTROL);
    // When
    key(&mut state, KeyCode::Char('r'), KeyModifiers::CONTROL);
    // Then
    assert_eq!(state.filter(), "src");
    assert_eq!(state.selected_changes(), [0, 1, 2]);
}

#[test]
fn escape_outside_modal_clears_text_and_checkboxes() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Enter);
    state.action(Action::Filter, 20, Instant::now());
    state.paste("src", Instant::now());
    state.action(Action::Accept, 20, Instant::now());
    // When
    press(&mut state, KeyCode::Esc);
    // Then
    assert_eq!(state.filter(), "");
    assert!(!state.change_filter().selected());
    assert_eq!(state.selected_changes(), [0, 1, 2, 3]);
}

#[test]
fn comparison_modal_starts_with_statuses() {
    // Given
    let mut state = State::new(false);
    state.set_comparison(true);
    let mut snapshot = snapshot_with(&[
        ("a.rs", Category::Comparison),
        ("b.rs", Category::Comparison),
    ]);
    snapshot.changes[1].status = 'A';
    state.update_snapshot(snapshot);
    key(&mut state, KeyCode::Char('f'), KeyModifiers::CONTROL);
    // When
    press(&mut state, KeyCode::Char(' '));
    // Then
    assert_eq!(state.selected_changes(), [0]);
    assert_eq!(state.change_filter().summary(), "M");
}

#[test]
fn incompatible_filters_keep_only_the_root() {
    // Given
    let mut state = staged();
    press(&mut state, KeyCode::Tab);
    press(&mut state, KeyCode::BackTab);
    press(&mut state, KeyCode::Up);
    // When
    press(&mut state, KeyCode::Char(' '));
    // Then
    assert!(state.selected_changes().is_empty());
    assert_eq!(paths(&state), [Path::new("")]);
    assert_eq!(state.tree(false)[0].count, 0);
}

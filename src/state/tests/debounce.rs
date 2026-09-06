use super::*;
use crate::input::Bindings;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn editing_filter(now: Instant) -> State {
    let mut state = state();
    state.action(Action::NextFile, 10, now);
    state.action(Action::Filter, 10, now);
    state
}

#[test]
fn typing_redraws_the_prompt_without_filtering_or_rendering() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    // When
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        &Bindings::defaults(),
        10,
        now,
    );
    // Then
    assert_eq!(
        effects,
        Effects {
            redraw: true,
            ..Effects::default()
        }
    );
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("t")
    );
    assert_eq!(state.filter(), "");
    assert_eq!(state.matching_paths().len(), 3);
    assert_eq!(state.selection(), Path::new("src/a.rs"));
    assert_eq!(state.selected_changes(), [0, 1]);
}

#[test]
fn the_filter_waits_for_the_full_delay() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(299));
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(state.filter(), "");
    assert_eq!(state.matching_paths().len(), 3);
}

#[test]
fn a_pasted_filter_applies_at_the_deadline_and_requests_a_render() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests\n", now);
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(300));
    // Then
    assert_eq!(
        effects,
        Effects {
            redraw: true,
            render: true,
            ..Effects::default()
        }
    );
    assert_eq!(state.filter(), "tests");
    assert_eq!(state.matching_paths().len(), 1);
    assert_eq!(state.selection(), Path::new(""));
    assert_eq!(state.selected_changes(), [3]);
    assert_eq!(state.filter_delay(now + Duration::from_millis(300)), None);
}

#[test]
fn further_typing_postpones_the_filter() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("t", now);
    state.paste("ests", now + Duration::from_millis(200));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(300));
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(state.filter(), "");
    assert_eq!(
        state.filter_delay(now + Duration::from_millis(300)),
        Some(Duration::from_millis(200))
    );
}

#[test]
fn only_the_latest_text_is_applied_after_a_burst() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("t", now);
    state.paste("ests", now + Duration::from_millis(200));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(500));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "tests");
    assert_eq!(state.selected_changes(), [3]);
}

#[test]
fn an_applied_filter_does_not_request_another_render_on_later_ticks() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(600));
    // Then
    assert_eq!(effects, Effects::default());
}

#[test]
fn cursor_movement_does_not_postpone_filtering() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.action(Action::CursorLeft, 10, now + Duration::from_millis(200));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(300));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "tests");
}

#[test]
fn deleting_at_the_end_does_not_postpone_filtering() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.action(Action::Delete, 10, now + Duration::from_millis(200));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(300));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "tests");
}

#[test]
fn pasting_only_control_characters_does_not_postpone_filtering() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.paste("\n\t", now + Duration::from_millis(200));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(300));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "tests");
}

#[test]
fn deleting_text_restarts_the_delay() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("testsx", now);
    // When
    let effects = state.action(Action::Backspace, 10, now + Duration::from_millis(200));
    // Then
    assert_eq!(
        effects,
        Effects {
            redraw: true,
            ..Effects::default()
        }
    );
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("tests")
    );
    assert_eq!(state.filter(), "");
    assert_eq!(
        state.filter_delay(now + Duration::from_millis(200)),
        Some(Duration::from_millis(300))
    );
}

#[test]
fn clearing_the_draft_waits_before_restoring_all_paths() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    // When
    let effects = state.action(Action::ClearBefore, 10, now + Duration::from_millis(400));
    // Then
    assert!(!effects.render);
    assert_eq!(state.filter(), "tests");
    assert_eq!(state.matching_paths().len(), 1);
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("")
    );
    assert_eq!(
        state.filter_delay(now + Duration::from_millis(400)),
        Some(Duration::from_millis(300))
    );
}

#[test]
fn a_cleared_draft_restores_all_paths_when_due() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    state.action(Action::ClearBefore, 10, now + Duration::from_millis(400));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(700));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "");
    assert_eq!(state.matching_paths().len(), 3);
    assert_eq!(state.selected_changes(), [0, 1, 2, 3]);
}

#[test]
fn returning_to_the_applied_query_cancels_pending_work() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    state.paste("x", now + Duration::from_millis(400));
    // When
    let effects = state.action(Action::Backspace, 10, now + Duration::from_millis(500));
    // Then
    assert!(!effects.render);
    assert_eq!(state.filter(), "tests");
    assert_eq!(state.filter_delay(now + Duration::from_millis(500)), None);
}

#[test]
fn enter_applies_pending_text_immediately_and_closes_the_prompt() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    // When
    let effects = state.action(Action::Accept, 10, now + Duration::from_millis(50));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "tests");
    assert_eq!(state.selected_changes(), [3]);
    assert!(state.prompt().is_none());
    assert_eq!(state.filter_delay(now + Duration::from_millis(50)), None);
}

#[test]
fn enter_after_the_filter_applies_does_not_repeat_the_render() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    // When
    let effects = state.action(Action::Accept, 10, now + Duration::from_millis(400));
    // Then
    assert!(!effects.render);
    assert_eq!(state.filter(), "tests");
    assert!(state.prompt().is_none());
}

#[test]
fn escape_before_the_deadline_discards_the_draft_without_rendering() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    // When
    let effects = state.action(Action::Cancel, 10, now + Duration::from_millis(50));
    // Then
    assert!(!effects.render);
    assert_eq!(state.filter(), "");
    assert_eq!(state.selection(), Path::new("src/a.rs"));
    assert_eq!(state.selected_changes(), [0, 1]);
    assert!(state.prompt().is_none());
    assert_eq!(state.filter_delay(now + Duration::from_millis(50)), None);
}

#[test]
fn escape_restores_the_original_filter_and_selection_after_a_preview() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("src", now);
    state.action(Action::Accept, 10, now);
    state.action(Action::Filter, 10, now);
    state.action(Action::ClearBefore, 10, now);
    state.paste("tests", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    state.paste("x", now + Duration::from_millis(400));
    // When
    let effects = state.action(Action::Cancel, 10, now + Duration::from_millis(500));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "src");
    assert_eq!(state.selection(), Path::new("src/a.rs"));
    assert_eq!(state.matching_paths().len(), 2);
    assert_eq!(state.selected_changes(), [0, 1]);
    assert!(state.prompt().is_none());
    assert_eq!(state.filter_delay(now + Duration::from_millis(500)), None);
}

#[test]
fn cancelled_edits_do_not_leak_into_a_new_prompt() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.action(Action::Cancel, 10, now + Duration::from_millis(50));
    state.action(Action::Filter, 10, now + Duration::from_millis(200));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(300));
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(state.filter(), "");
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("")
    );
}

#[test]
fn a_refresh_uses_the_applied_filter_and_preserves_the_pending_edit() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("src", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    state.action(Action::ClearBefore, 10, now + Duration::from_millis(400));
    state.paste("tests", now + Duration::from_millis(400));
    // When
    state.collection_finished(Ok(snapshot_with(&[
        ("src/new.rs", Category::Untracked),
        ("tests/new.rs", Category::Staged),
    ])));
    // Then
    assert_eq!(state.filter(), "src");
    assert_eq!(
        paths(&state),
        [Path::new(""), Path::new("src"), Path::new("src/new.rs")]
    );
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("tests")
    );
    assert_eq!(
        state.filter_delay(now + Duration::from_millis(400)),
        Some(Duration::from_millis(300))
    );
}

#[test]
fn a_pending_filter_applies_to_the_latest_snapshot() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.collection_finished(Ok(snapshot_with(&[
        ("src/new.rs", Category::Untracked),
        ("tests/new.rs", Category::Staged),
    ])));
    // When
    let effects = state.apply_pending_filter(now + Duration::from_millis(300));
    // Then
    assert!(effects.render);
    assert_eq!(state.filter(), "tests");
    assert_eq!(
        paths(&state),
        [Path::new(""), Path::new("tests"), Path::new("tests/new.rs")]
    );
    assert_eq!(state.selected_changes(), [1]);
}

#[test]
fn cancellation_reconciles_the_original_selection_with_a_refreshed_snapshot() {
    // Given
    let now = Instant::now();
    let mut state = editing_filter(now);
    state.paste("tests", now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    state.collection_finished(Ok(snapshot_with(&[("src/new.rs", Category::Untracked)])));
    // When
    state.action(Action::Cancel, 10, now + Duration::from_millis(400));
    // Then
    assert_eq!(state.filter(), "");
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(state.selected_changes(), [0]);
}

#[test]
fn search_remains_immediate_and_never_schedules_filtering() {
    // Given
    let now = Instant::now();
    let mut state = state();
    state.action(Action::Search, 10, now);
    // When
    let effects = state.paste("needle", now);
    // Then
    assert!(effects.render);
    assert_eq!(state.search(), "needle");
    assert_eq!(state.filter_delay(now), None);
}

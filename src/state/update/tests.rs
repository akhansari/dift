use super::*;
use crate::{
    model::Category,
    render::{Anchor, Document, Hit, Row},
};
use ratatui::text::Line;

fn document(lines: &[&str], partial: bool) -> Arc<Document> {
    Arc::new(Document {
        rows: lines
            .iter()
            .enumerate()
            .map(|(index, text)| {
                Arc::new(Row {
                    line: Line::raw((*text).to_owned()),
                    anchors: vec![Anchor {
                        path: Arc::new("file.txt".into()),
                        category: Category::Unstaged,
                        old: false,
                        number: index + 1,
                        byte: 0,
                        text: Arc::from(*text),
                    }],
                })
            })
            .collect(),
        hits: lines
            .iter()
            .enumerate()
            .filter(|(_, text)| text.contains("two"))
            .map(|(row, _)| Hit { row })
            .collect(),
        partial,
        ..Document::default()
    })
}

fn reading_state() -> State {
    let mut state = State::new(false);
    state.document = document(&["one", "two", "three"], false);
    state.generation = 2;
    state.rendering = true;
    state.search = "two".into();
    state.match_index = Some(0);
    state
}

#[test]
fn a_late_prefix_preview_preserves_the_users_new_scroll_position() {
    // Given
    let mut state = reading_state();
    state.displayed.query = "two".into();
    let result = RenderResult {
        generation: 2,
        document: document(&["one"], true),
        finished: false,
    };
    state.scroll_by(1, true, 1);
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(!changed);
    assert_eq!(state.document.rows[state.scroll].line.to_string(), "two");
    assert_eq!(state.document.rows.len(), 3);
    assert_eq!(state.match_index, Some(0));
    assert_eq!(state.document.hits.len(), 1);
    assert!(state.rendering);
}

#[test]
fn a_complete_refresh_updates_the_document_at_the_current_source_position() {
    // Given
    let mut state = reading_state();
    state.scroll_by(1, true, 1);
    state.displayed.query = "two".into();
    let result = RenderResult {
        generation: 2,
        document: document(&["inserted", "one", "two", "three"], false),
        finished: true,
    };
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(changed);
    assert_eq!(state.document.rows[state.scroll].line.to_string(), "two");
    assert_eq!(state.scroll, 2);
    assert_eq!(state.document.hits[0].row, 2);
    assert!(!state.rendering);
}

#[test]
fn an_obsolete_render_cannot_replace_the_current_document_or_search_results() {
    // Given
    let mut state = reading_state();
    state.scroll_by(1, true, 1);
    let result = RenderResult {
        generation: 1,
        document: document(&["obsolete"], false),
        finished: true,
    };
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(!changed);
    assert_eq!(state.document.rows[state.scroll].line.to_string(), "two");
    assert_eq!(state.document.hits.len(), 1);
    assert_eq!(state.match_index, Some(0));
    assert!(state.rendering);
}

#[test]
fn a_new_selection_can_display_its_preview_immediately() {
    // Given
    let mut state = reading_state();
    state.scroll_by(1, true, 1);
    state.selection = "different.txt".into();
    let result = RenderResult {
        generation: 2,
        document: document(&["new selection"], true),
        finished: false,
    };
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(changed);
    assert_eq!(state.scroll, 0);
    assert_eq!(state.document.rows[0].line.to_string(), "new selection");
    assert_eq!(state.match_index, None);
}

#[test]
fn moving_the_prompt_cursor_redraws_without_rendering_the_diff() {
    // Given
    let mut state = reading_state();
    state.action(Action::Search, 10);
    // When
    let effects = state.action(Action::CursorLeft, 10);
    // Then
    assert_eq!(effects, Effects::draw(false));
    assert_eq!(state.search, "two");
}

#[test]
fn pasting_a_query_requests_a_new_render() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Search, 10);
    // When
    let effects = state.paste("needle\n");
    // Then
    assert_eq!(effects, Effects::draw(true));
    assert_eq!(state.search, "needle");
}

#[test]
fn cancelling_a_changed_query_renders_the_restored_query() {
    // Given
    let mut state = reading_state();
    state.action(Action::Search, 10);
    state.paste(" more");
    // When
    let effects = state.action(Action::Cancel, 10);
    // Then
    assert_eq!(effects, Effects::draw(true));
    assert_eq!(state.search, "two");
    assert!(state.prompt.is_none());
}

#[test]
fn the_normal_quit_key_is_text_while_editing() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Search, 10);
    // When
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        &Bindings::defaults(),
        10,
    );
    // Then
    assert_eq!(effects, Effects::draw(true));
    assert_eq!(state.search, "q");
}

#[test]
fn manual_refresh_requests_collection_without_replacing_the_document() {
    // Given
    let mut state = reading_state();
    // When
    let effects = state.action(Action::Refresh, 10);
    // Then
    assert!(effects.collect);
    assert!(!effects.render);
    assert_eq!(state.document.rows.len(), 3);
}

#[test]
fn a_failed_collection_retains_the_document_and_allows_retry() {
    // Given
    let mut state = reading_state();
    state.collection_started();
    // When
    let effects =
        state.collection_finished(Err(GitError::Command("Cannot read repository".into())));
    // Then
    assert_eq!(effects, Effects::draw(false));
    assert_eq!(state.error.as_deref(), Some("Cannot read repository"));
    assert!(!state.collecting);
    assert_eq!(state.document.rows.len(), 3);
}

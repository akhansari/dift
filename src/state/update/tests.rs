use super::*;
use crate::config::Whitespace;
use crate::{
    model::{Category, Change},
    render::{Anchor, Document, Hit, Row},
};
use ratatui::text::Line;
mod activity;
mod toasts;

#[test]
fn obsolete_collections_cannot_replace_the_current_totals() {
    // Given
    let mut state = State::new(false);
    let current = crate::model::LineStats {
        added: 8,
        removed: 3,
    };
    state.update_snapshot(Snapshot {
        tracked_stats: Some(current),
        ..Snapshot::default()
    });
    let obsolete = state.collection_request();
    state.collection_request();
    // When
    let effects = state.collection_finished(
        obsolete.generation,
        Ok(Snapshot {
            tracked_stats: Some(crate::model::LineStats::default()),
            ..Snapshot::default()
        }),
        Instant::now(),
    );
    // Then
    assert_eq!(state.snapshot().tracked_stats, Some(current));
    assert!(!effects.redraw);
}

#[test]
fn a_successful_collection_publishes_new_totals() {
    // Given
    let mut state = State::new(false);
    let request = state.collection_request();
    let stats = crate::model::LineStats {
        added: 8,
        removed: 3,
    };
    // When
    let effects = state.collection_finished(
        request.generation,
        Ok(Snapshot {
            tracked_stats: Some(stats),
            ..Snapshot::default()
        }),
        Instant::now(),
    );
    // Then
    assert_eq!(state.snapshot().tracked_stats, Some(stats));
    assert!(effects.redraw);
    assert!(!state.collecting());
}

#[test]
fn whitespace_toggle_requests_ignore_changes_from_the_exact_default() {
    // Given
    let mut state = State::new(false);
    // When
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
        &Bindings::defaults(),
        10,
        Instant::now(),
    );
    // Then
    assert!(effects.collect);
    assert!(!effects.render);
    assert_eq!(state.requested_whitespace(), Whitespace::IgnoreChanges);
    assert_eq!(state.snapshot().whitespace, Whitespace::Exact);
    assert_eq!(state.toast(), None);
}

#[test]
fn a_configured_ignore_all_mode_can_be_restored_after_exact_comparison() {
    // Given
    let mut state = State::with_whitespace(false, Whitespace::IgnoreAll);
    state.action(Action::Whitespace, 10, Instant::now());
    // When
    state.action(Action::Whitespace, 10, Instant::now());
    // Then
    assert_eq!(state.requested_whitespace(), Whitespace::IgnoreAll);
}

#[test]
fn whitespace_toggle_returns_to_exact_without_changing_the_configured_preference() {
    // Given
    let mut state = State::with_whitespace(false, Whitespace::IgnoreAll);
    // When
    state.action(Action::Whitespace, 10, Instant::now());
    // Then
    assert_eq!(state.requested_whitespace(), Whitespace::Exact);
}

#[test]
fn w_is_text_when_a_search_prompt_is_open() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Search, 10, Instant::now());
    // When
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
        &Bindings::defaults(),
        10,
        Instant::now(),
    );
    // Then
    assert!(!effects.collect);
    assert_eq!(state.search(), "w");
    assert_eq!(state.requested_whitespace(), Whitespace::Exact);
}

#[test]
fn rapid_toggling_rejects_even_an_obsolete_result_with_the_same_mode() {
    // Given
    let mut state = State::new(false);
    let obsolete = state.collection_request();
    state.action(Action::Whitespace, 10, Instant::now());
    state.collection_request();
    state.action(Action::Whitespace, 10, Instant::now());
    state.collection_request();
    let result = Snapshot {
        branch: "obsolete".into(),
        ..Snapshot::default()
    };
    // When
    let effects = state.collection_finished(obsolete.generation, Ok(result), Instant::now());
    // Then
    assert!(!effects.redraw);
    assert!(!effects.render);
    assert!(state.snapshot().branch.is_empty());
    assert!(state.collecting());
}

#[test]
fn an_obsolete_collection_error_does_not_replace_current_status() {
    // Given
    let mut state = State::new(false);
    let obsolete = state.collection_request();
    state.collection_request();
    // When
    let effects = state.collection_finished(
        obsolete.generation,
        Err(GitError::Command("obsolete failure".into())),
        Instant::now(),
    );
    // Then
    assert!(!effects.redraw);
    assert!(state.error().is_none());
    assert!(state.collecting());
}

#[test]
fn a_failed_toggle_keeps_the_applied_mode_and_hidden_count() {
    // Given
    let mut state = State::with_whitespace(false, Whitespace::IgnoreAll);
    state.update_snapshot(Snapshot {
        whitespace: Whitespace::IgnoreAll,
        whitespace_hidden_files: 5,
        ..Snapshot::default()
    });
    state.action(Action::Whitespace, 10, Instant::now());
    let request = state.collection_request();
    // When
    state.collection_finished(
        request.generation,
        Err(GitError::Command("cannot compare".into())),
        Instant::now(),
    );
    // Then
    assert_eq!(state.snapshot().whitespace, Whitespace::IgnoreAll);
    assert_eq!(state.snapshot().whitespace_hidden_files, 5);
    assert_eq!(state.requested_whitespace(), Whitespace::Exact);
    assert_eq!(state.error(), Some("cannot compare"));
    assert_eq!(state.toast(), None);
}

#[test]
fn accepting_a_filtered_snapshot_invalidates_previous_renders() {
    // Given
    let (mut state, old_render) = reading_state();
    state.action(Action::Whitespace, 10, Instant::now());
    let request = state.collection_request();
    state.collection_finished(
        request.generation,
        Ok(Snapshot {
            whitespace: Whitespace::IgnoreChanges,
            whitespace_hidden_files: 2,
            ..Snapshot::default()
        }),
        Instant::now(),
    );
    // When
    let accepted = state.apply_render(
        RenderResult {
            generation: old_render,
            document: document(&["obsolete"], false),
            finished: true,
        },
        10,
    );
    // Then
    assert!(!accepted);
    assert_eq!(state.snapshot().whitespace_hidden_files, 2);
    assert_eq!(state.visible_files(), 0);
    assert!(state.document().rows.is_empty());
    assert_eq!(state.selection(), std::path::Path::new(""));
}

#[test]
fn switching_whitespace_modes_preserves_scroll_anchors_for_the_first_preview() {
    // Given
    let (mut state, _) = reading_state();
    state.scroll_by(1, true, 1);
    state.action(Action::Whitespace, 1, Instant::now());
    let collection = state.collection_request();
    let snapshot = Snapshot {
        changes: state.snapshot().changes.clone(),
        activity: None,
        branch: "main".into(),
        whitespace: Whitespace::IgnoreChanges,
        whitespace_hidden_files: 0,
        tracked_stats: None,
    };
    state.collection_finished(collection.generation, Ok(snapshot), Instant::now());
    let request = state.render_request(80, 1);
    // When
    state.apply_render(
        RenderResult {
            generation: request.generation,
            document: document(&["inserted", "one", "two", "three"], false),
            finished: true,
        },
        1,
    );
    // Then
    assert_eq!(state.scroll(), 2);
    assert_eq!(
        state.document().rows[state.scroll()].line.to_string(),
        "two"
    );
}

fn document(lines: &[&str], partial: bool) -> Arc<Document> {
    Arc::new(Document {
        rows: lines
            .iter()
            .enumerate()
            .map(|(index, text)| {
                Arc::new(Row {
                    code: Vec::new(),
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

fn reading_state() -> (State, u64) {
    let mut state = State::new(false);
    state.update_snapshot(Snapshot {
        activity: None,
        changes: ["different.txt", "file.txt"]
            .into_iter()
            .map(|path| Change {
                path: path.into(),
                old_path: None,
                category: Category::Unstaged,
                status: 'M',
                hunks: Vec::new(),
                summary: None,
                oversized: false,
                old_source: None,
                new_source: None,
            })
            .collect(),
        branch: "main".into(),
        whitespace: Default::default(),
        whitespace_hidden_files: 0,
        tracked_stats: None,
    });
    state.action(Action::Search, 10, Instant::now());
    state.paste("two", Instant::now());
    state.action(Action::Accept, 10, Instant::now());
    let request = state.render_request(80, 1);
    state.apply_render(
        RenderResult {
            generation: request.generation,
            document: document(&["one", "two", "three"], false),
            finished: true,
        },
        1,
    );
    state.action(Action::Focus, 1, Instant::now());
    state.action(Action::First, 1, Instant::now());
    let request = state.render_request(80, 1);
    (state, request.generation)
}

#[test]
fn intermediate_highlighting_preserves_scroll_and_search_position() {
    // Given
    let (mut state, generation) = reading_state();
    state.scroll_by(1, true, 1);
    let result = RenderResult {
        generation,
        document: document(&["one", "two", "three"], false),
        finished: false,
    };
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(changed);
    assert!(state.rendering());
    assert_eq!(
        state.document().rows[state.scroll()].line.to_string(),
        "two"
    );
    assert_eq!(state.match_index(), Some(0));
    assert_eq!(state.document().hits.len(), 1);
}

#[test]
fn a_late_prefix_preview_preserves_the_users_new_scroll_position() {
    // Given
    let (mut state, generation) = reading_state();
    let result = RenderResult {
        generation,
        document: document(&["one"], true),
        finished: false,
    };
    state.scroll_by(1, true, 1);
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(!changed);
    assert_eq!(
        state.document().rows[state.scroll()].line.to_string(),
        "two"
    );
    assert_eq!(state.document().rows.len(), 3);
    assert_eq!(state.match_index(), Some(0));
    assert_eq!(state.document().hits.len(), 1);
    assert!(state.rendering());
}

#[test]
fn a_complete_refresh_updates_the_document_at_the_current_source_position() {
    // Given
    let (mut state, generation) = reading_state();
    state.scroll_by(1, true, 1);
    let result = RenderResult {
        generation,
        document: document(&["inserted", "one", "two", "three"], false),
        finished: true,
    };
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(changed);
    assert_eq!(
        state.document().rows[state.scroll()].line.to_string(),
        "two"
    );
    assert_eq!(state.scroll(), 2);
    assert_eq!(state.document().hits[0].row, 2);
    assert!(!state.rendering());
}

#[test]
fn an_obsolete_render_cannot_replace_the_current_document_or_search_results() {
    // Given
    let (mut state, generation) = reading_state();
    state.scroll_by(1, true, 1);
    let result = RenderResult {
        generation: generation - 1,
        document: document(&["obsolete"], false),
        finished: true,
    };
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(!changed);
    assert_eq!(
        state.document().rows[state.scroll()].line.to_string(),
        "two"
    );
    assert_eq!(state.document().hits.len(), 1);
    assert_eq!(state.match_index(), Some(0));
    assert!(state.rendering());
}

#[test]
fn a_new_selection_can_display_its_preview_immediately() {
    // Given
    let (mut state, generation) = reading_state();
    state.scroll_by(1, true, 1);
    state.action(Action::NextFile, 1, Instant::now());
    let result = RenderResult {
        generation,
        document: document(&["new selection"], true),
        finished: false,
    };
    // When
    let changed = state.apply_render(result, 1);
    // Then
    assert!(changed);
    assert_eq!(state.scroll(), 0);
    assert_eq!(state.document().rows[0].line.to_string(), "new selection");
    assert_eq!(state.match_index(), None);
}

#[test]
fn moving_the_prompt_cursor_redraws_without_rendering_the_diff() {
    // Given
    let (mut state, _) = reading_state();
    state.action(Action::Search, 10, Instant::now());
    // When
    let effects = state.action(Action::CursorLeft, 10, Instant::now());
    // Then
    assert_eq!(effects, Effects::draw(false));
    assert_eq!(state.search(), "two");
}

#[test]
fn pasting_a_query_requests_a_new_render() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Search, 10, Instant::now());
    // When
    let effects = state.paste("needle\n", Instant::now());
    // Then
    assert_eq!(effects, Effects::draw(true));
    assert_eq!(state.search(), "needle");
}

#[test]
fn cancelling_a_changed_query_renders_the_restored_query() {
    // Given
    let (mut state, _) = reading_state();
    state.action(Action::Search, 10, Instant::now());
    state.paste(" more", Instant::now());
    // When
    let effects = state.action(Action::Cancel, 10, Instant::now());
    // Then
    assert_eq!(effects, Effects::draw(true));
    assert_eq!(state.search(), "two");
    assert!(state.prompt().is_none());
}

#[test]
fn the_normal_quit_key_is_text_while_editing() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Search, 10, Instant::now());
    // When
    let effects = state.handle_key(
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        &Bindings::defaults(),
        10,
        Instant::now(),
    );
    // Then
    assert_eq!(effects, Effects::draw(true));
    assert_eq!(state.search(), "q");
}

#[test]
fn manual_refresh_requests_collection_without_replacing_the_document() {
    // Given
    let (mut state, _) = reading_state();
    // When
    let effects = state.action(Action::Refresh, 10, Instant::now());
    // Then
    assert!(effects.collect);
    assert!(!effects.render);
    assert_eq!(state.document().rows.len(), 3);
}

#[test]
fn a_failed_collection_retains_the_document_and_allows_retry() {
    // Given
    let (mut state, _) = reading_state();
    state.collection_started(0);
    // When
    let effects = state.collection_finished(
        0,
        Err(GitError::Command("Cannot read repository".into())),
        Instant::now(),
    );
    // Then
    assert_eq!(effects, Effects::draw(false));
    assert_eq!(state.error(), Some("Cannot read repository"));
    assert!(!state.collecting());
    assert_eq!(state.document().rows.len(), 3);
}

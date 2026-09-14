use super::*;
use crate::{AppResult, config::Config, input::Bindings};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn row<'a>(state: &'a State, path: &str) -> AppResult<&'a TreeRow> {
    state
        .tree(false)
        .iter()
        .find(|row| row.path == Path::new(path))
        .ok_or_else(|| format!("Missing tree row {path}").into())
}

fn collapsed_src() -> State {
    let mut state = state();
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    state
}

fn filter(state: &mut State, query: &str) {
    state.action(Action::Filter, 10, Instant::now());
    state.paste(query, Instant::now());
    state.action(Action::Accept, 10, Instant::now());
}

fn find(state: &mut State, query: &str) {
    state.action(Action::Find, 10, Instant::now());
    state.paste(query, Instant::now());
}

fn compact_state() -> State {
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("dir-a/dir-b/dir-c/file-a", Category::Unstaged),
        ("dir-a/dir-b/dir-c/file-b", Category::Staged),
    ]));
    state
}

#[test]
fn directory_chains_share_one_row_above_their_files() -> AppResult<()> {
    // Given
    let state = compact_state();
    // When
    let directory = row(&state, "dir-a/dir-b/dir-c")?;
    // Then
    assert_eq!(directory.name, "dir-a/dir-b/dir-c");
    assert_eq!(directory.depth, 1);
    assert_eq!(directory.count, 2);
    assert_eq!(row(&state, "dir-a/dir-b/dir-c/file-a")?.depth, 2);
    assert_eq!(state.tree(false).len(), 4);
    Ok(())
}

#[test]
fn right_enters_a_compact_directory_from_its_displayed_parent() {
    // Given
    let mut state = compact_state();
    // When
    state.action(Action::Right, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("dir-a/dir-b/dir-c"));
    assert_eq!(state.selected_changes(), [0, 1]);
}

#[test]
fn left_skips_hidden_parents_of_a_collapsed_compact_directory() {
    // Given
    let mut state = compact_state();
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    // When
    state.action(Action::Left, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new(""));
    assert_eq!(state.tree(false).len(), 2);
}

#[test]
fn compact_directory_search_highlights_each_component_by_grapheme() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[("a\u{301}/b/c/file", Category::Unstaged)]));
    // When
    filter(&mut state, "a/b/c");
    // Then
    let directory = row(&state, "a\u{301}/b/c")?;
    assert_eq!(directory.name, "a\u{301}/b/c");
    assert_eq!(directory.match_indices, [0, 2, 4]);
    Ok(())
}

#[test]
fn compaction_stops_at_files_and_branches() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    // When
    state.update_snapshot(snapshot_with(&[
        ("a/b/file", Category::Unstaged),
        ("a/b/c/d/file", Category::Unstaged),
        ("x/y/file", Category::Unstaged),
        ("x/z/file", Category::Unstaged),
    ]));
    // Then
    assert_eq!(row(&state, "a/b")?.name, "a/b");
    assert_eq!(row(&state, "a/b/c/d")?.name, "c/d");
    assert_eq!(row(&state, "x")?.name, "x");
    assert_eq!(row(&state, "x/y")?.name, "y");
    assert_eq!(row(&state, "x/z")?.name, "z");
    Ok(())
}

#[test]
fn filtering_compacts_newly_single_child_directories_and_reveals_them() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/b/file", Category::Unstaged),
        ("a/other", Category::Unstaged),
    ]));
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    // When
    filter(&mut state, "file");
    // Then
    assert_eq!(row(&state, "a/b")?.name, "a/b");
    assert_eq!(state.selection(), Path::new("a/b"));
    assert_eq!(state.tree(false).len(), 3);
    assert!(!state.is_collapsed(Path::new("a/b")));
    Ok(())
}

#[test]
fn compaction_preserves_a_directory_with_its_own_changed_file_status() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    // When
    state.update_snapshot(snapshot_with(&[
        ("a", Category::Staged),
        ("a/b/c/file", Category::Unstaged),
    ]));
    // Then
    assert_eq!(row(&state, "a")?.count, 2);
    assert_eq!(row(&state, "a")?.badges.len(), 1);
    assert_eq!(row(&state, "a/b/c")?.name, "b/c");
    Ok(())
}

#[test]
fn refresh_preserves_collapse_when_the_selected_directory_becomes_compact() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/b/file", Category::Unstaged),
        ("a/other", Category::Unstaged),
    ]));
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    // When
    state.update_snapshot(snapshot_with(&[("a/b/file", Category::Unstaged)]));
    // Then
    assert_eq!(state.selection(), Path::new("a/b"));
    assert!(state.is_collapsed(Path::new("a/b")));
    assert_eq!(state.tree(false).len(), 2);
}

#[test]
fn expanding_a_compact_directory_clears_its_former_ancestor_collapse() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/b/file", Category::Unstaged),
        ("a/other", Category::Unstaged),
    ]));
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    state.update_snapshot(snapshot_with(&[("a/b/file", Category::Unstaged)]));
    // When
    state.action(Action::Right, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("a/b"));
    assert!(!state.is_collapsed(Path::new("a/b")));
    assert_eq!(state.tree(false).len(), 3);
}

#[test]
fn directories_count_each_changed_path_once() -> AppResult<()> {
    // Given
    let state = state();
    // When
    let root = row(&state, "")?;
    let src = row(&state, "src")?;
    let file = row(&state, "src/a.rs")?;
    // Then
    assert_eq!(root.name, "All changes");
    assert_eq!(root.count, 3);
    assert_eq!(src.count, 2);
    assert_eq!(file.count, 1);
    assert_eq!(
        file.badges,
        [
            TreeBadge {
                category: Category::Unstaged,
                status: 'M'
            },
            TreeBadge {
                category: Category::Staged,
                status: 'M'
            }
        ]
    );
    Ok(())
}

#[test]
fn filtering_reveals_collapsed_ancestors_and_counts_matching_paths() -> AppResult<()> {
    // Given
    let mut state = collapsed_src();
    // When
    filter(&mut state, "a.rs");
    // Then
    assert_eq!(
        paths(&state),
        [Path::new(""), Path::new("src"), Path::new("src/a.rs")]
    );
    assert_eq!(row(&state, "src")?.count, 1);
    assert_eq!(state.visible_files(), 3);
    assert!(!state.is_collapsed(Path::new("src")));
    Ok(())
}

#[test]
fn fuzzy_matches_highlight_their_visible_path_components() -> AppResult<()> {
    // Given
    let mut state = state();
    // When
    filter(&mut state, "sr/a");
    // Then
    assert_eq!(row(&state, "src")?.match_indices, [0, 1]);
    assert_eq!(row(&state, "src/a.rs")?.match_indices, [0]);
    Ok(())
}

#[test]
fn fuzzy_match_positions_follow_graphemes_across_path_components() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[(
        "a\u{301}/e\u{301}long.rs",
        Category::Unstaged,
    )]));
    // When
    filter(&mut state, "a/long");
    // Then
    assert_eq!(row(&state, "a\u{301}")?.match_indices, [0]);
    assert_eq!(
        row(&state, "a\u{301}/e\u{301}long.rs")?.match_indices,
        [1, 2, 3, 4]
    );
    Ok(())
}

#[test]
fn clearing_a_filter_restores_collapses_and_selects_a_visible_ancestor() {
    // Given
    let mut state = collapsed_src();
    filter(&mut state, "a.rs");
    state.action(Action::NextFile, 10, Instant::now());
    // When
    state.action(Action::Cancel, 10, Instant::now());
    // Then
    assert_eq!(state.filter(), "");
    assert!(state.is_collapsed(Path::new("src")));
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(
        paths(&state),
        [
            Path::new(""),
            Path::new("src"),
            Path::new("tests"),
            Path::new("tests/c.rs")
        ]
    );
}

#[test]
fn cancelling_an_edit_restores_the_previous_filtered_collapse_state() {
    // Given
    let mut state = state();
    filter(&mut state, "src");
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    let now = Instant::now();
    state.action(Action::Filter, 10, now);
    state.action(Action::ClearBefore, 10, now);
    state.apply_pending_filter(now + Duration::from_millis(300));
    // When
    state.action(Action::Cancel, 10, Instant::now());
    // Then
    assert_eq!(state.filter(), "src");
    assert_eq!(state.selection(), Path::new("src"));
    assert!(state.is_collapsed(Path::new("src")));
    assert_eq!(paths(&state), [Path::new(""), Path::new("src")]);
}

#[test]
fn filtered_collapses_do_not_change_the_normal_tree() {
    // Given
    let mut state = state();
    filter(&mut state, "src");
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    // When
    state.action(Action::Cancel, 10, Instant::now());
    // Then
    assert!(!state.is_collapsed(Path::new("src")));
    assert_eq!(state.tree(false).len(), 6);
}

#[test]
fn changing_a_filter_reopens_matching_directories() {
    // Given
    let mut state = state();
    filter(&mut state, "src");
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    // When
    filter(&mut state, "/a");
    // Then
    assert_eq!(state.filter(), "src/a");
    assert!(!state.is_collapsed(Path::new("src")));
    assert_eq!(
        paths(&state),
        [Path::new(""), Path::new("src"), Path::new("src/a.rs")]
    );
}

#[test]
fn refreshing_preserves_filtered_collapses_and_updates_counts() -> AppResult<()> {
    // Given
    let mut state = state();
    filter(&mut state, "src");
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    let snapshot = snapshot_with(&[
        ("src/a.rs", Category::Staged),
        ("src/b.rs", Category::Unstaged),
        ("src/c.rs", Category::Untracked),
    ]);
    // When
    state.update_snapshot(snapshot);
    // Then
    assert_eq!(row(&state, "src")?.count, 3);
    assert!(state.is_collapsed(Path::new("src")));
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(paths(&state), [Path::new(""), Path::new("src")]);
    Ok(())
}

#[test]
fn right_enters_the_first_child_of_an_expanded_directory() {
    // Given
    let mut state = state();
    state.action(Action::Down, 10, Instant::now());
    // When
    let effects = state.action(Action::Right, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src/a.rs"));
    assert!(effects.render);
}

#[test]
fn enter_keeps_an_expanded_directory_selected() {
    // Given
    let mut state = state();
    state.action(Action::Down, 10, Instant::now());
    // When
    state.action(Action::Accept, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(state.selected_changes(), [0, 1, 2]);
}

#[test]
fn filtering_out_a_file_selects_its_nearest_remaining_ancestor() {
    // Given
    let mut state = state();
    state.action(Action::NextFile, 10, Instant::now());
    // When
    filter(&mut state, "b.rs");
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(state.selected_changes(), [2]);
}

#[test]
fn find_live_selects_the_highest_scoring_file() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("alpha-target.rs", Category::Unstaged),
        ("target.rs", Category::Unstaged),
    ]));
    state.action(Action::NextFile, 10, Instant::now());
    assert_eq!(state.selection(), Path::new("alpha-target.rs"));
    // When
    find(&mut state, "target.rs");
    // Then
    assert_eq!(state.selection(), Path::new("target.rs"));
}

#[test]
fn find_live_selects_the_first_file_in_tree_order_when_scores_tie() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    state.action(Action::Last, 10, Instant::now());
    // When
    find(&mut state, "file.rs");
    // Then
    assert_eq!(state.selection(), Path::new("a/file.rs"));
}

#[test]
fn find_reveals_a_match_below_a_collapsed_directory() {
    // Given
    let mut state = collapsed_src();
    // When
    find(&mut state, "b.rs");
    // Then
    assert_eq!(state.selection(), Path::new("src/b.rs"));
    assert!(!state.is_collapsed(Path::new("src")));
    assert!(paths(&state).contains(&Path::new("src/b.rs")));
}

#[test]
fn accepting_find_keeps_its_selected_file() {
    // Given
    let mut state = state();
    find(&mut state, "b.rs");
    // When
    state.action(Action::Accept, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src/b.rs"));
    assert!(state.prompt().is_none());
}

#[test]
fn cancelling_find_restores_its_selection_and_collapses() {
    // Given
    let mut state = collapsed_src();
    find(&mut state, "b.rs");
    // When
    state.action(Action::Cancel, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert!(state.is_collapsed(Path::new("src")));
    assert!(!paths(&state).contains(&Path::new("src/b.rs")));
}

#[test]
fn emptying_find_restores_its_selection_and_collapses() {
    // Given
    let mut state = collapsed_src();
    state.action(Action::Last, 10, Instant::now());
    find(&mut state, "b.rs");
    // When
    state.action(Action::ClearBefore, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("tests/c.rs"));
    assert!(state.is_collapsed(Path::new("src")));
    assert!(!paths(&state).contains(&Path::new("src/b.rs")));
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("")
    );
}

#[test]
fn find_without_matches_leaves_the_current_selection_unchanged() {
    // Given
    let mut state = collapsed_src();
    // When
    find(&mut state, "missing-file");
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert!(state.is_collapsed(Path::new("src")));
}

#[test]
fn find_respects_an_existing_filter_without_changing_its_rows() {
    // Given
    let mut state = state();
    filter(&mut state, "tests");
    let filtered_paths: Vec<_> = paths(&state).into_iter().map(Path::to_owned).collect();
    // When
    find(&mut state, ".rs");
    // Then
    assert_eq!(state.selection(), Path::new("tests/c.rs"));
    assert_eq!(state.filter(), "tests");
    assert_eq!(paths(&state), filtered_paths);
}

#[test]
fn next_find_match_uses_descending_fuzzy_score_order() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("alpha-target.rs", Category::Unstaged),
        ("t-a-r-g-e-t.rs", Category::Unstaged),
        ("target.rs", Category::Unstaged),
    ]));
    find(&mut state, "target.rs");
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("alpha-target.rs"));
}

#[test]
fn next_find_match_preserves_tree_order_for_equal_scores() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
        ("c/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("b/file.rs"));
}

#[test]
fn next_find_match_wraps_to_the_highest_scoring_match() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    state.action(Action::NextFindMatch, 10, Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("a/file.rs"));
}

#[test]
fn find_cycling_reaches_matches_beyond_the_first_ten() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
        ("c/file.rs", Category::Unstaged),
        ("d/file.rs", Category::Unstaged),
        ("e/file.rs", Category::Unstaged),
        ("f/file.rs", Category::Unstaged),
        ("g/file.rs", Category::Unstaged),
        ("h/file.rs", Category::Unstaged),
        ("i/file.rs", Category::Unstaged),
        ("j/file.rs", Category::Unstaged),
        ("k/file.rs", Category::Unstaged),
        ("l/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::NextFindMatch, 10, Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("l/file.rs"));
}

#[test]
fn moving_the_find_cursor_does_not_restart_the_match_cycle() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
        ("c/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::CursorLeft, 10, Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("c/file.rs"));
}

#[test]
fn editing_the_find_query_restarts_the_match_cycle() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file");
    state.action(Action::NextFindMatch, 10, Instant::now());
    // When
    state.paste(".rs", Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("a/file.rs"));
}

#[test]
fn accepting_find_keeps_a_cycled_selection() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    state.action(Action::NextFindMatch, 10, Instant::now());
    // When
    state.action(Action::Accept, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("b/file.rs"));
    assert!(state.prompt().is_none());
}

#[test]
fn cycling_find_accumulates_revealed_ancestors() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    state.action(Action::First, 10, Instant::now());
    find(&mut state, "file.rs");
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("b/file.rs"));
    assert!(!state.is_collapsed(Path::new("a")));
    assert!(!state.is_collapsed(Path::new("b")));
}

#[test]
fn cancelling_find_restores_all_collapses_revealed_while_cycling() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    state.action(Action::First, 10, Instant::now());
    find(&mut state, "file.rs");
    state.action(Action::NextFindMatch, 10, Instant::now());
    // When
    state.action(Action::Cancel, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new(""));
    assert!(state.is_collapsed(Path::new("a")));
    assert!(state.is_collapsed(Path::new("b")));
    assert_eq!(
        paths(&state),
        [Path::new(""), Path::new("a"), Path::new("b")]
    );
}

#[test]
fn next_find_match_with_an_empty_query_is_a_no_op() {
    // Given
    let mut state = collapsed_src();
    state.action(Action::Find, 10, Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert!(state.is_collapsed(Path::new("src")));
}

#[test]
fn next_find_match_without_matches_is_a_no_op() {
    // Given
    let mut state = collapsed_src();
    find(&mut state, "missing-file");
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert!(state.is_collapsed(Path::new("src")));
}

#[test]
fn next_find_match_is_a_no_op_in_a_filter_prompt() {
    // Given
    let mut state = collapsed_src();
    state.action(Action::Filter, 10, Instant::now());
    state.paste("src", Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(state.filter(), "");
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("src")
    );
}

#[test]
fn next_find_match_is_a_no_op_in_a_search_prompt() {
    // Given
    let mut state = state();
    state.action(Action::Search, 10, Instant::now());
    state.paste("needle", Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new(""));
    assert_eq!(state.search(), "needle");
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some("needle")
    );
}

#[test]
fn find_cycling_respects_the_existing_filter() {
    // Given
    let mut state = state();
    filter(&mut state, "src");
    let filtered_paths: Vec<_> = paths(&state).into_iter().map(Path::to_owned).collect();
    find(&mut state, ".rs");
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src/b.rs"));
    assert_eq!(state.filter(), "src");
    assert_eq!(paths(&state), filtered_paths);
}

#[test]
fn default_ctrl_n_dispatches_next_find_match() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    // When
    state.handle_key(
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
        &Bindings::defaults(),
        10,
        Instant::now(),
    );
    // Then
    assert_eq!(state.selection(), Path::new("b/file.rs"));
}

#[test]
fn refreshing_find_keeps_the_cycled_match_when_it_survives() {
    // Given
    let mut state = state();
    find(&mut state, "r");
    state.action(Action::NextFindMatch, 10, Instant::now());
    let selected = state.selection().to_owned();
    // When
    state.update_snapshot(snapshot());
    // Then
    assert_eq!(state.selection(), selected);
}

#[test]
fn cycling_find_after_refresh_uses_the_new_matching_paths() {
    // Given
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("c/file.rs", Category::Unstaged),
    ]));
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("c/file.rs"));
}

#[test]
fn clearing_find_discards_the_previous_match_cycle() {
    // Given
    let mut state = collapsed_src();
    find(&mut state, "r");
    state.action(Action::NextFindMatch, 10, Instant::now());
    state.action(Action::ClearBefore, 10, Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), Path::new("src"));
    assert!(state.is_collapsed(Path::new("src")));
}

#[test]
fn editing_find_to_no_matches_discards_the_previous_match_cycle() {
    // Given
    let mut state = state();
    find(&mut state, "r");
    state.action(Action::NextFindMatch, 10, Instant::now());
    let selected = state.selection().to_owned();
    state.paste("missing-file", Instant::now());
    // When
    state.action(Action::NextFindMatch, 10, Instant::now());
    // Then
    assert_eq!(state.selection(), selected);
}

#[test]
fn remapped_key_dispatches_next_find_match() -> AppResult<()> {
    // Given
    let bindings = Config::parse("keybindings { input { next-find-match \"alt-j\"; }; }")?.bindings;
    let mut state = State::new(false);
    state.update_snapshot(snapshot_with(&[
        ("a/file.rs", Category::Unstaged),
        ("b/file.rs", Category::Unstaged),
    ]));
    find(&mut state, "file.rs");
    // When
    state.handle_key(
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT),
        &bindings,
        10,
        Instant::now(),
    );
    // Then
    assert_eq!(state.selection(), Path::new("b/file.rs"));
    Ok(())
}

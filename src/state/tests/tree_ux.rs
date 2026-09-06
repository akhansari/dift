use super::*;
use crate::AppResult;

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
    assert_eq!(file.badges, "US");
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
    state.apply_pending_filter(now + FILTER_DEBOUNCE);
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

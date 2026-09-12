use super::*;

#[test]
fn cancelling_search_restores_its_query_without_reverting_a_refreshed_tree() {
    // Given
    let mut state = state();
    let now = Instant::now();
    state.action(Action::NextFile, 10, now);
    state.action(Action::Search, 10, now);
    state.paste("previous", now);
    state.action(Action::Accept, 10, now);
    state.action(Action::Search, 10, now);
    state.paste(" draft", now);
    state.update_snapshot(snapshot_with(&[("src/new.rs", Category::Unstaged)]));
    // When
    let effects = state.action(Action::Cancel, 10, now);
    // Then
    assert_eq!(state.search(), "previous");
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(
        paths(&state),
        [Path::new(""), Path::new("src"), Path::new("src/new.rs")]
    );
    assert!(state.prompt().is_none());
    assert_eq!(state.filter_delay(now), None);
    assert!(effects.render);
    assert_eq!(effects.command, None);
}

use super::*;
use crate::model::{ActivityObservation, GitIdentity, WorktreeMetadata};
use std::{collections::BTreeMap, path::Path};

fn observed(files: &[(&str, &str)]) -> Snapshot {
    Snapshot {
        activity: Some(
            files
                .iter()
                .map(|(path, source)| {
                    (
                        (*path).into(),
                        BTreeMap::from([(
                            Category::Unstaged,
                            ActivityObservation {
                                git: GitIdentity {
                                    status: 'M',
                                    ..GitIdentity::default()
                                },
                                metadata: Some(WorktreeMetadata::Missing),
                                source: Some(Arc::from(*source)),
                            },
                        )]),
                    )
                })
                .collect(),
        ),
        changes: files
            .iter()
            .map(|(path, source)| Change {
                path: (*path).into(),
                old_path: None,
                category: Category::Unstaged,
                status: 'M',
                hunks: Vec::new(),
                summary: None,
                oversized: false,
                old_source: None,
                new_source: Some(Arc::from(*source)),
            })
            .collect(),
        ..Snapshot::default()
    }
}

fn collect(state: &mut State, files: &[(&str, &str)], now: Instant) -> Effects {
    let request = state.collection_request();
    state.collection_finished(request.generation, Ok(observed(files)), now)
}

fn active(now: Instant) -> State {
    let mut state = State::new(false);
    collect(
        &mut state,
        &[("src/a.rs", "before"), ("src/b.rs", "before")],
        now - Duration::from_secs(1),
    );
    collect(
        &mut state,
        &[("src/a.rs", "after"), ("src/b.rs", "after")],
        now,
    );
    state.advance_activity(now + Duration::from_secs(1));
    state
}

fn intensity(state: &State) -> f32 {
    state.activity_intensity(Path::new("src/a.rs"), false)
}

#[test]
fn first_successful_collection_establishes_a_quiet_baseline() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    // When
    collect(&mut state, &[("src/a.rs", "before")], now);
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert_eq!(intensity(&state), 0.0);
}

#[test]
fn an_addition_after_an_empty_baseline_is_highlighted() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    collect(&mut state, &[], now);
    // When
    collect(&mut state, &[("src/a.rs", "new")], now);
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert_eq!(intensity(&state), 1.0);
    assert_eq!(state.activity_intensity(Path::new("src"), true), 0.5);
    assert_eq!(state.activity_intensity(Path::new(""), true), 0.5);
}

#[test]
fn activity_fades_out_without_recollecting_or_rendering_the_diff() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    // When
    let effects = state.advance_activity(now + Duration::from_secs(31));
    // Then
    assert_eq!(intensity(&state), 0.5);
    assert!(effects.redraw);
    assert!(!effects.render);
    assert!(!effects.collect);
}

#[test]
fn activity_expires_after_thirty_two_seconds() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    // When
    state.advance_activity(now + Duration::from_secs(32));
    // Then
    assert_eq!(intensity(&state), 0.0);
    assert_eq!(state.activity_intensity(Path::new(""), true), 0.0);
    assert!(!state.advance_activity(now + Duration::from_secs(33)).redraw);
}

#[test]
fn a_new_edit_restarts_from_the_current_fade_intensity() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    state.advance_activity(now + Duration::from_secs(31));
    // When
    collect(
        &mut state,
        &[("src/a.rs", "third"), ("src/b.rs", "after")],
        now + Duration::from_secs(31),
    );
    // Then
    assert_eq!(intensity(&state), 0.5);
}

#[test]
fn a_new_edit_gets_another_full_thirty_two_seconds() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    collect(
        &mut state,
        &[("src/a.rs", "third"), ("src/b.rs", "after")],
        now + Duration::from_secs(31),
    );
    // When
    state.advance_activity(now + Duration::from_secs(62));
    // Then
    assert_eq!(intensity(&state), 0.5);
}

#[test]
fn visiting_a_file_fades_it_without_acknowledging_its_sibling() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    // When
    state.action(Action::NextFile, 10, now + Duration::from_secs(1));
    state.advance_activity(now + Duration::from_millis(1100));
    // Then
    assert_eq!(state.selection(), Path::new("src/a.rs"));
    assert_eq!(intensity(&state), 0.5);
    assert_eq!(state.activity_intensity(Path::new("src/b.rs"), false), 1.0);
    assert_eq!(state.activity_intensity(Path::new("src"), true), 0.5);
}

#[test]
fn visits_finish_fading_after_two_hundred_milliseconds() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    state.action(Action::NextFile, 10, now + Duration::from_secs(1));
    // When
    state.advance_activity(now + Duration::from_millis(1200));
    // Then
    assert_eq!(intensity(&state), 0.0);
}

#[test]
fn an_edit_on_the_selected_file_still_pulses() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    state.action(Action::NextFile, 10, now + Duration::from_secs(1));
    // When
    collect(
        &mut state,
        &[("src/a.rs", "third"), ("src/b.rs", "after")],
        now + Duration::from_secs(2),
    );
    state.advance_activity(now + Duration::from_secs(3));
    // Then
    assert_eq!(state.selection(), Path::new("src/a.rs"));
    assert_eq!(intensity(&state), 1.0);
}

#[test]
fn enter_acknowledges_a_new_edit_on_the_selected_file() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    state.action(Action::NextFile, 10, now + Duration::from_secs(1));
    collect(
        &mut state,
        &[("src/a.rs", "third"), ("src/b.rs", "after")],
        now + Duration::from_secs(2),
    );
    // When
    state.action(Action::Accept, 10, now + Duration::from_secs(3));
    state.advance_activity(now + Duration::from_millis(3200));
    // Then
    assert_eq!(intensity(&state), 0.0);
}

#[test]
fn clicking_the_selected_file_acknowledges_without_resetting_its_diff() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    state.action(Action::NextFile, 10, now + Duration::from_secs(1));
    collect(
        &mut state,
        &[("src/a.rs", "third"), ("src/b.rs", "after")],
        now + Duration::from_secs(2),
    );
    // When
    let effects = state.select_tree_row(2, now + Duration::from_secs(3));
    state.advance_activity(now + Duration::from_millis(3200));
    // Then
    assert_eq!(intensity(&state), 0.0);
    assert!(!effects.render);
}

#[test]
fn selecting_and_collapsing_a_parent_keeps_child_activity() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    // When
    state.action(Action::Down, 10, now + Duration::from_secs(1));
    state.action(Action::Left, 10, now + Duration::from_secs(1));
    // Then
    assert!(state.is_collapsed(Path::new("src")));
    assert_eq!(state.selection(), Path::new("src"));
    assert_eq!(intensity(&state), 1.0);
    assert_eq!(state.activity_intensity(Path::new("src"), true), 0.5);
}

#[test]
fn hidden_tree_activity_keeps_its_original_deadline() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    state.action(Action::Sidebar, 10, now + Duration::from_secs(1));
    // When
    state.advance_activity(now + Duration::from_secs(32));
    state.action(Action::Sidebar, 10, now + Duration::from_secs(33));
    // Then
    assert_eq!(intensity(&state), 0.0);
}

#[test]
fn excluded_files_do_not_highlight_visible_parents() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    collect(
        &mut state,
        &[("src/a.rs", "before"), ("src/b.rs", "before")],
        now,
    );
    collect(
        &mut state,
        &[("src/a.rs", "after"), ("src/b.rs", "before")],
        now,
    );
    state.advance_activity(now + Duration::from_secs(1));
    // When
    state.action(Action::Filter, 10, now + Duration::from_secs(1));
    state.paste("b.rs", now + Duration::from_secs(1));
    state.action(Action::Accept, 10, now + Duration::from_secs(1));
    // Then
    assert_eq!(state.activity_intensity(Path::new("src"), true), 0.0);
    assert_eq!(state.activity_intensity(Path::new(""), true), 0.0);
    assert_eq!(intensity(&state), 1.0);
}

#[test]
fn disappearing_files_leave_no_activity_on_the_parent() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    // When
    collect(&mut state, &[], now + Duration::from_secs(2));
    // Then
    assert_eq!(state.activity_intensity(Path::new(""), true), 0.0);
    assert_eq!(intensity(&state), 0.0);
}

#[test]
fn unchanged_refreshes_do_not_extend_activity() {
    // Given
    let now = Instant::now();
    let mut state = active(now);
    collect(
        &mut state,
        &[("src/a.rs", "after"), ("src/b.rs", "after")],
        now + Duration::from_secs(31),
    );
    // When
    state.advance_activity(now + Duration::from_secs(32));
    // Then
    assert_eq!(intensity(&state), 0.0);
}

#[test]
fn obsolete_collections_cannot_create_activity() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    collect(&mut state, &[("src/a.rs", "before")], now);
    let obsolete = state.collection_request();
    state.collection_request();
    // When
    state.collection_finished(
        obsolete.generation,
        Ok(observed(&[("src/a.rs", "after")])),
        now,
    );
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert_eq!(intensity(&state), 0.0);
}

#[test]
fn failed_collections_preserve_the_last_successful_baseline() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    collect(&mut state, &[("src/a.rs", "before")], now);
    let request = state.collection_request();
    state.collection_finished(
        request.generation,
        Err(GitError::InvalidOutput("failed".into())),
        now,
    );
    // When
    collect(&mut state, &[("src/a.rs", "after")], now);
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert_eq!(intensity(&state), 1.0);
}

#[test]
fn additional_observation_evidence_does_not_invalidate_the_diff() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    let mut baseline = observed(&[("src/a.rs", "before")]);
    baseline.activity = Some(BTreeMap::from([(
        "src/a.rs".into(),
        BTreeMap::from([(
            Category::Unstaged,
            ActivityObservation {
                git: GitIdentity {
                    status: 'M',
                    ..GitIdentity::default()
                },
                metadata: Some(WorktreeMetadata::Missing),
                source: None,
            },
        )]),
    )]));
    let request = state.collection_request();
    state.collection_finished(request.generation, Ok(baseline), now);
    // When
    let effects = collect(&mut state, &[("src/a.rs", "before")], now);
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert!(!effects.render);
    assert_eq!(intensity(&state), 0.0);
    assert!(state.snapshot().activity.is_none());
}

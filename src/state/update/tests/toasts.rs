use super::*;

fn apply_whitespace_toggle(state: &mut State, now: Instant) {
    state.action(Action::Whitespace, 10, now);
    let request = state.collection_request();
    state.collection_finished(
        request.generation,
        Ok(Snapshot {
            whitespace: request.whitespace,
            ..Snapshot::default()
        }),
        now,
    );
}

#[test]
fn switching_to_side_by_side_renders_without_a_toast() {
    // Given
    let mut state = State::new(false);
    // When
    let effects = state.action(Action::Layout, 10, Instant::now());
    // Then
    assert!(effects.render);
    assert!(state.split());
    assert_eq!(state.toast(), None);
}

#[test]
fn switching_to_unified_renders_without_a_toast() {
    // Given
    let mut state = State::new(true);
    // When
    let effects = state.action(Action::Layout, 10, Instant::now());
    // Then
    assert!(effects.render);
    assert!(!state.split());
    assert_eq!(state.toast(), None);
}

#[test]
fn layout_changes_preserve_an_active_whitespace_toast_and_its_deadline() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    apply_whitespace_toggle(&mut state, now);
    // When
    state.action(Action::Layout, 10, now + Duration::from_secs(1));
    // Then
    assert_eq!(
        state.toast(),
        Some("Whitespace: ignore-changes · 0 hidden by whitespace")
    );
    assert_eq!(
        state.toast_delay(now + Duration::from_secs(1)),
        Some(Duration::from_secs(2))
    );
}

#[test]
fn a_toast_remains_visible_for_the_full_three_seconds() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    apply_whitespace_toggle(&mut state, now);
    // When
    let effects = state.expire_toast(now + Duration::from_millis(2999));
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(
        state.toast(),
        Some("Whitespace: ignore-changes · 0 hidden by whitespace")
    );
}

#[test]
fn expiration_clears_the_toast_and_only_requests_a_redraw() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    apply_whitespace_toggle(&mut state, now);
    // When
    let effects = state.expire_toast(now + Duration::from_secs(3));
    // Then
    assert_eq!(
        effects,
        Effects {
            redraw: true,
            ..Effects::default()
        }
    );
    assert_eq!(state.toast(), None);
    assert_eq!(state.toast_delay(now + Duration::from_secs(3)), None);
}

#[test]
fn an_expired_toast_does_not_keep_requesting_redraws() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    apply_whitespace_toggle(&mut state, now);
    state.expire_toast(now + Duration::from_secs(3));
    // When
    let effects = state.expire_toast(now + Duration::from_secs(4));
    // Then
    assert_eq!(effects, Effects::default());
}

#[test]
fn a_replacement_toast_gets_its_own_three_seconds() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    apply_whitespace_toggle(&mut state, now);
    apply_whitespace_toggle(&mut state, now + Duration::from_secs(2));
    // When
    let effects = state.expire_toast(now + Duration::from_secs(3));
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(
        state.toast(),
        Some("Whitespace: exact · 0 hidden by whitespace")
    );
    assert_eq!(
        state.toast_delay(now + Duration::from_secs(3)),
        Some(Duration::from_secs(2))
    );
}

macro_rules! applied_whitespace_toast {
    ($name:ident, $initial:expr, $applied:expr, $hidden:expr, $message:expr) => {
        #[test]
        fn $name() {
            // Given
            let now = Instant::now();
            let mut state = State::with_whitespace(false, $initial);
            state.update_snapshot(Snapshot {
                whitespace: $initial,
                ..Snapshot::default()
            });
            state.action(Action::Whitespace, 10, now);
            let request = state.collection_request();
            // When
            state.collection_finished(
                request.generation,
                Ok(Snapshot {
                    whitespace: $applied,
                    whitespace_hidden_files: $hidden,
                    ..Snapshot::default()
                }),
                now + Duration::from_secs(2),
            );
            // Then
            assert_eq!(state.toast(), Some($message));
            assert_eq!(
                state.toast_delay(now + Duration::from_secs(2)),
                Some(Duration::from_secs(3))
            );
        }
    };
}

applied_whitespace_toast!(
    applying_ignore_changes_announces_the_mode_and_hidden_count,
    Whitespace::Exact,
    Whitespace::IgnoreChanges,
    5,
    "Whitespace: ignore-changes · 5 hidden by whitespace"
);

applied_whitespace_toast!(
    applying_exact_announces_the_mode_and_hidden_count,
    Whitespace::IgnoreAll,
    Whitespace::Exact,
    0,
    "Whitespace: exact · 0 hidden by whitespace"
);

#[test]
fn restoring_the_configured_ignore_all_mode_announces_it() {
    // Given
    let now = Instant::now();
    let mut state = State::with_whitespace(false, Whitespace::IgnoreAll);
    state.action(Action::Whitespace, 10, now);
    let exact = state.collection_request();
    state.collection_finished(exact.generation, Ok(Snapshot::default()), now);
    state.action(Action::Whitespace, 10, now);
    let request = state.collection_request();
    // When
    state.collection_finished(
        request.generation,
        Ok(Snapshot {
            whitespace: Whitespace::IgnoreAll,
            whitespace_hidden_files: 2,
            ..Snapshot::default()
        }),
        now,
    );
    // Then
    assert_eq!(
        state.toast(),
        Some("Whitespace: ignore-all · 2 hidden by whitespace")
    );
}

#[test]
fn configured_settings_are_silent_at_startup() {
    // Given
    let mut state = State::with_whitespace(true, Whitespace::IgnoreAll);
    let request = state.collection_request();
    // When
    state.collection_finished(
        request.generation,
        Ok(Snapshot {
            whitespace: Whitespace::IgnoreAll,
            ..Snapshot::default()
        }),
        Instant::now(),
    );
    // Then
    assert_eq!(state.toast(), None);
}

#[test]
fn an_obsolete_whitespace_change_does_not_replace_the_current_toast() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    apply_whitespace_toggle(&mut state, now);
    state.action(Action::Whitespace, 10, now);
    let obsolete = state.collection_request();
    state.action(Action::Whitespace, 10, now);
    state.collection_request();
    // When
    state.collection_finished(
        obsolete.generation,
        Ok(Snapshot {
            whitespace: Whitespace::Exact,
            ..Snapshot::default()
        }),
        now,
    );
    // Then
    assert_eq!(
        state.toast(),
        Some("Whitespace: ignore-changes · 0 hidden by whitespace")
    );
}

#[test]
fn rapid_toggles_returning_to_the_applied_mode_are_silent() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    state.action(Action::Whitespace, 10, now);
    state.collection_request();
    state.action(Action::Whitespace, 10, now);
    let request = state.collection_request();
    // When
    state.collection_finished(request.generation, Ok(Snapshot::default()), now);
    // Then
    assert_eq!(state.toast(), None);
}

#[test]
fn repository_refreshes_do_not_restart_a_whitespace_toast() {
    // Given
    let now = Instant::now();
    let mut state = State::new(false);
    state.action(Action::Whitespace, 10, now);
    let request = state.collection_request();
    state.collection_finished(
        request.generation,
        Ok(Snapshot {
            whitespace: Whitespace::IgnoreChanges,
            whitespace_hidden_files: 1,
            ..Snapshot::default()
        }),
        now,
    );
    // When
    state.collection_finished(
        request.generation,
        Ok(Snapshot {
            whitespace: Whitespace::IgnoreChanges,
            branch: "refreshed".into(),
            ..Snapshot::default()
        }),
        now + Duration::from_secs(1),
    );
    // Then
    assert_eq!(
        state.toast(),
        Some("Whitespace: ignore-changes · 1 hidden by whitespace")
    );
    assert_eq!(
        state.toast_delay(now + Duration::from_secs(1)),
        Some(Duration::from_secs(2))
    );
}

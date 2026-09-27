use super::*;
use crate::model::LineStats;

fn counted_state(added: u64, removed: u64) -> State {
    let mut state = state_with("src/main.rs", 1);
    state.update_snapshot(Snapshot {
        changes: state.snapshot().changes.clone(),
        tracked_stats: Some(LineStats { added, removed }),
        ..Snapshot::default()
    });
    state
}

macro_rules! theme_case {
    ($name:ident, $light:expr) => {
        #[test]
        fn $name() -> AppResult<()> {
            // Given
            let state = counted_state(1_813, 17);
            let config = Config {
                light: $light,
                ..Config::default()
            };
            let palette = Palette::new($light);
            // When
            let buffer = render(&state, &config, 80, 10)?;
            // Then
            assert!(line(&buffer, 8).ends_with("Tracked +1,813 −17 ■■■■■ "));
            assert_eq!(buffer[(55, 8)].fg, palette.muted);
            assert_eq!(buffer[(63, 8)].fg, palette.added_foreground);
            assert_eq!(buffer[(70, 8)].fg, palette.removed_foreground);
            assert_eq!(buffer[(63, 8)].modifier, Modifier::empty());
            assert_eq!(buffer[(63, 8)].bg, palette.background);
            assert!(line(&buffer, 9).starts_with(" n/p files"));
            Ok(())
        }
    };
}

theme_case!(dark_totals_are_right_aligned_and_use_the_diff_colors, false);
theme_case!(light_totals_are_right_aligned_and_use_the_diff_colors, true);

macro_rules! ratio_case {
    ($name:ident, $added:expr, $removed:expr, $colors:expr) => {
        #[test]
        fn $name() -> AppResult<()> {
            // Given
            let state = counted_state($added, $removed);
            let palette = Palette::new(false);
            let green = palette.added_foreground;
            let red = palette.removed_foreground;
            let muted = palette.muted;
            let expected: [Color; 5] = $colors(green, red, muted);
            // When
            let buffer = render(&state, &Config::default(), 100, 10)?;
            // Then
            assert_eq!(
                [
                    buffer[(94, 8)].fg,
                    buffer[(95, 8)].fg,
                    buffer[(96, 8)].fg,
                    buffer[(97, 8)].fg,
                    buffer[(98, 8)].fg
                ],
                expected
            );
            assert!(line(&buffer, 8).ends_with("■■■■■ "));
            Ok(())
        }
    };
}

ratio_case!(zero_totals_use_muted_blocks, 0, 0, |_, _, m| [
    m, m, m, m, m
]);
ratio_case!(only_additions_use_green_blocks, 5, 0, |g, _, _| [
    g, g, g, g, g
]);
ratio_case!(only_deletions_use_red_blocks, 0, 5, |_, r, _| [
    r, r, r, r, r
]);
ratio_case!(equal_totals_round_the_green_share_up, 10, 10, |g, r, _| [
    g, g, g, r, r
]);
ratio_case!(the_bar_is_proportional_to_the_counts, 2, 3, |g, r, _| [
    g, g, r, r, r
]);
ratio_case!(small_deletions_remain_visible, 1_813, 17, |g, r, _| [
    g, g, g, g, r
]);
ratio_case!(small_additions_remain_visible, 1, 1_000, |g, r, _| [
    g, r, r, r, r
]);
ratio_case!(
    maximum_totals_do_not_overflow_the_ratio,
    u64::MAX,
    u64::MAX,
    |g, r, _| [g, g, g, r, r]
);

#[test]
fn narrow_status_rows_drop_the_blocks_before_the_counts() -> AppResult<()> {
    // Given
    let state = counted_state(1_813, 17);
    // When
    let buffer = render(&state, &Config::default(), 35, 10)?;
    // Then
    assert!(line(&buffer, 8).starts_with(" Watching"));
    assert!(line(&buffer, 8).ends_with("Tracked +1,813 −17 "));
    assert!(!line(&buffer, 8).contains('■'));
    Ok(())
}

#[test]
fn the_counts_preserve_a_two_space_gap_at_the_smallest_fitting_width() -> AppResult<()> {
    // Given
    let state = counted_state(1_813, 17);
    // When
    let buffer = render(&state, &Config::default(), 30, 10)?;
    // Then
    assert_eq!(line(&buffer, 8), " Watching  Tracked +1,813 −17 ");
    Ok(())
}

#[test]
fn cramped_status_rows_preserve_activity_and_hide_the_whole_indicator() -> AppResult<()> {
    // Given
    let state = counted_state(1_813, 17);
    // When
    let buffer = render(&state, &Config::default(), 29, 10)?;
    // Then
    assert_eq!(line(&buffer, 8).trim(), "Watching");
    Ok(())
}

#[test]
fn no_totals_are_shown_before_a_snapshot_is_collected() -> AppResult<()> {
    // Given
    let state = State::new(false);
    // When
    let buffer = render(&state, &Config::default(), 80, 10)?;
    // Then
    assert!(!line(&buffer, 8).contains("Tracked"));
    assert!(!line(&buffer, 8).contains('■'));
    Ok(())
}

#[test]
fn full_file_view_hides_tracked_counts_and_the_ratio_bar() -> AppResult<()> {
    // Given: a historical full-file view with stale diff totals.
    let mut state = counted_state(1_813, 17);
    state.set_presentation(crate::model::Presentation::Files);
    // When: rendering its status line.
    let buffer = render(&state, &Config::default(), 80, 10)?;
    // Then: neither the tracked counts nor the ratio bar appears.
    assert!(!line(&buffer, 8).contains("Tracked"));
    assert!(!line(&buffer, 8).contains('■'));
    Ok(())
}

#[test]
fn selection_filter_and_search_do_not_change_repo_totals() -> AppResult<()> {
    // Given
    let mut state = counted_state(1_813, 17);
    let now = Instant::now();
    state.action(Action::NextFile, 7, now);
    state.action(Action::Filter, 7, now);
    state.paste("missing", now);
    state.action(Action::Accept, 7, now);
    state.action(Action::Search, 7, now);
    state.paste("query", now);
    state.action(Action::Accept, 7, now);
    prepare(&mut state, 120, 10)?;
    // When
    let buffer = render(&state, &Config::default(), 120, 10)?;
    // Then
    assert!(line(&buffer, 8).contains("filter: missing · /query: 0 matches"));
    assert!(line(&buffer, 8).ends_with("Tracked +1,813 −17 ■■■■■ "));
    Ok(())
}

#[test]
fn long_queries_keep_their_space_when_totals_do_not_fit() -> AppResult<()> {
    // Given
    let mut state = counted_state(1_813, 17);
    state.action(Action::Filter, 7, Instant::now());
    state.paste("a long filter query", Instant::now());
    state.action(Action::Accept, 7, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 50, 10)?;
    // Then
    assert_eq!(
        line(&buffer, 8).trim(),
        "Watching · filter: a long filter query"
    );
    Ok(())
}

#[test]
fn wide_query_characters_are_measured_in_terminal_cells() -> AppResult<()> {
    // Given
    let mut state = counted_state(1_813, 17);
    state.action(Action::Filter, 7, Instant::now());
    state.paste("界界界界界", Instant::now());
    state.action(Action::Accept, 7, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 45, 10)?;
    // Then
    assert!(!line(&buffer, 8).contains("Tracked"));
    assert_eq!(buffer[(28, 8)].symbol(), "界");
    Ok(())
}

#[test]
fn refreshing_keeps_the_last_successful_totals_visible() -> AppResult<()> {
    // Given
    let mut state = counted_state(1_813, 17);
    state.collection_request();
    // When
    let buffer = render(&state, &Config::default(), 80, 10)?;
    // Then
    assert!(line(&buffer, 8).starts_with(" Refreshing"));
    assert!(line(&buffer, 8).ends_with("Tracked +1,813 −17 ■■■■■ "));
    Ok(())
}

#[test]
fn errors_hide_even_previously_collected_totals() -> AppResult<()> {
    // Given
    let mut state = counted_state(1_813, 17);
    let request = state.collection_request();
    state.collection_finished(
        request.generation,
        Err(GitError::Command("cannot compare".into())),
        Instant::now(),
    );
    // When
    let buffer = render(&state, &Config::default(), 80, 10)?;
    // Then
    assert_eq!(line(&buffer, 8).trim(), "Error: cannot compare");
    Ok(())
}

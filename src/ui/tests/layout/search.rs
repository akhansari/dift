use super::*;
use ratatui::buffer::Cell;

fn searching(text: &str, query: &str, split: bool, width: u16) -> AppResult<State> {
    let mut change = state_with("main.rs", 1).snapshot().changes[0].clone();
    change.hunks[0].lines[0].text = text.into();
    let snapshot = Snapshot {
        changes: vec![change],
        ..Snapshot::default()
    };
    let mut state = State::new(split);
    state.update_snapshot(snapshot);
    state.action(Action::Sidebar, 9, Instant::now());
    state.action(Action::Search, 9, Instant::now());
    state.paste(query, Instant::now());
    state.action(Action::Accept, 9, Instant::now());
    prepare(&mut state, width, 12)?;
    Ok(state)
}

fn at<'a>(buffer: &'a Buffer, text: &str, offset: u16) -> AppResult<&'a Cell> {
    let (y, x) = (0..buffer.area.height)
        .find_map(|y| {
            let row = line(buffer, y);
            row.find(text).map(|byte| (y, row[..byte].width() as u16))
        })
        .ok_or("Missing rendered search text")?;
    Ok(&buffer[(x + offset, y)])
}

#[test]
fn current_occurrence_is_orange_and_other_occurrences_are_yellow() -> AppResult<()> {
    // Given
    let state = searching("needle needle", "needle", false, 80)?;
    // When
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(at(&buffer, "needle needle", 0)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "needle needle", 0)?.fg, Color::White);
    assert_eq!(
        at(&buffer, "needle needle", 7)?.bg,
        Color::Rgb(237, 197, 102)
    );
    Ok(())
}

#[test]
fn next_match_moves_color_between_occurrences_on_the_same_row() -> AppResult<()> {
    // Given
    let mut state = searching("needle needle", "needle", false, 80)?;
    // When
    let effects = state.action(Action::NextMatch, 9, Instant::now());
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert!(!effects.render);
    assert_eq!(
        at(&buffer, "needle needle", 0)?.bg,
        Color::Rgb(237, 197, 102)
    );
    assert_eq!(at(&buffer, "needle needle", 7)?.bg, Color::Rgb(184, 92, 31));
    Ok(())
}

#[test]
fn previous_match_wraps_to_the_last_occurrence() -> AppResult<()> {
    // Given
    let mut state = searching("needle needle", "needle", false, 80)?;
    // When
    state.action(Action::PreviousMatch, 9, Instant::now());
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(
        at(&buffer, "needle needle", 0)?.bg,
        Color::Rgb(237, 197, 102)
    );
    assert_eq!(at(&buffer, "needle needle", 7)?.bg, Color::Rgb(184, 92, 31));
    Ok(())
}

#[test]
fn next_match_wraps_back_to_the_first_occurrence() -> AppResult<()> {
    // Given
    let mut state = searching("needle needle", "needle", false, 80)?;
    state.action(Action::PreviousMatch, 9, Instant::now());
    // When
    state.action(Action::NextMatch, 9, Instant::now());
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(at(&buffer, "needle needle", 0)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(
        at(&buffer, "needle needle", 7)?.bg,
        Color::Rgb(237, 197, 102)
    );
    Ok(())
}

#[test]
fn light_mode_current_match_uses_white_on_orange() -> AppResult<()> {
    // Given
    let state = searching("needle needle", "needle", false, 80)?;
    let config = Config {
        light: true,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 80, 12)?;
    // Then
    assert_eq!(at(&buffer, "needle needle", 0)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "needle needle", 0)?.fg, Color::White);
    assert_eq!(
        at(&buffer, "needle needle", 7)?.bg,
        Color::Rgb(237, 197, 102)
    );
    Ok(())
}

#[test]
fn all_wrapped_fragments_of_the_current_match_are_orange() -> AppResult<()> {
    // Given
    let state = searching("123456789needle", "needle", false, 20)?;
    // When
    let buffer = render(&state, &Config::default(), 20, 12)?;
    // Then
    assert_eq!(at(&buffer, "123456789ne", 9)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "edle", 0)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "edle", 3)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(state.document().hits.len(), 1);
    Ok(())
}

#[test]
fn split_view_current_match_uses_the_right_pane_columns() -> AppResult<()> {
    // Given
    let state = searching("needle needle", "needle", true, 80)?;
    // When
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(at(&buffer, "needle needle", 0)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(
        at(&buffer, "needle needle", 7)?.bg,
        Color::Rgb(237, 197, 102)
    );
    Ok(())
}

#[test]
fn clearing_search_removes_the_current_highlight() -> AppResult<()> {
    // Given
    let mut state = searching("needle needle", "needle", false, 80)?;
    // When
    state.action(Action::Cancel, 9, Instant::now());
    prepare(&mut state, 80, 12)?;
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(
        at(&buffer, "needle needle", 0)?.bg,
        Palette::new(false).added
    );
    assert!(state.document().hits.is_empty());
    Ok(())
}

#[test]
fn unmatched_search_leaves_source_colors_intact() -> AppResult<()> {
    // Given
    let state = searching("needle needle", "missing", false, 80)?;
    // When
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(
        at(&buffer, "needle needle", 0)?.bg,
        Palette::new(false).added
    );
    assert!(state.document().hits.is_empty());
    Ok(())
}

#[test]
fn split_view_highlights_all_wrapped_fragments() -> AppResult<()> {
    // Given
    let state = searching("123456789needle", "needle", true, 40)?;
    // When
    let buffer = render(&state, &Config::default(), 40, 12)?;
    // Then
    assert_eq!(at(&buffer, "123456789ne", 9)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "edle", 0)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "edle", 3)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(state.document().hits.len(), 1);
    Ok(())
}

#[test]
fn current_unicode_match_colors_wide_and_combining_characters() -> AppResult<()> {
    // Given
    let state = searching("\tÉ界e\u{301}", "é界e\u{301}", false, 80)?;
    // When
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(at(&buffer, "É", 0)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "É", 1)?.bg, Color::Rgb(184, 92, 31));
    assert_eq!(at(&buffer, "É", 1)?.symbol(), "界");
    assert_eq!(at(&buffer, "É", 3)?.bg, Color::Rgb(184, 92, 31));
    Ok(())
}

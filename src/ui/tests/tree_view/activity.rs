use super::*;
use crate::model::{ActivityObservation, GitIdentity};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

fn snapshot(source: &str) -> Snapshot {
    Snapshot {
        activity: Some(BTreeMap::from([(
            "src/a.rs".into(),
            BTreeMap::from([(
                Category::Unstaged,
                ActivityObservation {
                    git: GitIdentity {
                        status: 'M',
                        ..GitIdentity::default()
                    },
                    metadata: None,
                    source: Some(Arc::from(source)),
                },
            )]),
        )])),
        changes: vec![Change {
            path: "src/a.rs".into(),
            old_path: None,
            category: Category::Unstaged,
            status: 'M',
            hunks: Vec::new(),
            summary: None,
            oversized: false,
            old_source: None,
            new_source: Some(Arc::from(source)),
        }],
        ..Snapshot::default()
    }
}

fn active(now: Instant, selected: bool) -> State {
    let mut state = State::new(false);
    let request = state.collection_request();
    state.collection_finished(request.generation, Ok(snapshot("before")), now);
    if selected {
        state.action(Action::NextFile, 10, now);
    }
    let request = state.collection_request();
    state.collection_finished(request.generation, Ok(snapshot("after")), now);
    state.advance_activity(now + Duration::from_secs(1));
    state
}

#[test]
fn dark_activity_colors_the_whole_row_and_preserves_badges() -> crate::AppResult<()> {
    // Given
    let now = Instant::now();
    let state = active(now, false);
    // When
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(buffer[(0, 2)].bg, Color::Rgb(65, 49, 27));
    assert_eq!(buffer[(31, 2)].bg, buffer[(0, 2)].bg);
    assert_eq!(buffer[(31, 2)].symbol(), "U");
    assert_eq!(buffer[(31, 2)].fg, Color::Rgb(205, 166, 92));
    assert_ne!(buffer[(10, 1)].bg, buffer[(10, 2)].bg);
    assert_ne!(buffer[(10, 1)].bg, Color::Reset);
    Ok(())
}

#[test]
fn light_activity_preserves_the_selected_files_blue_focus_marker() -> crate::AppResult<()> {
    // Given
    let now = Instant::now();
    let state = active(now, true);
    let config = Config {
        light: true,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 34, 8)?;
    // Then
    assert_eq!(buffer[(0, 2)].bg, Color::Rgb(248, 228, 190));
    assert_eq!(buffer[(0, 2)].symbol(), "▎");
    assert_eq!(buffer[(0, 2)].fg, Palette::new(true).accent);
    assert_eq!(buffer[(7, 2)].fg, Palette::new(true).accent);
    Ok(())
}

#[test]
fn expiry_restores_the_native_background() -> crate::AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = active(now, false);
    // When
    state.advance_activity(now + Duration::from_secs(16));
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(buffer[(10, 2)].bg, Color::Reset);
    assert_eq!(buffer[(10, 1)].bg, Color::Reset);
    Ok(())
}

#[test]
fn expiry_restores_the_selected_row_background() -> crate::AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = active(now, true);
    // When
    state.advance_activity(now + Duration::from_secs(16));
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(buffer[(10, 2)].bg, Color::Rgb(35, 48, 68));
    assert_eq!(buffer[(0, 2)].symbol(), "▎");
    Ok(())
}

#[test]
fn monochrome_has_no_visible_activity_signal() -> crate::AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = active(now, true);
    let mut pulsing = render(&state, &Config::default(), 34, 8)?;
    state.advance_activity(now + Duration::from_secs(16));
    let mut expired = render(&state, &Config::default(), 34, 8)?;
    // When
    crate::ui::suppress_colors(&mut pulsing);
    crate::ui::suppress_colors(&mut expired);
    // Then
    assert_eq!(pulsing, expired);
    Ok(())
}

#[test]
fn mouse_visits_start_the_same_short_fade_as_keyboard_visits() -> crate::AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = active(now, true);
    let mut viewport = tree::Viewport::default();
    // When
    viewport.handle_mouse(
        &mut state,
        &Config::default(),
        Rect::new(0, 0, 34, 8),
        MouseEvent {
            kind: MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
            column: 8,
            row: 2,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        },
        now + Duration::from_secs(1),
    );
    state.advance_activity(now + Duration::from_millis(1200));
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(state.selection(), Path::new("src/a.rs"));
    assert_eq!(buffer[(10, 2)].bg, Color::Rgb(35, 48, 68));
    Ok(())
}

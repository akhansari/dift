use super::*;
use ratatui::crossterm::event::{KeyModifiers, MouseButton};
use std::path::Path;

fn document(lines: usize) -> AppResult<State> {
    let mut state = state_with("main.rs", lines);
    prepare(&mut state, 90, 12)?;
    Ok(state)
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn wheel(state: &mut State, kind: MouseEventKind, column: u16, row: u16) -> Effects {
    ViewAreas::new(Rect::new(0, 0, 90, 12), state.sidebar()).handle_mouse(
        state,
        &Config::default(),
        &mut Interaction::default(),
        mouse(kind, column, row),
        Instant::now(),
    )
}

#[test]
fn wheel_over_diff_scrolls_down_while_tree_has_focus() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), 3);
    assert_eq!(state.focus(), Scope::Tree);
    assert_eq!(state.selection(), Path::new(""));
    assert_eq!(
        effects,
        Effects {
            redraw: true,
            ..Effects::default()
        }
    );
    Ok(())
}

#[test]
fn wheel_over_diff_scrolls_up_while_tree_has_focus() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.scroll_by(6, true, 9);
    // When
    wheel(&mut state, MouseEventKind::ScrollUp, 40, 2);
    // Then
    assert_eq!(state.scroll(), 3);
    assert_eq!(state.focus(), Scope::Tree);
    Ok(())
}

#[test]
fn wheel_over_focused_diff_scrolls_down() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.action(Action::Focus, 9, Instant::now());
    // When
    wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), 3);
    assert_eq!(state.focus(), Scope::Diff);
    Ok(())
}

#[test]
fn wheel_over_focused_diff_scrolls_up() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.action(Action::Focus, 9, Instant::now());
    state.scroll_by(6, true, 9);
    // When
    wheel(&mut state, MouseEventKind::ScrollUp, 40, 2);
    // Then
    assert_eq!(state.scroll(), 3);
    assert_eq!(state.focus(), Scope::Diff);
    Ok(())
}

#[test]
fn wheel_at_diff_top_does_not_redraw() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollUp, 40, 2);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn wheel_clamps_to_the_last_full_diff_viewport() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    let maximum = state.document().rows.len() - 9;
    state.scroll_by(maximum - 1, true, 9);
    // When
    wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), maximum);
    Ok(())
}

#[test]
fn wheel_at_diff_bottom_does_not_redraw() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.scroll_by(usize::MAX, true, 9);
    let previous = state.scroll();
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), previous);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn a_diff_that_fits_does_not_scroll() -> AppResult<()> {
    // Given
    let mut state = document(1)?;
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn diff_scrolling_works_in_the_former_tree_area_when_sidebar_is_hidden() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.action(Action::Sidebar, 9, Instant::now());
    // When
    wheel(&mut state, MouseEventKind::ScrollDown, 5, 2);
    // Then
    assert_eq!(state.scroll(), 3);
    assert_eq!(state.focus(), Scope::Diff);
    Ok(())
}

#[test]
fn help_blocks_diff_wheel_scrolling() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.action(Action::Help, 9, Instant::now());
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(state.help_scroll(), 3);
    assert!(effects.redraw);
    Ok(())
}

#[test]
fn filter_prompt_blocks_diff_wheel_scrolling() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.action(Action::Filter, 9, Instant::now());
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn search_prompt_blocks_diff_wheel_scrolling() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.action(Action::Search, 9, Instant::now());
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 2);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn status_bar_wheel_events_do_not_scroll_the_diff() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 10);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn footer_wheel_events_do_not_scroll_the_diff() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 40, 11);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn wheel_over_tree_scrolls_only_tree_even_when_diff_has_focus() -> AppResult<()> {
    // Given
    let mut state = state_with("a/b/c/d/e/f/g.rs", 40);
    prepare(&mut state, 90, 6)?;
    state.action(Action::Focus, 3, Instant::now());
    let view = ViewAreas::new(Rect::new(0, 0, 90, 6), true);
    let config = Config::default();
    let mut viewport = Interaction::default();
    // When
    let effects = view.handle_mouse(
        &mut state,
        &config,
        &mut viewport,
        mouse(MouseEventKind::ScrollDown, 10, 1),
        Instant::now(),
    );
    let mut terminal = Terminal::new(TestBackend::new(90, 6))?;
    terminal.draw(|frame| {
        tree::draw(frame, &state, &config, view.tree, &mut viewport.tree);
    })?;
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(state.focus(), Scope::Diff);
    assert_eq!(state.selection(), Path::new(""));
    assert_eq!(
        effects,
        Effects {
            redraw: true,
            ..Effects::default()
        }
    );
    assert_eq!(terminal.backend().buffer()[(9, 0)].symbol(), "c");
    Ok(())
}

#[test]
fn clicking_diff_does_not_scroll_it() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    // When
    let effects = wheel(&mut state, MouseEventKind::Down(MouseButton::Left), 40, 2);
    // Then
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn wheel_outside_help_does_not_scroll_either_view() -> AppResult<()> {
    // Given
    let mut state = document(40)?;
    state.action(Action::Help, 9, Instant::now());
    // When
    let effects = wheel(&mut state, MouseEventKind::ScrollDown, 0, 0);
    // Then
    assert_eq!(state.help_scroll(), 0);
    assert_eq!(state.scroll(), 0);
    assert_eq!(effects, Effects::default());
    Ok(())
}

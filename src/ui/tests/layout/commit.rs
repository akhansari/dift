use super::*;

#[test]
fn commit_dialog_shows_only_message_fields_and_controls() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.begin_commit();
    state.paste("Subject text", Instant::now());
    state.action(Action::Accept, 20, Instant::now());
    state.paste("Body line one\nBody line two", Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 80, 24)?;
    // Then
    let text = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains(" Commit "));
    assert!(text.contains("Subject text"));
    assert!(text.contains("Body line one"));
    assert!(text.contains("Body line two"));
    assert!(line(&buffer, 20).contains("ctrl-s commit · tab switch field · esc close"));
    assert_eq!(buffer[(2, 19)].symbol(), "└");
    assert!(!text.contains("files staged"));
    Ok(())
}

#[test]
fn commit_notice_replaces_bindings_in_the_footer() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.begin_commit();
    state.commit_message();
    // When
    let buffer = render(&state, &Config::default(), 80, 24)?;
    // Then
    assert!(line(&buffer, 20).contains("Enter a commit subject"));
    assert!(!line(&buffer, 20).contains("ctrl-s"));
    assert_eq!(buffer[(2, 20)].fg, Palette::new(false).removed_emphasis);
    assert_eq!(buffer[(2, 19)].symbol(), "└");
    Ok(())
}

#[test]
fn running_commit_replaces_bindings_in_the_footer() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.begin_commit();
    state.commit_started();
    // When
    let buffer = render(&state, &Config::default(), 80, 24)?;
    // Then
    assert!(line(&buffer, 20).contains("Committing…"));
    assert!(!line(&buffer, 20).contains("ctrl-s"));
    assert_eq!(buffer[(2, 20)].fg, Palette::new(false).accent);
    assert_eq!(buffer[(2, 19)].symbol(), "└");
    Ok(())
}

#[test]
fn narrow_commit_footer_clips_bindings_to_one_row() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.begin_commit();
    // When
    let buffer = render(&state, &Config::default(), 40, 14)?;
    // Then
    assert!(line(&buffer, 11).contains("│ctrl-s commit · tab switch field · e│"));
    assert_eq!(buffer[(2, 10)].symbol(), "└");
    assert_eq!(buffer[(1, 12)].symbol(), "└");
    Ok(())
}

#[test]
fn long_multiline_body_keeps_the_cursor_and_current_line_visible() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.begin_commit();
    state.paste("Subject", Instant::now());
    state.action(Action::Accept, 20, Instant::now());
    state.paste(
        &format!("{}{}END", "earlier\n".repeat(40), "界".repeat(60)),
        Instant::now(),
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 18))?;
    // When
    draw_into(&mut terminal, &state, &Config::default())?;
    // Then
    let cursor = terminal.get_cursor_position()?;
    let buffer = terminal.backend().buffer();
    assert!(buffer.area.contains(cursor));
    assert!(line(buffer, cursor.y).contains("END"));
    Ok(())
}

#[test]
fn small_terminal_explains_how_to_close_the_dialog() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    state.begin_commit();
    // When
    let buffer = render(&state, &Config::default(), 38, 12)?;
    // Then
    let text = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Enlarge terminal"));
    assert!(text.contains("Esc closes"));
    Ok(())
}

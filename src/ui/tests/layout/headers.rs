use super::*;

fn without_sidebar(mut state: State) -> State {
    state.action(Action::Sidebar, 7, Instant::now());
    state
}

fn two_files(category: Category) -> State {
    let first = state_with("first.rs", 2).snapshot().changes[0].clone();
    let mut second = state_with("second.rs", 40).snapshot().changes[0].clone();
    second.category = category;
    let mut state = State::new(false);
    state.update_snapshot(Snapshot {
        branch: "main".into(),
        changes: vec![first, second],
        ..Snapshot::default()
    });
    without_sidebar(state)
}

fn markers(buffer: &Buffer, x: u16) -> Vec<u16> {
    (0..buffer.area.height)
        .filter(|y| buffer[(x, *y)].symbol() == "▎")
        .collect()
}

#[test]
fn scrolling_keeps_the_file_header_above_visible_source() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 60, 10)?;
    state.scroll_by(4, true, 7);
    // When
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 0).contains("Modified · Unstaged"));
    assert!(line(&buffer, 1).starts_with(&"─".repeat(59)));
    assert!(line(&buffer, 2).contains("row 3"));
    assert!(line(&buffer, 6).contains("row 7"));
    assert_eq!(buffer[(0, 1)].fg, Palette::new(false).muted);
    assert_eq!(markers(&buffer, 0), vec![0]);
    assert_eq!(buffer[(0, 0)].fg, Palette::new(false).accent);
    assert_eq!(buffer[(0, 0)].bg, Palette::new(false).focused);
    assert_eq!(buffer[(2, 0)].bg, Palette::new(false).focused);
    assert_eq!(buffer[(58, 0)].bg, Palette::new(false).focused);
    assert_eq!(buffer[(59, 0)].bg, Palette::new(false).background);
    Ok(())
}

#[test]
fn only_the_first_visible_file_header_is_marked_before_scrolling() -> AppResult<()> {
    // Given
    let mut state = two_files(Category::Unstaged);
    prepare(&mut state, 60, 12)?;
    // When
    let buffer = render(&state, &Config::default(), 60, 12)?;
    // Then
    assert!(line(&buffer, 0).contains("UNSTAGED"));
    assert!(line(&buffer, 1).contains("first.rs"));
    assert!(line(&buffer, 7).contains("second.rs"));
    assert_eq!(markers(&buffer, 0), vec![1]);
    assert_eq!(buffer[(2, 1)].bg, Palette::new(false).focused);
    assert_eq!(buffer[(2, 7)].bg, Palette::new(false).panel);
    Ok(())
}

#[test]
fn reaching_the_next_file_replaces_the_sticky_header_without_duplication() -> AppResult<()> {
    // Given
    let mut state = two_files(Category::Unstaged);
    prepare(&mut state, 60, 10)?;
    state.scroll_by(7, true, 7);
    // When
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("second.rs"));
    assert!(line(&buffer, 1).starts_with(&"─".repeat(59)));
    assert!(line(&buffer, 2).contains("@@"));
    assert!(line(&buffer, 3).contains("row 1"));
    assert_eq!(markers(&buffer, 0), vec![0]);
    Ok(())
}

#[test]
fn a_category_heading_clears_the_previous_sticky_header() -> AppResult<()> {
    // Given
    let mut state = two_files(Category::Staged);
    prepare(&mut state, 60, 10)?;
    state.scroll_by(8, true, 7);
    // When
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("STAGED"));
    assert!(line(&buffer, 1).contains("second.rs"));
    assert!(line(&buffer, 1).contains("Modified · Staged"));
    assert_eq!(markers(&buffer, 0), vec![1]);
    Ok(())
}

#[test]
fn returning_focus_to_the_tree_clears_the_marker_without_shifting_the_header() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 40);
    state.action(Action::Focus, 7, Instant::now());
    prepare(&mut state, 120, 10)?;
    state.scroll_by(4, true, 7);
    let mut terminal = Terminal::new(TestBackend::new(120, 10))?;
    draw_into(&mut terminal, &state, &Config::default())?;
    let focused = line(terminal.backend().buffer(), 0);
    // When
    state.action(Action::Focus, 7, Instant::now());
    draw_into(&mut terminal, &state, &Config::default())?;
    // Then
    let buffer = terminal.backend().buffer();
    let unfocused = line(buffer, 0);
    assert_eq!(
        focused.chars().skip(35).collect::<String>(),
        unfocused.chars().skip(35).collect::<String>()
    );
    assert_eq!(buffer[(34, 0)].symbol(), " ");
    assert_eq!(markers(buffer, 34), Vec::<u16>::new());
    assert_eq!(buffer[(0, 0)].symbol(), "▎");
    assert_eq!(buffer[(34, 0)].bg, Palette::new(false).panel);
    assert_eq!(buffer[(36, 0)].bg, Palette::new(false).panel);
    Ok(())
}

#[test]
fn split_diffs_keep_the_same_header_above_the_inner_divider() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    state.action(Action::Layout, 7, Instant::now());
    prepare(&mut state, 65, 10)?;
    state.scroll_by(4, true, 7);
    // When
    let buffer = render(&state, &Config::default(), 65, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(!line(&buffer, 0).contains('│'));
    assert!(line(&buffer, 2).contains('│'));
    assert!(line(&buffer, 2).contains("row 3"));
    assert_eq!(markers(&buffer, 0), vec![0]);
    Ok(())
}

#[test]
fn light_headers_emphasize_the_basename_and_color_the_counts() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("src/ui/main.rs", 40));
    let config = Config {
        light: true,
        ..Config::default()
    };
    let request = state.render_request(79, 7);
    let document = Renderer::new(config.clone())?.document(
        &request.snapshot,
        &request.selected,
        79,
        false,
        "",
    )?;
    state.apply_render(
        RenderResult {
            generation: request.generation,
            document,
            finished: true,
        },
        7,
    );
    // When
    let buffer = render(&state, &config, 80, 10)?;
    // Then
    let palette = Palette::new(true);
    assert_eq!(buffer[(2, 1)].fg, palette.muted);
    assert!(!buffer[(2, 1)].modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[(9, 1)].fg, palette.foreground);
    assert!(buffer[(9, 1)].modifier.contains(Modifier::BOLD));
    assert_eq!(buffer[(72, 1)].fg, palette.added_foreground);
    assert_eq!(buffer[(76, 1)].fg, palette.removed_foreground);
    assert_eq!(buffer[(0, 1)].fg, palette.accent);
    assert_eq!(buffer[(0, 1)].bg, palette.focused);
    assert_eq!(buffer[(9, 1)].bg, palette.focused);
    assert_eq!(buffer[(78, 1)].bg, palette.focused);
    assert_eq!(line(&buffer, 2).trim(), "─".repeat(79));
    assert_eq!(buffer[(0, 2)].fg, palette.muted);
    assert_eq!(buffer[(0, 2)].bg, palette.background);
    Ok(())
}

#[test]
fn the_minimum_terminal_keeps_a_sticky_filename_and_source_visible() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 12, 6)?;
    state.scroll_by(4, true, 3);
    // When
    let buffer = render(&state, &Config::default(), 12, 6)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert_eq!(line(&buffer, 1).trim(), "─".repeat(11));
    assert!(line(&buffer, 2).contains("row 3"));
    assert_eq!(line(&buffer, 3), "─".repeat(12));
    Ok(())
}

#[test]
fn jumping_to_the_end_keeps_the_last_source_line_accessible() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 60, 10)?;
    // When
    state.action(Action::Last, 7, Instant::now());
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 6).contains("row 40"));
    assert_eq!(buffer[(59, 6)].symbol(), "┃");
    Ok(())
}

#[test]
fn search_matches_remain_below_the_sticky_header() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    state.action(Action::Search, 3, Instant::now());
    state.paste("row 20", Instant::now());
    // When
    prepare(&mut state, 60, 6)?;
    let buffer = render(&state, &Config::default(), 60, 6)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 2).contains("row 20"));
    assert_eq!(buffer[(14, 2)].bg, Color::Rgb(184, 92, 31));
    assert_ne!(buffer[(14, 0)].bg, Color::Rgb(184, 92, 31));
    Ok(())
}

#[test]
fn resizing_a_scrolled_diff_preserves_the_header_and_reading_position() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 80, 10)?;
    state.scroll_by(4, true, 7);
    // When
    prepare(&mut state, 60, 10)?;
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 2).contains("row 3"));
    assert_eq!(markers(&buffer, 0), vec![0]);
    Ok(())
}

#[test]
fn empty_diffs_have_no_file_focus_marker() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(State::new(false));
    prepare(&mut state, 60, 10)?;
    // When
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("No changes in this view"));
    assert_eq!(markers(&buffer, 0), Vec::<u16>::new());
    Ok(())
}

#[test]
fn files_without_source_anchors_still_have_sticky_headers() -> AppResult<()> {
    // Given
    let mut change = state_with("image.png", 1).snapshot().changes[0].clone();
    change.hunks.clear();
    change.summary = Some("Binary file changed\nDetails\nBefore\nAfter\nMetadata\nEnd".into());
    let mut state = without_sidebar(State::new(false));
    state.update_snapshot(Snapshot {
        changes: vec![change],
        ..Snapshot::default()
    });
    prepare(&mut state, 60, 6)?;
    state.scroll_by(4, true, 3);
    // When
    let buffer = render(&state, &Config::default(), 60, 6)?;
    // Then
    assert!(line(&buffer, 0).contains("image.png"));
    assert!(!line(&buffer, 0).contains('+'));
    assert!(line(&buffer, 2).contains("After"));
    assert_eq!(markers(&buffer, 0), vec![0]);
    Ok(())
}

#[test]
fn partial_previews_keep_the_header_and_full_loaded_line_counts() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    let request = state.render_request(59, 3);
    let mut renderer = Renderer::new(Config::default())?;
    let result = renderer.render_progressive(
        crate::render::RenderSpec {
            snapshot: &request.snapshot,
            selected: &request.selected,
            width: 59,
            split: false,
            show_line_numbers: true,
            query: "",
            preview_rows: 6,
        },
        &crate::render::Cancellation::default(),
        |update| {
            state.apply_render(
                RenderResult {
                    generation: request.generation,
                    document: update.document,
                    finished: update.finished,
                },
                3,
            );
            false
        },
    );
    // When
    state.scroll_by(2, true, 3);
    let buffer = render(&state, &Config::default(), 60, 6)?;
    // Then
    assert_eq!(result, Err(crate::render::Cancelled));
    assert!(state.document().partial);
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 0).contains("+40 −0"));
    assert!(line(&buffer, 2).contains("row 1"));
    Ok(())
}

#[test]
fn a_refresh_updates_the_sticky_header_while_preserving_source_context() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 60, 10)?;
    state.scroll_by(4, true, 7);
    let mut change = state.snapshot().changes[0].clone();
    change.category = Category::Staged;
    change.status = 'A';
    // When
    state.update_snapshot(Snapshot {
        changes: vec![change],
        ..Snapshot::default()
    });
    prepare(&mut state, 60, 10)?;
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 0).contains("Added · Staged"));
    assert!(line(&buffer, 2).contains("row 3"));
    Ok(())
}

#[test]
fn paging_down_from_a_sticky_header_starts_with_the_next_unread_line() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 60, 10)?;
    state.scroll_by(6, true, 7);
    let previous = render(&state, &Config::default(), 60, 10)?;
    // When
    state.action(Action::PageDown, 7, Instant::now());
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&previous, 6).contains("row 9"));
    assert!(line(&buffer, 2).contains("row 10"));
    assert!(line(&buffer, 6).contains("row 14"));
    Ok(())
}

#[test]
fn paging_up_from_a_sticky_header_keeps_adjacent_pages_contiguous() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 60, 10)?;
    state.scroll_by(12, true, 7);
    let previous = render(&state, &Config::default(), 60, 10)?;
    // When
    state.action(Action::PageUp, 7, Instant::now());
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&previous, 2).contains("row 11"));
    assert!(line(&buffer, 2).contains("row 6"));
    assert!(line(&buffer, 6).contains("row 10"));
    Ok(())
}

#[test]
fn the_sticky_border_cannot_cover_the_next_file_header() -> AppResult<()> {
    // Given
    let mut state = two_files(Category::Unstaged);
    prepare(&mut state, 60, 10)?;
    state.scroll_by(6, true, 7);
    // When
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("first.rs"));
    assert!(line(&buffer, 1).contains("second.rs"));
    assert!(line(&buffer, 2).starts_with(&"─".repeat(59)));
    assert_eq!(markers(&buffer, 0), vec![0]);
    Ok(())
}

#[test]
fn a_header_entering_the_bottom_of_the_view_cannot_cover_the_status_separator() -> AppResult<()> {
    // Given
    let mut state = two_files(Category::Staged);
    prepare(&mut state, 60, 6)?;
    state.scroll_by(7, true, 3);
    // When
    let buffer = render(&state, &Config::default(), 60, 6)?;
    // Then
    assert!(line(&buffer, 1).contains("STAGED"));
    assert!(line(&buffer, 2).contains("second.rs"));
    assert_eq!(markers(&buffer, 0), vec![2]);
    assert_eq!(line(&buffer, 3), "─".repeat(60));
    assert_eq!(line(&buffer, 4).trim(), "Watching");
    Ok(())
}

#[test]
fn jumping_to_the_end_in_a_tiny_view_keeps_source_below_the_border() -> AppResult<()> {
    // Given
    let mut state = without_sidebar(state_with("main.rs", 40));
    prepare(&mut state, 60, 6)?;
    // When
    state.action(Action::Last, 3, Instant::now());
    let buffer = render(&state, &Config::default(), 60, 6)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 1).starts_with(&"─".repeat(59)));
    assert!(line(&buffer, 2).contains("row 40"));
    Ok(())
}

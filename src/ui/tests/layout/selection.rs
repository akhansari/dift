use super::*;
use ratatui::{
    crossterm::event::{KeyCode, KeyModifiers, MouseButton},
    layout::Position,
};

struct Viewer {
    state: State,
    interaction: Interaction,
    config: Config,
    terminal: Terminal<TestBackend>,
}

fn change(path: &str, category: Category, lines: &[(LineKind, &str)]) -> Change {
    Change {
        path: path.into(),
        old_path: None,
        category,
        status: 'M',
        hunks: vec![Hunk {
            header: "@@ code @@".into(),
            lines: lines
                .iter()
                .enumerate()
                .map(|(index, (kind, text))| SourceLine {
                    kind: *kind,
                    old: (*kind != LineKind::Added).then_some(index + 1),
                    new: (*kind != LineKind::Removed).then_some(index + 1),
                    text: (*text).into(),
                    no_newline: false,
                })
                .collect(),
        }],
        summary: None,
        oversized: false,
        old_source: None,
        new_source: None,
    }
}

impl Viewer {
    fn new(changes: Vec<Change>, split: bool, width: u16, height: u16) -> AppResult<Self> {
        let mut state = State::new(split);
        state.update_snapshot(Snapshot {
            changes,
            ..Snapshot::default()
        });
        prepare(&mut state, width, height)?;
        let mut viewer = Self {
            state,
            interaction: Interaction::default(),
            config: Config::default(),
            terminal: Terminal::new(TestBackend::new(width, height))?,
        };
        viewer.draw()?;
        Ok(viewer)
    }

    fn code(lines: &[&str]) -> AppResult<Self> {
        Self::new(
            vec![change(
                "main.rs",
                Category::Unstaged,
                &lines
                    .iter()
                    .map(|line| (LineKind::Added, *line))
                    .collect::<Vec<_>>(),
            )],
            false,
            100,
            20,
        )
    }

    fn draw(&mut self) -> AppResult<()> {
        self.terminal.draw(|frame| {
            draw(
                frame,
                &self.state,
                &self.config,
                Activity::current(&self.state, true, false),
                &mut self.interaction,
                self.state.help_scroll(),
            );
        })?;
        Ok(())
    }

    fn at(&self, text: &str, offset: u16) -> AppResult<Position> {
        let buffer = self.terminal.backend().buffer();
        (0..buffer.area.height)
            .find_map(|y| {
                let mut text_row = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    text_row.push_str(symbol);
                    x += symbol.width().max(1) as u16;
                }
                text_row
                    .find(text)
                    .map(|index| Position::new(text_row[..index].width() as u16 + offset, y))
            })
            .ok_or_else(|| format!("Missing visible text {text:?}").into())
    }

    fn mouse(&mut self, kind: MouseEventKind, point: Position) -> AppResult<Effects> {
        let view = ViewAreas::new(self.terminal.backend().buffer().area, self.state.sidebar());
        let effects = view.handle_mouse(
            &mut self.state,
            &self.config,
            &mut self.interaction,
            MouseEvent {
                kind,
                column: point.x,
                row: point.y,
                modifiers: KeyModifiers::NONE,
            },
        );
        self.draw()?;
        Ok(effects)
    }

    fn select(&mut self, start: Position, end: Position) -> AppResult<()> {
        self.mouse(MouseEventKind::Down(MouseButton::Left), start)?;
        self.mouse(MouseEventKind::Drag(MouseButton::Left), end)?;
        self.mouse(MouseEventKind::Up(MouseButton::Left), end)?;
        Ok(())
    }

    fn key(&mut self, key: KeyCode) -> AppResult<()> {
        self.interaction.handle_key(
            &mut self.state,
            KeyEvent::new(key, KeyModifiers::NONE),
            &self.config.bindings,
            17,
            Instant::now(),
        );
        self.draw()
    }

    fn highlighted(&self) -> Vec<String> {
        let buffer = self.terminal.backend().buffer();
        (0..buffer.area.height)
            .filter_map(|y| {
                let mut text = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let cell = &buffer[(x, y)];
                    if cell.modifier.contains(Modifier::REVERSED) {
                        text.push_str(cell.symbol());
                    }
                    x += cell.symbol().width().max(1) as u16;
                }
                (!text.is_empty()).then_some(text)
            })
            .collect()
    }
}

#[test]
fn single_line_drag_highlights_only_selected_characters_and_focuses_diff() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha beta"])?;
    let start = viewer.at("alpha", 2)?;
    let end = viewer.at("alpha", 4)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["pha"]);
    assert_eq!(viewer.state.focus(), Scope::Diff);
    Ok(())
}

#[test]
fn multiline_selection_excludes_gutters_tree_and_padding() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha", "    beta", "gamma"])?;
    let start = viewer.at("alpha", 2)?;
    let end = viewer.at("gamma", 2)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["pha", "    beta", "gam"]);
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("pha\n    beta\ngam")
    );
    Ok(())
}

#[test]
fn reversed_selection_matches_forward_selection() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha", "    beta", "gamma"])?;
    let start = viewer.at("gamma", 2)?;
    let end = viewer.at("alpha", 2)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["pha", "    beta", "gam"]);
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("pha\n    beta\ngam")
    );
    Ok(())
}

#[test]
fn unified_selection_includes_both_removed_and_added_code_without_signs() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![change(
            "main.rs",
            Category::Unstaged,
            &[
                (LineKind::Removed, "old_value"),
                (LineKind::Added, "new_value"),
            ],
        )],
        false,
        100,
        20,
    )?;
    let start = viewer.at("old_value", 0)?;
    let end = viewer.at("new_value", 8)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["old_value", "new_value"]);
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("old_value\nnew_value")
    );
    Ok(())
}

#[test]
fn split_selection_stays_on_the_starting_side_when_drag_crosses_divider() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![change(
            "main.rs",
            Category::Unstaged,
            &[
                (LineKind::Removed, "old_value"),
                (LineKind::Added, "new_value"),
            ],
        )],
        true,
        120,
        20,
    )?;
    let start = viewer.at("old_value", 0)?;
    let end = viewer.at("new_value", 8)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["old_value"]);
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("old_value")
    );
    Ok(())
}

#[test]
fn new_side_selection_does_not_include_old_side() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![change(
            "main.rs",
            Category::Unstaged,
            &[
                (LineKind::Removed, "old_value"),
                (LineKind::Added, "new_value"),
            ],
        )],
        true,
        120,
        20,
    )?;
    let start = viewer.at("new_value", 8)?;
    let end = viewer.at("old_value", 0)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["new_value"]);
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("new_value")
    );
    Ok(())
}

#[test]
fn wrapped_code_selection_does_not_include_continuation_gutters() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![change(
            "main.rs",
            Category::Unstaged,
            &[(LineKind::Added, "abcdefghijklmnopqrstuvwxyz0123456789")],
        )],
        false,
        40,
        20,
    )?;
    let start = viewer.at("abcdefgh", 0)?;
    let end = viewer.at("456789", 5)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(
        viewer.highlighted().concat(),
        "abcdefghijklmnopqrstuvwxyz0123456789"
    );
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("abcdefghijklmnopqrstuvwxyz0123456789")
    );
    Ok(())
}

#[test]
fn tabs_and_combining_characters_stay_with_their_code() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["\te\u{301}界end"])?;
    let start = viewer.at("    e\u{301}界end", 0)?;
    let end = viewer.at("end", 2)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["    e\u{301}界end"]);
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("\té界end")
    );
    Ok(())
}

#[test]
fn selecting_the_second_cell_of_a_wide_character_highlights_the_whole_character() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["ab界cd"])?;
    let start = viewer.at("ab", 1)?;
    let end = viewer.at("ab", 3)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["b界"]);
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("b界")
    );
    Ok(())
}

#[test]
fn dragging_from_a_gutter_does_not_start_selection() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let code = viewer.at("alpha", 0)?;
    // When
    viewer.select(
        Position::new(code.x - 2, code.y),
        Position::new(code.x + 4, code.y),
    )?;
    // Then
    assert!(viewer.highlighted().is_empty());
    assert_eq!(viewer.state.focus(), Scope::Tree);
    Ok(())
}

#[test]
fn a_click_without_dragging_leaves_no_highlight() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let point = viewer.at("alpha", 2)?;
    // When
    viewer.select(point, point)?;
    // Then
    assert!(viewer.highlighted().is_empty());
    Ok(())
}

#[test]
fn selection_stops_at_the_starting_file() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![
            change(
                "a.rs",
                Category::Unstaged,
                &[(LineKind::Added, "first_file")],
            ),
            change(
                "b.rs",
                Category::Unstaged,
                &[(LineKind::Added, "second_file")],
            ),
        ],
        false,
        100,
        20,
    )?;
    let start = viewer.at("first_file", 0)?;
    let end = viewer.at("second_file", 10)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["first_file"]);
    Ok(())
}

#[test]
fn selection_stops_at_the_starting_diff_category() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![
            change(
                "a.rs",
                Category::Unstaged,
                &[(LineKind::Added, "working_code")],
            ),
            change(
                "a.rs",
                Category::Staged,
                &[(LineKind::Added, "indexed_code")],
            ),
        ],
        false,
        100,
        20,
    )?;
    let start = viewer.at("working_code", 0)?;
    let end = viewer.at("indexed_code", 11)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["working_code"]);
    Ok(())
}

#[test]
fn escape_clears_selection_before_clearing_search() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    viewer.state.action(Action::Search, 17, Instant::now());
    viewer.state.paste("alpha", Instant::now());
    viewer.state.action(Action::Accept, 17, Instant::now());
    prepare(&mut viewer.state, 100, 20)?;
    viewer.draw()?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    viewer.key(KeyCode::Esc)?;
    // Then
    assert!(viewer.highlighted().is_empty());
    assert_eq!(viewer.state.search(), "alpha");
    Ok(())
}

#[test]
fn replacing_the_displayed_document_clears_selection() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    prepare(&mut viewer.state, 100, 20)?;
    viewer.draw()?;
    // Then
    assert!(viewer.highlighted().is_empty());
    Ok(())
}

#[test]
fn resizing_clears_selection() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    viewer.terminal.backend_mut().resize(90, 20);
    viewer.draw()?;
    // Then
    assert!(viewer.highlighted().is_empty());
    Ok(())
}

#[test]
fn a_new_tree_click_clears_selection_and_still_selects_the_file() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    viewer.mouse(
        MouseEventKind::Down(MouseButton::Left),
        Position::new(10, 1),
    )?;
    // Then
    assert!(viewer.highlighted().is_empty());
    assert_eq!(viewer.state.selection(), std::path::Path::new("main.rs"));
    assert_eq!(viewer.state.focus(), Scope::Tree);
    Ok(())
}

#[test]
fn opening_help_clears_selection() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    viewer.key(KeyCode::Char('?'))?;
    viewer.key(KeyCode::Esc)?;
    // Then
    assert!(viewer.highlighted().is_empty());
    Ok(())
}

#[test]
fn opening_filter_clears_selection() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    viewer.key(KeyCode::Char('f'))?;
    // Then
    assert!(viewer.highlighted().is_empty());
    assert!(viewer.state.prompt().is_some());
    Ok(())
}

#[test]
fn wheel_during_drag_extends_selection_to_newly_visible_code() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![change(
            "main.rs",
            Category::Unstaged,
            &[
                (LineKind::Added, "alpha"),
                (LineKind::Added, "bravo"),
                (LineKind::Added, "charlie"),
                (LineKind::Added, "delta"),
                (LineKind::Added, "echo"),
                (LineKind::Added, "foxtrot"),
            ],
        )],
        false,
        100,
        10,
    )?;
    let start = viewer.at("alpha", 0)?;
    viewer.mouse(MouseEventKind::Down(MouseButton::Left), start)?;
    // When
    viewer.mouse(MouseEventKind::ScrollDown, start)?;
    // Then
    assert_eq!(viewer.state.scroll(), 3);
    assert_eq!(viewer.highlighted(), ["bravo", "charlie", "d"]);
    assert!(viewer.interaction.selection.take_copy().is_none());
    Ok(())
}

#[test]
fn sticky_headers_cannot_start_selection_of_the_hidden_code() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::new(
        vec![change(
            "main.rs",
            Category::Unstaged,
            &[
                (LineKind::Added, "alpha"),
                (LineKind::Added, "bravo"),
                (LineKind::Added, "charlie"),
                (LineKind::Added, "delta"),
                (LineKind::Added, "echo"),
                (LineKind::Added, "foxtrot"),
            ],
        )],
        false,
        100,
        8,
    )?;
    viewer.state.scroll_by(4, true, 5);
    viewer.draw()?;
    // When
    viewer.select(Position::new(50, 0), Position::new(55, 3))?;
    // Then
    assert!(viewer.highlighted().is_empty());
    Ok(())
}

#[test]
fn dragging_outside_the_pane_clamps_without_scrolling_or_selecting_tree() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha", "bravo", "charlie"])?;
    let start = viewer.at("alpha", 0)?;
    // When
    viewer.select(start, Position::new(0, 40))?;
    // Then
    assert_eq!(viewer.highlighted(), ["alpha", "bravo", "c"]);
    assert_eq!(viewer.state.scroll(), 0);
    assert_eq!(viewer.state.selection(), std::path::Path::new(""));
    Ok(())
}

#[test]
fn selection_skips_hunk_headers_within_the_same_file() -> AppResult<()> {
    // Given
    let mut file = change("main.rs", Category::Unstaged, &[(LineKind::Added, "alpha")]);
    file.hunks
        .extend(change("main.rs", Category::Unstaged, &[(LineKind::Added, "bravo")]).hunks);
    let mut viewer = Viewer::new(vec![file], false, 100, 20)?;
    let start = viewer.at("alpha", 0)?;
    let end = viewer.at("bravo", 4)?;
    // When
    viewer.select(start, end)?;
    // Then
    assert_eq!(viewer.highlighted(), ["alpha", "bravo"]);
    Ok(())
}

#[test]
fn opening_search_clears_selection() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    viewer.key(KeyCode::Char('/'))?;
    // Then
    assert!(viewer.highlighted().is_empty());
    assert!(viewer.state.prompt().is_some());
    Ok(())
}

#[test]
fn a_plain_click_does_not_consume_escape_for_search() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    viewer.state.action(Action::Search, 17, Instant::now());
    viewer.state.paste("alpha", Instant::now());
    viewer.state.action(Action::Accept, 17, Instant::now());
    prepare(&mut viewer.state, 100, 20)?;
    viewer.draw()?;
    let point = viewer.at("alpha", 2)?;
    viewer.select(point, point)?;
    // When
    viewer.key(KeyCode::Esc)?;
    // Then
    assert!(viewer.state.search().is_empty());
    Ok(())
}

#[test]
fn released_selection_survives_redraws_and_focus_changes() -> AppResult<()> {
    // Given
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    viewer.select(start, Position::new(start.x + 4, start.y))?;
    // When
    viewer.key(KeyCode::Tab)?;
    viewer.draw()?;
    // Then
    assert_eq!(viewer.highlighted(), ["alpha"]);
    assert_eq!(viewer.state.focus(), Scope::Tree);
    Ok(())
}

#[test]
fn copying_preserves_blank_source_lines() -> AppResult<()> {
    let mut viewer = Viewer::code(&["alpha", "", "beta"])?;
    let start = viewer.at("alpha", 0)?;
    let end = viewer.at("beta", 3)?;
    viewer.select(start, end)?;
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("alpha\n\nbeta")
    );
    Ok(())
}

#[test]
fn dragging_does_not_copy_until_release() -> AppResult<()> {
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    let end = viewer.at("alpha", 4)?;
    viewer.mouse(MouseEventKind::Down(MouseButton::Left), start)?;
    viewer.mouse(MouseEventKind::Drag(MouseButton::Left), end)?;
    assert!(viewer.interaction.selection.take_copy().is_none());
    Ok(())
}

#[test]
fn repeated_release_does_not_copy_again() -> AppResult<()> {
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    let end = viewer.at("alpha", 4)?;
    viewer.select(start, end)?;
    let copied = viewer.interaction.selection.take_copy();
    viewer.mouse(MouseEventKind::Up(MouseButton::Left), end)?;
    assert_eq!(copied.as_deref(), Some("alpha"));
    assert!(viewer.interaction.selection.take_copy().is_none());
    Ok(())
}

#[test]
fn clicking_without_dragging_does_not_copy() -> AppResult<()> {
    let mut viewer = Viewer::code(&["alpha"])?;
    let point = viewer.at("alpha", 0)?;
    viewer.select(point, point)?;
    assert!(viewer.interaction.selection.take_copy().is_none());
    Ok(())
}

#[test]
fn escape_during_drag_prevents_copy_on_release() -> AppResult<()> {
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    let end = viewer.at("alpha", 4)?;
    viewer.mouse(MouseEventKind::Down(MouseButton::Left), start)?;
    viewer.mouse(MouseEventKind::Drag(MouseButton::Left), end)?;
    viewer.key(KeyCode::Esc)?;
    viewer.mouse(MouseEventKind::Up(MouseButton::Left), end)?;
    assert!(viewer.interaction.selection.take_copy().is_none());
    Ok(())
}

#[test]
fn document_replacement_during_drag_prevents_copy_on_release() -> AppResult<()> {
    let mut viewer = Viewer::code(&["alpha"])?;
    let start = viewer.at("alpha", 0)?;
    let end = viewer.at("alpha", 4)?;
    viewer.mouse(MouseEventKind::Down(MouseButton::Left), start)?;
    viewer.mouse(MouseEventKind::Drag(MouseButton::Left), end)?;
    prepare(&mut viewer.state, 100, 20)?;
    viewer.mouse(MouseEventKind::Up(MouseButton::Left), end)?;
    assert!(viewer.interaction.selection.take_copy().is_none());
    Ok(())
}

#[test]
fn tab_wrapping_across_rows_is_copied_once() -> AppResult<()> {
    let mut viewer = Viewer::new(
        vec![change(
            "main.rs",
            Category::Unstaged,
            &[(LineKind::Added, "abcdefghi\tEND")],
        )],
        false,
        40,
        20,
    )?;
    let start = viewer.at("abcdefghi", 0)?;
    let end = viewer.at("END", 2)?;
    viewer.select(start, end)?;
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("abcdefghi\tEND")
    );
    Ok(())
}

#[test]
fn partially_selected_tab_copies_the_original_tab() -> AppResult<()> {
    let mut viewer = Viewer::code(&["\tend"])?;
    let start = viewer.at("    end", 1)?;
    let end = viewer.at("    end", 2)?;
    viewer.select(start, end)?;
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("\t")
    );
    Ok(())
}

#[test]
fn escaped_control_character_copies_its_source_byte() -> AppResult<()> {
    let mut viewer = Viewer::code(&["a\rb"])?;
    let start = viewer.at("a\\rb", 1)?;
    let end = viewer.at("a\\rb", 2)?;
    viewer.select(start, end)?;
    assert_eq!(
        viewer.interaction.selection.take_copy().as_deref(),
        Some("\r")
    );
    Ok(())
}

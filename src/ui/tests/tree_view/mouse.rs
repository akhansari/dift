use super::*;
use crate::render::{Document, Row, worker::RenderResult};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::{path::Path, sync::Arc};

struct MouseTree {
    state: State,
    config: Config,
    viewport: tree::Viewport,
    terminal: Terminal<TestBackend>,
}

impl MouseTree {
    fn new(state: State, width: u16, height: u16) -> AppResult<Self> {
        let mut tree = Self {
            state,
            config: Config::default(),
            viewport: tree::Viewport::default(),
            terminal: Terminal::new(TestBackend::new(width, height))?,
        };
        tree.draw()?;
        Ok(tree)
    }

    fn long() -> AppResult<Self> {
        Self::new(
            state_with(&[
                ("src/a.rs", Category::Unstaged),
                ("src/b.rs", Category::Unstaged),
                ("src/c.rs", Category::Unstaged),
                ("src/d.rs", Category::Unstaged),
                ("src/e.rs", Category::Unstaged),
                ("src/f.rs", Category::Unstaged),
                ("src/g.rs", Category::Unstaged),
                ("src/h.rs", Category::Unstaged),
                ("README.md", Category::Untracked),
            ]),
            34,
            4,
        )
    }

    fn draw(&mut self) -> AppResult<()> {
        self.terminal.draw(|frame| {
            tree::draw(
                frame,
                &self.state,
                &self.config,
                frame.area(),
                &mut self.viewport,
            );
        })?;
        Ok(())
    }

    fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) -> AppResult<Effects> {
        let effects = self.viewport.handle_mouse(
            &mut self.state,
            &self.config,
            self.terminal.backend().buffer().area,
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
        );
        self.draw()?;
        Ok(effects)
    }

    fn click(&mut self, column: u16, row: u16) -> AppResult<Effects> {
        self.mouse(MouseEventKind::Down(MouseButton::Left), column, row)
    }

    fn key(&mut self, code: KeyCode) -> AppResult<()> {
        self.viewport.handle_key(
            &mut self.state,
            KeyEvent::new(code, KeyModifiers::NONE),
            &self.config.bindings,
            4,
            Instant::now(),
        );
        self.draw()
    }

    fn line(&self, row: u16) -> String {
        line(self.terminal.backend().buffer(), row)
    }

    fn scrolled_diff(&mut self) {
        let request = self.state.render_request(80, 4);
        self.state.apply_render(
            RenderResult {
                generation: request.generation,
                document: Arc::new(Document {
                    rows: vec![
                        Arc::new(Row {
                            line: Line::raw("diff content"),
                            anchors: Vec::new(),
                            code: Vec::new(),
                        });
                        20
                    ],
                    ..Document::default()
                }),
                finished: true,
            },
            4,
        );
        self.state.action(Action::PageDown, 4, Instant::now());
    }
}

#[test]
fn clicking_a_scrolled_row_selects_the_visible_file_and_focuses_the_tree() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.state.action(Action::Focus, 4, Instant::now());
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // When
    let effects = tree.click(10, 1)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new("src/c.rs"));
    assert_eq!(tree.state.focus(), Scope::Tree);
    assert!(effects.render);
    assert!(tree.line(0).contains("b.rs"));
    Ok(())
}

#[test]
fn wheel_scrolling_leaves_the_selection_diff_and_focus_unchanged_across_redraws() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.scrolled_diff();
    let scroll = tree.state.scroll();
    tree.state.action(Action::Focus, 4, Instant::now());
    // When
    let effects = tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    tree.draw()?;
    // Then
    assert_eq!(tree.state.selection(), Path::new(""));
    assert_eq!(tree.state.focus(), Scope::Diff);
    assert_eq!(tree.state.scroll(), scroll);
    assert!(!effects.render);
    assert!(tree.line(0).contains("b.rs"));
    assert!(!tree.line(0).contains('▎'));
    Ok(())
}

#[test]
fn keyboard_navigation_reveals_the_selection_after_wheel_scrolling() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // When
    tree.key(KeyCode::Down)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new("src"));
    assert!(tree.line(0).contains("All changes"));
    assert!(tree.line(1).contains('▎'));
    Ok(())
}

#[test]
fn keyboard_navigation_at_a_boundary_still_reveals_selection() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // When
    tree.key(KeyCode::Up)?;
    // Then
    assert!(tree.line(0).contains("All changes"));
    assert!(tree.line(0).contains('▎'));
    Ok(())
}

#[test]
fn clicking_the_selected_row_preserves_diff_scroll() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.scrolled_diff();
    let scroll = tree.state.scroll();
    // When
    let effects = tree.click(10, 0)?;
    // Then
    assert!(scroll > 0);
    assert_eq!(tree.state.scroll(), scroll);
    assert!(!effects.render);
    Ok(())
}

#[test]
fn clicking_a_folder_name_selects_without_collapsing() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    // When
    let effects = tree.click(6, 1)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new("src"));
    assert!(!tree.state.is_collapsed(Path::new("src")));
    assert!(effects.render);
    Ok(())
}

#[test]
fn clicking_a_folder_arrow_collapses_without_selecting() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    // When
    let effects = tree.click(3, 1)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new(""));
    assert!(tree.state.is_collapsed(Path::new("src")));
    assert!(tree.line(2).contains("README.md"));
    assert!(!effects.render);
    Ok(())
}

#[test]
fn clicking_a_collapsed_folder_arrow_expands_without_selecting() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.click(3, 1)?;
    // When
    let effects = tree.click(3, 1)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new(""));
    assert!(!tree.state.is_collapsed(Path::new("src")));
    assert!(tree.line(2).contains("a.rs"));
    assert!(!effects.render);
    Ok(())
}

#[test]
fn collapsing_a_selected_descendants_folder_selects_the_folder() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.click(10, 2)?;
    tree.scrolled_diff();
    // When
    let effects = tree.click(3, 1)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new("src"));
    assert!(tree.state.is_collapsed(Path::new("src")));
    assert_eq!(tree.state.scroll(), 0);
    assert!(effects.render);
    Ok(())
}

#[test]
fn the_root_arrow_can_collapse_all_changes() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.click(10, 2)?;
    // When
    tree.click(1, 0)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new(""));
    assert!(tree.state.is_collapsed(Path::new("")));
    assert_eq!(tree.state.tree(false).len(), 1);
    Ok(())
}

#[test]
fn nerd_folder_icons_are_toggle_targets() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.config.icons = Icons::Nerd;
    tree.draw()?;
    // When
    tree.click(3, 1)?;
    // Then
    assert!(tree.state.is_collapsed(Path::new("src")));
    assert!(tree.line(1).contains('\u{f07b}'));
    assert_eq!(tree.state.selection(), Path::new(""));
    Ok(())
}

#[test]
fn nested_folder_arrows_follow_compressed_indentation() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::new(
        state_with(&[("one/two/three/file.rs", Category::Unstaged)]),
        16,
        6,
    )?;
    // When
    tree.click(5, 3)?;
    // Then
    assert!(tree.state.is_collapsed(Path::new("one/two/three")));
    assert!(tree.line(3).contains('▸'));
    assert_eq!(tree.state.selection(), Path::new(""));
    Ok(())
}

#[test]
fn filtered_row_clicks_select_the_matching_path() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    filter(&mut tree.state, "h.rs");
    tree.draw()?;
    // When
    tree.click(10, 2)?;
    // Then
    assert_eq!(tree.state.selection(), Path::new("src/h.rs"));
    assert_eq!(tree.state.filter(), "h.rs");
    Ok(())
}

#[test]
fn wheel_scrolling_stops_at_the_last_full_viewport() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // When
    let effects = tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // Then
    assert!(tree.line(0).contains("f.rs"));
    assert!(tree.line(3).contains("README.md"));
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn wheel_scrolling_stops_at_the_first_row() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    // When
    let effects = tree.mouse(MouseEventKind::ScrollUp, 10, 1)?;
    // Then
    assert!(tree.line(0).contains("All changes"));
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn growing_the_viewport_clamps_scroll_without_selecting() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    tree.terminal.backend_mut().resize(34, 10);
    // When
    tree.draw()?;
    // Then
    assert!(tree.line(0).contains("src"));
    assert!(tree.line(9).contains("README.md"));
    assert_eq!(tree.state.selection(), Path::new(""));
    Ok(())
}

#[test]
fn repository_updates_reveal_a_reconciled_selection() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.click(10, 2)?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // When
    tree.state.update_snapshot(Snapshot {
        changes: sample().snapshot().changes.clone(),
        ..Snapshot::default()
    });
    tree.draw()?;
    // Then
    assert_eq!(tree.state.selection(), Path::new("src"));
    assert!(tree.line(1).contains("src"));
    assert!(tree.line(1).contains('▎'));
    Ok(())
}

#[test]
fn filtering_to_no_matches_clamps_the_viewport() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // When
    filter(&mut tree.state, "absent-name");
    tree.draw()?;
    let effects = tree.click(10, 1)?;
    // Then
    assert!(tree.line(0).contains("All changes"));
    assert!(tree.line(1).contains("No matching files"));
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn empty_space_below_rows_is_not_selectable() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::new(sample(), 34, 8)?;
    // When
    let effects = tree.click(10, 7)?;
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(tree.state.selection(), Path::new(""));
    Ok(())
}

#[test]
fn the_scrollbar_column_does_not_select_rows() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    // When
    let effects = tree.click(33, 2)?;
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(tree.state.selection(), Path::new(""));
    Ok(())
}

#[test]
fn clicks_outside_the_tree_are_ignored() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    // When
    let effects = tree.click(40, 2)?;
    // Then
    assert_eq!(effects, Effects::default());
    Ok(())
}

#[test]
fn help_blocks_mouse_selection() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.state.action(Action::Help, 4, Instant::now());
    // When
    let effects = tree.click(10, 2)?;
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(tree.state.selection(), Path::new(""));
    Ok(())
}

#[test]
fn filter_prompts_block_mouse_scrolling() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.state.action(Action::Filter, 4, Instant::now());
    // When
    let effects = tree.mouse(MouseEventKind::ScrollDown, 10, 1)?;
    // Then
    assert_eq!(effects, Effects::default());
    assert!(tree.line(0).contains("All changes"));
    Ok(())
}

#[test]
fn search_prompts_block_mouse_selection() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.state.action(Action::Search, 4, Instant::now());
    // When
    let effects = tree.click(10, 2)?;
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(tree.state.selection(), Path::new(""));
    Ok(())
}

#[test]
fn a_hidden_sidebar_ignores_mouse_input() -> AppResult<()> {
    // Given
    let mut tree = MouseTree::long()?;
    tree.state.action(Action::Sidebar, 4, Instant::now());
    // When
    let effects = tree.click(10, 2)?;
    // Then
    assert_eq!(effects, Effects::default());
    assert_eq!(tree.state.focus(), Scope::Diff);
    Ok(())
}

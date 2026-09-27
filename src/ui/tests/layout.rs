use super::*;
use crate::{
    git::GitError,
    model::{Category, Change, Hunk, LineKind, Snapshot, SourceLine},
    render::{Renderer, worker::RenderResult},
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
mod commit;
mod discard;
mod editor;
mod filters;
mod headers;
mod mouse;
mod search;
mod selection;
mod stats;

fn state_with(path: &str, lines: usize) -> State {
    let mut state = State::new(false);
    state.update_snapshot(Snapshot {
        branch: "main".into(),
        changes: vec![Change {
            path: path.into(),
            old_path: None,
            category: Category::Unstaged,
            status: 'M',
            hunks: vec![Hunk {
                header: format!("@@ -0,0 +1,{lines} @@"),
                lines: (1..=lines)
                    .map(|number| SourceLine {
                        kind: LineKind::Added,
                        old: None,
                        new: Some(number),
                        text: format!("row {number}"),
                        no_newline: false,
                    })
                    .collect(),
            }],
            summary: None,
            oversized: false,
            old_source: None,
            new_source: None,
        }],
        ..Snapshot::default()
    });
    state
}

fn state_with_files(paths: &[&str], lines: usize) -> State {
    let mut state = State::new(false);
    state.update_snapshot(Snapshot {
        changes: paths
            .iter()
            .flat_map(|path| state_with(path, lines).snapshot().changes.clone())
            .collect(),
        ..Snapshot::default()
    });
    state
}

fn state_with_whitespace_toast() -> State {
    let mut state = state_with("main.rs", 1);
    state.action(Action::Whitespace, 8, Instant::now());
    let request = state.collection_request();
    state.collection_finished(
        request.generation,
        Ok(Snapshot {
            whitespace: crate::config::Whitespace::IgnoreChanges,
            whitespace_hidden_files: 1,
            ..Snapshot::default()
        }),
        Instant::now(),
    );
    state
}

fn prepare(state: &mut State, width: u16, height: u16) -> AppResult<()> {
    let view = ViewAreas::new(Rect::new(0, 0, width, height), state.sidebar());
    let (width, height) = view.viewport();
    let request = state.render_request(width, height);
    let mut document = Default::default();
    Renderer::new(Config::default())?.render_progressive(
        crate::render::RenderSpec {
            snapshot: &request.snapshot,
            selected: &request.selected,
            width: request.width,
            split: request.split,
            presentation: request.presentation,
            show_line_numbers: request.show_line_numbers,
            query: &request.query,
            preview_rows: 0,
        },
        &crate::render::Cancellation::default(),
        |update| {
            document = update.document;
            true
        },
    )?;
    state.apply_render(
        RenderResult {
            generation: request.generation,
            document,
            finished: true,
        },
        height,
    );
    Ok(())
}

fn state_with_queries(filter: &str, search: &str) -> AppResult<State> {
    let now = Instant::now();
    let mut state = state_with("main.rs", 1);
    state.action(Action::Filter, 9, now);
    state.paste(filter, now);
    state.action(Action::Accept, 9, now);
    state.action(Action::Search, 9, now);
    state.paste(search, now);
    state.action(Action::Accept, 9, now);
    prepare(&mut state, 120, 12)?;
    Ok(state)
}

fn render(state: &State, config: &Config, width: u16, height: u16) -> AppResult<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    draw_into(&mut terminal, state, config)?;
    Ok(terminal.backend().buffer().clone())
}

fn draw_into(
    terminal: &mut Terminal<TestBackend>,
    state: &State,
    config: &Config,
) -> AppResult<()> {
    draw_with_activity(
        terminal,
        state,
        config,
        Activity::current(state, true, false),
    )
}

fn draw_with_activity(
    terminal: &mut Terminal<TestBackend>,
    state: &State,
    config: &Config,
    activity: Activity,
) -> AppResult<()> {
    terminal.draw(|frame| {
        draw(
            frame,
            state,
            config,
            activity,
            &mut Interaction::default(),
            state.help_scroll(),
        )
    })?;
    Ok(())
}

fn line(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn historical_title_and_footer_replace_worktree_status_and_shortcuts() -> AppResult<()> {
    // Given: a selected historical change with its own navigation hints.
    let mut state = state_with("main.rs", 1);
    prepare(&mut state, 120, 20)?;
    let mut terminal = Terminal::new(TestBackend::new(120, 20))?;
    // When: drawing the shared viewer with a commit title.
    terminal.draw(|frame| {
        draw_in(
            frame,
            &state,
            &Config::default(),
            StatusLabel::Title {
                text: "abc123 · CHANGES · initial",
                footer: " esc history · v timecapsule · q quit",
            },
            &mut Interaction::default(),
            0,
            frame.area(),
        );
    })?;
    // Then: the historical status and footer occupy their own rows without worktree shortcuts.
    let buffer = terminal.backend().buffer();
    assert!(line(buffer, 18).contains("abc123 · CHANGES · initial"));
    let footer = line(buffer, 19);
    assert!(footer.contains("esc history · v timecapsule · q quit"));
    assert!(!footer.contains("/ search"));
    assert!(!footer.contains("sidebar"));
    Ok(())
}

#[test]
fn main_panes_have_only_a_shared_divider_joined_to_the_status_rule() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    prepare(&mut state, 120, 12)?;
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert!(line(&buffer, 0).contains("All changes"));
    assert!(line(&buffer, 0).contains("UNSTAGED"));
    assert_eq!(buffer[(0, 0)].symbol(), "▎");
    assert_eq!(buffer[(33, 0)].symbol(), "│");
    assert_eq!(buffer[(33, 8)].symbol(), "│");
    assert_eq!(buffer[(119, 0)].symbol(), " ");
    assert_eq!(buffer[(0, 9)].symbol(), "─");
    assert_eq!(buffer[(33, 9)].symbol(), "┴");
    assert_eq!(buffer[(119, 9)].symbol(), "─");
    assert_eq!(line(&buffer, 10).trim(), "Watching");
    assert!(!line(&buffer, 10).contains("Unified"));
    assert!(!line(&buffer, 10).contains("whitespace"));
    assert!(line(&buffer, 11).contains("help"));
    Ok(())
}

#[test]
fn hiding_the_sidebar_removes_the_divider_and_its_junction() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Sidebar, 8, Instant::now());
    prepare(&mut state, 120, 12)?;
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert!(line(&buffer, 0).starts_with(" UNSTAGED "));
    assert_eq!(line(&buffer, 9), "─".repeat(120));
    assert!(!line(&buffer, 0).contains('│'));
    assert!(!line(&buffer, 0).contains("All changes"));
    Ok(())
}

#[test]
fn opening_the_filter_keeps_the_scrollbar_at_the_top_of_the_tree() -> AppResult<()> {
    // Given
    let mut state = state_with_files(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"], 1);
    let config = Config::default();
    let mut terminal = Terminal::new(TestBackend::new(120, 8))?;
    draw_into(&mut terminal, &state, &config)?;
    // When
    state.action(Action::Filter, 5, Instant::now());
    draw_into(&mut terminal, &state, &config)?;
    // Then
    let buffer = terminal.backend().buffer();
    assert!(line(buffer, 0).contains("All changes"));
    assert!(line(buffer, 7).starts_with("Filter › "));
    assert_eq!(buffer[(33, 0)].symbol(), "┃");
    assert_eq!(buffer[(33, 1)].symbol(), "┃");
    assert_eq!(buffer[(33, 4)].symbol(), "│");
    assert_eq!(buffer[(33, 4)].fg, Palette::new(false).muted);
    assert_eq!(buffer[(33, 0)].bg, Palette::new(false).background);
    assert_eq!(buffer[(33, 4)].bg, Palette::new(false).background);
    assert_eq!(buffer[(33, 5)].symbol(), "─");
    Ok(())
}

#[test]
fn editing_a_filter_keeps_the_divider_when_the_tree_fits() -> AppResult<()> {
    // Given
    let mut state = state_with("a/b/c/main.rs", 1);
    let config = Config {
        light: true,
        ..Config::default()
    };
    let mut terminal = Terminal::new(TestBackend::new(120, 8))?;
    draw_into(&mut terminal, &state, &config)?;
    // When
    state.action(Action::Filter, 5, Instant::now());
    state.paste("main", Instant::now());
    draw_into(&mut terminal, &state, &config)?;
    // Then
    let buffer = terminal.backend().buffer();
    assert!(line(buffer, 0).contains("All changes"));
    assert!(line(buffer, 1).contains("a/b/c"));
    assert!(line(buffer, 2).contains("main.rs"));
    assert_eq!(line(buffer, 6).trim(), "Watching");
    assert!(line(buffer, 7).starts_with("Filter › main"));
    assert_eq!(buffer[(33, 0)].symbol(), "│");
    assert_eq!(buffer[(33, 1)].symbol(), "│");
    assert_eq!(buffer[(33, 4)].symbol(), "│");
    assert_eq!(buffer[(33, 1)].fg, Palette::new(true).muted);
    assert_eq!(buffer[(33, 5)].symbol(), "┴");
    Ok(())
}

#[test]
fn find_uses_its_own_prompt_label() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Find, 5, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 8)?;
    // Then
    assert!(line(&buffer, 7).starts_with("Find › "));
    assert!(line(&buffer, 7).contains("ctrl-n: next match"));
    Ok(())
}

#[test]
fn find_hint_uses_the_configured_navigation_key() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Find, 5, Instant::now());
    let config = Config::parse("keybindings { input { next-find-match \"alt-j\"; }; }")?;
    // When
    let buffer = render(&state, &config, 120, 8)?;
    // Then
    assert!(line(&buffer, 7).contains("alt-j: next match"));
    assert!(!line(&buffer, 7).contains("ctrl-n"));
    Ok(())
}

#[test]
fn filter_hint_does_not_offer_find_navigation() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Filter, 5, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 8)?;
    // Then
    assert!(!line(&buffer, 7).contains("next match"));
    Ok(())
}

#[test]
fn diff_focus_keeps_the_status_activity_and_rules_muted() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::NextFile, 8, Instant::now());
    state.action(Action::Focus, 8, Instant::now());
    let config = Config {
        light: true,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 120, 12)?;
    // Then
    assert!(line(&buffer, 10).starts_with(" Watching"));
    assert!(!line(&buffer, 10).contains("main.rs"));
    assert_eq!(buffer[(1, 10)].fg, Palette::new(true).muted);
    assert_eq!(buffer[(0, 1)].symbol(), " ");
    assert_eq!(buffer[(7, 1)].bg, Palette::new(true).panel);
    assert_eq!(buffer[(33, 1)].fg, Palette::new(true).muted);
    assert_eq!(buffer[(33, 9)].fg, Palette::new(true).muted);
    Ok(())
}

#[test]
fn a_selected_directory_keeps_the_status_activity_muted_without_showing_its_path() -> AppResult<()>
{
    // Given
    let mut state = state_with("src/main.rs", 1);
    state.action(Action::Down, 9, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(buffer[(0, 1)].symbol(), "▎");
    assert!(line(&buffer, 10).starts_with(" Watching"));
    assert!(!line(&buffer, 10).contains("src"));
    assert_eq!(buffer[(1, 10)].fg, Palette::new(false).muted);
    assert_eq!(buffer[(33, 0)].fg, Palette::new(false).muted);
    Ok(())
}

#[test]
fn dark_status_filters_use_accent_color_without_bold() -> AppResult<()> {
    // Given
    let state = state_with_queries("main", "")?;
    let palette = Palette::new(false);
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(line(&buffer, 10).trim(), "Watching · filter: main");
    assert_eq!(buffer[(12, 10)].fg, palette.accent);
    assert_eq!(buffer[(19, 10)].fg, palette.accent);
    assert_eq!(buffer[(12, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(19, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(19, 10)].bg, palette.background);
    assert_eq!(buffer[(10, 10)].fg, palette.muted);
    assert_eq!(buffer[(1, 10)].fg, palette.muted);
    Ok(())
}

#[test]
fn light_status_filters_use_accent_color_without_bold() -> AppResult<()> {
    // Given
    let state = state_with_queries("main", "")?;
    let config = Config {
        light: true,
        ..Config::default()
    };
    let palette = Palette::new(true);
    // When
    let buffer = render(&state, &config, 120, 12)?;
    // Then
    assert_eq!(line(&buffer, 10).trim(), "Watching · filter: main");
    assert_eq!(buffer[(12, 10)].fg, palette.accent);
    assert_eq!(buffer[(19, 10)].fg, palette.accent);
    assert_eq!(buffer[(12, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(19, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(19, 10)].bg, palette.background);
    assert_eq!(buffer[(10, 10)].fg, palette.muted);
    assert_eq!(buffer[(1, 10)].fg, palette.muted);
    Ok(())
}

#[test]
fn dark_status_searches_use_accent_color_without_bold() -> AppResult<()> {
    // Given
    let state = state_with_queries("", "row")?;
    let palette = Palette::new(false);
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(line(&buffer, 10).trim(), "Watching · /row: 1 matches");
    assert_eq!(buffer[(12, 10)].fg, palette.accent);
    assert_eq!(buffer[(13, 10)].fg, palette.accent);
    assert_eq!(buffer[(12, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(13, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(13, 10)].bg, palette.background);
    assert_eq!(buffer[(10, 10)].fg, palette.muted);
    assert_eq!(buffer[(18, 10)].fg, palette.muted);
    Ok(())
}

#[test]
fn light_status_searches_use_accent_color_without_bold() -> AppResult<()> {
    // Given
    let state = state_with_queries("", "row")?;
    let config = Config {
        light: true,
        ..Config::default()
    };
    let palette = Palette::new(true);
    // When
    let buffer = render(&state, &config, 120, 12)?;
    // Then
    assert_eq!(line(&buffer, 10).trim(), "Watching · /row: 1 matches");
    assert_eq!(buffer[(12, 10)].fg, palette.accent);
    assert_eq!(buffer[(13, 10)].fg, palette.accent);
    assert_eq!(buffer[(12, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(13, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(13, 10)].bg, palette.background);
    assert_eq!(buffer[(10, 10)].fg, palette.muted);
    assert_eq!(buffer[(18, 10)].fg, palette.muted);
    Ok(())
}

#[test]
fn dark_status_filters_and_searches_share_the_same_emphasis() -> AppResult<()> {
    // Given
    let mut state = state_with_queries("main", "row")?;
    state.action(Action::Focus, 9, Instant::now());
    let palette = Palette::new(false);
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(
        line(&buffer, 10).trim(),
        "Watching · filter: main · /row: 1 matches"
    );
    assert_eq!(buffer[(19, 10)].fg, palette.accent);
    assert_eq!(buffer[(27, 10)].fg, palette.accent);
    assert_eq!(buffer[(19, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(27, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(24, 10)].fg, palette.muted);
    assert_eq!(buffer[(32, 10)].fg, palette.muted);
    assert_eq!(buffer[(1, 10)].fg, palette.muted);
    Ok(())
}

#[test]
fn light_status_filters_and_searches_share_the_same_emphasis() -> AppResult<()> {
    // Given
    let mut state = state_with_queries("main", "row")?;
    state.action(Action::Focus, 9, Instant::now());
    let config = Config {
        light: true,
        ..Config::default()
    };
    let palette = Palette::new(true);
    // When
    let buffer = render(&state, &config, 120, 12)?;
    // Then
    assert_eq!(
        line(&buffer, 10).trim(),
        "Watching · filter: main · /row: 1 matches"
    );
    assert_eq!(buffer[(19, 10)].fg, palette.accent);
    assert_eq!(buffer[(27, 10)].fg, palette.accent);
    assert_eq!(buffer[(19, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(27, 10)].modifier, Modifier::empty());
    assert_eq!(buffer[(24, 10)].fg, palette.muted);
    assert_eq!(buffer[(32, 10)].fg, palette.muted);
    assert_eq!(buffer[(1, 10)].fg, palette.muted);
    Ok(())
}

#[test]
fn selected_file_paths_are_omitted_from_error_status() -> AppResult<()> {
    // Given
    let mut state = state_with("e\u{301}界abcdefghijklmnopqrstuvwxyz0123456789.rs", 1);
    state.action(Action::NextFile, 8, Instant::now());
    state.action(Action::Focus, 8, Instant::now());
    state.collection_finished(
        0,
        Err(GitError::Command("cannot compare".into())),
        Instant::now(),
    );
    // When
    let buffer = render(&state, &Config::default(), 80, 12)?;
    // Then
    assert_eq!(line(&buffer, 10).trim(), "Error: cannot compare");
    assert!(!line(&buffer, 10).contains("界"));
    assert!(!line(&buffer, 10).contains('…'));
    assert_eq!(buffer[(1, 10)].fg, Palette::new(false).removed_emphasis);
    assert_eq!(buffer[(11, 10)].fg, Palette::new(false).removed_emphasis);
    assert!(line(&buffer, 11).starts_with(" n/p files"));
    Ok(())
}

#[test]
fn an_updating_diff_keeps_its_activity_in_the_status() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::NextFile, 8, Instant::now());
    state.render_request(85, 9);
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(line(&buffer, 10).trim(), "Rendering");
    assert!(!line(&buffer, 10).contains("Unified"));
    assert!(!line(&buffer, 10).contains("whitespace"));
    assert!(!line(&buffer, 0).contains("Unified"));
    Ok(())
}

#[test]
fn a_redraw_keeps_the_displayed_activity_while_a_refresh_is_pending() -> AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = state_with("main.rs", 1);
    prepare(&mut state, 120, 12)?;
    let mut status = ActivityStatus::new(Activity::current(&state, true, false));
    state.collection_request();
    status.update(Activity::current(&state, true, false), now);
    let mut terminal = Terminal::new(TestBackend::new(120, 12))?;
    // When
    draw_with_activity(
        &mut terminal,
        &state,
        &Config::default(),
        status.displayed(),
    )?;
    // Then
    let buffer = terminal.backend().buffer();
    assert_eq!(line(buffer, 10).trim(), "Watching");
    assert!(line(buffer, 4).contains("row 1"));
    Ok(())
}

#[test]
fn errors_appear_immediately_while_an_activity_change_is_pending() -> AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = state_with_queries("main", "row")?;
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    state.collection_finished(0, Err(GitError::Command("cannot compare".into())), now);
    let mut terminal = Terminal::new(TestBackend::new(120, 12))?;
    // When
    draw_with_activity(
        &mut terminal,
        &state,
        &Config::default(),
        status.displayed(),
    )?;
    // Then
    let buffer = terminal.backend().buffer();
    assert_eq!(line(buffer, 10).trim(), "Error: cannot compare");
    assert_eq!(buffer[(1, 10)].fg, Palette::new(false).removed_emphasis);
    Ok(())
}

#[test]
fn search_results_and_diff_content_update_while_activity_is_pending() -> AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = state_with("main.rs", 1);
    state.action(Action::Search, 9, now);
    state.paste("row", now);
    prepare(&mut state, 120, 12)?;
    let mut status = ActivityStatus::new(Activity::Rendering);
    status.update(Activity::current(&state, true, false), now);
    let mut terminal = Terminal::new(TestBackend::new(120, 12))?;
    // When
    draw_with_activity(
        &mut terminal,
        &state,
        &Config::default(),
        status.displayed(),
    )?;
    // Then
    let buffer = terminal.backend().buffer();
    assert_eq!(line(buffer, 10).trim(), "Rendering · /row: 1 matches");
    assert!(line(buffer, 4).contains("row 1"));
    Ok(())
}

#[test]
fn split_mode_and_its_inner_divider_start_at_32_content_columns() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Sidebar, 8, Instant::now());
    state.action(Action::Layout, 8, Instant::now());
    prepare(&mut state, 33, 12)?;
    // When
    let buffer = render(&state, &Config::default(), 33, 12)?;
    // Then
    assert!(line(&buffer, 10).starts_with(" Watching"));
    assert!(!line(&buffer, 10).contains("Split"));
    assert!(line(&buffer, 4).contains("row 1"));
    assert_eq!(buffer[(15, 4)].symbol(), "│");
    assert_eq!(buffer[(32, 4)].symbol(), " ");
    Ok(())
}

#[test]
fn resizing_below_32_content_columns_returns_to_unified_mode() -> AppResult<()> {
    // Given
    let now = Instant::now();
    let mut state = state_with("main.rs", 1);
    state.action(Action::Sidebar, 8, now);
    state.action(Action::Layout, 8, now);
    prepare(&mut state, 33, 12)?;
    // When
    prepare(&mut state, 32, 12)?;
    let buffer = render(&state, &Config::default(), 32, 12)?;
    // Then
    assert!(line(&buffer, 10).starts_with(" Watching"));
    assert_eq!(state.toast(), None);
    assert!(line(&buffer, 4).contains("row 1"));
    assert!(!line(&buffer, 4).contains('│'));
    assert_eq!(buffer[(31, 4)].symbol(), " ");
    Ok(())
}

#[test]
fn paging_leaves_room_for_the_file_header_and_preserves_the_scrollbar_lane() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 40);
    state.action(Action::Sidebar, 7, Instant::now());
    prepare(&mut state, 60, 10)?;
    // When
    let (_, height) = ViewAreas::new(Rect::new(0, 0, 60, 10), state.sidebar()).viewport();
    state.action(Action::PageDown, height, Instant::now());
    let buffer = render(&state, &Config::default(), 60, 10)?;
    // Then
    assert!(line(&buffer, 0).contains("main.rs"));
    assert!(line(&buffer, 1).starts_with(&"─".repeat(59)));
    assert!(line(&buffer, 2).contains("row 4"));
    assert!(line(&buffer, 6).contains("row 8"));
    assert_eq!(buffer[(59, 0)].symbol(), " ");
    assert_eq!(buffer[(59, 1)].symbol(), "┃");
    assert_eq!(buffer[(59, 6)].symbol(), " ");
    assert_eq!(buffer[(59, 7)].symbol(), "─");
    assert_eq!(line(&buffer, 8).trim(), "Watching");
    Ok(())
}

#[test]
fn minimum_terminal_size_keeps_content_the_status_rule_and_input_on_separate_rows() -> AppResult<()>
{
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Search, 3, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 12, 6)?;
    // Then
    assert_eq!(line(&buffer, 3), "────────────");
    assert_eq!(line(&buffer, 4).trim(), "Watching");
    assert!(line(&buffer, 5).starts_with("Search › "));
    assert!(!line(&buffer, 0).contains("All changes"));
    Ok(())
}

#[test]
fn help_uses_the_available_height_and_keeps_its_scrollbar_above_the_footer() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Help, 8, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 20)?;
    // Then
    let rect = super::help::Help::new(
        Rect::new(0, 0, 120, 20),
        &Config::default().bindings,
        &Palette::new(false),
    )
    .rect();
    assert_eq!(rect.width, 60);
    assert_eq!(rect.height, 20);
    assert_eq!(buffer[(rect.x, rect.y)].symbol(), "┌");
    assert_eq!(buffer[(rect.right() - 1, rect.y)].symbol(), "┐");
    assert_eq!(buffer[(rect.x, rect.bottom() - 1)].symbol(), "└");
    assert_eq!(buffer[(rect.right() - 1, rect.bottom() - 1)].symbol(), "┘");
    assert!(line(&buffer, rect.y).contains(concat!("Dift ", env!("CARGO_PKG_VERSION"))));
    assert_eq!(buffer[(rect.x + 2, rect.y + 1)].symbol(), " ");
    assert_eq!(buffer[(rect.right() - 2, rect.y + 2)].symbol(), "┃");
    assert_eq!(
        buffer[(rect.right() - 2, rect.y + 2)].fg,
        Palette::new(false).accent
    );
    assert_eq!(buffer[(rect.right() - 3, rect.y + 2)].symbol(), " ");
    assert_eq!(buffer[(rect.x + 2, rect.bottom() - 3)].symbol(), " ");
    assert_eq!(buffer[(rect.right() - 2, rect.bottom() - 2)].symbol(), " ");
    assert!(
        line(&buffer, rect.bottom() - 2)
            .contains("down|j/up|k scroll · pgDn/pgUp page · esc close")
    );
    Ok(())
}

#[test]
fn help_lists_the_find_binding() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Help, 8, Instant::now());
    super::super::help::Help::new(
        Rect::new(0, 0, 120, 20),
        &Config::default().bindings,
        &Palette::new(false),
    )
    .sync(&mut state);
    state.action(Action::PageDown, 8, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 20)?;
    // Then
    assert!(
        (0..20)
            .map(|y| line(&buffer, y))
            .any(|row| row.contains("shift-f") && row.contains("Find a file"))
    );
    Ok(())
}

#[test]
fn whitespace_toasts_use_a_bordered_panel_above_the_status_and_prompt() -> AppResult<()> {
    // Given
    let mut state = state_with_whitespace_toast();
    state.action(Action::Search, 8, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(buffer[(119, 6)].symbol(), "┐");
    assert!(line(&buffer, 7).ends_with("│ Whitespace: ignore-changes · 1 hidden by whitespace │"));
    assert_eq!(buffer[(119, 8)].symbol(), "┘");
    assert_eq!(buffer[(117, 7)].bg, Palette::new(false).panel);
    assert_eq!(buffer[(119, 6)].fg, Palette::new(false).accent);
    assert_eq!(buffer[(119, 9)].symbol(), "─");
    assert_eq!(line(&buffer, 10).trim(), "Watching");
    assert!(line(&buffer, 11).starts_with("Search › "));
    Ok(())
}

#[test]
fn whitespace_counts_appear_only_in_the_toast() -> AppResult<()> {
    // Given
    let state = state_with_whitespace_toast();
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert!(line(&buffer, 7).ends_with("│ Whitespace: ignore-changes · 1 hidden by whitespace │"));
    assert_eq!(line(&buffer, 10).trim(), "Watching");
    Ok(())
}

#[test]
fn pending_whitespace_changes_keep_only_activity_in_the_status() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Whitespace, 8, Instant::now());
    state.collection_request();
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(line(&buffer, 10).trim(), "Refreshing");
    assert_eq!(state.toast(), None);
    Ok(())
}

#[test]
fn narrow_toasts_truncate_without_covering_the_status_or_input() -> AppResult<()> {
    // Given
    let mut state = state_with_whitespace_toast();
    state.action(Action::Search, 3, Instant::now());
    let config = Config {
        light: true,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 12, 6)?;
    // Then
    assert_eq!(line(&buffer, 0), "┌──────────┐");
    assert_eq!(line(&buffer, 1), "│ Whitesp… │");
    assert_eq!(line(&buffer, 2), "└──────────┘");
    assert_eq!(buffer[(2, 1)].bg, Palette::new(true).panel);
    assert_eq!(line(&buffer, 3), "────────────");
    assert_eq!(line(&buffer, 4).trim(), "Watching");
    assert!(line(&buffer, 5).starts_with("Search › "));
    Ok(())
}

#[test]
fn help_hides_the_active_toast() -> AppResult<()> {
    // Given
    let mut state = state_with_whitespace_toast();
    state.action(Action::Help, 8, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 12)?;
    // Then
    assert_eq!(buffer[(119, 6)].symbol(), " ");
    assert!(!line(&buffer, 7).contains("Whitespace:"));
    assert!(line(&buffer, 0).contains(concat!("Dift ", env!("CARGO_PKG_VERSION"))));
    Ok(())
}

#[test]
fn help_lists_the_line_number_toggle() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Help, 8, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 120, 20)?;
    // Then
    assert!(
        (0..20)
            .map(|y| line(&buffer, y))
            .any(|row| row.contains("shift-l") && row.contains("line numbers"))
    );
    Ok(())
}

#[test]
fn narrow_help_wraps_descriptions_below_the_keys() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.action(Action::Help, 8, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 32, 30)?;
    // Then
    let rows: Vec<_> = (0..30).map(|y| line(&buffer, y)).collect();
    assert!(rows.iter().any(|row| row.contains("  Show or hide the")));
    assert!(rows.iter().any(|row| row.contains("  tree")));
    Ok(())
}

#[test]
fn help_uses_the_last_interior_row_for_content() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    let config = Config::default();
    state.action(Action::Help, 8, Instant::now());
    super::super::help::Help::new(
        Rect::new(0, 0, 120, 20),
        &config.bindings,
        &Palette::new(false),
    )
    .sync(&mut state);
    state.action(Action::Last, 8, Instant::now());
    // When
    let buffer = render(&state, &config, 120, 20)?;
    // Then
    assert!(line(&buffer, 16).contains("Delete text after the cursor"));
    Ok(())
}

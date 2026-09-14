use super::*;
use crate::{
    config::Icons,
    model::{Category, Change, Snapshot},
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
mod activity;
mod mouse;

fn state_with(files: &[(&str, Category)]) -> State {
    state_with_statuses(
        &files
            .iter()
            .map(|(path, category)| {
                (
                    *path,
                    *category,
                    if *category == Category::Untracked {
                        '?'
                    } else {
                        'M'
                    },
                )
            })
            .collect::<Vec<_>>(),
    )
}

fn state_with_statuses(files: &[(&str, Category, char)]) -> State {
    let mut state = State::new(false);
    state.update_snapshot(Snapshot {
        changes: files
            .iter()
            .map(|(path, category, status)| Change {
                path: path.into(),
                old_path: None,
                category: *category,
                status: *status,
                hunks: Vec::new(),
                summary: None,
                oversized: false,
                old_source: None,
                new_source: None,
            })
            .collect(),
        ..Snapshot::default()
    });
    state
}

fn sample() -> State {
    state_with(&[
        ("src/state.rs", Category::Unstaged),
        ("src/ui.rs", Category::Unstaged),
        ("src/ui.rs", Category::Staged),
        ("README.md", Category::Untracked),
    ])
}

fn render(state: &State, config: &Config, width: u16, height: u16) -> crate::AppResult<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|frame| {
        tree::draw(
            frame,
            state,
            config,
            frame.area(),
            &mut tree::Viewport::default(),
        );
    })?;
    Ok(terminal.backend().buffer().clone())
}

fn line(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

fn filter(state: &mut State, query: &str) {
    state.action(Action::Filter, 10, Instant::now());
    state.paste(query, Instant::now());
    state.action(Action::Accept, 10, Instant::now());
}

#[test]
fn compact_rows_align_counts_and_badges_without_icons() -> crate::AppResult<()> {
    // Given
    let state = sample();
    // When
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert!(line(&buffer, 0).contains("All changes"));
    assert_eq!(buffer[(31, 0)].symbol(), "3");
    assert_eq!(line(&buffer, 1), " │ ▾ src                       2 │");
    assert_eq!(line(&buffer, 3), " │     ui.rs                  MM │");
    assert_eq!(buffer[(31, 2)].symbol(), "M");
    assert_eq!(buffer[(31, 4)].symbol(), "?");
    assert_eq!(buffer[(30, 3)].fg, Palette::new(false).removed_foreground);
    assert_eq!(buffer[(31, 3)].fg, Palette::new(false).added_foreground);
    Ok(())
}

#[test]
fn nerd_font_icons_fit_the_same_badge_column() -> crate::AppResult<()> {
    // Given
    let state = sample();
    let config = Config {
        icons: Icons::Nerd,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 34, 8)?;
    // Then
    assert!(line(&buffer, 0).contains(" All changes"));
    assert_eq!(buffer[(31, 0)].symbol(), "3");
    assert_eq!(line(&buffer, 1), " │  src                       2 │");
    assert_eq!(buffer[(5, 2)].symbol(), "");
    assert!(line(&buffer, 2).contains(" state.rs"));
    assert_eq!(buffer[(31, 2)].symbol(), "M");
    assert_eq!(buffer[(30, 3)].symbol(), "M");
    assert_eq!(buffer[(31, 3)].symbol(), "M");
    Ok(())
}

#[test]
fn collapsed_folders_and_unknown_files_have_distinct_icons() -> crate::AppResult<()> {
    // Given
    let mut state = state_with(&[
        ("src/a.rs", Category::Unstaged),
        ("data.xyz", Category::Staged),
    ]);
    state.action(Action::Down, 10, Instant::now());
    state.action(Action::Left, 10, Instant::now());
    let config = Config {
        icons: Icons::Nerd,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 34, 8)?;
    // Then
    assert_eq!(line(&buffer, 1), "▎   src                       1 │");
    assert!(line(&buffer, 2).contains(" data.xyz"));
    assert_eq!(buffer[(1, 2)].symbol(), " ");
    Ok(())
}

#[test]
fn selected_files_stay_visible_when_the_tree_loses_focus() -> crate::AppResult<()> {
    // Given
    let mut state = sample();
    state.action(Action::NextFile, 10, Instant::now());
    let focused = render(&state, &Config::default(), 34, 8)?;
    state.action(Action::Focus, 10, Instant::now());
    // When
    let unfocused = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(focused[(0, 2)].symbol(), "▎");
    assert_eq!(unfocused[(0, 2)].symbol(), " ");
    assert_ne!(focused[(11, 2)].bg, unfocused[(11, 2)].bg);
    assert_eq!(unfocused[(11, 2)].bg, Palette::new(false).panel);
    assert_eq!(focused[(31, 2)].fg, unfocused[(31, 2)].fg);
    assert_eq!(
        unfocused[(31, 2)].fg,
        Palette::new(false).removed_foreground
    );
    assert_eq!(focused[(3, 2)].symbol(), "│");
    assert_eq!(unfocused[(3, 2)].symbol(), "│");
    assert_eq!(unfocused[(1, 2)].symbol(), " ");
    Ok(())
}

#[test]
fn deep_paths_give_space_to_the_filename_and_preserve_its_extension() -> crate::AppResult<()> {
    // Given
    let mut state = state_with(&[("a/b/c/d/e/f/long_filename.rs", Category::Unstaged)]);
    state.action(Action::NextFile, 10, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 16, 12)?;
    // Then
    assert!(
        line(&buffer, 2).contains("long…e.rs"),
        "{}",
        line(&buffer, 2)
    );
    assert_eq!(buffer[(13, 2)].symbol(), "M");
    assert_eq!(buffer[(1, 2)].symbol(), " ");
    Ok(())
}

#[test]
fn unicode_names_keep_combining_marks_and_wide_characters_intact() -> crate::AppResult<()> {
    // Given
    let state = state_with(&[("e\u{301}界verylong.rs", Category::Unstaged)]);
    // When
    let buffer = render(&state, &Config::default(), 16, 6)?;
    // Then
    assert!(line(&buffer, 1).contains("e\u{301}界"));
    assert!(line(&buffer, 1).contains("…"));
    assert!(line(&buffer, 1).contains(".rs"));
    assert_eq!(buffer[(13, 1)].symbol(), "M");
    Ok(())
}

#[test]
fn filtering_starts_with_the_root_counts_and_highlights_matches() -> crate::AppResult<()> {
    // Given
    let mut state = sample();
    filter(&mut state, "ui");
    // When
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert!(line(&buffer, 0).contains("All changes"));
    assert!(line(&buffer, 0).ends_with("1/3 │"));
    assert_eq!(buffer[(7, 2)].symbol(), "u");
    assert!(buffer[(7, 2)].modifier.contains(Modifier::UNDERLINED));
    assert!(!buffer[(9, 2)].modifier.contains(Modifier::UNDERLINED));
    Ok(())
}

#[test]
fn combining_marks_do_not_shift_filter_highlighting() -> crate::AppResult<()> {
    // Given
    let mut state = state_with(&[("a\u{301}/e\u{301}long.rs", Category::Unstaged)]);
    filter(&mut state, "long");
    // When
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(buffer[(7, 2)].symbol(), "e\u{301}");
    assert!(!buffer[(7, 2)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buffer[(8, 2)].symbol(), "l");
    assert!(buffer[(8, 2)].modifier.contains(Modifier::UNDERLINED));
    assert!(buffer[(11, 2)].modifier.contains(Modifier::UNDERLINED));
    assert!(!buffer[(12, 2)].modifier.contains(Modifier::UNDERLINED));
    Ok(())
}

#[test]
fn tiny_panes_omit_prefixes_before_losing_badges() -> crate::AppResult<()> {
    // Given
    let state = state_with(&[
        ("long_name.rs", Category::Unstaged),
        ("long_name.rs", Category::Staged),
    ]);
    let config = Config {
        icons: Icons::Nerd,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 8, 6)?;
    // Then
    assert_eq!(line(&buffer, 1), " l… MM │");
    Ok(())
}

#[test]
fn an_empty_filter_result_explains_why_the_tree_is_empty() -> crate::AppResult<()> {
    // Given
    let mut state = sample();
    filter(&mut state, "missing");
    // When
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert!(line(&buffer, 0).contains("All changes"));
    assert!(line(&buffer, 0).ends_with("0/3 │"));
    assert!(line(&buffer, 1).contains("No matching files"));
    Ok(())
}

#[test]
fn a_clean_repository_has_a_distinct_empty_state() -> crate::AppResult<()> {
    // Given
    let state = State::new(false);
    // When
    let buffer = render(&state, &Config::default(), 34, 6)?;
    // Then
    assert!(line(&buffer, 0).contains("All changes"));
    assert_eq!(buffer[(31, 0)].symbol(), "0");
    assert!(line(&buffer, 1).contains("No changes"));
    Ok(())
}

#[test]
fn the_light_palette_preserves_selected_badge_colors() -> crate::AppResult<()> {
    // Given
    let mut state = sample();
    state.action(Action::NextFile, 10, Instant::now());
    let config = Config {
        light: true,
        icons: Icons::Nerd,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 34, 8)?;
    // Then
    assert_eq!(buffer[(0, 2)].symbol(), "▎");
    assert_eq!(buffer[(31, 2)].fg, Palette::new(true).removed_foreground);
    assert_ne!(buffer[(10, 2)].bg, Palette::new(true).background);
    Ok(())
}

#[test]
fn scrolling_keeps_context_and_a_muted_track_behind_the_thumb() -> crate::AppResult<()> {
    // Given
    let mut state = state_with(&[
        ("a", Category::Unstaged),
        ("b", Category::Unstaged),
        ("c", Category::Unstaged),
        ("d", Category::Unstaged),
        ("e", Category::Unstaged),
        ("f", Category::Unstaged),
        ("g", Category::Unstaged),
    ]);
    state.action(Action::NextFile, 10, Instant::now());
    state.action(Action::NextFile, 10, Instant::now());
    state.action(Action::NextFile, 10, Instant::now());
    state.action(Action::NextFile, 10, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 34, 5)?;
    // Then
    assert!(line(&buffer, 2).contains("d"));
    assert!(line(&buffer, 0).contains("b"));
    assert!(line(&buffer, 4).contains("f"));
    assert_eq!(buffer[(33, 0)].symbol(), "│");
    assert_eq!(buffer[(33, 0)].fg, Palette::new(false).muted);
    assert_eq!(buffer[(33, 2)].symbol(), "┃");
    assert_eq!(buffer[(33, 4)].symbol(), "│");
    assert_eq!(buffer[(33, 4)].fg, Palette::new(false).muted);
    Ok(())
}

fn nested_branches() -> State {
    state_with(&[
        ("src/render/cache.rs", Category::Unstaged),
        ("src/ui.rs", Category::Unstaged),
        ("src-other/a.rs", Category::Staged),
    ])
}

#[test]
fn a_selected_directory_guides_its_subtree_without_marking_other_branches() -> crate::AppResult<()>
{
    // Given
    let mut state = nested_branches();
    state.action(Action::Down, 10, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 34, 10)?;
    // Then
    assert_eq!(buffer[(1, 1)].symbol(), " ");
    assert_eq!(buffer[(3, 2)].symbol(), "│");
    assert_eq!(buffer[(3, 3)].symbol(), "│");
    assert_eq!(buffer[(3, 4)].symbol(), "│");
    assert_eq!(buffer[(5, 3)].symbol(), " ");
    assert_eq!(buffer[(1, 6)].symbol(), " ");
    assert_eq!(buffer[(3, 6)].symbol(), " ");
    Ok(())
}

#[test]
fn a_nested_file_guides_only_its_immediate_parent() -> crate::AppResult<()> {
    // Given
    let mut state = nested_branches();
    state.action(Action::NextFile, 10, Instant::now());
    let config = Config {
        icons: Icons::Nerd,
        ..Config::default()
    };
    // When
    let buffer = render(&state, &config, 34, 10)?;
    // Then
    assert_eq!(buffer[(1, 3)].symbol(), " ");
    assert_eq!(buffer[(3, 3)].symbol(), " ");
    assert_eq!(buffer[(5, 3)].symbol(), "│");
    assert_eq!(buffer[(3, 4)].symbol(), " ");
    assert_eq!(buffer[(3, 6)].symbol(), " ");
    assert_eq!(buffer[(31, 3)].symbol(), "M");
    Ok(())
}

#[test]
fn the_active_directory_has_a_guide_even_without_following_siblings() -> crate::AppResult<()> {
    // Given
    let mut state = state_with(&[("src/ui.rs", Category::Unstaged)]);
    state.action(Action::Down, 10, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 34, 6)?;
    // Then
    assert_eq!(buffer[(3, 2)].symbol(), "│");
    assert_eq!(buffer[(1, 2)].symbol(), " ");
    Ok(())
}

#[test]
fn selecting_a_root_file_activates_the_repository_guide() -> crate::AppResult<()> {
    // Given
    let mut state = sample();
    state.action(Action::Last, 10, Instant::now());
    // When
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(buffer[(1, 1)].symbol(), "│");
    assert_eq!(buffer[(1, 2)].symbol(), "│");
    assert_eq!(buffer[(1, 4)].symbol(), "│");
    assert_eq!(buffer[(3, 2)].symbol(), " ");
    Ok(())
}

#[test]
fn filtering_keeps_the_selected_files_parent_guide() -> crate::AppResult<()> {
    // Given
    let mut state = sample();
    state.action(Action::NextFile, 10, Instant::now());
    filter(&mut state, "state");
    // When
    let buffer = render(&state, &Config::default(), 34, 8)?;
    // Then
    assert_eq!(buffer[(3, 2)].symbol(), "│");
    assert_eq!(buffer[(1, 2)].symbol(), " ");
    assert!(line(&buffer, 2).contains("state.rs"));
    Ok(())
}

macro_rules! status_badge_test {
    ($name:ident, $category:ident, $status:literal, $light:literal, $color:ident) => {
        #[test]
        fn $name() -> crate::AppResult<()> {
            // Given
            let state = state_with_statuses(&[("file.rs", Category::$category, $status)]);
            let config = Config {
                light: $light,
                ..Config::default()
            };
            // When
            let buffer = render(&state, &config, 34, 6)?;
            // Then
            assert_eq!(buffer[(31, 1)].symbol(), $status.to_string());
            assert_eq!(buffer[(31, 1)].fg, Palette::new($light).$color);
            Ok(())
        }
    };
}
status_badge_test!(
    dark_unstaged_a_badge,
    Unstaged,
    'A',
    false,
    removed_foreground
);
status_badge_test!(
    dark_unstaged_m_badge,
    Unstaged,
    'M',
    false,
    removed_foreground
);
status_badge_test!(
    dark_unstaged_d_badge,
    Unstaged,
    'D',
    false,
    removed_foreground
);
status_badge_test!(
    dark_unstaged_r_badge,
    Unstaged,
    'R',
    false,
    removed_foreground
);
status_badge_test!(
    dark_unstaged_c_badge,
    Unstaged,
    'C',
    false,
    removed_foreground
);
status_badge_test!(
    dark_unstaged_t_badge,
    Unstaged,
    'T',
    false,
    removed_foreground
);
status_badge_test!(
    dark_unstaged_u_badge,
    Unstaged,
    'U',
    false,
    removed_foreground
);
status_badge_test!(dark_staged_a_badge, Staged, 'A', false, added_foreground);
status_badge_test!(dark_staged_m_badge, Staged, 'M', false, added_foreground);
status_badge_test!(dark_staged_d_badge, Staged, 'D', false, added_foreground);
status_badge_test!(dark_staged_r_badge, Staged, 'R', false, added_foreground);
status_badge_test!(dark_staged_c_badge, Staged, 'C', false, added_foreground);
status_badge_test!(dark_staged_t_badge, Staged, 'T', false, added_foreground);
status_badge_test!(
    dark_untracked_untracked_badge,
    Untracked,
    '?',
    false,
    removed_foreground
);
status_badge_test!(dark_comparison_a_badge, Comparison, 'A', false, muted);
status_badge_test!(dark_comparison_m_badge, Comparison, 'M', false, muted);
status_badge_test!(dark_comparison_d_badge, Comparison, 'D', false, muted);
status_badge_test!(dark_comparison_r_badge, Comparison, 'R', false, muted);
status_badge_test!(
    light_unstaged_a_badge,
    Unstaged,
    'A',
    true,
    removed_foreground
);
status_badge_test!(
    light_unstaged_m_badge,
    Unstaged,
    'M',
    true,
    removed_foreground
);
status_badge_test!(
    light_unstaged_d_badge,
    Unstaged,
    'D',
    true,
    removed_foreground
);
status_badge_test!(
    light_unstaged_r_badge,
    Unstaged,
    'R',
    true,
    removed_foreground
);
status_badge_test!(
    light_unstaged_c_badge,
    Unstaged,
    'C',
    true,
    removed_foreground
);
status_badge_test!(
    light_unstaged_t_badge,
    Unstaged,
    'T',
    true,
    removed_foreground
);
status_badge_test!(
    light_unstaged_u_badge,
    Unstaged,
    'U',
    true,
    removed_foreground
);
status_badge_test!(light_staged_a_badge, Staged, 'A', true, added_foreground);
status_badge_test!(light_staged_m_badge, Staged, 'M', true, added_foreground);
status_badge_test!(light_staged_d_badge, Staged, 'D', true, added_foreground);
status_badge_test!(light_staged_r_badge, Staged, 'R', true, added_foreground);
status_badge_test!(light_staged_c_badge, Staged, 'C', true, added_foreground);
status_badge_test!(light_staged_t_badge, Staged, 'T', true, added_foreground);
status_badge_test!(
    light_untracked_untracked_badge,
    Untracked,
    '?',
    true,
    removed_foreground
);
status_badge_test!(light_comparison_a_badge, Comparison, 'A', true, muted);
status_badge_test!(light_comparison_m_badge, Comparison, 'M', true, muted);
status_badge_test!(light_comparison_d_badge, Comparison, 'D', true, muted);
status_badge_test!(light_comparison_r_badge, Comparison, 'R', true, muted);

#[test]
fn mixed_status_badges_keep_unstaged_before_staged() -> crate::AppResult<()> {
    // Given
    let state = state_with_statuses(&[
        ("file.rs", Category::Staged, 'A'),
        ("file.rs", Category::Unstaged, 'M'),
        ("file.rs", Category::Unstaged, 'M'),
    ]);
    // When
    let buffer = render(&state, &Config::default(), 34, 6)?;
    // Then
    assert_eq!(buffer[(30, 1)].symbol(), "M");
    assert_eq!(buffer[(30, 1)].fg, Palette::new(false).removed_foreground);
    assert_eq!(buffer[(31, 1)].symbol(), "A");
    assert_eq!(buffer[(31, 1)].fg, Palette::new(false).added_foreground);
    assert_eq!(buffer[(31, 0)].symbol(), "1");
    Ok(())
}

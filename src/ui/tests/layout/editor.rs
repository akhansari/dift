use super::*;
use crate::state::{EditorPosition, EditorTarget};
use std::path::Path;

#[test]
fn uppercase_o_requests_a_normal_editor_open() {
    // Given
    let mut state = State::new(false);
    // When
    let effects = state.handle_key(
        KeyEvent::from(event::KeyCode::Char('O')),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(effects.open_editor, Some(EditorPosition::Default));
}

#[test]
fn opening_normally_does_not_include_a_visible_hunk_position() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 12);
    prepare(&mut state, 100, 20)?;
    state.focus_diff();
    // When
    let target = state.editor_target(17, EditorPosition::Default);
    // Then
    assert_eq!(
        target,
        Some(EditorTarget {
            path: Path::new("main.rs"),
            line: None
        })
    );
    Ok(())
}

#[test]
fn opening_a_visible_hunk_includes_its_first_change() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 12);
    prepare(&mut state, 100, 20)?;
    state.focus_diff();
    // When
    let target = state.editor_target(17, EditorPosition::VisibleHunk);
    // Then
    assert_eq!(
        target,
        Some(EditorTarget {
            path: Path::new("main.rs"),
            line: Some(1)
        })
    );
    Ok(())
}

fn metadata_state(
    category: Category,
    status: char,
    oversized: bool,
    split: bool,
) -> AppResult<State> {
    let mut state = State::new(split);
    state.update_snapshot(Snapshot {
        changes: ["src/first.rs", "src/second.rs"]
            .into_iter()
            .map(|path| Change {
                path: path.into(),
                old_path: (status == 'R').then(|| "old.rs".into()),
                category,
                status,
                hunks: Vec::new(),
                summary: Some("Content omitted".into()),
                oversized,
                old_source: None,
                new_source: None,
            })
            .collect(),
        ..Snapshot::default()
    });
    prepare(&mut state, 100, 20)?;
    Ok(state)
}

#[test]
fn opening_from_the_tree_uses_its_selection_despite_diff_scrolling() -> AppResult<()> {
    // Given
    let mut state = metadata_state(Category::Unstaged, 'M', false, false)?;
    state.action(Action::NextFile, 1, Instant::now());
    state.scroll_by(usize::MAX, true, 1);
    // When
    let path = state.active_file(1);
    // Then
    assert_eq!(path, Some(Path::new("src/first.rs")));
    Ok(())
}

#[test]
fn a_selected_directory_has_no_active_tree_file() -> AppResult<()> {
    // Given
    let state = metadata_state(Category::Unstaged, 'M', false, false)?;
    // When
    let path = state.active_file(20);
    // Then
    assert_eq!(path, None);
    Ok(())
}

#[test]
fn a_diff_with_no_visible_file_has_no_editor_target() {
    // Given
    let mut state = State::new(false);
    state.focus_diff();
    // When
    let path = state.active_file(20);
    // Then
    assert_eq!(path, None);
}

#[test]
fn the_first_entering_header_identifies_a_file_without_source_anchors() -> AppResult<()> {
    // Given
    let mut state = metadata_state(Category::Unstaged, 'M', false, false)?;
    state.focus_diff();
    // When
    let path = state.active_file(20);
    // Then
    assert_eq!(path, Some(Path::new("src/first.rs")));
    Ok(())
}

macro_rules! current_file_test {
    ($name:ident, $category:expr, $status:literal, $oversized:literal, $split:literal) => {
        #[test]
        fn $name() -> AppResult<()> {
            // Given
            let mut state = metadata_state($category, $status, $oversized, $split)?;
            state.focus_diff();
            state.scroll_by(usize::MAX, true, 1);
            // When
            let path = state.active_file(1);
            // Then
            assert_eq!(path, Some(Path::new("src/second.rs")));
            assert_eq!(state.selection(), Path::new(""));
            Ok(())
        }
    };
}

current_file_test!(
    aggregated_diff_scrolling_selects_the_current_file,
    Category::Unstaged,
    'M',
    false,
    false
);
current_file_test!(
    split_diff_scrolling_selects_the_current_file,
    Category::Unstaged,
    'M',
    false,
    true
);
current_file_test!(
    renames_target_the_destination_path,
    Category::Unstaged,
    'R',
    false,
    false
);
current_file_test!(
    oversized_files_can_open_without_loading_content,
    Category::Unstaged,
    'M',
    true,
    false
);
current_file_test!(
    staged_files_target_the_working_tree_path,
    Category::Staged,
    'M',
    false,
    false
);
current_file_test!(
    comparison_files_target_the_working_tree_path,
    Category::Comparison,
    'M',
    false,
    false
);

#[test]
fn the_default_editor_shortcut_requests_a_launch() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    let config = Config::parse(&Config::template())?;
    // When
    let effects = state.handle_key(
        KeyEvent::from(event::KeyCode::Char('o')),
        &config.bindings,
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(
        effects.open_editor,
        Some(crate::state::EditorPosition::VisibleHunk)
    );
    Ok(())
}

#[test]
fn the_editor_shortcut_can_be_rebound() -> AppResult<()> {
    // Given
    let mut state = State::new(false);
    let config = Config::parse("keybindings { global { open-editor \"v\"; }; }")?;
    // When
    let effects = state.handle_key(
        KeyEvent::from(event::KeyCode::Char('v')),
        &config.bindings,
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(
        effects.open_editor,
        Some(crate::state::EditorPosition::VisibleHunk)
    );
    Ok(())
}

#[test]
fn the_editor_shortcut_inserts_text_in_a_search_prompt() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Search, 20, Instant::now());
    // When
    let effects = state.handle_key(
        KeyEvent::from(event::KeyCode::Char('o')),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(effects.open_editor, None);
    assert_eq!(state.search(), "o");
}

#[test]
fn help_suppresses_editor_launches() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Help, 20, Instant::now());
    // When
    let effects = state.handle_key(
        KeyEvent::from(event::KeyCode::Char('o')),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert_eq!(effects.open_editor, None);
    assert!(state.help());
}

#[test]
fn stage_target_follows_the_visible_diff_file() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 12);
    prepare(&mut state, 100, 20)?;
    state.focus_diff();
    // When
    let target = state.stage_target(17);
    // Then
    assert_eq!(
        target,
        Some(crate::git::StageTarget::File("main.rs".into()))
    );
    Ok(())
}

#[test]
fn tree_root_targets_all_changes() {
    // Given
    let state = state_with("main.rs", 12);
    // When
    let target = state.stage_target(17);
    // Then
    assert_eq!(target, Some(crate::git::StageTarget::Subtree("".into())));
}

#[test]
fn space_in_a_prompt_is_text() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Filter, 20, Instant::now());
    // When
    let effects = state.handle_key(
        KeyEvent::from(event::KeyCode::Char(' ')),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert!(!effects.toggle_stage);
    assert_eq!(
        state.prompt().map(|prompt| prompt.editor().text()),
        Some(" ")
    );
}

#[test]
fn help_ignores_space() {
    // Given
    let mut state = State::new(false);
    state.action(Action::Help, 20, Instant::now());
    // When
    let effects = state.handle_key(
        KeyEvent::from(event::KeyCode::Char(' ')),
        &Bindings::defaults(),
        20,
        Instant::now(),
    );
    // Then
    assert!(!effects.toggle_stage);
    assert!(state.help());
}

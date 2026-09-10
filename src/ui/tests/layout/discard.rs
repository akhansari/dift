use super::*;
use crate::git::ChangeTarget;

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|row| line(buffer, row))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn discard_menu_shows_target_without_removed_notices() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.begin_discard(ChangeTarget::Subtree("".into()));
    // When
    let buffer = render(&state, &Config::default(), 70, 18)?;
    // Then
    let text = text(&buffer);
    assert!(text.contains("All changes in repository"));
    assert!(!text.contains("Permanent: no undo"));
    assert!(!text.contains("Includes changes hidden by filters"));
    assert!(text.contains("› Cancel"));
    Ok(())
}

#[test]
fn discard_long_target_wraps_without_interpreting_control_characters() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.begin_discard(ChangeTarget::File(
        "long/directory/with/a/long/path/and/界界界/filename\n.rs".into(),
    ));
    // When
    let buffer = render(&state, &Config::default(), 50, 20)?;
    // Then
    let text = text(&buffer);
    assert!(text.contains("File: \"long/directory/with/a/long/path"));
    assert!(text.contains("filename\\n.rs\""));
    assert!(text.contains("› Cancel"));
    Ok(())
}

#[test]
fn discard_explains_unavailable_options() -> AppResult<()> {
    // Given
    let mut state = state_with("main.rs", 1);
    state.begin_discard(ChangeTarget::File("main.rs".into()));
    state.discard_prepared([
        Err("Unresolved conflict: main.rs".into()),
        Err("Unresolved conflict: main.rs".into()),
    ]);
    // When
    let buffer = render(&state, &Config::default(), 100, 20)?;
    // Then
    let text = text(&buffer);
    assert!(text.contains("Discard all changes unavailable: Unresolved conflict: main.rs"));
    assert!(text.contains("Discard all changes (unavailable)"));
    assert!(text.contains("› Cancel"));
    Ok(())
}

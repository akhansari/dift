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
fn discard_menu_keeps_options_above_its_shortcut_footer() -> AppResult<()> {
    // Given: an open discard menu with its options visible.
    let mut state = state_with("main.rs", 1);
    state.begin_discard(ChangeTarget::File("main.rs".into()));
    // When: drawing the modal.
    let buffer = render(&state, &Config::default(), 70, 18)?;
    // Then: the options remain above the shortcut footer.
    let text = text(&buffer);
    assert!(text.contains("› Cancel"));
    assert!(!text.contains(" accept · "));
    let option = text
        .lines()
        .position(|line| line.contains("› Cancel"))
        .ok_or("Missing cancel option")?;
    let footer = text
        .lines()
        .position(|line| line.contains("down|j/up|k move · enter select · esc cancel"))
        .ok_or("Missing discard footer")?;
    assert!(option < footer, "discard footer must follow its options");
    assert!(
        !text
            .lines()
            .nth(option)
            .ok_or("Missing option row")?
            .contains("enter select")
    );
    Ok(())
}

#[test]
fn discard_footer_uses_remapped_shortcuts() -> AppResult<()> {
    // Given: navigation and confirmation keys remapped away from the defaults.
    let config = Config::parse(
        "keybindings { global { down \"down\" \"alt-i\"; up \"up\" \"alt-p\"; }; input { accept \"alt-enter\"; cancel \"alt-escape\"; }; }",
    )?;
    let mut state = state_with("main.rs", 1);
    state.begin_discard(ChangeTarget::File("main.rs".into()));
    // When: rendering the discard footer.
    let buffer = render(&state, &config, 100, 20)?;
    // Then: labels reflect the effective bindings, with alternatives separated from actions.
    let text = text(&buffer);
    assert!(text.contains("down|alt-i/up|alt-p move · alt-enter select"));
    assert!(text.contains("alt-esc cancel"));
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
    assert!(text.contains("File: \"long/directory"));
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

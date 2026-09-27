use super::*;

fn recording_editor(fixture: &Fixture) -> AppResult<String> {
    use std::os::unix::fs::PermissionsExt;
    let path = fixture.root.join("nvim");
    fs::write(&path, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(format!("'{}'", path.display()))
}

fn prepare(
    fixture: &Fixture,
    visual: Option<&str>,
    editor: Option<&str>,
    line: Option<usize>,
) -> AppResult<EditorCommand> {
    Ok(EditorCommand::prepare(
        &fixture.root,
        Some(EditorTarget {
            path: Path::new("tracked.txt"),
            line,
        }),
        visual.map(Into::into),
        editor.map(Into::into),
    )?)
}

#[test]
fn nvim_receives_the_line_before_the_argument_delimiter() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    // When
    let prepared = prepare(&fixture, Some("nvim --clean --"), None, Some(17))?;
    // Then
    assert_eq!(prepared.command.get_program(), "nvim");
    assert_eq!(
        prepared.command.get_args().collect::<Vec<_>>(),
        [
            OsStr::new("--clean"),
            OsStr::new("+17"),
            OsStr::new("--"),
            fixture.root.join("tracked.txt").as_os_str()
        ]
    );
    assert_eq!(
        prepared.command.get_current_dir(),
        Some(fixture.root.as_path())
    );
    Ok(())
}

#[test]
fn helix_preserves_literal_arguments_when_adding_a_line() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    // When
    let prepared = prepare(
        &fixture,
        Some("hx --config 'literal $value; text'"),
        None,
        Some(8),
    )?;
    // Then
    assert_eq!(prepared.command.get_program(), "hx");
    assert_eq!(
        prepared.command.get_args().collect::<Vec<_>>(),
        [
            OsStr::new("--config"),
            OsStr::new("literal $value; text"),
            OsStr::new("+8"),
            fixture.root.join("tracked.txt").as_os_str()
        ]
    );
    Ok(())
}

#[test]
fn vscode_preserves_wait_and_receives_a_goto_destination() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    let destination = format!("{}:9:1", fixture.root.join("tracked.txt").display());
    // When
    let prepared = prepare(&fixture, Some("code --wait"), None, Some(9))?;
    // Then
    assert_eq!(prepared.command.get_program(), "code");
    assert_eq!(
        prepared.command.get_args().collect::<Vec<_>>(),
        ["--wait", "--goto", &destination]
    );
    Ok(())
}

#[test]
fn historical_vscode_waits_before_the_argument_delimiter() -> AppResult<()> {
    // Given: a historical file opened through a literal VS Code command.
    let fixture = Fixture::new("")?;
    let target = EditorTarget {
        path: Path::new("tracked.txt"),
        line: None,
    };
    // When: preparing the temporary copy for editing.
    let prepared = EditorCommand::prepare_with_wait(
        &fixture.root,
        Some(target),
        Some("CODE.EXE --reuse-window --".into()),
        None,
        true,
    )?;
    // Then: the editor keeps the temporary file alive until it closes.
    assert_eq!(
        prepared.command.get_args().collect::<Vec<_>>(),
        [
            OsStr::new("--reuse-window"),
            OsStr::new("--wait"),
            OsStr::new("--"),
            fixture.root.join("tracked.txt").as_os_str(),
        ]
    );
    Ok(())
}

#[test]
fn historical_vscode_positions_and_waits_before_the_argument_delimiter() -> AppResult<()> {
    // Given: a historical file with a source position.
    let fixture = Fixture::new("")?;
    let target = EditorTarget {
        path: Path::new("tracked.txt"),
        line: Some(9),
    };
    let destination = format!("{}:9:1", fixture.root.join("tracked.txt").display());
    // When: preparing its editor command.
    let prepared = EditorCommand::prepare_with_wait(
        &fixture.root,
        Some(target),
        Some("code --reuse-window --".into()),
        None,
        true,
    )?;
    // Then: both options are interpreted as options, not file names.
    assert_eq!(
        prepared.command.get_args().collect::<Vec<_>>(),
        ["--reuse-window", "--goto", "--wait", "--", &destination]
    );
    Ok(())
}

#[test]
fn normal_open_does_not_position_a_supported_editor() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    // When
    let prepared = prepare(&fixture, Some("nvim"), None, None)?;
    // Then
    assert_eq!(
        prepared.command.get_args().collect::<Vec<_>>(),
        [fixture.root.join("tracked.txt").as_os_str()]
    );
    Ok(())
}

#[test]
fn a_shell_wrapper_is_passed_the_file_without_positioning() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    let command = format!("env {}", recording_editor(&fixture)?);
    // When
    let mut prepared = prepare(&fixture, Some(&command), None, Some(9))?;
    let output = prepared.command.stdout(Stdio::piped()).output()?;
    // Then
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        format!("{}\n", fixture.root.join("tracked.txt").display())
    );
    Ok(())
}

#[test]
fn shell_expansion_keeps_normal_opening_and_a_literal_path() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    let command = format!(
        "{} \"$DIFT_TEST_EDITOR_ARGUMENT\"",
        recording_editor(&fixture)?
    );
    let mut prepared = prepare(&fixture, Some(&command), None, Some(9))?;
    prepared
        .command
        .env("DIFT_TEST_EDITOR_ARGUMENT", "literal $value; text");
    // When
    let output = prepared.command.stdout(Stdio::piped()).output()?;
    // Then
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        format!(
            "literal $value; text\n{}\n",
            fixture.root.join("tracked.txt").display()
        )
    );
    Ok(())
}

#[test]
fn blank_visual_falls_back_to_editor() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    // When
    let prepared = prepare(&fixture, Some(" \t "), Some("nvim"), None)?;
    // Then
    assert_eq!(prepared.variable, "EDITOR");
    assert_eq!(prepared.command.get_program(), "nvim");
    Ok(())
}

#[test]
fn unset_visual_falls_back_to_editor() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    // When
    let prepared = prepare(&fixture, None, Some("nvim"), None)?;
    // Then
    assert_eq!(prepared.variable, "EDITOR");
    assert_eq!(prepared.command.get_program(), "nvim");
    Ok(())
}

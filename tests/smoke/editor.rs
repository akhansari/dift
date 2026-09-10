//! Real editor handoff checks; command-construction variants live in Rust unit tests.
use super::{Fixture, Result};
use rustix::termios::{LocalModes, tcgetattr};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
};

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn fake_editor(fixture: &Fixture) -> Result<String> {
    let binary = env::current_exe()?;
    let script = fixture.path(".git/fake editor");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor dift_editor_target do :; done\nexport DIFT_TEST_EDITOR_TARGET=\"$dift_editor_target\"\nexec {} --ignored --exact editor::editor_process --nocapture\n",
            quote(binary.to_str().ok_or("Invalid test binary path")?)
        ),
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    Ok(quote(script.to_str().ok_or("Invalid editor path")?))
}

fn records(fixture: &Fixture) -> Result<Vec<Value>> {
    Ok(serde_json::from_slice(&fs::read(
        fixture.path(".git/editor-record.json"),
    )?)?)
}

fn first_record(fixture: &Fixture) -> Result<Value> {
    records(fixture)?
        .into_iter()
        .next()
        .ok_or_else(|| "Missing editor record".into())
}

fn raw(session: &super::terminal::Session) -> Result<bool> {
    Ok(!session.termios()?.local_modes.contains(LocalModes::ICANON))
}

/// Invoked only as a child executable by the tiny shell launcher above. The
/// actual editor behavior is Rust; libtest provides an already-built helper.
#[test]
#[ignore = "subprocess helper, not a smoke scenario"]
fn editor_process() -> Result {
    let target = env::var("DIFT_TEST_EDITOR_TARGET")?;
    let path = env::current_dir()?.join(".git/editor-record.json");
    let mut records: Vec<Value> = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let modes = tcgetattr(io::stdin())?.local_modes;
    records.push(json!({"target": target, "cwd": env::current_dir()?, "contents": fs::read_to_string(&target)?,
        "canonical": modes.contains(LocalModes::ICANON), "echo": modes.contains(LocalModes::ECHO)}));
    fs::write(path, serde_json::to_vec(&records)?)?;
    print!("\x1b[H\x1b[2JEDITOR_READY {}\n", records.len());
    io::stdout().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    if response.trim() == "save" {
        fs::write(target, "EDITOR_SAVED_CONTENT\n")?;
    }
    if env::var_os("DIFT_TEST_EDITOR_FAIL").is_some() {
        return Err("Requested unsuccessful editor exit".into());
    }
    Ok(())
}

#[test]
fn editor_save_restores_terminal_and_refreshes_selected_file() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let editor = fake_editor(&fixture)?;
    let mut session = fixture.start_with(
        &fixture.path("src"),
        &[],
        &[
            ("VISUAL", Some(&editor)),
            ("EDITOR", Some("missing-fallback-editor")),
        ],
    )?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    session.send("o")?;
    session.wait("EDITOR_READY 1")?;
    let record = first_record(&fixture)?;
    session.send("save\n")?;
    // Then
    session.wait("EDITOR_SAVED_CONTENT")?;
    session.wait("Watching")?;
    session.select("main.rs")?;
    assert_eq!(
        record["target"],
        fixture.path("src/main.rs").to_string_lossy().as_ref()
    );
    assert_eq!(record["cwd"], fixture.root().to_string_lossy().as_ref());
    assert!(
        record["contents"]
            .as_str()
            .is_some_and(|text| text.contains("\"working\""))
    );
    assert_eq!(record["canonical"], true);
    assert_eq!(record["echo"], true);
    assert!(raw(&session)?);
    session.quit()
}

#[test]
fn missing_editor_configuration_keeps_terminal_usable() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    session.send("o")?;
    // Then
    session.wait("Set VISUAL or EDITOR to open files")?;
    assert!(raw(&session)?);
    session.quit()
}

#[test]
fn editor_spawn_failure_restores_view_without_fallback() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let fallback = fake_editor(&fixture)?;
    let missing = fixture.path(".git/missing-editor");
    let mut session = fixture.start_with(
        &fixture.path("src"),
        &[],
        &[
            ("VISUAL", Some(missing.to_str().ok_or("Invalid path")?)),
            ("EDITOR", Some(&fallback)),
        ],
    )?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    session.send("o")?;
    // Then
    session.wait("Cannot start $VISUAL")?;
    assert!(!fixture.path(".git/editor-record.json").exists());
    assert!(raw(&session)?);
    session.quit()
}

#[test]
fn unsuccessful_editor_exit_restores_view() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let editor = fake_editor(&fixture)?;
    let mut session = fixture.start_with(
        &fixture.path("src"),
        &[],
        &[
            ("VISUAL", Some(&editor)),
            ("DIFT_TEST_EDITOR_FAIL", Some("1")),
        ],
    )?;
    session.send("no")?;
    session.wait("EDITOR_READY 1")?;
    // When
    session.send("close\n")?;
    // Then
    session.wait("$VISUAL exited unsuccessfully")?;
    session.select("main.rs")?;
    assert!(raw(&session)?);
    session.quit()
}

#[test]
fn editor_receives_literal_path_with_shell_metacharacters() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let path = "src/-odd '$(touch INJECTED)' ; name.rs";
    fixture.write(path, "SPECIAL_PATH_CONTENT\n")?;
    let editor = fake_editor(&fixture)?;
    let mut session =
        fixture.start_with(&fixture.path("src"), &[], &[("VISUAL", Some(&editor))])?;
    session.send("f-odd\rn")?;
    session.wait("SPECIAL_PATH_CONTENT")?;
    // When
    session.send("o")?;
    // Then
    session.wait("EDITOR_READY 1")?;
    assert_eq!(
        first_record(&fixture)?["target"],
        fixture.path(path).to_string_lossy().as_ref()
    );
    assert!(!fixture.path("INJECTED").exists());
    session.send("close\n")?;
    session.wait("Watching")?;
    session.quit()
}

#[test]
fn repeated_editor_handoffs_survive_resize() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let editor = fake_editor(&fixture)?;
    let mut session =
        fixture.start_with(&fixture.path("src"), &[], &[("VISUAL", Some(&editor))])?;
    session.send("no")?;
    session.wait("EDITOR_READY 1")?;
    session.send("close\n")?;
    session.wait("Watching")?;
    // When
    session.send("o")?;
    session.wait("EDITOR_READY 2")?;
    session.resize(80, 25)?;
    session.send("close\n")?;
    // Then
    session.wait("Watching")?;
    session.select("main.rs")?;
    assert_eq!(records(&fixture)?.len(), 2);
    session.quit()
}

#[test]
fn comparison_editor_opens_working_contents_without_changing_snapshot() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.git(&["commit", "-m", "staged version"])?;
    let editor = fake_editor(&fixture)?;
    let mut session = fixture.start_with(
        &fixture.path("src"),
        &["diff", "HEAD~1", "HEAD"],
        &[("VISUAL", Some(&editor))],
    )?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    session.send("o")?;
    session.wait("EDITOR_READY 1")?;
    let record = first_record(&fixture)?;
    session.send("save\n")?;
    // Then
    session.wait("Snapshot")?;
    assert!(
        record["contents"]
            .as_str()
            .is_some_and(|text| text.contains("\"working\""))
    );
    assert!(!session.screen.contains("EDITOR_SAVED_CONTENT"));
    session.quit()
}

#[test]
fn deleted_file_shows_notice_without_starting_editor() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fs::remove_file(fixture.path("src/main.rs"))?;
    let editor = fake_editor(&fixture)?;
    let mut session =
        fixture.start_with(&fixture.path("src"), &[], &[("VISUAL", Some(&editor))])?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    session.send("o")?;
    // Then
    session.wait("Cannot open")?;
    assert!(!fixture.path(".git/editor-record.json").exists());
    session.quit()
}

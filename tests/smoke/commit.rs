use super::{Fixture, Result};
use rustix::termios::LocalModes;
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

fn repository() -> Result<Fixture> {
    let fixture = Fixture::new()?;
    fixture.git(&["config", "user.name", "Commit Test"])?;
    fixture.git(&["config", "user.email", "commit@example.invalid"])?;
    fixture.git(&["config", "commit.gpgsign", "false"])?;
    Ok(fixture)
}

fn hook(fixture: &Fixture, name: &str, script: &str) -> Result {
    let path = format!(".git/hooks/{name}");
    fixture.write(&path, format!("#!/bin/sh\n{script}\n"))?;
    fs::set_permissions(fixture.path(&path), fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn close_dialog(session: &mut super::terminal::Session) -> Result {
    session.send("\x1b")?;
    session.wait_for("commit dialog closed", Duration::from_secs(4), |screen| {
        !screen.contains(" Commit ")
    })
}

fn open_dialog(session: &mut super::terminal::Session, text: &str) -> Result {
    session.send("c")?;
    session.wait(" Commit ")?;
    session.send(text)
}

fn wait_committed(session: &mut super::terminal::Session) -> Result {
    session.wait_for(
        "commit completed and dift refreshed",
        Duration::from_secs(8),
        |screen| {
            screen.alternate_screen() && screen.contains("Watching") && !screen.contains(" Commit ")
        },
    )
}

#[test]
fn commit_records_only_staged_contents_with_subject_and_body() -> Result {
    // Given
    let fixture = repository()?;
    let mut session = fixture.start()?;
    // When
    open_dialog(
        &mut session,
        "Literal ' $(touch INJECTED)\rFirst body line\rSecond body line\x13",
    )?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%B"])?,
        "Literal ' $(touch INJECTED)\n\nFirst body line\nSecond body line"
    );
    assert_eq!(
        fixture.git(&["show", "HEAD:src/main.rs"])?,
        "fn main() { println!(\"staged\"); }"
    );
    assert!(fixture.git(&["diff"])?.contains("working"));
    assert!(
        fixture
            .git(&["status", "--porcelain"])?
            .contains("?? notes.txt")
    );
    assert!(fixture.git(&["diff", "--cached"])?.is_empty());
    assert!(!fixture.path("INJECTED").exists());
    assert!(!session.screen.contains(" Commit "));
    assert!(!session.output_contains(b"\x1b[?1049l"));
    assert!(!session.output_contains(b"Committed "));
    assert!(!session.termios()?.local_modes.contains(LocalModes::ICANON));
    session.quit()
}

#[test]
fn reopening_a_cancelled_dialog_restores_subject_and_body() -> Result {
    // Given
    let fixture = repository()?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "DRAFT_SUBJECT\rDRAFT_BODY")?;
    session.wait("DRAFT_BODY")?;
    close_dialog(&mut session)?;
    // When
    open_dialog(&mut session, "")?;
    // Then
    session.wait("DRAFT_SUBJECT")?;
    session.wait("DRAFT_BODY")?;
    assert_eq!(fixture.git(&["log", "-1", "--format=%s"])?, "initial");
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn empty_subject_keeps_the_dialog_open_without_running_git() -> Result {
    // Given
    let fixture = repository()?;
    let mut session = fixture.start()?;
    // When
    open_dialog(&mut session, "   \rBody without subject\x13")?;
    // Then
    session.wait("Enter a commit subject")?;
    assert_eq!(fixture.git(&["log", "-1", "--format=%s"])?, "initial");
    assert!(!session.termios()?.local_modes.contains(LocalModes::ICANON));
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn failed_hook_keeps_output_visible_then_restores_the_draft() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "pre-commit",
        "echo HOOK_REJECTED_MESSAGE >&2\nexit 1",
    )?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "RETAINED_SUBJECT\rRETAINED_BODY\x13")?;
    session.wait("Press Enter to return")?;
    assert!(session.screen.contains("HOOK_REJECTED_MESSAGE"));
    assert!(!session.termios()?.local_modes.contains(LocalModes::ICANON));
    // When
    session.send("\r")?;
    // Then
    session.wait(" Commit ")?;
    session.wait("RETAINED_SUBJECT")?;
    session.wait("RETAINED_BODY")?;
    assert!(!session.termios()?.local_modes.contains(LocalModes::ICANON));
    assert_eq!(fixture.git(&["log", "-1", "--format=%s"])?, "initial");
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn retry_after_a_hook_failure_uses_the_retained_draft_and_clears_it_on_success() -> Result {
    // Given
    let fixture = repository()?;
    hook(&fixture, "pre-commit", "exit 1")?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "RETRY_SUBJECT\rRETRY_BODY\x13")?;
    session.wait("Press Enter to return")?;
    session.send("\r")?;
    session.wait(" Commit ")?;
    fs::remove_file(fixture.path(".git/hooks/pre-commit"))?;
    // When
    session.send("\x13")?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%B"])?,
        "RETRY_SUBJECT\n\nRETRY_BODY"
    );
    open_dialog(&mut session, "")?;
    session.wait(" Commit ")?;
    assert!(!session.screen.contains("RETRY_SUBJECT"));
    assert!(!session.screen.contains("RETRY_BODY"));
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn interactive_hook_receives_the_terminal_and_restores_dift_after_resize() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "pre-commit",
        "echo run >> .git/hook-runs\ntest -t 0 < \"$GPG_TTY\" || exit 1\necho HOOK_READY > /dev/tty\nread dift_hook_answer < /dev/tty\nstty size < /dev/tty > .git/hook-size\ntest \"$dift_hook_answer\" = proceed",
    )?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "Interactive hook\x13")?;
    session.wait("HOOK_READY")?;
    assert!(
        !session
            .termios()?
            .local_modes
            .intersects(LocalModes::ICANON | LocalModes::ECHO)
    );
    // When
    session.resize(80, 25)?;
    session.send("proceed\n")?;
    // Then
    wait_committed(&mut session)?;
    session.wait("Watching")?;
    assert!(!session.termios()?.local_modes.contains(LocalModes::ICANON));
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%s"])?,
        "Interactive hook"
    );
    assert_eq!(fs::read_to_string(fixture.path(".git/hook-runs"))?, "run\n");
    assert_eq!(
        fs::read_to_string(fixture.path(".git/hook-size"))?,
        "25 80\n"
    );
    session.quit()
}

#[test]
fn slow_commit_reveals_output_after_one_second_even_with_continuous_output() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "pre-commit",
        "echo CAPTURED_EARLY\nwhile [ ! -f .git/release-hook ]; do echo STILL_RUNNING; sleep 0.02; done",
    )?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "Slow commit\x13")?;
    session.wait("Committing…")?;
    // When
    session.observe(Duration::from_millis(200))?;
    // Then
    assert!(session.screen.alternate_screen());
    assert!(!session.output_contains(b"CAPTURED_EARLY"));
    session.wait("STILL_RUNNING")?;
    assert!(!session.screen.alternate_screen());
    assert!(session.output_contains(b"CAPTURED_EARLY"));
    fixture.write(".git/release-hook", "")?;
    wait_committed(&mut session)?;
    assert_eq!(fixture.git(&["log", "-1", "--format=%s"])?, "Slow commit");
    session.quit()
}

#[test]
fn busy_dialog_ignores_editing_and_duplicate_submission_and_handles_resize() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "pre-commit",
        "echo run >> .git/hook-runs\nwhile [ ! -f .git/release-hook ]; do sleep 0.01; done",
    )?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "Original subject\x13")?;
    session.wait("Committing…")?;
    // When
    session.send("\x13\x1bqchanged\x1b[200~pasted\x1b[201~")?;
    session.resize(80, 25)?;
    session.observe(Duration::from_millis(100))?;
    fixture.write(".git/release-hook", "")?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%s"])?,
        "Original subject"
    );
    assert_eq!(fs::read_to_string(fixture.path(".git/hook-runs"))?, "run\n");
    assert!(!session.output_contains(b"\x1b[?1049l"));
    session.quit()
}

#[test]
fn revealed_hook_failure_waits_for_enter_and_restores_terminal_modes() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "pre-commit",
        "echo INTERACTIVE_FAILURE_READY > /dev/tty\nread answer < /dev/tty\nprintf '\033[?1000h\033[?2004h\033[?1h\033[?25l' > /dev/tty\necho FINAL_DIAGNOSTIC >&2\nexit 1",
    )?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "Failed interactive draft\x13")?;
    session.wait("INTERACTIVE_FAILURE_READY")?;
    // When
    session.send("reject\r")?;
    // Then
    session.wait("Press Enter to return")?;
    assert!(session.screen.contains("FINAL_DIAGNOSTIC"));
    assert!(!session.screen.alternate_screen());
    session.send("\r")?;
    session.wait(" Commit ")?;
    session.wait("Failed interactive draft")?;
    assert!(session.screen.alternate_screen());
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn interactive_signer_uses_the_child_tty_and_preserves_the_draft_on_failure() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "test-signer",
        "echo SIGNER_READY > \"$GPG_TTY\"\nread answer < \"$GPG_TTY\"\necho SIGNER_REJECTED >&2\nexit 1",
    )?;
    fixture.git(&["config", "gpg.program", ".git/hooks/test-signer"])?;
    fixture.git(&["config", "commit.gpgsign", "true"])?;
    let mut session = fixture.start_with(
        fixture.root(),
        &[],
        &[("GPG_TTY", Some("/invalid/parent-tty"))],
    )?;
    open_dialog(&mut session, "Signing draft\x13")?;
    session.wait("SIGNER_READY")?;
    // When
    session.send("reject\r")?;
    // Then
    session.wait("Press Enter to return")?;
    assert!(session.screen.contains("SIGNER_REJECTED"));
    session.send("\r")?;
    session.wait(" Commit ")?;
    assert!(session.screen.contains("Signing draft"));
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn missing_git_shows_a_spawn_error_and_restores_the_draft() -> Result {
    // Given
    let fixture = repository()?;
    let mut session = fixture.spawn(
        fixture.root(),
        &[],
        &[("PATH", Some("/nonexistent/dift-test-bin"))],
    )?;
    session.wait("Error: Git I/O")?;
    // When
    open_dialog(&mut session, "Spawn failure draft\x13")?;
    // Then
    session.wait("Press Enter to return")?;
    assert!(session.screen.contains("Cannot run git commit"));
    session.send("\r")?;
    session.wait(" Commit ")?;
    assert!(session.screen.contains("Spawn failure draft"));
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn empty_index_allows_drafting_but_git_rejects_the_commit() -> Result {
    // Given
    let fixture = repository()?;
    fixture.git(&["restore", "--staged", "."])?;
    let mut session = fixture.start()?;
    // When
    open_dialog(&mut session, "DRAFT_WITH_EMPTY_INDEX\x13")?;
    // Then
    session.wait("Press Enter to return")?;
    assert!(session.screen.contains("no changes added to commit"));
    session.send("\r")?;
    session.wait("DRAFT_WITH_EMPTY_INDEX")?;
    close_dialog(&mut session)?;
    session.quit()
}

#[test]
fn commit_uses_external_staging_changes_made_while_composing() -> Result {
    // Given
    let fixture = repository()?;
    let mut session = fixture.start()?;
    open_dialog(&mut session, "Current index")?;
    session.wait("Current index")?;
    fixture.git(&["add", "notes.txt"])?;
    // When
    session.send("\x13")?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(fixture.git(&["show", "HEAD:notes.txt"])?, "untracked note");
    session.quit()
}

#[test]
fn comparison_mode_rejects_committing() -> Result {
    // Given
    let fixture = repository()?;
    let mut session = fixture.start_with(fixture.root(), &["diff", "HEAD", "HEAD"], &[])?;
    // When
    session.send("c")?;
    // Then
    session.wait("Committing is unavailable in comparison mode")?;
    assert!(!session.screen.contains(" Commit "));
    assert_eq!(fixture.git(&["log", "-1", "--format=%s"])?, "initial");
    session.quit()
}

#[test]
fn commit_works_on_an_unborn_branch() -> Result {
    // Given
    let fixture = repository()?;
    fixture.git(&["update-ref", "-d", "HEAD"])?;
    let mut session = fixture.start()?;
    // When
    open_dialog(&mut session, "First commit\x13")?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(fixture.git(&["rev-list", "--count", "HEAD"])?, "1");
    assert_eq!(fixture.git(&["log", "-1", "--format=%s"])?, "First commit");
    session.quit()
}

#[test]
fn commit_honors_message_hooks() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "commit-msg",
        "printf 'Rewritten by hook\\n' > \"$1\"",
    )?;
    let mut session = fixture.start()?;
    // When
    open_dialog(&mut session, "Original subject\x13")?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%s"])?,
        "Rewritten by hook"
    );
    session.quit()
}

#[test]
fn commit_works_with_a_detached_head() -> Result {
    // Given
    let fixture = repository()?;
    fixture.git(&["checkout", "--detach"])?;
    let mut session = fixture.start()?;
    // When
    open_dialog(&mut session, "Detached commit\x13")?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(fixture.git(&["rev-list", "--count", "HEAD"])?, "2");
    assert_eq!(fixture.git(&["branch", "--show-current"])?, "");
    session.quit()
}

#[test]
fn commit_can_complete_a_merge() -> Result {
    // Given
    let fixture = repository()?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "clean base"])?;
    fixture.git(&["checkout", "-b", "other"])?;
    fixture.write("other.txt", "merged contents\n")?;
    fixture.git(&["add", "other.txt"])?;
    fixture.git(&["commit", "-m", "other branch"])?;
    fixture.git(&["checkout", "main"])?;
    fixture.git(&["merge", "--no-ff", "--no-commit", "other"])?;
    let mut session = fixture.start()?;
    // When
    open_dialog(&mut session, "Complete merge\x13")?;
    // Then
    wait_committed(&mut session)?;
    assert_eq!(
        fixture.git(&["log", "-1", "--format=%s"])?,
        "Complete merge"
    );
    assert_eq!(fixture.git(&["show", "HEAD:other.txt"])?, "merged contents");
    assert!(!fixture.path(".git/MERGE_HEAD").exists());
    session.quit()
}

#[test]
fn configured_signing_is_attempted_and_its_failure_preserves_the_draft() -> Result {
    // Given
    let fixture = repository()?;
    hook(
        &fixture,
        "test-signer",
        "echo SIGNING_WAS_REQUESTED >&2\nexit 1",
    )?;
    fixture.git(&["config", "gpg.program", ".git/hooks/test-signer"])?;
    fixture.git(&["config", "commit.gpgsign", "true"])?;
    let mut session = fixture.start()?;
    // When
    open_dialog(&mut session, "SIGNED_DRAFT\x13")?;
    // Then
    session.wait("Press Enter to return")?;
    assert!(session.screen.contains("SIGNING_WAS_REQUESTED"));
    assert_eq!(fixture.git(&["log", "-1", "--format=%s"])?, "initial");
    session.send("\r")?;
    session.wait("SIGNED_DRAFT")?;
    close_dialog(&mut session)?;
    session.quit()
}

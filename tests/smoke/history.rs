use crate::{fixtures::Fixture, terminal::Result};
use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

#[test]
fn selecting_two_commits_compares_them_without_losing_the_worktree_view() -> Result {
    // Given: a branch with two commits and uncommitted work.
    let fixture = Fixture::new()?;
    fixture.write("historical.txt", "historical text\n")?;
    fixture.git(&["add", "historical.txt"])?;
    fixture.git(&["commit", "-m", "historical change"])?;
    let newest = fixture.git(&["rev-parse", "--short=12", "HEAD"])?;
    let parent = fixture.git(&["rev-parse", "--short=12", "HEAD^"])?;
    let mut session = fixture.start()?;
    // When: marking the newest commit, comparing it to its parent, then returning.
    session.send("t")?;
    session.wait("History · main")?;
    session.wait("historical change")?;
    session.send("bjm")?;
    // Then: the direction is visible and the original view survives.
    session.wait(&format!("Compare {newest} → {parent}"))?;
    assert!(
        session
            .screen
            .row(session.screen.rows() - 2)
            .contains(&format!("Compare {newest} → {parent}"))
    );
    session.send("x")?;
    session.wait(&format!("Compare {parent} → {newest}"))?;
    session.send("\x1b")?;
    session.wait("History · main")?;
    session.send("t")?;
    session.wait_for("restored worktree diff", Duration::from_secs(4), |screen| {
        screen.contains("Watching") && screen.contains("working") && !screen.contains("History ·")
    })?;
    session.quit()
}

#[test]
fn commit_details_use_the_status_row_instead_of_a_header() -> Result {
    // Given: a historical commit with full metadata available through the details shortcut.
    let fixture = Fixture::new()?;
    let id = fixture.git(&["rev-parse", "--short=12", "HEAD"])?;
    let mut session = fixture.start()?;
    session.send("t")?;
    session.wait("initial (HEAD)")?;
    // When: opening its Changes view.
    session.send("\r")?;
    session.wait_for(
        "commit title in status row",
        Duration::from_secs(4),
        |screen| {
            screen
                .row(screen.rows() - 2)
                .contains(&format!("{id} · CHANGES · initial"))
        },
    )?;
    // Then: the diff starts on the first row, with metadata hidden until requested.
    assert!(session.screen.row(0).contains("All changes"));
    assert!(!session.screen.contains("Committer:"));
    session.send("i")?;
    session.wait("Committer:")?;
    assert!(session.screen.row(0).contains("All changes"));
    session.quit()
}

#[test]
fn commit_details_show_only_history_shortcuts_in_the_footer() -> Result {
    // Given: a commit opened from history in a wide terminal.
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("t")?;
    session.wait("initial (HEAD)")?;
    // When: showing the commit's Changes view.
    session.send("\r")?;
    session.wait("v timecapsule")?;
    // Then: no shortcuts from the worktree footer remain after the history hints.
    let footer = session.screen.row(session.screen.rows() - 1);
    assert!(footer.trim_end().ends_with("q quit"), "{footer}");
    assert_eq!(footer.matches("help").count(), 1, "{footer}");
    assert!(!footer.contains("/ search"), "{footer}");
    session.quit()
}

#[test]
fn commit_help_scrolls_with_the_keyboard() -> Result {
    // Given: the help overlay in a historical commit viewer.
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("t")?;
    session.wait("initial (HEAD)")?;
    session.send("\r?")?;
    session.wait("GENERAL")?;
    let first_page = session.screen.text();
    // When: paging down without any mouse interaction.
    session.send("\x1b[6~")?;
    // Then: the help content advances.
    session.wait_for("historical help scroll", Duration::from_secs(4), |screen| {
        screen.text() != first_page
    })?;
    session.quit()
}

#[test]
fn long_commit_details_scroll_inside_the_available_area() -> Result {
    // Given: a commit message longer than the available popup height.
    let fixture = Fixture::new()?;
    let message = format!("{}last history line", "earlier history line\n".repeat(60));
    fixture.git(&[
        "commit",
        "--allow-empty",
        "-m",
        "long details",
        "-m",
        &message,
    ])?;
    let mut session = fixture.start()?;
    session.send("t")?;
    session.wait("long details")?;
    session.send("\r")?;
    // When: viewing and paging through the full commit details.
    session.send("i")?;
    session.wait("Commit details")?;
    session.wait("esc close")?;
    assert!(session.screen.row(0).contains("Commit details"));
    assert!(
        session
            .screen
            .row(session.screen.rows() - 2)
            .contains("close")
    );
    session.send(&"\x1b[6~".repeat(6))?;
    // Then: the final line becomes readable without scrolling the footer away.
    session.wait("last history line")?;
    assert!(
        session
            .screen
            .row(session.screen.rows() - 2)
            .contains("close")
    );
    session.quit()
}

#[test]
fn commit_files_display_historical_content_instead_of_the_worktree() -> Result {
    // Given: the checked-out version differs from the committed file.
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When: opening the root commit's complete file tree.
    session.send("t")?;
    session.wait("History · main")?;
    session.wait("initial (HEAD)")?;
    session.send("\r")?;
    session.wait("CHANGES")?;
    session.send("v")?;
    session.wait("FILES")?;
    session.wait("▸ src")?;
    assert!(session.screen.row(0).contains("▸ src"));
    assert!(!session.screen.text().to_lowercase().contains("changes"));
    session.send("\x1b[<0;2;1M\x1b[<0;2;1m")?;
    session.wait("▾ src")?;
    session.send("Fmain.rs\r")?;
    // Then: the historical source is shown without the staged or working edits.
    session.wait("original")?;
    session.wait("TIMECAPSULE")?;
    assert!(!session.screen.contains("println!(\"working\")"));
    session.send("\x1b")?;
    session.wait("History · main")?;
    session.quit()
}

#[test]
fn a_top_level_historical_file_opens_without_an_extra_selection() -> Result {
    // Given: a commit with only a root-level file, distinct from the worktree version.
    let fixture = Fixture::new()?;
    fixture.git(&["rm", "-f", "src/main.rs"])?;
    fixture.write("README.md", "committed root content\n")?;
    fixture.git(&["add", "README.md"])?;
    fixture.git(&["commit", "-m", "root file only"])?;
    fixture.write("README.md", "uncommitted root content\n")?;
    let mut session = fixture.start_with(fixture.root(), &[], &[])?;
    // When: switching from that commit's Changes view to Files.
    session.send("t")?;
    session.wait("root file only")?;
    session.send("\r")?;
    session.wait("CHANGES")?;
    session.send("v")?;
    // Then: the first visible file is selected and its committed bytes are loaded.
    session.wait("committed root content")?;
    assert!(!session.screen.contains("All changes"));
    assert!(!session.screen.contains("uncommitted root content"));
    session.quit()
}

#[test]
fn filtering_historical_files_loads_the_new_selection_after_debounce() -> Result {
    // Given: two committed root files with different contents.
    let fixture = Fixture::new()?;
    fixture.git(&["rm", "-f", "src/main.rs"])?;
    fixture.write("first.txt", "first historical content\n")?;
    fixture.write("second.txt", "second historical content\n")?;
    fixture.git(&["add", "first.txt", "second.txt"])?;
    fixture.git(&["commit", "-m", "two files"])?;
    let mut session = fixture.start_with(fixture.root(), &[], &[])?;
    session.send("t")?;
    session.wait("two files")?;
    session.send("\r")?;
    session.wait("CHANGES")?;
    session.send("v")?;
    session.wait("first historical content")?;
    // When: narrowing the file tree while the filter prompt is still open.
    session.send("fsecond.txt")?;
    // Then: the newly selected file's bytes are loaded without accepting the prompt.
    session.wait("second historical content")?;
    assert!(!session.screen.contains("first historical content"));
    session.send("\x1b")?;
    session.wait("first historical content")?;
    session.quit()
}

#[test]
fn commit_info_modal_blocks_mouse_selection_of_files_behind_it() -> Result {
    // Given: two committed files with the first one selected in the file view.
    let fixture = Fixture::new()?;
    fixture.git(&["rm", "-f", "src/main.rs"])?;
    fixture.write("first.txt", "first historical content\n")?;
    fixture.write("second.txt", "second historical content\n")?;
    fixture.git(&["add", "first.txt", "second.txt"])?;
    fixture.git(&["commit", "-m", "two files"])?;
    let mut session = fixture.start_with(fixture.root(), &[], &[])?;
    session.send("t")?;
    session.wait("two files")?;
    session.send("\r")?;
    session.wait("CHANGES")?;
    session.send("v")?;
    session.wait("first historical content")?;
    session.send("i")?;
    session.wait("Commit details")?;
    // When: clicking the second tree row while the details overlay is open.
    session.send("\x1b[<0;5;2M\x1b[<0;5;2m")?;
    session.send("\x1b")?;
    // Then: the overlay did not allow the file selection to change behind it.
    session.wait_for(
        "original file selection",
        Duration::from_secs(4),
        |screen| screen.contains("first historical content") && !screen.contains("Commit details"),
    )?;
    assert!(!session.screen.contains("second historical content"));
    session.quit()
}

#[test]
fn dragging_historical_source_copies_it_to_the_clipboard() -> Result {
    // Given: a committed file displayed as full historical source.
    let fixture = Fixture::new()?;
    fixture.git(&["rm", "-f", "src/main.rs"])?;
    fixture.write("history.txt", "historical selection line\n")?;
    fixture.git(&["add", "history.txt"])?;
    fixture.git(&["commit", "-m", "copyable history"])?;
    let mut session = fixture.start_with(fixture.root(), &[], &[])?;
    session.send("t")?;
    session.wait("copyable history")?;
    session.send("\r")?;
    session.wait("CHANGES")?;
    session.send("v")?;
    session.wait("historical selection line")?;
    let (x, y) = session.screen.locate("historical selection line")?;
    // When: dragging over the historical source and releasing.
    session.send(&format!("\x1b[<0;{};{}M", x + 1, y + 1))?;
    session.send(&format!("\x1b[<32;{};{}M", x + 25, y + 1))?;
    session.send(&format!("\x1b[<0;{};{}m", x + 25, y + 1))?;
    // Then: the source text reaches the terminal clipboard.
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while session.clipboard.is_empty() && std::time::Instant::now() < deadline {
        session.pump(Duration::from_millis(10))?;
    }
    assert_eq!(session.clipboard, ["historical selection line"]);
    session.quit()
}

#[test]
fn comparison_launch_can_browse_history_and_restore_its_original_diff() -> Result {
    // Given: a comparison launched directly from the CLI.
    let fixture = Fixture::new()?;
    let mut session = fixture.start_with(&fixture.path("src"), &["diff", "HEAD", "HEAD"], &[])?;
    // When: entering history and returning to the original view.
    session.send("t")?;
    session.wait("initial (HEAD)")?;
    session.send("t")?;
    // Then: the original comparison remains active.
    session.wait_for("original comparison", Duration::from_secs(4), |screen| {
        screen.contains("Snapshot")
            && screen.contains("No changes")
            && !screen.contains("History ·")
    })?;
    session.quit()
}

#[test]
fn toggling_history_preserves_the_open_detail_session() -> Result {
    // Given: a commit detail view opened over the worktree.
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("t")?;
    session.wait("initial (HEAD)")?;
    session.send("\r")?;
    session.wait("CHANGES · initial")?;
    // When: returning to the worktree and showing history again.
    session.send("t")?;
    session.wait("Watching")?;
    session.send("t")?;
    // Then: the same detail view and its collected file remain available.
    session.wait("CHANGES · initial")?;
    session.wait("main.rs")?;
    session.quit()
}

#[test]
fn historical_editor_copy_is_cleaned_up_without_changing_the_worktree() -> Result {
    // Given: a recorded editor and a committed file different from the checkout.
    let fixture = Fixture::new()?;
    let script = fixture.path(".git/historical-editor");
    let record = fixture.path(".git/historical-target");
    let content = fixture.path(".git/historical-content");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" > '{}'\ncat \"$1\" > '{}'\nprintf 'EDITOR_READY\\n'\nread reply\nprintf 'changed\\n' > \"$1\"\n",
            record.display(),
            content.display()
        ),
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    let editor = script.to_str().ok_or("Non-UTF-8 editor path")?;
    let mut session = fixture.start_with(&fixture.path("src"), &[], &[("VISUAL", Some(editor))])?;
    session.send("t")?;
    session.wait("initial (HEAD)")?;
    session.send("\r")?;
    session.wait("CHANGES")?;
    session.send("v")?;
    session.wait("FILES")?;
    session.send("Fmain.rs\r")?;
    session.wait("original")?;
    // When: opening, editing and closing the disposable historical copy.
    session.send("O")?;
    session.wait("EDITOR_READY")?;
    let target = fs::read_to_string(&record)?.trim().to_owned();
    session.send("close\n")?;
    // Then: the editor received the committed bytes and its copy is removed.
    session.wait_for("temporary file cleanup", Duration::from_secs(4), |_| {
        !std::path::Path::new(&target).exists()
    })?;
    assert_eq!(
        fs::read_to_string(content)?,
        "fn main() { println!(\"original\"); }\n"
    );
    assert!(!target.starts_with(fixture.root().to_string_lossy().as_ref()));
    assert_eq!(
        fs::read_to_string(fixture.path("src/main.rs"))?,
        "fn main() { println!(\"working\"); }\n"
    );
    session.quit()
}

#[test]
fn remapped_history_keys_open_commits_and_switch_files() -> Result {
    // Given: custom history and commit-file shortcuts.
    let fixture = Fixture::new()?;
    fixture.configure(
        "keybindings { global { history \"z\"; }; history { toggle-commit-files \"y\"; }; }",
    )?;
    let mut session = fixture.start()?;
    // When: opening history with the new shortcut and selecting a commit.
    session.send("z")?;
    session.wait("History · main")?;
    assert!(session.screen.contains("z toggle"));
    session.send("\r")?;
    session.wait("CHANGES")?;
    session.wait("y timecapsule")?;
    session.send("y")?;
    // Then: the remapped key opens the complete tree, and the original view remains accessible.
    session.wait("FILES")?;
    session.wait("y diff view")?;
    session.send("z")?;
    session.wait_for("restored worktree view", Duration::from_secs(4), |screen| {
        screen.contains("Watching") && !screen.contains("History ·")
    })?;
    session.quit()
}

#[test]
fn unborn_branch_opens_an_empty_history_screen() -> Result {
    // Given: a named branch with no commit yet.
    let fixture = Fixture::new()?;
    fixture.git(&["checkout", "--orphan", "unborn"])?;
    fixture.git(&["rm", "--cached", "-f", "-r", "."])?;
    let mut session = fixture.start()?;
    // When: entering history from the working-tree view.
    session.send("t")?;
    // Then: the empty branch has a usable history screen.
    session.wait("History · unborn")?;
    session.wait("No commits yet")?;
    session.quit()
}

#[test]
fn detached_head_opens_its_pinned_history() -> Result {
    // Given: a detached checkout of an existing commit.
    let fixture = Fixture::new()?;
    fixture.git(&["checkout", "--detach"])?;
    let mut session = fixture.start()?;
    // When: opening the history screen.
    session.send("t")?;
    // Then: the detached tip is available as the current history head.
    session.wait("History · Detached HEAD")?;
    session.wait("initial (HEAD)")?;
    session.quit()
}

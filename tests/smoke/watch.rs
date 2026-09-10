use super::fixtures::git;
use super::{Fixture, Result};
use std::{fs, time::Duration};

const WAIT: Duration = Duration::from_secs(5);

fn mapped_file(path: &std::path::Path) -> Result<(fs::File, memmap2::MmapMut)> {
    let file = fs::OpenOptions::new().read(true).write(true).open(path)?;
    // SAFETY: each test owns its fixture exclusively. The child only reads this
    // file, and the test never truncates it while the mapping exists.
    let mapped = unsafe { memmap2::MmapMut::map_mut(&file)? };
    Ok((file, mapped))
}

#[test]
fn mapped_write_refreshes_tree_and_diff_after_close() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    let (file, mut mapped) = mapped_file(&fixture.path("src/main.rs"))?;
    let offset = mapped
        .windows(7)
        .position(|bytes| bytes == b"working")
        .ok_or("Missing fixture text")?;
    mapped[offset..offset + 7].copy_from_slice(b"MAPPED!");
    mapped.flush()?;
    drop(mapped);
    drop(file);
    // Then
    session.wait_for("mapped edit", WAIT, |s| {
        s.contains("MAPPED!") && s.badge("main.rs", "MM")
    })?;
    session.quit()
}

#[test]
fn periodic_scan_recovers_a_mapped_write_before_close() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fs::write(fixture.config(), "watch { reconcile-seconds 1; }\n")?;
    let mut session = fixture.start()?;
    let (_file, mut mapped) = mapped_file(&fixture.path("notes.txt"))?;
    // When
    mapped.copy_from_slice(b"PERIODIC_EDIT!\n");
    mapped.flush()?;
    // Then
    session.wait_for("periodic recovery", WAIT, |s| {
        s.contains("PERIODIC_EDIT!") && s.badge("notes.txt", "?")
    })?;
    session.quit()
}

#[test]
fn content_change_without_status_change_reaches_the_screen() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    fixture.write("src/main.rs", "REFRESHED_CONTENT\n")?;
    // Then
    session.wait("REFRESHED_CONTENT")?;
    assert!(session.screen.badge("main.rs", "MM"));
    session.quit()
}

#[test]
fn atomic_save_preserves_selection() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("src/save.tmp", "ATOMIC_SAVE\n")?;
    let mut session = fixture.start()?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    fs::rename(fixture.path("src/save.tmp"), fixture.path("src/main.rs"))?;
    // Then
    session.wait("ATOMIC_SAVE")?;
    assert!(session.screen.selected("main.rs"));
    session.quit()
}

#[test]
fn untracked_creation_updates_both_panes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    fixture.write("new.txt", "NEW_FILE_CONTENT\n")?;
    // Then
    session.wait_for("new tree row and diff", WAIT, |s| {
        s.contains("NEW_FILE_CONTENT") && s.badge("new.txt", "?") && s.badge("All changes", "3")
    })?;
    session.quit()
}

#[test]
fn deleting_selected_file_returns_to_root() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("nn")?;
    session.select("notes.txt")?;
    // When
    fs::remove_file(fixture.path("notes.txt"))?;
    // Then
    session.wait_for("root without deleted file", WAIT, |s| {
        s.selected("All changes") && !s.contains("notes.txt") && s.badge("All changes", "1")
    })?;
    session.quit()
}

#[test]
fn tracked_deletion_updates_status() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    fs::remove_file(fixture.path("src/main.rs"))?;
    // Then
    session.wait_for("deleted file", WAIT, |s| {
        s.contains("Deleted") && s.badge("main.rs", "DM")
    })?;
    session.quit()
}

#[test]
fn staging_updates_both_panes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    fixture.git(&["add", "src/main.rs"])?;
    // Then
    session.wait_for("staged file", WAIT, |s| {
        s.badge("main.rs", "M")
            && !s.contains(" UNSTAGED ")
            && s.contains(" STAGED ")
            && s.contains("working")
    })?;
    session.quit()
}

#[test]
fn unstaging_updates_both_panes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("n")?;
    session.select("main.rs")?;
    // When
    fixture.git(&["restore", "--staged", "src/main.rs"])?;
    // Then
    session.wait_for("unstaged file", WAIT, |s| {
        s.badge("main.rs", "M")
            && !s.contains(" STAGED ")
            && s.contains(" UNSTAGED ")
            && s.contains("working")
    })?;
    session.quit()
}

#[test]
fn commit_clears_both_panes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.git(&["add", "."])?;
    let mut session = fixture.start()?;
    // When
    fixture.git(&["commit", "-m", "all changes"])?;
    // Then
    session.wait_for("clean view", WAIT, |s| {
        s.contains("No changes in this view") && s.badge("All changes", "0")
    })?;
    session.quit()
}

#[test]
fn staged_rename_replaces_the_tree_path() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.git(&[
        "restore",
        "--source=HEAD",
        "--staged",
        "--worktree",
        "src/main.rs",
    ])?;
    let mut session = fixture.start()?;
    // When
    fixture.git(&["mv", "src/main.rs", "src/renamed.rs"])?;
    // Then
    session.wait_for("renamed file", WAIT, |s| {
        s.badge("renamed.rs", "R") && s.contains("src/renamed.rs") && !s.badge("main.rs", "M")
    })?;
    session.quit()
}

#[test]
fn branch_switch_refreshes_comparison() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let tree = fixture.git(&["write-tree"])?;
    let commit = fixture.git(&["commit-tree", &tree, "-p", "HEAD", "-m", "staged branch"])?;
    fixture.git(&["branch", "watch-new-branch", &commit])?;
    let mut session = fixture.start()?;
    // When
    fixture.git(&["checkout", "watch-new-branch"])?;
    // Then
    session.wait_for("new comparison base", WAIT, |s| {
        s.badge("main.rs", "M") && !s.contains(" STAGED ") && s.contains("working")
    })?;
    session.quit()
}

#[test]
fn ignore_change_removes_file_from_both_panes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    fixture.write(".gitignore", "notes.txt\n")?;
    // Then
    session.wait_for("new ignore rules", WAIT, |s| {
        s.badge(".gitignore", "?") && !s.contains("untracked note") && !s.badge("notes.txt", "?")
    })?;
    session.quit()
}

#[test]
fn manual_refresh_detects_external_ignore_change() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let (_directory, ignore) = fixture.external_ignore()?;
    let mut session = fixture.start()?;
    fs::write(ignore, "")?;
    // When
    session.send("r")?;
    // Then
    session.wait("EXTERNAL_CHANGE")?;
    assert!(session.screen.badge("external.txt", "?"));
    session.quit()
}

#[test]
fn replacing_external_ignore_refreshes_without_periodic_scan() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let (_directory, ignore) = fixture.external_ignore()?;
    let replacement = ignore.with_file_name("replacement");
    fs::write(&replacement, "")?;
    let mut session = fixture.start()?;
    // When
    fs::rename(replacement, ignore)?;
    // Then
    session.wait("EXTERNAL_CHANGE")?;
    assert!(session.screen.badge("external.txt", "?"));
    session.quit()
}

#[test]
fn nested_ignore_negation_updates_both_panes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.write(".gitignore", "*.log\n")?;
    fixture.write("nested/keep.log", "NESTED_REVEALED\n")?;
    let mut session = fixture.start()?;
    // When
    fixture.write("nested/.gitignore", "!keep.log\n")?;
    // Then
    session.wait("NESTED_REVEALED")?;
    assert!(session.screen.badge("keep.log", "?"));
    session.quit()
}

#[test]
fn linked_worktree_observes_external_index() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let directory = tempfile::tempdir()?;
    let linked = directory.path().join("worktree");
    fixture.git(&[
        "worktree",
        "add",
        "-b",
        "linked",
        linked.to_str().ok_or("Invalid worktree path")?,
    ])?;
    fs::write(linked.join("src/main.rs"), "LINKED_CONTENT\n")?;
    let mut session = fixture.start_with(&linked, &[], &[])?;
    // When
    git(&linked, &["add", "src/main.rs"])?;
    // Then
    session.wait_for("linked worktree staging", WAIT, |s| {
        s.badge("main.rs", "M") && !s.contains(" UNSTAGED ") && s.contains("LINKED_CONTENT")
    })?;
    session.quit()
}

#[test]
fn refresh_preserves_filter_selection_and_search() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.resize(160, 35)?;
    session.send("nfsrc\r/working\r")?;
    session.wait("/working: 1 matches")?;
    // When
    fixture.write("src/main.rs", "working\nworking again\n")?;
    // Then
    session.wait_for("preserved view with updated matches", WAIT, |s| {
        s.contains("/working: 2 matches")
            && s.selected("main.rs")
            && s.contains("filter: src")
            && !s.contains("notes.txt")
    })?;
    session.quit()
}

#[test]
fn burst_eventually_displays_final_content() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    // When
    std::thread::scope(|scope| -> Result {
        let writer = scope.spawn(|| -> Result {
            for i in 0..40 {
                fixture.write("src/main.rs", format!("burst {i}\n"))?;
                std::thread::sleep(Duration::from_millis(20));
            }
            fixture.write("src/main.rs", "BURST_FINAL\n")
        });
        // Then
        session.wait("BURST_FINAL")?;
        writer.join().map_err(|_| "Burst writer failed")??;
        Ok(())
    })?;
    assert!(session.screen.badge("main.rs", "MM"));
    session.quit()
}

#[test]
fn renamed_directory_and_subsequent_edits_are_observed() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("draft/child.txt", "DIRECTORY_CONTENT\n")?;
    let mut session = fixture.start()?;
    // When
    fs::rename(fixture.path("draft"), fixture.path("renamed"))?;
    // Then
    session.wait_for("renamed directory", WAIT, |s| {
        s.contains("renamed/child.txt") && !s.contains("draft") && s.contains("DIRECTORY_CONTENT")
    })?;
    fixture.write("renamed/child.txt", "AFTER_RENAME\n")?;
    session.wait("AFTER_RENAME")?;
    assert!(session.screen.contains("renamed/child.txt"));
    session.quit()
}

#[test]
fn submodule_head_change_refreshes_clean_tree() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let source = tempfile::tempdir()?;
    git(source.path(), &["init", "--initial-branch=main"])?;
    git(source.path(), &["commit", "--allow-empty", "-m", "initial"])?;
    fixture.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        source.path().to_str().ok_or("Invalid module path")?,
        "module",
    ])?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "add module"])?;
    let mut session = fixture.start()?;
    // When
    git(
        &fixture.path("module"),
        &["commit", "--allow-empty", "-m", "new module commit"],
    )?;
    // Then
    session.wait_for("changed submodule", WAIT, |s| {
        s.contains("Submodule change") && s.badge("All changes", "1") && s.badge("module", "M")
    })?;
    session.quit()
}

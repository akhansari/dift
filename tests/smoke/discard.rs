use super::*;

fn open(session: &mut terminal::Session) -> Result {
    session.send("D")?;
    session.wait("Discard unstaged changes (")?;
    session.wait("› Discard unstaged changes")
}

#[test]
fn discard_menu_can_cancel_without_touching_changes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let status = fixture.git(&["status", "--porcelain"])?;
    let mut session = fixture.start()?;
    open(&mut session)?;
    // When
    session.send("k\r")?;
    // Then
    session.wait_for("closed discard menu", Duration::from_secs(4), |screen| {
        !screen.contains("Discard changes")
    })?;
    assert_eq!(fixture.git(&["status", "--porcelain"])?, status);
    assert!(fs::read_to_string(fixture.path("src/main.rs"))?.contains("working"));
    session.quit()
}

#[test]
fn discard_defaults_to_all_when_unstaged_is_unavailable() -> Result {
    // Given
    let fixture = Fixture::new()?;
    fixture.git(&["add", "src/main.rs"])?;
    let mut session = fixture.start()?;
    session.send("D")?;
    session.wait("› Discard all changes")?;
    // When
    session.send("\r")?;
    // Then
    session.wait("Discarded changes")?;
    assert!(fixture.git(&["status", "--porcelain"])?.is_empty());
    session.quit()
}

#[test]
fn discard_unstaged_preserves_index_and_untracked_files_and_refreshes_panes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    open(&mut session)?;
    // When
    session.send("\r")?;
    // Then
    session.wait("Discarded changes")?;
    assert!(fixture.git(&["diff"])?.is_empty());
    assert!(fixture.git(&["show", ":src/main.rs"])?.contains("staged"));
    assert!(fixture.path("notes.txt").exists());
    session.wait_for("refreshed categories", Duration::from_secs(4), |screen| {
        !screen.contains("UNSTAGED") && screen.contains("UNTRACKED") && screen.contains("STAGED")
    })?;
    session.quit()
}

#[test]
fn discard_all_from_filtered_root_includes_hidden_changes() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("fnotes\r")?;
    session.wait("filter: notes")?;
    open(&mut session)?;
    session.wait("Discard all changes (2 files, 1 deletions)")?;
    // When
    session.send("j\r")?;
    // Then
    session.wait("Discarded changes")?;
    assert!(fixture.git(&["status", "--porcelain"])?.is_empty());
    assert!(!fixture.path("notes.txt").exists());
    assert!(fs::read_to_string(fixture.path("src/main.rs"))?.contains("original"));
    session.quit()
}

#[test]
fn discard_from_diff_focus_only_changes_current_file() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    session.send("\t")?;
    open(&mut session)?;
    session.wait("File: \"src/main.rs\"")?;
    // When
    session.send("j\r")?;
    // Then
    session.wait("Discarded changes")?;
    assert!(fixture.git(&["diff"])?.is_empty());
    assert!(fixture.git(&["diff", "--cached"])?.is_empty());
    assert_eq!(
        fs::read_to_string(fixture.path("notes.txt"))?,
        "untracked note\n"
    );
    session.quit()
}

#[test]
fn discard_stale_review_resets_to_cancel_and_preserves_new_edits() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    open(&mut session)?;
    session.send("j")?;
    fixture.write("src/main.rs", "new edit during review\n")?;
    // When
    session.send("\r")?;
    // Then
    session.wait("Changes changed; review again")?;
    session.wait("› Cancel")?;
    assert_eq!(
        fs::read_to_string(fixture.path("src/main.rs"))?,
        "new edit during review\n"
    );
    assert!(fixture.path("notes.txt").exists());
    session.send("\x1b")?;
    session.wait_for("closed discard menu", Duration::from_secs(4), |screen| {
        !screen.contains("Discard changes")
    })?;
    session.quit()
}

#[test]
fn discard_menu_captures_stage_editor_filter_and_mouse_input() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let status = fixture.git(&["status", "--porcelain"])?;
    let mut session = fixture.start()?;
    open(&mut session)?;
    // When
    session.send(" of/\t\x1b[<0;2;4M\x1b[<0;2;4m")?;
    // Then
    session.observe(Duration::from_millis(150))?;
    assert!(session.screen.contains("› Discard unstaged changes"));
    assert!(session.screen.contains("All changes in repository"));
    assert_eq!(fixture.git(&["status", "--porcelain"])?, status);
    session.send("\x1b")?;
    session.wait_for("closed discard menu", Duration::from_secs(4), |screen| {
        !screen.contains("Discard changes")
    })?;
    session.quit()
}

#[test]
fn comparison_mode_rejects_discard() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let status = fixture.git(&["status", "--porcelain"])?;
    let mut session = fixture.start_with(fixture.root(), &["diff", "HEAD", "HEAD"], &[])?;
    // When
    session.send("D")?;
    // Then
    session.wait("Discard is unavailable in comparison mode")?;
    assert_eq!(fixture.git(&["status", "--porcelain"])?, status);
    session.quit()
}

#[test]
fn discard_internal_activity_is_quiet_and_later_external_edit_highlights() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let mut session = fixture.start()?;
    let background = tree_background(&session.screen, "main.rs").ok_or("Missing tree file")?;
    open(&mut session)?;
    session.send("\r")?;
    session.wait("Discarded changes")?;
    session.observe(Duration::from_millis(500))?;
    assert_eq!(
        tree_background(&session.screen, "main.rs"),
        Some(background)
    );
    // When
    fixture.write("src/main.rs", "external edit after discard\n")?;
    // Then
    session.wait_for(
        "external activity after discard",
        Duration::from_secs(4),
        |screen| tree_background(screen, "main.rs").is_some_and(|color| color != background),
    )?;
    session.quit()
}

#[test]
fn discard_cannot_execute_when_terminal_is_too_small_to_review() -> Result {
    // Given
    let fixture = Fixture::new()?;
    let status = fixture.git(&["status", "--porcelain"])?;
    let mut session = fixture.start()?;
    open(&mut session)?;
    session.send("j")?;
    session.resize(35, 10)?;
    session.wait("Enlarge terminal")?;
    // When
    session.send("\r")?;
    // Then
    session.observe(Duration::from_millis(150))?;
    assert_eq!(fixture.git(&["status", "--porcelain"])?, status);
    session.send("\x1b")?;
    session.wait_for("closed discard menu", Duration::from_secs(4), |screen| {
        !screen.contains("Discard changes")
    })?;
    session.quit()
}

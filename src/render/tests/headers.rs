use super::*;

macro_rules! status_test {
    ($name:ident, $status:literal, $label:literal) => {
        #[test]
        fn $name() -> crate::AppResult<()> {
            // Given
            let mut renderer = Renderer::new(Config::default())?;
            let mut snapshot = snapshot();
            snapshot.changes[0].status = $status;
            // When
            let document = renderer.document(&snapshot, &[0], 100, false, "")?;
            // Then
            assert!(document.rows[1].line.to_string().contains($label));
            Ok(())
        }
    };
}

status_test!(
    modified_files_have_a_readable_status,
    'M',
    "Modified · Unstaged"
);
status_test!(added_files_have_a_readable_status, 'A', "Added · Unstaged");
status_test!(
    deleted_files_have_a_readable_status,
    'D',
    "Deleted · Unstaged"
);
status_test!(
    renamed_files_have_a_readable_status,
    'R',
    "Renamed · Unstaged"
);
status_test!(
    copied_files_have_a_readable_status,
    'C',
    "Copied · Unstaged"
);
status_test!(
    type_changes_have_a_readable_status,
    'T',
    "Type changed · Unstaged"
);
status_test!(unmerged_status_means_conflict, 'U', "Conflict · Unstaged");
status_test!(
    untracked_files_have_a_readable_status,
    '?',
    "Untracked · Unstaged"
);
status_test!(
    unknown_status_codes_remain_visible,
    'X',
    "Changed (X) · Unstaged"
);

#[test]
fn header_counts_measure_source_lines_instead_of_wrapped_rows() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = text_snapshot(&"a long source line ".repeat(20));
    // When
    let document = renderer.document(&snapshot, &[0], 50, false, "")?;
    // Then
    assert!(document.rows.len() > 5);
    assert!(document.rows[1].line.to_string().ends_with("+1 −0 "));
    assert_eq!(document.rows[1].line.width(), 50);
    Ok(())
}

#[test]
fn omitted_contents_do_not_claim_zero_changes() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let mut snapshot = snapshot();
    snapshot.changes[0].hunks.clear();
    snapshot.changes[0].oversized = true;
    snapshot.changes[0].summary = Some("Contents omitted".into());
    // When
    let document = renderer.document(&snapshot, &[0], 80, false, "")?;
    // Then
    let header = document.rows[1].line.to_string();
    assert!(header.contains("Modified · Unstaged"));
    assert!(!header.contains('+'));
    assert!(!header.contains('−'));
    Ok(())
}

#[test]
fn conflict_previews_do_not_claim_zero_additions_and_deletions() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let mut snapshot = text_snapshot("unmerged content");
    snapshot.changes[0].status = 'U';
    snapshot.changes[0].hunks[0].lines[0].kind = LineKind::Conflict;
    // When
    let document = renderer.document(&snapshot, &[0], 80, false, "")?;
    // Then
    let header = document.rows[1].line.to_string();
    assert!(header.contains("Conflict · Unstaged"));
    assert!(!header.contains('+'));
    assert!(!header.contains('−'));
    Ok(())
}

#[test]
fn renames_keep_both_paths_when_space_allows() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let mut snapshot = language_snapshot("new/main.rs", "changed");
    snapshot.changes[0].status = 'R';
    snapshot.changes[0].old_path = Some("old/file.rs".into());
    // When
    let document = renderer.document(&snapshot, &[0], 100, false, "")?;
    // Then
    let header = document.rows[1].line.to_string();
    assert!(header.starts_with("  old/file.rs → new/main.rs "));
    assert!(header.contains("Renamed · Unstaged"));
    Ok(())
}

#[test]
fn narrow_headers_prioritize_the_filename_over_metadata() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("very/long/directory/main.rs", "changed");
    // When
    let document = renderer.document(&snapshot, &[0], 14, false, "")?;
    // Then
    let header = document.rows[1].line.to_string();
    assert!(header.contains("main.rs"));
    assert!(header.contains('…'));
    assert!(!header.contains("Modified"));
    assert!(!header.contains("Unstaged"));
    assert!(!header.contains('+'));
    assert_eq!(document.rows[1].line.width(), 14);
    Ok(())
}

#[test]
fn narrow_renames_preserve_the_new_filename() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let mut snapshot = language_snapshot("new/directory/main.rs", "changed");
    snapshot.changes[0].status = 'R';
    snapshot.changes[0].old_path = Some("old/directory/file.rs".into());
    // When
    let document = renderer.document(&snapshot, &[0], 10, false, "")?;
    // Then
    assert_eq!(document.rows[1].line.to_string(), "  main.rs ");
    Ok(())
}

#[test]
fn truncation_preserves_emoji_graphemes_and_the_extension() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("👩‍💻résumé.rs", "changed");
    // When
    let document = renderer.document(&snapshot, &[0], 12, false, "")?;
    // Then
    let header = document.rows[1].line.to_string();
    assert!(header.starts_with("  👩‍💻"));
    assert!(header.contains('…'));
    assert!(header.ends_with(".rs "));
    assert_eq!(document.rows[1].line.width(), 12);
    Ok(())
}

#[test]
fn headers_escape_control_characters_in_paths() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("src/new\nfile.rs", "changed");
    // When
    let document = renderer.document(&snapshot, &[0], 80, false, "")?;
    // Then
    let header = document.rows[1].line.to_string();
    assert!(header.contains("src/new\\nfile.rs"));
    assert!(!header.contains('\n'));
    Ok(())
}

#[test]
fn counts_give_way_before_the_directory_is_shortened() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("src/main.rs", "changed");
    // When
    let document = renderer.document(&snapshot, &[0], 35, false, "")?;
    // Then
    let header = document.rows[1].line.to_string();
    assert!(header.starts_with("  src/main.rs "));
    assert!(header.contains("Modified · Unstaged"));
    assert!(!header.contains('+'));
    assert_eq!(document.rows[1].line.width(), 35);
    Ok(())
}

#[test]
fn a_preview_ending_before_a_file_header_has_no_current_file() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = snapshot();
    let mut preview = None;
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &snapshot,
            selected: &[0],
            width: 80,
            split: false,
            show_line_numbers: true,
            query: "",
            preview_rows: 1,
        },
        &Cancellation::default(),
        |update| {
            preview = Some(update.document);
            false
        },
    );
    // Then
    assert_eq!(result, Err(Cancelled));
    let document = preview.ok_or("Missing preview")?;
    assert!(document.partial);
    assert_eq!(document.rows.len(), 1);
    assert_eq!(document.current_file_section(0, 7), None);
    Ok(())
}

#[test]
fn a_partial_file_header_does_not_extend_past_the_preview() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = snapshot();
    let mut preview = None;
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &snapshot,
            selected: &[0],
            width: 80,
            split: false,
            show_line_numbers: true,
            query: "",
            preview_rows: 4,
        },
        &Cancellation::default(),
        |update| {
            preview = Some(update.document);
            false
        },
    );
    // Then
    assert_eq!(result, Err(Cancelled));
    let document = preview.ok_or("Missing preview")?;
    assert!(document.partial);
    assert_eq!(document.rows.len(), 4);
    assert_eq!(
        document
            .current_file_section(3, 7)
            .map(|section| &section.rows),
        Some(&(1..4))
    );
    assert_eq!(document.current_file_section(4, 7), None);
    Ok(())
}

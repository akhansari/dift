use super::*;
use crate::model::Hunk;

mod headers;
mod refinement;

fn snapshot() -> Snapshot {
    Snapshot {
        activity: None,
        branch: "main".into(),
        whitespace: Default::default(),
        whitespace_hidden_files: 0,
        tracked_stats: None,
        changes: vec![Change {
            path: "main.rs".into(),
            old_path: None,
            category: Category::Unstaged,
            status: 'M',
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".into(),
                lines: vec![
                    SourceLine {
                        kind: LineKind::Removed,
                        old: Some(1),
                        new: None,
                        text: "let before = 1;".into(),
                        no_newline: false,
                    },
                    SourceLine {
                        kind: LineKind::Added,
                        old: None,
                        new: Some(1),
                        text: "let after = 2;".into(),
                        no_newline: false,
                    },
                ],
            }],
            old_source: Some(Arc::from("let before = 1;\n")),
            new_source: Some(Arc::from("let after = 2;\n")),
            summary: None,
            oversized: false,
        }],
    }
}

fn text_snapshot(text: &str) -> Snapshot {
    let mut snapshot = snapshot();
    snapshot.changes[0].hunks[0].lines = vec![SourceLine {
        kind: LineKind::Added,
        old: None,
        new: Some(1),
        text: text.into(),
        no_newline: false,
    }];
    snapshot.changes[0].new_source = Some(Arc::from(text));
    snapshot
}

fn source_content(document: &Document) -> String {
    document
        .rows
        .iter()
        .filter(|r| !r.anchors.is_empty())
        .flat_map(|r| r.line.spans.iter().skip(1))
        .map(|span| span.content.as_ref())
        .collect()
}

fn styled_source_lines(document: &Document) -> Vec<Line<'static>> {
    document
        .rows
        .iter()
        .filter(|row| !row.anchors.is_empty())
        .map(|row| row.line.clone())
        .collect()
}

fn language_snapshot(path: &str, text: &str) -> Snapshot {
    let mut snapshot = text_snapshot(text);
    snapshot.changes[0].path = path.into();
    snapshot
}

fn two_file_snapshot() -> Snapshot {
    let mut snapshot = language_snapshot("first.ts", "const first = 1;");
    snapshot
        .changes
        .extend(language_snapshot("second.ts", "const needle = 2;").changes);
    snapshot
}

fn two_file_spec(snapshot: &Snapshot) -> RenderSpec<'_> {
    RenderSpec {
        snapshot,
        selected: &[0, 1],
        width: 100,
        split: false,
        query: "needle",
        preview_rows: 0,
    }
}

#[test]
fn a_highlighted_file_is_published_with_the_searchable_plain_remainder() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = two_file_snapshot();
    let mut updates = Vec::new();
    // When
    let result = renderer.render_progressive(
        two_file_spec(&snapshot),
        &Cancellation::default(),
        |update| {
            updates.push(update);
            updates.len() < 2
        },
    );
    // Then
    assert_eq!(result, Err(Cancelled));
    assert_eq!(updates.len(), 2);
    let plain = &updates[0].document;
    let enriched = &updates[1].document;
    assert!(!updates[1].finished);
    assert!(!enriched.partial);
    assert_eq!(source_content(enriched), source_content(plain));
    assert_eq!(enriched.hits.len(), 1);
    assert_eq!(enriched.hits[0].row, plain.hits[0].row);
    assert_ne!(
        source_foreground(enriched, "const")?,
        source_foreground(plain, "const")?
    );
    assert_eq!(renderer.stats().syntax_lines, 1);
    Ok(())
}

#[test]
fn changing_selection_after_a_highlighted_file_cancels_the_remainder() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = two_file_snapshot();
    let latest = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let cancellation = Cancellation::new(Arc::clone(&latest), 1);
    let mut updates = Vec::new();
    // When
    let result = renderer.render_progressive(two_file_spec(&snapshot), &cancellation, |update| {
        updates.push(update);
        if updates.len() == 2 {
            latest.store(2, std::sync::atomic::Ordering::Relaxed);
        }
        true
    });
    // Then
    assert_eq!(result, Err(Cancelled));
    assert_eq!(updates.len(), 2);
    assert_eq!(renderer.stats().syntax_lines, 1);
    assert!(!updates[1].finished);
    Ok(())
}

#[test]
fn progressive_highlighting_finishes_with_the_same_colors_as_individual_files()
-> crate::AppResult<()> {
    // Given
    let snapshot = two_file_snapshot();
    let mut renderer = Renderer::new(Config::default())?;
    let first = renderer.document(&snapshot, &[0], 100, false, "needle")?;
    let second = renderer.document(&snapshot, &[1], 100, false, "needle")?;
    let expected: Vec<_> = styled_source_lines(&first)
        .into_iter()
        .chain(styled_source_lines(&second))
        .collect();
    let mut renderer = Renderer::new(Config::default())?;
    let mut updates = Vec::new();
    // When
    renderer.render_progressive(
        two_file_spec(&snapshot),
        &Cancellation::default(),
        |update| {
            updates.push(update);
            true
        },
    )?;
    // Then
    let final_update = updates.last().ok_or("Missing final rendering")?;
    let final_document = &final_update.document;
    assert!(final_update.finished);
    assert!(!final_document.partial);
    assert_eq!(styled_source_lines(final_document), expected);
    assert_eq!(final_document.hits.len(), 1);
    Ok(())
}

fn source_foreground(document: &Document, token: &str) -> crate::AppResult<Color> {
    document
        .rows
        .iter()
        .filter(|row| !row.anchors.is_empty())
        .flat_map(|row| row.line.spans.iter().skip(1))
        .find(|span| span.content.contains(token))
        .and_then(|span| span.style.fg)
        .ok_or_else(|| format!("Missing rendered token {token:?}").into())
}

#[test]
fn typescript_keywords_have_syntax_colors() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("answer.ts", "const answer: number = 42;");
    // When
    let document = renderer.document(&snapshot, &[0], 100, false, "")?;
    // Then
    assert_ne!(
        source_foreground(&document, "const")?,
        source_foreground(&document, "answer")?
    );
    Ok(())
}

#[test]
fn tsx_tags_and_attribute_values_have_distinct_syntax_colors() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("view.tsx", "const view = <Button label=\"ready\" />;");
    // When
    let document = renderer.document(&snapshot, &[0], 100, true, "")?;
    // Then
    assert_ne!(
        source_foreground(&document, "Button")?,
        source_foreground(&document, "ready")?
    );
    Ok(())
}

#[test]
fn dockerfile_name_selects_syntax_without_an_extension() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("Dockerfile", "FROM alpine:3.20");
    // When
    let document = renderer.document(&snapshot, &[0], 100, false, "")?;
    // Then
    assert_ne!(
        source_foreground(&document, "FROM")?,
        source_foreground(&document, "alpine")?
    );
    Ok(())
}

#[test]
fn toml_keys_and_values_have_distinct_syntax_colors() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("config.toml", "enabled = true");
    // When
    let document = renderer.document(&snapshot, &[0], 100, false, "")?;
    // Then
    assert_ne!(
        source_foreground(&document, "enabled")?,
        source_foreground(&document, "true")?
    );
    Ok(())
}

#[test]
fn rust_keeps_syntax_highlighting_with_the_expanded_bundle() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("answer.rs", "let answer: u32 = 42;");
    // When
    let document = renderer.document(&snapshot, &[0], 100, false, "")?;
    // Then
    assert_ne!(
        source_foreground(&document, "let")?,
        source_foreground(&document, "answer")?
    );
    Ok(())
}

#[test]
fn unknown_extensions_keep_plain_text_and_diff_styling() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = language_snapshot("answer.unknown-language", "const answer: number = 42;");
    // When
    let document = renderer.document(&snapshot, &[0], 100, false, "")?;
    // Then
    assert_eq!(
        source_foreground(&document, "const")?,
        source_foreground(&document, "answer")?
    );
    assert!(source_content(&document).contains("const answer: number = 42;"));
    Ok(())
}

#[test]
fn narrow_wrapping_preserves_expanded_tabs_and_wide_characters() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = text_snapshot("\t界x\u{1b}");
    // When
    let document = renderer.document(&snapshot, &[0], 4, false, "界")?;
    // Then
    assert!(
        document
            .rows
            .iter()
            .filter(|r| !r.anchors.is_empty())
            .all(|r| r.line.width() <= 4)
    );
    assert_eq!(source_content(&document), "    界x\\u{1b} ");
    assert_eq!(document.hits.len(), 1);
    Ok(())
}

#[test]
fn missing_final_newline_is_visible_without_becoming_searchable_source() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let mut snapshot = text_snapshot("after");
    snapshot.changes[0].hunks[0].lines[0].no_newline = true;
    // When
    let document = renderer.document(&snapshot, &[0], 80, true, "No newline")?;
    // Then
    assert!(
        document
            .rows
            .iter()
            .any(|row| row.line.to_string().contains("No newline at end of file"))
    );
    assert!(document.hits.is_empty());
    Ok(())
}

#[test]
fn smart_case_search_preserves_original_unicode_byte_ranges() {
    // Given
    let text = "ÉTÉ été";
    // When
    let matches = search_ranges(text, "été");
    // Then
    assert_eq!(matches, vec![0..5, 6..11]);
}

#[test]
fn uppercase_query_is_case_sensitive() {
    // Given
    let text = "thing Thing";
    // When
    let matches = search_ranges(text, "Thing");
    // Then
    assert_eq!(matches, vec![6..11]);
}

#[test]
fn split_view_preserves_both_sides_and_searches_source_text() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = snapshot();
    // When
    let doc = renderer.document(&snapshot, &[0], 80, true, "after")?;
    // Then
    assert_eq!(doc.hits.len(), 1);
    assert!(
        doc.rows
            .iter()
            .any(|r| r.line.to_string().contains("before") && r.line.to_string().contains("after"))
    );
    Ok(())
}

#[test]
fn source_anchor_survives_layout_change() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = snapshot();
    let unified = renderer.document(&snapshot, &[0], 30, false, "after")?;
    let anchor = unified
        .anchor(unified.hits[0].row)
        .ok_or("Missing source anchor")?;
    // When
    let split = renderer.document(&snapshot, &[0], 80, true, "after")?;
    // Then
    assert_eq!(split.locate(&anchor), Some(split.hits[0].row));
    Ok(())
}

#[test]
fn changing_layout_at_the_top_keeps_section_headers_visible() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = snapshot();
    let unified = renderer.document(&snapshot, &[0], 30, false, "")?;
    let split = renderer.document(&snapshot, &[0], 80, true, "")?;
    // When
    let scroll = split.reanchored_scroll(&unified, 0);
    // Then
    assert_eq!(scroll, 0);
    assert_eq!(split.rows[scroll].line.to_string(), " UNSTAGED ");
    Ok(())
}

#[test]
fn refresh_at_a_file_header_preserves_its_offset_from_source() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = snapshot();
    let previous = renderer.document(&snapshot, &[0], 80, false, "")?;
    let refreshed = renderer.document(&snapshot, &[0], 80, false, "")?;
    // When
    let scroll = refreshed.reanchored_scroll(&previous, 1);
    // Then
    let header = refreshed.rows[scroll].line.to_string();
    assert!(header.starts_with("  main.rs "));
    assert!(header.contains("Modified · Unstaged"));
    Ok(())
}

#[test]
fn staging_a_file_keeps_its_source_visible_in_an_aggregate_diff() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let mut before = text_snapshot("reading here");
    before.changes[0].category = Category::Untracked;
    let previous = renderer.document(&before, &[0], 80, false, "reading")?;
    let mut after = language_snapshot("other.rs", "unrelated content");
    let mut staged = text_snapshot("reading here");
    staged.changes[0].category = Category::Staged;
    after.changes.extend(staged.changes);
    let refreshed = renderer.document(&after, &[0, 1], 80, false, "reading")?;
    // When
    let scroll = refreshed.reanchored_scroll(&previous, previous.hits[0].row);
    // Then
    assert_eq!(scroll, refreshed.hits[0].row);
    assert_eq!(refreshed.rows[scroll].anchors[0].category, Category::Staged);
    assert_eq!(
        refreshed.rows[scroll].anchors[0].text.as_ref(),
        "reading here"
    );
    Ok(())
}

fn source_anchor(category: Category, old: bool, number: usize, text: &str) -> Anchor {
    Anchor {
        path: Arc::new("main.rs".into()),
        category,
        old,
        number,
        byte: 0,
        text: Arc::from(text),
    }
}

fn anchored_document(anchors: Vec<Anchor>) -> Document {
    Document {
        rows: anchors
            .into_iter()
            .map(|anchor| {
                Arc::new(Row {
                    line: Line::raw(anchor.text.to_string()),
                    anchors: vec![anchor],
                    code: Vec::new(),
                })
            })
            .collect(),
        ..Document::default()
    }
}

#[test]
fn an_exact_match_in_the_original_category_wins_over_another_category() {
    // Given
    let anchor = source_anchor(Category::Unstaged, false, 10, "reading");
    let document = anchored_document(vec![
        source_anchor(Category::Staged, false, 10, "reading"),
        source_anchor(Category::Unstaged, false, 20, "reading"),
    ]);
    // When
    let position = document.locate(&anchor);
    // Then
    assert_eq!(position, Some(1));
}

#[test]
fn a_line_moved_by_partial_staging_wins_over_an_approximate_unstaged_match() {
    // Given
    let anchor = source_anchor(Category::Unstaged, false, 10, "reading");
    let document = anchored_document(vec![
        source_anchor(Category::Unstaged, false, 10, "other text"),
        source_anchor(Category::Staged, false, 20, "reading"),
    ]);
    // When
    let position = document.locate(&anchor);
    // Then
    assert_eq!(position, Some(1));
}

#[test]
fn unstaging_keeps_the_same_text_on_the_same_side() {
    // Given
    let anchor = source_anchor(Category::Staged, false, 10, "reading");
    let document = anchored_document(vec![
        source_anchor(Category::Unstaged, true, 10, "reading"),
        source_anchor(Category::Unstaged, false, 20, "reading"),
    ]);
    // When
    let position = document.locate(&anchor);
    // Then
    assert_eq!(position, Some(1));
}

#[test]
fn different_text_in_another_category_is_not_a_source_match() {
    // Given
    let anchor = source_anchor(Category::Staged, false, 10, "reading");
    let document = anchored_document(vec![source_anchor(
        Category::Unstaged,
        false,
        10,
        "other text",
    )]);
    // When
    let position = document.locate(&anchor);
    // Then
    assert_eq!(position, None);
}

#[test]
fn edited_text_falls_back_to_the_nearest_line_in_its_category() {
    // Given
    let anchor = source_anchor(Category::Unstaged, false, 10, "reading");
    let document = anchored_document(vec![
        source_anchor(Category::Unstaged, false, 9, "edited"),
        source_anchor(Category::Unstaged, false, 20, "other text"),
    ]);
    // When
    let position = document.locate(&anchor);
    // Then
    assert_eq!(position, Some(0));
}

#[test]
fn a_comparison_anchor_does_not_match_a_working_tree_category() {
    // Given
    let anchor = source_anchor(Category::Comparison, false, 10, "reading");
    let document = anchored_document(vec![source_anchor(Category::Staged, false, 10, "reading")]);
    // When
    let position = document.locate(&anchor);
    // Then
    assert_eq!(position, None);
}

fn large_snapshot(lines: usize, comment_at: Option<usize>) -> Snapshot {
    let source = (0..lines)
        .map(|i| {
            if Some(i) == comment_at {
                "/* opened comment\n".to_owned()
            } else {
                format!("let value_{i} = {i};\n")
            }
        })
        .collect::<String>();
    let mut snapshot = language_snapshot(
        "large.rs",
        &format!("let value_{} = {};", lines - 1, lines - 1),
    );
    snapshot.changes[0].hunks[0].lines[0].new = Some(lines);
    snapshot.changes[0].new_source = Some(Arc::from(source));
    snapshot
}

fn rendered_lines(document: &Document) -> Vec<Line<'static>> {
    document.rows.iter().map(|row| row.line.clone()).collect()
}

#[test]
fn returning_to_an_unchanged_file_reuses_the_prepared_layout() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let file = language_snapshot("answer.ts", "const answer: number = 42;");
    let other = language_snapshot("other.rs", "let other = 7;");
    let first = renderer.document(&file, &[0], 100, false, "")?;
    renderer.document(&other, &[0], 100, false, "")?;
    let before = renderer.stats();
    // When
    let revisited = renderer.document(&file, &[0], 100, false, "")?;
    // Then
    assert_eq!(renderer.stats().syntax_lines, before.syntax_lines);
    assert_eq!(renderer.stats().layout_hits, before.layout_hits + 1);
    assert_eq!(rendered_lines(&revisited), rendered_lines(&first));
    Ok(())
}

#[test]
fn layout_and_search_changes_reuse_syntax_processing() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let file = language_snapshot("answer.ts", "const answer: number = 42;");
    renderer.document(&file, &[0], 100, false, "")?;
    let before = renderer.stats();
    // When
    let document = renderer.document(&file, &[0], 60, true, "answer")?;
    // Then
    assert_eq!(renderer.stats().syntax_lines, before.syntax_lines);
    assert!(renderer.stats().prepared_hits > before.prepared_hits);
    assert_eq!(document.hits.len(), 1);
    Ok(())
}

#[test]
fn the_first_preview_arrives_before_any_syntax_is_parsed() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let file = large_snapshot(1024, None);
    let mut preview = None;
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &file,
            selected: &[0],
            width: 100,
            split: false,
            query: "value",
            preview_rows: 16,
        },
        &Cancellation::default(),
        |update| {
            preview = Some(update.document);
            false
        },
    );
    // Then
    assert_eq!(result, Err(Cancelled));
    assert_eq!(renderer.stats().syntax_lines, 0);
    assert!(
        source_content(preview.ok_or("Missing preview")?.as_ref())
            .contains("let value_1023 = 1023;")
    );
    Ok(())
}

#[test]
fn an_obsolete_request_publishes_nothing() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let file = large_snapshot(1024, None);
    let latest = Arc::new(std::sync::atomic::AtomicU64::new(2));
    let cancellation = Cancellation::new(latest, 1);
    let mut updates = Vec::new();
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &file,
            selected: &[0],
            width: 100,
            split: false,
            query: "",
            preview_rows: 16,
        },
        &cancellation,
        |update| {
            updates.push(update);
            true
        },
    );
    // Then
    assert_eq!(result, Err(Cancelled));
    assert!(updates.is_empty());
    assert_eq!(renderer.stats().syntax_lines, 0);
    Ok(())
}

#[test]
fn changing_selection_after_the_preview_cancels_enrichment() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let file = large_snapshot(1024, None);
    let latest = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let cancellation = Cancellation::new(Arc::clone(&latest), 1);
    let mut updates = Vec::new();
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &file,
            selected: &[0],
            width: 100,
            split: false,
            query: "",
            preview_rows: 16,
        },
        &cancellation,
        |update| {
            updates.push(update);
            latest.store(2, std::sync::atomic::Ordering::Relaxed);
            true
        },
    );
    // Then
    assert_eq!(result, Err(Cancelled));
    assert_eq!(updates.len(), 1);
    assert!(!updates[0].finished);
    assert_eq!(renderer.stats().syntax_lines, 0);
    Ok(())
}

#[test]
fn an_edit_reuses_checkpoints_and_preserves_multiline_syntax_state() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let original = large_snapshot(300, None);
    let edited = large_snapshot(300, Some(260));
    renderer.document(&original, &[0], 100, false, "")?;
    let before = renderer.stats();
    let expected = Renderer::new(Config::default())?.document(&edited, &[0], 100, false, "")?;
    // When
    let actual = renderer.document(&edited, &[0], 100, false, "")?;
    // Then
    assert!(renderer.stats().syntax_lines - before.syntax_lines <= 128);
    assert!(renderer.stats().reused_syntax_lines > before.reused_syntax_lines);
    assert_eq!(rendered_lines(&actual), rendered_lines(&expected));
    Ok(())
}

#[test]
fn changing_a_multiline_opener_invalidates_later_checkpoints() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let original = large_snapshot(300, Some(5));
    let edited = large_snapshot(300, None);
    renderer.document(&original, &[0], 100, false, "")?;
    let expected = Renderer::new(Config::default())?.document(&edited, &[0], 100, false, "")?;
    // When
    let actual = renderer.document(&edited, &[0], 100, false, "")?;
    // Then
    assert_eq!(rendered_lines(&actual), rendered_lines(&expected));
    Ok(())
}

#[test]
fn the_full_basic_preview_includes_matches_beyond_the_initial_viewport() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let file = language_snapshot("long.txt", &("start ".repeat(100) + "needle"));
    let mut updates = Vec::new();
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &file,
            selected: &[0],
            width: 24,
            split: false,
            query: "needle",
            preview_rows: 16,
        },
        &Cancellation::default(),
        |update| {
            updates.push(update);
            updates.len() < 2
        },
    );
    // Then
    assert_eq!(result, Err(Cancelled));
    assert_eq!(updates.len(), 2);
    assert!(updates[0].document.partial);
    assert!(updates[0].document.hits.is_empty());
    assert!(!updates[1].document.partial);
    assert!(!updates[1].finished);
    assert_eq!(updates[1].document.hits.len(), 1);
    assert_eq!(renderer.stats().syntax_lines, 0);
    Ok(())
}

#[test]
fn extending_a_final_line_without_a_newline_reparses_that_line() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let original = language_snapshot("answer.ts", "const answer =");
    let edited = language_snapshot("answer.ts", "const answer = 42;");
    renderer.document(&original, &[0], 100, false, "")?;
    let expected = Renderer::new(Config::default())?.document(&edited, &[0], 100, false, "")?;
    // When
    let actual = renderer.document(&edited, &[0], 100, false, "")?;
    // Then
    assert_eq!(rendered_lines(&actual), rendered_lines(&expected));
    Ok(())
}

#[test]
fn changing_one_file_preserves_other_files_cached_syntax() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let mut original = language_snapshot("first.ts", "const first = 1;");
    original
        .changes
        .extend(language_snapshot("second.ts", "const second = 2;").changes);
    renderer.document(&original, &[0, 1], 100, false, "")?;
    let mut edited = language_snapshot("first.ts", "const first = 3;");
    edited
        .changes
        .extend(language_snapshot("second.ts", "const second = 2;").changes);
    let before = renderer.stats();
    // When
    let document = renderer.document(&edited, &[0, 1], 100, false, "")?;
    // Then
    assert_eq!(renderer.stats().syntax_lines - before.syntax_lines, 1);
    assert!(renderer.stats().layout_hits > before.layout_hits);
    assert!(source_content(&document).contains("const first = 3;"));
    assert!(source_content(&document).contains("const second = 2;"));
    Ok(())
}

#[test]
fn a_zero_refinement_budget_still_preserves_both_sides_of_the_diff() -> crate::AppResult<()> {
    // Given
    let snapshot = snapshot();
    let mut budget = WorkBudget::new(Duration::ZERO);
    let cancel = Cancellation::default();
    let config = Config::default();
    let prepared = prepare::prepare(
        0,
        Arc::new(snapshot.changes[0].clone()),
        true,
        config.emphasis,
        &mut budget,
        &cancel,
    )?;
    // When
    let document = layout::file(
        &prepared,
        &Visuals::default(),
        &config,
        RenderSpec {
            snapshot: &snapshot,
            selected: &[0],
            width: 80,
            split: true,
            query: "",
            preview_rows: 0,
        },
        None,
        &cancel,
    )?;
    // Then
    assert_eq!(budget.comparisons, 0);
    assert!(source_content(&document).contains("before"));
    assert!(source_content(&document).contains("after"));
    Ok(())
}

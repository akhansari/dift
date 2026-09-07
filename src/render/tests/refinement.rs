use super::*;
use crate::config::Emphasis;

fn replacement(old: &[&str], new: &[&str]) -> Snapshot {
    Snapshot {
        activity: None,
        branch: "main".into(),
        whitespace: Default::default(),
        whitespace_hidden_files: 0,
        tracked_stats: None,
        changes: vec![Change {
            path: "change.txt".into(),
            old_path: None,
            category: Category::Unstaged,
            status: 'M',
            hunks: vec![Hunk {
                header: "@@ replacement @@".into(),
                lines: old
                    .iter()
                    .enumerate()
                    .map(|(i, text)| SourceLine {
                        kind: LineKind::Removed,
                        old: Some(i + 1),
                        new: None,
                        text: (*text).into(),
                        no_newline: false,
                    })
                    .chain(new.iter().enumerate().map(|(i, text)| SourceLine {
                        kind: LineKind::Added,
                        old: None,
                        new: Some(i + 1),
                        text: (*text).into(),
                        no_newline: false,
                    }))
                    .collect(),
            }],
            old_source: Some(Arc::from(old.join("\n") + "\n")),
            new_source: Some(Arc::from(new.join("\n") + "\n")),
            summary: None,
            oversized: false,
        }],
    }
}

fn row_pairs(document: &Document) -> Vec<(Option<usize>, Option<usize>)> {
    document
        .rows
        .iter()
        .filter(|row| !row.anchors.is_empty())
        .map(|row| {
            (
                row.anchors.iter().find(|a| a.old).map(|a| a.number),
                row.anchors.iter().find(|a| !a.old).map(|a| a.number),
            )
        })
        .collect()
}

fn emphasized(document: &Document, old: bool) -> Vec<String> {
    let palette = Palette::new(false);
    let background = if old {
        palette.removed_emphasis
    } else {
        palette.added_emphasis
    };
    document
        .rows
        .iter()
        .flat_map(|row| &row.line.spans)
        .filter(|span| span.style.bg == Some(background))
        .map(|span| span.content.to_string())
        .collect()
}

fn source_lines(document: &Document) -> Vec<&str> {
    document
        .rows
        .iter()
        .flat_map(|row| &row.anchors)
        .map(|anchor| anchor.text.as_ref())
        .collect()
}

fn prepared(snapshot: &Snapshot, remaining: Duration) -> crate::AppResult<Prepared> {
    Ok(prepare::prepare(
        0,
        Arc::new(snapshot.changes[0].clone()),
        true,
        Emphasis::Word,
        &mut WorkBudget::new(remaining),
        &Cancellation::default(),
    )?)
}

fn prepared_pairs(prepared: &Prepared) -> Vec<(Option<usize>, Option<usize>)> {
    prepared
        .blocks
        .iter()
        .zip(&prepared.change.hunks)
        .flat_map(|(blocks, hunk)| {
            blocks.values().flat_map(|block| &block.pairs).map(|pair| {
                (
                    pair.old.and_then(|i| hunk.lines[i].old),
                    pair.new.and_then(|i| hunk.lines[i].new),
                )
            })
        })
        .collect()
}

#[test]
fn several_insertions_do_not_displace_an_edited_line() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &["let answer = 1;", "return answer;"],
        &[
            "// explain",
            "validate();",
            "let answer = 2;",
            "return answer;",
        ],
    );
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "answer")?;
    // Then
    assert_eq!(
        row_pairs(&document),
        vec![
            (None, Some(1)),
            (None, Some(2)),
            (Some(1), Some(3)),
            (Some(2), Some(4))
        ]
    );
    assert_eq!(document.hits.len(), 4);
    Ok(())
}

#[test]
fn several_deletions_do_not_displace_an_edited_line() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &[
            "// explain",
            "validate();",
            "let answer = 1;",
            "return answer;",
        ],
        &["let answer = 2;", "return answer;"],
    );
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(
        row_pairs(&document),
        vec![
            (Some(1), None),
            (Some(2), None),
            (Some(3), Some(1)),
            (Some(4), Some(2))
        ]
    );
    Ok(())
}

#[test]
fn indentation_changes_keep_repeated_braces_aligned() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &[
            "if ready {",
            "    run();",
            "}",
            "if waiting {",
            "    wait();",
            "}",
        ],
        &[
            "    if ready {",
            "        run();",
            "    }",
            "    if waiting {",
            "        wait();",
            "    }",
        ],
    );
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(
        row_pairs(&document),
        vec![
            (Some(1), Some(1)),
            (Some(2), Some(2)),
            (Some(3), Some(3)),
            (Some(4), Some(4)),
            (Some(5), Some(5)),
            (Some(6), Some(6))
        ]
    );
    assert!(!emphasized(&document, false).is_empty());
    Ok(())
}

#[test]
fn repeated_lines_around_insertions_preserve_source_order() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &["start();", "}", "finish();", "}"],
        &["start();", "}", "added();", "}", "finish();", "}"],
    );
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(
        row_pairs(&document),
        vec![
            (Some(1), Some(1)),
            (Some(2), Some(2)),
            (None, Some(3)),
            (None, Some(4)),
            (Some(3), Some(5)),
            (Some(4), Some(6))
        ]
    );
    Ok(())
}

#[test]
fn equally_similar_lines_pair_the_earliest_line_first() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["let item = 1;", "let item = 2;"], &["let item = 3;"]);
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(
        row_pairs(&document),
        vec![(Some(1), Some(1)), (Some(2), None)]
    );
    Ok(())
}

#[test]
fn unrelated_lines_have_empty_opposite_cells() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["alpha"], &["ΩΩΩ"]);
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(row_pairs(&document), vec![(Some(1), None), (None, Some(1))]);
    assert!(emphasized(&document, true).is_empty());
    assert!(emphasized(&document, false).is_empty());
    Ok(())
}

#[test]
fn identifier_edits_emphasize_the_whole_word() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &["const result = user_count;"],
        &["const result = user_total;"],
    );
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(emphasized(&document, true), vec!["user_count"]);
    assert_eq!(emphasized(&document, false), vec!["user_total"]);
    Ok(())
}

#[test]
fn configured_grapheme_emphasis_is_fine_grained_in_unified_view() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["const count = 1000;"], &["const count = 1001;"]);
    let config = Config::parse("appearance { emphasis \"grapheme\"; }")?;
    let mut renderer = Renderer::new(config)?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "count")?;
    // Then
    assert_eq!(emphasized(&document, true), vec!["0"]);
    assert_eq!(emphasized(&document, false), vec!["1"]);
    assert_eq!(document.hits.len(), 2);
    assert_eq!(
        source_lines(&document),
        vec!["const count = 1000;", "const count = 1001;"]
    );
    Ok(())
}

#[test]
fn grapheme_emphasis_is_fine_grained_in_split_view() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &["const result = user_count;"],
        &["const result = user_counts;"],
    );
    let mut renderer = Renderer::new(Config {
        emphasis: Emphasis::Grapheme,
        ..Config::default()
    })?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert!(emphasized(&document, true).is_empty());
    assert_eq!(emphasized(&document, false), vec!["s"]);
    assert_eq!(row_pairs(&document), vec![(Some(1), Some(1))]);
    Ok(())
}

#[test]
fn grapheme_emphasis_keeps_a_combining_mark_with_its_base() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["let cafe\u{301} = 1;"], &["let cafe\u{300} = 1;"]);
    let mut renderer = Renderer::new(Config {
        emphasis: Emphasis::Grapheme,
        ..Config::default()
    })?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "")?;
    // Then
    assert_eq!(emphasized(&document, true), vec!["e\u{301}"]);
    assert_eq!(emphasized(&document, false), vec!["e\u{300}"]);
    Ok(())
}

#[test]
fn grapheme_emphasis_keeps_an_emoji_joiner_sequence_together() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["let icon = \"👩‍💻\";"], &["let icon = \"👨‍💻\";"]);
    let mut renderer = Renderer::new(Config {
        emphasis: Emphasis::Grapheme,
        ..Config::default()
    })?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(emphasized(&document, true), vec!["👩‍💻"]);
    assert_eq!(emphasized(&document, false), vec!["👨‍💻"]);
    Ok(())
}

#[test]
fn grapheme_emphasis_retains_semantic_phrase_boundaries() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["The came."], &["The cat came."]);
    let mut renderer = Renderer::new(Config {
        emphasis: Emphasis::Grapheme,
        ..Config::default()
    })?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "")?;
    // Then
    assert!(emphasized(&document, true).is_empty());
    assert_eq!(emphasized(&document, false), vec!["cat "]);
    Ok(())
}

#[test]
fn disabling_emphasis_preserves_refined_split_alignment_and_search() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &["let answer = 1;", "return answer;"],
        &[
            "// explain",
            "validate();",
            "let answer = 2;",
            "return answer;",
        ],
    );
    let config = Config::parse("appearance { emphasis \"off\"; }")?;
    let mut renderer = Renderer::new(config)?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "answer")?;
    // Then
    assert!(emphasized(&document, true).is_empty());
    assert!(emphasized(&document, false).is_empty());
    assert_eq!(
        row_pairs(&document),
        vec![
            (None, Some(1)),
            (None, Some(2)),
            (Some(1), Some(3)),
            (Some(2), Some(4))
        ]
    );
    assert_eq!(document.hits.len(), 4);
    Ok(())
}

#[test]
fn disabling_emphasis_preserves_unified_source_order() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["let answer = 1;"], &["validate();", "let answer = 2;"]);
    let mut renderer = Renderer::new(Config {
        emphasis: Emphasis::Off,
        ..Config::default()
    })?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "")?;
    // Then
    assert!(emphasized(&document, true).is_empty());
    assert!(emphasized(&document, false).is_empty());
    assert_eq!(
        source_lines(&document),
        vec!["let answer = 1;", "validate();", "let answer = 2;"]
    );
    assert!(
        document
            .rows
            .iter()
            .flat_map(|row| &row.line.spans)
            .any(|span| span.content.contains("let answer = 1;")
                && span.style.bg == Some(Palette::new(false).removed))
    );
    assert!(
        document
            .rows
            .iter()
            .flat_map(|row| &row.line.spans)
            .any(|span| span.content.contains("let answer = 2;")
                && span.style.bg == Some(Palette::new(false).added))
    );
    Ok(())
}

#[test]
fn semantic_cleanup_emphasizes_the_inserted_phrase_at_a_word_boundary() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["The came."], &["The cat came."]);
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "")?;
    // Then
    assert!(emphasized(&document, true).is_empty());
    assert_eq!(emphasized(&document, false), vec!["cat "]);
    Ok(())
}

#[test]
fn unicode_emphasis_uses_original_byte_boundaries() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["let café = \"été\";"], &["let café = \"hiver\";"]);
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "café")?;
    // Then
    assert_eq!(emphasized(&document, true), vec!["été"]);
    assert_eq!(emphasized(&document, false), vec!["hiver"]);
    assert_eq!(document.hits.len(), 2);
    Ok(())
}

#[test]
fn whitespace_edits_remain_visible_after_alignment_normalization() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["run();"], &["  run();  "]);
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "")?;
    // Then
    assert_eq!(source_lines(&document), vec!["run();", "  run();  "]);
    assert_eq!(emphasized(&document, false), vec!["  ", "  "]);
    Ok(())
}

#[test]
fn an_empty_added_line_is_preserved_without_emphasis() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&[], &[""]);
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "")?;
    // Then
    assert_eq!(row_pairs(&document), vec![(None, Some(1))]);
    assert!(emphasized(&document, false).is_empty());
    Ok(())
}

#[test]
fn unified_layout_keeps_patch_order_after_block_alignment() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &["let answer = 1;", "return answer;"],
        &[
            "// explain",
            "validate();",
            "let answer = 2;",
            "return answer;",
        ],
    );
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "")?;
    // Then
    assert_eq!(
        row_pairs(&document),
        vec![
            (Some(1), None),
            (Some(2), None),
            (None, Some(1)),
            (None, Some(2)),
            (None, Some(3)),
            (None, Some(4))
        ]
    );
    assert_eq!(
        source_lines(&document),
        vec![
            "let answer = 1;",
            "return answer;",
            "// explain",
            "validate();",
            "let answer = 2;",
            "return answer;"
        ]
    );
    Ok(())
}

#[test]
fn exhausted_alignment_budget_preserves_positional_rows() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(
        &["let answer = 1;"],
        &["// explain", "validate();", "let answer = 2;"],
    );
    // When
    let prepared = prepared(&snapshot, Duration::ZERO)?;
    // Then
    assert_eq!(
        prepared_pairs(&prepared),
        vec![(Some(1), Some(1)), (None, Some(2)), (None, Some(3))]
    );
    Ok(())
}

#[test]
fn a_large_block_uses_positional_rows_without_losing_its_tail() -> crate::AppResult<()> {
    // Given
    let long = "a".repeat(33 * 1024);
    let snapshot = replacement(&[&long, "old tail"], &[&long, "new tail", "extra"]);
    // When
    let prepared = prepared(&snapshot, Duration::from_millis(20))?;
    // Then
    assert_eq!(
        prepared_pairs(&prepared),
        vec![(Some(1), Some(1)), (Some(2), Some(2)), (None, Some(3))]
    );
    Ok(())
}

#[test]
fn too_many_candidate_pairs_preserve_anchors_and_fall_back_between_them() -> crate::AppResult<()> {
    // Given
    let old: Vec<_> = std::iter::once("anchor")
        .chain(std::iter::repeat_n("old", 65))
        .collect();
    let new: Vec<_> = std::iter::once("anchor")
        .chain(std::iter::repeat_n("new", 65))
        .collect();
    let snapshot = replacement(&old, &new);
    // When
    let prepared = prepared(&snapshot, Duration::from_millis(20))?;
    // Then
    assert_eq!(prepared_pairs(&prepared).len(), 66);
    assert_eq!(prepared_pairs(&prepared).first(), Some(&(Some(1), Some(1))));
    assert_eq!(
        prepared_pairs(&prepared).last(),
        Some(&(Some(66), Some(66)))
    );
    Ok(())
}

#[test]
fn lines_above_the_emphasis_limit_keep_plain_diff_styling() -> crate::AppResult<()> {
    // Given
    let old = "a".repeat(4097);
    let new = "b".repeat(4097);
    let snapshot = replacement(&[&old], &[&new]);
    // When
    let prepared = prepared(&snapshot, Duration::from_millis(20))?;
    // Then
    assert_eq!(prepared_pairs(&prepared), vec![(Some(1), Some(1))]);
    assert!(
        prepared.blocks[0]
            .values()
            .all(|block| block.emphasis.is_empty())
    );
    Ok(())
}

#[test]
fn an_expired_block_deadline_uses_complete_positional_rows() -> crate::AppResult<()> {
    // Given
    let snapshot = replacement(&["let answer = 1;"], &["validate();", "let answer = 2;"]);
    let lines = &snapshot.changes[0].hunks[0].lines;
    let deadline = std::time::Instant::now();
    // When
    let pairs = refine::align(
        lines,
        &[0],
        &[1, 2],
        refine::ALGORITHMS,
        deadline,
        &Cancellation::default(),
    )?;
    // Then
    assert_eq!(
        pairs,
        vec![
            refine::Pair {
                old: Some(0),
                new: Some(1)
            },
            refine::Pair {
                old: None,
                new: Some(2)
            }
        ]
    );
    Ok(())
}

#[test]
fn an_expired_emphasis_deadline_returns_plain_styling_on_both_sides() -> crate::AppResult<()> {
    // Given
    let deadline = std::time::Instant::now();
    // When
    let ranges = refine::emphasize(
        "let answer = 1;",
        "let answer = 2;",
        refine::ALGORITHMS.inline,
        Emphasis::Word,
        deadline,
        &Cancellation::default(),
    )?;
    // Then
    assert_eq!(ranges, (vec![], vec![]));
    Ok(())
}

#[test]
fn an_expired_grapheme_deadline_returns_plain_styling_on_both_sides() -> crate::AppResult<()> {
    // Given
    let deadline = std::time::Instant::now();
    // When
    let ranges = refine::emphasize(
        "let answer = 1000;",
        "let answer = 1001;",
        refine::ALGORITHMS.inline,
        Emphasis::Grapheme,
        deadline,
        &Cancellation::default(),
    )?;
    // Then
    assert_eq!(ranges, (vec![], vec![]));
    Ok(())
}

#[test]
fn oversized_grapheme_inputs_return_plain_styling() -> crate::AppResult<()> {
    // Given
    let old = "a".repeat(4096) + "0";
    let new = "a".repeat(4096) + "1";
    // When
    let ranges = refine::emphasize(
        &old,
        &new,
        refine::ALGORITHMS.inline,
        Emphasis::Grapheme,
        std::time::Instant::now() + Duration::from_secs(1),
        &Cancellation::default(),
    )?;
    // Then
    assert_eq!(ranges, (vec![], vec![]));
    Ok(())
}

#[test]
fn obsolete_block_refinement_is_cancelled_before_producing_rows() {
    // Given
    let snapshot = replacement(&["let answer = 1;"], &["let answer = 2;"]);
    let cancel = Cancellation::new(Arc::new(std::sync::atomic::AtomicU64::new(1)), 0);
    // When
    let result = refine::align(
        &snapshot.changes[0].hunks[0].lines,
        &[0],
        &[1],
        refine::ALGORITHMS,
        std::time::Instant::now() + Duration::from_millis(3),
        &cancel,
    );
    // Then
    assert_eq!(result, Err(Cancelled));
}

#[test]
fn obsolete_word_refinement_is_cancelled_before_producing_emphasis() {
    // Given
    let cancel = Cancellation::new(Arc::new(std::sync::atomic::AtomicU64::new(1)), 0);
    // When
    let result = refine::emphasize(
        "let answer = 1;",
        "let answer = 2;",
        refine::ALGORITHMS.inline,
        Emphasis::Word,
        std::time::Instant::now() + Duration::from_millis(3),
        &cancel,
    );
    // Then
    assert_eq!(result, Err(Cancelled));
}

fn unterminated_replacement() -> Snapshot {
    let mut snapshot = replacement(&["let answer = 1;"], &["let answer = 2;"]);
    let change = &mut snapshot.changes[0];
    change.old_source = Some(Arc::from("let answer = 1;"));
    change.new_source = Some(Arc::from("let answer = 2;"));
    change.hunks[0]
        .lines
        .iter_mut()
        .for_each(|line| line.no_newline = true);
    snapshot
}

#[test]
fn refinement_keeps_missing_newline_markers_out_of_source_and_search() -> crate::AppResult<()> {
    // Given
    let snapshot = unterminated_replacement();
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "No newline")?;
    // Then
    assert_eq!(
        source_lines(&document),
        vec!["let answer = 1;", "let answer = 2;"]
    );
    assert_eq!(emphasized(&document, true), vec!["1"]);
    assert_eq!(emphasized(&document, false), vec!["2"]);
    assert!(document.hits.is_empty());
    assert!(
        document
            .rows
            .iter()
            .any(|row| row.line.to_string().contains("No newline at end of file"))
    );
    Ok(())
}

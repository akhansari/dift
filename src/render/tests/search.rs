use super::*;

#[test]
fn separate_occurrences_have_separate_column_ranges() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = text_snapshot("needle needle");
    // When
    let document = renderer.document(&snapshot, &[0], 80, false, "needle")?;
    // Then
    assert_eq!(document.hits.len(), 2);
    assert_eq!(
        document.hits[0].fragments,
        vec![HitFragment {
            row: 4,
            columns: 14..20
        }]
    );
    assert_eq!(
        document.hits[1].fragments,
        vec![HitFragment {
            row: 4,
            columns: 21..27
        }]
    );
    Ok(())
}

#[test]
fn split_occurrences_on_both_sides_have_distinct_ranges() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = snapshot();
    // When
    let document = renderer.document(&snapshot, &[0], 80, true, "let")?;
    // Then
    assert_eq!(document.hits.len(), 2);
    assert_eq!(
        document.hits[0].fragments,
        vec![HitFragment {
            row: 4,
            columns: 8..11
        }]
    );
    assert_eq!(
        document.hits[1].fragments,
        vec![HitFragment {
            row: 4,
            columns: 48..51
        }]
    );
    Ok(())
}

#[test]
fn unicode_matches_use_terminal_columns_after_expanded_tabs() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = text_snapshot("\tÉ界e\u{301}");
    // When
    let document = renderer.document(&snapshot, &[0], 80, false, "é界e\u{301}")?;
    // Then
    assert_eq!(document.hits.len(), 1);
    assert_eq!(
        document.hits[0].fragments,
        vec![HitFragment {
            row: 4,
            columns: 18..22
        }]
    );
    Ok(())
}

#[test]
fn a_matching_tab_keeps_all_of_its_wrapped_display_cells() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = text_snapshot("123456789\t");
    // When
    let document = renderer.document(&snapshot, &[0], 18, false, "\t")?;
    // Then
    assert_eq!(document.hits.len(), 1);
    assert_eq!(
        document.hits[0].fragments,
        vec![
            HitFragment {
                row: 4,
                columns: 17..18
            },
            HitFragment {
                row: 5,
                columns: 8..10
            },
        ]
    );
    Ok(())
}

#[test]
fn combining_documents_offsets_match_fragments_with_their_file() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = two_file_snapshot();
    // When
    let document = renderer.document(&snapshot, &[0, 1], 80, false, "needle")?;
    // Then
    assert_eq!(document.hits.len(), 1);
    assert_eq!(document.hits[0].row, 9);
    assert_eq!(
        document.hits[0].fragments,
        vec![HitFragment {
            row: 9,
            columns: 20..26
        }]
    );
    Ok(())
}

#[test]
fn prefix_preview_clips_wrapped_match_fragments() -> crate::AppResult<()> {
    // Given
    let mut renderer = Renderer::new(Config::default())?;
    let snapshot = text_snapshot("123456789needle");
    let mut preview = None;
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &snapshot,
            selected: &[0],
            width: 19,
            split: false,
            query: "needle",
            preview_rows: 5,
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
    assert_eq!(document.hits.len(), 1);
    assert_eq!(
        document.hits[0].fragments,
        vec![HitFragment {
            row: 4,
            columns: 17..19
        }]
    );
    Ok(())
}

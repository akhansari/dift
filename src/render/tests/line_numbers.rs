use super::*;

fn spec(snapshot: &Snapshot, split: bool, width: usize, visible: bool) -> RenderSpec<'_> {
    RenderSpec {
        snapshot,
        selected: &[0],
        width,
        split,
        show_line_numbers: visible,
        query: "after",
        preview_rows: 0,
    }
}

fn render(renderer: &mut Renderer, spec: RenderSpec<'_>) -> Result<Arc<Document>, Cancelled> {
    let mut document = Arc::new(Document::default());
    renderer.render_progressive(spec, &Cancellation::default(), |update| {
        document = update.document;
        true
    })?;
    Ok(document)
}

#[test]
fn hidden_numbers_keep_markers_and_source_coordinates() -> crate::AppResult<()> {
    // Given
    let snapshot = snapshot();
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = render(&mut renderer, spec(&snapshot, false, 80, false))?;
    // Then
    assert_eq!(
        styled_source_lines(&document)
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect::<Vec<_>>(),
        ["- let before = 1;", "+ let after = 2;"]
    );
    let hit = &document.hits[0];
    let row = &document.rows[hit.row];
    assert_eq!(hit.fragments[0].columns, 6..11);
    assert_eq!(row.code[0].columns.start, 2);
    assert_eq!(row.anchors[0].number, 1);
    assert!(!row.anchors[0].old);
    assert_eq!(row.anchors[0].byte, 0);
    Ok(())
}

#[test]
fn split_layout_hides_numbers_on_both_sides() -> crate::AppResult<()> {
    // Given
    let snapshot = snapshot();
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = render(&mut renderer, spec(&snapshot, true, 80, false))?;
    // Then
    let rows = styled_source_lines(&document);
    assert_eq!(rows.len(), 1);
    let text = rows[0].to_string();
    assert!(text.starts_with("- let before = 1;"));
    assert!(text.contains("│+ let after = 2;"));
    Ok(())
}

#[test]
fn hiding_numbers_reclaims_wrapping_width() -> crate::AppResult<()> {
    // Given
    let snapshot = text_snapshot("abcdefghijklmnopqrstuvwx");
    let mut renderer = Renderer::new(Config::default())?;
    let visible = render(&mut renderer, spec(&snapshot, false, 28, true))?;
    // When
    let hidden = render(&mut renderer, spec(&snapshot, false, 28, false))?;
    // Then
    assert_eq!(styled_source_lines(&visible).len(), 2);
    assert_eq!(styled_source_lines(&hidden).len(), 1);
    assert_eq!(
        source_content(&hidden).trim_end(),
        "abcdefghijklmnopqrstuvwx"
    );
    Ok(())
}

#[test]
fn toggling_back_restores_cached_number_columns() -> crate::AppResult<()> {
    // Given
    let snapshot = snapshot();
    let mut renderer = Renderer::new(Config::default())?;
    let visible = render(&mut renderer, spec(&snapshot, false, 80, true))?;
    render(&mut renderer, spec(&snapshot, false, 80, false))?;
    // When
    let restored = render(&mut renderer, spec(&snapshot, false, 80, true))?;
    // Then
    assert_eq!(
        styled_source_lines(&restored),
        styled_source_lines(&visible)
    );
    assert!(
        restored
            .rows
            .iter()
            .filter(|row| !row.code.is_empty())
            .all(|row| row.code[0].columns.start > 2)
    );
    Ok(())
}

#[test]
fn narrow_hidden_rows_keep_continuation_padding() -> crate::AppResult<()> {
    // Given
    let snapshot = text_snapshot("abcdef");
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = render(&mut renderer, spec(&snapshot, false, 6, false))?;
    // Then
    assert_eq!(
        styled_source_lines(&document)
            .iter()
            .map(|line| line.to_string().trim_end().to_owned())
            .collect::<Vec<_>>(),
        ["+ abcd", "  ef"]
    );
    Ok(())
}

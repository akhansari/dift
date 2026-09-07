use super::*;
use std::path::Path;

fn source(kind: LineKind, old: Option<usize>, new: Option<usize>) -> SourceLine {
    SourceLine {
        kind,
        old,
        new,
        text: "source line that wraps in a narrow pane".into(),
        no_newline: false,
    }
}

fn hunk(lines: Vec<SourceLine>) -> Hunk {
    Hunk {
        header: "@@ example @@".into(),
        lines,
    }
}

fn changes() -> Vec<Hunk> {
    vec![
        hunk(vec![
            source(LineKind::Context, Some(8), Some(8)),
            source(LineKind::Removed, Some(9), None),
            source(LineKind::Added, None, Some(9)),
            source(LineKind::Context, Some(10), Some(10)),
            source(LineKind::Context, Some(11), Some(11)),
            source(LineKind::Context, Some(12), Some(12)),
        ]),
        hunk(vec![
            source(LineKind::Context, Some(49), Some(49)),
            source(LineKind::Added, None, Some(50)),
            source(LineKind::Context, Some(50), Some(51)),
        ]),
    ]
}

fn fixture(hunks: Vec<Hunk>) -> Snapshot {
    let mut snapshot = snapshot();
    snapshot.changes[0].hunks = hunks;
    snapshot
}

fn document(hunks: Vec<Hunk>, split: bool, width: usize) -> crate::AppResult<Arc<Document>> {
    Ok(Renderer::new(Config::default())?.document(&fixture(hunks), &[0], width, split, "")?)
}

#[test]
fn the_topmost_hunk_targets_its_first_change_after_context() -> crate::AppResult<()> {
    // Given
    let document = document(changes(), false, 100)?;
    // When
    let line = document.visible_hunk_line(Path::new("main.rs"), 0, 20);
    // Then
    assert_eq!(line, Some(9));
    Ok(())
}

#[test]
fn scrolling_past_the_first_change_still_targets_that_change() -> crate::AppResult<()> {
    // Given
    let document = document(changes(), false, 100)?;
    // When
    let line = document.visible_hunk_line(Path::new("main.rs"), 5, 5);
    // Then
    assert_eq!(line, Some(9));
    Ok(())
}

#[test]
fn a_hunk_hidden_behind_the_sticky_header_is_not_visible() -> crate::AppResult<()> {
    // Given
    let document = document(changes(), false, 100)?;
    // When
    let line = document.visible_hunk_line(Path::new("main.rs"), 8, 5);
    // Then
    assert_eq!(line, Some(50));
    Ok(())
}

#[test]
fn split_and_wrapped_rows_preserve_the_source_destination() -> crate::AppResult<()> {
    // Given
    let document = document(changes(), true, 40)?;
    // When
    let line = document.visible_hunk_line(Path::new("main.rs"), 6, 8);
    // Then
    assert_eq!(line, Some(9));
    Ok(())
}

macro_rules! deletion_test {
    ($name:ident, $lines:expr, $expected:expr) => {
        #[test]
        fn $name() -> crate::AppResult<()> {
            // Given
            let document = document(vec![hunk($lines)], false, 100)?;
            // When
            let line = document.visible_hunk_line(Path::new("main.rs"), 0, 20);
            // Then
            assert_eq!(line, Some($expected));
            Ok(())
        }
    };
}
deletion_test!(
    deletions_use_the_next_surviving_line,
    vec![
        source(LineKind::Removed, Some(9), None),
        source(LineKind::Context, Some(10), Some(9)),
    ],
    9
);
deletion_test!(
    deletions_at_eof_use_the_preceding_surviving_line,
    vec![
        source(LineKind::Context, Some(8), Some(8)),
        source(LineKind::Removed, Some(9), None),
    ],
    8
);
deletion_test!(
    a_fully_deleted_hunk_falls_back_to_line_one,
    vec![source(LineKind::Removed, Some(1), None),],
    1
);

#[test]
fn a_header_only_view_has_no_visible_hunk() -> crate::AppResult<()> {
    // Given
    let document = document(changes(), false, 100)?;
    // When
    let line = document.visible_hunk_line(Path::new("main.rs"), 0, 3);
    // Then
    assert_eq!(line, None);
    Ok(())
}

#[test]
fn positions_do_not_cross_into_another_file() -> crate::AppResult<()> {
    // Given
    let document = document(changes(), false, 100)?;
    // When
    let line = document.visible_hunk_line(Path::new("other.rs"), 0, 20);
    // Then
    assert_eq!(line, None);
    Ok(())
}

#[test]
fn previews_keep_the_first_change_even_before_its_row_is_rendered() -> crate::AppResult<()> {
    // Given
    let snapshot = fixture(changes());
    let mut renderer = Renderer::new(Config::default())?;
    let mut preview = None;
    // When
    let result = renderer.render_progressive(
        RenderSpec {
            snapshot: &snapshot,
            selected: &[0],
            width: 100,
            split: false,
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
    assert_eq!(
        document.visible_hunk_line(Path::new("main.rs"), 0, 20),
        Some(9)
    );
    Ok(())
}

#[test]
fn cached_layouts_retain_hunk_positions() -> crate::AppResult<()> {
    // Given
    let snapshot = fixture(changes());
    let mut renderer = Renderer::new(Config::default())?;
    renderer.document(&snapshot, &[0], 100, false, "")?;
    // When
    let document = renderer.document(&snapshot, &[0], 100, false, "")?;
    // Then
    assert_eq!(
        document.visible_hunk_line(Path::new("main.rs"), 8, 5),
        Some(50)
    );
    Ok(())
}

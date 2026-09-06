use super::*;
use dift::config::{Config, Whitespace};
use dift::render::Renderer;

fn edited(old: &str, new: &str) -> AppResult<Fixture> {
    let fixture = Fixture::new()?;
    fixture.write("text.txt", old)?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "initial"])?;
    fixture.write("text.txt", new)?;
    Ok(fixture)
}

macro_rules! comparison_case {
    ($name:ident, $old:expr, $new:expr, $mode:ident, $visible:expr, $hidden:expr) => {
        #[test]
        fn $name() -> AppResult<()> {
            // Given
            let fixture = edited($old, $new)?;
            // When
            let snapshot = fixture.repo()?.snapshot_with_whitespace(
                &Mode::Watch,
                Limits::default(),
                &BTreeSet::new(),
                Whitespace::$mode,
            )?;
            // Then
            assert_eq!(snapshot.changes.len(), $visible);
            assert_eq!(snapshot.whitespace_hidden_files, $hidden);
            assert_eq!(snapshot.whitespace, Whitespace::$mode);
            Ok(())
        }
    };
}

comparison_case!(exact_keeps_spacing_edits, "a b\n", "a  b\n", Exact, 1, 0);
comparison_case!(
    ignore_changes_hides_spacing_tabs_and_trailing_spaces,
    "  a b\n",
    "\ta  b \t\n",
    IgnoreChanges,
    0,
    1
);
comparison_case!(
    ignore_changes_keeps_a_removed_separator,
    "a b\n",
    "ab\n",
    IgnoreChanges,
    1,
    0
);
comparison_case!(
    ignore_all_hides_a_removed_separator,
    "a b\n",
    "ab\n",
    IgnoreAll,
    0,
    1
);
comparison_case!(
    ignore_changes_keeps_new_indentation,
    "a\n",
    "  a\n",
    IgnoreChanges,
    1,
    0
);
comparison_case!(
    ignore_all_hides_new_indentation,
    "a\n",
    "  a\n",
    IgnoreAll,
    0,
    1
);
comparison_case!(
    ignore_changes_hides_crlf_conversion,
    "a\r\n",
    "a\n",
    IgnoreChanges,
    0,
    1
);
comparison_case!(
    ignore_changes_hides_a_final_newline_edit,
    "a\n",
    "a",
    IgnoreChanges,
    0,
    1
);
comparison_case!(
    ignore_all_keeps_added_blank_lines,
    "a\n",
    "a\n\n",
    IgnoreAll,
    1,
    0
);
comparison_case!(
    ignore_all_keeps_unicode_nonbreaking_space_changes,
    "a\u{a0}b\n",
    "ab\n",
    IgnoreAll,
    1,
    0
);

#[test]
fn hidden_paths_are_counted_once_across_staged_and_unstaged_changes() -> AppResult<()> {
    // Given
    let fixture = edited("a b\n", "a  b\n")?;
    fixture.git(&["add", "text.txt"])?;
    fixture.write("text.txt", "a   b\n")?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert!(snapshot.changes.is_empty());
    assert_eq!(snapshot.whitespace_hidden_files, 1);
    Ok(())
}

#[test]
fn a_visible_category_prevents_counting_its_path_as_hidden() -> AppResult<()> {
    // Given
    let fixture = edited("a b\n", "a  b\n")?;
    fixture.git(&["add", "text.txt"])?;
    fixture.write("text.txt", "new content\n")?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].category, Category::Unstaged);
    assert_eq!(snapshot.whitespace_hidden_files, 0);
    Ok(())
}

#[test]
fn untracked_and_gitignored_paths_are_not_counted_as_whitespace_hidden() -> AppResult<()> {
    // Given
    let fixture = edited("a b\n", "a  b\n")?;
    fixture.write(".git/info/exclude", "ignored.txt\n")?;
    fixture.write("ignored.txt", "ignored\n")?;
    fixture.write("new.txt", " \n")?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].path, Path::new("new.txt"));
    assert_eq!(snapshot.whitespace_hidden_files, 1);
    Ok(())
}

#[test]
fn renamed_files_remain_visible_when_their_text_changes_are_ignored() -> AppResult<()> {
    // Given
    let fixture = edited(
        "one two\nthree four\nfive six\n",
        "one  two\nthree four\nfive six\n",
    )?;
    fixture.git(&["mv", "text.txt", "renamed.txt"])?;
    fixture.git(&["add", "renamed.txt"])?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(
        snapshot.changes[0].old_path.as_deref(),
        Some(Path::new("text.txt"))
    );
    assert_eq!(snapshot.changes[0].status, 'R');
    assert!(snapshot.changes[0].hunks.is_empty());
    assert_eq!(snapshot.whitespace_hidden_files, 0);
    Ok(())
}

#[test]
fn whitespace_only_deleted_and_added_files_remain_visible() -> AppResult<()> {
    // Given
    let fixture = edited(" \n", " \n")?;
    fixture.git(&["rm", "text.txt"])?;
    fixture.write("added.txt", "\t\n")?;
    fixture.git(&["add", "added.txt"])?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreAll,
    )?;
    // Then
    assert_eq!(snapshot.changes.len(), 2);
    assert_eq!(snapshot.changes[0].status, 'A');
    assert_eq!(snapshot.changes[1].status, 'D');
    assert_eq!(snapshot.whitespace_hidden_files, 0);
    Ok(())
}

#[test]
fn permission_changes_survive_ignored_text_changes() -> AppResult<()> {
    // Given
    let fixture = edited("a b\n", "a  b\n")?;
    fixture.git(&["config", "core.filemode", "false"])?;
    fixture.git(&["add", "text.txt"])?;
    fixture.git(&["update-index", "--chmod=+x", "text.txt"])?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert!(snapshot.changes[0].hunks.is_empty());
    assert!(
        snapshot.changes[0]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("new mode 100755"))
    );
    assert_eq!(snapshot.whitespace_hidden_files, 0);
    Ok(())
}

#[test]
fn binary_edits_are_not_suppressed_by_whitespace_comparison() -> AppResult<()> {
    // Given
    let fixture = edited("a\0 b", "a\0  b")?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreAll,
    )?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(
        snapshot.changes[0].summary.as_deref(),
        Some("Binary file changed")
    );
    assert_eq!(snapshot.whitespace_hidden_files, 0);
    Ok(())
}

#[test]
fn submodules_keep_their_metadata_when_whitespace_is_ignored() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let nested = Fixture::committed()?;
    fixture.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        &nested.path.to_string_lossy(),
        "module",
    ])?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreAll,
    )?;
    // Then
    assert_eq!(snapshot.changes[1].path, Path::new("module"));
    assert!(
        snapshot.changes[1]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.starts_with("Submodule change"))
    );
    assert!(snapshot.changes[1].hunks.is_empty());
    assert_eq!(snapshot.whitespace_hidden_files, 0);
    Ok(())
}

#[test]
fn hidden_files_do_not_require_loading_their_full_sources() -> AppResult<()> {
    // Given
    let fixture = edited("a b\n", "a  b\n")?;
    let limits = Limits { bytes: 1, lines: 1 };
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        limits,
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert!(snapshot.changes.is_empty());
    assert_eq!(snapshot.whitespace_hidden_files, 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn names_only_counting_handles_quotes_and_newlines_in_paths() -> AppResult<()> {
    // Given
    let fixture = edited("a b\n", "a b\n")?;
    fixture.git(&["mv", "text.txt", "a ' quoted\npath.txt"])?;
    fixture.git(&["commit", "-m", "rename"])?;
    fixture.write("a ' quoted\npath.txt", "a  b\n")?;
    // When
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert!(snapshot.changes.is_empty());
    assert_eq!(snapshot.whitespace_hidden_files, 1);
    Ok(())
}

fn committed_spacing_edit() -> AppResult<Fixture> {
    let fixture = edited("a b\n", "a  b\n")?;
    fixture.git(&["commit", "-am", "spacing"])?;
    Ok(fixture)
}

#[test]
fn revision_comparisons_apply_whitespace_filtering() -> AppResult<()> {
    // Given
    let fixture = committed_spacing_edit()?;
    let repo = fixture.repo()?;
    let mode = repo.comparison("HEAD^", "HEAD", false)?;
    // When
    let snapshot = repo.snapshot_with_whitespace(
        &mode,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert!(snapshot.changes.is_empty());
    assert_eq!(snapshot.whitespace_hidden_files, 1);
    Ok(())
}

#[test]
fn reviews_apply_whitespace_filtering() -> AppResult<()> {
    // Given
    let fixture = committed_spacing_edit()?;
    let repo = fixture.repo()?;
    let mode = repo.comparison("HEAD^", "HEAD", true)?;
    // When
    let snapshot = repo.snapshot_with_whitespace(
        &mode,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert!(snapshot.changes.is_empty());
    assert_eq!(snapshot.whitespace_hidden_files, 1);
    Ok(())
}

#[test]
fn split_context_retains_each_sides_original_text_and_anchors() -> AppResult<()> {
    // Given
    let fixture = edited("  let café = 1;\nbefore\n", "\tlet  café = 1;\nafter\n")?;
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, true, "café")?;
    // Then
    let anchors: Vec<_> = document
        .rows
        .iter()
        .flat_map(|row| &row.anchors)
        .filter(|anchor| anchor.number == 1)
        .map(|anchor| (anchor.old, anchor.text.as_ref(), anchor.byte))
        .collect();
    assert_eq!(
        anchors,
        vec![(true, "  let café = 1;", 0), (false, "\tlet  café = 1;", 0)]
    );
    assert_eq!(document.hits.len(), 1);
    Ok(())
}

#[test]
fn unified_context_retains_the_context_selected_by_git() -> AppResult<()> {
    // Given
    let fixture = edited("  context\nbefore\n", "\tcontext\nafter\n")?;
    let snapshot = fixture.repo()?.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    let mut renderer = Renderer::new(Config::default())?;
    // When
    let document = renderer.document(&snapshot, &[0], 120, false, "context")?;
    // Then
    let context: Vec<_> = document
        .rows
        .iter()
        .flat_map(|row| &row.anchors)
        .filter(|anchor| anchor.number == 1)
        .map(|anchor| anchor.text.as_ref())
        .collect();
    assert_eq!(context, vec!["\tcontext"]);
    assert_eq!(document.hits.len(), 1);
    Ok(())
}

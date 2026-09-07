use super::*;
use dift::{config::Whitespace, model::LineStats};

#[test]
fn staged_and_unstaged_edits_are_counted_even_when_they_cancel_out() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let original = fs::read(fixture.path.join("src/main.rs"))?;
    fixture.write("src/main.rs", "staged\n")?;
    fixture.git(&["add", "."])?;
    fixture.write("src/main.rs", original)?;
    fixture.write("untracked.txt", "not\ncounted\n")?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.tracked_stats,
        Some(LineStats {
            added: 4,
            removed: 4
        })
    );
    Ok(())
}

#[test]
fn an_empty_repository_has_collected_zero_totals() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.tracked_stats, Some(LineStats::default()));
    Ok(())
}

#[test]
fn untracked_files_do_not_contribute_to_totals() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("new.txt", "one\ntwo\n")?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.tracked_stats, Some(LineStats::default()));
    assert_eq!(snapshot.changes.len(), 1);
    Ok(())
}

#[test]
fn staged_files_are_counted_before_the_first_commit() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("new.txt", "one\ntwo")?;
    fixture.git(&["add", "."])?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.tracked_stats,
        Some(LineStats {
            added: 2,
            removed: 0
        })
    );
    Ok(())
}

#[test]
fn omitted_previews_still_contribute_complete_line_counts() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "one\ntwo\n")?;
    // When
    let snapshot = fixture.repo()?.snapshot(
        &Mode::Watch,
        Limits { bytes: 1, lines: 1 },
        &BTreeSet::new(),
    )?;
    // Then
    assert_eq!(
        snapshot.tracked_stats,
        Some(LineStats {
            added: 2,
            removed: 3
        })
    );
    assert!(snapshot.changes[0].oversized);
    assert!(snapshot.changes[0].hunks.is_empty());
    Ok(())
}

#[test]
fn loading_an_oversized_file_preserves_the_totals() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "one\ntwo\n")?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let limits = Limits { bytes: 1, lines: 1 };
    let initial = collector.snapshot(&Mode::Watch, limits, &BTreeSet::new())?;
    let loaded = BTreeSet::from([PathBuf::from("src/main.rs")]);
    // When
    let snapshot = collector.snapshot(&Mode::Watch, limits, &loaded)?;
    // Then
    assert_eq!(snapshot.tracked_stats, initial.tracked_stats);
    assert!(!snapshot.changes[0].oversized);
    Ok(())
}

#[cfg(unix)]
#[test]
fn renamed_paths_with_tabs_and_newlines_keep_their_statistics() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let renamed = "renamed\t界\nfile.rs";
    fixture.git(&["mv", "src/main.rs", renamed])?;
    fixture.write(
        renamed,
        "fn main() {\n    println!(\"before\");\n}\n// extra\n",
    )?;
    fixture.git(&["add", "."])?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.tracked_stats,
        Some(LineStats {
            added: 1,
            removed: 0
        })
    );
    assert_eq!(snapshot.changes[0].path, Path::new(renamed));
    assert_eq!(
        snapshot.changes[0].old_path.as_deref(),
        Some(Path::new("src/main.rs"))
    );
    Ok(())
}

#[test]
fn binary_files_do_not_contribute_line_counts() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("binary", b"one\0two\n")?;
    fixture.git(&["add", "."])?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.tracked_stats, Some(LineStats::default()));
    assert_eq!(snapshot.changes.len(), 1);
    Ok(())
}

#[test]
fn unresolved_conflicts_do_not_contribute_line_counts() -> AppResult<()> {
    // Given
    let fixture = Fixture::conflicted()?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.tracked_stats, Some(LineStats::default()));
    assert_eq!(snapshot.changes[0].status, 'U');
    Ok(())
}

#[test]
fn refreshing_the_whitespace_policy_updates_totals_with_the_snapshot() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("text", "a b\n")?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "initial"])?;
    fixture.write("text", "a  b\n")?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let initial = collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // When
    let snapshot = collector.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        Whitespace::IgnoreChanges,
    )?;
    // Then
    assert_eq!(
        initial.tracked_stats,
        Some(LineStats {
            added: 1,
            removed: 1
        })
    );
    assert_eq!(snapshot.tracked_stats, Some(LineStats::default()));
    assert!(snapshot.changes.is_empty());
    Ok(())
}

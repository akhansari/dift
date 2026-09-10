use dift::{
    AppResult,
    git::{Collector, GitError, Limits, Mode, Repository},
    model::{Category, LineKind, LineStats},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

#[path = "git_workflows/activity.rs"]
mod activity;
#[path = "git_workflows/stage.rs"]
mod stage;
#[path = "git_workflows/stats.rs"]
mod stats;
#[path = "git_workflows/whitespace.rs"]
mod whitespace;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    path: PathBuf,
}
impl Fixture {
    fn new() -> AppResult<Self> {
        let path = std::env::temp_dir().join(format!(
            "dift-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        let fixture = Self { path };
        fixture.git(&["init", "--initial-branch=main"])?;
        Ok(fixture)
    }
    fn write(&self, path: &str, content: impl AsRef<[u8]>) -> AppResult<()> {
        let file = self.path.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, content)?;
        Ok(())
    }
    fn git(&self, args: &[&str]) -> AppResult<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args([
                "-c",
                "user.name=dift Test",
                "-c",
                "user.email=tests@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.hooksPath=/nonexistent-dift-hooks",
            ])
            .args(args)
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Git fixture command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
    fn committed() -> AppResult<Self> {
        let fixture = Self::new()?;
        fixture.write("src/main.rs", "fn main() {\n    println!(\"before\");\n}\n")?;
        fixture.git(&["add", "."])?;
        fixture.git(&["commit", "-m", "initial"])?;
        Ok(fixture)
    }
    fn repo(&self) -> AppResult<Repository> {
        Ok(Repository::discover(&self.path)?)
    }
    fn conflicted() -> AppResult<Self> {
        let fixture = Self::committed()?;
        fixture.git(&["checkout", "-b", "feature"])?;
        fixture.write("src/main.rs", "feature version\n")?;
        fixture.git(&["commit", "-am", "feature"])?;
        fixture.git(&["checkout", "main"])?;
        fixture.write("src/main.rs", "main version\n")?;
        fixture.git(&["commit", "-am", "main"])?;
        let output = Command::new("git")
            .arg("-C")
            .arg(&fixture.path)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "merge",
                "feature",
            ])
            .output()?;
        if output.status.code() != Some(1)
            || !String::from_utf8_lossy(&output.stdout).contains("CONFLICT")
        {
            return Err("Expected an unresolved merge in the fixture".into());
        }
        Ok(fixture)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn watch_separates_staged_unstaged_and_untracked_changes() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "fn main() {\n    println!(\"staged\");\n}\n")?;
    fixture.git(&["add", "src/main.rs"])?;
    fixture.write(
        "src/main.rs",
        "fn main() {\n    println!(\"working\");\n}\n",
    )?;
    fixture.write("new.txt", "untracked\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot
            .changes
            .iter()
            .map(|c| c.category)
            .collect::<Vec<_>>(),
        vec![Category::Unstaged, Category::Untracked, Category::Staged]
    );
    assert!(
        snapshot.changes[0].hunks[0]
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Added && l.text.contains("working"))
    );
    assert!(
        snapshot.changes[2].hunks[0]
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Added && l.text.contains("staged"))
    );
    Ok(())
}

#[test]
fn repository_without_commits_supports_index_and_untracked_files() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("added.txt", "index\n")?;
    fixture.git(&["add", "added.txt"])?;
    fixture.write("other.txt", "untracked\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 2);
    assert_eq!(snapshot.changes[1].category, Category::Staged);
    assert_eq!(snapshot.changes[1].hunks[0].lines[0].text, "index");
    Ok(())
}

#[test]
fn staged_rename_retains_both_paths_and_metadata() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.git(&["mv", "src/main.rs", "src/renamed.rs"])?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].path, Path::new("src/renamed.rs"));
    assert_eq!(
        snapshot.changes[0].old_path.as_deref(),
        Some(Path::new("src/main.rs"))
    );
    assert_eq!(snapshot.changes[0].status, 'R');
    Ok(())
}

#[test]
fn subdirectory_launch_discovers_the_whole_repository() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("outside.txt", "outside\n")?;
    // When
    let repo = Repository::discover(&fixture.path.join("src"))?;
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(repo.root.canonicalize()?, fixture.path.canonicalize()?);
    assert_eq!(snapshot.changes[0].path, Path::new("outside.txt"));
    Ok(())
}

#[test]
fn binary_file_is_summarized_without_text_hunks() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("image.bin", b"\0\x01\x02")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes[0].summary.as_deref(), Some("Binary file"));
    assert!(snapshot.changes[0].hunks.is_empty());
    Ok(())
}

#[test]
fn oversized_file_stays_a_summary_until_loaded() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("large.txt", "one\ntwo\nthree\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(
        &Mode::Watch,
        Limits { bytes: 4, lines: 2 },
        &BTreeSet::new(),
    )?;
    // Then
    assert!(snapshot.changes[0].oversized);
    assert!(snapshot.changes[0].hunks.is_empty());
    Ok(())
}

#[test]
fn explicit_load_displays_all_oversized_content() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("large.txt", "one\ntwo\nthree\n")?;
    let loaded = BTreeSet::from([PathBuf::from("large.txt")]);
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits { bytes: 4, lines: 2 }, &loaded)?;
    // Then
    assert!(!snapshot.changes[0].oversized);
    assert_eq!(snapshot.changes[0].hunks[0].lines.len(), 3);
    Ok(())
}

#[test]
fn comparison_is_fixed_even_when_a_branch_moves() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "second\n")?;
    fixture.git(&["commit", "-am", "second"])?;
    let repo = fixture.repo()?;
    let comparison = repo.comparison("HEAD~1", "HEAD", false)?;
    fixture.write("src/main.rs", "third\n")?;
    fixture.git(&["commit", "-am", "third"])?;
    // When
    let snapshot = repo.snapshot(&comparison, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes[0].new_source.as_deref(), Some("second\n"));
    assert_eq!(snapshot.changes[0].category, Category::Comparison);
    assert_eq!(
        snapshot.tracked_stats,
        Some(LineStats {
            added: 1,
            removed: 3
        })
    );
    Ok(())
}

#[test]
fn review_excludes_changes_unique_to_the_base_branch() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.git(&["checkout", "-b", "feature"])?;
    fixture.write("feature.txt", "feature\n")?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "feature"])?;
    fixture.git(&["checkout", "main"])?;
    fixture.write("base.txt", "base\n")?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "base"])?;
    let repo = fixture.repo()?;
    let comparison = repo.comparison("main", "feature", true)?;
    // When
    let snapshot = repo.snapshot(&comparison, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].path, Path::new("feature.txt"));
    assert_eq!(
        snapshot.tracked_stats,
        Some(LineStats {
            added: 1,
            removed: 0
        })
    );
    Ok(())
}

#[test]
fn paths_with_spaces_and_quotes_keep_their_identity() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("some ' file.txt", "before\n")?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "initial"])?;
    fixture.write("some ' file.txt", "after\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes[0].path, Path::new("some ' file.txt"));
    assert_eq!(snapshot.changes[0].new_source.as_deref(), Some("after\n"));
    Ok(())
}

#[test]
fn deleting_a_tracked_file_preserves_the_old_source() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fs::remove_file(fixture.path.join("src/main.rs"))?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes[0].status, 'D');
    assert!(snapshot.changes[0].old_source.is_some());
    assert!(snapshot.changes[0].new_source.is_none());
    assert!(snapshot.changes[0].summary.is_none());
    assert!(
        snapshot.changes[0].hunks[0]
            .lines
            .iter()
            .all(|l| l.kind == LineKind::Removed)
    );
    Ok(())
}

#[test]
fn linked_worktree_uses_its_own_index_and_working_directory() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.git(&["worktree", "add", "-b", "linked", "linked"])?;
    fixture.write("linked/new.txt", "linked\n")?;
    // When
    let repo = Repository::discover(&fixture.path.join("linked"))?;
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_ne!(repo.git_dir, repo.common_dir);
    assert_eq!(snapshot.changes[0].path, Path::new("new.txt"));
    Ok(())
}

#[test]
fn merge_conflicts_have_a_readable_unstaged_view() -> AppResult<()> {
    // Given
    let fixture = Fixture::conflicted()?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].status, 'U');
    assert_eq!(snapshot.changes[0].category, Category::Unstaged);
    assert!(
        snapshot.changes[0].hunks[0]
            .lines
            .iter()
            .any(|line| line.text.starts_with("<<<<<<<"))
    );
    Ok(())
}

#[test]
fn missing_conflict_file_does_not_hide_other_changes() -> AppResult<()> {
    // Given
    let fixture = Fixture::conflicted()?;
    fs::remove_file(fixture.path.join("src/main.rs"))?;
    fixture.write("other.txt", "other\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 2);
    assert_eq!(snapshot.changes[0].status, 'U');
    assert!(
        snapshot.changes[0]
            .summary
            .as_deref()
            .is_some_and(|s| s.contains("no working-tree file"))
    );
    assert_eq!(snapshot.changes[1].path, Path::new("other.txt"));
    Ok(())
}

#[test]
fn text_mentioning_binary_patch_headers_remains_visible() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write(
        "src/main.rs",
        "Binary files are ordinary text here\nGIT binary patch\n",
    )?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert!(snapshot.changes[0].summary.is_none());
    assert!(
        snapshot.changes[0].hunks[0]
            .lines
            .iter()
            .any(|line| line.text == "GIT binary patch")
    );
    Ok(())
}

#[test]
fn custom_git_output_indicators_do_not_hide_diff_content() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.git(&["config", "diff.outputIndicatorNew", ">"])?;
    fixture.git(&["config", "diff.outputIndicatorOld", "<"])?;
    fixture.git(&["config", "diff.suppressBlankEmpty", "true"])?;
    fixture.write("src/main.rs", "after\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert!(
        snapshot.changes[0].hunks[0]
            .lines
            .iter()
            .any(|line| line.kind == LineKind::Added && line.text == "after")
    );
    Ok(())
}

#[test]
fn line_ending_changes_preserve_the_carriage_return_in_removed_text() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("line.txt", "before\r\n")?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "initial"])?;
    fixture.write("line.txt", "before\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes[0].hunks[0].lines[0].text, "before\r");
    assert_eq!(snapshot.changes[0].hunks[0].lines[1].text, "before");
    Ok(())
}

#[test]
fn executable_bit_only_change_has_a_readable_summary() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.git(&["config", "core.filemode", "false"])?;
    fixture.git(&["update-index", "--chmod=+x", "src/main.rs"])?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].category, Category::Staged);
    assert!(snapshot.changes[0].hunks.is_empty());
    assert!(
        snapshot.changes[0]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("new mode 100755"))
    );
    Ok(())
}

#[test]
fn submodule_change_has_a_summary_without_blob_lookup() -> AppResult<()> {
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
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes[1].path, Path::new("module"));
    assert!(
        snapshot.changes[1]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.starts_with("Submodule change"))
    );
    assert!(snapshot.changes[1].hunks.is_empty());
    Ok(())
}

#[cfg(unix)]
#[test]
fn untracked_symlink_is_summarized_without_reading_its_target() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    std::os::unix::fs::symlink("missing-target", fixture.path.join("link"))?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.changes[0].summary.as_deref(),
        Some("Symbolic link → missing-target")
    );
    assert!(snapshot.changes[0].hunks.is_empty());
    Ok(())
}

fn warmed_collector(fixture: &Fixture) -> AppResult<Collector> {
    let mut collector = Collector::new(fixture.repo()?)?;
    collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    Ok(collector)
}

#[test]
fn persistent_collector_reads_content_changes_with_the_same_status() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "first edit\n")?;
    let mut collector = warmed_collector(&fixture)?;
    fixture.write("src/main.rs", "later edit\n")?;
    // When
    let snapshot = collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.changes[0].new_source.as_deref(),
        Some("later edit\n")
    );
    Ok(())
}

#[test]
fn persistent_collector_observes_index_replacement_with_unchanged_mtime() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let mut collector = warmed_collector(&fixture)?;
    let index = fixture.path.join(".git/index");
    let modified = fs::metadata(&index)?.modified()?;
    fixture.write("src/main.rs", "staged replacement\n")?;
    fixture.git(&["add", "."])?;
    fs::File::options()
        .write(true)
        .open(&index)?
        .set_times(fs::FileTimes::new().set_modified(modified))?;
    // When
    let snapshot = collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].category, Category::Staged);
    assert_eq!(
        snapshot.changes[0].new_source.as_deref(),
        Some("staged replacement\n")
    );
    Ok(())
}

#[test]
fn persistent_collector_reloads_exclusion_configuration() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("untracked.txt", "visible\n")?;
    fixture.write(".git/external-ignore", "untracked.txt\n")?;
    let mut collector = warmed_collector(&fixture)?;
    fixture.git(&["config", "core.excludesFile", ".git/external-ignore"])?;
    // When
    let snapshot = collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert!(snapshot.changes.is_empty());
    Ok(())
}

#[test]
fn sparse_index_preserves_changes_inside_the_worktree() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("other/file.txt", "outside sparse tree\n")?;
    fixture.git(&["add", "."])?;
    fixture.git(&["commit", "-m", "other tree"])?;
    fixture.git(&["sparse-checkout", "set", "--cone", "--sparse-index", "src"])?;
    fixture.write("src/main.rs", "sparse edit\n")?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(
        snapshot.changes[0].new_source.as_deref(),
        Some("sparse edit\n")
    );
    Ok(())
}

#[test]
fn split_index_preserves_staged_and_untracked_changes() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.git(&["update-index", "--split-index"])?;
    fixture.write("src/main.rs", "split edit\n")?;
    fixture.git(&["add", "."])?;
    fixture.write("new.txt", "untracked\n")?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes.len(), 2);
    assert_eq!(snapshot.changes[1].category, Category::Staged);
    assert_eq!(
        snapshot.changes[1].new_source.as_deref(),
        Some("split edit\n")
    );
    Ok(())
}

#[test]
fn packed_objects_remain_readable_after_an_external_commit() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let mut collector = warmed_collector(&fixture)?;
    fixture.write("src/main.rs", "new committed source\n")?;
    fixture.git(&["commit", "-am", "replace"])?;
    fixture.git(&["gc", "--prune=now"])?;
    fixture.write("src/main.rs", "working source\n")?;
    // When
    let snapshot = collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.changes[0].old_source.as_deref(),
        Some("new committed source\n")
    );
    assert_eq!(
        snapshot.changes[0].new_source.as_deref(),
        Some("working source\n")
    );
    Ok(())
}

#[test]
fn collecting_preserves_repository_index_and_configuration() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "changed\n")?;
    let index = fs::read(fixture.path.join(".git/index"))?;
    let config = fs::read(fixture.path.join(".git/config"))?;
    // When
    fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(fs::read(fixture.path.join(".git/index"))?, index);
    assert_eq!(fs::read(fixture.path.join(".git/config"))?, config);
    Ok(())
}

#[test]
fn deleting_a_tracked_gitignore_reveals_previously_ignored_files() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write(".gitignore", "hidden.txt\n")?;
    fixture.git(&["add", ".gitignore"])?;
    fixture.git(&["commit", "-m", "ignore"])?;
    fixture.write("hidden.txt", "revealed\n")?;
    let mut collector = warmed_collector(&fixture)?;
    fs::remove_file(fixture.path.join(".gitignore"))?;
    // When
    let snapshot = collector.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert!(
        snapshot
            .changes
            .iter()
            .any(|change| change.path == Path::new("hidden.txt")
                && change.category == Category::Untracked)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_keep_their_identity() -> AppResult<()> {
    use std::os::unix::ffi::OsStrExt;
    // Given
    let fixture = Fixture::committed()?;
    let name = std::ffi::OsStr::from_bytes(b"non-utf8-\xff.txt");
    fs::write(fixture.path.join(name), "visible\n")?;
    // When
    let snapshot = fixture
        .repo()?
        .snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(snapshot.changes[0].path, Path::new(name));
    assert_eq!(snapshot.changes[0].new_source.as_deref(), Some("visible\n"));
    Ok(())
}

#[test]
fn tracked_deletion_applies_the_byte_limit_to_the_old_source() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fs::remove_file(fixture.path.join("src/main.rs"))?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(
        &Mode::Watch,
        Limits {
            bytes: 1,
            lines: 100,
        },
        &BTreeSet::new(),
    )?;
    // Then
    assert!(snapshot.changes[0].oversized);
    assert!(snapshot.changes[0].hunks.is_empty());
    assert!(snapshot.changes[0].old_source.is_none());
    assert!(snapshot.changes[0].new_source.is_none());
    assert!(
        snapshot.changes[0]
            .summary
            .as_deref()
            .is_some_and(|summary| {
                summary.starts_with("Oversized file (") && summary.contains("3 diff lines")
            })
    );
    Ok(())
}

#[test]
fn tracked_diff_line_limit_hides_automatically_loaded_content() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "replacement\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(
        &Mode::Watch,
        Limits {
            bytes: 100,
            lines: 3,
        },
        &BTreeSet::new(),
    )?;
    // Then
    assert!(snapshot.changes[0].oversized);
    assert!(snapshot.changes[0].hunks.is_empty());
    assert!(
        snapshot.changes[0]
            .summary
            .as_deref()
            .is_some_and(|summary| { summary.contains("4 diff lines") })
    );
    Ok(())
}

#[test]
fn explicitly_loading_a_tracked_file_bypasses_both_limits() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "replacement\n")?;
    let repo = fixture.repo()?;
    let loaded = BTreeSet::from([PathBuf::from("src/main.rs")]);
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits { bytes: 1, lines: 1 }, &loaded)?;
    // Then
    assert!(!snapshot.changes[0].oversized);
    assert!(snapshot.changes[0].summary.is_none());
    assert_eq!(snapshot.changes[0].hunks[0].lines.len(), 4);
    assert_eq!(
        snapshot.changes[0].old_source.as_deref(),
        Some("fn main() {\n    println!(\"before\");\n}\n")
    );
    assert_eq!(
        snapshot.changes[0].new_source.as_deref(),
        Some("replacement\n")
    );
    Ok(())
}

#[test]
fn tracked_binary_changes_keep_a_summary_even_when_explicitly_loaded() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", b"\0binary")?;
    let repo = fixture.repo()?;
    let loaded = BTreeSet::from([PathBuf::from("src/main.rs")]);
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &loaded)?;
    // Then
    assert_eq!(
        snapshot.changes[0].summary.as_deref(),
        Some("Binary file changed")
    );
    assert!(snapshot.changes[0].hunks.is_empty());
    assert!(snapshot.changes[0].old_source.is_none());
    assert!(snapshot.changes[0].new_source.is_none());
    Ok(())
}

#[test]
fn tracked_non_utf8_text_keeps_the_patch_without_full_source() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", b"after\xff\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert!(snapshot.changes[0].summary.is_none());
    assert!(snapshot.changes[0].old_source.is_some());
    assert!(snapshot.changes[0].new_source.is_none());
    assert_eq!(snapshot.changes[0].hunks[0].lines[3].text, "after\u{fffd}");
    Ok(())
}

#[test]
fn untracked_non_utf8_files_have_an_explanatory_summary() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    fixture.write("file.txt", b"text\xff\n")?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.changes[0].summary.as_deref(),
        Some("Non-UTF-8 file (text preview unavailable)")
    );
    assert!(snapshot.changes[0].hunks.is_empty());
    assert!(snapshot.changes[0].new_source.is_none());
    Ok(())
}

#[test]
fn git_diff_failures_are_reported_as_collection_errors() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let repo = fixture.repo()?;
    let mode = Mode::Comparison {
        base: "missing-revision".into(),
        head: "HEAD".into(),
        label: "invalid comparison".into(),
    };
    // When
    let result = repo.snapshot(&mode, Limits::default(), &BTreeSet::new());
    // Then
    assert!(
        matches!(result, Err(GitError::Command(message)) if message.contains("missing-revision"))
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn tracked_symlink_sources_show_the_target_path_without_following_it() -> AppResult<()> {
    // Given
    let fixture = Fixture::new()?;
    let link = fixture.path.join("link");
    std::os::unix::fs::symlink("old-missing-target", &link)?;
    fixture.git(&["add", "link"])?;
    fixture.git(&["commit", "-m", "link"])?;
    fs::remove_file(&link)?;
    std::os::unix::fs::symlink("new-missing-target", &link)?;
    let repo = fixture.repo()?;
    // When
    let snapshot = repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?;
    // Then
    assert_eq!(
        snapshot.changes[0].old_source.as_deref(),
        Some("old-missing-target")
    );
    assert_eq!(
        snapshot.changes[0].new_source.as_deref(),
        Some("new-missing-target")
    );
    assert_eq!(
        snapshot.changes[0].hunks[0].lines[1].text,
        "new-missing-target"
    );
    assert!(snapshot.changes[0].summary.is_none());
    Ok(())
}

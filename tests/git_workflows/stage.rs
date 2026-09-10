use super::*;
use dift::git::{StageOutcome, StageTarget};

fn file(path: &str) -> StageTarget {
    StageTarget::File(path.into())
}
fn root() -> StageTarget {
    StageTarget::Subtree(PathBuf::new())
}

#[test]
fn partially_staged_file_stages_current_contents_without_changing_head_or_worktree() -> AppResult<()>
{
    // Given
    let f = Fixture::committed()?;
    let head = f.git(&["rev-parse", "HEAD"])?;
    f.write("src/main.rs", "staged\n")?;
    f.git(&["add", "."])?;
    f.write("src/main.rs", "working\n")?;
    f.write("other", "unrelated")?;
    // When
    let result = f.repo()?.toggle_stage(&file("src/main.rs"))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert_eq!(f.git(&["show", ":src/main.rs"])?, "working");
    assert_eq!(fs::read_to_string(f.path.join("src/main.rs"))?, "working\n");
    assert_eq!(f.git(&["rev-parse", "HEAD"])?, head);
    assert!(f.git(&["status", "--porcelain"])?.contains("?? other"));
    Ok(())
}

#[test]
fn fully_staged_file_unstages_without_changing_worktree() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "working\n")?;
    f.git(&["add", "."])?;
    // When
    let result = f.repo()?.toggle_stage(&file("src/main.rs"))?;
    // Then
    assert_eq!(result, StageOutcome::Unstaged);
    assert!(f.git(&["diff", "--cached"])?.is_empty());
    assert_eq!(fs::read_to_string(f.path.join("src/main.rs"))?, "working\n");
    Ok(())
}

#[test]
fn mixed_subtree_stages_all_descendants_and_preserves_unrelated_index_entries() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "staged\n")?;
    f.git(&["add", "."])?;
    f.write("src/deep/new", "new")?;
    f.write("outside", "outside")?;
    // When
    let result = f
        .repo()?
        .toggle_stage(&StageTarget::Subtree("src".into()))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert_eq!(f.git(&["show", ":src/deep/new"])?, "new");
    assert_eq!(f.git(&["show", ":src/main.rs"])?, "staged");
    assert!(f.git(&["status", "--porcelain"])?.contains("?? outside"));
    Ok(())
}

#[test]
fn deleted_file_is_staged() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    fs::remove_file(f.path.join("src/main.rs"))?;
    // When
    let result = f.repo()?.toggle_stage(&file("src/main.rs"))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert_eq!(
        f.git(&["diff", "--cached", "--name-status"])?,
        "D\tsrc/main.rs"
    );
    assert!(!f.path.join("src/main.rs").exists());
    Ok(())
}

#[test]
fn clean_root_has_nothing_to_toggle() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    // When
    let result = f.repo()?.toggle_stage(&root())?;
    // Then
    assert_eq!(result, StageOutcome::Unchanged);
    Ok(())
}

#[test]
fn unborn_repository_can_unstage_added_files() -> AppResult<()> {
    // Given
    let f = Fixture::new()?;
    f.write("new", "contents")?;
    f.git(&["add", "."])?;
    // When
    let result = f.repo()?.toggle_stage(&root())?;
    // Then
    assert_eq!(result, StageOutcome::Unstaged);
    assert!(f.git(&["ls-files"])?.is_empty());
    assert_eq!(fs::read_to_string(f.path.join("new"))?, "contents");
    Ok(())
}

#[test]
fn cross_directory_rename_unstages_both_endpoints() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.git(&["mv", "src/main.rs", "renamed.rs"])?;
    // When
    let result = f
        .repo()?
        .toggle_stage(&StageTarget::Subtree("src".into()))?;
    // Then
    assert_eq!(result, StageOutcome::Unstaged);
    assert!(f.git(&["diff", "--cached"])?.is_empty());
    assert!(f.path.join("renamed.rs").exists());
    Ok(())
}

#[test]
fn edited_staged_rename_only_adds_the_current_path() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.git(&["mv", "src/main.rs", "renamed.rs"])?;
    f.write(
        "renamed.rs",
        "fn main() {\n    println!(\"before\");\n}\n// edit\n",
    )?;
    // When
    let result = f.repo()?.toggle_stage(&file("renamed.rs"))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert!(f.git(&["diff"])?.is_empty());
    assert!(f.git(&["show", ":renamed.rs"])?.ends_with("// edit"));
    Ok(())
}

#[test]
fn unusual_names_are_literal_and_nul_delimited() -> AppResult<()> {
    // Given
    let f = Fixture::new()?;
    f.write(":(glob)*\n\tfile", "literal")?;
    f.write("other", "untouched")?;
    // When
    let result = f.repo()?.toggle_stage(&file(":(glob)*\n\tfile"))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert_eq!(f.repo()?.run(["ls-files", "-z"])?, b":(glob)*\n\tfile\0");
    Ok(())
}

#[test]
fn index_lock_failure_preserves_worktree_and_head() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let head = f.git(&["rev-parse", "HEAD"])?;
    f.write("src/main.rs", "working")?;
    f.write(".git/index.lock", "")?;
    // When
    let result = f.repo()?.toggle_stage(&root());
    // Then
    assert!(
        result
            .err()
            .ok_or("expected error")?
            .to_string()
            .contains("index.lock")
    );
    assert_eq!(f.git(&["rev-parse", "HEAD"])?, head);
    assert_eq!(fs::read_to_string(f.path.join("src/main.rs"))?, "working");
    Ok(())
}

#[test]
fn root_stages_whitespace_changes_and_respects_ignore_rules() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write(".git/info/exclude", "ignored\n")?;
    f.write("ignored", "ignored")?;
    f.write(
        "src/main.rs",
        "fn main() {\n     println!(\"before\");\n}\n",
    )?;
    f.write("new", "new")?;
    // When
    let result = f.repo()?.toggle_stage(&root())?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert!(f.git(&["diff"])?.is_empty());
    assert_eq!(f.git(&["ls-files"])?, "new\nsrc/main.rs");
    Ok(())
}

#[test]
fn conflicted_file_stages_the_working_resolution() -> AppResult<()> {
    // Given
    let f = Fixture::conflicted()?;
    f.write("src/main.rs", "resolved\n")?;
    // When
    let result = f.repo()?.toggle_stage(&file("src/main.rs"))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert!(f.git(&["ls-files", "--unmerged"])?.is_empty());
    assert_eq!(f.git(&["show", ":src/main.rs"])?, "resolved");
    Ok(())
}

#[cfg(unix)]
#[test]
fn non_utf8_filename_is_preserved_when_staging() -> AppResult<()> {
    use std::os::unix::ffi::OsStringExt;
    // Given
    let f = Fixture::new()?;
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"bad-\xff".to_vec()));
    fs::write(f.path.join(&path), "bytes")?;
    // When
    let result = f.repo()?.toggle_stage(&StageTarget::File(path))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert_eq!(f.repo()?.run(["ls-files", "-z"])?, b"bad-\xff\0");
    Ok(())
}

#[test]
fn submodule_staging_records_commit_without_staging_internal_files() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let nested = Fixture::committed()?;
    f.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        &nested.path.to_string_lossy(),
        "module",
    ])?;
    f.git(&["commit", "-am", "add module"])?;
    f.git(&["-C", "module", "commit", "--allow-empty", "-m", "next"])?;
    f.write("module/internal", "leave untracked")?;
    let commit = f.git(&["-C", "module", "rev-parse", "HEAD"])?;
    // When
    let result = f.repo()?.toggle_stage(&file("module"))?;
    // Then
    assert_eq!(result, StageOutcome::Staged);
    assert!(f.git(&["ls-files", "--stage", "module"])?.contains(&commit));
    assert_eq!(
        f.git(&["-C", "module", "status", "--porcelain"])?,
        "?? internal"
    );
    Ok(())
}

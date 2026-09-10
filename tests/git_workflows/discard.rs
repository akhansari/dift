use super::*;
use dift::git::{ChangeTarget, DiscardMode, DiscardOutcome};

fn root() -> ChangeTarget {
    ChangeTarget::Subtree(PathBuf::new())
}
fn file(path: &str) -> ChangeTarget {
    ChangeTarget::File(path.into())
}
fn discard(f: &Fixture, target: &ChangeTarget, mode: DiscardMode) -> AppResult<DiscardOutcome> {
    let repo = f.repo()?;
    Ok(repo.discard(&repo.prepare_discard(target, mode)?)?)
}

#[test]
fn unstaged_discard_preserves_staged_content_head_and_untracked_files() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let head = f.git(&["rev-parse", "HEAD"])?;
    f.write("src/main.rs", "staged\n")?;
    f.git(&["add", "."])?;
    f.write("src/main.rs", "working\n")?;
    f.write("new", "keep\n")?;
    // When
    let outcome = discard(&f, &root(), DiscardMode::Unstaged)?;
    // Then
    assert_eq!(outcome, DiscardOutcome::Discarded);
    assert_eq!(fs::read_to_string(f.path.join("src/main.rs"))?, "staged\n");
    assert_eq!(f.git(&["show", ":src/main.rs"])?, "staged");
    assert_eq!(f.git(&["rev-parse", "HEAD"])?, head);
    assert_eq!(fs::read_to_string(f.path.join("new"))?, "keep\n");
    Ok(())
}

#[test]
fn all_discard_restores_head_and_deletes_new_files_but_preserves_ignored_files() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let original = fs::read(f.path.join("src/main.rs"))?;
    let head = f.git(&["rev-parse", "HEAD"])?;
    f.write("src/main.rs", "staged\n")?;
    f.write("added", "staged addition")?;
    f.git(&["add", "."])?;
    f.write("src/main.rs", "working\n")?;
    f.write("new/deep/file", "delete")?;
    f.write(".git/info/exclude", "new/ignored\n")?;
    f.write("new/ignored", "keep")?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&root(), DiscardMode::All)?;
    // When
    let outcome = repo.discard(&preview)?;
    // Then
    assert_eq!(preview.files(), 3);
    assert_eq!(preview.deletions(), 2);
    assert_eq!(outcome, DiscardOutcome::Discarded);
    assert_eq!(fs::read(f.path.join("src/main.rs"))?, original);
    assert!(!f.path.join("added").exists());
    assert!(!f.path.join("new/deep/file").exists());
    assert_eq!(fs::read_to_string(f.path.join("new/ignored"))?, "keep");
    assert!(f.git(&["status", "--porcelain"])?.is_empty());
    assert_eq!(f.git(&["rev-parse", "HEAD"])?, head);
    Ok(())
}

#[test]
fn recursive_target_preserves_siblings_and_their_staged_content() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "discard")?;
    f.write("src/deep/new", "discard")?;
    f.write("src-extra/other", "keep")?;
    f.git(&["add", "src-extra"])?;
    // When
    discard(&f, &ChangeTarget::Subtree("src".into()), DiscardMode::All)?;
    // Then
    assert!(f.git(&["diff"])?.is_empty());
    assert!(!f.path.join("src/deep/new").exists());
    assert_eq!(f.git(&["show", ":src-extra/other"])?, "keep");
    Ok(())
}

#[test]
fn unstaged_discard_restores_a_deleted_tracked_file() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let original = fs::read(f.path.join("src/main.rs"))?;
    fs::remove_file(f.path.join("src/main.rs"))?;
    // When
    discard(&f, &root(), DiscardMode::Unstaged)?;
    // Then
    assert_eq!(fs::read(f.path.join("src/main.rs"))?, original);
    assert!(f.git(&["status", "--porcelain"])?.is_empty());
    Ok(())
}

#[test]
fn all_discard_restores_staged_deletion_even_when_untracked_content_reoccupies_path()
-> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let original = fs::read(f.path.join("src/main.rs"))?;
    f.git(&["rm", "src/main.rs"])?;
    f.write("src/main.rs", "replacement")?;
    // When
    discard(&f, &root(), DiscardMode::All)?;
    // Then
    assert_eq!(fs::read(f.path.join("src/main.rs"))?, original);
    assert!(f.git(&["status", "--porcelain"])?.is_empty());
    Ok(())
}

#[test]
fn unstaged_discard_restores_indexed_addition_in_unborn_repository() -> AppResult<()> {
    // Given
    let f = Fixture::new()?;
    f.write("added", "indexed")?;
    f.git(&["add", "."])?;
    f.write("added", "edited")?;
    // When
    discard(&f, &root(), DiscardMode::Unstaged)?;
    // Then
    assert_eq!(fs::read_to_string(f.path.join("added"))?, "indexed");
    assert_eq!(f.git(&["show", ":added"])?, "indexed");
    Ok(())
}

#[test]
fn all_discard_uses_empty_baseline_without_a_first_commit() -> AppResult<()> {
    // Given
    let f = Fixture::new()?;
    f.write("added", "indexed")?;
    f.git(&["add", "."])?;
    f.write("added", "edited")?;
    f.write("untracked", "new")?;
    // When
    discard(&f, &root(), DiscardMode::All)?;
    // Then
    assert!(!f.path.join("added").exists());
    assert!(!f.path.join("untracked").exists());
    assert!(f.git(&["ls-files"])?.is_empty());
    Ok(())
}

#[test]
fn rename_review_includes_outside_endpoint_and_discard_restores_both() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.git(&["mv", "src/main.rs", "renamed.rs"])?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&ChangeTarget::Subtree("src".into()), DiscardMode::All)?;
    // When
    repo.discard(&preview)?;
    // Then
    assert_eq!(
        preview.outside_paths().collect::<Vec<_>>(),
        vec![Path::new("renamed.rs")]
    );
    assert_eq!(preview.files(), 2);
    assert_eq!(preview.deletions(), 1);
    assert!(f.path.join("src/main.rs").exists());
    assert!(!f.path.join("renamed.rs").exists());
    assert!(f.git(&["status", "--porcelain"])?.is_empty());
    Ok(())
}

#[test]
fn unstaged_discard_preserves_staged_rename() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let original = fs::read(f.path.join("src/main.rs"))?;
    f.git(&["mv", "src/main.rs", "renamed.rs"])?;
    f.write("renamed.rs", "edited")?;
    // When
    discard(&f, &file("renamed.rs"), DiscardMode::Unstaged)?;
    // Then
    assert_eq!(fs::read(f.path.join("renamed.rs"))?, original);
    assert!(!f.path.join("src/main.rs").exists());
    assert!(f.git(&["diff"])?.is_empty());
    assert!(
        f.git(&["diff", "--cached", "--name-status"])?
            .starts_with("R100")
    );
    Ok(())
}

#[test]
fn edited_content_invalidates_confirmation_without_writing() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "first")?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&root(), DiscardMode::All)?;
    f.write("src/main.rs", "later")?;
    // When
    let outcome = repo.discard(&preview)?;
    // Then
    assert_eq!(outcome, DiscardOutcome::Stale);
    assert_eq!(fs::read_to_string(f.path.join("src/main.rs"))?, "later");
    Ok(())
}

#[test]
fn new_file_in_scope_invalidates_confirmation_without_deleting_anything() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("first", "first")?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&root(), DiscardMode::All)?;
    f.write("later", "later")?;
    // When
    let outcome = repo.discard(&preview)?;
    // Then
    assert_eq!(outcome, DiscardOutcome::Stale);
    assert!(f.path.join("first").exists());
    assert!(f.path.join("later").exists());
    Ok(())
}

#[test]
fn unrelated_edits_do_not_invalidate_file_confirmation() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "discard")?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&file("src/main.rs"), DiscardMode::All)?;
    f.write("outside", "keep")?;
    f.git(&["add", "outside"])?;
    // When
    let outcome = repo.discard(&preview)?;
    // Then
    assert_eq!(outcome, DiscardOutcome::Discarded);
    assert_eq!(f.git(&["show", ":outside"])?, "keep");
    Ok(())
}

#[test]
fn conflicts_block_the_whole_operation_before_mutation() -> AppResult<()> {
    // Given
    let f = Fixture::conflicted()?;
    f.write("new", "keep")?;
    let status = f.git(&["status", "--porcelain"])?;
    // When
    let result = f.repo()?.prepare_discard(&root(), DiscardMode::All);
    // Then
    assert!(
        result
            .err()
            .ok_or("expected conflict")?
            .to_string()
            .contains("Unresolved conflict")
    );
    assert_eq!(f.git(&["status", "--porcelain"])?, status);
    assert_eq!(fs::read_to_string(f.path.join("new"))?, "keep");
    Ok(())
}

#[test]
fn nested_repository_blocks_all_but_not_unstaged_discard() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.git(&["init", "nested"])?;
    f.write("nested/file", "keep")?;
    f.write("src/main.rs", "edit")?;
    let repo = f.repo()?;
    // When
    let all = repo.prepare_discard(&root(), DiscardMode::All);
    let unstaged = repo.prepare_discard(&root(), DiscardMode::Unstaged)?;
    // Then
    assert!(
        all.err()
            .ok_or("expected nested repository error")?
            .to_string()
            .contains("unsupported")
    );
    assert_eq!(unstaged.files(), 1);
    assert_eq!(fs::read_to_string(f.path.join("nested/file"))?, "keep");
    Ok(())
}

#[test]
fn submodule_changes_block_discard_without_touching_nested_files() -> AppResult<()> {
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
    f.git(&["commit", "-am", "module"])?;
    f.write("module/src/main.rs", "keep")?;
    // When
    let result = f.repo()?.prepare_discard(&root(), DiscardMode::Unstaged);
    // Then
    assert!(
        result
            .err()
            .ok_or("expected submodule error")?
            .to_string()
            .contains("unsupported")
    );
    assert_eq!(
        fs::read_to_string(f.path.join("module/src/main.rs"))?,
        "keep"
    );
    Ok(())
}

#[test]
fn index_lock_failure_does_not_delete_untracked_files() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "keep")?;
    f.write("new", "keep too")?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&root(), DiscardMode::All)?;
    f.write(".git/index.lock", "")?;
    // When
    let result = repo.discard(&preview);
    // Then
    assert!(
        result
            .err()
            .ok_or("expected lock error")?
            .to_string()
            .contains("index.lock")
    );
    assert_eq!(fs::read_to_string(f.path.join("new"))?, "keep too");
    assert_eq!(fs::read_to_string(f.path.join("src/main.rs"))?, "keep");
    Ok(())
}

#[test]
fn unusual_tracked_names_are_restored_literally() -> AppResult<()> {
    // Given
    let f = Fixture::new()?;
    f.write(":(glob)*\n\tfile", "original")?;
    f.git(&["add", "."])?;
    f.git(&["commit", "-m", "initial"])?;
    f.write(":(glob)*\n\tfile", "edit")?;
    f.write("other", "keep")?;
    // When
    discard(&f, &file(":(glob)*\n\tfile"), DiscardMode::All)?;
    // Then
    assert_eq!(
        fs::read_to_string(f.path.join(":(glob)*\n\tfile"))?,
        "original"
    );
    assert_eq!(fs::read_to_string(f.path.join("other"))?, "keep");
    Ok(())
}

#[cfg(unix)]
#[test]
fn untracked_non_utf8_path_is_deleted_literally() -> AppResult<()> {
    use std::os::unix::ffi::OsStringExt;
    // Given
    let f = Fixture::new()?;
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"bad-\xff".to_vec()));
    fs::write(f.path.join(&path), "delete")?;
    f.write("other", "keep")?;
    // When
    discard(&f, &ChangeTarget::File(path.clone()), DiscardMode::All)?;
    // Then
    assert!(!f.path.join(path).exists());
    assert_eq!(fs::read_to_string(f.path.join("other"))?, "keep");
    Ok(())
}

#[cfg(unix)]
#[test]
fn restoring_file_replaced_by_symlink_does_not_modify_link_target() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let original = fs::read(f.path.join("src/main.rs"))?;
    f.write("outside", "keep")?;
    fs::remove_file(f.path.join("src/main.rs"))?;
    std::os::unix::fs::symlink("../outside", f.path.join("src/main.rs"))?;
    // When
    discard(&f, &file("src/main.rs"), DiscardMode::Unstaged)?;
    // Then
    assert_eq!(fs::read(f.path.join("src/main.rs"))?, original);
    assert_eq!(fs::read_to_string(f.path.join("outside"))?, "keep");
    Ok(())
}

#[test]
fn untracked_only_target_has_no_unstaged_discard() -> AppResult<()> {
    // Given
    let f = Fixture::new()?;
    f.write("new", "keep")?;
    // When
    let outcome = discard(&f, &root(), DiscardMode::Unstaged)?;
    // Then
    assert_eq!(outcome, DiscardOutcome::Unchanged);
    assert_eq!(fs::read_to_string(f.path.join("new"))?, "keep");
    Ok(())
}

#[test]
fn content_change_with_identical_size_and_timestamp_invalidates_confirmation() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "first")?;
    let path = f.path.join("src/main.rs");
    let modified = fs::metadata(&path)?.modified()?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&root(), DiscardMode::Unstaged)?;
    f.write("src/main.rs", "later")?;
    fs::File::options()
        .write(true)
        .open(&path)?
        .set_times(fs::FileTimes::new().set_modified(modified))?;
    // When
    let outcome = repo.discard(&preview)?;
    // Then
    assert_eq!(outcome, DiscardOutcome::Stale);
    assert_eq!(fs::read_to_string(path)?, "later");
    Ok(())
}

#[test]
fn changed_head_invalidates_confirmation_even_when_files_are_unchanged() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    f.write("src/main.rs", "keep")?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&root(), DiscardMode::All)?;
    f.git(&["commit", "--allow-empty", "-m", "new head"])?;
    // When
    let outcome = repo.discard(&preview)?;
    // Then
    assert_eq!(outcome, DiscardOutcome::Stale);
    assert_eq!(fs::read_to_string(f.path.join("src/main.rs"))?, "keep");
    Ok(())
}

#[cfg(unix)]
#[test]
fn unstaged_discard_restores_executable_mode() -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let f = Fixture::committed()?;
    f.git(&["config", "core.filemode", "true"])?;
    let path = f.path.join("src/main.rs");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    // When
    discard(&f, &root(), DiscardMode::Unstaged)?;
    // Then
    assert_eq!(fs::metadata(path)?.permissions().mode() & 0o111, 0);
    assert!(f.git(&["diff"])?.is_empty());
    Ok(())
}

#[cfg(unix)]
#[test]
fn a_filter_edit_during_restore_is_preserved_and_reports_partial_failure() -> AppResult<()> {
    // Given
    let f = Fixture::committed()?;
    let original = fs::read(f.path.join("src/main.rs"))?;
    f.write("src/main.rs", "discard")?;
    f.write("victim", "reviewed")?;
    f.write(".git/info/attributes", "src/main.rs filter=touch-victim\n")?;
    f.git(&[
        "config",
        "filter.touch-victim.smudge",
        "printf later > victim; cat",
    ])?;
    let repo = f.repo()?;
    let preview = repo.prepare_discard(&root(), DiscardMode::All)?;
    // When
    let result = repo.discard(&preview);
    // Then
    let error = result
        .err()
        .ok_or("expected changed-file error")?
        .to_string();
    assert!(error.contains("may be partial"));
    assert!(error.contains("File changed before deletion: victim"));
    assert_eq!(fs::read_to_string(f.path.join("victim"))?, "later");
    assert_eq!(fs::read(f.path.join("src/main.rs"))?, original);
    Ok(())
}

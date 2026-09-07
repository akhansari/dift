use super::*;
use dift::{
    config::Whitespace,
    model::{ActivityObservation, Snapshot},
    state::State,
};
use std::time::{Duration, Instant};

fn replace_conflict_stage(fixture: &Fixture, oid: &str) -> AppResult<()> {
    use std::{io::Write, process::Stdio};
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&fixture.path)
        .args(["update-index", "--index-info"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("Missing index input pipe")?;
    writeln!(input, "100644 {oid} 2\tsrc/main.rs")?;
    drop(input);
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(format!(
            "Update conflict stage: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn observation<'a>(
    snapshot: &'a Snapshot,
    path: &str,
    category: Category,
) -> AppResult<&'a ActivityObservation> {
    snapshot
        .activity
        .as_ref()
        .and_then(|paths| paths.get(Path::new(path)))
        .and_then(|categories| categories.get(&category))
        .ok_or_else(|| format!("Missing observation for {path}").into())
}

fn collect(collector: &mut Collector, whitespace: Whitespace) -> AppResult<Snapshot> {
    Ok(collector.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        whitespace,
    )?)
}

fn accept(state: &mut State, snapshot: Snapshot, now: Instant) {
    let request = state.collection_request();
    state.collection_finished(request.generation, Ok(snapshot), now);
}

#[test]
fn whitespace_hidden_files_keep_their_repository_identity() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write(
        "src/main.rs",
        "fn main() {\n        println!(\"before\");\n}\n",
    )?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let before = collect(&mut collector, Whitespace::IgnoreAll)?;
    // When
    let after = collect(&mut collector, Whitespace::Exact)?;
    // Then
    assert!(before.changes.is_empty());
    assert!(!after.changes.is_empty());
    assert!(
        !observation(&after, "src/main.rs", Category::Unstaged)?.changed_since(observation(
            &before,
            "src/main.rs",
            Category::Unstaged
        )?)
    );
    Ok(())
}

#[test]
fn whitespace_toggling_does_not_change_an_unstaged_identity() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write(
        "src/main.rs",
        "fn main() {\n        println!(\"after\");\n}\n",
    )?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let before = collect(&mut collector, Whitespace::Exact)?;
    // When
    let after = collect(&mut collector, Whitespace::IgnoreAll)?;
    // Then
    assert!(
        !observation(&after, "src/main.rs", Category::Unstaged)?.changed_since(observation(
            &before,
            "src/main.rs",
            Category::Unstaged
        )?)
    );
    Ok(())
}

#[test]
fn loading_an_omitted_file_does_not_signal_an_edit() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("new.txt", "a file omitted by the byte limit\n")?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let limits = Limits { bytes: 1, lines: 1 };
    let before = collector.snapshot(&Mode::Watch, limits, &BTreeSet::new())?;
    // When
    let after = collector.snapshot(
        &Mode::Watch,
        limits,
        &BTreeSet::from([PathBuf::from("new.txt")]),
    )?;
    // Then
    assert!(
        observation(&before, "new.txt", Category::Untracked)?
            .source
            .is_none()
    );
    assert!(
        observation(&after, "new.txt", Category::Untracked)?
            .source
            .is_some()
    );
    assert!(
        !observation(&after, "new.txt", Category::Untracked)?.changed_since(observation(
            &before,
            "new.txt",
            Category::Untracked
        )?)
    );
    Ok(())
}

#[test]
fn an_external_edit_during_loading_is_still_detected() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("new.txt", "omitted\n")?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let limits = Limits { bytes: 1, lines: 1 };
    let before = collector.snapshot(&Mode::Watch, limits, &BTreeSet::new())?;
    // When
    fixture.write("new.txt", "edited while requesting its preview\n")?;
    let after = collector.snapshot(
        &Mode::Watch,
        limits,
        &BTreeSet::from([PathBuf::from("new.txt")]),
    )?;
    // Then
    assert!(
        observation(&after, "new.txt", Category::Untracked)?.changed_since(observation(
            &before,
            "new.txt",
            Category::Untracked
        )?)
    );
    Ok(())
}

#[test]
fn loaded_text_edits_are_detected_even_with_unchanged_size_and_mtime() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "after one\n")?;
    let path = fixture.path.join("src/main.rs");
    let modified = fs::metadata(&path)?.modified()?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let before = collect(&mut collector, Whitespace::Exact)?;
    // When
    fixture.write("src/main.rs", "after two\n")?;
    fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .set_modified(modified)?;
    let after = collect(&mut collector, Whitespace::Exact)?;
    // Then
    assert_eq!(
        observation(&before, "src/main.rs", Category::Unstaged)?.metadata,
        observation(&after, "src/main.rs", Category::Unstaged)?.metadata
    );
    assert!(
        observation(&after, "src/main.rs", Category::Unstaged)?.changed_since(observation(
            &before,
            "src/main.rs",
            Category::Unstaged
        )?)
    );
    Ok(())
}

#[test]
fn touching_unchanged_loaded_text_does_not_signal_an_edit() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "after\n")?;
    let path = fixture.path.join("src/main.rs");
    let modified = fs::metadata(&path)?.modified()?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let before = collect(&mut collector, Whitespace::Exact)?;
    // When
    fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .set_modified(modified + Duration::from_secs(10))?;
    let after = collect(&mut collector, Whitespace::Exact)?;
    // Then
    assert_ne!(
        observation(&before, "src/main.rs", Category::Unstaged)?.metadata,
        observation(&after, "src/main.rs", Category::Unstaged)?.metadata
    );
    assert!(
        !observation(&after, "src/main.rs", Category::Unstaged)?.changed_since(observation(
            &before,
            "src/main.rs",
            Category::Unstaged
        )?)
    );
    Ok(())
}

#[test]
fn binary_edits_use_metadata_without_loading_a_preview() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("blob.bin", b"\0one")?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let before = collect(&mut collector, Whitespace::Exact)?;
    // When
    fixture.write("blob.bin", b"\0another")?;
    let after = collect(&mut collector, Whitespace::Exact)?;
    // Then
    assert!(
        observation(&after, "blob.bin", Category::Untracked)?
            .source
            .is_none()
    );
    assert!(
        observation(&after, "blob.bin", Category::Untracked)?.changed_since(observation(
            &before,
            "blob.bin",
            Category::Untracked
        )?)
    );
    Ok(())
}

#[test]
fn external_staging_highlights_the_existing_tree_path() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    fixture.write("src/main.rs", "after\n")?;
    let now = Instant::now();
    let mut collector = Collector::new(fixture.repo()?)?;
    let mut state = State::new(false);
    accept(&mut state, collect(&mut collector, Whitespace::Exact)?, now);
    // When
    fixture.git(&["add", "src/main.rs"])?;
    accept(&mut state, collect(&mut collector, Whitespace::Exact)?, now);
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert_eq!(
        state.activity_intensity(Path::new("src/main.rs"), false),
        1.0
    );
    Ok(())
}

#[test]
fn external_renames_highlight_the_destination_without_leaving_a_ghost() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let now = Instant::now();
    let mut collector = Collector::new(fixture.repo()?)?;
    let mut state = State::new(false);
    accept(&mut state, collect(&mut collector, Whitespace::Exact)?, now);
    // When
    fixture.git(&["mv", "src/main.rs", "src/renamed.rs"])?;
    accept(&mut state, collect(&mut collector, Whitespace::Exact)?, now);
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert_eq!(
        state.activity_intensity(Path::new("src/renamed.rs"), false),
        1.0
    );
    assert_eq!(
        state.activity_intensity(Path::new("src/main.rs"), false),
        0.0
    );
    Ok(())
}

#[test]
fn external_deletions_highlight_the_retained_diff_row() -> AppResult<()> {
    // Given
    let fixture = Fixture::committed()?;
    let now = Instant::now();
    let mut collector = Collector::new(fixture.repo()?)?;
    let mut state = State::new(false);
    accept(&mut state, collect(&mut collector, Whitespace::Exact)?, now);
    // When
    fs::remove_file(fixture.path.join("src/main.rs"))?;
    accept(&mut state, collect(&mut collector, Whitespace::Exact)?, now);
    state.advance_activity(now + Duration::from_secs(1));
    // Then
    assert_eq!(state.snapshot().changes[0].status, 'D');
    assert_eq!(
        state.activity_intensity(Path::new("src/main.rs"), false),
        1.0
    );
    Ok(())
}

#[test]
fn updating_conflict_stages_without_editing_the_worktree_is_detected() -> AppResult<()> {
    // Given
    let fixture = Fixture::conflicted()?;
    let mut collector = Collector::new(fixture.repo()?)?;
    let before = collect(&mut collector, Whitespace::Exact)?;
    let theirs = fixture.git(&["rev-parse", ":3:src/main.rs"])?;
    // When
    replace_conflict_stage(&fixture, &theirs)?;
    let after = collect(&mut collector, Whitespace::Exact)?;
    // Then
    assert_eq!(
        observation(&after, "src/main.rs", Category::Unstaged)?
            .git
            .status,
        'U'
    );
    assert_eq!(
        observation(&after, "src/main.rs", Category::Unstaged)?.source,
        observation(&before, "src/main.rs", Category::Unstaged)?.source
    );
    assert!(
        observation(&after, "src/main.rs", Category::Unstaged)?.changed_since(observation(
            &before,
            "src/main.rs",
            Category::Unstaged
        )?)
    );
    Ok(())
}

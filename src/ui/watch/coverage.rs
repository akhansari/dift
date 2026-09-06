use super::*;
use crate::{AppResult, ui::fixtures::Fixture};

fn watching(fixture: &Fixture) -> AppResult<(Watches, Inbox)> {
    let collector = fixture.collector()?;
    let mut filter = EventFilter::new(collector.watch_context()?);
    let (mut watches, inbox) = Watches::new(&fixture.repo()?, Arc::default(), true);
    watches.reconcile(&mut filter, &collector, &Batch::default(), false);
    Ok((watches, inbox))
}

fn event_for(inbox: &Inbox, path: &Path) -> AppResult<bool> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while let Ok(event) = inbox
        .events
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        if event.paths.iter().any(|event_path| event_path == path) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[test]
fn ignored_dependency_edits_do_not_enter_the_event_queue() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("target/\n")?;
    fixture.write("target/dependency/output", "before\n")?;
    let (_watches, inbox) = watching(&fixture)?;
    // When
    fixture.write("target/dependency/output", "after\n")?;
    let event = inbox.events.recv_timeout(Duration::from_millis(300));
    // Then
    assert!(matches!(event, Err(mpsc::RecvTimeoutError::Timeout)));
    Ok(())
}

#[test]
fn tracked_descendants_of_ignored_directories_deliver_events() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("target/\n")?;
    fixture.write("target/pinned/file.txt", "before\n")?;
    fixture.git(&["add", "-f", "target/pinned/file.txt"])?;
    let (_watches, inbox) = watching(&fixture)?;
    // When
    fixture.write("target/pinned/file.txt", "after\n")?;
    let delivered = event_for(&inbox, &fixture.root.join("target/pinned/file.txt"))?;
    // Then
    assert!(delivered);
    Ok(())
}

#[cfg(unix)]
#[test]
fn directory_symlinks_do_not_watch_their_targets() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    let external = Fixture::new("")?;
    external.write("package/file.txt", "before\n")?;
    std::os::unix::fs::symlink(external.root.join("package"), fixture.root.join("linked"))?;
    let (_watches, inbox) = watching(&fixture)?;
    // When
    external.write("package/file.txt", "after\n")?;
    let event = inbox.events.recv_timeout(Duration::from_millis(300));
    // Then
    assert!(matches!(event, Err(mpsc::RecvTimeoutError::Timeout)));
    Ok(())
}

fn replaced_directory(fixture: &Fixture) -> AppResult<(Watches, Inbox, Collector, EventFilter)> {
    fixture.write("src/nested/file.txt", "before\n")?;
    let (watches, inbox) = watching(fixture)?;
    fs::remove_dir_all(fixture.root.join("src"))?;
    fixture.write("src/nested/file.txt", "recreated\n")?;
    while inbox
        .events
        .recv_timeout(Duration::from_millis(100))
        .is_ok()
    {}
    let collector = fixture.collector()?;
    let filter = EventFilter::new(collector.watch_context()?);
    Ok((watches, inbox, collector, filter))
}

#[test]
fn periodic_reconciliation_repairs_recreated_directories_without_events() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    let (mut watches, inbox, collector, mut filter) = replaced_directory(&fixture)?;
    // When
    let expanded = watches.reconcile(&mut filter, &collector, &Batch::default(), true);
    fixture.write("src/nested/file.txt", "edited again\n")?;
    let delivered = event_for(&inbox, &fixture.root.join("src/nested/file.txt"))?;
    // Then
    assert!(expanded);
    assert!(delivered);
    Ok(())
}

#[test]
fn overflow_repairs_recreated_directories_and_requests_a_follow_up_scan() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    let (mut watches, inbox, collector, mut filter) = replaced_directory(&fixture)?;
    let batch = Batch {
        force: true,
        ..Batch::default()
    };
    // When
    let expanded = watches.reconcile(&mut filter, &collector, &batch, false);
    fixture.write("src/nested/file.txt", "edited again\n")?;
    let delivered = event_for(&inbox, &fixture.root.join("src/nested/file.txt"))?;
    // Then
    assert!(expanded);
    assert!(delivered);
    Ok(())
}

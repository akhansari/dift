use super::*;

#[test]
fn a_disconnected_worker_reports_which_worker_stopped() {
    // Given
    let (sender, receiver) = mpsc::channel::<()>();
    drop(sender);
    // When
    let result = worker_update(receiver.try_recv(), "Rendering");
    // Then
    assert!(matches!(result, Err(error) if error.to_string() == "Rendering worker disconnected"));
}

#[test]
fn an_idle_worker_keeps_the_viewer_running() -> io::Result<()> {
    // Given
    let (_sender, receiver) = mpsc::channel::<()>();
    // When
    let update = worker_update(receiver.try_recv(), "Git collection")?;
    // Then
    assert_eq!(update, None);
    Ok(())
}

use super::*;

#[test]
fn the_initial_activity_is_available_immediately() {
    // Given
    let now = Instant::now();
    // When
    let status = ActivityStatus::new(Activity::Refreshing);
    // Then
    assert_eq!(status.displayed(), Activity::Refreshing);
    assert_eq!(status.delay(now), None);
}

#[test]
fn a_new_activity_waits_for_the_full_delay() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    // When
    let redraw = status.update(Activity::Polling, now + Duration::from_millis(499));
    // Then
    assert!(!redraw);
    assert_eq!(status.displayed(), Activity::Watching);
    assert_eq!(
        status.delay(now + Duration::from_millis(499)),
        Some(Duration::from_millis(1)),
    );
}

#[test]
fn a_sustained_activity_requests_a_redraw_at_the_deadline() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    // When
    let redraw = status.update(Activity::Polling, now + Duration::from_millis(500));
    // Then
    assert!(redraw);
    assert_eq!(status.displayed(), Activity::Polling);
    assert_eq!(status.delay(now + Duration::from_millis(500)), None);
}

#[test]
fn returning_to_the_displayed_activity_cancels_a_brief_change() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    // When
    let redraw = status.update(Activity::Watching, now + Duration::from_millis(300));
    // Then
    assert!(!redraw);
    assert_eq!(status.displayed(), Activity::Watching);
    assert_eq!(status.delay(now + Duration::from_millis(300)), None);
}

#[test]
fn a_cancelled_activity_does_not_appear_on_a_later_tick() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    status.update(Activity::Watching, now + Duration::from_millis(300));
    // When
    let redraw = status.update(Activity::Watching, now + Duration::from_millis(500));
    // Then
    assert!(!redraw);
    assert_eq!(status.displayed(), Activity::Watching);
}

#[test]
fn a_different_candidate_restarts_the_delay() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    status.update(Activity::Rendering, now + Duration::from_millis(300));
    // When
    let redraw = status.update(Activity::Rendering, now + Duration::from_millis(799));
    // Then
    assert!(!redraw);
    assert_eq!(status.displayed(), Activity::Watching);
    assert_eq!(
        status.delay(now + Duration::from_millis(799)),
        Some(Duration::from_millis(1)),
    );
}

#[test]
fn only_the_latest_candidate_appears_after_its_own_delay() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    status.update(Activity::Rendering, now + Duration::from_millis(300));
    // When
    let redraw = status.update(Activity::Rendering, now + Duration::from_millis(800));
    // Then
    assert!(redraw);
    assert_eq!(status.displayed(), Activity::Rendering);
}

#[test]
fn repeated_observations_do_not_postpone_a_change() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Refreshing, now);
    status.update(Activity::Refreshing, now + Duration::from_millis(200));
    status.update(Activity::Refreshing, now + Duration::from_millis(400));
    // When
    let redraw = status.update(Activity::Refreshing, now + Duration::from_millis(500));
    // Then
    assert!(redraw);
    assert_eq!(status.displayed(), Activity::Refreshing);
}

#[test]
fn an_outdated_candidate_is_discarded_even_after_its_deadline() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    // When
    let redraw = status.update(Activity::Refreshing, now + Duration::from_millis(600));
    // Then
    assert!(!redraw);
    assert_eq!(status.displayed(), Activity::Watching);
    assert_eq!(
        status.delay(now + Duration::from_millis(600)),
        Some(Duration::from_millis(500)),
    );
}

#[test]
fn returning_from_a_displayed_activity_also_waits_for_the_delay() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Watching);
    status.update(Activity::Polling, now);
    status.update(Activity::Polling, now + Duration::from_millis(500));
    status.update(Activity::Watching, now + Duration::from_millis(600));
    // When
    let redraw = status.update(Activity::Watching, now + Duration::from_millis(1099));
    // Then
    assert!(!redraw);
    assert_eq!(status.displayed(), Activity::Polling);
}

#[test]
fn a_displayed_activity_does_not_keep_requesting_redraws() {
    // Given
    let now = Instant::now();
    let mut status = ActivityStatus::new(Activity::Rendering);
    status.update(Activity::Snapshot, now);
    status.update(Activity::Snapshot, now + Duration::from_millis(500));
    // When
    let redraw = status.update(Activity::Snapshot, now + Duration::from_millis(600));
    // Then
    assert!(!redraw);
    assert_eq!(status.displayed(), Activity::Snapshot);
    assert_eq!(status.delay(now + Duration::from_millis(600)), None);
}

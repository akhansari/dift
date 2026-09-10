use super::*;
use crate::{
    AppResult,
    git::{Limits, Mode, StageOutcome, StageTarget},
    state::State,
    ui::fixtures::Fixture,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

struct Scenario {
    fixture: Fixture,
    repo: Repository,
    pending: Mutations,
    state: State,
    now: Instant,
}

impl Scenario {
    fn new(fixture: Fixture) -> AppResult<Self> {
        let repo = fixture.repo()?;
        let mut state = State::new(false);
        let now = Instant::now();
        let request = state.collection_request();
        state.collection_finished(request.generation, Ok(snapshot(&repo)?), now);
        Ok(Self {
            fixture,
            repo,
            pending: Mutations::default(),
            state,
            now,
        })
    }

    fn stage(&mut self, target: StageTarget) -> AppResult<StageOutcome> {
        let head = self.repo.activity_head()?;
        let before = snapshot(&self.repo)?;
        self.state
            .observe_activity(before.activity.clone().ok_or("Missing activity")?, self.now);
        let report = self.repo.toggle_stage_tracked(&target);
        self.pending
            .record(report.attribution, before.activity.as_ref(), Some(&head));
        Ok(report.outcome?)
    }

    fn discard(&mut self) -> AppResult<()> {
        let head = self.repo.activity_head()?;
        let before = snapshot(&self.repo)?;
        let preview = self
            .repo
            .prepare_discard(&file(), crate::git::DiscardMode::Unstaged)?;
        self.state
            .observe_activity(before.activity.clone().ok_or("Missing activity")?, self.now);
        let report = self.repo.discard_tracked(&preview);
        self.pending
            .record(report.attribution, before.activity.as_ref(), Some(&head));
        report.outcome?;
        Ok(())
    }

    fn refresh(&mut self) -> AppResult<BTreeSet<PathBuf>> {
        let guard = self.pending.before_refresh(&self.repo);
        let after = Ok(snapshot(&self.repo)?);
        let quiet = self.finish(&guard, &after)?;
        let request = self.state.collection_request();
        self.state
            .collection_finished(request.generation, after, self.now);
        self.state
            .advance_activity(self.now + Duration::from_secs(1));
        Ok(quiet)
    }

    fn finish(
        &mut self,
        guard: &BTreeSet<PathBuf>,
        after: &Result<Snapshot, GitError>,
    ) -> AppResult<BTreeSet<PathBuf>> {
        match self.pending.finish(&self.repo, guard, after) {
            Some(Update::InternalActivity(paths, observations, external)) => {
                self.state.internal_activity(&paths, observations);
                self.state.highlight_activity(&external, self.now);
                Ok(paths)
            }
            None => Ok(BTreeSet::new()),
            _ => Err("Unexpected mutation update".into()),
        }
    }

    fn intensity(&self, path: &str) -> f32 {
        self.state.activity_intensity(Path::new(path), false)
    }
}

fn snapshot(repo: &Repository) -> AppResult<Snapshot> {
    Ok(repo.snapshot(&Mode::Watch, Limits::default(), &BTreeSet::new())?)
}
fn file() -> StageTarget {
    StageTarget::File("tracked.txt".into())
}
fn working() -> AppResult<Fixture> {
    let fixture = Fixture::new("")?;
    fixture.write("tracked.txt", "working\n")?;
    Ok(fixture)
}
fn staged() -> AppResult<Fixture> {
    let fixture = working()?;
    fixture.git(&["add", "tracked.txt"])?;
    Ok(fixture)
}

#[test]
fn internal_stage_is_quiet() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    // When
    scenario.stage(file())?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn internal_unstage_is_quiet() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(staged()?)?;
    // When
    scenario.stage(file())?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn partial_staging_is_quiet() -> AppResult<()> {
    // Given
    let fixture = staged()?;
    fixture.write("tracked.txt", "remaining\n")?;
    let mut scenario = Scenario::new(fixture)?;
    // When
    scenario.stage(file())?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn directory_staging_only_quiets_index_entries_that_changed() -> AppResult<()> {
    // Given
    let fixture = staged()?;
    fixture.write("target/deep/new", "new")?;
    let mut scenario = Scenario::new(fixture)?;
    // When
    scenario.stage(StageTarget::Subtree("".into()))?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(quiet, BTreeSet::from(["target/deep/new".into()]));
    assert_eq!(scenario.intensity("target/deep/new"), 0.0);
    Ok(())
}

#[test]
fn unstaging_rename_quiets_both_endpoints() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    fixture.git(&["mv", "tracked.txt", "target/renamed"])?;
    let mut scenario = Scenario::new(fixture)?;
    // When
    scenario.stage(StageTarget::Subtree("target".into()))?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(
        quiet,
        BTreeSet::from(["tracked.txt".into(), "target/renamed".into()])
    );
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    assert_eq!(scenario.intensity("target/renamed"), 0.0);
    Ok(())
}

#[test]
fn save_after_stage_before_refresh_highlights_same_file() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    // When
    scenario
        .fixture
        .write("tracked.txt", "external save after staging\n")?;
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn save_after_unstage_before_refresh_highlights_same_file() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(staged()?)?;
    scenario.stage(file())?;
    // When
    scenario
        .fixture
        .write("tracked.txt", "external save after unstaging\n")?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn external_unstage_after_internal_stage_highlights_same_file() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    // When
    scenario
        .fixture
        .git(&["reset", "--quiet", "--", "tracked.txt"])?;
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    // The receipt observed the intermediate index, even though the final state
    // matches the original baseline.
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn external_staging_different_contents_after_internal_stage_highlights() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    // When
    scenario
        .fixture
        .write("tracked.txt", "external staged contents\n")?;
    scenario.fixture.git(&["add", "tracked.txt"])?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn unobserved_external_edit_before_the_command_is_not_quieted() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    let head = scenario.repo.activity_head()?;
    let before = snapshot(&scenario.repo)?;
    scenario
        .fixture
        .write("tracked.txt", "edit between observation and command\n")?;
    // When
    let report = scenario.repo.toggle_stage_tracked(&file());
    report.outcome?;
    scenario
        .pending
        .record(report.attribution, before.activity.as_ref(), Some(&head));
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn external_edit_observed_before_internal_staging_keeps_its_highlight() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario
        .fixture
        .write("tracked.txt", "external edit awaiting collection\n")?;
    // When
    scenario.stage(file())?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn edit_during_snapshot_collection_disables_suppression() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    let guard = scenario.pending.before_refresh(&scenario.repo);
    let after = Ok(snapshot(&scenario.repo)?);
    // When
    scenario
        .fixture
        .write("tracked.txt", "edit after snapshot read\n")?;
    let quiet = scenario.finish(&guard, &after)?;
    // Then
    assert!(quiet.is_empty());
    scenario.refresh()?;
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn unrelated_external_change_still_highlights_during_root_staging() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(StageTarget::Subtree("".into()))?;
    // When
    scenario.fixture.write("other", "external file")?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    assert_eq!(scenario.intensity("other"), 1.0);
    Ok(())
}

#[test]
fn head_change_invalidates_mutation_attribution() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    // When
    scenario
        .fixture
        .git(&["commit", "--allow-empty", "-m", "external commit"])?;
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    Ok(())
}

#[test]
fn failed_command_cannot_claim_a_later_external_index_change() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.fixture.write(".git/index.lock", "")?;
    assert!(scenario.stage(file()).is_err());
    std::fs::remove_file(scenario.repo.git_dir.join("index.lock"))?;
    // When
    scenario.fixture.git(&["add", "tracked.txt"])?;
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn no_op_has_no_attribution() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(Fixture::new("")?)?;
    // When
    let result = scenario.stage(StageTarget::Subtree("".into()))?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(result, StageOutcome::Unchanged);
    assert!(quiet.is_empty());
    Ok(())
}

#[test]
fn successful_collection_retry_preserves_confirmed_attribution() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    let guard = scenario.pending.before_refresh(&scenario.repo);
    let failed = Err(GitError::Command("collection failed".into()));
    // When
    let failed_update = scenario.pending.finish(&scenario.repo, &guard, &failed);
    let quiet = scenario.refresh()?;
    // Then
    assert!(failed_update.is_none());
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn external_edit_during_collection_retry_is_not_absorbed() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    let guard = scenario.pending.before_refresh(&scenario.repo);
    let failed = Err(GitError::Command("collection failed".into()));
    scenario.pending.finish(&scenario.repo, &guard, &failed);
    // When
    scenario
        .fixture
        .write("tracked.txt", "external edit during retry\n")?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn missing_precommand_observations_leave_highlights_enabled() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    // When
    let report = scenario.repo.toggle_stage_tracked(&file());
    report.outcome?;
    scenario.pending.record(report.attribution, None, None);
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn external_index_change_between_observation_and_command_is_not_quieted() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    let head = scenario.repo.activity_head()?;
    let before = snapshot(&scenario.repo)?;
    scenario.fixture.git(&["add", "tracked.txt"])?;
    // When
    let report = scenario.repo.toggle_stage_tracked(&file());
    report.outcome?;
    scenario
        .pending
        .record(report.attribution, before.activity.as_ref(), Some(&head));
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn head_change_between_observation_and_command_is_not_quieted() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    let head = scenario.repo.activity_head()?;
    let before = snapshot(&scenario.repo)?;
    scenario
        .fixture
        .git(&["commit", "--allow-empty", "-m", "external HEAD"])?;
    // When
    let report = scenario.repo.toggle_stage_tracked(&file());
    report.outcome?;
    scenario
        .pending
        .record(report.attribution, before.activity.as_ref(), Some(&head));
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn retry_does_not_restore_suppression_after_contradictory_evidence() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    let staged_index = std::fs::read(scenario.repo.git_dir.join("index"))?;
    scenario
        .fixture
        .git(&["reset", "--quiet", "--", "tracked.txt"])?;
    let guard = scenario.pending.before_refresh(&scenario.repo);
    scenario.pending.finish(
        &scenario.repo,
        &guard,
        &Err(GitError::Command("failed collection".into())),
    );
    // When
    std::fs::write(scenario.repo.git_dir.join("index"), staged_index)?;
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn unloaded_files_have_enough_metadata_for_quiet_internal_staging() -> AppResult<()> {
    // Given
    let fixture = working()?;
    let repo = fixture.repo()?;
    let limits = Limits { bytes: 1, lines: 1 };
    let before = repo.snapshot(&Mode::Watch, limits, &BTreeSet::new())?;
    let head = repo.activity_head()?;
    let mut pending = Mutations::default();
    // When
    let report = repo.toggle_stage_tracked(&file());
    report.outcome?;
    pending.record(report.attribution, before.activity.as_ref(), Some(&head));
    let guard = pending.before_refresh(&repo);
    let after = Ok(repo.snapshot(&Mode::Watch, limits, &BTreeSet::new())?);
    let update = pending.finish(&repo, &guard, &after);
    // Then
    let Some(Update::InternalActivity(quiet, _, external)) = update else {
        return Err("Missing activity update".into());
    };
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert!(external.is_empty());
    Ok(())
}

#[test]
fn whitespace_hidden_changes_are_staged_quietly() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    fixture.write("tracked.txt", "original  \n")?;
    let repo = fixture.repo()?;
    let before = repo.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        crate::config::Whitespace::IgnoreAll,
    )?;
    let head = repo.activity_head()?;
    let mut pending = Mutations::default();
    assert!(before.changes.is_empty());
    // When
    let report = repo.toggle_stage_tracked(&file());
    report.outcome?;
    pending.record(report.attribution, before.activity.as_ref(), Some(&head));
    let guard = pending.before_refresh(&repo);
    let after = Ok(repo.snapshot_with_whitespace(
        &Mode::Watch,
        Limits::default(),
        &BTreeSet::new(),
        crate::config::Whitespace::IgnoreAll,
    )?);
    let update = pending.finish(&repo, &guard, &after);
    // Then
    let Some(Update::InternalActivity(quiet, _, external)) = update else {
        return Err("Missing activity update".into());
    };
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert!(external.is_empty());
    Ok(())
}

#[test]
fn conflict_resolution_keeps_internal_activity_quiet() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    fixture.git(&["checkout", "-b", "other"])?;
    fixture.write("tracked.txt", "other branch\n")?;
    fixture.git(&["commit", "-am", "other"])?;
    fixture.git(&["checkout", "main"])?;
    fixture.write("tracked.txt", "main branch\n")?;
    fixture.git(&["commit", "-am", "main"])?;
    assert!(fixture.git(&["merge", "other"]).is_err());
    let mut scenario = Scenario::new(fixture)?;
    // When
    scenario.stage(file())?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn delayed_refresh_does_not_restart_internal_activity() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    scenario.refresh()?;
    // When
    scenario.now += Duration::from_secs(2);
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[cfg(unix)]
#[test]
fn executable_bit_change_during_refresh_is_external_activity() -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.fixture.git(&["config", "core.filemode", "true"])?;
    scenario.stage(file())?;
    // When
    std::fs::set_permissions(
        scenario.fixture.root.join("tracked.txt"),
        std::fs::Permissions::from_mode(0o755),
    )?;
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn binary_files_use_working_metadata_without_loading_text() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    std::fs::write(fixture.root.join("tracked.txt"), b"binary\0contents")?;
    let mut scenario = Scenario::new(fixture)?;
    // When
    scenario.stage(file())?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn an_internal_unstage_does_not_compare_index_blob_text_as_working_text() -> AppResult<()> {
    // Given
    let fixture = Fixture::new("")?;
    fixture.write(".git/info/attributes", "tracked.txt text eol=lf\n")?;
    fixture.write("tracked.txt", "normalized\r\n")?;
    fixture.git(&["add", "tracked.txt"])?;
    let mut scenario = Scenario::new(fixture)?;
    // When
    scenario.stage(file())?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn successful_retry_cannot_absorb_a_new_external_index_transition() -> AppResult<()> {
    // Given
    let mut scenario = Scenario::new(working()?)?;
    scenario.stage(file())?;
    let guard = scenario.pending.before_refresh(&scenario.repo);
    scenario.pending.finish(
        &scenario.repo,
        &guard,
        &Err(GitError::Command("retry".into())),
    );
    // When
    scenario
        .fixture
        .git(&["reset", "--quiet", "--", "tracked.txt"])?;
    scenario.refresh()?;
    // Then
    assert_eq!(scenario.intensity("tracked.txt"), 1.0);
    Ok(())
}

#[test]
fn internal_unstaged_discard_keeps_retained_staged_file_quiet() -> AppResult<()> {
    // Given
    let fixture = staged()?;
    fixture.write("tracked.txt", "unstaged edit\n")?;
    let mut scenario = Scenario::new(fixture)?;
    // When
    scenario.discard()?;
    let quiet = scenario.refresh()?;
    // Then
    assert_eq!(quiet, BTreeSet::from(["tracked.txt".into()]));
    assert_eq!(scenario.intensity("tracked.txt"), 0.0);
    Ok(())
}

#[test]
fn external_edit_after_discard_still_highlights() -> AppResult<()> {
    // Given
    let fixture = staged()?;
    fixture.write("tracked.txt", "unstaged edit\n")?;
    let mut scenario = Scenario::new(fixture)?;
    scenario.discard()?;
    // When
    scenario.fixture.write("tracked.txt", "external edit\n")?;
    let quiet = scenario.refresh()?;
    // Then
    assert!(quiet.is_empty());
    assert!(scenario.intensity("tracked.txt") > 0.0);
    Ok(())
}

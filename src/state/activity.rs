use crate::model::ActivityObservations;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ATTACK: Duration = Duration::from_millis(150);
const LIFETIME: Duration = Duration::from_secs(32);
const HOLD_UNTIL: Duration = LIFETIME.saturating_sub(Duration::from_secs(2));
const DISMISS: Duration = Duration::from_millis(200);

struct Pulse {
    started: Instant,
    from: f32,
    dismissed: Option<(Instant, f32)>,
}

impl Pulse {
    fn intensity(&self, now: Instant) -> f32 {
        if let Some((started, from)) = self.dismissed {
            return from * (1.0 - progress(now.saturating_duration_since(started), DISMISS));
        }
        let age = now.saturating_duration_since(self.started);
        if age < ATTACK {
            self.from + (1.0 - self.from) * progress(age, ATTACK)
        } else if age < HOLD_UNTIL {
            1.0
        } else {
            1.0 - progress(age.saturating_sub(HOLD_UNTIL), LIFETIME - HOLD_UNTIL)
        }
    }

    fn finished(&self, now: Instant) -> bool {
        now >= self.started + LIFETIME
            || self
                .dismissed
                .is_some_and(|(started, _)| now >= started + DISMISS)
    }
}

fn progress(elapsed: Duration, duration: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Default)]
pub(super) struct ActivityState {
    baseline: Option<ActivityObservations>,
    pulses: BTreeMap<PathBuf, Pulse>,
    now: Option<Instant>,
}

impl ActivityState {
    pub(super) fn rebase(&mut self, paths: &BTreeSet<PathBuf>, observations: ActivityObservations) {
        self.pulses
            .retain(|path, _| !paths.contains(path) || observations.contains_key(path));
        if let Some(baseline) = &mut self.baseline {
            baseline.retain(|path, _| !paths.contains(path));
            baseline.extend(observations);
        }
    }

    pub(super) fn observe(&mut self, observations: Option<ActivityObservations>, now: Instant) {
        let Some(observations) = observations else {
            return;
        };
        self.pulses
            .retain(|path, pulse| observations.contains_key(path) && !pulse.finished(now));
        if let Some(previous) = &self.baseline {
            for (path, categories) in &observations {
                let changed = previous.get(path).is_none_or(|before| {
                    before.len() != categories.len()
                        || categories.iter().any(|(category, observation)| {
                            before
                                .get(category)
                                .is_none_or(|before| observation.changed_since(before))
                        })
                });
                if changed {
                    start_pulse(&mut self.pulses, path, now);
                }
            }
        }
        self.baseline = Some(observations);
        self.now = Some(now);
    }

    pub(super) fn highlight(&mut self, paths: &BTreeSet<PathBuf>, now: Instant) {
        for path in paths {
            start_pulse(&mut self.pulses, path, now);
        }
        self.now = Some(now);
    }

    pub(super) fn advance(&mut self, now: Instant) -> bool {
        let changed = self.pulses.values().any(|pulse| {
            self.now
                .is_none_or(|before| pulse.intensity(before) != pulse.intensity(now))
                || pulse.finished(now)
        });
        self.pulses.retain(|_, pulse| !pulse.finished(now));
        self.now = Some(now);
        changed
    }

    pub(super) fn visit(&mut self, path: &Path, now: Instant) {
        if let Some(pulse) = self.pulses.get_mut(path)
            && pulse.dismissed.is_none()
            && !pulse.finished(now)
        {
            pulse.dismissed = Some((now, pulse.intensity(now)));
        }
        self.now = Some(now);
    }

    pub(super) fn intensity(
        &self,
        path: &Path,
        directory: bool,
        matching: &BTreeSet<PathBuf>,
    ) -> f32 {
        let Some(now) = self.now else { return 0.0 };
        if directory {
            self.pulses
                .iter()
                .filter(|(file, _)| file.starts_with(path) && matching.contains(*file))
                .map(|(_, pulse)| pulse.intensity(now) * 0.5)
                .fold(0.0, f32::max)
        } else {
            self.pulses
                .get(path)
                .map_or(0.0, |pulse| pulse.intensity(now))
        }
    }
}

fn start_pulse(pulses: &mut BTreeMap<PathBuf, Pulse>, path: &Path, now: Instant) {
    let from = pulses.get(path).map_or(0.0, |pulse| pulse.intensity(now));
    pulses.insert(
        path.to_owned(),
        Pulse {
            started: now,
            from,
            dismissed: None,
        },
    );
}

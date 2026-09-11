//! Bounded activity context between screenshots, independent of model scheduling.
use crate::models::{Activity, ActivityHistory, ActivityPoint, AppDwell, AppSwitch};

const MAX_POINTS: usize = 120;
const MAX_SWITCHES: usize = 500;
const MAX_AGE_MS: i64 = 600_000;

#[derive(Default)]
pub struct ActivityTracker {
    points: Vec<ActivityPoint>,
    switches: Vec<AppSwitch>,
    truncated: bool,
}
impl ActivityTracker {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn observe(&mut self, at: i64, activity: &Activity, excluded: &[String]) {
        if excluded.contains(&activity.bundle_id)
            || activity
                .switches
                .iter()
                .any(|s| excluded.contains(&s.bundle_id))
        {
            // A briefly visited excluded app can occur entirely between two polls.
            // Drop this interval too, rather than forwarding its event metadata.
            self.clear();
            return;
        }
        if self
            .points
            .last()
            .is_some_and(|p| at < p.at || at - p.at > 15_000)
        {
            self.clear();
        }
        let since = self.points.first().map(|p| p.at).unwrap_or(at);
        self.switches.extend(
            activity
                .switches
                .iter()
                .filter(|s| s.at > since && s.at <= at)
                .cloned(),
        );
        self.switches.sort_by_key(|s| s.at);
        self.switches
            .dedup_by(|a, b| a.at == b.at && a.bundle_id == b.bundle_id);
        let point = ActivityPoint {
            at,
            app_name: activity.app_name.clone(),
            bundle_id: activity.bundle_id.clone(),
            idle_seconds: if activity.idle_seconds.is_finite() {
                activity.idle_seconds.max(0.0)
            } else {
                0.0
            },
        };
        if self.points.last().is_some_and(|p| p.at == at) {
            self.points.pop();
        }
        self.points.push(point);
        while self.points.len() > MAX_POINTS
            || self.points.first().is_some_and(|p| at - p.at > MAX_AGE_MS)
        {
            self.points.remove(0);
            self.truncated = true;
        }
        // If event volume exceeds the cap, clip the whole interval at an observed
        // point, so omitted switches cannot become invented app dwell time.
        while self.switches.len() > MAX_SWITCHES && self.points.len() > 1 {
            self.points.remove(0);
            self.truncated = true;
            let start = self.points[0].at;
            self.switches.retain(|s| s.at > start);
        }
        let start = self.points[0].at;
        self.switches.retain(|s| s.at > start);
        if self.switches.len() > MAX_SWITCHES {
            self.switches.clear();
            self.truncated = true;
        }
    }
    pub fn history(&self) -> Option<ActivityHistory> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        let mut spans = Vec::new();
        let (mut start, mut name, mut bundle) =
            (first.at, first.app_name.clone(), first.bundle_id.clone());
        // Poll snapshots repair missing activation notifications, but their times
        // remain estimates. Idle values are checkpoints, never typing durations.
        let mut changes: Vec<_> = self
            .switches
            .iter()
            .map(|s| (s.at, 0, &s.app_name, &s.bundle_id))
            .collect();
        changes.extend(
            self.points
                .iter()
                .skip(1)
                .map(|p| (p.at, 1, &p.app_name, &p.bundle_id)),
        );
        changes.sort_by_key(|c| (c.0, c.1));
        for (at, _, next_name, next_bundle) in changes {
            if *next_bundle == bundle {
                continue;
            }
            if at > start {
                spans.push(AppDwell {
                    start,
                    end: at,
                    app_name: name,
                    bundle_id: bundle,
                });
            }
            start = at;
            name = next_name.clone();
            bundle = next_bundle.clone();
        }
        if last.at > start {
            spans.push(AppDwell {
                start,
                end: last.at,
                app_name: name,
                bundle_id: bundle,
            });
        }
        Some(ActivityHistory {
            start: first.at,
            end: last.at,
            truncated: self.truncated,
            app_spans: spans,
            input_checkpoints: self.points.clone(),
            switches: self.switches.clone(),
        })
    }
    pub fn begin_after_capture(&mut self, at: i64, activity: &Activity, excluded: &[String]) {
        self.clear();
        self.observe(at, activity, excluded);
    }
}

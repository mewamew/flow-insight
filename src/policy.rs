//! Fixed sampling cadence. Local evidence can pause sampling, never classify work.
use crate::models::{Activity, Presence, PresenceRegion, PresenceState};
use serde::{Deserialize, Serialize};

pub const POLL_SECONDS: u32 = 5;
pub const ANALYSIS_INTERVAL_SECONDS: u32 = 60;
pub const JUDGMENT_TTL_MS: i64 = 2 * ANALYSIS_INTERVAL_SECONDS as i64 * 1000;
pub const OBSERVATION_GAP_MS: i64 = 10_000;
const AWAY_MS: i64 = 45_000;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Observation {
    pub captured_at: i64,
    #[serde(default)]
    pub activity: Activity,
    #[serde(default)]
    pub presence: Presence,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    /// Live detection geometry for the page; not part of the persisted presence record.
    #[serde(default)]
    pub presence_regions: Vec<PresenceRegion>,
    #[serde(default)]
    pub camera_active: bool,
    #[serde(default)]
    pub camera_device: String,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Decision {
    pub analyze: bool,
    pub away: bool,
    pub reason: String,
    pub next_check_seconds: u32,
}
#[derive(Default)]
pub struct Policy {
    last_poll: Option<i64>,
    last_attempt: Option<i64>,
    missing_since: Option<i64>,
}
impl Policy {
    pub fn decide(&mut self, o: &Observation) -> Decision {
        let at = o.captured_at;
        if self
            .last_poll
            .is_some_and(|last| at - last > OBSERVATION_GAP_MS || at < last)
        {
            self.missing_since = None;
        }
        if self.last_poll.is_some_and(|last| at < last) {
            self.last_attempt = None;
        }
        self.last_poll = Some(at);
        if let Some(reason) = &o.blocked_reason {
            self.missing_since = None;
            return decision(false, false, reason, 0);
        }
        let away =
            if o.presence.state == PresenceState::NotDetected && o.activity.idle_seconds >= 5.0 {
                let since = *self.missing_since.get_or_insert(at);
                at - since >= AWAY_MS && o.activity.idle_seconds >= 45.0
            } else {
                self.missing_since = None;
                false
            };
        if away {
            return decision(false, true, "away", 0);
        }
        let interval = i64::from(ANALYSIS_INTERVAL_SECONDS) * 1000;
        let wait = self
            .last_attempt
            .map(|last| (interval - (at - last)).max(0))
            .unwrap_or(0);
        if wait > 0 {
            return decision(false, false, "waiting", ((wait + 999) / 1000) as u32);
        }
        let reason = if self.last_attempt.is_none() {
            "started"
        } else {
            "periodic"
        };
        // A failed or busy attempt also consumes this slot. No event-based retries.
        self.last_attempt = Some(at);
        decision(true, false, reason, ANALYSIS_INTERVAL_SECONDS)
    }
}
fn decision(analyze: bool, away: bool, reason: &str, next_check_seconds: u32) -> Decision {
    Decision {
        analyze,
        away,
        reason: reason.into(),
        next_check_seconds,
    }
}

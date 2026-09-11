use crate::{
    models::{Category, Report, Sample, Session},
    store::{Result, Store},
};
use chrono::{Local, NaiveDate, TimeZone};
use serde::Serialize;
use std::collections::HashMap;

// Keep measured coverage separate from the state estimated between screenshots.
const SAMPLE_JITTER_MS: i64 = 1_000;
use crate::policy::JUDGMENT_TTL_MS;
pub use crate::policy::OBSERVATION_GAP_MS;

#[derive(Serialize, Clone)]
pub struct Segment {
    pub start: i64,
    pub end: i64,
    pub category: Category,
    pub sample: Sample,
    /// Original screenshot supporting an inferred interval. Raw samples stay intact.
    pub estimated_from: Option<Sample>,
}
#[derive(Serialize)]
pub struct Day {
    pub date: String,
    pub mode: String,
    pub segments: Vec<Segment>,
    pub counts: HashMap<String, u32>,
    pub work_seconds: i64,
    pub distracted_seconds: i64,
    pub away_seconds: i64,
    pub local_seconds: i64,
    pub ai_observed_seconds: i64,
    pub unknown_seconds: i64,
    pub observed_seconds: i64,
    pub unobserved_seconds: i64,
    pub longest_work_seconds: i64,
    pub fingerprint: String,
    pub report: Option<Report>,
    pub report_stale: bool,
}
pub fn bounds(date: &str) -> Result<(i64, i64)> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "日期格式应为 YYYY-MM-DD")?;
    let start = Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .ok_or("日期无效")?;
    let end = Local
        .from_local_datetime(
            &d.succ_opt()
                .ok_or("日期无效")?
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .earliest()
        .ok_or("日期无效")?;
    Ok((start.timestamp_millis(), end.timestamp_millis()))
}
pub fn build(
    date: &str,
    mode: &str,
    samples: Vec<Sample>,
    sessions: Vec<Session>,
    now: i64,
) -> Result<Day> {
    let (start, end) = bounds(date)?;
    let sessions: HashMap<_, _> = sessions.into_iter().map(|s| (s.id.clone(), s)).collect();
    let mut samples: Vec<_> = samples.into_iter().filter(|s| s.mode == mode).collect();
    samples.sort_by_key(|s| s.captured_at);
    let mut segments = Vec::new();
    let mut counts = HashMap::new();
    let mut estimate: Option<&Sample> = None;
    let mut previous_end = 0;
    let mut previous_session = "";
    let (mut observed, mut local) = (0, 0);
    for (i, s) in samples.iter().enumerate() {
        let next = samples
            .get(i + 1)
            .map(|s| s.captured_at)
            .unwrap_or(i64::MAX);
        let stop = sessions
            .get(&s.session_id)
            .and_then(|s| s.ended_at)
            .unwrap_or(now);
        let raw_start = s.captured_at;
        let b = s
            .evidence
            .as_ref()
            .map(|e| e.valid_until)
            .unwrap_or(s.captured_at + s.interval_seconds as i64 * 1000)
            .min(next)
            .min(stop)
            .min(end)
            .min(now);
        let mut category = s.category();
        let blocked = s.evidence.as_ref().is_some_and(|e| {
            e.away
                || [
                    "locked",
                    "excluded",
                    "screen_permission",
                    "unavailable",
                    "capture_failed",
                    "model_unavailable",
                ]
                .contains(&e.trigger.as_str())
        }) || ["error", "skipped"].contains(&s.state.as_str());
        if s.session_id != previous_session
            || raw_start - previous_end > OBSERVATION_GAP_MS
            || blocked
            || s.evidence.is_none()
        {
            estimate = None;
        }
        let local_sample = s.evidence.as_ref().is_some_and(|e| e.basis == "local");
        let completed = !local_sample && (s.analysis.is_some() || s.correction.is_some());
        if completed {
            estimate = if !blocked
                && s.evidence.is_some()
                && matches!(category, Category::Work | Category::Distracted)
            {
                Some(s)
            } else {
                None
            };
        }
        previous_end = b.max(raw_start);
        previous_session = &s.session_id;
        let a = raw_start.max(start);
        if b <= a {
            continue;
        }
        observed += b - a;
        if local_sample {
            local += b - a;
        }
        *counts.entry(s.state.clone()).or_insert(0) += 1;
        // One deadline applies to both the screenshot and any inherited state.
        // A long legacy observation must not bypass expiry on its own sample.
        if completed
            && s.evidence.is_some()
            && matches!(category, Category::Work | Category::Distracted)
        {
            category = Category::Unknown;
        }
        let source = estimate.filter(|source| !blocked && a < source.captured_at + JUDGMENT_TTL_MS);
        if let Some(source) = source {
            let until = b.min(source.captured_at + JUDGMENT_TTL_MS);
            segments.push(Segment {
                start: a,
                end: until,
                category: source.category(),
                sample: s.clone(),
                estimated_from: (!completed).then(|| source.clone()),
            });
            if until == b {
                continue;
            }
            segments.push(Segment {
                start: until,
                end: b,
                category,
                sample: s.clone(),
                estimated_from: None,
            });
            continue;
        }
        segments.push(Segment {
            start: a,
            end: b,
            category,
            sample: s.clone(),
            estimated_from: None,
        });
    }
    let (mut work, mut distracted, mut unknown, mut longest, mut run, mut prev, mut away) =
        (0, 0, 0, 0, 0, 0, 0);
    let mut previous_session = "";
    for s in &segments {
        let n = s.end - s.start;
        match s.category {
            Category::Work => {
                work += n;
                let jitter = if s.sample.evidence.is_some() {
                    OBSERVATION_GAP_MS
                } else {
                    SAMPLE_JITTER_MS
                };
                if s.sample.session_id == previous_session && s.start - prev <= jitter {
                    run += n
                } else {
                    run = n
                }
                longest = longest.max(run)
            }
            Category::Distracted => {
                distracted += n;
                run = 0
            }
            Category::Away => {
                away += n;
                run = 0;
            }
            Category::Unknown => {
                unknown += n;
                run = 0
            }
        }
        prev = s.end;
        previous_session = &s.sample.session_id;
    }
    let span = segments
        .last()
        .zip(segments.first())
        .map(|(b, a)| b.end - a.start)
        .unwrap_or(0);
    // Include the result and user corrections so an old AI report is visibly stale.
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&segments)
        .unwrap_or_default()
        .hash(&mut hash);
    Ok(Day {
        date: date.into(),
        mode: mode.into(),
        segments,
        counts,
        work_seconds: work / 1000,
        distracted_seconds: distracted / 1000,
        away_seconds: away / 1000,
        local_seconds: local / 1000,
        ai_observed_seconds: (observed - local) / 1000,
        unknown_seconds: unknown / 1000,
        observed_seconds: observed / 1000,
        unobserved_seconds: (span - observed).max(0) / 1000,
        longest_work_seconds: longest / 1000,
        fingerprint: hash.finish().to_string(),
        report: None,
        report_stale: false,
    })
}
pub fn day(store: &Store, date: &str, mode: &str, now: i64) -> Result<Day> {
    if mode != "live" {
        return Err("无效的数据类型".into());
    }
    let mut d = build(
        date,
        mode,
        store.list("sample", Some(mode))?,
        store.list("session", Some(mode))?,
        now,
    )?;
    d.report = store.get("report", &format!("{mode}:{date}"))?;
    d.report_stale = d
        .report
        .as_ref()
        .is_some_and(|r| r.fingerprint != d.fingerprint);
    Ok(d)
}

//! Cross-day views reuse the daily estimator; no additional model calls or stored judgments.
use crate::{
    api::{ApiResult, App},
    flow,
    models::{now, Sample, Session},
    store::{Result, Store},
    summary,
};
use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{Days, Local, NaiveDate, TimeZone, Timelike};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub const MIN_DAYS: usize = 3;
pub const MIN_JUDGED_SECONDS: i64 = 600;

#[derive(Deserialize)]
pub struct RangeQuery {
    pub date: String,
    #[serde(default = "default_days")]
    pub days: u32,
    #[serde(default = "default_mode")]
    pub mode: String,
}
fn default_days() -> u32 {
    7
}
fn default_mode() -> String {
    "live".into()
}

pub fn build(store: &Store, query: &RangeQuery, at: i64) -> Result<Value> {
    if ![7, 14, 21].contains(&query.days) {
        return Err("请选择 7、14 或 21 天".into());
    }
    if query.mode != "live" {
        return Err("无效的数据类型".into());
    }
    let end = NaiveDate::parse_from_str(&query.date, "%Y-%m-%d")
        .map_err(|_| "日期格式应为 YYYY-MM-DD")?;
    let start = end
        .checked_sub_days(Days::new(u64::from(query.days - 1)))
        .ok_or("日期超出范围")?;
    // One read per object kind, rather than reopening the entire store for every day.
    let samples: Vec<Sample> = store.list("sample", Some(&query.mode))?;
    let sessions: Vec<Session> = store.list("session", Some(&query.mode))?;
    let mut rows = Vec::new();
    let mut eligible: Vec<Vec<(f64, i64, i64)>> = vec![Vec::new(); 24];
    for offset in 0..query.days {
        let date = start
            .checked_add_days(Days::new(u64::from(offset)))
            .ok_or("日期超出范围")?
            .to_string();
        let (_, day_end) = summary::bounds(&date)?;
        // Include all earlier samples so midnight continuation and legacy long intervals
        // are exactly the same as /api/flow. summary::build clips to the requested day.
        let day = summary::build(
            &date,
            &query.mode,
            samples
                .iter()
                .filter(|s| s.captured_at < day_end)
                .cloned()
                .collect(),
            sessions.clone(),
            at,
        )?;
        let metrics = flow::derived(&day.segments);
        let mut hours = metrics["hourly"].as_array().unwrap().clone();
        for (hour, cell) in hours.iter_mut().enumerate() {
            let work = cell["work_seconds"].as_i64().unwrap();
            let judged = work + cell["distracted_seconds"].as_i64().unwrap();
            let enough = judged >= MIN_JUDGED_SECONDS;
            cell["eligible"] = json!(enough);
            cell["judged_seconds"] = json!(judged);
            if enough {
                eligible[hour].push((work as f64 / judged as f64, work, judged));
            }
        }
        // Return only a navigation pointer, never screenshots, titles or analysis text.
        for segment in &day.segments {
            let mut t = segment.start;
            while t < segment.end {
                let dt = Local
                    .timestamp_millis_opt(t)
                    .single()
                    .ok_or("记录时间无效")?;
                let cell = &mut hours[dt.hour() as usize];
                if cell["sample_key"].is_null() {
                    cell["sample_key"] = json!(format!("{}:{}", segment.sample.id, segment.start));
                    cell["sample_at"] = json!(t);
                }
                let step = (3_600_000
                    - i64::from(dt.minute()) * 60_000
                    - i64::from(dt.second()) * 1000
                    - i64::from(dt.timestamp_subsec_millis()))
                .max(1);
                t = (t + step).min(segment.end);
            }
        }
        rows.push(
            json!({"date":date,"hours":hours,"observed_seconds":day.observed_seconds,
            "work_seconds":day.work_seconds,"longest_work_seconds":day.longest_work_seconds}),
        );
    }
    let mut candidates: Vec<_> = eligible.iter().enumerate()
        .filter(|(_, values)| values.len() >= MIN_DAYS)
        .map(|(hour, values)| {
            let ratio = values.iter().map(|v| v.0).sum::<f64>() / values.len() as f64;
            json!({"hour":hour,"end_hour":hour+1,"eligible_days":values.len(),
                "work_ratio":ratio,"mean_work_seconds":values.iter().map(|v|v.1).sum::<i64>() / values.len() as i64,
                "judged_seconds":values.iter().map(|v|v.2).sum::<i64>()})
        }).collect();
    candidates.sort_by(|a, b| {
        a["work_ratio"]
            .as_f64()
            .partial_cmp(&b["work_ratio"].as_f64())
            .unwrap()
            .then_with(|| {
                a["mean_work_seconds"]
                    .as_i64()
                    .cmp(&b["mean_work_seconds"].as_i64())
            })
    });
    // Comparable coverage on multiple dates is required. Tiny differences don't establish a peak.
    let contrast = candidates.len() >= 2
        && candidates.last().unwrap()["work_ratio"].as_f64().unwrap()
            - candidates.first().unwrap()["work_ratio"].as_f64().unwrap()
            >= 0.05;
    let recorded_days = rows
        .iter()
        .filter(|d| d["observed_seconds"].as_i64().unwrap() > 0)
        .count();
    Ok(
        json!({"start_date":start.to_string(),"end_date":end.to_string(),"days":query.days,
        "mode":query.mode,"recorded_days":recorded_days,"retention_days":store.settings().retention_days,
        "minimum_days":MIN_DAYS,"minimum_judged_seconds":MIN_JUDGED_SECONDS,
        "status":if candidates.len()<2 {"insufficient"} else if contrast {"ready"} else {"similar"},
        "best_period":if contrast {candidates.last()} else {None},"rows":rows,
        "generated_at":at}),
    )
}

pub async fn range(
    State(app): State<Arc<App>>,
    Query(query): Query<RangeQuery>,
) -> ApiResult<Json<Value>> {
    // Large histories should not block the async recorder or HTTP executor.
    let result = tokio::task::spawn_blocking(move || build(&app.store, &query, now()))
        .await
        .map_err(|_| "跨日统计暂时不可用")??;
    Ok(Json(result))
}

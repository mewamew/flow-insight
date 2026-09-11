//! Derived observations keep measured activity separate from AI interpretation.
use crate::{
    api::{ApiResult, App},
    models::*,
    summary::{self, Segment},
};
use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{Local, TimeZone, Timelike};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub struct ReminderState {
    pub snoozed_until: i64,
    pub last_sent: i64,
    pub last_sample: String,
    pub error: Option<String>,
}
pub type ReminderLock = Mutex<ReminderState>;
#[derive(Deserialize)]
pub struct QueryDay {
    pub date: String,
    #[serde(default = "live")]
    pub mode: String,
}
fn live() -> String {
    "live".into()
}

pub fn trailing_run(segments: &[Segment], category: Category) -> i64 {
    let Some(last) = segments.last() else {
        return 0;
    };
    let mut edge = last.end;
    let mut total = 0;
    for s in segments.iter().rev() {
        if s.category != category
            || s.sample.session_id != last.sample.session_id
            || edge - s.end
                > if s.sample.evidence.is_some() {
                    summary::OBSERVATION_GAP_MS
                } else {
                    1500
                }
        {
            break;
        }
        total += s.end - s.start;
        edge = s.start;
    }
    total / 1000
}
pub fn derived(segments: &[Segment]) -> Value {
    let mut hours = vec![(0_i64, 0_i64, 0_i64, 0_i64); 24];
    let mut interruptions = 0;
    let mut previous: Option<&Segment> = None;
    let mut recoveries = Vec::new();
    let mut interruption_start = None;
    for s in segments {
        let contiguous = previous.is_some_and(|p| {
            p.sample.session_id == s.sample.session_id
                && s.start - p.end
                    <= if s.sample.evidence.is_some() {
                        summary::OBSERVATION_GAP_MS
                    } else {
                        1500
                    }
        });
        if s.category == Category::Distracted
            && (!contiguous || previous.is_none_or(|p| p.category != Category::Distracted))
        {
            interruptions += 1;
            interruption_start = Some((s.start, s.sample.id.clone()));
        }
        if !contiguous {
            interruption_start = if s.category == Category::Distracted {
                Some((s.start, s.sample.id.clone()))
            } else {
                None
            };
        }
        if s.category == Category::Work {
            if let Some((start, id)) = interruption_start.take() {
                recoveries.push(json!({"interruption_id":id,"recovered_id":s.sample.id,"start":start,"end":s.start,"seconds":(s.start-start)/1000}));
            }
        } else if s.category != Category::Distracted {
            interruption_start = None;
        }
        let mut t = s.start;
        while t < s.end {
            let dt = Local.timestamp_millis_opt(t).single().unwrap();
            let step = ((3600 - dt.minute() as i64 * 60 - dt.second() as i64) * 1000
                - dt.timestamp_subsec_millis() as i64)
                .max(1);
            let end = (t + step).min(s.end);
            let n = end - t;
            let h = &mut hours[dt.hour() as usize];
            match s.category {
                Category::Work => h.0 += n,
                Category::Distracted => h.1 += n,
                Category::Unknown => h.2 += n,
                Category::Away => h.3 += n,
            };
            t = end;
        }
        previous = Some(s);
    }
    let hourly:Vec<_>=hours.iter().enumerate().map(|(hour,(w,d,u,a))|json!({"hour":hour,"work_seconds":w/1000,"distracted_seconds":d/1000,"unknown_seconds":u/1000,"away_seconds":a/1000,"coverage_seconds":(w+d+u+a)/1000,"work_ratio":if w+d>0 {Some(*w as f64/(w+d) as f64)}else{None}})).collect();
    let mut eligible: Vec<_> = hourly
        .iter()
        .filter(|h| {
            h["work_seconds"].as_i64().unwrap() + h["distracted_seconds"].as_i64().unwrap() >= 600
        })
        .cloned()
        .collect();
    eligible.sort_by(|a, b| {
        a["work_ratio"]
            .as_f64()
            .partial_cmp(&b["work_ratio"].as_f64())
            .unwrap()
            .then_with(|| a["work_seconds"].as_i64().cmp(&b["work_seconds"].as_i64()))
    });
    // One hour cannot establish high/low contrast; equal ratios also convey no contrast.
    let contrast = eligible.len() >= 2
        && eligible.first().unwrap()["work_ratio"] != eligible.last().unwrap()["work_ratio"];
    let mut seen = HashSet::new();
    let records: Vec<_> = segments
        .iter()
        .filter(|s| seen.insert(&s.sample.id))
        .collect();
    json!({"hourly":hourly,"interruptions":interruptions,"current_run_seconds":trailing_run(segments,Category::Work),"high_period":if contrast{eligible.last()}else{None},"low_period":if contrast{eligible.first()}else{None},"recoveries":recoveries,"switch_count":records.iter().map(|s|if s.sample.evidence.is_some(){s.sample.activity.switches.len()}else{s.sample.activity.switches.len().saturating_sub(1)}).sum::<usize>(),"ai_samples":records.iter().filter(|s|!s.sample.screen_files().is_empty()).count(),"local_records":records.iter().filter(|s|s.sample.state=="local").count()})
}
pub async fn overview(
    State(app): State<Arc<App>>,
    Query(q): Query<QueryDay>,
) -> ApiResult<Json<Value>> {
    let day = summary::day(&app.store, &q.date, &q.mode, now())?;
    let metrics = derived(&day.segments);
    Ok(Json(json!({"day":day,"metrics":metrics})))
}
pub async fn reminders(State(app): State<Arc<App>>) -> Json<Value> {
    let r = app.reminders.lock().unwrap();
    Json(
        json!({"enabled":app.store.settings().reminders,"snoozed_until":r.snoozed_until,"last_sent":r.last_sent,"error":r.error}),
    )
}
#[derive(Deserialize)]
pub struct Snooze {
    #[serde(default)]
    pub minutes: u32,
}
pub async fn snooze(State(app): State<Arc<App>>, Json(s): Json<Snooze>) -> ApiResult<Json<Value>> {
    if s.minutes > 1440 {
        return Err("暂缓时间不能超过一天".into());
    }
    app.reminders.lock().unwrap().snoozed_until = now() + s.minutes as i64 * 60000;
    let _ = app
        .recorder
        .native
        .request(json!({"command":"dismiss_card"}))
        .await;
    Ok(Json(json!({"ok":true})))
}
pub async fn preview(State(app): State<Arc<App>>) -> ApiResult<Json<Value>> {
    app.recorder.native.request(json!({"command":"show_card","preview":true,"port":app.port,"title":"桌面轻提醒 · 预览","message":"当近期多次采样提示工作中断时，这里会轻声提醒。","sample_id":""})).await?;
    Ok(Json(json!({"ok":true})))
}
pub fn should_remind(segments: &[Segment], now: i64, threshold: i64) -> bool {
    let Some(s) = segments.last() else {
        return false;
    };
    if segments.iter().any(|s| s.sample.evidence.is_some()) {
        return sampled_distraction(segments, now, threshold);
    }
    s.sample.mode == "live"
        && s.sample.state == "done"
        && s.sample
            .analysis
            .as_ref()
            .is_some_and(|a| a.confidence >= 0.75)
        && now - s.sample.captured_at <= (s.sample.interval_seconds as i64 + 30) * 1000
        && trailing_run(segments, Category::Distracted) >= threshold
}

// Two comparable AI observations can support a gentle reminder, but cannot turn
// inferred intervals into additional independent AI observations.
fn sampled_distraction(segments: &[Segment], at: i64, threshold: i64) -> bool {
    let Some(last) = segments.iter().rev().find(|s| s.sample.analysis.is_some()) else {
        return false;
    };
    if at - last.sample.captured_at > 60_000
        || last.category != Category::Distracted
        || last.sample.mode != "live"
        || !last
            .sample
            .analysis
            .as_ref()
            .is_some_and(|a| a.confidence >= 0.75)
    {
        return false;
    }
    let mut count = 0;
    let mut edge = segments.last().unwrap().end;
    for s in segments.iter().rev() {
        if s.sample.session_id != last.sample.session_id
            || edge - s.end > 10_000
            || s.sample.activity.bundle_id != last.sample.activity.bundle_id
            || s.sample.activity.window_title != last.sample.activity.window_title
        {
            break;
        }
        if s.sample.evidence.as_ref().is_some_and(|e| {
            e.away || ["locked", "excluded", "screen_permission"].contains(&e.trigger.as_str())
        }) {
            break;
        }
        if s.sample.analysis.is_some() {
            if s.category != Category::Distracted
                || !s
                    .sample
                    .analysis
                    .as_ref()
                    .is_some_and(|a| a.confidence >= 0.75)
            {
                break;
            }
            count += 1;
            if count >= 2 && last.start - s.start >= threshold * 1000 {
                return true;
            }
        }
        edge = s.start;
    }
    false
}

pub async fn maybe_remind(app: &Arc<App>) {
    let settings = app.store.settings();
    if !settings.reminders || !app.recorder.snapshot().running {
        return;
    }
    let date = Local::now().format("%Y-%m-%d").to_string();
    let Ok(day) = summary::day(&app.store, &date, "live", now()) else {
        return;
    };
    if !should_remind(&day.segments, now(), settings.reminder_minutes as i64 * 60) {
        return;
    }
    let last = day
        .segments
        .iter()
        .rev()
        .find(|s| s.sample.analysis.is_some())
        .unwrap();
    {
        let mut r = app.reminders.lock().unwrap();
        if now() < r.snoozed_until
            || now() - r.last_sent < 15 * 60000
            || r.last_sample == last.sample.id
        {
            return;
        }
        r.last_sent = now();
        r.last_sample = last.sample.id.clone();
    }
    let description = if last.sample.task.trim().is_empty() {
        "近期采样多次显示可能在进行非工作活动"
    } else {
        "近期采样多次提示可能偏离填写的任务"
    };
    let result=app.recorder.native.request(json!({"command":"show_card","port":app.port,"title":"要继续工作吗？","message":format!("{}，采样相隔至少 {} 分钟。中间未连续判断；也可以先休息。",description,settings.reminder_minutes),"sample_id":last.sample.id})).await;
    app.reminders.lock().unwrap().error = result.err();
}
pub async fn erase_day(
    State(app): State<Arc<App>>,
    Json(q): Json<QueryDay>,
) -> ApiResult<Json<Value>> {
    let _lock = app.lifecycle.lock().map_err(|_| "记录锁异常")?;
    if app.recorder.snapshot().running
        || app.jobs.available_permits() == 0
        || app
            .store
            .samples()?
            .iter()
            .any(|s| ["queued", "analyzing"].contains(&s.state.as_str()))
    {
        return Err("请先暂停记录，并等待当前分析结束后再删除".into());
    }
    let (start, end) = summary::bounds(&q.date)?;
    if q.mode != "live" {
        return Err("数据类型无效".into());
    }
    let count = app.store.erase_range(start, end, &q.mode)?;
    Ok(Json(json!({"ok":true,"deleted":count})))
}

#[derive(Deserialize)]
pub struct ResetDataInput {
    confirmation: String,
}

pub async fn reset_data(
    State(app): State<Arc<App>>,
    Json(input): Json<ResetDataInput>,
) -> ApiResult<Json<Value>> {
    if input.confirmation.trim() != "清空全部数据" {
        return Err("请输入“清空全部数据”确认重置".into());
    }
    let result = {
        let _lock = app.lifecycle.lock().map_err(|_| "记录锁异常")?;
        if app.recorder.snapshot().running
            || app.jobs.available_permits() == 0
            || app
                .store
                .samples()?
                .iter()
                .any(|s| ["queued", "analyzing"].contains(&s.state.as_str()))
        {
            return Err("请先暂停记录，并等待当前分析结束后再清空数据".into());
        }
        let result = app.store.reset_data()?;
        app.recorder.clear_stopped_state()?;
        *app.reminders.lock().map_err(|_| "提醒状态异常")? = ReminderState::default();
        result
    };
    Ok(Json(json!({
        "ok": true,
        "deleted": {
            "samples": result.samples,
            "sessions": result.sessions,
            "reports": result.reports,
            "files": result.files
        },
        "settings_preserved": true
    })))
}

pub async fn export(
    State(app): State<Arc<App>>,
    Query(q): Query<QueryDay>,
) -> ApiResult<axum::response::Response> {
    use axum::{
        http::{header, HeaderValue},
        response::IntoResponse,
    };
    let day = summary::day(&app.store, &q.date, &q.mode, now())?;
    let metrics = derived(&day.segments);
    let value = json!({"product":"Flow Insight","date":q.date,"source":"本机采样","note":"状态为 AI 辅助估计，不代表精确测量心流","metrics":metrics,"records":day.segments,"report":day.report});
    let mut response = (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::to_vec_pretty(&value).map_err(|_| "导出失败")?,
    )
        .into_response();
    let filename = format!(
        "attachment; filename=\"flow-insight-{}-{}.json\"",
        q.date, q.mode
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&filename).map_err(|_| "导出日期无效")?,
    );
    Ok(response)
}

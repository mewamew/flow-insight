use crate::{
    activity::ActivityTracker,
    ai,
    api::{self, App, CaptureInput},
    models::{
        now, CaptureEvidence, Presence, PresenceRegion, Sample, SelectedDisplay, Session,
        MAX_DISPLAYS,
    },
    native::NativeCapture,
    policy::{Decision, Observation, Policy, OBSERVATION_GAP_MS, POLL_SECONDS},
    store::Result,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{watch, Mutex as AsyncMutex},
    task::JoinHandle,
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct RecordingStatus {
    pub running: bool,
    pub session: Option<Session>,
    pub camera: bool,
    pub display_id: u32,
    pub display_ids: Vec<u32>,
    pub displays: Vec<SelectedDisplay>,
    pub last_capture: Option<i64>,
    pub error: Option<String>,
    pub presence: Presence,
    /// Whether the helper's camera session is running right now; false whenever the camera is off, paused or failed.
    pub camera_active: bool,
    pub camera_device: String,
    /// Live only: where the last frame showed a person. Never persisted with samples.
    pub presence_regions: Vec<PresenceRegion>,
    pub scheduling: Option<Decision>,
    pub local_checks: u64,
    pub screen_captures: u64,
    pub analyses_scheduled: u64,
}
#[derive(Default, Deserialize)]
pub struct StartInput {
    #[serde(default)]
    pub camera: bool,
    #[serde(default)]
    pub display_id: Option<u32>,
    /// Omitted selects all displays at start; an explicit empty list is rejected.
    #[serde(default)]
    pub display_ids: Option<Vec<u32>>,
}

fn select_displays(input: &StartInput, permissions: &Value) -> Result<Vec<SelectedDisplay>> {
    let available: Vec<SelectedDisplay> = serde_json::from_value(permissions["displays"].clone())
        .map_err(|_| "无法读取显示器列表")?;
    let ids = if let Some(ids) = &input.display_ids {
        ids.clone()
    } else if let Some(id) = input.display_id {
        vec![if id == 0 {
            permissions["displays"]
                .as_array()
                .and_then(|ds| ds.iter().find(|d| d["primary"] == true))
                .and_then(|d| d["id"].as_u64())
                .map(|id| id as u32)
                .ok_or("主显示器不可用")?
        } else {
            id
        }]
    } else {
        available.iter().map(|d| d.id).collect()
    };
    if ids.is_empty() {
        return Err("请至少选择一块显示器".into());
    }
    if ids.len() > MAX_DISPLAYS {
        return Err("一次最多记录 16 块显示器".into());
    }
    let mut selected = Vec::new();
    for id in ids {
        if selected.iter().any(|d: &SelectedDisplay| d.id == id) {
            return Err("显示器选择不能重复".into());
        }
        selected.push(
            available
                .iter()
                .find(|d| d.id == id)
                .cloned()
                .ok_or("所选显示器已断开，请刷新权限与设备状态")?,
        );
    }
    Ok(selected)
}
struct Job {
    stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
}
pub struct Recorder {
    pub native: NativeCapture,
    state: Mutex<RecordingStatus>,
    transition: AsyncMutex<()>,
    job: Mutex<Option<Job>>,
}
impl Recorder {
    pub fn new(helper: PathBuf) -> Self {
        Self {
            native: NativeCapture::new(helper),
            state: Mutex::new(RecordingStatus::default()),
            transition: AsyncMutex::new(()),
            job: Mutex::new(None),
        }
    }
    pub fn snapshot(&self) -> RecordingStatus {
        self.state.lock().expect("recorder state").clone()
    }
    pub fn clear_stopped_state(&self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| "记录状态异常")?;
        if state.running {
            return Err("请先暂停记录，再清空数据".into());
        }
        *state = RecordingStatus::default();
        Ok(())
    }
    pub async fn permissions(&self) -> Value {
        match self.native.request(json!({"command":"permissions"})).await {
            Ok(v) => v,
            Err(e) => {
                json!({"ok":false,"screen_permission":false,"camera_permission":"unavailable","displays":[],"error":e})
            }
        }
    }
    pub async fn request_permission(&self, kind: &str) -> Result<Value> {
        if !["screen", "camera"].contains(&kind) {
            return Err("权限类型无效".into());
        }
        self.native
            .request(json!({"command":"request_permission","kind":kind}))
            .await
    }
    pub async fn start(app: Arc<App>, input: StartInput) -> Result<RecordingStatus> {
        let _transition = app.recorder.transition.lock().await;
        if app.recorder.snapshot().running {
            return Err("后台已经在记录，请先停止".into());
        }
        let permissions = app
            .recorder
            .native
            .request(json!({"command":"permissions"}))
            .await?;
        if permissions["screen_permission"] != true {
            return Err("请先为本地程序授权屏幕录制，再开始记录；无需授权 Chrome".into());
        }
        if input.camera && permissions["camera_permission"] != "authorized" {
            return Err("请先授权本地程序使用摄像头，或取消摄像头选项".into());
        }
        let displays = select_displays(&input, &permissions)?;
        app.recorder
            .native
            .request(json!({"command":"begin"}))
            .await?;
        let session = api::create_session(&app).map_err(|e| e.1)?;
        *app.recorder.state.lock().map_err(|_| "记录状态异常")? = RecordingStatus {
            running: true,
            session: Some(session.clone()),
            camera: input.camera,
            display_id: displays[0].id,
            display_ids: displays.iter().map(|d| d.id).collect(),
            displays: displays.clone(),
            last_capture: None,
            error: None,
            ..Default::default()
        };
        let (stop, mut stopped) = watch::channel(false);
        let task_app = app.clone();
        let handle = tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(POLL_SECONDS.into()));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut policy = Policy::default();
            let mut previous: Option<Sample> = None;
            let mut activity_tracker = ActivityTracker::default();
            loop {
                tokio::select! { biased;
                    _ = stopped.changed() => break,
                    _ = timer.tick() => {}
                }
                let settings = task_app.store.settings();
                let observation = task_app.recorder.native.request(json!({"command":"observe","camera":input.camera,"excluded_apps":settings.excluded_apps}));
                let observed = tokio::select! { biased;
                    _ = stopped.changed() => break,
                    result = observation => result
                }
                .and_then(|v| {
                    serde_json::from_value::<Observation>(v)
                        .map_err(|_| "本地观察数据格式无效".into())
                });
                let o = match observed {
                    Ok(o) if o.captured_at <= now() + 3000 && now() - o.captured_at < 15_000 => o,
                    _ => {
                        let mut s = task_app.recorder.state.lock().unwrap();
                        s.error = Some("本地观察暂不可用，未延长任何状态时长；正在重试".into());
                        s.presence = Presence::default();
                        s.camera_active = false;
                        s.camera_device.clear();
                        s.presence_regions.clear();
                        s.scheduling = Some(Decision {
                            analyze: false,
                            away: false,
                            reason: "unavailable".into(),
                            next_check_seconds: POLL_SECONDS,
                        });
                        drop(s);
                        // Persist a zero-length fault boundary so a quick recovery
                        // cannot revive a judgment from before the failed check.
                        previous = None;
                        activity_tracker.clear();
                        let failed = Observation {
                            captured_at: now(),
                            ..Default::default()
                        };
                        let decision = Decision {
                            analyze: false,
                            away: false,
                            reason: "unavailable".into(),
                            next_check_seconds: POLL_SECONDS,
                        };
                        let _ = save_local(&task_app, &session, &failed, &decision, &mut previous);
                        previous = None;
                        continue;
                    }
                };
                let mut d = policy.decide(&o);
                if o.blocked_reason.is_some() || d.away {
                    activity_tracker.clear();
                } else {
                    activity_tracker.observe(o.captured_at, &o.activity, &settings.excluded_apps);
                }
                {
                    let mut s = task_app.recorder.state.lock().unwrap();
                    s.local_checks += 1;
                    s.presence = o.presence.clone();
                    s.camera_active = o.camera_active;
                    s.camera_device = if o.camera_active {
                        o.camera_device.clone()
                    } else {
                        String::new()
                    };
                    s.presence_regions = o
                        .presence_regions
                        .iter()
                        .cloned()
                        .filter_map(PresenceRegion::sanitized)
                        .take(8)
                        .collect();
                    s.error = None;
                    s.scheduling = Some(d.clone());
                }
                if d.analyze {
                    if ai::backend_access_error(&settings).is_some() {
                        d.analyze = false;
                        d.reason = "model_unavailable".into();
                    } else if task_app.jobs.available_permits() == 0 {
                        d.analyze = false;
                        d.reason = "model_busy".into();
                    }
                }
                if d.analyze {
                    let capture = task_app.recorder.native.request(json!({"command":"sample","camera":false,"displays":displays,"excluded_apps":settings.excluded_apps,"expected_bundle_id":o.activity.bundle_id}));
                    let result = tokio::select! { biased;
                        _ = stopped.changed() => break,
                        result = capture => result
                    };
                    let result = result.and_then(|v| {
                        let at = v["captured_at"].as_i64().ok_or("原生采样缺少时间")?;
                        let mut activity = o.activity.clone();
                        if v["activity"].is_object() {
                            activity = serde_json::from_value(v["activity"].clone())
                                .map_err(|_| "原生应用信息无效")?;
                            activity.switches.splice(0..0, o.activity.switches.clone());
                            activity.observed_since = o.activity.observed_since;
                        }
                        activity_tracker.observe(at, &activity, &settings.excluded_apps);
                        activity.history = activity_tracker.history();
                        // The legacy switch field now carries the same complete interval.
                        activity.switches = activity
                            .history
                            .as_ref()
                            .map(|h| h.switches.clone())
                            .unwrap_or_default();
                        activity.observed_since =
                            activity.history.as_ref().map(|h| h.start).unwrap_or(at);
                        let frame = CaptureInput {
                            screens: serde_json::from_value(
                                v.get("screens").cloned().unwrap_or(json!([])),
                            )
                            .map_err(|_| "原生多屏采样格式无效")?,
                            evidence: Some(CaptureEvidence {
                                basis: "ai".into(),
                                trigger: d.reason.clone(),
                                presence: o.presence.clone(),
                                away: false,
                                valid_until: at,
                            }),
                            activity,
                            capture_warning: v["capture_warning"].as_str().map(str::to_string),
                            id: uuid::Uuid::new_v4().to_string(),
                            session_id: session.id.clone(),
                            captured_at: at,
                            screen: v["screen"].as_str().unwrap_or_default().into(),
                            camera: None,
                            capture_source: v["capture_source"]
                                .as_str()
                                .unwrap_or("本机屏幕")
                                .into(),
                        };
                        api::save_capture(task_app.clone(), frame).map_err(|e| e.1)
                    });
                    match result {
                        Ok(sample) => {
                            activity_tracker.begin_after_capture(
                                sample.captured_at,
                                &sample.activity,
                                &settings.excluded_apps,
                            );
                            let extension = extend_observation(
                                &task_app,
                                previous.as_ref(),
                                sample.captured_at,
                            );
                            previous = Some(sample.clone());
                            let mut s = task_app.recorder.state.lock().unwrap();
                            s.error = extension.err();
                            s.last_capture = Some(sample.captured_at);
                            s.screen_captures += 1;
                            s.analyses_scheduled += 1;
                            s.scheduling = Some(d);
                            continue;
                        }
                        Err(e) => {
                            task_app.recorder.state.lock().unwrap().error = Some(e);
                            d.reason = "capture_failed".into();
                        }
                    }
                }
                task_app.recorder.state.lock().unwrap().scheduling = Some(d.clone());
                if let Err(error) = save_local(&task_app, &session, &o, &d, &mut previous) {
                    task_app.recorder.state.lock().unwrap().error = Some(error);
                } else {
                    // Evaluate reminders after coverage exists, even if AI finished
                    // before the next local check could extend its zero-length sample.
                    crate::flow::maybe_remind(&task_app).await;
                }
            }
        });
        if let Some(old) = app
            .recorder
            .job
            .lock()
            .map_err(|_| "采样任务异常")?
            .replace(Job { stop, handle })
        {
            old.handle.abort();
        }
        Ok(app.recorder.snapshot())
    }
    pub async fn stop(app: &Arc<App>) -> Result<RecordingStatus> {
        let _transition = app.recorder.transition.lock().await;
        if let Some(session) = app.recorder.snapshot().session {
            api::end_session(app, &session.id).map_err(|e| e.1)?;
        }
        let job = app.recorder.job.lock().map_err(|_| "采样任务异常")?.take();
        if let Some(job) = job {
            let _ = job.stop.send(true);
            let _ = job.handle.await;
        }
        // Killing the private native worker also releases its camera immediately.
        app.recorder.native.close().await;
        let mut state = app.recorder.state.lock().map_err(|_| "记录状态异常")?;
        state.running = false;
        state.camera_active = false;
        state.camera_device.clear();
        state.presence_regions.clear();
        if let Some(session) = state.session.as_mut() {
            session.ended_at.get_or_insert(now());
        }
        Ok(state.clone())
    }
}

// Only a subsequent observation can extend coverage. Update just the timestamp:
// an AI result may already have been written since `previous` was read.
fn extend_observation(app: &Arc<App>, previous: Option<&Sample>, at: i64) -> Result<()> {
    if let Some(previous) = previous {
        if previous
            .evidence
            .as_ref()
            .is_some_and(|e| (0..=OBSERVATION_GAP_MS).contains(&(at - e.valid_until)))
        {
            app.store.update_sample(&previous.id, |s| {
                if let Some(e) = &mut s.evidence {
                    e.valid_until = at;
                }
            })?;
        }
    }
    Ok(())
}

fn save_local(
    app: &Arc<App>,
    session: &Session,
    o: &Observation,
    d: &Decision,
    previous: &mut Option<Sample>,
) -> Result<()> {
    let same = previous.as_ref().is_some_and(|s| {
        s.state == "local"
            && s.evidence.as_ref().is_some_and(|e| {
                e.away == d.away
                    && e.trigger == d.reason
                    && e.presence.state == o.presence.state
                    && (0..=OBSERVATION_GAP_MS).contains(&(o.captured_at - e.valid_until))
            })
            && s.activity.bundle_id == o.activity.bundle_id
            && s.activity.window_title == o.activity.window_title
            && o.captured_at - s.captured_at < 60_000
    });
    let mut sample = if same {
        previous.take().unwrap()
    } else {
        extend_observation(app, previous.as_ref(), o.captured_at)?;
        Sample {
            screens: vec![],
            evidence: Some(CaptureEvidence {
                basis: "local".into(),
                trigger: d.reason.clone(),
                presence: o.presence.clone(),
                away: d.away,
                valid_until: o.captured_at,
            }),
            activity: o.activity.clone(),
            capture_warning: None,
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session.id.clone(),
            captured_at: o.captured_at.max(session.started_at),
            interval_seconds: POLL_SECONDS,
            mode: "live".into(),
            task: session.task.clone(),
            screen: None,
            camera: None,
            capture_source: "本地观察 · 无图片上传".into(),
            state: "local".into(),
            analysis: None,
            error: None,
            correction: None,
            correction_note: String::new(),
        }
    };
    if same {
        sample.activity.switches.extend(o.activity.switches.clone());
        sample.activity.idle_seconds = o.activity.idle_seconds;
    }
    if let Some(e) = &mut sample.evidence {
        e.valid_until = o.captured_at;
        e.presence = o.presence.clone();
    }
    // No future seconds are added to a local observation; each poll extends it.
    app.store
        .put("sample", &sample.id, sample.captured_at, "live", &sample)?;
    *previous = Some(sample);
    Ok(())
}

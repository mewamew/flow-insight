use flow_insight::{
    flow,
    models::*,
    store::Store,
    summary::{self, Segment},
};
fn segment(start: i64, seconds: u32, c: Category) -> Segment {
    Segment {
        estimated_from: None,
        start,
        end: start + seconds as i64 * 1000,
        category: c.clone(),
        sample: Sample {
            evidence: None,
            id: start.to_string(),
            session_id: "session".into(),
            captured_at: start,
            interval_seconds: seconds,
            mode: "live".into(),
            task: "工作".into(),
            screens: vec![],
            screen: None,
            camera: None,
            capture_source: "合成数据".into(),
            state: "done".into(),
            analysis: Some(Analysis {
                category: c,
                confidence: 0.9,
                app_name: "test".into(),
                screen_activity: "test".into(),
                camera_state: "无摄像头".into(),
                summary: "test".into(),
                evidence: vec![],
            }),
            error: None,
            correction: None,
            correction_note: String::new(),
            activity: Default::default(),
            capture_warning: None,
        },
    }
}
#[test]
fn interruption_counts_runs_not_frames_and_unknown_breaks_recovery() {
    let t = 1_000_000;
    let s = vec![
        segment(t, 60, Category::Work),
        segment(t + 60_000, 60, Category::Distracted),
        segment(t + 120_000, 60, Category::Distracted),
        segment(t + 180_000, 60, Category::Work),
        segment(t + 240_000, 60, Category::Distracted),
        segment(t + 300_000, 60, Category::Unknown),
        segment(t + 360_000, 60, Category::Work),
    ];
    let d = flow::derived(&s);
    assert_eq!(d["interruptions"], 2);
    assert_eq!(d["recoveries"].as_array().unwrap().len(), 1);
    assert_eq!(d["recoveries"][0]["seconds"], 120);
}
#[test]
fn gaps_and_sessions_never_count_as_continuous_distraction() {
    let mut s = vec![
        segment(100_000, 60, Category::Distracted),
        segment(160_000, 60, Category::Distracted),
        segment(220_000, 60, Category::Distracted),
    ];
    assert!(flow::should_remind(&s, 280_000, 180));
    assert!(!flow::should_remind(&s, 600_000, 180));
    s[2].start += 10_000;
    assert!(!flow::should_remind(&s, 280_000, 180));
    s[2].start -= 10_000;
    s[2].sample.session_id = "another".into();
    assert!(!flow::should_remind(&s, 280_000, 180));
    s[2].sample.session_id = "session".into();
    s[2].sample.analysis.as_mut().unwrap().confidence = 0.6;
    assert!(!flow::should_remind(&s, 280_000, 180));
    s[2].sample.analysis.as_mut().unwrap().confidence = 0.9;
}
#[test]
fn cross_hour_split_and_sparse_data_do_not_invent_high_low_periods() {
    let (t, _) = summary::bounds("2026-09-06").unwrap();
    let s = vec![segment(t + 9 * 3600000 + 59 * 60000, 120, Category::Work)];
    let d = flow::derived(&s);
    assert_eq!(d["hourly"][9]["work_seconds"], 60);
    assert_eq!(d["hourly"][10]["work_seconds"], 60);
    assert!(d["high_period"].is_null());
    assert!(d["low_period"].is_null());
}
#[test]
fn deleting_a_day_removes_media_and_only_its_own_report() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    let (t, end) = summary::bounds("2026-09-06").unwrap();
    let mut s = segment(t, 60, Category::Work).sample;
    s.screen = Some("test.jpg".into());
    std::fs::write(root.path().join("captures/test.jpg"), b"test").unwrap();
    store.put("sample", &s.id, t, "live", &s).unwrap();
    for date in ["2026-09-05", "2026-09-06"] {
        let r = Report {
            date: date.into(),
            mode: "live".into(),
            state: "done".into(),
            headline: "x".into(),
            observations: vec![],
            suggestions: vec![],
            generated_at: t,
            fingerprint: "x".into(),
            error: None,
        };
        store
            .put("report", &format!("live:{date}"), t, "live", &r)
            .unwrap();
    }
    assert_eq!(store.erase_range(t, end, "live").unwrap(), 1);
    assert!(!root.path().join("captures/test.jpg").exists());
    assert!(store
        .get::<Report>("report", "live:2026-09-06")
        .unwrap()
        .is_none());
    assert!(store
        .get::<Report>("report", "live:2026-09-05")
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn automatic_card_dispatch_is_cooldown_limited() {
    use flow_insight::{
        api::App,
        recorder::{Recorder, StartInput},
    };
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("native.py");
    let marker = dir.path().join("cards.txt");
    let script = format!(
        r#"#!/usr/bin/env python3
import json,sys
for line in sys.stdin:
 v=json.loads(line)
 if v['command']=='permissions':
  r={{'ok':True,'screen_permission':True,'camera_permission':'authorized','displays':[{{'id':1}}]}}
 elif v['command']=='sample':r={{'ok':False,'error':'synthetic capture paused'}}
 elif v['command']=='show_card':
  with open({marker},'a') as f:f.write(v['sample_id']+'\n')
  r={{'ok':True}}
 else:r={{'ok':True}}
 print(json.dumps(r),flush=True)
"#,
        marker = serde_json::json!(marker.to_string_lossy())
    );
    std::fs::write(&helper, script).unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let app = App::with_helper(
        Store::open(dir.path().join("store")).unwrap(),
        17903,
        helper,
    )
    .unwrap();
    let status = Recorder::start(
        app.clone(),
        StartInput {
            camera: false,
            display_id: Some(1),
            display_ids: None,
        },
    )
    .await
    .unwrap();
    let end = now() - 5000;
    for i in 0..4 {
        let mut s = segment(end - (3 - i) * 60000, 60, Category::Distracted).sample;
        s.session_id = status.session.as_ref().unwrap().id.clone();
        app.store
            .put("sample", &s.id, s.captured_at, "live", &s)
            .unwrap();
    }
    flow::maybe_remind(&app).await;
    flow::maybe_remind(&app).await;
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
    app.reminders.lock().unwrap().last_sent = 0;
    app.reminders.lock().unwrap().last_sample.clear();
    app.reminders.lock().unwrap().snoozed_until = now() + 900000;
    flow::maybe_remind(&app).await;
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
    Recorder::stop(&app).await.unwrap();
}

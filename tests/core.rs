use flow_insight::{ai, models::*, store::Store, summary};

fn sample(at: i64, category: Category) -> Sample {
    Sample {
        evidence: None,
        activity: Default::default(),
        capture_warning: None,
        id: at.to_string(),
        session_id: "s".into(),
        captured_at: at,
        interval_seconds: 60,
        mode: "live".into(),
        task: "写脚本".into(),
        screens: vec![],
        screen: None,
        camera: None,
        capture_source: "test".into(),
        state: "done".into(),
        analysis: Some(Analysis {
            category,
            confidence: 0.9,
            app_name: "文档".into(),
            screen_activity: "写稿".into(),
            camera_state: "未知".into(),
            summary: "测试".into(),
            evidence: vec![],
        }),
        error: None,
        correction: None,
        correction_note: String::new(),
    }
}
fn session(start: i64, end: i64) -> Session {
    Session {
        id: "s".into(),
        started_at: start,
        ended_at: Some(end),
        interval_seconds: 60,
        task: "写脚本".into(),
        mode: "live".into(),
    }
}

#[test]
fn gaps_are_never_filled_as_work_and_stop_clips_last_sample() {
    let (t, _) = summary::bounds("2026-09-06").unwrap();
    let t = t + 9 * 3_600_000;
    let d = summary::build(
        "2026-09-06",
        "live",
        vec![
            sample(t, Category::Work),
            sample(t + 3_600_000, Category::Work),
        ],
        vec![session(t, t + 3_610_000)],
        t + 7_200_000,
    )
    .unwrap();
    assert_eq!(d.work_seconds, 70);
    assert_eq!(d.observed_seconds, 70);
    assert_eq!(d.unobserved_seconds, 3540);
    assert_eq!(d.longest_work_seconds, 60);
}
#[test]
fn midnight_clips_and_user_corrections_take_precedence() {
    let (t, _) = summary::bounds("2026-09-06").unwrap();
    let mut a = sample(t - 20_000, Category::Distracted);
    a.correction = Some(Category::Work);
    let d = summary::build(
        "2026-09-06",
        "live",
        vec![a, sample(t + 40_000, Category::Work)],
        vec![session(t - 20_000, t + 100_000)],
        t + 200_000,
    )
    .unwrap();
    assert_eq!(d.work_seconds, 100);
    assert_eq!(d.distracted_seconds, 0);
    assert_eq!(d.longest_work_seconds, 100);
}
#[test]
fn legacy_break_records_and_corrections_count_as_interruptions_without_rewriting_data() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    let (t, _) = summary::bounds("2026-09-09").unwrap();
    let t = t + 9 * 3_600_000;
    let samples = [
        sample(t, Category::Work),
        sample(t + 60_000, Category::Distracted),
        sample(t + 120_000, Category::Work),
        sample(t + 180_000, Category::Work),
    ];
    let mut raw: Vec<_> = samples
        .iter()
        .map(|s| serde_json::to_value(s).unwrap())
        .collect();
    raw[1]["analysis"]["category"] = serde_json::json!("break");
    raw[2]["correction"] = serde_json::json!("break");
    for (s, value) in samples.iter().zip(&raw) {
        store
            .put("sample", &s.id, s.captured_at, "live", value)
            .unwrap();
    }
    let d = summary::build(
        "2026-09-09",
        "live",
        store.samples().unwrap(),
        vec![session(t, t + 240_000)],
        t + 240_000,
    )
    .unwrap();
    assert_eq!(d.work_seconds, 120);
    assert_eq!(d.distracted_seconds, 120);
    assert_eq!(d.observed_seconds, 240);
    assert_eq!(d.longest_work_seconds, 60);
    let metrics = flow_insight::flow::derived(&d.segments);
    assert_eq!(metrics["interruptions"], 1);
    assert_eq!(metrics["recoveries"][0]["seconds"], 120);
    assert_eq!(metrics["hourly"][9]["distracted_seconds"], 120);
    assert_eq!(metrics["hourly"][9]["coverage_seconds"], 240);
    let exported = serde_json::to_value(&d).unwrap();
    assert_eq!(exported["segments"][1]["category"], "distracted");
    assert_eq!(
        exported["segments"][2]["sample"]["correction"],
        "distracted"
    );
    assert!(exported.get("break_seconds").is_none());
    for (s, value) in samples.iter().zip(raw) {
        assert_eq!(
            store.get::<serde_json::Value>("sample", &s.id).unwrap(),
            Some(value)
        );
    }
}
#[test]
fn small_timer_drift_joins_work_but_never_fills_gaps_or_joins_sessions() {
    let (t, _) = summary::bounds("2026-09-06").unwrap();
    let mut third = sample(t + 121_000, Category::Work);
    third.session_id = "second-session".into();
    let mut second_session = session(t + 121_000, t + 181_000);
    second_session.id = "second-session".into();
    let d = summary::build(
        "2026-09-06",
        "live",
        vec![
            sample(t, Category::Work),
            sample(t + 60_500, Category::Work),
            third,
        ],
        vec![session(t, t + 120_500), second_session],
        t + 181_000,
    )
    .unwrap();
    assert_eq!(d.work_seconds, 180);
    assert_eq!(d.longest_work_seconds, 120);
    assert_eq!(d.unobserved_seconds, 1);
}
#[test]
fn no_ai_result_is_unknown() {
    let (t, _) = summary::bounds("2026-09-06").unwrap();
    let mut s = sample(t, Category::Work);
    s.analysis = None;
    s.state = "pending".into();
    let d = summary::build(
        "2026-09-06",
        "live",
        vec![s],
        vec![session(t, t + 120_000)],
        t + 120_000,
    )
    .unwrap();
    assert_eq!(d.unknown_seconds, 60);
    assert_eq!(d.work_seconds, 0);
    assert_eq!(d.segments.len(), 1);
}
#[test]
fn malformed_ai_replies_and_unsecure_endpoints_are_rejected() {
    let reply = r#"{"category":"work","confidence":0.3,"app_name":"文档","screen_activity":"写稿","camera_state":"在座","summary":"写稿","evidence":["文档可见"]}"#;
    let a = ai::parse_analysis(&format!("```json\n{reply}\n```"), false).unwrap();
    assert_eq!(a.category, Category::Unknown);
    assert!(a.camera_state.contains("未启用"));
    assert!(ai::parse_analysis(&reply.replace("0.3", "1.3"), true).is_err());
    assert!(ai::parse_analysis("不是JSON", true).is_err());
    assert!(ai::parse_analysis("错误的括号 } 然后 {", true).is_err());
    let mut s = Settings {
        base_url: "http://example.com/v1".into(),
        ..Settings::default()
    };
    assert!(ai::validate_settings(&s).is_err());
    s.base_url = "http://127.0.0.1:23456/v1".into();
    assert!(ai::validate_settings(&s).is_ok());
    assert!(ai::endpoint(&s).ends_with("/v1/chat/completions"));
}
#[test]
fn key_is_private_and_restart_closes_orphaned_sessions() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    let mut settings = store.settings();
    settings.api_key = "not-a-real-key".into();
    store.save_settings(settings).unwrap();
    assert!(!store
        .settings()
        .public()
        .to_string()
        .contains("not-a-real-key"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path().join("settings.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let mut s = session(now() - 120_000, now());
    s.ended_at = None;
    store
        .put("session", &s.id, s.started_at, "live", &s)
        .unwrap();
    let mut record = sample(now() - 60_000, Category::Work);
    record.state = "analyzing".into();
    store
        .put("sample", &record.id, record.captured_at, "live", &record)
        .unwrap();
    drop(store);
    let store = Store::open(root.path().into()).unwrap();
    assert!(store
        .get::<Session>("session", "s")
        .unwrap()
        .unwrap()
        .ended_at
        .is_some());
    assert_eq!(
        store
            .get::<Sample>("sample", &record.id)
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
}

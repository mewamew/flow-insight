use flow_insight::{
    activity::ActivityTracker,
    models::{Activity, AppSwitch},
};
fn activity(app: &str) -> Activity {
    Activity {
        app_name: app.into(),
        bundle_id: app.into(),
        ..Default::default()
    }
}
fn switched(at: i64, app: &str) -> AppSwitch {
    AppSwitch {
        at,
        app_name: app.into(),
        bundle_id: app.into(),
    }
}

#[test]
fn brief_chat_between_polls_survives_the_whole_minute() {
    let mut tracker = ActivityTracker::default();
    tracker.observe(0, &activity("editor"), &[]);
    let mut a = activity("editor");
    a.switches = vec![switched(1000, "wechat"), switched(3000, "editor")];
    tracker.observe(5000, &a, &[]);
    for at in (10000..=60000).step_by(5000) {
        tracker.observe(at, &activity("editor"), &[]);
    }
    let h = tracker.history().unwrap();
    assert_eq!(h.switches.len(), 2);
    assert_eq!(h.app_spans.len(), 3);
    assert_eq!(h.app_spans[1].bundle_id, "wechat");
    assert_eq!(h.app_spans[1].end - h.app_spans[1].start, 2000);
    assert_eq!(h.start, 0);
    assert_eq!(h.end, 60000);
    let mut captured = activity("editor");
    captured.history = Some(h);
    let decoded: Activity =
        serde_json::from_value(serde_json::to_value(&captured).unwrap()).unwrap();
    assert_eq!(decoded.history.unwrap().switches.len(), 2);
    tracker.begin_after_capture(60000, &captured, &[]);
    assert!(tracker.history().unwrap().switches.is_empty());
}
#[test]
fn privacy_boundaries_and_gaps_discard_old_context() {
    let mut t = ActivityTracker::default();
    t.observe(0, &activity("editor"), &[]);
    let mut a = activity("editor");
    a.switches = vec![switched(1000, "private"), switched(2000, "editor")];
    t.observe(5000, &a, &["private".into()]);
    assert!(t.history().is_none());
    t.observe(10000, &activity("editor"), &[]);
    t.observe(30000, &activity("browser"), &[]);
    assert_eq!(t.history().unwrap().start, 30000);
    t.clear();
    assert!(t.history().is_none());
}
#[test]
fn input_is_idle_checkpoints_and_retention_is_bounded() {
    let mut t = ActivityTracker::default();
    for at in (0..=900000).step_by(5000) {
        let mut a = activity("editor");
        a.idle_seconds = at as f64 / 1000.0;
        t.observe(at, &a, &[]);
    }
    let h = t.history().unwrap();
    assert!(h.truncated);
    assert!(h.input_checkpoints.len() <= 120);
    assert!(h.end - h.start <= 600000);
    assert_eq!(h.input_checkpoints.last().unwrap().idle_seconds, 900.0);
}
#[test]
fn duplicate_events_and_missed_notifications_do_not_double_count_dwell() {
    let mut t = ActivityTracker::default();
    t.observe(0, &activity("editor"), &[]);
    let mut a = activity("browser");
    a.switches = vec![switched(2000, "browser"), switched(2000, "browser")];
    t.observe(5000, &a, &[]);
    t.observe(10000, &activity("editor"), &[]);
    let h = t.history().unwrap();
    assert_eq!(h.switches.len(), 1);
    assert_eq!(
        h.app_spans.iter().map(|s| s.end - s.start).sum::<i64>(),
        10000
    );
}

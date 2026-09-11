use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use flow_insight::{
    api::{router, App},
    flow,
    models::{Sample, Session},
    rhythm::{self, RangeQuery},
    store::Store,
    summary,
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn time(date: &str, hour: i64, minute: i64) -> i64 {
    summary::bounds(date).unwrap().0 + (hour * 60 + minute) * 60_000
}
fn add(
    store: &Store,
    date: &str,
    hour: i64,
    minute: i64,
    seconds: u32,
    category: &str,
    mode: &str,
) {
    let start = time(date, hour, minute);
    let id = format!("{mode}:{start}");
    let sample: Sample = serde_json::from_value(json!({
        "id":id,"session_id":id,"captured_at":start,"interval_seconds":seconds,
        "mode":mode,"task":"private synthetic task","screen":null,"camera":null,
        "capture_source":"synthetic test","state":"done","error":null,
        "correction":category,"correction_note":"", "analysis":null
    }))
    .unwrap();
    let session = Session {
        id: id.clone(),
        started_at: start,
        ended_at: Some(start + i64::from(seconds) * 1000),
        interval_seconds: seconds,
        task: String::new(),
        mode: mode.into(),
    };
    store.put("sample", &id, start, mode, &sample).unwrap();
    store.put("session", &id, start, mode, &session).unwrap();
}
fn range(store: &Store, days: u32) -> Value {
    rhythm::build(
        store,
        &RangeQuery {
            date: "2026-09-09".into(),
            days,
            mode: "live".into(),
        },
        time("2026-09-10", 0, 0),
    )
    .unwrap()
}

#[test]
fn empty_ranges_sparse_days_and_modes_cannot_invent_a_peak() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    for days in [7, 14, 21] {
        let r = range(&store, days);
        assert_eq!(r["rows"].as_array().unwrap().len(), days as usize);
        assert_eq!(r["end_date"], "2026-09-09");
        assert_eq!(r["recorded_days"], 0);
        assert_eq!(r["status"], "insufficient");
        assert!(r["best_period"].is_null());
    }
    for date in ["2026-09-07", "2026-09-08", "2026-09-09"] {
        add(&store, date, 12, 0, 599, "work", "live");
        add(&store, date, 13, 0, 3600, "unknown", "live");
        add(&store, date, 14, 0, 3600, "away", "live");
    }
    add(&store, "2026-09-09", 9, 0, 3600, "work", "live");
    add(&store, "2026-09-09", 10, 0, 3600, "distracted", "live");
    let r = range(&store, 7);
    assert_eq!(r["recorded_days"], 3);
    assert_eq!(r["status"], "insufficient");
    assert_eq!(r["rows"][6]["hours"][12]["eligible"], false);
    assert_eq!(r["rows"][6]["hours"][13]["judged_seconds"], 0);
    assert_eq!(r["rows"][6]["hours"][14]["judged_seconds"], 0);
    assert!(!r.to_string().contains("private synthetic task"));
}

#[test]
fn peak_uses_equal_day_ratios_and_does_not_overweight_long_recordings() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    for date in ["2026-09-07", "2026-09-08"] {
        add(&store, date, 9, 0, 600, "work", "live");
    }
    add(&store, "2026-09-09", 9, 0, 3600, "distracted", "live");
    for date in ["2026-09-07", "2026-09-08", "2026-09-09"] {
        add(&store, date, 10, 0, 1200, "work", "live");
        add(&store, date, 10, 20, 1200, "distracted", "live");
    }
    let r = range(&store, 7);
    assert_eq!(r["status"], "ready");
    assert_eq!(r["best_period"]["hour"], 9);
    assert_eq!(r["best_period"]["eligible_days"], 3);
    assert!((r["best_period"]["work_ratio"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-9);
}

#[test]
fn similar_periods_do_not_claim_a_winner() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    for date in ["2026-09-07", "2026-09-08", "2026-09-09"] {
        for hour in [9, 10] {
            add(&store, date, hour, 0, 1200, "work", "live");
            add(&store, date, hour, 20, 1200, "distracted", "live");
        }
    }
    assert_eq!(range(&store, 7)["status"], "similar");
    assert!(range(&store, 7)["best_period"].is_null());
}

#[test]
fn midnight_clipping_navigation_and_daily_totals_agree() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    add(&store, "2026-09-08", 23, 50, 1800, "work", "live");
    add(&store, "2026-09-09", 9, 0, 3600, "unknown", "live");
    let at = time("2026-09-09", 9, 15);
    let r = rhythm::build(
        &store,
        &RangeQuery {
            date: "2026-09-09".into(),
            days: 7,
            mode: "live".into(),
        },
        at,
    )
    .unwrap();
    assert_eq!(r["rows"][5]["hours"][23]["work_seconds"], 600);
    assert_eq!(r["rows"][6]["hours"][0]["work_seconds"], 1200);
    assert_eq!(r["rows"][6]["hours"][9]["unknown_seconds"], 900);
    for row in r["rows"].as_array().unwrap() {
        let d = summary::build(
            row["date"].as_str().unwrap(),
            "live",
            store.samples().unwrap(),
            store.list("session", Some("live")).unwrap(),
            at,
        )
        .unwrap();
        assert_eq!(row["observed_seconds"], d.observed_seconds);
        let daily = flow::derived(&d.segments);
        for h in 0..24 {
            for field in [
                "work_seconds",
                "distracted_seconds",
                "unknown_seconds",
                "away_seconds",
                "coverage_seconds",
            ] {
                assert_eq!(row["hours"][h][field], daily["hourly"][h][field]);
            }
            if let Some(key) = row["hours"][h]["sample_key"].as_str() {
                assert!(d
                    .segments
                    .iter()
                    .any(|s| format!("{}:{}", s.sample.id, s.start) == key));
            }
        }
    }
    assert_eq!(
        r["rows"][6]["hours"][0]["sample_at"],
        time("2026-09-09", 0, 0)
    );
}

#[tokio::test]
async fn route_validates_ranges_modes_dates_and_serves_new_script() {
    let root = tempfile::tempdir().unwrap();
    let app = router(App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap());
    for (path, expected) in [
        ("/api/rhythm?date=2026-09-09", StatusCode::OK),
        (
            "/api/rhythm?date=2026-09-09&days=22",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/rhythm?date=2026-09-09&mode=invalid",
            StatusCode::BAD_REQUEST,
        ),
        ("/api/rhythm?date=invalid", StatusCode::BAD_REQUEST),
        ("/rhythm.js", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "127.0.0.1:17901")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{path}");
        if path == "/rhythm.js" {
            let bytes = to_bytes(response.into_body(), 100_000).await.unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains("FlowRhythm"));
        }
    }
}

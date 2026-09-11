use flow_insight::{ai, models::*, policy::*, summary};
fn observation(at: i64, presence: PresenceState, idle: f64, app: &str) -> Observation {
    Observation {
        captured_at: at,
        presence: Presence {
            state: presence,
            observed_at: at,
            ..Default::default()
        },
        activity: Activity {
            bundle_id: app.into(),
            window_title: "任务文档".into(),
            idle_seconds: idle,
            ..Default::default()
        },
        blocked_reason: None,
        ..Default::default()
    }
}
#[test]
fn presence_regions_are_clamped_to_the_frame_and_unknown_kinds_are_dropped() {
    let region = |kind: &str, x, y, w, h| PresenceRegion {
        kind: kind.into(),
        x,
        y,
        w,
        h,
        confidence: 1.5,
    };
    let clamped = region("face", -0.2, 0.75, 0.5, 0.9).sanitized().unwrap();
    assert_eq!(
        (
            clamped.x,
            clamped.y,
            clamped.w,
            clamped.h,
            clamped.confidence
        ),
        (0.0, 0.75, 0.5, 0.25, 1.0)
    );
    assert!(region("body", 0.9, 0.9, 0.05, 0.05).sanitized().is_some());
    assert!(region("face", 0.2, 0.2, 0.0, 0.4).sanitized().is_none());
    assert!(region("pixels", 0.2, 0.2, 0.3, 0.4).sanitized().is_none());
    let raw: Observation =
        serde_json::from_value(serde_json::json!({"captured_at":1,"presence":{"state":"present"}}))
            .unwrap();
    assert!(!raw.camera_active && raw.presence_regions.is_empty());
}
#[test]
fn fixed_minute_sampling_ignores_app_window_and_switch_events() {
    for changing in [false, true] {
        let mut p = Policy::default();
        let mut attempts = Vec::new();
        for t in (0..=240_000).step_by(5000) {
            let mut o = observation(t, PresenceState::Present, 0.0, "editor");
            if changing {
                o.activity.bundle_id = t.to_string();
                o.activity.window_title = format!("window {t}");
            }
            let d = p.decide(&o);
            if d.analyze {
                attempts.push(t);
            } else {
                assert!((1..=60).contains(&d.next_check_seconds));
            }
        }
        assert_eq!(attempts, [0, 60_000, 120_000, 180_000, 240_000]);
    }
}
#[test]
fn absence_requires_both_conditions_and_return_uses_the_same_minute_interval() {
    let mut p = Policy::default();
    assert!(
        p.decide(&observation(0, PresenceState::Present, 0.0, "editor"))
            .analyze
    );
    for t in (5000..=240_000).step_by(5000) {
        let d = p.decide(&observation(t, PresenceState::NotDetected, 90.0, "editor"));
        assert_eq!(d.away, t >= 50_000);
        assert!(!d.analyze);
    }
    // The interval is already due: no extra 15-second return wait.
    assert!(
        p.decide(&observation(245_000, PresenceState::Present, 0.0, "editor"))
            .analyze
    );
    assert!(
        !p.decide(&observation(250_000, PresenceState::Present, 0.0, "other"))
            .analyze
    );
}
#[test]
fn camera_uncertainty_and_keyboard_activity_never_mean_away() {
    for state in [
        PresenceState::Unknown,
        PresenceState::Disabled,
        PresenceState::Present,
        PresenceState::NotDetected,
    ] {
        let mut p = Policy::default();
        for t in (0..=180_000).step_by(5000) {
            let idle = if state == PresenceState::NotDetected {
                0.0
            } else {
                1000.0
            };
            assert!(
                !p.decide(&observation(t, state.clone(), idle, "browser"))
                    .away
            );
        }
    }
    let mut p = Policy::default();
    p.decide(&observation(0, PresenceState::NotDetected, 1000.0, "x"));
    assert!(
        !p.decide(&observation(
            180_000,
            PresenceState::NotDetected,
            1000.0,
            "x"
        ))
        .away
    );
}
#[test]
fn pauses_win_and_recovery_does_not_add_an_extra_timer() {
    for reason in ["locked", "excluded", "screen_permission"] {
        let mut p = Policy::default();
        assert!(
            p.decide(&observation(0, PresenceState::Present, 0.0, "editor"))
                .analyze
        );
        let mut o = observation(30_000, PresenceState::Present, 0.0, "editor");
        o.blocked_reason = Some(reason.into());
        assert!(!p.decide(&o).analyze);
        o.blocked_reason = None;
        o.captured_at = 40_000;
        let d = p.decide(&o);
        assert!(!d.analyze);
        assert_eq!(d.next_check_seconds, 20);
        o.captured_at = 60_000;
        assert!(p.decide(&o).analyze);
    }
}
#[test]
fn a_busy_or_failed_attempt_still_waits_one_minute() {
    let mut p = Policy::default();
    assert!(
        p.decide(&observation(0, PresenceState::Present, 0.0, "x"))
            .analyze
    );
    // Recorder may reject this attempt; Policy has already recorded it.
    for t in (5000..60_000).step_by(5000) {
        assert!(
            !p.decide(&observation(t, PresenceState::Present, 0.0, "y"))
                .analyze
        );
    }
    assert!(
        p.decide(&observation(60_000, PresenceState::Present, 0.0, "y"))
            .analyze
    );
}
fn sample(at: i64, end: i64, basis: &str, away: bool, category: Category) -> Sample {
    serde_json::from_value(serde_json::json!({"id":at.to_string(),"session_id":"s","captured_at":at,"interval_seconds":180,"mode":"live","task":"写稿","screen":null,"camera":null,"capture_source":"test","state":if basis=="ai"{"done"}else{"local"},"analysis":if basis=="ai"{Some(serde_json::json!({"category":category,"confidence":0.9,"app_name":"editor","screen_activity":"写稿","camera_state":"本地在座","summary":"写稿","evidence":[]}))}else{None},"error":null,"correction":null,"correction_note":"","evidence":{"basis":basis,"trigger":"stable","presence":{},"away":away,"valid_until":end}})).unwrap()
}
#[test]
fn local_continuity_estimates_work_without_inflating_screen_coverage() {
    let (t, _) = summary::bounds("2026-09-07").unwrap();
    let d = estimate_day(
        vec![
            sample(t, t + 5000, "ai", false, Category::Work),
            sample(t + 5000, t + 60_000, "local", false, Category::Unknown),
            sample(t + 60_000, t + 65_000, "ai", false, Category::Work),
            sample(t + 65_000, t + 180_000, "local", true, Category::Unknown),
        ],
        t + 180_000,
    );
    assert_eq!(d.work_seconds, 65);
    assert_eq!(d.longest_work_seconds, 65);
    assert_eq!(d.unknown_seconds, 0);
    assert_eq!(d.away_seconds, 115);
    assert_eq!(d.local_seconds, 170);
    assert_eq!(d.ai_observed_seconds, 10);
}
#[test]
fn configured_model_endpoints_are_not_blocked_by_subscription_type() {
    let s = Settings {
        api_key: "synthetic-not-a-real-key".into(),
        base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1".into(),
        ..Default::default()
    };
    assert!(ai::validate_settings(&s).is_ok());
    assert!(ai::backend_access_error(&s).is_none());
    let s = Settings {
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
        ..s
    };
    assert!(ai::backend_access_error(&s).is_none());
    let s = Settings {
        api_key: String::new(),
        ..s
    };
    assert!(ai::backend_access_error(&s).is_some());
}

fn estimate_day(samples: Vec<Sample>, end: i64) -> summary::Day {
    summary::build(
        "2026-09-07",
        "live",
        samples,
        vec![Session {
            id: "s".into(),
            started_at: end - 3_600_000,
            ended_at: Some(end),
            interval_seconds: 60,
            task: "写稿".into(),
            mode: "live".into(),
        }],
        end,
    )
    .unwrap()
}

#[test]
fn estimate_survives_app_changes_and_pending_analysis_then_uses_new_judgment() {
    let (t, _) = summary::bounds("2026-09-07").unwrap();
    let first = sample(t, t + 30_000, "ai", false, Category::Work);
    let mut switched = sample(t + 35_000, t + 90_000, "local", false, Category::Unknown);
    switched.activity.bundle_id = "another.app".into();
    switched.evidence.as_mut().unwrap().trigger = "waiting".into();
    let mut pending = sample(t + 90_000, t + 100_000, "ai", false, Category::Unknown);
    pending.analysis = None;
    pending.state = "analyzing".into();
    let mut samples = vec![first, switched, pending];
    let d = estimate_day(samples.clone(), t + 100_000);
    assert_eq!(d.work_seconds, 95);
    assert_eq!(d.longest_work_seconds, 95);
    assert_eq!(d.unobserved_seconds, 5);
    assert_eq!(
        d.segments[1].estimated_from.as_ref().unwrap().id,
        t.to_string()
    );
    assert!(d.segments[1].sample.analysis.is_none());
    samples[2] = sample(t + 90_000, t + 100_000, "ai", false, Category::Distracted);
    samples.push(sample(
        t + 100_000,
        t + 140_000,
        "local",
        false,
        Category::Unknown,
    ));
    let d = estimate_day(samples, t + 140_000);
    assert_eq!(d.work_seconds, 85);
    assert_eq!(d.distracted_seconds, 50);
    assert_eq!(flow_insight::flow::derived(&d.segments)["interruptions"], 1);
}

#[test]
fn estimate_expires_inside_local_record_without_counting_it_twice() {
    let (t, _) = summary::bounds("2026-09-07").unwrap();
    let d = estimate_day(
        vec![
            sample(t, t + 30_000, "ai", false, Category::Work),
            sample(t + 30_000, t + 330_000, "local", false, Category::Unknown),
        ],
        t + 330_000,
    );
    assert_eq!(d.work_seconds, 120);
    assert_eq!(d.unknown_seconds, 210);
    assert_eq!(d.ai_observed_seconds, 30);
    assert_eq!(d.local_seconds, 300);
    assert_eq!(d.counts["local"], 1);
    assert_eq!(d.segments[2].start, t + 120_000);
    assert!(d.segments[2].estimated_from.is_none());
    assert_eq!(flow_insight::flow::derived(&d.segments)["local_records"], 1);
}

#[test]
fn pauses_faults_unknown_results_and_session_boundaries_clear_the_estimate() {
    let (t, _) = summary::bounds("2026-09-07").unwrap();
    for trigger in [
        "locked",
        "excluded",
        "screen_permission",
        "unavailable",
        "capture_failed",
        "model_unavailable",
        "away",
        "error",
        "ai_unknown",
        "new_session",
        "gap",
    ] {
        let mut boundary = sample(
            t + 30_000,
            t + 60_000,
            "local",
            trigger == "away",
            Category::Unknown,
        );
        boundary.evidence.as_mut().unwrap().trigger = trigger.into();
        if trigger == "error" {
            boundary.state = "error".into();
        }
        if trigger == "ai_unknown" {
            boundary = sample(t + 30_000, t + 60_000, "ai", false, Category::Unknown);
        }
        if trigger == "new_session" {
            boundary.session_id = "other".into();
        }
        if trigger == "gap" {
            boundary.captured_at = t + 45_000;
        }
        let d = estimate_day(
            vec![
                sample(t, t + 30_000, "ai", false, Category::Work),
                boundary,
                sample(t + 60_000, t + 90_000, "local", false, Category::Unknown),
            ],
            t + 90_000,
        );
        assert_eq!(d.work_seconds, 30, "{trigger}");
        assert!(
            d.segments.last().unwrap().estimated_from.is_none(),
            "{trigger}"
        );
    }
}

#[test]
fn midnight_keeps_source_and_correction_but_never_extends_past_session_stop() {
    let (t, _) = summary::bounds("2026-09-07").unwrap();
    let mut source = sample(t - 60_000, t - 30_000, "ai", false, Category::Distracted);
    source.correction = Some(Category::Work);
    let d = estimate_day(
        vec![
            source,
            sample(t - 30_000, t + 60_000, "local", false, Category::Unknown),
        ],
        t + 20_000,
    );
    assert_eq!(d.work_seconds, 20);
    assert_eq!(d.ai_observed_seconds, 0);
    assert_eq!(
        d.segments[0].estimated_from.as_ref().unwrap().captured_at,
        t - 60_000
    );
    assert_eq!(d.segments[0].end, t + 20_000);
}

#[test]
fn the_same_two_minute_deadline_applies_to_the_source_record_itself() {
    let (t, _) = summary::bounds("2026-09-07").unwrap();
    let d = estimate_day(
        vec![sample(t, t + 180_000, "ai", false, Category::Work)],
        t + 180_000,
    );
    assert_eq!(d.work_seconds, 120);
    assert_eq!(d.unknown_seconds, 60);
    assert_eq!(d.segments[1].start, t + JUDGMENT_TTL_MS);
}
#[test]
fn zero_length_failure_boundary_stops_a_quick_recovery_from_reusing_work() {
    let (t, _) = summary::bounds("2026-09-07").unwrap();
    let mut failed = sample(t + 10_000, t + 10_000, "local", false, Category::Unknown);
    failed.evidence.as_mut().unwrap().trigger = "unavailable".into();
    let d = estimate_day(
        vec![
            sample(t, t + 5000, "ai", false, Category::Work),
            failed,
            sample(t + 15_000, t + 60_000, "local", false, Category::Unknown),
        ],
        t + 60_000,
    );
    assert_eq!(d.work_seconds, 5);
    assert_eq!(d.unknown_seconds, 45);
    assert_eq!(d.unobserved_seconds, 10);
}

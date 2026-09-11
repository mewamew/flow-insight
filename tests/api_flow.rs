use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use flow_insight::{
    api::{router, App},
    models::{Analysis, Category, Report, Sample, Session},
    store::{private_dir, private_write, Store},
    summary,
};
use serde_json::{json, Value};
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tower::ServiceExt;

async fn call(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "127.0.0.1:17901")
                .header("content-type", "application/json")
                .header("x-flow-insight-client", "web")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 10_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
fn frame() -> String {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        64,
        64,
        image::Rgb([220, 210, 180]),
    ))
    .write_to(&mut out, image::ImageFormat::Png)
    .unwrap();
    format!(
        "data:image/png;base64,{}",
        STANDARD.encode(out.into_inner())
    )
}
/// Ten finished live samples, one closed session and one report, all on one past date.
fn seed_live_day(store: &Store) -> String {
    let date = "2026-09-06";
    let (midnight, _) = summary::bounds(date).unwrap();
    let start = midnight + 9 * 3_600_000;
    let session = Session {
        id: "seeded".into(),
        started_at: start,
        ended_at: Some(start + 11 * 60_000),
        interval_seconds: 60,
        task: "写脚本".into(),
        mode: "live".into(),
    };
    store
        .put("session", &session.id, start, "live", &session)
        .unwrap();
    for i in 0..10 {
        let at = start + i * 60_000;
        let sample = Sample {
            evidence: None,
            activity: Default::default(),
            capture_warning: None,
            id: format!("seeded-{i}"),
            session_id: session.id.clone(),
            captured_at: at,
            interval_seconds: 60,
            mode: "live".into(),
            task: session.task.clone(),
            screens: vec![],
            screen: None,
            camera: None,
            capture_source: "test".into(),
            state: "done".into(),
            analysis: Some(Analysis {
                category: if i % 3 == 0 {
                    Category::Distracted
                } else {
                    Category::Work
                },
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
        };
        store
            .put("sample", &sample.id, at, "live", &sample)
            .unwrap();
    }
    let report = Report {
        date: date.into(),
        mode: "live".into(),
        state: "done".into(),
        headline: "测试日报".into(),
        observations: vec![],
        suggestions: vec![],
        generated_at: start,
        fingerprint: String::new(),
        error: None,
    };
    store
        .put("report", &format!("live:{date}"), start, "live", &report)
        .unwrap();
    date.into()
}
async fn await_sample(app: &Arc<App>, id: &str) {
    for _ in 0..100 {
        if app
            .store
            .get::<Sample>("sample", id)
            .unwrap()
            .unwrap()
            .state
            == "done"
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("AI worker did not finish")
}

#[tokio::test]
async fn capture_ai_correction_report_and_reload_flow() {
    let image_requests = Arc::new(AtomicUsize::new(0));
    let count = image_requests.clone();
    let mock=Router::new().route("/v1/chat/completions",post(move|headers:axum::http::HeaderMap,Json(v):Json<Value>|{let count=count.clone();async move{
 assert_eq!(headers.get("authorization").unwrap(),"Bearer synthetic-test-key");assert_eq!(v["model"],"test-vision");
 let content=if v["messages"].as_array().unwrap().iter().any(|m|m["content"].is_array()){let parts=v["messages"].as_array().unwrap().iter().find(|m|m["content"].is_array()).unwrap()["content"].as_array().unwrap();assert_eq!(parts.iter().filter(|x|x["type"]=="image_url").count(),1);count.fetch_add(1,Ordering::SeqCst);json!({"category":"distracted","confidence":0.9,"app_name":"测试文档","screen_activity":"测试画面","camera_state":"拿着手机","summary":"受控测试判断","evidence":["这是假 AI 服务返回的测试依据"]})}else{json!({"headline":"受控测试日报","observations":["用户已纠正片段"],"suggestions":["明天安排一段专注时间"]})};Json(json!({"choices":[{"message":{"content":content.to_string()}}]}))}}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock_task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let state = App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap();
    let app = router(state.clone());
    let (status, session) = call(&app, "POST", "/api/sessions", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let id = uuid::Uuid::new_v4().to_string();
    let capture = json!({"id":id,"session_id":session["id"],"captured_at":flow_insight::models::now(),"screen":frame(),"camera":null,"capture_source":"受控测试图像"});
    let mut camera_upload = capture.clone();
    camera_upload["camera"] = json!(frame());
    let (rejected, _) = call(&app, "POST", "/api/captures", camera_upload).await;
    assert_eq!(rejected, StatusCode::BAD_REQUEST);
    let (status, _) = call(&app, "POST", "/api/captures", capture.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        state
            .store
            .get::<Sample>("sample", &id)
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
    let (status, _) = call(&app, "POST", "/api/captures", capture).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.store.samples().unwrap().len(), 1);
    let s = state.store.settings();
    let settings = json!({"base_url":format!("http://{addr}/v1"),"model":"test-vision","api_key":"synthetic-test-key","interval_seconds":30,"task":s.task,"report_hour":20});
    let (status, response) = call(&app, "PUT", "/api/settings", settings.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!response.to_string().contains("synthetic-test-key"));
    assert_eq!(response["interval_seconds"], 60); // Old client choice is ignored.
    assert_eq!(response["judgment_ttl_seconds"], 120);
    let mut preserve = settings;
    preserve["api_key"] = json!("");
    call(&app, "PUT", "/api/settings", preserve).await;
    assert_eq!(state.store.settings().api_key, "synthetic-test-key");
    call(
        &app,
        "POST",
        &format!("/api/samples/{id}/analyze"),
        json!({}),
    )
    .await;
    await_sample(&state, &id).await;
    assert_eq!(image_requests.load(Ordering::SeqCst), 1);
    call(
        &app,
        "PUT",
        &format!("/api/samples/{id}/correction"),
        json!({"category":"work","note":"在手机找素材"}),
    )
    .await;
    call(
        &app,
        "POST",
        &format!("/api/samples/{id}/analyze"),
        json!({}),
    )
    .await;
    await_sample(&state, &id).await;
    assert_eq!(
        state
            .store
            .get::<Sample>("sample", &id)
            .unwrap()
            .unwrap()
            .correction_note,
        "在手机找素材"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let stop_path = format!("/api/sessions/{}/stop", session["id"].as_str().unwrap());
    let (_, stopped) = call(&app, "POST", &stop_path, json!({})).await;
    let (_, again) = call(&app, "POST", &stop_path, json!({})).await;
    assert_eq!(stopped["ended_at"], again["ended_at"]);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let (_, d) = call(
        &app,
        "GET",
        &format!("/api/day?date={date}&mode=live"),
        json!(null),
    )
    .await;
    assert!(d["work_seconds"].as_i64().unwrap() >= 1);
    assert_eq!(d["distracted_seconds"], 0);
    let (status, _) = call(
        &app,
        "POST",
        "/api/report",
        json!({"date":date,"mode":"live"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for _ in 0..100 {
        if state
            .store
            .get::<Report>("report", &format!("live:{date}"))
            .unwrap()
            .is_some_and(|r| r.state == "done")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let report = state
        .store
        .get::<Report>("report", &format!("live:{date}"))
        .unwrap()
        .unwrap();
    assert_eq!(report.headline, "受控测试日报");
    let (status, _) = call(&app, "GET", &format!("/api/media/{id}/screen"), json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    mock_task.abort();
}
#[tokio::test]
async fn unsafe_origins_and_bad_inputs_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let app = router(App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap());
    for (host, origin, method, client) in [
        ("attacker.test:17901", "", "GET", true),
        ("127.0.0.1:17901", "https://attacker.test", "GET", true),
        ("127.0.0.1:17901", "", "POST", false),
    ] {
        let mut request = Request::builder()
            .uri("/api/settings")
            .method(method)
            .header("host", host);
        if !origin.is_empty() {
            request = request.header("origin", origin)
        }
        if client {
            request = request.header("x-flow-insight-client", "web")
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let (status, _) = call(
        &app,
        "GET",
        "/api/day?date=not-a-date&mode=live",
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(flow_insight::api::decode_image("data:image/jpeg;base64,bad").is_err());
}

#[tokio::test]
async fn reset_data_requires_confirmation_and_preserves_settings() {
    let root = tempfile::tempdir().unwrap();
    let state = App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap();
    let mut settings = state.store.settings();
    settings.api_key = "synthetic-key-to-preserve".into();
    settings.daily_goal_minutes = 240;
    state.store.save_settings(settings.clone()).unwrap();
    let app = router(state.clone());

    seed_live_day(&state.store);
    let nested = state.store.root.join("captures").join("legacy");
    private_dir(&nested).unwrap();
    private_write(&state.store.root.join("captures/sample.jpg"), b"screen").unwrap();
    private_write(&nested.join("camera.jpg"), b"camera").unwrap();

    let (status, _) = call(
        &app,
        "POST",
        "/api/data/reset",
        json!({"confirmation":"清空"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(state.store.samples().unwrap().len(), 10);

    let sample_id = state.store.samples().unwrap()[0].id.clone();
    state
        .store
        .update_sample(&sample_id, |sample| sample.state = "queued".into())
        .unwrap();
    let (status, _) = call(
        &app,
        "POST",
        "/api/data/reset",
        json!({"confirmation":"清空全部数据"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    state
        .store
        .update_sample(&sample_id, |sample| sample.state = "done".into())
        .unwrap();

    let (status, response) = call(
        &app,
        "POST",
        "/api/data/reset",
        json!({"confirmation":"清空全部数据"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["deleted"]["samples"], 10);
    assert_eq!(response["deleted"]["sessions"], 1);
    assert_eq!(response["deleted"]["reports"], 1);
    assert_eq!(response["deleted"]["files"], 2);
    assert!(state.store.samples().unwrap().is_empty());
    assert!(state
        .store
        .list::<Report>("report", None)
        .unwrap()
        .is_empty());
    assert!(state
        .store
        .list::<flow_insight::models::Session>("session", None)
        .unwrap()
        .is_empty());
    assert_eq!(
        std::fs::read_dir(state.store.root.join("captures"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(state.store.settings().api_key, settings.api_key);
    assert_eq!(
        state.store.settings().daily_goal_minutes,
        settings.daily_goal_minutes
    );
}

#[tokio::test]
async fn export_download_is_scoped_and_never_contains_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    let mut s = store.settings();
    s.api_key = "synthetic-secret-never-export".into();
    store.save_settings(s).unwrap();
    let date = seed_live_day(&store);
    let app = router(App::new(store, 17901).unwrap());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/export?date={date}"))
                .header("host", "127.0.0.1:17901")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("attachment; filename="));
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-secret"));
    let data: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(data["records"].as_array().unwrap().len(), 10);
    assert_eq!(data["source"], "本机采样");
}

#[tokio::test]
async fn changing_provider_requires_its_own_key() {
    let root = tempfile::tempdir().unwrap();
    let state = App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap();
    let mut settings = state.store.settings();
    settings.api_key = "synthetic-qwen-key".into();
    state.store.save_settings(settings.clone()).unwrap();
    let app = router(state.clone());
    let mut input = settings.public();
    input["base_url"] = json!("https://api.deepseek.com");
    input["model"] = json!("deepseek-flash");
    input["api_key"] = json!("");
    assert_eq!(
        call(&app, "PUT", "/api/settings", input.clone()).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(state.store.settings().base_url, settings.base_url);
    assert_eq!(state.store.settings().api_key, settings.api_key);
    input["api_key"] = json!("synthetic-deepseek-key");
    let (status, response) = call(&app, "PUT", "/api/settings", input.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!response.to_string().contains("synthetic-deepseek-key"));
    input["base_url"] = json!("https://api.deepseek.com/v1");
    input["api_key"] = json!("");
    assert_eq!(
        call(&app, "PUT", "/api/settings", input).await.0,
        StatusCode::OK
    );
    assert_eq!(state.store.settings().api_key, "synthetic-deepseek-key");
}

#[tokio::test]
async fn deepseek_and_qwen_use_their_own_thinking_parameters() {
    let mock = Router::new().route(
        "/chat/completions",
        post(|Json(body): Json<Value>| async move {
            if body["model"] == "deepseek-flash" {
                assert_eq!(body["thinking"]["type"], "disabled");
                assert!(body.get("enable_thinking").is_none());
            } else {
                assert_eq!(body["enable_thinking"], false);
                assert!(body.get("thinking").is_none());
            }
            assert_eq!(body["messages"][0]["content"][0]["type"], "image_url");
            Json(json!({"choices":[{"message":{"content":"synthetic vision reply"}}]}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let service = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    for model in ["deepseek-flash", "qwen3.8-flash"] {
        let settings = flow_insight::models::Settings {
            base_url: format!("http://{address}"),
            model: model.into(),
            api_key: "synthetic-key".into(),
            ..Default::default()
        };
        let reply = flow_insight::ai::complete(
            &reqwest::Client::new(),
            &settings,
            json!([{"role":"user","content":[{"type":"image_url","image_url":{"url":frame()}}]}]),
            100,
        )
        .await
        .unwrap();
        assert_eq!(reply, "synthetic vision reply");
    }
    service.abort();
}

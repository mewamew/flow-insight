use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use flow_insight::{
    api::{router, App},
    models::*,
    store::Store,
    summary,
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
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
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn frame() -> String {
    format!(
        "data:image/png;base64,{}",
        STANDARD.encode(include_bytes!("fixtures/synthetic-frame.png"))
    )
}
fn capture(session: &Value, missing: bool) -> Value {
    let at = now();
    json!({"id":uuid::Uuid::new_v4().to_string(),"session_id":session["id"],"captured_at":at,
        "capture_source":"多屏合成测试","activity":{"foreground_display_id":5},
        "evidence":{"basis":"ai","trigger":"started","presence":{},"away":false,"valid_until":at+30000},
        "screens":[{"display_id":1,"display_name":"内置屏幕","captured_at":at,"image":frame()},
        if missing {json!({"display_id":5,"display_name":"外接屏幕","captured_at":at,"error":"已断开"})}
        else {json!({"display_id":5,"display_name":"外接屏幕","captured_at":at,"image":frame()})}]})
}
async fn finished(state: &Arc<App>, id: &str) -> Sample {
    for _ in 0..200 {
        let sample = state.store.get::<Sample>("sample", id).unwrap().unwrap();
        if sample.state == "done" {
            return sample;
        }
        assert_ne!(sample.state, "error", "{:?}", sample.error);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("model request did not finish");
}

#[tokio::test]
async fn multiple_images_share_one_request_sample_and_duration_and_all_media_are_deleted() {
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let observed = requests.clone();
    let mock = Router::new().route("/v1/chat/completions", post(move |Json(v): Json<Value>| {
        observed.lock().unwrap().push(v);
        async { Json(json!({"choices":[{"message":{"content":json!({"category":"work","confidence":0.9,
            "app_name":"测试编辑器","screen_activity":"开发与查阅文档","camera_state":"无图片",
            "summary":"合成测试","evidence":["屏幕 1 的代码与屏幕 5 的文档"]}).to_string()}}]})) }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let service = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let state = App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap();
    let mut settings = state.store.settings();
    settings.api_key = "synthetic-test-key".into();
    settings.base_url = format!("http://{address}/v1");
    state.store.save_settings(settings).unwrap();
    let app = router(state.clone());
    let (_, session) = call(&app, "POST", "/api/sessions", json!({})).await;
    let input = capture(&session, false);
    let id = input["id"].as_str().unwrap();
    assert_eq!(
        call(&app, "POST", "/api/captures", input.clone()).await.0,
        StatusCode::OK
    );
    let sample = finished(&state, id).await;
    assert_eq!(
        call(&app, "POST", "/api/captures", input.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(state.store.samples().unwrap().len(), 1);
    assert_eq!(sample.screens.len(), 2);
    assert!(sample.camera.is_none());
    assert_eq!(sample.screen_files().len(), 2);
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let parts = requests[0]["messages"][1]["content"].as_array().unwrap();
        assert_eq!(parts.iter().filter(|p| p["type"] == "image_url").count(), 2);
        assert!(parts[0]["text"]
            .as_str()
            .unwrap()
            .contains("只输出一个整体工作状态"));
        let prompt = parts[0]["text"].as_str().unwrap();
        assert!(prompt.contains("work|distracted|unknown"));
        assert!(prompt.contains("未填写任务：按活动性质判断"));
        assert!(prompt.contains("\"task\":\"\""));
        assert!(!prompt.contains("任务为空时只能描述活动"));
        assert!(!prompt.contains("work|distracted|break|unknown"));
        assert!(parts[1]["text"].as_str().unwrap().contains("屏幕 1"));
        assert!(parts[3]["text"].as_str().unwrap().contains("屏幕 5"));
    }
    for kind in ["screen", "screen-1", "screen-5"] {
        assert_eq!(
            call(&app, "GET", &format!("/api/media/{id}/{kind}"), Value::Null)
                .await
                .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/media/{id}/screen-9"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut timed = sample.clone();
    let at = summary::bounds("2026-09-09").unwrap().0 + 3_600_000;
    timed.captured_at = at;
    timed.evidence.as_mut().unwrap().valid_until = at + 30000;
    let mut session: Session = serde_json::from_value(session).unwrap();
    session.started_at = at;
    session.ended_at = Some(at + 30000);
    let day = summary::build("2026-09-09", "live", vec![timed], vec![session], at + 30000).unwrap();
    assert_eq!(day.work_seconds, 30);
    assert_eq!(day.ai_observed_seconds, 30);
    assert_eq!(day.counts["done"], 1);
    assert_eq!(flow_insight::flow::derived(&day.segments)["ai_samples"], 1);
    assert_eq!(
        std::fs::read_dir(root.path().join("captures"))
            .unwrap()
            .count(),
        2
    );
    state.store.erase_range(0, now() + 1000, "live").unwrap();
    assert_eq!(
        std::fs::read_dir(root.path().join("captures"))
            .unwrap()
            .count(),
        0
    );
    service.abort();
}

#[tokio::test]
async fn partial_capture_keeps_missing_display_and_model_receives_the_gap() {
    let seen = Arc::new(Mutex::new(None));
    let observed = seen.clone();
    let mock = Router::new().route("/v1/chat/completions",post(move |Json(v): Json<Value>| {
        *observed.lock().unwrap()=Some(v);
        async { Json(json!({"choices":[{"message":{"content":json!({"category":"unknown","confidence":0.2,
            "app_name":"未知","screen_activity":"主操作画面缺失","camera_state":"无图片","summary":"信息不足","evidence":["屏幕 5 缺失"]}).to_string()}}]})) }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let service = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let state = App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap();
    let mut settings = state.store.settings();
    settings.api_key = "synthetic-test-key".into();
    settings.base_url = format!("http://{address}/v1");
    state.store.save_settings(settings).unwrap();
    let app = router(state.clone());
    let (_, session) = call(&app, "POST", "/api/sessions", json!({})).await;
    let input = capture(&session, true);
    let id = input["id"].as_str().unwrap();
    assert_eq!(
        call(&app, "POST", "/api/captures", input.clone()).await.0,
        StatusCode::OK
    );
    let sample = finished(&state, id).await;
    assert_eq!(sample.category(), Category::Unknown);
    assert_eq!(sample.screens.len(), 2);
    assert!(sample.screens[1].file.is_none());
    assert!(sample
        .capture_warning
        .unwrap()
        .contains("前台窗口所在屏幕未包含"));
    let request = seen.lock().unwrap().clone().unwrap();
    let parts = request["messages"][1]["content"].as_array().unwrap();
    assert_eq!(parts.iter().filter(|p| p["type"] == "image_url").count(), 1);
    let context = parts[0]["text"].as_str().unwrap();
    assert!(context.contains("\"foreground_display_id\":5"));
    assert!(context.contains("\"available\":false"));
    assert!(context.contains("若前台屏幕缺失且剩余证据不足，必须 unknown"));
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/media/{id}/screen-5"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    service.abort();
}

#[tokio::test]
async fn invalid_group_never_leaves_partial_files_and_legacy_single_screen_still_loads() {
    let root = tempfile::tempdir().unwrap();
    let state = App::new(Store::open(root.path().into()).unwrap(), 17901).unwrap();
    let app = router(state.clone());
    let (_, session) = call(&app, "POST", "/api/sessions", json!({})).await;
    for kind in ["duplicate", "invalid_image", "all_failed", "camera"] {
        let mut input = capture(&session, false);
        match kind {
            "duplicate" => input["screens"][1]["display_id"] = json!(1),
            "invalid_image" => input["screens"][1]["image"] = json!("bad-image"),
            "all_failed" => {
                for screen in input["screens"].as_array_mut().unwrap() {
                    screen.as_object_mut().unwrap().remove("image");
                    screen["error"] = json!("已断开");
                }
            }
            "camera" => input["camera"] = json!(frame()),
            _ => unreachable!(),
        }
        assert_eq!(
            call(&app, "POST", "/api/captures", input).await.0,
            StatusCode::BAD_REQUEST,
            "{kind}"
        );
        assert!(state.store.samples().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(root.path().join("captures"))
                .unwrap()
                .count(),
            0
        );
    }
    let id = uuid::Uuid::new_v4().to_string();
    let (status,mut sample)=call(&app,"POST","/api/captures",json!({"id":id,"session_id":session["id"],"captured_at":now(),"screen":frame(),"capture_source":"旧版单屏"})).await;
    assert_eq!(status, StatusCode::OK);
    sample.as_object_mut().unwrap().remove("screens");
    let legacy: Sample = serde_json::from_value(sample).unwrap();
    assert!(legacy.screens.is_empty());
    assert_eq!(legacy.screen_files().len(), 1);
    assert_eq!(
        call(&app, "GET", &format!("/api/media/{id}/screen"), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
}

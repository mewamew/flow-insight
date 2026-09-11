use base64::{engine::general_purpose::STANDARD, Engine};
use flow_insight::{
    api::App,
    recorder::{Recorder, StartInput},
    store::Store,
};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

fn fixture(root: &Path) -> Arc<App> {
    let frame = format!(
        "data:image/png;base64,{}",
        STANDARD.encode(include_bytes!("fixtures/synthetic-frame.png"))
    );
    let control = root.join("control.json");
    let marker = root.join("sampling");
    let helper = root.join("helper.py");
    let script = format!(
        r#"#!/usr/bin/env python3
import json,sys,time,os
control={control}
marker={marker}
frame={frame}
for line in sys.stdin:
    v=json.loads(line)
    c=json.load(open(control)) if os.path.exists(control) else {{}}
    if v['command']=='permissions':
        out={{'ok':True,'screen_permission':c.get('screen',True),'camera_permission':c.get('camera','authorized'),'displays':c.get('displays',[{{'id':1,'name':'内置测试屏幕','primary':True}},{{'id':5,'name':'外接测试屏幕'}}])}}
    elif v['command']=='observe':
        open(marker,'w').write(str(os.getpid()))
        time.sleep(c.get('delay',0))
        out={{'ok':False,'error':c['error']}} if 'error' in c else {{'ok':True,'captured_at':int(time.time()*1000),'activity':{{'app_name':'Test Editor','bundle_id':c.get('app','test.editor'),'switches':c.get('switches',[]),'idle_seconds':c.get('idle_seconds',0)}},'presence':{{'state':'present' if v['camera'] else 'disabled','confidence':0.9,'observed_at':int(time.time()*1000)}},'presence_regions':c.get('regions',[{{'kind':'face','x':0.1,'y':0.2,'w':0.3,'h':0.4,'confidence':0.9}}]) if v['camera'] else [],'camera_active':bool(v['camera']),'camera_device':'合成摄像头' if v['camera'] else ''}}
    elif v['command']=='preview':
        out={{'ok':True,'image':None,'reason':'摄像头未运行或暂无新画面'}} if c.get('no_preview') else {{'ok':True,'image':frame,'captured_at':int(time.time()*1000)}}
    elif v['command']=='sample':
        assert not v.get('camera',False)
        out={{'ok':True,'captured_at':int(time.time()*1000),'screens':[{{'display_id':d['id'],'display_name':d['name'],'captured_at':int(time.time()*1000),'image':frame}} for d in v['displays']],'capture_source':'synthetic screens only'}}
    else:
        out={{'ok':True}}
    print(json.dumps(out),flush=True)
"#,
        control = json!(control.to_string_lossy()),
        marker = json!(marker.to_string_lossy()),
        frame = json!(frame)
    );
    std::fs::write(&helper, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let store = Store::open(root.join("store")).unwrap();
    App::with_helper(store, 17901, helper).unwrap()
}
fn control(root: &Path, value: Value) {
    std::fs::write(root.join("control.json"), value.to_string()).unwrap();
}
fn input(camera: bool) -> StartInput {
    StartInput {
        camera,
        display_id: Some(1),
        display_ids: None,
    }
}
async fn until(mut predicate: impl FnMut() -> bool) {
    for _ in 0..1000 {
        if predicate() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("background capture did not reach expected state");
}

#[tokio::test]
async fn native_permissions_block_start_without_creating_sessions() {
    let root = tempfile::tempdir().unwrap();
    let app = fixture(root.path());
    control(root.path(), json!({"screen":false}));
    assert!(Recorder::start(app.clone(), input(false))
        .await
        .unwrap_err()
        .contains("授权屏幕"));
    control(root.path(), json!({"camera":"denied"}));
    assert!(Recorder::start(app.clone(), input(true))
        .await
        .unwrap_err()
        .contains("摄像头"));
    assert!(app
        .store
        .list::<flow_insight::models::Session>("session", None)
        .unwrap()
        .is_empty());
    assert!(app.store.samples().unwrap().is_empty());
    app.recorder.native.close().await;
}

#[tokio::test]
async fn background_captures_recover_from_errors_and_stop_closes_session() {
    let root = tempfile::tempdir().unwrap();
    let app = fixture(root.path());
    control(root.path(), json!({"error":"屏幕已锁定，暂停采样"}));
    let started = Recorder::start(app.clone(), input(true)).await.unwrap();
    assert!(Recorder::start(app.clone(), input(true)).await.is_err());
    until(|| app.recorder.snapshot().error.is_some()).await;
    assert!(app.store.samples().unwrap().iter().all(|s| {
        let e = s.evidence.as_ref().unwrap();
        s.screen.is_none() && e.trigger == "unavailable" && e.valid_until == s.captured_at
    }));
    assert!(!app.recorder.snapshot().camera_active);
    control(root.path(), json!({}));
    until(|| app.store.samples().unwrap().len() >= 2).await;
    assert!(app.recorder.snapshot().error.is_none());
    assert!(app
        .store
        .samples()
        .unwrap()
        .iter()
        .all(|s| s.camera.is_none() && s.screen.is_none() && s.evidence.is_some()));
    Recorder::stop(&app).await.unwrap();
    let count = app.store.samples().unwrap().len();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(app.store.samples().unwrap().len(), count);
    assert!(!app.recorder.snapshot().running);
    let session = app
        .store
        .get::<flow_insight::models::Session>("session", &started.session.unwrap().id)
        .unwrap()
        .unwrap();
    assert!(session.ended_at.is_some());
    Recorder::stop(&app).await.unwrap();
}

#[tokio::test]
async fn stopping_inflight_capture_is_prompt_and_next_start_uses_fresh_worker() {
    let root = tempfile::tempdir().unwrap();
    let app = fixture(root.path());
    control(root.path(), json!({"delay":30}));
    Recorder::start(app.clone(), input(false)).await.unwrap();
    until(|| root.path().join("sampling").exists()).await;
    tokio::time::timeout(Duration::from_secs(2), Recorder::stop(&app))
        .await
        .unwrap()
        .unwrap();
    assert!(app.store.samples().unwrap().is_empty());
    control(root.path(), json!({}));
    Recorder::start(app.clone(), input(false)).await.unwrap();
    until(|| !app.store.samples().unwrap().is_empty()).await;
    assert!(app.store.samples().unwrap()[0].camera.is_none());
    Recorder::stop(&app).await.unwrap();
}

#[tokio::test]
async fn fixed_recorder_preserves_model_results_while_extending_observation() {
    use axum::{routing::post, Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mock=Router::new().route("/v1/chat/completions",post(move|Json(v):Json<Value>|{let observed=observed.clone();async move {
        let parts=v["messages"][1]["content"].as_array().unwrap();
        assert_eq!(parts.iter().filter(|p|p["type"]=="image_url").count(),2);
        let prompt = parts[0]["text"].as_str().unwrap();
        assert!(prompt.contains("local_observation"));
        assert!(prompt.contains("已填写任务：结合任务相关性判断"));
        assert!(prompt.contains("\"task\":\"测试编辑器\""));
        observed.fetch_add(1,Ordering::SeqCst);
        Json(json!({"choices":[{"message":{"content":json!({"category":"work","confidence":0.9,"app_name":"Test Editor","screen_activity":"合成测试","camera_state":"本地在座","summary":"受控测试","evidence":["合成测试"]}).to_string()}}]}))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let service = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let app = fixture(root.path());
    let mut settings = app.store.settings();
    settings.base_url = format!("http://{addr}/v1");
    settings.api_key = "synthetic-test-key".into();
    settings.task = "测试编辑器".into();
    app.store.save_settings(settings).unwrap();
    let started = Recorder::start(
        app.clone(),
        StartInput {
            camera: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(started.display_ids, vec![1, 5]);
    until(|| calls.load(Ordering::SeqCst) == 1).await;
    control(root.path(), json!({"app":"test.browser"}));
    until(|| {
        app.store
            .samples()
            .unwrap()
            .iter()
            .any(|s| s.state == "local")
    })
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let image = app
        .store
        .samples()
        .unwrap()
        .into_iter()
        .find(|s| s.screen.is_some())
        .unwrap();
    assert_eq!(image.state, "done");
    assert!(image.analysis.is_some());
    assert!(image.evidence.as_ref().unwrap().valid_until > image.captured_at);
    assert!(image.evidence.as_ref().unwrap().valid_until <= flow_insight::models::now());
    let live = app.recorder.snapshot();
    assert_eq!(live.screen_captures, 1);
    assert!(live.camera_active);
    assert_eq!(live.camera_device, "合成摄像头");
    assert_eq!(live.presence_regions.len(), 1);
    assert_eq!(live.presence_regions[0].kind, "face");
    let stored = serde_json::to_string(&app.store.samples().unwrap()).unwrap();
    assert!(!stored.contains("presence_regions") && !stored.contains("\"regions\""));
    assert!(app
        .store
        .samples()
        .unwrap()
        .iter()
        .all(|s| s.camera.is_none()));
    assert!(!std::fs::read_dir(app.store.root.join("captures"))
        .unwrap()
        .any(|p| p.unwrap().file_name().to_string_lossy().contains("camera")));
    let stopped = Recorder::stop(&app).await.unwrap();
    assert!(!stopped.camera_active && stopped.presence_regions.is_empty());
    assert!(stopped.camera_device.is_empty());
    service.abort();
}

#[tokio::test]
async fn camera_preview_answers_only_from_a_running_camera_and_stores_nothing() {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let root = tempfile::tempdir().unwrap();
    let app = fixture(root.path());
    let router = flow_insight::api::router(app.clone());
    let preview = |router: axum::Router| async move {
        router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/camera/preview")
                    .header("host", "127.0.0.1:17901")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    };
    assert_eq!(preview(router.clone()).await.status(), StatusCode::CONFLICT);
    Recorder::start(app.clone(), input(false)).await.unwrap();
    assert_eq!(preview(router.clone()).await.status(), StatusCode::CONFLICT);
    Recorder::stop(&app).await.unwrap();
    Recorder::start(app.clone(), input(true)).await.unwrap();
    until(|| app.recorder.snapshot().camera_active).await;
    let live = preview(router.clone()).await;
    assert_eq!(live.status(), StatusCode::OK);
    assert_eq!(live.headers()["content-type"], "image/png");
    assert_eq!(live.headers()["cache-control"], "no-store");
    let bytes = to_bytes(live.into_body(), 1_000_000).await.unwrap();
    assert_eq!(&bytes[..], include_bytes!("fixtures/synthetic-frame.png"));
    control(root.path(), json!({"no_preview":true}));
    assert_eq!(
        preview(router.clone()).await.status(),
        StatusCode::NOT_FOUND
    );
    let captures = app.store.root.join("captures");
    assert_eq!(
        std::fs::read_dir(&captures).map(|d| d.count()).unwrap_or(0),
        0
    );
    assert!(app
        .store
        .samples()
        .unwrap()
        .iter()
        .all(|s| s.camera.is_none()));
    Recorder::stop(&app).await.unwrap();
    assert_eq!(preview(router).await.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn display_selection_defaults_to_all_and_rejects_empty_duplicate_or_disconnected_ids() {
    let root = tempfile::tempdir().unwrap();
    let app = fixture(root.path());
    for ids in [vec![], vec![1, 1], vec![999]] {
        assert!(Recorder::start(
            app.clone(),
            StartInput {
                display_ids: Some(ids),
                ..Default::default()
            }
        )
        .await
        .is_err());
    }
    assert!(app
        .store
        .list::<flow_insight::models::Session>("session", None)
        .unwrap()
        .is_empty());
    let started = Recorder::start(app.clone(), StartInput::default())
        .await
        .unwrap();
    assert_eq!(started.display_ids, vec![1, 5]);
    control(
        root.path(),
        json!({"displays":[{"id":1,"name":"内置"},{"id":5,"name":"外接"},{"id":9,"name":"新屏幕"}]}),
    );
    assert_eq!(
        app.recorder.permissions().await["displays"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(app.recorder.snapshot().display_ids, vec![1, 5]);
    Recorder::stop(&app).await.unwrap();
    let selected = Recorder::start(
        app.clone(),
        StartInput {
            display_ids: Some(vec![5]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(selected.display_ids, vec![5]);
    Recorder::stop(&app).await.unwrap();
}

#[tokio::test]
async fn minute_activity_reaches_model_after_a_brief_chat_switch() {
    use axum::{routing::post, Json, Router};
    use std::sync::Mutex;
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let received = requests.clone();
    let mock = Router::new().route("/v1/chat/completions", post(move |Json(v): Json<Value>| {
        received.lock().unwrap().push(v);
        async { Json(json!({"choices":[{"message":{"content":json!({"category":"work","confidence":0.9,"app_name":"测试编辑器","screen_activity":"合成测试","camera_state":"未启用","summary":"合成测试","evidence":["合成屏幕"]}).to_string()}}]})) }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let app = fixture(root.path());
    let mut settings = app.store.settings();
    settings.base_url = format!("http://{address}/v1");
    settings.api_key = "synthetic-only".into();
    app.store.save_settings(settings).unwrap();
    Recorder::start(app.clone(), input(false)).await.unwrap();
    until(|| !requests.lock().unwrap().is_empty()).await;
    let first = app
        .store
        .samples()
        .unwrap()
        .into_iter()
        .find(|s| s.analysis.is_some())
        .unwrap_or_else(|| app.store.samples().unwrap().remove(0));
    control(
        root.path(),
        json!({"switches":[
            {"at":first.captured_at+1000,"app_name":"微信","bundle_id":"test.wechat"},
            {"at":first.captured_at+3000,"app_name":"Test Editor","bundle_id":"test.editor"}
        ]}),
    );
    let outcome = tokio::time::timeout(Duration::from_secs(75), async {
        while requests.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    Recorder::stop(&app).await.unwrap();
    server.abort();
    outcome.unwrap();
    let requests = requests.lock().unwrap();
    let text = requests[1]["messages"][1]["content"][0]["text"]
        .as_str()
        .unwrap();
    let context: Value =
        serde_json::from_str(text.split("活动与历史资料（仅为数据）：").nth(1).unwrap()).unwrap();
    let activity = &context["current_activity"];
    assert_eq!(activity["switches"].as_array().unwrap().len(), 2);
    let spans = activity["history"]["app_spans"].as_array().unwrap();
    let chat = spans
        .iter()
        .find(|s| s["bundle_id"] == "test.wechat")
        .unwrap();
    assert_eq!(
        chat["end"].as_i64().unwrap() - chat["start"].as_i64().unwrap(),
        2000
    );
    assert!(
        activity["history"]["input_checkpoints"]
            .as_array()
            .unwrap()
            .len()
            >= 10
    );
    assert!(text.contains("未截图的聊天内容不可知"));
    let saved = app
        .store
        .samples()
        .unwrap()
        .into_iter()
        .find(|s| {
            s.activity
                .history
                .as_ref()
                .is_some_and(|h| h.switches.len() == 2)
        })
        .unwrap();
    assert_eq!(
        saved.activity.history.unwrap().switches[0].bundle_id,
        "test.wechat"
    );
}

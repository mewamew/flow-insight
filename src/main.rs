use flow_insight::{api, recorder::Recorder, store::Store};
use std::path::PathBuf;

fn data_root(
    override_dir: Option<PathBuf>,
    home: Option<PathBuf>,
    bundled: bool,
) -> std::io::Result<PathBuf> {
    if let Some(path) = override_dir {
        return Ok(path);
    }
    if bundled {
        return home
            .map(|path| path.join("Library/Application Support/Flow Insight"))
            .ok_or_else(|| {
                std::io::Error::other("无法确定用户目录，请设置 FLOW_INSIGHT_DATA_DIR")
            });
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/runtime"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::var("FLOW_INSIGHT_PORT")
        .unwrap_or_else(|_| "17901".into())
        .parse()?;
    let bundled = std::env::current_exe().ok().is_some_and(|path| {
        path.parent()
            .and_then(|p| p.parent())
            .is_some_and(|contents| {
                contents.join("Info.plist").is_file()
                    && contents.join("Resources/web/index.html").is_file()
            })
    });
    let root = data_root(
        std::env::var_os("FLOW_INSIGHT_DATA_DIR").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        bundled,
    )?;
    // Reject a second instance before restart recovery can touch the live store.
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let store = Store::open(root).map_err(std::io::Error::other)?;
    let cutoff = flow_insight::models::now() - store.settings().retention_days as i64 * 86_400_000;
    store
        .erase_range(0, cutoff, "live")
        .map_err(std::io::Error::other)?;
    let app = api::App::new(store, port).map_err(std::io::Error::other)?;
    api::start_scheduler(app.clone());
    println!("Flow Insight 已启动：http://127.0.0.1:{port}（Ctrl+C 停止）");
    // The development start script opens the browser itself. Finder launches
    // have no data override and must not depend on that script.
    #[cfg(target_os = "macos")]
    if bundled
        && std::env::var_os("FLOW_INSIGHT_DATA_DIR").is_none()
        && std::env::var("FLOW_INSIGHT_OPEN_BROWSER").as_deref() != Ok("0")
    {
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let url = format!("http://127.0.0.1:{port}");
            for _ in 0..50 {
                if client
                    .get(format!("{url}/api/status"))
                    .timeout(std::time::Duration::from_secs(1))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    let _ = tokio::process::Command::new("/usr/bin/open")
                        .arg(&url)
                        .status()
                        .await;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });
    }
    axum::serve(listener, api::router(app.clone()))
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            let _ = Recorder::stop(&app).await;
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_app_uses_user_storage_and_preserves_explicit_override() {
        let home = PathBuf::from("/Users/test-user");
        assert_eq!(
            data_root(None, Some(home.clone()), true).unwrap(),
            home.join("Library/Application Support/Flow Insight")
        );
        let isolated = PathBuf::from("/tmp/flow-insight-test");
        assert_eq!(
            data_root(Some(isolated.clone()), None, true).unwrap(),
            isolated
        );
        assert!(data_root(None, None, true).is_err());
        assert_eq!(
            data_root(None, None, false).unwrap(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/runtime")
        );
    }
}

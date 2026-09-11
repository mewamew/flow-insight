//! Persistent native capture worker. Cancellation drops and kills the worker so
//! a late reply can never be mistaken for the next command's reply.
use crate::store::Result;
use serde_json::Value;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

struct Connection {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
pub struct NativeCapture {
    path: PathBuf,
    connection: Mutex<Option<Connection>>,
}
impl NativeCapture {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            connection: Mutex::new(None),
        }
    }
    pub fn default_path() -> PathBuf {
        std::env::var_os("FLOW_INSIGHT_CAPTURE_HELPER")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap_or_default()
                    .with_file_name("capture-helper")
            })
    }
    pub async fn request(&self, message: Value) -> Result<Value> {
        let mut slot = self.connection.lock().await;
        let mut conn = match slot.take() {
            Some(c) => c,
            None => {
                let mut child = Command::new(&self.path)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|_| {
                        "原生采集程序未安装或无法启动，请使用 scripts/start.sh 启动 macOS 应用"
                    })?;
                Connection {
                    input: child.stdin.take().ok_or("采集进程输入不可用")?,
                    output: BufReader::new(child.stdout.take().ok_or("采集进程输出不可用")?),
                    child,
                }
            }
        };
        let timeout = if message["command"] == "request_permission" {
            120
        } else {
            20
        };
        let result = tokio::time::timeout(Duration::from_secs(timeout), async {
            conn.input
                .write_all(format!("{message}\n").as_bytes())
                .await
                .map_err(|_| "采集进程已退出")?;
            conn.input.flush().await.map_err(|_| "无法通知采集进程")?;
            let mut line = String::new();
            // Worker output is private IPC, not a network endpoint. Images are
            // independently size-checked and decoded before persistence.
            if conn
                .output
                .read_line(&mut line)
                .await
                .map_err(|_| "无法读取采集结果")?
                == 0
            {
                return Err("采集进程已退出，请重新开始记录".to_string());
            }
            if line.len() > crate::models::MAX_DISPLAYS * 3_000_000 + 1_000_000 {
                return Err("采集结果超出大小限制".into());
            }
            serde_json::from_str::<Value>(&line).map_err(|_| "原生采集结果格式无效".into())
        })
        .await
        .map_err(|_| "原生采集超时；本次未计入活动时间，请检查系统权限")?;
        if let Ok(value) = &result {
            *slot = Some(conn);
            if value["ok"] != true {
                return Err(value["error"]
                    .as_str()
                    .unwrap_or("原生采集失败")
                    .to_string());
            }
        }
        result
    }
    pub async fn close(&self) {
        if let Some(mut conn) = self.connection.lock().await.take() {
            let _ = conn.child.kill().await;
            let _ = conn.child.wait().await;
        }
    }
}

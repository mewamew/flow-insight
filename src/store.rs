use crate::models::{now, Report, Sample, Session, Settings};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub type Result<T> = std::result::Result<T, String>;
#[derive(Debug, PartialEq)]
pub struct DataReset {
    pub samples: usize,
    pub sessions: usize,
    pub reports: usize,
    pub files: usize,
}
pub struct Store {
    db: Mutex<Connection>,
    pub root: PathBuf,
    settings: Mutex<Settings>,
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|_| "无法创建数据目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法设置数据目录权限")?;
    }
    Ok(())
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path).map_err(|_| "无法写入本地文件")?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|_| "本地文件写入失败".into())
}
impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        private_dir(&root)?;
        private_dir(&root.join("captures"))?;
        let db =
            Connection::open(root.join("flow-insight.sqlite3")).map_err(|_| "无法打开数据库")?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; CREATE TABLE IF NOT EXISTS objects(kind TEXT NOT NULL,id TEXT NOT NULL,stamp INTEGER NOT NULL,mode TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(kind,id)); CREATE INDEX IF NOT EXISTS objects_time ON objects(kind,mode,stamp);").map_err(|_|"数据库初始化失败")?;
        let settings = match fs::read(root.join("settings.json")) {
            Ok(b) => {
                serde_json::from_slice(&b).map_err(|_| "设置文件损坏，请检查 settings.json")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(_) => return Err("无法读取设置".into()),
        };
        let store = Self {
            db: Mutex::new(db),
            root,
            settings: Mutex::new(settings),
        };
        // A service restart ends the old session; capture resumes only after an explicit start.
        for mut s in store.list::<Session>("session", None)? {
            if s.ended_at.is_none() {
                let last = store
                    .samples()?
                    .iter()
                    .filter(|x| x.session_id == s.id)
                    .map(|x| {
                        x.evidence
                            .as_ref()
                            .map(|e| e.valid_until)
                            .unwrap_or(x.captured_at + x.interval_seconds as i64 * 1000)
                    })
                    .max()
                    .unwrap_or(s.started_at);
                s.ended_at = Some(last.min(now()));
                store.put("session", &s.id, s.started_at, &s.mode, &s)?;
            }
        }
        for sample in store.samples()? {
            if sample.state == "analyzing" || sample.state == "queued" {
                store.update_sample(&sample.id, |s| {
                    s.state = "pending".into();
                    s.error = Some("上次分析被中断，可重新分析".into());
                })?;
            }
        }
        for mut report in store.list::<Report>("report", None)? {
            if report.state == "generating" {
                report.state = "error".into();
                report.error = Some("报告生成被中断，请重试".into());
                store.put(
                    "report",
                    &format!("{}:{}", report.mode, report.date),
                    report.generated_at,
                    &report.mode,
                    &report,
                )?;
            }
        }
        Ok(store)
    }
    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("settings lock").clone()
    }
    pub fn save_settings(&self, s: Settings) -> Result<()> {
        let mut guard = self.settings.lock().map_err(|_| "设置锁异常")?;
        let temp = self.root.join("settings.new");
        private_write(
            &temp,
            &serde_json::to_vec_pretty(&s).map_err(|_| "设置编码失败")?,
        )?;
        fs::rename(temp, self.root.join("settings.json")).map_err(|_| "保存设置失败")?;
        *guard = s;
        Ok(())
    }
    pub fn put<T: Serialize>(
        &self,
        kind: &str,
        id: &str,
        stamp: i64,
        mode: &str,
        item: &T,
    ) -> Result<()> {
        let body = serde_json::to_string(item).map_err(|_| "数据编码失败")?;
        self.db.lock().map_err(|_|"数据库锁异常")?.execute("INSERT INTO objects(kind,id,stamp,mode,body) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(kind,id) DO UPDATE SET body=excluded.body,stamp=excluded.stamp,mode=excluded.mode",params![kind,id,stamp,mode,body]).map_err(|_|"数据库写入失败")?;
        Ok(())
    }
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let data: Option<String> = self
            .db
            .lock()
            .map_err(|_| "数据库锁异常")?
            .query_row(
                "SELECT body FROM objects WHERE kind=?1 AND id=?2",
                params![kind, id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "数据库读取失败")?;
        data.map(|v| serde_json::from_str(&v).map_err(|_| "数据库内容损坏".into()))
            .transpose()
    }
    pub fn list<T: DeserializeOwned>(&self, kind: &str, mode: Option<&str>) -> Result<Vec<T>> {
        let db = self.db.lock().map_err(|_| "数据库锁异常")?;
        let mut stmt=db.prepare("SELECT body FROM objects WHERE kind=?1 AND (?2 IS NULL OR mode=?2) ORDER BY stamp,id").map_err(|_|"数据库查询失败")?;
        let rows = stmt
            .query_map(params![kind, mode], |r| r.get::<_, String>(0))
            .map_err(|_| "数据库查询失败")?;
        rows.map(|r| {
            serde_json::from_str(&r.map_err(|_| "数据库读取失败")?)
                .map_err(|_| "数据库内容损坏".into())
        })
        .collect()
    }
    pub fn erase_range(&self, start: i64, end: i64, mode: &str) -> Result<usize> {
        let reports: Vec<Report> = self.list("report", Some(mode))?;
        let report_ids: Vec<String> = reports
            .into_iter()
            .filter(|r| crate::summary::bounds(&r.date).is_ok_and(|(a, b)| a < end && b > start))
            .map(|r| format!("{}:{}", r.mode, r.date))
            .collect();
        let mut db = self.db.lock().map_err(|_| "数据库锁异常")?;
        let tx = db.transaction().map_err(|_| "无法开始删除")?;
        let files: Vec<String> = {
            let mut stmt=tx.prepare("SELECT body FROM objects WHERE kind='sample' AND mode=?1 AND stamp>=?2 AND stamp<?3").map_err(|_|"无法读取待删除记录")?;
            let rows = stmt
                .query_map(params![mode, start, end], |r| r.get::<_, String>(0))
                .map_err(|_| "无法读取待删除记录")?;
            rows.filter_map(|r| r.ok())
                .filter_map(|s| serde_json::from_str::<Sample>(&s).ok())
                .flat_map(|s| {
                    let mut files: Vec<String> =
                        s.screen_files().into_iter().map(str::to_string).collect();
                    files.extend(s.camera);
                    files
                })
                .collect()
        };
        let n = tx
            .execute(
                "DELETE FROM objects WHERE kind='sample' AND mode=?1 AND stamp>=?2 AND stamp<?3",
                params![mode, start, end],
            )
            .map_err(|_| "无法删除记录")?;
        for id in report_ids {
            tx.execute("DELETE FROM objects WHERE kind='report' AND id=?1", [id])
                .map_err(|_| "无法删除分析")?;
        }
        tx.commit().map_err(|_| "删除未完成")?;
        for file in files {
            let _ = fs::remove_file(self.root.join("captures").join(file));
        }
        Ok(n)
    }
    pub fn reset_data(&self) -> Result<DataReset> {
        let (samples, sessions, reports) = {
            let mut db = self.db.lock().map_err(|_| "数据库锁异常")?;
            db.execute_batch("PRAGMA secure_delete=ON;")
                .map_err(|_| "无法准备安全清理")?;
            let tx = db.transaction().map_err(|_| "无法开始重置")?;
            let count = |kind: &str| -> Result<usize> {
                tx.query_row(
                    "SELECT COUNT(*) FROM objects WHERE kind=?1",
                    [kind],
                    |row| row.get(0),
                )
                .map_err(|_| "无法统计待删除数据".into())
            };
            let counts = (count("sample")?, count("session")?, count("report")?);
            tx.execute("DELETE FROM objects", [])
                .map_err(|_| "无法清空记录")?;
            tx.commit().map_err(|_| "重置未完成")?;
            db.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(|_| "记录已清空，但数据库空间清理未完成；请重试")?;
            counts
        };
        let files = clear_directory(&self.root.join("captures"))?;
        Ok(DataReset {
            samples,
            sessions,
            reports,
            files,
        })
    }
    pub fn samples(&self) -> Result<Vec<Sample>> {
        self.list("sample", None)
    }
    pub fn update_sample(&self, id: &str, update: impl FnOnce(&mut Sample)) -> Result<Sample> {
        let db = self.db.lock().map_err(|_| "数据库锁异常")?;
        let body: String = db
            .query_row(
                "SELECT body FROM objects WHERE kind='sample' AND id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| "数据库读取失败")?
            .ok_or("记录不存在")?;
        let mut s: Sample = serde_json::from_str(&body).map_err(|_| "记录损坏")?;
        update(&mut s);
        db.execute(
            "UPDATE objects SET body=?1 WHERE kind='sample' AND id=?2",
            params![serde_json::to_string(&s).map_err(|_| "记录编码失败")?, id],
        )
        .map_err(|_| "记录保存失败")?;
        Ok(s)
    }
}

fn clear_directory(path: &Path) -> Result<usize> {
    let entries = fs::read_dir(path).map_err(|_| "无法读取本地画面目录")?;
    let mut files = 0;
    for entry in entries {
        let entry = entry.map_err(|_| "无法读取本地画面")?;
        let kind = entry.file_type().map_err(|_| "无法检查本地画面")?;
        if kind.is_dir() {
            files += count_files(&entry.path())?;
            fs::remove_dir_all(entry.path()).map_err(|_| {
                "记录已清空，但部分本地画面无法删除；请检查目录权限后重试".to_string()
            })?;
        } else {
            fs::remove_file(entry.path()).map_err(|_| {
                "记录已清空，但部分本地画面无法删除；请检查目录权限后重试".to_string()
            })?;
            files += 1;
        }
    }
    Ok(files)
}

fn count_files(path: &Path) -> Result<usize> {
    let mut files = 0;
    for entry in fs::read_dir(path).map_err(|_| "无法读取本地画面")? {
        let entry = entry.map_err(|_| "无法读取本地画面")?;
        if entry.file_type().map_err(|_| "无法检查本地画面")?.is_dir() {
            files += count_files(&entry.path())?;
        } else {
            files += 1;
        }
    }
    Ok(files)
}

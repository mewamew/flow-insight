use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    pub task: String,
    pub report_hour: u32,
    pub reminders: bool,
    pub reminder_minutes: u32,
    pub daily_goal_minutes: u32,
    pub retention_days: u32,
    pub excluded_apps: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            model: "qwen3.8-flash".into(),
            api_key: String::new(),
            task: String::new(),
            report_hour: 20,
            reminders: true,
            reminder_minutes: 3,
            daily_goal_minutes: 180,
            retention_days: 7,
            excluded_apps: vec![
                "com.apple.keychainaccess".into(),
                "com.1password.1password".into(),
            ],
        }
    }
}
impl Settings {
    pub fn public(&self) -> serde_json::Value {
        serde_json::json!({"base_url":self.base_url,"model":self.model,"api_key_configured":!self.api_key.is_empty(),"interval_seconds":crate::policy::ANALYSIS_INTERVAL_SECONDS,"judgment_ttl_seconds":crate::policy::JUDGMENT_TTL_MS / 1000,"task":self.task,"report_hour":self.report_hour,"reminders":self.reminders,"reminder_minutes":self.reminder_minutes,"daily_goal_minutes":self.daily_goal_minutes,"retention_days":self.retention_days,"excluded_apps":self.excluded_apps})
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Work,
    // Old records and model replies use `break`; read them as interruptions.
    #[serde(alias = "break")]
    Distracted,
    Away,
    Unknown,
}
impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Work => "工作中",
            Self::Distracted => "中断",
            Self::Away => "离开",
            Self::Unknown => "信息不足",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    pub category: Category,
    pub confidence: f64,
    pub app_name: String,
    pub screen_activity: String,
    pub camera_state: String,
    pub summary: String,
    pub evidence: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    /// One observation can contain several displays. `screen` remains a legacy preview alias.
    #[serde(default)]
    pub screens: Vec<ScreenCapture>,
    #[serde(default)]
    pub evidence: Option<CaptureEvidence>,
    #[serde(default)]
    pub activity: Activity,
    #[serde(default)]
    pub capture_warning: Option<String>,
    pub id: String,
    pub session_id: String,
    pub captured_at: i64,
    pub interval_seconds: u32,
    pub mode: String,
    pub task: String,
    pub screen: Option<String>,
    pub camera: Option<String>,
    pub capture_source: String,
    pub state: String,
    pub analysis: Option<Analysis>,
    pub error: Option<String>,
    pub correction: Option<Category>,
    pub correction_note: String,
}
impl Sample {
    pub fn screen_files(&self) -> Vec<&str> {
        if self.screens.is_empty() {
            self.screen.as_deref().into_iter().collect()
        } else {
            self.screens
                .iter()
                .filter_map(|s| s.file.as_deref())
                .collect()
        }
    }
    pub fn category(&self) -> Category {
        if self.evidence.as_ref().is_some_and(|e| e.away) {
            return Category::Away;
        }
        if self.evidence.as_ref().is_some_and(|e| e.basis == "local") {
            return Category::Unknown;
        }
        self.correction
            .clone()
            .or_else(|| self.analysis.as_ref().map(|a| a.category.clone()))
            .unwrap_or(Category::Unknown)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenCapture {
    pub display_id: u32,
    pub display_name: String,
    pub captured_at: i64,
    pub file: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectedDisplay {
    pub id: u32,
    #[serde(default)]
    pub name: String,
}
pub const MAX_DISPLAYS: usize = 16;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub interval_seconds: u32,
    pub task: String,
    pub mode: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub date: String,
    pub mode: String,
    pub state: String,
    pub headline: String,
    pub observations: Vec<String>,
    pub suggestions: Vec<String>,
    pub generated_at: i64,
    pub fingerprint: String,
    pub error: Option<String>,
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Activity {
    /// Estimated from the front application's frontmost visible window, never gaze.
    pub foreground_display_id: Option<u32>,
    pub app_name: String,
    pub bundle_id: String,
    pub window_title: String,
    pub idle_seconds: f64,
    pub switches: Vec<AppSwitch>,
    pub observed_since: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<ActivityHistory>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSwitch {
    pub at: i64,
    pub app_name: String,
    pub bundle_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityHistory {
    pub start: i64,
    pub end: i64,
    pub truncated: bool,
    pub switches: Vec<AppSwitch>,
    pub app_spans: Vec<AppDwell>,
    pub input_checkpoints: Vec<ActivityPoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppDwell {
    pub start: i64,
    pub end: i64,
    pub app_name: String,
    pub bundle_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivityPoint {
    pub at: i64,
    pub app_name: String,
    pub bundle_id: String,
    pub idle_seconds: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Present,
    NotDetected,
    #[default]
    Unknown,
    Disabled,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Presence {
    pub state: PresenceState,
    pub confidence: f64,
    pub observed_at: i64,
    pub reason: String,
}
/// Where a person appears in the camera frame, as normalized top-left boxes.
/// Geometry only: no pixels leave the native helper, and boxes are never stored.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PresenceRegion {
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub confidence: f64,
}
impl PresenceRegion {
    /// Clamp helper output into the unit square so the page can draw it blindly.
    pub fn sanitized(mut self) -> Option<Self> {
        let unit = |v: f64| v.clamp(0.0, 1.0);
        self.x = unit(self.x);
        self.y = unit(self.y);
        self.w = unit(self.w).min(1.0 - self.x);
        self.h = unit(self.h).min(1.0 - self.y);
        self.confidence = unit(self.confidence);
        (["face", "body"].contains(&self.kind.as_str()) && self.w > 0.0 && self.h > 0.0)
            .then_some(self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureEvidence {
    pub basis: String,
    pub trigger: String,
    pub presence: Presence,
    pub away: bool,
    pub valid_until: i64,
}

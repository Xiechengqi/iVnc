use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub frame: ObservationFrame,
    pub display: DisplayMetadata,
    pub windows: Option<serde_json::Value>,
    pub clipboard_preview: Option<String>,
    pub timestamp_ms: u64,
}

impl Observation {
    pub fn digest(&self) -> ObservationDigest {
        let (image_width, image_height, sha256, frame_path) = match &self.frame {
            ObservationFrame::JpegBytes {
                image_width,
                image_height,
                sha256,
                frame_path,
                ..
            }
            | ObservationFrame::PngBytes {
                image_width,
                image_height,
                sha256,
                frame_path,
                ..
            } => (
                *image_width,
                *image_height,
                sha256.clone(),
                frame_path.clone(),
            ),
            ObservationFrame::RawXrgb {
                screen_width,
                screen_height,
                sha256,
                ..
            } => (*screen_width, *screen_height, sha256.clone(), None),
        };
        ObservationDigest {
            screen_width: self.display.screen_width,
            screen_height: self.display.screen_height,
            image_width,
            image_height,
            sha256,
            frame_path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObservationFrame {
    JpegBytes {
        image_width: u32,
        image_height: u32,
        bytes: Vec<u8>,
        quality: u8,
        sha256: String,
        frame_path: Option<PathBuf>,
    },
    PngBytes {
        image_width: u32,
        image_height: u32,
        bytes: Vec<u8>,
        sha256: String,
        frame_path: Option<PathBuf>,
    },
    RawXrgb {
        screen_width: u32,
        screen_height: u32,
        pixels: Vec<u8>,
        sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DisplayMetadata {
    pub screen_width: u32,
    pub screen_height: u32,
    pub image_width: u32,
    pub image_height: u32,
    pub image_to_screen_scale_x: f32,
    pub image_to_screen_scale_y: f32,
    #[serde(default)]
    pub client_dpr: Option<f32>,
    #[serde(default)]
    pub monitors: Vec<MonitorRect>,
}

impl DisplayMetadata {
    #[allow(dead_code)]
    pub fn provider_to_image(&self, p: (f32, f32), space: CoordinateSpace) -> (i32, i32) {
        match space {
            CoordinateSpace::ImagePixels => (p.0.round() as i32, p.1.round() as i32),
            CoordinateSpace::ScreenPixels => (
                (p.0 / self.image_to_screen_scale_x).round() as i32,
                (p.1 / self.image_to_screen_scale_y).round() as i32,
            ),
            CoordinateSpace::NormalizedUnit => (
                (p.0 / 1000.0 * self.image_width as f32).round() as i32,
                (p.1 / 1000.0 * self.image_height as f32).round() as i32,
            ),
        }
    }

    pub fn image_to_screen(&self, p: (i32, i32)) -> (i32, i32) {
        (
            (p.0 as f32 * self.image_to_screen_scale_x).round() as i32,
            (p.1 as f32 * self.image_to_screen_scale_y).round() as i32,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum CoordinateSpace {
    ImagePixels,
    ScreenPixels,
    NormalizedUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    MouseMove {
        x: i32,
        y: i32,
        #[serde(default)]
        label: Option<String>,
    },
    MouseClick {
        x: i32,
        y: i32,
        button: MouseButton,
        #[serde(default = "default_click_count")]
        click_count: u8,
        #[serde(default)]
        label: Option<String>,
    },
    MouseDown {
        x: i32,
        y: i32,
        button: MouseButton,
        #[serde(default)]
        label: Option<String>,
    },
    MouseUp {
        x: i32,
        y: i32,
        button: MouseButton,
        #[serde(default)]
        label: Option<String>,
    },
    MouseDrag {
        path: Vec<(i32, i32)>,
        button: MouseButton,
        #[serde(default)]
        label: Option<String>,
    },
    Scroll {
        #[serde(default)]
        x: Option<i32>,
        #[serde(default)]
        y: Option<i32>,
        dx: i32,
        dy: i32,
        #[serde(default)]
        label: Option<String>,
    },
    Zoom {
        level_delta: i32,
        #[serde(default)]
        label: Option<String>,
    },
    TypeText {
        text: String,
        #[serde(default)]
        press_enter: bool,
    },
    KeyChord {
        combo: String,
    },
    KeyHold {
        key: String,
        ms: u32,
    },
    ClipboardWrite {
        text: String,
    },
    ClipboardRead,
    WindowFocus {
        id: u32,
    },
    WindowClose {
        id: u32,
    },
    Wait {
        ms: u32,
    },
    Screenshot,
    Done {
        success: bool,
        #[serde(default)]
        reason: String,
    },
    Ask {
        question: String,
    },
}

fn default_click_count() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct History {
    pub task: String,
    pub steps: Vec<Step>,
    pub budget: Budget,
    #[serde(default)]
    pub observations_seen: u32,
}

impl History {
    pub fn new(task: String, budget: Budget) -> Self {
        Self {
            task,
            steps: Vec::new(),
            budget,
            observations_seen: 0,
        }
    }

    pub fn push(&mut self, step: Step) {
        self.steps.push(step);
    }

    pub fn record_observation(&mut self) {
        self.observations_seen = self.observations_seen.saturating_add(1);
    }

    pub fn over_budget(&self, started_ms: u64) -> bool {
        let tokens_in: u64 = self
            .steps
            .iter()
            .filter_map(|s| s.provider_usage.as_ref().map(|u| u.input_tokens))
            .sum();
        let tokens_out: u64 = self
            .steps
            .iter()
            .filter_map(|s| s.provider_usage.as_ref().map(|u| u.output_tokens))
            .sum();
        let cost_micros: u64 = self
            .steps
            .iter()
            .filter_map(|s| s.provider_usage.as_ref().and_then(|u| u.cost_usd_micros))
            .sum();
        let cost_ceiling = self.budget.max_cost_usd_micros.unwrap_or(u64::MAX);
        self.steps.len() as u32 >= self.budget.max_steps
            || tokens_in > self.budget.max_input_tokens
            || tokens_out > self.budget.max_output_tokens
            || cost_micros > cost_ceiling
            || self.observations_seen > self.budget.max_screenshots
            || now_ms().saturating_sub(started_ms) / 1000 > self.budget.max_wall_seconds
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub observation: ObservationDigest,
    pub action: Action,
    pub result: ActionResult,
    pub elapsed_ms: u64,
    pub provider_usage: Option<ProviderUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionResult {
    Ok,
    OutOfBounds { x: i32, y: i32, w: u32, h: u32 },
    UnsupportedAction { message: String },
    ExecutorError { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Budget {
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default = "default_max_input_tokens")]
    pub max_input_tokens: u64,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u64,
    #[serde(default = "default_max_wall_seconds")]
    pub max_wall_seconds: u64,
    #[serde(default = "default_max_screenshots")]
    pub max_screenshots: u32,
    /// Optional ceiling on aggregate provider cost (micro-USD, i.e. 1e-6 USD).
    /// `None` means unlimited. Only providers that populate
    /// `ProviderUsage.cost_usd_micros` contribute toward this ceiling.
    #[serde(default)]
    pub max_cost_usd_micros: Option<u64>,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_steps: default_max_steps(),
            max_input_tokens: default_max_input_tokens(),
            max_output_tokens: default_max_output_tokens(),
            max_wall_seconds: default_max_wall_seconds(),
            max_screenshots: default_max_screenshots(),
            max_cost_usd_micros: None,
        }
    }
}

fn default_max_steps() -> u32 {
    50
}
fn default_max_input_tokens() -> u64 {
    200_000
}
fn default_max_output_tokens() -> u64 {
    20_000
}
fn default_max_wall_seconds() -> u64 {
    300
}
fn default_max_screenshots() -> u32 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationDigest {
    pub screen_width: u32,
    pub screen_height: u32,
    pub image_width: u32,
    pub image_height: u32,
    pub sha256: String,
    pub frame_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub provider_latency_ms: u64,
    pub cost_usd_micros: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunOptions {
    #[serde(default)]
    pub budget: Budget,
    #[serde(default = "default_action_settle_ms")]
    pub action_settle_ms: u64,
    #[serde(default = "default_max_actions_per_step")]
    pub max_actions_per_step: usize,
    #[serde(default = "default_max_history_images")]
    pub max_history_images: usize,
    #[serde(default)]
    pub allow_destructive: bool,
    #[serde(default)]
    pub require_confirmation_for: Vec<DestructiveKind>,
    #[serde(default)]
    pub screenshot_format: ScreenshotFormat,
    #[serde(default = "default_screenshot_max_bytes")]
    pub screenshot_max_bytes: usize,
    #[serde(default = "default_record_trajectory")]
    pub record_trajectory: bool,
    #[serde(default)]
    pub record_frames_to_disk: bool,
    #[serde(default)]
    pub dry_run: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            budget: Budget::default(),
            action_settle_ms: default_action_settle_ms(),
            max_actions_per_step: default_max_actions_per_step(),
            max_history_images: default_max_history_images(),
            allow_destructive: false,
            require_confirmation_for: Vec::new(),
            screenshot_format: ScreenshotFormat::default(),
            screenshot_max_bytes: default_screenshot_max_bytes(),
            record_trajectory: default_record_trajectory(),
            record_frames_to_disk: false,
            dry_run: false,
        }
    }
}

fn default_action_settle_ms() -> u64 {
    250
}
fn default_max_actions_per_step() -> usize {
    30
}
fn default_max_history_images() -> usize {
    3
}
fn default_screenshot_max_bytes() -> usize {
    800_000
}
fn default_record_trajectory() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub enum ScreenshotFormat {
    #[default]
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum DestructiveKind {
    KeyboardCombo,
    WindowClose,
    ClipboardWriteOverwrite,
    TypeIntoPasswordField,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub task: String,
    pub success: Option<bool>,
    pub finish_reason: FinishReason,
    pub steps_taken: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub wall_ms: u64,
    pub trajectory_path: Option<PathBuf>,
    pub pending_question: Option<String>,
    pub pending_safety_checks: Vec<SafetyCheck>,
    pub last_action: Option<Action>,
    pub last_result: Option<ActionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FinishReason {
    Running,
    Done { success: bool },
    Ask,
    Safety,
    Interrupted,
    BudgetExceeded,
    MaxStepsReached,
    ProviderError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyCheck {
    pub provider: String,
    pub code: String,
    pub message: String,
    pub raw: serde_json::Value,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_normalized_to_image() {
        let display = DisplayMetadata {
            screen_width: 1920,
            screen_height: 1080,
            image_width: 960,
            image_height: 540,
            image_to_screen_scale_x: 2.0,
            image_to_screen_scale_y: 2.0,
            client_dpr: None,
            monitors: vec![],
        };
        assert_eq!(
            display.provider_to_image((500.0, 500.0), CoordinateSpace::NormalizedUnit),
            (480, 270)
        );
        assert_eq!(display.image_to_screen((480, 270)), (960, 540));
    }

    fn step_with_cost(cost: u64) -> Step {
        Step {
            observation: ObservationDigest {
                screen_width: 0,
                screen_height: 0,
                image_width: 0,
                image_height: 0,
                sha256: String::new(),
                frame_path: None,
            },
            action: Action::Wait { ms: 0 },
            result: ActionResult::Ok,
            elapsed_ms: 0,
            provider_usage: Some(ProviderUsage {
                input_tokens: 0,
                output_tokens: 0,
                provider_latency_ms: 0,
                cost_usd_micros: Some(cost),
            }),
        }
    }

    #[test]
    fn over_budget_enforces_cost_ceiling() {
        let mut budget = Budget::default();
        budget.max_cost_usd_micros = Some(1_000);
        let mut history = History::new("t".to_string(), budget);
        history.push(step_with_cost(600));
        assert!(!history.over_budget(now_ms()));
        history.push(step_with_cost(600));
        // sum = 1_200 > 1_000 → over budget
        assert!(history.over_budget(now_ms()));
    }

    #[test]
    fn over_budget_ignores_cost_when_unlimited() {
        let mut budget = Budget::default();
        budget.max_cost_usd_micros = None;
        let mut history = History::new("t".to_string(), budget);
        history.push(step_with_cost(u64::MAX / 2));
        history.push(step_with_cost(u64::MAX / 2));
        assert!(!history.over_budget(now_ms()));
    }
}

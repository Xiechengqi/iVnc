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
    /// Readable page text extracted alongside a full-page capture (innerText).
    /// `None` for ordinary live-viewport frames. Providers inject this into the
    /// per-turn user text block so the model can read content beyond the image.
    #[serde(default)]
    pub page_text: Option<String>,
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
    /// True for full-page CDP capture frames, which are taller than the live
    /// viewport so their coordinates cannot be mapped back to clickable screen
    /// positions. The executor rejects coordinate/input actions on such frames.
    #[serde(default)]
    pub read_only: bool,
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
    /// Capture a READ-ONLY full-page screenshot (and extracted text) of the
    /// active built-in Chrome tab via CDP, including content below the fold.
    /// The returned frame is taller than the viewport, so its coordinates are
    /// NOT clickable — use it only to read/locate content, then return to the
    /// live view with a normal screenshot before scrolling and clicking.
    /// Requires the built-in Chrome to be running (launch_app first).
    CaptureFullPage,
    /// Launch a managed desktop app (e.g. the built-in Chrome) by id or name.
    /// An optional URL is appended to the launch command so the app can open it directly.
    LaunchApp {
        app: String,
        #[serde(default)]
        url: Option<String>,
    },
    /// Run a registered CLI app as a one-shot command. The app must be a
    /// managed CLI app; args are passed directly without a shell.
    CliAppRun {
        app: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Done {
        success: bool,
        #[serde(default)]
        reason: String,
        /// The deliverable produced by the task (the actual answer/result the
        /// user asked for), not just a status sentence.
        #[serde(default)]
        output: String,
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

    pub fn budget_exceeded(&self, started_ms: u64) -> Option<BudgetExceeded> {
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
        let wall_seconds = now_ms().saturating_sub(started_ms) / 1000;
        let kind = if self.steps.len() as u32 >= self.budget.max_steps {
            BudgetExceededKind::Steps
        } else if tokens_in > self.budget.max_input_tokens {
            BudgetExceededKind::InputTokens
        } else if tokens_out > self.budget.max_output_tokens {
            BudgetExceededKind::OutputTokens
        } else if self.observations_seen > self.budget.max_screenshots {
            BudgetExceededKind::Screenshots
        } else if wall_seconds > self.budget.max_wall_seconds {
            BudgetExceededKind::WallTime
        } else {
            return None;
        };
        Some(BudgetExceeded {
            kind,
            steps: self.steps.len() as u32,
            max_steps: self.budget.max_steps,
            input_tokens: tokens_in,
            max_input_tokens: self.budget.max_input_tokens,
            output_tokens: tokens_out,
            max_output_tokens: self.budget.max_output_tokens,
            screenshots: self.observations_seen,
            max_screenshots: self.budget.max_screenshots,
            wall_seconds,
            max_wall_seconds: self.budget.max_wall_seconds,
        })
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
    CommandOutput {
        success: bool,
        exit_code: Option<i32>,
        #[serde(default)]
        env_keys: Vec<String>,
        stdout: String,
        stderr: String,
    },
    OutOfBounds {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    UnsupportedAction {
        message: String,
    },
    ExecutorError {
        message: String,
    },
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
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_steps: default_max_steps(),
            max_input_tokens: default_max_input_tokens(),
            max_output_tokens: default_max_output_tokens(),
            max_wall_seconds: default_max_wall_seconds(),
            max_screenshots: default_max_screenshots(),
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetExceededKind {
    Steps,
    InputTokens,
    OutputTokens,
    Screenshots,
    WallTime,
    OutOfBoundsGuard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BudgetExceeded {
    pub kind: BudgetExceededKind,
    pub steps: u32,
    pub max_steps: u32,
    pub input_tokens: u64,
    pub max_input_tokens: u64,
    pub output_tokens: u64,
    pub max_output_tokens: u64,
    pub screenshots: u32,
    pub max_screenshots: u32,
    pub wall_seconds: u64,
    pub max_wall_seconds: u64,
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
    /// Max CSS height (in px) captured by `capture_full_page`. Content beyond
    /// this is truncated to bound decode memory and provider payload size.
    #[serde(default = "default_fullpage_max_css_height")]
    pub fullpage_max_css_height: f64,
    /// Megapixel ceiling for full-page captures. The loop takes
    /// `min(provider_cap, this)` so the encoded frame never exceeds what the
    /// provider accepts.
    #[serde(default = "default_fullpage_max_megapixels")]
    pub fullpage_max_megapixels: f32,
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
            fullpage_max_css_height: default_fullpage_max_css_height(),
            fullpage_max_megapixels: default_fullpage_max_megapixels(),
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
fn default_fullpage_max_css_height() -> f64 {
    20_000.0
}
fn default_fullpage_max_megapixels() -> f32 {
    4.0
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunSource {
    /// Started manually (MCP tool or `POST /api/console/agent-start`).
    Manual,
    /// Fired by the scheduler from the named schedule entry.
    Schedule {
        id: String,
        #[serde(default)]
        name: Option<String>,
    },
}

impl Default for RunSource {
    fn default() -> Self {
        RunSource::Manual
    }
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
    /// Unix epoch millis when the run started. 0 if unknown.
    #[serde(default)]
    pub started_at_ms: u64,
    pub trajectory_path: Option<PathBuf>,
    /// JSONL event stream for this run. Newer than trajectory_path and more
    /// useful for provider/tool/session diagnostics.
    #[serde(default)]
    pub event_path: Option<PathBuf>,
    #[serde(default)]
    pub budget_exceeded: Option<BudgetExceeded>,
    pub pending_question: Option<String>,
    pub pending_safety_checks: Vec<SafetyCheck>,
    pub last_action: Option<Action>,
    pub last_result: Option<ActionResult>,
    /// The deliverable the agent produced via `done.output`, if any.
    #[serde(default)]
    pub output: Option<String>,
    /// Non-fatal warnings surfaced by the run (e.g. a success claim that was
    /// not backed by any substantive on-screen work).
    #[serde(default)]
    pub warnings: Vec<String>,
    /// How this run was launched. Older sidecars without this field default to
    /// `RunSource::Manual`.
    #[serde(default)]
    pub source: RunSource,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRecord {
    pub tool_call_id: String,
    pub ok: bool,
    pub text: String,
    pub data: serde_json::Value,
}

impl ToolCallRecord {
    pub fn from_action(step: usize, action: &Action) -> Self {
        let (name, arguments) = action_tool_parts(action);
        Self {
            id: format!("step_{}", step),
            name,
            arguments,
        }
    }
}

impl ToolResultRecord {
    pub fn from_action_result(tool_call_id: String, result: &ActionResult) -> Self {
        let ok = matches!(result, ActionResult::Ok)
            || matches!(result, ActionResult::CommandOutput { success: true, .. });
        let text = match result {
            ActionResult::Ok => "ok".to_string(),
            ActionResult::CommandOutput {
                success,
                exit_code,
                stdout,
                stderr,
                ..
            } => {
                let mut text = format!("command success={} exit_code={:?}", success, exit_code);
                if !stdout.trim().is_empty() {
                    text.push_str("\nstdout:\n");
                    text.push_str(stdout.trim());
                }
                if !stderr.trim().is_empty() {
                    text.push_str("\nstderr:\n");
                    text.push_str(stderr.trim());
                }
                text
            }
            ActionResult::OutOfBounds { x, y, w, h } => {
                format!("out of bounds: ({}, {}) outside {}x{}", x, y, w, h)
            }
            ActionResult::UnsupportedAction { message }
            | ActionResult::ExecutorError { message } => message.clone(),
        };
        Self {
            tool_call_id,
            ok,
            text,
            data: serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
        }
    }
}

fn action_tool_parts(action: &Action) -> (String, serde_json::Value) {
    match action {
        Action::MouseMove { x, y, label } => (
            "mouse_move".to_string(),
            serde_json::json!({ "x": x, "y": y, "label": label }),
        ),
        Action::MouseClick {
            x,
            y,
            button,
            click_count,
            label,
        } => (
            "mouse_click".to_string(),
            serde_json::json!({ "x": x, "y": y, "button": button, "click_count": click_count, "label": label }),
        ),
        Action::MouseDown {
            x,
            y,
            button,
            label,
        } => (
            "mouse_down".to_string(),
            serde_json::json!({ "x": x, "y": y, "button": button, "label": label }),
        ),
        Action::MouseUp {
            x,
            y,
            button,
            label,
        } => (
            "mouse_up".to_string(),
            serde_json::json!({ "x": x, "y": y, "button": button, "label": label }),
        ),
        Action::MouseDrag {
            path,
            button,
            label,
        } => (
            "mouse_drag".to_string(),
            serde_json::json!({ "path": path, "button": button, "label": label }),
        ),
        Action::Scroll {
            x,
            y,
            dx,
            dy,
            label,
        } => (
            "scroll".to_string(),
            serde_json::json!({ "x": x, "y": y, "dx": dx, "dy": dy, "label": label }),
        ),
        Action::Zoom { level_delta, label } => (
            "zoom".to_string(),
            serde_json::json!({ "level_delta": level_delta, "label": label }),
        ),
        Action::TypeText { text, press_enter } => (
            "type_text".to_string(),
            serde_json::json!({ "text": text, "press_enter": press_enter }),
        ),
        Action::KeyChord { combo } => (
            "key_chord".to_string(),
            serde_json::json!({ "combo": combo }),
        ),
        Action::KeyHold { key, ms } => (
            "key_hold".to_string(),
            serde_json::json!({ "key": key, "ms": ms }),
        ),
        Action::ClipboardWrite { text } => (
            "clipboard_write".to_string(),
            serde_json::json!({ "text": text }),
        ),
        Action::ClipboardRead => ("clipboard_read".to_string(), serde_json::json!({})),
        Action::WindowFocus { id } => ("window_focus".to_string(), serde_json::json!({ "id": id })),
        Action::WindowClose { id } => ("window_close".to_string(), serde_json::json!({ "id": id })),
        Action::Wait { ms } => ("wait".to_string(), serde_json::json!({ "ms": ms })),
        Action::Screenshot => ("screenshot".to_string(), serde_json::json!({})),
        Action::CaptureFullPage => ("capture_full_page".to_string(), serde_json::json!({})),
        Action::LaunchApp { app, url } => (
            "launch_app".to_string(),
            serde_json::json!({ "app": app, "url": url }),
        ),
        Action::CliAppRun {
            app,
            args,
            timeout_ms,
        } => (
            "cli_app_run".to_string(),
            serde_json::json!({ "app": app, "args": args, "timeout_ms": timeout_ms }),
        ),
        Action::Done {
            success,
            reason,
            output,
        } => (
            "done".to_string(),
            serde_json::json!({ "success": success, "reason": reason, "output": output }),
        ),
        Action::Ask { question } => (
            "ask".to_string(),
            serde_json::json!({ "question": question }),
        ),
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod event_contract_tests {
    use super::*;

    #[test]
    fn budget_exceeded_reports_specific_kind() {
        let mut history = History::new(
            "task".to_string(),
            Budget {
                max_steps: 1,
                ..Budget::default()
            },
        );
        history.push(Step {
            observation: ObservationDigest {
                screen_width: 1,
                screen_height: 1,
                image_width: 1,
                image_height: 1,
                sha256: "x".to_string(),
                frame_path: None,
            },
            action: Action::Wait { ms: 1 },
            result: ActionResult::Ok,
            elapsed_ms: 0,
            provider_usage: None,
        });
        let exceeded = history.budget_exceeded(now_ms()).unwrap();
        assert_eq!(exceeded.kind, BudgetExceededKind::Steps);
        assert_eq!(exceeded.steps, 1);
    }

    #[test]
    fn action_and_result_have_tool_records() {
        let action = Action::CliAppRun {
            app: "agent-browser".to_string(),
            args: vec!["snapshot".to_string(), "--json".to_string()],
            timeout_ms: Some(1000),
        };
        let call = ToolCallRecord::from_action(7, &action);
        assert_eq!(call.id, "step_7");
        assert_eq!(call.name, "cli_app_run");
        assert_eq!(call.arguments["app"], "agent-browser");

        let result = ToolResultRecord::from_action_result(
            call.id,
            &ActionResult::CommandOutput {
                success: true,
                exit_code: Some(0),
                env_keys: vec!["NO_COLOR".to_string()],
                stdout: "{}".to_string(),
                stderr: String::new(),
            },
        );
        assert!(result.ok);
        assert!(result.text.contains("stdout"));
    }
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
            read_only: false,
        };
        assert_eq!(
            display.provider_to_image((500.0, 500.0), CoordinateSpace::NormalizedUnit),
            (480, 270)
        );
        assert_eq!(display.image_to_screen((480, 270)), (960, 540));
    }
}

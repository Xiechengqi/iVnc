use super::safety::destructive_kind;
use super::types::{Action, ActionResult, DisplayMetadata, MouseButton, RunOptions};
use crate::mcp::input_exec;
use crate::web::SharedState;
use std::sync::Arc;
use std::time::Duration;

const AUTOMATION_WAKEUP: Duration = Duration::from_secs(5);

pub async fn execute(
    state: &Arc<SharedState>,
    display: &DisplayMetadata,
    action: &Action,
    options: &RunOptions,
) -> ActionResult {
    if state.agent_stop_requested() {
        return ActionResult::ExecutorError {
            message: "interrupted_by_user".to_string(),
        };
    }
    if let Some(kind) = destructive_kind(action) {
        if !options.allow_destructive || options.require_confirmation_for.contains(&kind) {
            let tag = match kind {
                super::types::DestructiveKind::KeyboardCombo => "keyboard_combo",
                super::types::DestructiveKind::WindowClose => "window_close",
                super::types::DestructiveKind::ClipboardWriteOverwrite => {
                    "clipboard_write_overwrite"
                }
                super::types::DestructiveKind::TypeIntoPasswordField => "type_into_password_field",
            };
            return ActionResult::ExecutorError {
                message: format!("destructive_blocked:{}", tag),
            };
        }
    }
    if display.read_only && action_requires_live_viewport(action) {
        return ActionResult::UnsupportedAction {
            message: "read_only_frame: this is a full-page capture; its coordinates are not \
                clickable. Call screenshot to return to the live viewport, then scroll and \
                click on that frame."
                .to_string(),
        };
    }
    match execute_inner(state, display, action).await {
        Ok(()) => ActionResult::Ok,
        Err(result) => result,
    }
}

async fn execute_inner(
    state: &Arc<SharedState>,
    display: &DisplayMetadata,
    action: &Action,
) -> Result<(), ActionResult> {
    match action {
        Action::MouseMove { x, y, .. } => {
            let (sx, sy) = to_screen(display, *x, *y)?;
            input_exec::mouse_move(state, sx, sy).await;
        }
        Action::MouseClick {
            x,
            y,
            button,
            click_count,
            ..
        } => {
            let (sx, sy) = to_screen(display, *x, *y)?;
            input_exec::mouse_click(state, sx, sy, button_name(*button), *click_count)
                .await
                .map_err(|e| ActionResult::ExecutorError {
                    message: e.to_string(),
                })?;
        }
        Action::MouseDown { x, y, button, .. } => {
            state.request_automation_wakeup(AUTOMATION_WAKEUP);
            let (sx, sy) = to_screen(display, *x, *y)?;
            input_exec::mouse_move(state, sx, sy).await;
            let button = input_exec::mouse_button_id(button_name(*button))
                .map_err(|e| ActionResult::ExecutorError { message: e.to_string() })?;
            let _ = state.input_sender.send(crate::input::InputEventData {
                event_type: crate::input::InputEvent::MouseButton,
                mouse_x: sx,
                mouse_y: sy,
                mouse_button: button,
                button_pressed: true,
                ..Default::default()
            });
        }
        Action::MouseUp { x, y, button, .. } => {
            state.request_automation_wakeup(AUTOMATION_WAKEUP);
            let (sx, sy) = to_screen(display, *x, *y)?;
            input_exec::mouse_move(state, sx, sy).await;
            let button = input_exec::mouse_button_id(button_name(*button))
                .map_err(|e| ActionResult::ExecutorError { message: e.to_string() })?;
            let _ = state.input_sender.send(crate::input::InputEventData {
                event_type: crate::input::InputEvent::MouseButton,
                mouse_x: sx,
                mouse_y: sy,
                mouse_button: button,
                button_pressed: false,
                ..Default::default()
            });
        }
        Action::MouseDrag { path, button, .. } => {
            state.request_automation_wakeup(AUTOMATION_WAKEUP);
            let Some(first) = path.first().copied() else {
                return Err(ActionResult::ExecutorError { message: "empty_drag_path".to_string() });
            };
            let Some(last) = path.last().copied() else {
                return Err(ActionResult::ExecutorError { message: "empty_drag_path".to_string() });
            };
            let (first_x, first_y) = to_screen(display, first.0, first.1)?;
            input_exec::mouse_move(state, first_x, first_y).await;
            let button_id = input_exec::mouse_button_id(button_name(*button))
                .map_err(|e| ActionResult::ExecutorError { message: e.to_string() })?;
            let _ = state.input_sender.send(crate::input::InputEventData {
                event_type: crate::input::InputEvent::MouseButton,
                mouse_x: first_x,
                mouse_y: first_y,
                mouse_button: button_id,
                button_pressed: true,
                ..Default::default()
            });
            for &(x, y) in path.iter().skip(1) {
                let (sx, sy) = to_screen(display, x, y)?;
                input_exec::mouse_move(state, sx, sy).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let (last_x, last_y) = to_screen(display, last.0, last.1)?;
            input_exec::mouse_move(state, last_x, last_y).await;
            let _ = state.input_sender.send(crate::input::InputEventData {
                event_type: crate::input::InputEvent::MouseButton,
                mouse_x: last_x,
                mouse_y: last_y,
                mouse_button: button_id,
                button_pressed: false,
                ..Default::default()
            });
        }
        Action::Scroll { x, y, dx, dy, .. } => {
            if let (Some(x), Some(y)) = (*x, *y) {
                let (sx, sy) = to_screen(display, x, y)?;
                input_exec::mouse_move(state, sx, sy).await;
            }
            input_exec::mouse_scroll(state, clamp_i16(*dx), clamp_i16(*dy));
        }
        Action::TypeText { text, press_enter } => {
            input_exec::type_text(state, text, *press_enter).await;
        }
        Action::KeyChord { combo } => {
            input_exec::key_chord(state, combo)
                .await
                .map_err(|e| ActionResult::ExecutorError { message: e.to_string() })?;
        }
        Action::KeyHold { key, ms } => {
            let (mods, sym) = crate::mcp::keyboard::parse_key_combo(key)
                .map_err(|message| ActionResult::ExecutorError { message })?;
            if !mods.is_empty() {
                return Err(ActionResult::UnsupportedAction {
                    message: "KeyHold does not support modifier combos".to_string(),
                });
            }
            input_exec::send_key(state, sym, true);
            tokio::time::sleep(Duration::from_millis(*ms as u64)).await;
            input_exec::send_key(state, sym, false);
        }
        Action::ClipboardWrite { text } => {
            let b64 =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, text.as_bytes());
            state.set_clipboard(b64.clone());
            let _ = state.clipboard_incoming_tx.send(b64);
        }
        Action::WindowFocus { id } => input_exec::window_focus(state, *id),
        Action::WindowClose { id } => input_exec::window_close(state, *id),
        Action::Wait { ms } => tokio::time::sleep(Duration::from_millis(*ms as u64)).await,
        Action::Screenshot | Action::ClipboardRead => {}
        Action::Zoom { .. } => {
            return Err(ActionResult::UnsupportedAction { message: "zoom".to_string() })
        }
        Action::LaunchApp { app, url } => {
            let Some(apps) = state.apps_state() else {
                return Err(ActionResult::ExecutorError {
                    message: "launch_app_unavailable: apps manager not initialized".to_string(),
                });
            };
            let windows_before = mapped_window_count(state);
            match apps.launch_named(app, url.as_deref()).await {
                Ok(crate::apps::api::LaunchOutcome::Started { .. }) => {
                    // A freshly launched app needs seconds to connect to Wayland
                    // and map its first surface. Without waiting, the agent's next
                    // observation is captured before any window exists (blank
                    // screen), so it wastes steps re-launching. Block until a new
                    // window maps or we time out.
                    wait_for_new_window(state, windows_before, Duration::from_secs(10)).await;
                }
                Ok(crate::apps::api::LaunchOutcome::OpenedInExisting) => {}
                Ok(crate::apps::api::LaunchOutcome::AlreadyRunning { .. }) => {
                    if let Some(u) = url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
                        return Err(ActionResult::ExecutorError {
                            message: format!(
                                "launch_app_no_op: {} is already running; URL {:?} was NOT delivered. \
                                 Use the existing window — type the URL into the address bar or click a link.",
                                app, u
                            ),
                        });
                    }
                }
                Err(message) => {
                    return Err(ActionResult::ExecutorError {
                        message: format!("launch_app_failed: {}", message),
                    })
                }
            }
        }
        Action::Done { .. } | Action::Ask { .. } => {}
        Action::CaptureFullPage => {
            let Some(apps) = state.apps_state() else {
                return Err(ActionResult::ExecutorError {
                    message: "capture_full_page_unavailable: apps manager not initialized"
                        .to_string(),
                });
            };
            if apps.process.pid("builtin-chrome").is_none() {
                return Err(ActionResult::ExecutorError {
                    message: "capture_full_page_unavailable: built-in Chrome is not running; \
                        call launch_app(app='chrome', url='<page>') first"
                        .to_string(),
                });
            }
            // Actual full-page capture runs at the top of the next loop iteration.
        }
    }
    Ok(())
}

/// Actions whose coordinates or input must target the live viewport. Returning
/// true means the action cannot run on a read-only full-page capture frame.
fn action_requires_live_viewport(action: &Action) -> bool {
    matches!(
        action,
        Action::MouseMove { .. }
            | Action::MouseClick { .. }
            | Action::MouseDown { .. }
            | Action::MouseUp { .. }
            | Action::MouseDrag { .. }
            | Action::Scroll { .. }
            | Action::Zoom { .. }
            | Action::TypeText { .. }
            | Action::KeyChord { .. }
            | Action::KeyHold { .. }
            | Action::WindowFocus { .. }
            | Action::WindowClose { .. }
    )
}

/// Count toplevel windows currently mapped in the compositor, read from the
/// cached taskbar JSON the compositor loop maintains. Returns 0 when unavailable
/// (e.g. in unit tests with no compositor running).
fn mapped_window_count(state: &Arc<SharedState>) -> usize {
    state
        .last_taskbar_json
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .and_then(|v| {
            v.get("windows")
                .and_then(|w| w.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0)
}

/// Block until the mapped window count exceeds `baseline` (a newly launched app
/// has mapped its surface) or `timeout` elapses. On success, waits a short extra
/// beat so the client has a chance to paint its first real frame before the
/// agent captures the next observation.
async fn wait_for_new_window(state: &Arc<SharedState>, baseline: usize, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if state.agent_stop_requested() {
            return;
        }
        if mapped_window_count(state) > baseline {
            tokio::time::sleep(Duration::from_millis(400)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn to_screen(display: &DisplayMetadata, x: i32, y: i32) -> Result<(i32, i32), ActionResult> {
    let (sx, sy) = display.image_to_screen((x, y));
    if sx < 0 || sy < 0 || sx >= display.screen_width as i32 || sy >= display.screen_height as i32 {
        return Err(ActionResult::OutOfBounds {
            x: sx,
            y: sy,
            w: display.screen_width,
            h: display.screen_height,
        });
    }
    Ok((sx, sy))
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Middle => "middle",
        MouseButton::Right => "right",
    }
}

fn clamp_i16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ui::UiConfig, Config};
    use crate::input::InputEventData;
    use crate::runtime_settings::RuntimeSettings;

    fn test_state() -> Arc<SharedState> {
        let config = Config::default();
        let ui_config = UiConfig::from_env(&config);
        let runtime_settings = Arc::new(RuntimeSettings::new(&config));
        let (input_sender, _input_rx) = tokio::sync::mpsc::unbounded_channel::<InputEventData>();
        Arc::new(SharedState::new(
            config,
            ui_config,
            input_sender,
            runtime_settings,
        ))
    }

    fn test_display() -> DisplayMetadata {
        DisplayMetadata {
            screen_width: 1920,
            screen_height: 1080,
            image_width: 1920,
            image_height: 1080,
            image_to_screen_scale_x: 1.0,
            image_to_screen_scale_y: 1.0,
            client_dpr: None,
            monitors: Vec::new(),
            read_only: false,
        }
    }

    #[tokio::test]
    async fn clipboard_write_updates_read_cache() {
        let state = test_state();
        let text = "agent clipboard roundtrip";

        execute_inner(
            &state,
            &test_display(),
            &Action::ClipboardWrite {
                text: text.to_string(),
            },
        )
        .await
        .unwrap();

        let encoded = state.clipboard.lock().unwrap().clone().unwrap();
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), text);
    }

    #[test]
    fn action_result_error_variants_serialize() {
        let cases = [
            ActionResult::Ok,
            ActionResult::OutOfBounds { x: -1, y: 2, w: 1920, h: 1080 },
            ActionResult::UnsupportedAction { message: "zoom".into() },
            ActionResult::ExecutorError { message: "destructive_blocked".into() },
        ];
        for r in cases {
            let v = serde_json::to_value(&r).expect("ActionResult must serialize");
            assert!(v.get("kind").is_some(), "missing kind tag: {v}");
        }
    }

    #[test]
    fn action_requires_live_viewport_classification() {
        // Mutating actions need the live viewport.
        assert!(action_requires_live_viewport(&Action::MouseClick {
            x: 0, y: 0, button: MouseButton::Left, click_count: 1, label: None,
        }));
        assert!(action_requires_live_viewport(&Action::Scroll {
            x: None, y: None, dx: 0, dy: -3, label: None,
        }));
        assert!(action_requires_live_viewport(&Action::TypeText {
            text: "hi".into(), press_enter: false,
        }));
        // Observation-only / control actions must remain allowed on a read-only frame
        // so the model can escape it.
        assert!(!action_requires_live_viewport(&Action::Screenshot));
        assert!(!action_requires_live_viewport(&Action::CaptureFullPage));
        assert!(!action_requires_live_viewport(&Action::Wait { ms: 100 }));
        assert!(!action_requires_live_viewport(&Action::Done {
            success: true, reason: String::new(), output: String::new(),
        }));
    }

    #[tokio::test]
    async fn read_only_frame_rejects_clicks() {
        let state = test_state();
        let mut display = test_display();
        display.read_only = true;
        let result = execute(
            &state,
            &display,
            &Action::MouseClick {
                x: 100, y: 100, button: MouseButton::Left, click_count: 1, label: None,
            },
            &super::super::types::RunOptions::default(),
        )
        .await;
        match result {
            ActionResult::UnsupportedAction { message } => {
                assert!(message.contains("read_only_frame"), "unexpected: {}", message);
            }
            other => panic!("expected UnsupportedAction, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn read_only_frame_allows_screenshot_to_escape() {
        let state = test_state();
        let mut display = test_display();
        display.read_only = true;
        let result = execute(
            &state,
            &display,
            &Action::Screenshot,
            &super::super::types::RunOptions::default(),
        )
        .await;
        assert!(matches!(result, ActionResult::Ok), "got {:?}", result);
    }
}

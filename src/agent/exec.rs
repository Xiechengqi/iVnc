use super::safety::destructive_kind;
use super::types::{Action, ActionResult, DisplayMetadata, MouseButton, RunOptions};
use crate::mcp::input_exec;
use crate::web::SharedState;
use std::sync::Arc;
use std::time::Duration;

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
            return ActionResult::ExecutorError {
                message: "destructive_blocked".to_string(),
            };
        }
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
            let _ = state.clipboard_incoming_tx.send(b64);
        }
        Action::WindowFocus { id } => input_exec::window_focus(state, *id),
        Action::WindowClose { id } => input_exec::window_close(state, *id),
        Action::Wait { ms } => tokio::time::sleep(Duration::from_millis(*ms as u64)).await,
        Action::Screenshot | Action::ClipboardRead => {}
        Action::Zoom { .. } => {
            return Err(ActionResult::UnsupportedAction { message: "zoom".to_string() })
        }
        Action::Done { .. } | Action::Ask { .. } => {}
    }
    Ok(())
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
}

use crate::input::{InputEvent, InputEventData};
use crate::mcp::keyboard;
use crate::web::SharedState;
use rmcp::ErrorData as McpError;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub enum McpActionKind {
    Mouse,
    Keyboard,
    ClipboardWrite,
    WindowFocus,
    WindowClose,
}

pub fn guard_mcp_action_allowed(
    state: &Arc<SharedState>,
    kind: McpActionKind,
) -> Result<(), McpError> {
    #[cfg(feature = "agent")]
    {
        if state.agent_exclusive() {
            return Err(McpError::invalid_request(
                format!("agent_control_active: refusing {:?}", kind),
                None,
            ));
        }
        if state.agent_stop_requested() {
            return Err(McpError::invalid_request(
                format!("interrupted_by_user: refusing {:?}", kind),
                None,
            ));
        }
    }
    #[cfg(not(feature = "agent"))]
    {
        let _ = (state, kind);
    }
    Ok(())
}

pub fn validate_coords(state: &Arc<SharedState>, x: i32, y: i32) -> Result<(), McpError> {
    let (w, h) = state.display_size();
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return Err(McpError::invalid_params(
            format!("coordinates ({}, {}) out of bounds ({}x{})", x, y, w, h),
            None,
        ));
    }
    Ok(())
}

pub fn mouse_button_id(button: &str) -> Result<u8, McpError> {
    match button {
        "left" => Ok(0),
        "middle" => Ok(1),
        "right" => Ok(2),
        other => Err(McpError::invalid_params(
            format!("unknown button: {}", other),
            None,
        )),
    }
}

pub fn send_key(state: &Arc<SharedState>, keysym: u32, pressed: bool) {
    let _ = state.input_sender.send(InputEventData {
        event_type: InputEvent::Keyboard,
        keysym,
        key_pressed: pressed,
        ..Default::default()
    });
}

pub async fn type_char(state: &Arc<SharedState>, c: char) {
    let needs_shift = keyboard::char_needs_shift(c);
    let base = if needs_shift {
        keyboard::get_unshifted_char(c)
    } else {
        c
    };
    let sym = keyboard::char_to_keysym(base);
    if needs_shift {
        send_key(state, 0xffe1, true);
    }
    send_key(state, sym, true);
    tokio::time::sleep(Duration::from_millis(50)).await;
    send_key(state, sym, false);
    if needs_shift {
        send_key(state, 0xffe1, false);
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
}

pub fn send_text_input(state: &Arc<SharedState>, text: &str) {
    let _ = state.input_sender.send(InputEventData {
        event_type: InputEvent::TextInput,
        text: text.to_string(),
        ..Default::default()
    });
}

pub fn text_is_ascii_typeable(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii() && !c.is_ascii_control())
}

pub async fn type_text(state: &Arc<SharedState>, text: &str, enter: bool) {
    if text_is_ascii_typeable(text) {
        for c in text.chars() {
            type_char(state, c).await;
        }
    } else {
        send_text_input(state, text);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if enter {
        send_key(state, 0xff0d, true);
        tokio::time::sleep(Duration::from_millis(50)).await;
        send_key(state, 0xff0d, false);
    }
}

pub async fn key_chord(state: &Arc<SharedState>, combo: &str) -> Result<(), McpError> {
    let (modifiers, main_sym) =
        keyboard::parse_key_combo(combo).map_err(|e| McpError::invalid_params(e, None))?;
    for &m in &modifiers {
        send_key(state, m, true);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    send_key(state, main_sym, true);
    tokio::time::sleep(Duration::from_millis(50)).await;
    send_key(state, main_sym, false);
    for &m in modifiers.iter().rev() {
        tokio::time::sleep(Duration::from_millis(10)).await;
        send_key(state, m, false);
    }
    Ok(())
}

pub async fn mouse_move(state: &Arc<SharedState>, x: i32, y: i32) {
    let _ = state.input_sender.send(InputEventData {
        event_type: InputEvent::MouseMove,
        mouse_x: x,
        mouse_y: y,
        ..Default::default()
    });
}

pub async fn mouse_click(
    state: &Arc<SharedState>,
    x: i32,
    y: i32,
    button: &str,
    click_count: u8,
) -> Result<(), McpError> {
    let button = mouse_button_id(button)?;
    mouse_move(state, x, y).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    for i in 0..click_count.max(1) {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = state.input_sender.send(InputEventData {
            event_type: InputEvent::MouseButton,
            mouse_x: x,
            mouse_y: y,
            mouse_button: button,
            button_pressed: true,
            ..Default::default()
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = state.input_sender.send(InputEventData {
            event_type: InputEvent::MouseButton,
            mouse_x: x,
            mouse_y: y,
            mouse_button: button,
            button_pressed: false,
            ..Default::default()
        });
    }
    Ok(())
}

pub fn mouse_scroll(state: &Arc<SharedState>, dx: i16, dy: i16) {
    let _ = state.input_sender.send(InputEventData {
        event_type: InputEvent::MouseWheel,
        wheel_delta_x: dx,
        wheel_delta_y: dy,
        ..Default::default()
    });
}

pub fn window_focus(state: &Arc<SharedState>, window_id: u32) {
    let _ = state.input_sender.send(InputEventData {
        event_type: InputEvent::WindowFocus,
        window_id,
        ..Default::default()
    });
}

pub fn window_close(state: &Arc<SharedState>, window_id: u32) {
    let _ = state.input_sender.send(InputEventData {
        event_type: InputEvent::WindowClose,
        window_id,
        ..Default::default()
    });
}

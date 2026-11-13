//! WebRTC DataChannel input parsing
//!
//! Provides input event parsing for DataChannel text messages,
//! compatible with the existing WebSocket input protocol.
//! The actual DataChannel I/O is handled by str0m in rtc_session.rs.

#![allow(dead_code)]

use super::WebRTCError;
use crate::input::{InputEvent, InputEventData};

/// Input message parser for DataChannel text protocol.
///
/// This is a stateless parser — the actual DataChannel lifecycle
/// is managed by str0m's event-based API in `rtc_session.rs`.
pub struct InputDataChannel;

impl InputDataChannel {
    /// v1 pointer messages use desktop coordinates and carry a geometry revision.
    /// Legacy clients use encoded pixels; convert only at this boundary.
    pub fn parse_display_input(
        text: &str,
        state: &crate::display::DisplayState,
    ) -> Result<InputEventData, WebRTCError> {
        let (text, modern) = if let Some(wrapped) = text.strip_prefix("i,") {
            let (revision, text) = wrapped
                .split_once(',')
                .ok_or_else(|| WebRTCError::DataChannelError("Invalid geometry input".into()))?;
            if revision.parse::<u64>().ok() != Some(state.revision) {
                return Err(WebRTCError::DataChannelError("Stale geometry input".into()));
            }
            (text, true)
        } else {
            (text, false)
        };
        let mut event = Self::parse_input_text(text)?;
        let pointer = matches!(
            event.event_type,
            InputEvent::MouseMove | InputEvent::MouseButton | InputEvent::MouseWheel
        );
        if modern && !pointer {
            return Err(WebRTCError::DataChannelError(
                "Geometry wrapper requires pointer input".into(),
            ));
        }
        if pointer {
            event.geometry_revision = Some(state.revision);
        }
        if !modern && event.event_type == InputEvent::MouseMove {
            event.mouse_x = (f64::from(event.mouse_x) * f64::from(state.desktop.width)
                / f64::from(state.encoded.width.max(1)))
            .round() as i32;
            event.mouse_y = (f64::from(event.mouse_y) * f64::from(state.desktop.height)
                / f64::from(state.encoded.height.max(1)))
            .round() as i32;
        }
        Ok(event)
    }

    /// Parse input text message
    ///
    /// Supports the same protocol as WebSocket:
    /// - Mouse move: `m,x,y` or `m,x,y,buttons`
    /// - Relative mouse: `m2,dx,dy,buttons,0`
    /// - Mouse button: `b,button,pressed`
    /// - Mouse wheel: `w,dx,dy`
    /// - Keyboard: `k,keysym,pressed`
    /// - Key down: `kd,keysym`
    /// - Key up: `ku,keysym`
    /// - Text input: `t,<utf8_text>`
    /// - Clipboard: `c,<base64_text>`
    /// - Ping: `p,timestamp`
    pub fn parse_input_text(text: &str) -> Result<InputEventData, WebRTCError> {
        let parts: Vec<&str> = text.split(',').collect();

        if parts.is_empty() {
            return Err(WebRTCError::DataChannelError(
                "Empty input message".to_string(),
            ));
        }

        let mut event = InputEventData::default();

        match parts[0] {
            "m" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid mouse move format".to_string(),
                    ));
                }
                event.event_type = InputEvent::MouseMove;
                event.mouse_x = parts[1]
                    .parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse X".to_string()))?;
                event.mouse_y = parts[2]
                    .parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse Y".to_string()))?;
                if parts.len() > 3 {
                    event.button_mask = parts[3].parse().unwrap_or(0);
                }
            }

            "m2" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid relative mouse move format".to_string(),
                    ));
                }
                event.event_type = InputEvent::MouseMove;
                event.mouse_x = parts[1]
                    .parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse dX".to_string()))?;
                event.mouse_y = parts[2]
                    .parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse dY".to_string()))?;
                event.text = "relative".to_string();
                if parts.len() > 3 {
                    event.button_mask = parts[3].parse().unwrap_or(0);
                }
            }

            "b" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid mouse button format".to_string(),
                    ));
                }
                event.event_type = InputEvent::MouseButton;
                event.mouse_button = parts[1].parse().map_err(|_| {
                    WebRTCError::DataChannelError("Invalid button number".to_string())
                })?;
                event.button_pressed = parts[2] == "1";
            }

            "w" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid mouse wheel format".to_string(),
                    ));
                }
                event.event_type = InputEvent::MouseWheel;
                event.wheel_delta_x = parts[1].parse().map_err(|_| {
                    WebRTCError::DataChannelError("Invalid wheel delta X".to_string())
                })?;
                event.wheel_delta_y = parts[2].parse().map_err(|_| {
                    WebRTCError::DataChannelError("Invalid wheel delta Y".to_string())
                })?;
            }

            "k" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid keyboard format".to_string(),
                    ));
                }
                event.event_type = InputEvent::Keyboard;
                let keysym_str = parts[1];
                event.keysym = if keysym_str.starts_with("0x") || keysym_str.starts_with("0X") {
                    u32::from_str_radix(&keysym_str[2..], 16).map_err(|_| {
                        WebRTCError::DataChannelError("Invalid hex keysym".to_string())
                    })?
                } else {
                    keysym_str
                        .parse()
                        .map_err(|_| WebRTCError::DataChannelError("Invalid keysym".to_string()))?
                };
                event.key_pressed = parts[2] == "1";
            }

            "kd" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid kd format".to_string(),
                    ));
                }
                event.event_type = InputEvent::Keyboard;
                let keysym_str = parts[1];
                event.keysym = if keysym_str.starts_with("0x") || keysym_str.starts_with("0X") {
                    u32::from_str_radix(&keysym_str[2..], 16).map_err(|_| {
                        WebRTCError::DataChannelError("Invalid hex keysym".to_string())
                    })?
                } else {
                    keysym_str
                        .parse()
                        .map_err(|_| WebRTCError::DataChannelError("Invalid keysym".to_string()))?
                };
                event.key_pressed = true;
            }

            "ku" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid ku format".to_string(),
                    ));
                }
                event.event_type = InputEvent::Keyboard;
                let keysym_str = parts[1];
                event.keysym = if keysym_str.starts_with("0x") || keysym_str.starts_with("0X") {
                    u32::from_str_radix(&keysym_str[2..], 16).map_err(|_| {
                        WebRTCError::DataChannelError("Invalid hex keysym".to_string())
                    })?
                } else {
                    keysym_str
                        .parse()
                        .map_err(|_| WebRTCError::DataChannelError("Invalid keysym".to_string()))?
                };
                event.key_pressed = false;
            }

            "t" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid text input format".to_string(),
                    ));
                }
                event.event_type = InputEvent::TextInput;
                event.text = parts[1..].join(",");
            }

            "c" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError(
                        "Invalid clipboard format".to_string(),
                    ));
                }
                event.event_type = InputEvent::Clipboard;
                event.text = parts[1..].join(",");
            }

            "p" => {
                event.event_type = InputEvent::Ping;
                if parts.len() > 1 {
                    event.timestamp = parts[1].parse().unwrap_or(0);
                }
            }

            _ => {
                return Err(WebRTCError::DataChannelError(format!(
                    "Unknown input type: {}",
                    parts[0]
                )));
            }
        }

        Ok(event)
    }
}

/// Format an outgoing message for the DataChannel
pub fn format_output_message(msg_type: &str, data: &str) -> String {
    format!("{},{}", msg_type, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_input_separates_desktop_pixels_from_legacy_encoded_pixels() {
        let controller =
            crate::display::DisplayController::new(crate::display::Size::new(1920, 1080), false);
        controller.set_encoded(crate::display::Size::new(1280, 720));
        let s = controller.snapshot();
        let old = InputDataChannel::parse_display_input("m,640,360,1", &s).unwrap();
        let new = InputDataChannel::parse_display_input("i,1,m,960,540,1", &s).unwrap();
        assert_eq!((old.mouse_x, old.mouse_y), (new.mouse_x, new.mouse_y));
        assert_eq!((new.mouse_x, new.mouse_y), (960, 540));
        assert_eq!(new.geometry_revision, Some(1));
        assert!(InputDataChannel::parse_display_input("i,0,m,960,540,1", &s).is_err());
        assert!(InputDataChannel::parse_display_input("i,1,k,65,1", &s).is_err());
        let relative = InputDataChannel::parse_display_input("m2,20,-10,0", &s).unwrap();
        assert_eq!((relative.mouse_x, relative.mouse_y), (30, -15));
        assert_eq!(
            InputDataChannel::parse_display_input("i,1,w,0,1", &s)
                .unwrap()
                .geometry_revision,
            Some(1)
        );
    }

    #[test]
    fn test_parse_mouse_move() {
        let event = InputDataChannel::parse_input_text("m,100,200").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseMove);
        assert_eq!(event.mouse_x, 100);
        assert_eq!(event.mouse_y, 200);
    }

    #[test]
    fn test_parse_mouse_move_with_buttons() {
        let event = InputDataChannel::parse_input_text("m,100,200,3").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseMove);
        assert_eq!(event.button_mask, 3);
    }

    #[test]
    fn test_parse_mouse_button() {
        let event = InputDataChannel::parse_input_text("b,1,1").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseButton);
        assert_eq!(event.mouse_button, 1);
        assert!(event.button_pressed);
    }

    #[test]
    fn test_parse_keyboard() {
        let event = InputDataChannel::parse_input_text("k,0xff08,1").unwrap();
        assert_eq!(event.event_type, InputEvent::Keyboard);
        assert_eq!(event.keysym, 0xff08);
        assert!(event.key_pressed);
    }

    #[test]
    fn test_parse_keyboard_decimal() {
        let event = InputDataChannel::parse_input_text("k,65,1").unwrap();
        assert_eq!(event.event_type, InputEvent::Keyboard);
        assert_eq!(event.keysym, 65);
        assert!(event.key_pressed);
    }

    #[test]
    fn test_parse_wheel() {
        let event = InputDataChannel::parse_input_text("w,0,-120").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseWheel);
        assert_eq!(event.wheel_delta_x, 0);
        assert_eq!(event.wheel_delta_y, -120);
    }

    #[test]
    fn test_parse_text_with_comma() {
        let event = InputDataChannel::parse_input_text("t,hello,world").unwrap();
        assert_eq!(event.event_type, InputEvent::TextInput);
        assert_eq!(event.text, "hello,world");
    }
}

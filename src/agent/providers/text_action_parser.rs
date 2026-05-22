use crate::agent::types::{Action, MouseButton};

pub fn parse_text_actions(text: &str) -> Result<Vec<Action>, String> {
    let mut actions = Vec::new();
    for candidate in action_candidates(text) {
        if let Some(action) = parse_candidate(candidate)? {
            actions.push(action);
        }
    }
    if actions.is_empty() {
        Err("no parseable action found in provider text".to_string())
    } else {
        Ok(actions)
    }
}

fn action_candidates(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let lower = line.to_ascii_lowercase();
        if let Some(idx) = lower.find("action:") {
            out.push(line[idx + "action:".len()..].trim());
        } else if looks_like_action(line) {
            out.push(line);
        }
    }
    if out.is_empty() && looks_like_action(text.trim()) {
        out.push(text.trim());
    }
    out
}

fn looks_like_action(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    [
        "click",
        "double_click",
        "move",
        "scroll",
        "type",
        "key",
        "wait",
        "done",
        "screenshot",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn parse_candidate(candidate: &str) -> Result<Option<Action>, String> {
    let name = candidate
        .split_once('(')
        .map(|(name, _)| name)
        .unwrap_or(candidate)
        .trim()
        .trim_matches(|c: char| c == '`' || c == '"' || c == '\'')
        .to_ascii_lowercase();
    let body = candidate
        .split_once('(')
        .and_then(|(_, rest)| rest.rsplit_once(')').map(|(body, _)| body))
        .unwrap_or("");

    match name.as_str() {
        "click" | "left_click" => {
            let (x, y) = parse_point(body)?;
            Ok(Some(Action::MouseClick {
                x,
                y,
                button: MouseButton::Left,
                click_count: 1,
                label: None,
            }))
        }
        "double_click" => {
            let (x, y) = parse_point(body)?;
            Ok(Some(Action::MouseClick {
                x,
                y,
                button: MouseButton::Left,
                click_count: 2,
                label: None,
            }))
        }
        "move" | "mouse_move" => {
            let (x, y) = parse_point(body)?;
            Ok(Some(Action::MouseMove { x, y, label: None }))
        }
        "scroll" => Ok(Some(Action::Scroll {
            x: parse_optional_i32(body, &["x"]),
            y: parse_optional_i32(body, &["y"]),
            dx: parse_optional_i32(body, &["dx", "scroll_x"]).unwrap_or(0),
            dy: parse_optional_i32(body, &["dy", "scroll_y"]).unwrap_or(0),
            label: None,
        })),
        "type" | "input_text" => Ok(Some(Action::TypeText {
            text: parse_string_arg(body, &["content", "text"]).unwrap_or_default(),
            press_enter: parse_bool_arg(body, &["press_enter", "enter"]).unwrap_or(false),
        })),
        "key" | "keypress" => Ok(Some(Action::KeyChord {
            combo: parse_string_arg(body, &["key", "combo", "keys"]).unwrap_or_default(),
        })),
        "wait" => Ok(Some(Action::Wait {
            ms: parse_optional_i32(body, &["ms"]).unwrap_or(1000).max(0) as u32,
        })),
        "screenshot" => Ok(Some(Action::Screenshot)),
        "done" | "finish" => Ok(Some(Action::Done {
            success: parse_bool_arg(body, &["success"]).unwrap_or(true),
            reason: parse_string_arg(body, &["reason"]).unwrap_or_default(),
        })),
        _ => Ok(None),
    }
}

fn parse_point(body: &str) -> Result<(i32, i32), String> {
    if let Some(point) = parse_string_arg(body, &["point", "coordinate", "coords"]) {
        let nums = integers(&point);
        if nums.len() >= 2 {
            return Ok((nums[0], nums[1]));
        }
    }
    let x = parse_optional_i32(body, &["x"]).ok_or_else(|| "missing x".to_string())?;
    let y = parse_optional_i32(body, &["y"]).ok_or_else(|| "missing y".to_string())?;
    Ok((x, y))
}

fn parse_optional_i32(body: &str, keys: &[&str]) -> Option<i32> {
    for key in keys {
        if let Some(value) = parse_value(body, key) {
            if let Ok(parsed) = value.trim_matches('"').trim_matches('\'').parse() {
                return Some(parsed);
            }
        }
    }
    None
}

fn parse_bool_arg(body: &str, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(value) = parse_value(body, key) {
            return match value.trim_matches('"').trim_matches('\'') {
                "true" | "True" | "1" => Some(true),
                "false" | "False" | "0" => Some(false),
                _ => None,
            };
        }
    }
    None
}

fn parse_string_arg(body: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = parse_value(body, key) {
            return Some(value.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn parse_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let lower = body.to_ascii_lowercase();
    let needle = key.to_ascii_lowercase();
    let key_idx = lower.find(&needle)?;
    let rest = body[key_idx + key.len()..].trim_start();
    let rest = rest
        .strip_prefix('=')
        .or_else(|| rest.strip_prefix(':'))?
        .trim_start();
    if let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') {
        let after = &rest[quote.len_utf8()..];
        let end = after.find(quote).unwrap_or(after.len());
        Some(&after[..end])
    } else {
        let end = rest.find(',').unwrap_or(rest.len());
        Some(rest[..end].trim())
    }
}

fn integers(text: &str) -> Vec<i32> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() || (ch == '-' && current.is_empty()) {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(n) = current.parse() {
                out.push(n);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(n) = current.parse() {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ui_tars_click() {
        let actions = parse_text_actions("Thought: ok\nAction: click(point='(840,210)')").unwrap();
        assert!(matches!(
            actions[0],
            Action::MouseClick {
                x: 840,
                y: 210,
                click_count: 1,
                ..
            }
        ));
    }

    #[test]
    fn parses_type_text() {
        let actions = parse_text_actions("Action: type(content='hello')").unwrap();
        assert!(matches!(actions[0], Action::TypeText { ref text, .. } if text == "hello"));
    }
}

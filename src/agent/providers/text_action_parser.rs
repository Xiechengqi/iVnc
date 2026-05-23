use crate::agent::types::{Action, MouseButton};
use serde_json::Value;

pub fn parse_text_actions(text: &str) -> Result<Vec<Action>, String> {
    if let Some(actions) = parse_json_actions(text)? {
        return Ok(actions);
    }

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
        } else {
            let line = strip_leading_label(line);
            if looks_like_action(line) {
                out.push(line);
            }
        }
    }
    if out.is_empty() {
        let text = strip_leading_label(text.trim());
        if looks_like_action(text) {
            out.push(text);
        }
    }
    out
}

fn strip_leading_label(line: &str) -> &str {
    let mut out = line.trim();
    for label in [
        "final answer:",
        "final:",
        "answer:",
        "assistant:",
        "result:",
        "output:",
    ] {
        if out.to_ascii_lowercase().starts_with(label) {
            out = out[label.len()..].trim();
            break;
        }
    }
    out
}

fn looks_like_action(s: &str) -> bool {
    let lower = s
        .trim()
        .trim_matches(|c: char| c == '`' || c == '"' || c == '\'' || c.is_ascii_punctuation())
        .to_ascii_lowercase();
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
    let candidate = candidate.trim();
    let name_part = candidate
        .split_once('(')
        .map(|(name, _)| name.trim())
        .unwrap_or(candidate);
    let name = leading_action_name(name_part).trim().to_ascii_lowercase();
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

fn leading_action_name(candidate: &str) -> &str {
    let candidate =
        candidate.trim_matches(|c: char| c == '`' || c == '"' || c == '\'' || c.is_whitespace());
    let end = candidate
        .char_indices()
        .find_map(|(idx, ch)| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                None
            } else {
                Some(idx)
            }
        })
        .unwrap_or(candidate.len());
    &candidate[..end]
}

fn parse_json_actions(text: &str) -> Result<Option<Vec<Action>>, String> {
    for candidate in json_candidates(text) {
        if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
            let actions = json_value_to_actions(&value)?;
            if !actions.is_empty() {
                return Ok(Some(actions));
            }
        }
    }
    Ok(None)
}

fn json_candidates(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let mut out = vec![trimmed.to_string()];

    if let Some(fenced) = fenced_json(trimmed) {
        out.push(fenced.to_string());
    }

    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            out.push(trimmed[start..=end].to_string());
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if start < end {
            out.push(trimmed[start..=end].to_string());
        }
    }

    out
}

fn fenced_json(text: &str) -> Option<&str> {
    let start = text.find("```")?;
    let after_start = &text[start + 3..];
    let after_lang = after_start
        .strip_prefix("json")
        .unwrap_or(after_start)
        .trim_start_matches(|c: char| c.is_whitespace());
    let end = after_lang.find("```")?;
    Some(after_lang[..end].trim())
}

fn json_value_to_actions(value: &Value) -> Result<Vec<Action>, String> {
    match value {
        Value::Array(items) => items
            .iter()
            .map(json_object_to_action)
            .collect::<Result<Vec<_>, _>>()
            .map(|actions| actions.into_iter().flatten().collect()),
        Value::Object(object) => {
            for key in ["actions", "tool_calls"] {
                if let Some(items) = object.get(key).and_then(Value::as_array) {
                    return items
                        .iter()
                        .map(json_object_to_action)
                        .collect::<Result<Vec<_>, _>>()
                        .map(|actions| actions.into_iter().flatten().collect());
                }
            }
            Ok(json_object_to_action(value)?.into_iter().collect())
        }
        _ => Ok(Vec::new()),
    }
}

fn json_object_to_action(value: &Value) -> Result<Option<Action>, String> {
    let function = value.get("function").unwrap_or(value);
    let name = function
        .get("name")
        .or_else(|| function.get("action"))
        .or_else(|| function.get("type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.is_empty() {
        return Ok(None);
    }

    let args = match function.get("arguments") {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .map_err(|e| format!("invalid JSON action arguments: {}", e))?,
        Some(value) => value.clone(),
        None => function.clone(),
    };
    json_action_to_action(&name, &args).map(Some)
}

fn json_action_to_action(name: &str, args: &Value) -> Result<Action, String> {
    match name {
        "click" | "left_click" | "mouse_click" => {
            let (x, y) = json_point(args)?;
            Ok(Action::MouseClick {
                x,
                y,
                button: json_mouse_button(args),
                click_count: json_u8(args, &["click_count"]).unwrap_or(1).clamp(1, 3),
                label: json_string(args, &["label", "reason"]),
            })
        }
        "double_click" => {
            let (x, y) = json_point(args)?;
            Ok(Action::MouseClick {
                x,
                y,
                button: json_mouse_button(args),
                click_count: 2,
                label: json_string(args, &["label", "reason"]),
            })
        }
        "move" | "mouse_move" => {
            let (x, y) = json_point(args)?;
            Ok(Action::MouseMove {
                x,
                y,
                label: json_string(args, &["label", "reason"]),
            })
        }
        "scroll" => Ok(Action::Scroll {
            x: json_i32(args, &["x"]),
            y: json_i32(args, &["y"]),
            dx: json_i32(args, &["dx", "scroll_x"]).unwrap_or(0),
            dy: json_i32(args, &["dy", "scroll_y"]).unwrap_or(0),
            label: json_string(args, &["label", "reason"]),
        }),
        "type" | "type_text" | "input_text" => Ok(Action::TypeText {
            text: json_string(args, &["text", "content"]).unwrap_or_default(),
            press_enter: json_bool(args, &["press_enter", "enter"]).unwrap_or(false),
        }),
        "key" | "keypress" | "key_chord" => Ok(Action::KeyChord {
            combo: json_string(args, &["combo", "key", "keys"]).unwrap_or_default(),
        }),
        "wait" => Ok(Action::Wait {
            ms: json_i32(args, &["ms"]).unwrap_or(1000).max(0) as u32,
        }),
        "screenshot" => Ok(Action::Screenshot),
        "done" | "finish" => Ok(Action::Done {
            success: json_bool(args, &["success"]).unwrap_or(true),
            reason: json_string(args, &["reason"]).unwrap_or_default(),
        }),
        _ => Err(format!("unsupported provider action JSON: {}", name)),
    }
}

fn json_point(args: &Value) -> Result<(i32, i32), String> {
    if let Some(point) = args
        .get("point")
        .or_else(|| args.get("coordinate"))
        .or_else(|| args.get("coords"))
    {
        if let Some(values) = point.as_array() {
            if values.len() >= 2 {
                return Ok((
                    values[0].as_f64().unwrap_or_default().round() as i32,
                    values[1].as_f64().unwrap_or_default().round() as i32,
                ));
            }
        }
        if let Some(s) = point.as_str() {
            let nums = integers(s);
            if nums.len() >= 2 {
                return Ok((nums[0], nums[1]));
            }
        }
    }
    Ok((
        json_i32(args, &["x"]).ok_or_else(|| "missing x".to_string())?,
        json_i32(args, &["y"]).ok_or_else(|| "missing y".to_string())?,
    ))
}

fn json_mouse_button(args: &Value) -> MouseButton {
    match json_string(args, &["button"])
        .unwrap_or_else(|| "left".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "middle" => MouseButton::Middle,
        "right" => MouseButton::Right,
        _ => MouseButton::Left,
    }
}

fn json_i32(args: &Value, keys: &[&str]) -> Option<i32> {
    for key in keys {
        if let Some(value) = args.get(*key) {
            if let Some(n) = value.as_i64() {
                return Some(n as i32);
            }
            if let Some(n) = value.as_f64() {
                return Some(n.round() as i32);
            }
            if let Some(s) = value.as_str().and_then(|s| s.parse::<f64>().ok()) {
                return Some(s.round() as i32);
            }
        }
    }
    None
}

fn json_u8(args: &Value, keys: &[&str]) -> Option<u8> {
    json_i32(args, keys).map(|value| value.max(0) as u8)
}

fn json_bool(args: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(value) = args.get(*key) {
            if let Some(b) = value.as_bool() {
                return Some(b);
            }
            if let Some(s) = value.as_str() {
                return match s.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => Some(true),
                    "false" | "0" | "no" => Some(false),
                    _ => None,
                };
            }
        }
    }
    None
}

fn json_string(args: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = args.get(*key) {
            if let Some(s) = value.as_str() {
                return Some(s.to_string());
            }
            if value.is_number() || value.is_boolean() {
                return Some(value.to_string());
            }
            if let Some(values) = value.as_array() {
                return Some(
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("+"),
                );
            }
        }
    }
    None
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

    #[test]
    fn parses_done_with_punctuation() {
        let actions = parse_text_actions("Final answer: Done.").unwrap();
        assert!(matches!(actions[0], Action::Done { success: true, .. }));
    }

    #[test]
    fn parses_json_action_object() {
        let actions =
            parse_text_actions(r#"{"action":"done","success":true,"reason":"visible"}"#).unwrap();
        assert!(matches!(
            actions[0],
            Action::Done {
                success: true,
                ref reason
            } if reason == "visible"
        ));
    }

    #[test]
    fn parses_fenced_json_tool_call() {
        let actions = parse_text_actions(
            r#"```json
{"function":{"name":"click","arguments":"{\"x\":840,\"y\":210}"}}
```"#,
        )
        .unwrap();
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
}

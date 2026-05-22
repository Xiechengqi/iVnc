use super::types::{Action, DestructiveKind};

pub fn destructive_kind(action: &Action) -> Option<DestructiveKind> {
    match action {
        Action::WindowClose { .. } => Some(DestructiveKind::WindowClose),
        Action::ClipboardWrite { .. } => Some(DestructiveKind::ClipboardWriteOverwrite),
        Action::KeyChord { combo } if is_destructive_combo(combo) => {
            Some(DestructiveKind::KeyboardCombo)
        }
        _ => None,
    }
}

fn is_destructive_combo(combo: &str) -> bool {
    let normalized = combo.to_ascii_lowercase().replace(' ', "");
    matches!(
        normalized.as_str(),
        "alt+f4" | "ctrl+alt+del" | "cmd+q" | "super+q"
    )
}

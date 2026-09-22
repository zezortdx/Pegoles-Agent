//! Platform-independent key vocabulary (Phase 5).
//!
//! Actions carry key names as strings, never raw macOS keycodes or Linux
//! keycodes. Adapters map these names to platform codes; unknown names
//! are rejected before anything executes.

/// Canonical modifier names.
pub const MODIFIERS: &[&str] = &["Shift", "Control", "Alt", "Meta"];

/// Canonical named keys (beyond single printable characters).
pub const NAMED_KEYS: &[&str] = &[
    "Enter",
    "Escape",
    "Tab",
    "Backspace",
    "Delete",
    "Insert",
    "Space",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
];

/// Normalize a user/model key name to canonical form.
/// Returns `None` for unknown names. Aliases: Return->Enter, Esc->Escape,
/// Del->Delete, Super/Cmd/Command->Meta, Option->Alt.
pub fn normalize_key_name(name: &str) -> Option<String> {
    let t = name.trim();
    if t.is_empty() {
        return None;
    }
    // Single printable ASCII character (typed literally, Shift handled by
    // the adapter from case or explicit modifier).
    if t.chars().count() == 1 {
        let c = t.chars().next().expect("counted");
        if c.is_ascii_graphic() || c == ' ' {
            return Some(t.to_string());
        }
        return None;
    }
    let canonical = match t.to_lowercase().as_str() {
        "enter" | "return" => "Enter",
        "escape" | "esc" => "Escape",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "insert" | "ins" => "Insert",
        "space" => "Space",
        "arrowup" | "up" => "ArrowUp",
        "arrowdown" | "down" => "ArrowDown",
        "arrowleft" | "left" => "ArrowLeft",
        "arrowright" | "right" => "ArrowRight",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" => "PageUp",
        "pagedown" | "pgdn" => "PageDown",
        "shift" => "Shift",
        "control" | "ctrl" => "Control",
        "alt" | "option" => "Alt",
        "meta" | "super" | "cmd" | "command" | "windows" => "Meta",
        "f1" => "F1",
        "f2" => "F2",
        "f3" => "F3",
        "f4" => "F4",
        "f5" => "F5",
        "f6" => "F6",
        "f7" => "F7",
        "f8" => "F8",
        "f9" => "F9",
        "f10" => "F10",
        "f11" => "F11",
        "f12" => "F12",
        _ => return None,
    };
    Some(canonical.to_string())
}

/// True for modifier keys (chord members that stay held).
pub fn is_modifier(name: &str) -> bool {
    normalize_key_name(name).is_some_and(|n| MODIFIERS.contains(&n.as_str()))
}

/// Validate a chord: 1..=4 keys, at least one non-modifier, no duplicates.
pub fn validate_chord(keys: &[String]) -> Result<Vec<String>, String> {
    if keys.is_empty() || keys.len() > 4 {
        return Err(format!("chord must hold 1..=4 keys, got {}", keys.len()));
    }
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        let n = normalize_key_name(k).ok_or_else(|| format!("unknown key: {k}"))?;
        if out.contains(&n) {
            return Err(format!("duplicate key in chord: {n}"));
        }
        out.push(n);
    }
    if out.iter().all(|k| MODIFIERS.contains(&k.as_str())) {
        return Err("chord needs at least one non-modifier key".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_normalize() {
        assert_eq!(normalize_key_name("return").as_deref(), Some("Enter"));
        assert_eq!(normalize_key_name("ESC").as_deref(), Some("Escape"));
        assert_eq!(normalize_key_name("cmd").as_deref(), Some("Meta"));
        assert_eq!(normalize_key_name("Option").as_deref(), Some("Alt"));
        assert_eq!(normalize_key_name("a").as_deref(), Some("a"));
        assert_eq!(normalize_key_name("F5").as_deref(), Some("F5"));
        assert_eq!(normalize_key_name("Nope"), None);
        assert_eq!(normalize_key_name(""), None);
    }

    #[test]
    fn chord_rules() {
        assert!(validate_chord(&["Control".into(), "c".into()]).is_ok());
        assert!(validate_chord(&["Shift".into()]).is_err());
        assert!(validate_chord(&["Control".into(), "Control".into()]).is_err());
        assert!(validate_chord(&[]).is_err());
    }
}

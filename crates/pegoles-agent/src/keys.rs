//! Key names models use (xdotool / pyautogui / plain words) mapped to
//! canonical Pegoles key names. Shared by every provider's parser; the
//! guest re-validates every key it receives.

/// Parse an xdotool-style combination ("ctrl+shift+t", "Return",
/// "Page_Down") into canonical Pegoles key names.
pub fn parse_key_combo(combo: &str) -> Result<Vec<String>, String> {
    let combo = combo.trim();
    let tokens: Vec<&str> = if combo == "+" {
        vec!["+"]
    } else if let Some(head) = combo.strip_suffix("++") {
        head.split('+').chain(std::iter::once("+")).collect()
    } else {
        combo.split('+').collect()
    };
    let mut keys = Vec::new();
    for token in tokens.into_iter().filter(|t| !t.is_empty()) {
        keys.push(key_name(token).ok_or_else(|| format!("unknown key {token:?}"))?);
    }
    if keys.is_empty() || keys.len() > 4 {
        return Err(format!("key combination {combo:?} must name 1 to 4 keys"));
    }
    Ok(keys)
}

pub fn key_name(token: &str) -> Option<String> {
    let lower = token.to_lowercase();
    let named = match lower.as_str() {
        "return" | "enter" | "kp_enter" => "Enter",
        "backspace" => "Backspace",
        "escape" | "esc" => "Escape",
        "tab" | "iso_left_tab" => "Tab",
        "space" => "Space",
        "delete" | "del" | "kp_delete" => "Delete",
        "insert" => "Insert",
        "home" | "kp_home" => "Home",
        "end" | "kp_end" => "End",
        "page_up" | "prior" | "pageup" => "PageUp",
        "page_down" | "next" | "pagedown" => "PageDown",
        "up" | "kp_up" => "ArrowUp",
        "down" | "kp_down" => "ArrowDown",
        "left" | "kp_left" => "ArrowLeft",
        "right" | "kp_right" => "ArrowRight",
        "ctrl" | "control" | "control_l" | "control_r" => "Control",
        "alt" | "alt_l" | "alt_r" | "option" => "Alt",
        "shift" | "shift_l" | "shift_r" => "Shift",
        "super" | "super_l" | "super_r" | "meta" | "meta_l" | "cmd" | "win" => "Meta",
        "minus" => "-",
        "plus" => "+",
        "equal" => "=",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "backslash" => "\\",
        "semicolon" => ";",
        "apostrophe" => "'",
        "quotedbl" => "\"",
        "grave" => "`",
        "bracketleft" => "[",
        "bracketright" => "]",
        "braceleft" => "{",
        "braceright" => "}",
        "parenleft" => "(",
        "parenright" => ")",
        "underscore" => "_",
        "colon" => ":",
        "less" => "<",
        "greater" => ">",
        "question" => "?",
        "exclam" => "!",
        "at" => "@",
        "numbersign" => "#",
        "dollar" => "$",
        "percent" => "%",
        "asciicircum" => "^",
        "ampersand" => "&",
        "asterisk" => "*",
        "asciitilde" => "~",
        "bar" => "|",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    pegoles_protocol::normalize_key_name(token)
}

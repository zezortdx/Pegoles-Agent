//! Strict parsing of local-model output into typed Pegoles actions.
//!
//! The model is an untrusted planner. Its text is searched for exactly
//! one `<tool_call>{json}</tool_call>`; the JSON must name the family's
//! tool and use only known actions and argument keys with bounded,
//! finite values. Anything else is an error the model can read, never an
//! action. The result is a [`ModelAction`], converted to the same typed
//! `ComputerAction`s every provider produces, then policy-checked by
//! Core like any other action.

use pegoles_inference::ModelFamily;
use pegoles_protocol::{limits, ComputerAction, PointerButton};
use serde_json::{Map, Value};

/// Largest model output the parser looks at (the worker caps text too).
pub const MAX_OUTPUT_CHARS: usize = 16 * 1024;
const MAX_THOUGHT_CHARS: usize = 400;
const MAX_ANSWER_CHARS: usize = 2_000;
/// Logical scroll units per swipe / scroll "click".
const SWIPE_UNITS: f64 = 5.0;
const MAX_SCROLL_CLICKS: f64 = 30.0;
const MAX_WAIT_SECS: f64 = 10.0;
const DRAG_MS: u32 = 600;

/// What the model decided, in model terms but already validated.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelAction {
    Computer(Vec<ComputerAction>),
    /// The model says the objective is complete (answer or summary).
    Done(String),
    /// The model says the objective cannot be completed.
    Fail(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedStep {
    pub thought: Option<String>,
    pub action: ModelAction,
    /// Canonical JSON of the tool call (for the history the model sees).
    pub call_json: String,
    /// Short verb for logs ("click", "type", …).
    pub verb: String,
}

/// Coordinate scale: MAI-UI emits 0..=999, Qwen3-VL 0..=1000, both
/// relative to the image the model saw.
fn scale(family: ModelFamily) -> f64 {
    match family {
        ModelFamily::MaiUi => 999.0,
        ModelFamily::Qwen3Vl => 1000.0,
    }
}

pub fn tool_name(family: ModelFamily) -> &'static str {
    match family {
        ModelFamily::MaiUi => "mobile_use",
        ModelFamily::Qwen3Vl => "computer_use",
    }
}

/// Parse one model reply. `cursor` is the last pointer position the
/// planner set (normalized), for drag-from-cursor actions.
pub fn parse(
    family: ModelFamily,
    raw: &str,
    truncated: bool,
    cursor: Option<(f64, f64)>,
) -> Result<ParsedStep, String> {
    if raw.chars().count() > MAX_OUTPUT_CHARS {
        return Err("the reply is too long".into());
    }
    let thought = extract_thought(raw);
    let open = "<tool_call>";
    let close = "</tool_call>";
    let starts: Vec<usize> = raw.match_indices(open).map(|(i, _)| i).collect();
    match starts.len() {
        0 if truncated => return Err("the reply was cut off before the action".into()),
        0 => return Err("no <tool_call> found; reply with exactly one tool call".into()),
        1 => {}
        _ => return Err("more than one <tool_call>; propose exactly one action per step".into()),
    }
    let body_start = starts[0] + open.len();
    let body_len = raw[body_start..]
        .find(close)
        .ok_or("the <tool_call> is not closed")?;
    let body = raw[body_start..body_start + body_len].trim();
    let call: Value =
        serde_json::from_str(body).map_err(|_| "the tool call is not valid JSON".to_string())?;
    let obj = call
        .as_object()
        .ok_or("the tool call must be a JSON object")?;
    only_keys(obj, &["name", "arguments"], "tool call")?;
    // There is one tool: a missing name is harmless, a different one is not.
    let name = match obj.get("name") {
        None => tool_name(family),
        Some(v) => v.as_str().ok_or("the tool name must be a string")?,
    };
    if name != tool_name(family) {
        return Err(format!(
            "unknown tool {:?}; the only tool is {:?}",
            short(name),
            tool_name(family)
        ));
    }
    let args = obj
        .get("arguments")
        .and_then(Value::as_object)
        .ok_or("missing arguments object")?;
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or("missing \"action\"")?;
    let parsed = match family {
        ModelFamily::MaiUi => mai_action(action, args, thought.as_deref())?,
        ModelFamily::Qwen3Vl => qwen_action(action, args, cursor, thought.as_deref())?,
    };
    Ok(ParsedStep {
        thought,
        action: parsed,
        call_json: canonical_call(family, args),
        verb: action.chars().take(24).collect(),
    })
}

/// The call as the model's own format writes it: name first, then the
/// arguments with `action` first. The model imitates its history, so
/// the history must look like its training data.
fn canonical_call(family: ModelFamily, args: &Map<String, Value>) -> String {
    const ORDER: &[&str] = &[
        "action",
        "coordinate",
        "start_coordinate",
        "end_coordinate",
        "text",
        "keys",
        "direction",
        "button",
        "pixels",
        "time",
        "status",
    ];
    let fields: Vec<String> = ORDER
        .iter()
        .filter_map(|k| {
            args.get(*k).map(|v| {
                format!(
                    "{}:{}",
                    Value::String((*k).to_string()),
                    serde_json::to_string(v).unwrap_or_default()
                )
            })
        })
        .collect();
    format!(
        "{{\"name\":\"{}\",\"arguments\":{{{}}}}}",
        tool_name(family),
        fields.join(",")
    )
}

fn extract_thought(raw: &str) -> Option<String> {
    let (open, close) = if raw.contains("<thinking>") {
        ("<thinking>", "</thinking>")
    } else if raw.contains("<think>") {
        ("<think>", "</think>")
    } else {
        // Qwen3-VL Instruct writes plain text before the tool call.
        let before = raw.split("<tool_call>").next().unwrap_or("").trim();
        return (!before.is_empty()).then(|| clip(before, MAX_THOUGHT_CHARS));
    };
    let start = raw.find(open)? + open.len();
    let end = raw[start..].find(close).map_or(raw.len(), |e| start + e);
    let t = raw[start..end].trim();
    let t = t.strip_prefix("Thought:").map_or(t, str::trim);
    (!t.is_empty()).then(|| clip(t, MAX_THOUGHT_CHARS))
}

/// Bidi overrides/isolates, zero-width and other invisible formatting
/// characters: they can make displayed text read differently from what
/// it is.
pub fn is_invisible_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{061C}' | '\u{00AD}')
}

fn clip(s: &str, n: usize) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !is_invisible_format(*c))
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    if cleaned.chars().count() > n {
        let mut out: String = cleaned.chars().take(n).collect();
        out.push('…');
        out
    } else {
        cleaned
    }
}

fn short(s: &str) -> String {
    clip(s, 40)
}

fn only_keys(obj: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    match obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("unexpected field {:?} in {what}", short(k))),
        None => Ok(()),
    }
}

fn point(family: ModelFamily, v: Option<&Value>, key: &str) -> Result<(f64, f64), String> {
    let arr = v
        .and_then(Value::as_array)
        .ok_or_else(|| format!("\"{key}\" must be [x, y]"))?;
    let nums: Vec<f64> = arr
        .iter()
        .map(|n| n.as_f64().filter(|f| f.is_finite()))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("\"{key}\" must contain numbers"))?;
    let (x, y) = match nums.as_slice() {
        [x, y] => (*x, *y),
        // A box: act on its center (MAI-UI sometimes answers with one).
        [x1, y1, x2, y2] => ((x1 + x2) / 2.0, (y1 + y2) / 2.0),
        _ => return Err(format!("\"{key}\" must be [x, y]")),
    };
    let s = scale(family);
    if !(0.0..=s).contains(&x) || !(0.0..=s).contains(&y) {
        return Err(format!("\"{key}\" is outside the screen (0..{s})"));
    }
    Ok((x / s, y / s))
}

fn text_arg(args: &Map<String, Value>, key: &str, max: usize) -> Result<String, String> {
    let t = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("\"{key}\" must be a string"))?;
    if t.chars().count() > max {
        return Err(format!("\"{key}\" is longer than {max} characters"));
    }
    Ok(t.to_string())
}

fn typed(text: String) -> Result<ModelAction, String> {
    if text.is_empty() {
        return Err("\"text\" is empty".into());
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("\"text\" contains control characters".into());
    }
    if text.chars().any(is_invisible_format) {
        return Err("\"text\" contains invisible formatting characters".into());
    }
    Ok(ModelAction::Computer(vec![ComputerAction::TypeText {
        text,
        sensitive: false,
    }]))
}

fn keys_action(v: Option<&Value>) -> Result<ModelAction, String> {
    let list: Vec<&str> = match v {
        Some(Value::Array(a)) => a
            .iter()
            .map(Value::as_str)
            .collect::<Option<_>>()
            .ok_or("\"keys\" must be a list of key names")?,
        // A list written as a string ("[enter]", "['ctrl', 'c']"): the
        // names are still checked one by one below.
        Some(Value::String(s)) if s.starts_with('[') && s.ends_with(']') => s[1..s.len() - 1]
            .split(',')
            .map(|k| k.trim().trim_matches(|c| c == '"' || c == '\''))
            .filter(|k| !k.is_empty())
            .collect(),
        Some(Value::String(s)) => vec![s.as_str()],
        _ => return Err("\"keys\" must be a list of key names".into()),
    };
    // One entry may itself be a combo ("ctrl+c").
    let mut keys = Vec::new();
    for k in &list {
        if k.len() > 1 && k.contains('+') {
            keys.extend(crate::keys::parse_key_combo(k)?);
        } else {
            keys.push(
                crate::keys::key_name(k).ok_or_else(|| format!("unknown key {:?}", short(k)))?,
            );
        }
    }
    let action = match keys.len() {
        1 => ComputerAction::KeyPress {
            key: keys.remove(0),
        },
        2..=4 => ComputerAction::KeyChord { keys },
        _ => return Err("press 1 to 4 keys at once".into()),
    };
    Ok(ModelAction::Computer(vec![action]))
}

fn click(x: f64, y: f64, button: PointerButton, double: bool) -> ModelAction {
    ModelAction::Computer(vec![if double {
        ComputerAction::DoubleClick { x, y, button }
    } else {
        ComputerAction::Click { x, y, button }
    }])
}

fn wait_secs(v: Option<&Value>) -> Result<ModelAction, String> {
    let secs = match v {
        None => 1.0,
        Some(n) => n
            .as_f64()
            .filter(|f| f.is_finite() && *f >= 0.0)
            .ok_or("\"time\" must be a number of seconds")?,
    };
    let ms = (secs.min(MAX_WAIT_SECS) * 1000.0).round() as u32;
    Ok(ModelAction::Computer(vec![ComputerAction::Wait {
        duration_ms: ms.max(100),
    }]))
}

fn finish(success: bool, answer: Option<String>, thought: Option<&str>) -> ModelAction {
    let text = answer
        .filter(|a| !a.trim().is_empty())
        .or_else(|| thought.map(str::to_string))
        .unwrap_or_else(|| {
            if success {
                "Finished.".into()
            } else {
                "The model could not complete the task.".into()
            }
        });
    let text = clip(text.trim(), MAX_ANSWER_CHARS);
    if success {
        ModelAction::Done(text)
    } else {
        ModelAction::Fail(text)
    }
}

/// MAI-UI: its trained mobile action space plus the desktop additions
/// Pegoles' prompt declares (`key`, `double_click`, `right_click`).
fn mai_action(
    action: &str,
    args: &Map<String, Value>,
    thought: Option<&str>,
) -> Result<ModelAction, String> {
    let fam = ModelFamily::MaiUi;
    match action {
        "click" | "double_click" | "right_click" => {
            only_keys(args, &["action", "coordinate"], action)?;
            let (x, y) = point(fam, args.get("coordinate"), "coordinate")?;
            let button = if action == "right_click" {
                PointerButton::Secondary
            } else {
                PointerButton::Primary
            };
            Ok(click(x, y, button, action == "double_click"))
        }
        "type" => {
            only_keys(args, &["action", "text"], action)?;
            typed(text_arg(args, "text", limits::MAX_TYPE_CHARS)?)
        }
        "key" => {
            only_keys(args, &["action", "keys"], action)?;
            keys_action(args.get("keys"))
        }
        "swipe" => {
            only_keys(args, &["action", "direction", "coordinate"], action)?;
            let (x, y) = match args.get("coordinate") {
                Some(c) => point(fam, Some(c), "coordinate")?,
                None => (0.5, 0.5),
            };
            // Touch semantics: swiping up reveals content below.
            let (dx, dy) = match args.get("direction").and_then(Value::as_str) {
                Some("up") => (0.0, SWIPE_UNITS),
                Some("down") => (0.0, -SWIPE_UNITS),
                Some("left") => (SWIPE_UNITS, 0.0),
                Some("right") => (-SWIPE_UNITS, 0.0),
                _ => return Err("\"direction\" must be up, down, left or right".into()),
            };
            Ok(ModelAction::Computer(vec![ComputerAction::Scroll {
                x,
                y,
                delta_x: dx,
                delta_y: dy,
            }]))
        }
        "drag" => {
            only_keys(
                args,
                &["action", "start_coordinate", "end_coordinate"],
                action,
            )?;
            let (fx, fy) = point(fam, args.get("start_coordinate"), "start_coordinate")?;
            let (tx, ty) = point(fam, args.get("end_coordinate"), "end_coordinate")?;
            Ok(ModelAction::Computer(vec![ComputerAction::Drag {
                from_x: fx,
                from_y: fy,
                to_x: tx,
                to_y: ty,
                button: PointerButton::Primary,
                duration_ms: DRAG_MS,
            }]))
        }
        "system_button" => {
            only_keys(args, &["action", "button"], action)?;
            match args
                .get("button")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
            {
                Some(b) if b == "enter" => keys_action(Some(&Value::String("Enter".into()))),
                Some(b) if b == "back" => keys_action(Some(&Value::String("Escape".into()))),
                _ => Err("on this desktop \"button\" must be enter or back".into()),
            }
        }
        "wait" => {
            only_keys(args, &["action", "time"], action)?;
            wait_secs(args.get("time"))
        }
        "terminate" => {
            only_keys(args, &["action", "status"], action)?;
            match args.get("status").and_then(Value::as_str) {
                Some("success") => Ok(finish(true, None, thought)),
                Some("fail") | Some("failure") => Ok(finish(false, None, thought)),
                _ => Err("\"status\" must be success or fail".into()),
            }
        }
        "answer" => {
            only_keys(args, &["action", "text"], action)?;
            Ok(finish(
                true,
                Some(text_arg(args, "text", MAX_ANSWER_CHARS)?),
                thought,
            ))
        }
        "open" => {
            Err("there is no app launcher; open apps by clicking or from the terminal".into())
        }
        "long_press" => Err("long_press is not available on this desktop; use click".into()),
        other => Err(format!("unknown action {:?}", short(other))),
    }
}

/// Qwen3-VL: the official `computer_use` tool.
fn qwen_action(
    action: &str,
    args: &Map<String, Value>,
    cursor: Option<(f64, f64)>,
    thought: Option<&str>,
) -> Result<ModelAction, String> {
    let fam = ModelFamily::Qwen3Vl;
    match action {
        "left_click" | "right_click" | "middle_click" | "double_click" => {
            only_keys(args, &["action", "coordinate"], action)?;
            let (x, y) = point(fam, args.get("coordinate"), "coordinate")?;
            let button = match action {
                "right_click" => PointerButton::Secondary,
                "middle_click" => PointerButton::Middle,
                _ => PointerButton::Primary,
            };
            Ok(click(x, y, button, action == "double_click"))
        }
        "mouse_move" => {
            only_keys(args, &["action", "coordinate"], action)?;
            let (x, y) = point(fam, args.get("coordinate"), "coordinate")?;
            Ok(ModelAction::Computer(vec![ComputerAction::MovePointer {
                x,
                y,
            }]))
        }
        "left_click_drag" => {
            only_keys(args, &["action", "coordinate"], action)?;
            let (tx, ty) = point(fam, args.get("coordinate"), "coordinate")?;
            let (fx, fy) = cursor.ok_or("move the mouse to the drag start first (mouse_move)")?;
            Ok(ModelAction::Computer(vec![ComputerAction::Drag {
                from_x: fx,
                from_y: fy,
                to_x: tx,
                to_y: ty,
                button: PointerButton::Primary,
                duration_ms: DRAG_MS,
            }]))
        }
        "type" => {
            only_keys(args, &["action", "text"], action)?;
            typed(text_arg(args, "text", limits::MAX_TYPE_CHARS)?)
        }
        "key" => {
            only_keys(args, &["action", "keys"], action)?;
            keys_action(args.get("keys"))
        }
        "scroll" | "hscroll" => {
            only_keys(args, &["action", "pixels", "coordinate"], action)?;
            let clicks = args
                .get("pixels")
                .and_then(Value::as_f64)
                .filter(|f| f.is_finite())
                .ok_or("\"pixels\" must be a number")?;
            if clicks.abs() > MAX_SCROLL_CLICKS * 100.0 {
                return Err("\"pixels\" is out of range".into());
            }
            let (x, y) = match args.get("coordinate") {
                Some(c) => point(fam, Some(c), "coordinate")?,
                None => cursor.unwrap_or((0.5, 0.5)),
            };
            // Positive `pixels` scrolls up (content moves down): the
            // opposite of Pegoles' positive `delta_y`.
            let units = (-clicks).clamp(-MAX_SCROLL_CLICKS, MAX_SCROLL_CLICKS);
            let (dx, dy) = if action == "hscroll" {
                (units, 0.0)
            } else {
                (0.0, units)
            };
            if dx == 0.0 && dy == 0.0 {
                return Err("\"pixels\" must not be zero".into());
            }
            Ok(ModelAction::Computer(vec![ComputerAction::Scroll {
                x,
                y,
                delta_x: dx,
                delta_y: dy,
            }]))
        }
        "wait" => {
            only_keys(args, &["action", "time"], action)?;
            wait_secs(args.get("time"))
        }
        "terminate" => {
            only_keys(args, &["action", "status"], action)?;
            match args.get("status").and_then(Value::as_str) {
                Some("success") => Ok(finish(true, None, thought)),
                Some("failure") | Some("fail") => Ok(finish(false, None, thought)),
                _ => Err("\"status\" must be success or failure".into()),
            }
        }
        "answer" => {
            only_keys(args, &["action", "text"], action)?;
            Ok(finish(
                true,
                Some(text_arg(args, "text", MAX_ANSWER_CHARS)?),
                thought,
            ))
        }
        "triple_click" => Err("triple_click is not available; use double_click".into()),
        other => Err(format!("unknown action {:?}", short(other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: ModelFamily = ModelFamily::Qwen3Vl;
    const M: ModelFamily = ModelFamily::MaiUi;

    fn q(json: &str) -> Result<ParsedStep, String> {
        parse(
            Q,
            &format!("I will act.\n<tool_call>\n{json}\n</tool_call>"),
            false,
            None,
        )
    }

    fn m(json: &str) -> Result<ParsedStep, String> {
        parse(
            M,
            &format!("<thinking>\nplan\n</thinking>\n<tool_call>\n{json}\n</tool_call>"),
            false,
            None,
        )
    }

    fn computer(p: ParsedStep) -> Vec<ComputerAction> {
        match p.action {
            ModelAction::Computer(a) => a,
            other => panic!("expected computer action, got {other:?}"),
        }
    }

    #[test]
    fn qwen_click_maps_relative_coordinates() {
        let p = q(
            r#"{"name":"computer_use","arguments":{"action":"left_click","coordinate":[500,250]}}"#,
        )
        .unwrap();
        assert_eq!(p.thought.as_deref(), Some("I will act."));
        assert_eq!(
            computer(p),
            vec![ComputerAction::Click {
                x: 0.5,
                y: 0.25,
                button: PointerButton::Primary
            }]
        );
    }

    #[test]
    fn mai_click_uses_its_999_scale_and_box_centers() {
        let p = m(r#"{"name":"mobile_use","arguments":{"action":"click","coordinate":[999,0]}}"#)
            .unwrap();
        assert_eq!(p.thought.as_deref(), Some("plan"));
        assert!(
            matches!(computer(p)[0], ComputerAction::Click { x, y, .. } if x == 1.0 && y == 0.0)
        );
        let p = m(r#"{"name":"mobile_use","arguments":{"action":"click","coordinate":[100,100,300,300]}}"#)
            .unwrap();
        assert!(
            matches!(computer(p)[0], ComputerAction::Click { x, .. } if (x - 200.0/999.0).abs() < 1e-9)
        );
    }

    #[test]
    fn hostile_coordinates_are_rejected() {
        for c in [
            "[-1,5]",
            "[1001,5]",
            "[5,1e308]",
            "[\"5\",5]",
            "[5]",
            "[1,2,3]",
            "null",
            "[5,5,5,5,5]",
        ] {
            let r = q(&format!(
                r#"{{"name":"computer_use","arguments":{{"action":"left_click","coordinate":{c}}}}}"#
            ));
            assert!(r.is_err(), "{c} accepted");
        }
        // NaN / Infinity are not JSON at all.
        assert!(q(
            r#"{"name":"computer_use","arguments":{"action":"left_click","coordinate":[NaN,1]}}"#
        )
        .is_err());
        assert!(q(r#"{"name":"computer_use","arguments":{"action":"left_click","coordinate":[Infinity,1]}}"#).is_err());
        // MAI's scale ends at 999.
        assert!(
            m(r#"{"name":"mobile_use","arguments":{"action":"click","coordinate":[1000,5]}}"#)
                .is_err()
        );
    }

    #[test]
    fn schema_escapes_are_rejected() {
        let bad = [
            // other tools / host verbs
            r#"{"name":"bash","arguments":{"action":"run","command":"rm -rf ~"}}"#,
            r#"{"name":"computer_use","arguments":{"action":"shell","command":"ls"}}"#,
            r#"{"name":"computer_use","arguments":{"action":"open_url","url":"http://x"}}"#,
            // smuggled fields
            r#"{"name":"computer_use","arguments":{"action":"left_click","coordinate":[1,1],"exec":"x"}}"#,
            r#"{"name":"computer_use","arguments":{"action":"type","text":"hi"},"policy":"bypass"}"#,
            // wrong types
            r#"{"name":"computer_use","arguments":"left_click"}"#,
            r#"["computer_use"]"#,
            r#"{"name":"computer_use","arguments":{"action":"type","text":"a\u0007b"}}"#,
            r#"{"name":"computer_use","arguments":{"action":"type","text":""}}"#,
            r#"{"name":"computer_use","arguments":{"action":"key","keys":["ctrl","alt","shift","meta","x"]}}"#,
            r#"{"name":"computer_use","arguments":{"action":"key","keys":["hyper"]}}"#,
            r#"{"name":"computer_use","arguments":{"action":"scroll","pixels":0}}"#,
            r#"{"name":"computer_use","arguments":{"action":"triple_click","coordinate":[1,1]}}"#,
        ];
        for b in bad {
            assert!(q(b).is_err(), "accepted: {b}");
        }
        assert!(
            m(r#"{"name":"mobile_use","arguments":{"action":"open","text":"Terminal"}}"#).is_err()
        );
        assert!(m(
            r#"{"name":"mobile_use","arguments":{"action":"system_button","button":"home"}}"#
        )
        .is_err());
    }

    #[test]
    fn history_echo_keeps_the_trained_key_order() {
        let p = m(r#"{"arguments":{"text":"hi","action":"type"}}"#).unwrap();
        assert_eq!(
            p.call_json,
            r#"{"name":"mobile_use","arguments":{"action":"type","text":"hi"}}"#
        );
        let p =
            q(r#"{"name":"computer_use","arguments":{"coordinate":[1,2],"action":"left_click"}}"#)
                .unwrap();
        assert_eq!(
            p.call_json,
            r#"{"name":"computer_use","arguments":{"action":"left_click","coordinate":[1,2]}}"#
        );
        assert!(q(r#"{"name":7,"arguments":{"action":"wait"}}"#).is_err());
    }

    #[test]
    fn exactly_one_closed_call_is_required() {
        let one = r#"{"name":"computer_use","arguments":{"action":"wait"}}"#;
        let two = format!("<tool_call>{one}</tool_call><tool_call>{one}</tool_call>");
        assert!(parse(Q, &two, false, None)
            .unwrap_err()
            .contains("more than one"));
        assert!(parse(Q, &format!("<tool_call>{one}"), false, None).is_err());
        assert!(parse(Q, "just text", false, None)
            .unwrap_err()
            .contains("no <tool_call>"));
        assert!(parse(Q, "<thinking>long", true, None)
            .unwrap_err()
            .contains("cut off"));
        assert!(parse(Q, &"x".repeat(MAX_OUTPUT_CHARS + 1), false, None).is_err());
    }

    #[test]
    fn typing_keys_scroll_and_drag_translate() {
        let a = computer(q(r#"{"name":"computer_use","arguments":{"action":"type","text":"echo 'ok' $(date)\n"}}"#).unwrap());
        assert_eq!(
            a,
            vec![ComputerAction::TypeText {
                text: "echo 'ok' $(date)\n".into(),
                sensitive: false
            }]
        );
        let a = computer(
            q(r#"{"name":"computer_use","arguments":{"action":"key","keys":["ctrl","c"]}}"#)
                .unwrap(),
        );
        assert_eq!(
            a,
            vec![ComputerAction::KeyChord {
                keys: vec!["Control".into(), "c".into()]
            }]
        );
        let a = computer(
            q(r#"{"name":"computer_use","arguments":{"action":"key","keys":["Return"]}}"#).unwrap(),
        );
        assert_eq!(
            a,
            vec![ComputerAction::KeyPress {
                key: "Enter".into()
            }]
        );
        let a = computer(
            q(r#"{"name":"computer_use","arguments":{"action":"scroll","pixels":-3}}"#).unwrap(),
        );
        assert!(matches!(a[0], ComputerAction::Scroll { delta_y, .. } if delta_y == 3.0));
        let a = computer(
            m(r#"{"name":"mobile_use","arguments":{"action":"swipe","direction":"up"}}"#).unwrap(),
        );
        assert!(matches!(a[0], ComputerAction::Scroll { delta_y, .. } if delta_y > 0.0));
        let a = computer(
            m(r#"{"name":"mobile_use","arguments":{"action":"key","keys":"[enter]"}}"#).unwrap(),
        );
        assert_eq!(
            a,
            vec![ComputerAction::KeyPress {
                key: "Enter".into()
            }]
        );
        let a = computer(
            m(r#"{"name":"mobile_use","arguments":{"action":"key","keys":"['ctrl', 'c']"}}"#)
                .unwrap(),
        );
        assert_eq!(
            a,
            vec![ComputerAction::KeyChord {
                keys: vec!["Control".into(), "c".into()]
            }]
        );
        assert!(
            m(r#"{"name":"mobile_use","arguments":{"action":"key","keys":"[rm -rf]"}}"#).is_err()
        );
        let a = computer(
            m(r#"{"name":"mobile_use","arguments":{"action":"system_button","button":"enter"}}"#)
                .unwrap(),
        );
        assert_eq!(
            a,
            vec![ComputerAction::KeyPress {
                key: "Enter".into()
            }]
        );
        let a = computer(
            m(r#"{"name":"mobile_use","arguments":{"action":"key","keys":["ctrl+shift+t"]}}"#)
                .unwrap(),
        );
        assert_eq!(
            a,
            vec![ComputerAction::KeyChord {
                keys: vec!["Control".into(), "Shift".into(), "t".into()]
            }]
        );
        // Qwen drags from the cursor it last set.
        let drag = r#"{"name":"computer_use","arguments":{"action":"left_click_drag","coordinate":[900,900]}}"#;
        assert!(parse(Q, &format!("<tool_call>{drag}</tool_call>"), false, None).is_err());
        let a = computer(
            parse(
                Q,
                &format!("<tool_call>{drag}</tool_call>"),
                false,
                Some((0.1, 0.1)),
            )
            .unwrap(),
        );
        assert!(
            matches!(a[0], ComputerAction::Drag { from_x, to_x, .. } if from_x == 0.1 && to_x == 0.9)
        );
    }

    #[test]
    fn terminate_and_answer_end_the_task() {
        let p =
            q(r#"{"name":"computer_use","arguments":{"action":"terminate","status":"success"}}"#)
                .unwrap();
        assert_eq!(p.action, ModelAction::Done("I will act.".into()));
        let p = m(r#"{"name":"mobile_use","arguments":{"action":"terminate","status":"fail"}}"#)
            .unwrap();
        assert_eq!(p.action, ModelAction::Fail("plan".into()));
        let p = m(r#"{"name":"mobile_use","arguments":{"action":"answer","text":"42"}}"#).unwrap();
        assert_eq!(p.action, ModelAction::Done("42".into()));
    }

    #[test]
    fn oversized_or_hostile_unicode_is_bounded() {
        let long = "é".repeat(limits::MAX_TYPE_CHARS + 1);
        assert!(q(&format!(
            r#"{{"name":"computer_use","arguments":{{"action":"type","text":"{long}"}}}}"#
        ))
        .is_err());
        // Bidi overrides and zero-width characters could make the typed
        // text read differently in the activity feed: refused.
        let p = q("{\"name\":\"computer_use\",\"arguments\":{\"action\":\"type\",\"text\":\"a\\u202eb\"}}");
        assert!(p.unwrap_err().contains("invisible"));
        let p = q("{\"name\":\"computer_use\",\"arguments\":{\"action\":\"type\",\"text\":\"a\\u200bb\"}}");
        assert!(p.is_err());
        // Thoughts are clipped and control characters neutralized.
        let noisy = format!("{}\u{7}<tool_call>{{\"name\":\"computer_use\",\"arguments\":{{\"action\":\"wait\"}}}}</tool_call>", "t".repeat(5000));
        let p = parse(Q, &noisy, false, None).unwrap();
        let t = p.thought.unwrap();
        assert!(t.chars().count() <= MAX_THOUGHT_CHARS + 1);
        assert!(!t.contains('\u{7}'));
    }
}

//! Property tests for the local-model parser: whatever the model writes,
//! `parse` never panics, what it returns is bounded, and anything that
//! does not conform to the one-call, known-schema format is an error.
//! The minimized cases proptest found (or that pin a known edge) are kept
//! as plain tests at the end.

use super::*;
use proptest::prelude::*;
use proptest::sample::select;
use serde_json::json;

const FAMILIES: [ModelFamily; 2] = [ModelFamily::MaiUi, ModelFamily::Qwen3Vl];

/// Every action either family knows, plus host-shaped verbs a hostile
/// model might try.
const ACTIONS: &[&str] = &[
    "click",
    "double_click",
    "right_click",
    "middle_click",
    "left_click",
    "mouse_move",
    "left_click_drag",
    "drag",
    "type",
    "key",
    "system_button",
    "swipe",
    "scroll",
    "hscroll",
    "wait",
    "terminate",
    "answer",
    "open",
    "long_press",
    "triple_click",
    "shell",
    "bash",
    "open_url",
    "read_file",
    "screenshot",
];

/// Every argument key either family's schema knows.
const ARG_KEYS: &[&str] = &[
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

/// The schema keys plus smuggling attempts.
const KEYS: &[&str] = &[
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
    "command",
    "url",
    "path",
    "exec",
    "policy",
];

/// Fragments that have broken parsers and prompts before.
const HOSTILE: &[&str] = &[
    "<tool_call>",
    "</tool_call>",
    "<thinking>",
    "</thinking>",
    "<think>",
    "</think>",
    "<|im_start|>",
    "<|im_end|>",
    "<|image_pad|>",
    "\u{202E}",
    "\u{200B}",
    "\u{2066}",
    "\u{FEFF}",
    "\u{E0041}",
    "\u{0}",
    "\u{7}",
    "\u{1b}[31m",
    "\u{85}",
    "\u{212A}",
    "é",
    "漢字",
    "🙂",
    "ctrl+c",
    "[enter]",
    "['ctrl', 'c']",
    "Return",
    "success",
    "fail",
    "up",
    "enter",
    "computer_use",
    "mobile_use",
    "-----BEGIN RSA PRIVATE KEY-----",
    "sk-ant-api03-",
    "NaN",
    "Infinity",
    "1e999",
    "{",
    "}",
    "[",
    "]",
    "\"",
    "\\",
    "\n",
];

fn hostile_string() -> impl Strategy<Value = String> {
    prop_oneof![
        ".{0,24}",
        "[a-z_]{0,12}",
        select(HOSTILE).prop_map(|s| s.to_string()),
        prop::collection::vec(select(HOSTILE), 0..6).prop_map(|v| v.concat()),
    ]
}

fn json_leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter("JSON numbers are finite", |f| f.is_finite())
            .prop_map(Value::from),
        (-100.0f64..1100.0).prop_map(Value::from),
        (0u32..=1000).prop_map(Value::from),
        hostile_string().prop_map(Value::from),
    ]
}

fn json_value() -> impl Strategy<Value = Value> {
    json_leaf().prop_recursive(3, 24, 5, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::btree_map(key(), inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn key() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => select(KEYS).prop_map(|s| s.to_string()),
        1 => hostile_string(),
    ]
}

/// A tool call as a model might write it: mostly the right shape, often
/// subtly wrong.
fn call() -> impl Strategy<Value = Value> {
    let action = prop_oneof![
        4 => select(ACTIONS).prop_map(Value::from),
        1 => json_leaf(),
    ];
    let name = prop_oneof![
        Just(Some(Value::from("computer_use"))),
        Just(Some(Value::from("mobile_use"))),
        Just(None),
        json_leaf().prop_map(Some),
    ];
    let extra_top = prop::option::weighted(0.1, (hostile_string(), json_leaf()));
    (
        name,
        action,
        prop::collection::btree_map(key(), json_value(), 0..4),
        extra_top,
    )
        .prop_map(|(name, action, args, extra)| {
            let mut args: Map<String, Value> = args.into_iter().collect();
            args.insert("action".into(), action);
            let mut call = Map::new();
            if let Some(n) = name {
                call.insert("name".into(), n);
            }
            call.insert("arguments".into(), Value::Object(args));
            if let Some((k, v)) = extra {
                call.insert(k, v);
            }
            Value::Object(call)
        })
}

/// A whole reply: free text, an optional thinking block, zero or more
/// (possibly unclosed) tool-call wrappers around a call body that may
/// also be textually corrupted.
fn reply() -> impl Strategy<Value = String> {
    let body = (call(), 0u8..6, hostile_string()).prop_map(|(c, how, noise)| {
        let json = c.to_string();
        match how {
            0 => json.chars().take(json.chars().count() / 2).collect(),
            1 => json.replacen("1", "NaN", 1),
            2 => format!("{json}{noise}"),
            _ => json,
        }
    });
    (
        hostile_string(),
        prop::option::of(hostile_string()),
        0u8..8,
        body,
        hostile_string(),
    )
        .prop_map(|(pre, think, wrap, body, post)| {
            let think = think.map_or(String::new(), |t| format!("<thinking>{t}</thinking>\n"));
            let wrapped = match wrap {
                0 => body,
                1 => format!("<tool_call>{body}"),
                2 => format!("<tool_call>{body}</tool_call><tool_call>{body}</tool_call>"),
                3 => format!("{body}</tool_call>"),
                _ => format!("<tool_call>\n{body}\n</tool_call>"),
            };
            format!("{pre}{think}{wrapped}{post}")
        })
}

fn any_reply() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => reply(),
        1 => ".{0,200}",
        1 => any::<String>(),
    ]
}

fn cursor() -> impl Strategy<Value = Option<(f64, f64)>> {
    prop::option::of((0.0f64..=1.0, 0.0f64..=1.0))
}

fn family() -> impl Strategy<Value = ModelFamily> {
    select(FAMILIES.to_vec())
}

fn in_unit(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}

fn clean(t: &str, keep: &[char]) -> bool {
    !t.chars()
        .any(|c| is_invisible_format(c) || (c.is_control() && !keep.contains(&c)))
}

fn canonical_key(k: &str) -> bool {
    pegoles_protocol::normalize_key_name(k).as_deref() == Some(k)
}

/// What any accepted reply must look like, whatever the input was.
fn assert_bounded(family: ModelFamily, raw: &str, step: &ParsedStep) -> Result<(), TestCaseError> {
    if let Some(t) = &step.thought {
        prop_assert!(t.chars().count() <= MAX_THOUGHT_CHARS + 1, "thought {t:?}");
        prop_assert!(clean(t, &['\n']), "thought {t:?}");
    }
    prop_assert!(step.verb.chars().count() <= 24);
    prop_assert!(step.call_json.len() <= 2 * raw.len() + 256);
    // The history echo is JSON naming this family's one tool.
    let echo: Value = serde_json::from_str(&step.call_json)
        .map_err(|e| TestCaseError::fail(format!("echo is not JSON: {e}")))?;
    prop_assert_eq!(echo["name"].as_str(), Some(tool_name(family)));
    match &step.action {
        ModelAction::Done(t) | ModelAction::Fail(t) => {
            prop_assert!(t.chars().count() <= MAX_ANSWER_CHARS + 1);
            prop_assert!(clean(t, &['\n']), "{t:?}");
        }
        ModelAction::Computer(actions) => {
            prop_assert_eq!(actions.len(), 1, "one action per step");
            match &actions[0] {
                ComputerAction::Click { x, y, .. }
                | ComputerAction::DoubleClick { x, y, .. }
                | ComputerAction::MovePointer { x, y } => {
                    prop_assert!(in_unit(*x) && in_unit(*y), "{x} {y}")
                }
                ComputerAction::Scroll {
                    x,
                    y,
                    delta_x,
                    delta_y,
                } => {
                    prop_assert!(in_unit(*x) && in_unit(*y));
                    prop_assert!(delta_x.is_finite() && delta_y.is_finite());
                    prop_assert!(delta_x.abs() <= MAX_SCROLL_CLICKS);
                    prop_assert!(delta_y.abs() <= MAX_SCROLL_CLICKS);
                    prop_assert!(*delta_x != 0.0 || *delta_y != 0.0);
                }
                ComputerAction::Drag {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    duration_ms,
                    ..
                } => {
                    prop_assert!([from_x, from_y, to_x, to_y].iter().all(|v| in_unit(**v)));
                    prop_assert_eq!(*duration_ms, DRAG_MS);
                }
                ComputerAction::TypeText { text, sensitive } => {
                    prop_assert!(!sensitive);
                    let n = text.chars().count();
                    prop_assert!((1..=limits::MAX_TYPE_CHARS).contains(&n));
                    prop_assert!(clean(text, &['\n', '\t']), "{text:?}");
                }
                ComputerAction::KeyPress { key } => prop_assert!(canonical_key(key), "{key:?}"),
                ComputerAction::KeyChord { keys } => {
                    prop_assert!((2..=4).contains(&keys.len()));
                    prop_assert!(keys.iter().all(|k| canonical_key(k)), "{keys:?}");
                }
                ComputerAction::Wait { duration_ms } => {
                    prop_assert!((100..=(MAX_WAIT_SECS as u32 * 1000)).contains(duration_ms))
                }
                other => prop_assert!(false, "the parser never emits {other:?}"),
            }
        }
    }
    Ok(())
}

fn wrap(call: &Value) -> String {
    format!("<tool_call>\n{call}\n</tool_call>")
}

/// One well-formed call per action, per family.
fn valid_calls(family: ModelFamily) -> Vec<Value> {
    let name = tool_name(family);
    let args: Vec<Value> = match family {
        ModelFamily::MaiUi => vec![
            json!({"action": "click", "coordinate": [10, 20]}),
            json!({"action": "type", "text": "hi"}),
            json!({"action": "key", "keys": ["ctrl", "c"]}),
            json!({"action": "swipe", "direction": "up"}),
            json!({"action": "drag", "start_coordinate": [1, 1], "end_coordinate": [9, 9]}),
            json!({"action": "system_button", "button": "enter"}),
            json!({"action": "wait"}),
            json!({"action": "terminate", "status": "success"}),
            json!({"action": "answer", "text": "42"}),
        ],
        ModelFamily::Qwen3Vl => vec![
            json!({"action": "left_click", "coordinate": [10, 20]}),
            json!({"action": "mouse_move", "coordinate": [10, 20]}),
            json!({"action": "type", "text": "hi"}),
            json!({"action": "key", "keys": ["Return"]}),
            json!({"action": "scroll", "pixels": -3}),
            json!({"action": "wait", "time": 1}),
            json!({"action": "terminate", "status": "success"}),
            json!({"action": "answer", "text": "42"}),
        ],
    };
    args.into_iter()
        .map(|a| json!({"name": name, "arguments": a}))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        // Keep minimized cases below as tests instead of files on disk.
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn never_panics_and_what_it_accepts_is_bounded(
        raw in any_reply(),
        family in family(),
        truncated in any::<bool>(),
        cursor in cursor(),
    ) {
        if let Ok(step) = parse(family, &raw, truncated, cursor) {
            assert_bounded(family, &raw, &step)?;
            // Exactly one closed call was present.
            prop_assert_eq!(raw.matches("<tool_call>").count(), 1);
        }
    }

    #[test]
    fn replies_without_exactly_one_closed_call_are_refused(
        text in ".{0,300}",
        family in family(),
        call in call(),
    ) {
        let text = text.replace("<tool_call>", "");
        prop_assert!(parse(family, &text, false, None).is_err());
        prop_assert!(parse(family, &text, true, None).is_err());
        let one = wrap(&call);
        let two = format!("{one}{text}{one}");
        prop_assert!(parse(family, &two, false, None).is_err());
        let unclosed = format!("<tool_call>{call}{}", text.replace("</tool_call>", ""));
        prop_assert!(parse(family, &unclosed, false, None).is_err());
    }

    #[test]
    fn smuggled_fields_are_refused(
        family in family(),
        pick in any::<prop::sample::Index>(),
        extra in "[a-z_]{1,12}",
        value in json_leaf(),
        top_level in any::<bool>(),
    ) {
        let calls = valid_calls(family);
        let mut call = pick.get(&calls).clone();
        prop_assert!(parse(family, &wrap(&call), false, Some((0.5, 0.5))).is_ok());
        prop_assume!(!ARG_KEYS.contains(&extra.as_str()) && extra != "name" && extra != "arguments");
        if top_level {
            call[extra.as_str()] = value;
        } else {
            call["arguments"][extra.as_str()] = value;
        }
        prop_assert!(parse(family, &wrap(&call), false, Some((0.5, 0.5))).is_err());
    }

    #[test]
    fn other_tool_names_are_refused(
        family in family(),
        pick in any::<prop::sample::Index>(),
        name in hostile_string(),
    ) {
        prop_assume!(name != tool_name(family));
        let mut call = pick.get(&valid_calls(family)).clone();
        call["name"] = Value::from(name);
        prop_assert!(parse(family, &wrap(&call), false, None).is_err());
    }

    #[test]
    fn coordinates_outside_the_screen_are_refused(
        family in family(),
        x in prop_oneof![-2000.0f64..3000.0, any::<f64>().prop_filter("finite", |f| f.is_finite())],
        y in prop_oneof![-2000.0f64..3000.0, any::<f64>().prop_filter("finite", |f| f.is_finite())],
    ) {
        let action = match family {
            ModelFamily::MaiUi => "click",
            ModelFamily::Qwen3Vl => "left_click",
        };
        let call = json!({
            "name": tool_name(family),
            "arguments": {"action": action, "coordinate": [x, y]},
        });
        let s = scale(family);
        let inside = (0.0..=s).contains(&x) && (0.0..=s).contains(&y);
        match parse(family, &wrap(&call), false, None) {
            Ok(step) => {
                prop_assert!(inside, "({x}, {y}) accepted");
                assert_bounded(family, &wrap(&call), &step)?;
            }
            Err(_) => prop_assert!(!inside, "({x}, {y}) refused"),
        }
    }

    #[test]
    fn typed_text_is_exact_or_refused(family in family(), text in prop_oneof![
        ".{0,64}",
        hostile_string(),
        (select(HOSTILE), 4000usize..4200).prop_map(|(s, n)| s.repeat(n / s.chars().count().max(1))),
    ]) {
        let call = json!({
            "name": tool_name(family),
            "arguments": {"action": "type", "text": text},
        });
        let n = text.chars().count();
        // Text quoting the call tags makes the reply ambiguous (two
        // openings, or a close inside the JSON): refused, fail closed.
        let conforming = (1..=limits::MAX_TYPE_CHARS).contains(&n)
            && clean(&text, &['\n', '\t'])
            && !text.contains("<tool_call>")
            && !text.contains("</tool_call>");
        match parse(family, &wrap(&call), false, None) {
            Ok(step) => {
                prop_assert!(conforming, "{text:?} accepted");
                prop_assert_eq!(
                    step.action,
                    ModelAction::Computer(vec![ComputerAction::TypeText { text, sensitive: false }])
                );
            }
            Err(_) => prop_assert!(!conforming, "{text:?} refused"),
        }
    }

    #[test]
    fn replies_over_the_size_cap_are_refused(family in family(), pad in 0usize..64) {
        let call = wrap(&valid_calls(family)[0]);
        let raw = format!("{}{call}", "t".repeat(MAX_OUTPUT_CHARS + 1 - call.chars().count() + pad));
        prop_assert!(parse(family, &raw, false, None).is_err());
    }
}

// --- minimized regressions ------------------------------------------------

#[test]
fn look_alike_key_names_are_refused() {
    // KELVIN SIGN lowercases to ASCII `k`: "BAC\u{212A}SPACE" used to
    // become Backspace.
    for family in FAMILIES {
        for keys in [
            json!(["BAC\u{212A}SPACE"]),
            json!("\u{212A}p_enter"),
            json!(["ctrl", "\u{212A}"]),
        ] {
            let call =
                json!({"name": tool_name(family), "arguments": {"action": "key", "keys": keys}});
            assert!(parse(family, &wrap(&call), false, None).is_err(), "{keys}");
        }
    }
}

#[test]
fn typed_text_quoting_the_call_tags_is_refused() {
    // Found by `typed_text_is_exact_or_refused`: fail closed, never a
    // guess at which tag is the real one.
    for text in ["<tool_call>", "a</tool_call>b"] {
        for family in FAMILIES {
            let call =
                json!({"name": tool_name(family), "arguments": {"action": "type", "text": text}});
            assert!(parse(family, &wrap(&call), false, None).is_err(), "{text}");
        }
    }
}

#[test]
fn a_call_quoted_inside_the_thought_counts_as_a_second_call() {
    let one = wrap(&valid_calls(ModelFamily::MaiUi)[0]);
    let raw = format!("<thinking>I will write {one}</thinking>{one}");
    assert!(parse(ModelFamily::MaiUi, &raw, false, None).is_err());
}

#[test]
fn overflowing_box_centers_and_unrepresentable_numbers_are_refused() {
    for coordinate in [
        "[1e308,1e308,1e308,1e308]",
        "[1e999,1]",
        "[NaN,1]",
        "[-0.0001,1]",
    ] {
        let raw = format!(
            "<tool_call>{{\"name\":\"mobile_use\",\"arguments\":{{\"action\":\"click\",\"coordinate\":{coordinate}}}}}</tool_call>"
        );
        assert!(
            parse(ModelFamily::MaiUi, &raw, false, None).is_err(),
            "{coordinate}"
        );
    }
}

#[test]
fn deeply_nested_json_fails_without_overflowing_the_stack() {
    let deep = format!("{}1{}", "[".repeat(5_000), "]".repeat(5_000));
    let raw = format!(
        "<tool_call>{{\"name\":\"computer_use\",\"arguments\":{{\"action\":\"type\",\"text\":{deep}}}}}</tool_call>"
    );
    assert!(parse(ModelFamily::Qwen3Vl, &raw, false, None).is_err());
}

#[test]
fn degenerate_key_lists_are_refused() {
    for keys in [
        json!("[]"),
        json!([]),
        json!("[,,]"),
        json!(["", ""]),
        json!("[a,b,c,d,e]"),
    ] {
        let call = json!({"name": "mobile_use", "arguments": {"action": "key", "keys": keys}});
        assert!(
            parse(ModelFamily::MaiUi, &wrap(&call), false, None).is_err(),
            "{keys}"
        );
    }
}

#[test]
fn multibyte_text_around_the_tags_is_sliced_safely() {
    let raw = "é漢🙂<thinking>ção 🙂<tool_call>{\"name\":\"computer_use\",\"arguments\":{\"action\":\"wait\"}}</tool_call>漢";
    // `<thinking>` never closes: the thought runs to the end, clipped.
    let step = parse(ModelFamily::Qwen3Vl, raw, false, None).unwrap();
    assert!(matches!(step.action, ModelAction::Computer(_)));
    assert!(parse(ModelFamily::Qwen3Vl, "🙂<tool_call>", true, None).is_err());
    assert!(parse(ModelFamily::Qwen3Vl, "<think>🙂", true, None).is_err());
}

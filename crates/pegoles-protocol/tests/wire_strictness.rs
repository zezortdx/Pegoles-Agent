//! Wire strictness for the action vocabulary (release security matrix).
//!
//! `ComputerAction` arrives from untrusted JSON (debug IPC, input
//! scripts). The contract: ONE object, the `type` tag plus exactly the
//! variant's fields. Everything else must fail to deserialize, never
//! parse into something a caller did not write, and never panic.

use pegoles_protocol::{ActionRequest, ComputerAction, ComputerId, PointerButton, TaskId};
use proptest::prelude::*;
use serde_json::{json, Value};

/// Every tag and the fields it accepts (besides `type`).
const SCHEMA: &[(&str, &[&str])] = &[
    ("observe_screen", &[]),
    ("get_display_info", &[]),
    ("move_pointer", &["x", "y"]),
    ("click", &["x", "y", "button"]),
    ("double_click", &["x", "y", "button"]),
    ("mouse_down", &["x", "y", "button"]),
    ("mouse_up", &["x", "y", "button"]),
    (
        "drag",
        &["from_x", "from_y", "to_x", "to_y", "button", "duration_ms"],
    ),
    ("scroll", &["x", "y", "delta_x", "delta_y"]),
    ("key_press", &["key"]),
    ("key_chord", &["keys"]),
    ("type_text", &["text", "sensitive"]),
    ("wait", &["duration_ms"]),
];

fn allowed_fields(tag: &str) -> Option<&'static [&'static str]> {
    SCHEMA.iter().find(|(t, _)| *t == tag).map(|(_, f)| *f)
}

/// One valid, fully spelled-out object per variant.
fn valid_objects() -> Vec<Value> {
    vec![
        json!({"type": "observe_screen"}),
        json!({"type": "get_display_info"}),
        json!({"type": "move_pointer", "x": 0.5, "y": 0.5}),
        json!({"type": "click", "x": 0.5, "y": 0.5, "button": "primary"}),
        json!({"type": "double_click", "x": 0.5, "y": 0.5, "button": "secondary"}),
        json!({"type": "mouse_down", "x": 0.5, "y": 0.5, "button": "middle"}),
        json!({"type": "mouse_up", "x": 0.5, "y": 0.5, "button": "primary"}),
        json!({"type": "drag", "from_x": 0.1, "from_y": 0.1, "to_x": 0.9, "to_y": 0.9,
               "button": "primary", "duration_ms": 400}),
        json!({"type": "scroll", "x": 0.5, "y": 0.5, "delta_x": 0.0, "delta_y": 3.0}),
        json!({"type": "key_press", "key": "Enter"}),
        json!({"type": "key_chord", "keys": ["Control", "c"]}),
        json!({"type": "type_text", "text": "hello", "sensitive": false}),
        json!({"type": "wait", "duration_ms": 500}),
    ]
}

fn parse_value(v: &Value) -> Result<ComputerAction, serde_json::Error> {
    serde_json::from_value::<ComputerAction>(v.clone())
}

fn parse_text(s: &str) -> Result<ComputerAction, serde_json::Error> {
    serde_json::from_str::<ComputerAction>(s)
}

/// Both paths real callers use: JSON text and a `serde_json::Value`
/// (the IPC layer deserializes command arguments from a `Value`).
fn assert_rejected(v: &Value) {
    assert!(parse_value(v).is_err(), "value parsed: {v}");
    assert!(parse_text(&v.to_string()).is_err(), "text parsed: {v}");
}

fn with_field(obj: &Value, key: &str, val: Value) -> Value {
    let mut o = obj.clone();
    o.as_object_mut().unwrap().insert(key.into(), val);
    o
}

#[test]
fn every_variant_parses_and_round_trips() {
    let objects = valid_objects();
    assert_eq!(objects.len(), SCHEMA.len(), "one object per tag");
    for obj in &objects {
        let action = parse_value(obj).unwrap_or_else(|e| panic!("{obj}: {e}"));
        assert_eq!(parse_text(&obj.to_string()).unwrap(), action);
        let back = serde_json::to_value(&action).unwrap();
        assert_eq!(&back, obj, "serialized form is the canonical wire form");
        assert_eq!(parse_value(&back).unwrap(), action);
    }
}

#[test]
fn optional_fields_still_default() {
    assert_eq!(
        parse_text(r#"{"type":"click","x":0.5,"y":0.5}"#).unwrap(),
        ComputerAction::Click {
            x: 0.5,
            y: 0.5,
            button: PointerButton::Primary
        }
    );
    assert_eq!(
        parse_text(r#"{"type":"type_text","text":"a"}"#).unwrap(),
        ComputerAction::TypeText {
            text: "a".into(),
            sensitive: false
        }
    );
    // Field order is free; `type` need not come first.
    assert!(parse_text(r#"{"y":0.5,"x":0.5,"type":"move_pointer"}"#).is_ok());
}

#[test]
fn unknown_fields_are_rejected_on_every_variant() {
    let extras = [
        ("command", json!("ls")),
        ("path", json!("/etc/passwd")),
        ("url", json!("http://x")),
        ("extra", Value::Null),
        ("Type", json!("click")),
        ("X", json!(0.5)),
        ("__proto__", json!({})),
        ("host", json!(true)),
        // A nested action smuggled in beside the real one.
        (
            "action",
            json!({"type": "type_text", "text": "rm -rf ~", "sensitive": false}),
        ),
    ];
    for obj in valid_objects() {
        for (k, v) in &extras {
            assert_rejected(&with_field(&obj, k, v.clone()));
        }
    }
}

#[test]
fn unit_variants_reject_fields_that_belong_elsewhere() {
    // serde's own internally tagged unit visitor skips these silently.
    for tag in ["observe_screen", "get_display_info"] {
        assert_rejected(&json!({"type": tag, "x": 0.5, "y": 0.5}));
        assert_rejected(&json!({"type": tag, "text": "whoami"}));
        assert_rejected(&json!({"type": tag, "observe_after": true}));
    }
}

#[test]
fn unknown_or_malformed_tags_are_rejected() {
    for tag in [
        json!("shell"),
        json!("run"),
        json!("exec"),
        json!("open_url"),
        json!("read_file"),
        json!("host_shell"),
        json!("type"),
        json!("screenshot"),
        json!("Click"),
        json!("CLICK"),
        json!(" click"),
        json!("click\u{200B}"),
        json!("move-pointer"),
        json!("MovePointer"),
        json!(""),
        json!(1),
        json!(null),
        json!(true),
        json!(["click"]),
        json!({"click": {}}),
    ] {
        assert_rejected(&json!({"type": tag, "x": 0.5, "y": 0.5}));
    }
    // Missing tag entirely.
    assert_rejected(&json!({"x": 0.5, "y": 0.5}));
    assert_rejected(&json!({}));
}

#[test]
fn non_object_shapes_are_rejected() {
    for v in [
        // Positional forms serde accepts for internally tagged enums.
        json!(["click", 0.5, 0.5, "primary"]),
        json!(["move_pointer", 0.5, 0.5]),
        json!(["observe_screen"]),
        json!(["wait", 10]),
        // Other representations.
        json!("observe_screen"),
        json!("click"),
        json!({"click": {"x": 0.5, "y": 0.5}}),
        json!({"type": "click", "content": {"x": 0.5, "y": 0.5}}),
        json!({"Click": {"x": 0.5, "y": 0.5, "button": "primary"}}),
        // A batch is not an action.
        json!([{"type": "observe_screen"}, {"type": "wait", "duration_ms": 1}]),
        json!([]),
        json!(null),
        json!(0),
        json!(true),
    ] {
        assert_rejected(&v);
    }
}

#[test]
fn duplicate_fields_and_tags_are_rejected() {
    // Only text can carry duplicates (a `Value` map cannot).
    for s in [
        r#"{"type":"click","x":0.1,"x":0.9,"y":0.5}"#,
        r#"{"type":"click","type":"click","x":0.5,"y":0.5}"#,
        r#"{"type":"click","type":"wait","x":0.5,"y":0.5,"duration_ms":1}"#,
        r#"{"type":"type_text","text":"ls","text":"rm -rf ~"}"#,
        r#"{"type":"type_text","text":"a","sensitive":true,"sensitive":false}"#,
        r#"{"type":"key_chord","keys":["Control","c"],"keys":["Meta","q"]}"#,
        r#"{"type":"wait","duration_ms":1,"duration_ms":30000}"#,
        r#"{"type":"observe_screen","type":"observe_screen"}"#,
    ] {
        assert!(parse_text(s).is_err(), "parsed: {s}");
    }
}

#[test]
fn nested_or_mistyped_fields_are_rejected() {
    let cases = [
        json!({"type": "move_pointer", "x": {"type": "number"}, "y": 0.5}),
        json!({"type": "move_pointer", "x": [0.5], "y": 0.5}),
        json!({"type": "move_pointer", "x": "0.5", "y": 0.5}),
        json!({"type": "move_pointer", "x": true, "y": 0.5}),
        json!({"type": "move_pointer", "x": null, "y": 0.5}),
        json!({"type": "move_pointer", "x": 0.5}),
        json!({"type": "click", "x": 0.5, "y": 0.5, "button": "left"}),
        json!({"type": "click", "x": 0.5, "y": 0.5, "button": "PRIMARY"}),
        json!({"type": "click", "x": 0.5, "y": 0.5, "button": 0}),
        json!({"type": "click", "x": 0.5, "y": 0.5, "button": null}),
        json!({"type": "key_press", "key": ["Enter"]}),
        json!({"type": "key_press", "key": {"name": "Enter"}}),
        json!({"type": "key_press", "key": 13}),
        json!({"type": "key_press"}),
        json!({"type": "key_chord", "keys": "Control+c"}),
        json!({"type": "key_chord", "keys": [["Control"], "c"]}),
        json!({"type": "key_chord", "keys": [{"key": "Control"}, "c"]}),
        json!({"type": "key_chord", "keys": null}),
        json!({"type": "type_text", "text": {"$ref": "#/secret"}}),
        json!({"type": "type_text", "text": ["a", "b"]}),
        json!({"type": "type_text", "text": 42}),
        json!({"type": "type_text", "text": "a", "sensitive": "true"}),
        json!({"type": "type_text", "text": "a", "sensitive": 1}),
        json!({"type": "type_text"}),
        json!({"type": "scroll", "x": 0.5, "y": 0.5, "delta_x": "1", "delta_y": 0}),
        json!({"type": "scroll", "x": 0.5, "y": 0.5, "delta_y": 3}),
        json!({"type": "drag", "from_x": 0.1, "from_y": 0.1, "to_x": 0.2, "to_y": 0.2}),
    ];
    for v in &cases {
        assert_rejected(v);
    }
}

#[test]
fn durations_outside_u32_or_fractional_are_rejected() {
    for d in [
        json!(-1),
        json!(-0.0),
        json!(1.5),
        json!(4_294_967_296u64),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(1e3),
        json!("500"),
        json!(null),
    ] {
        assert_rejected(&json!({"type": "wait", "duration_ms": d}));
        assert_rejected(
            &json!({"type": "drag", "from_x": 0.1, "from_y": 0.1, "to_x": 0.2,
                                "to_y": 0.2, "duration_ms": d}),
        );
    }
}

#[test]
fn non_json_numbers_are_rejected_as_text() {
    // JSON has no NaN/Infinity; overflowing literals do not become inf.
    for s in [
        r#"{"type":"move_pointer","x":NaN,"y":0.5}"#,
        r#"{"type":"move_pointer","x":Infinity,"y":0.5}"#,
        r#"{"type":"move_pointer","x":-Infinity,"y":0.5}"#,
        r#"{"type":"move_pointer","x":1e400,"y":0.5}"#,
        r#"{"type":"move_pointer","x":-1e400,"y":0.5}"#,
        r#"{"type":"move_pointer","x":0x1,"y":0.5}"#,
        r#"{"type":"move_pointer","x":.5,"y":0.5}"#,
        r#"{"type":"move_pointer","x":0.5,"y":0.5,}"#,
    ] {
        assert!(parse_text(s).is_err(), "parsed: {s}");
    }
}

#[test]
fn action_request_rejects_unknown_fields() {
    let req = ActionRequest::new(
        TaskId::new(),
        ComputerId::new(),
        ComputerAction::Wait { duration_ms: 10 },
    );
    let wire = serde_json::to_value(&req).unwrap();
    assert_eq!(
        serde_json::from_value::<ActionRequest>(wire.clone()).unwrap(),
        req
    );
    for (k, v) in [
        ("approved", json!(true)),
        ("policy", json!("allow")),
        ("verdict", json!({"decision": "allow"})),
        ("host_action", json!("ls")),
    ] {
        let tampered = with_field(&wire, k, v);
        assert!(
            serde_json::from_value::<ActionRequest>(tampered).is_err(),
            "{k} accepted"
        );
    }
    // The embedded action is held to the same contract.
    let smuggled = with_field(
        &wire,
        "action",
        json!({"type": "wait", "duration_ms": 10, "then": "shell"}),
    );
    assert!(serde_json::from_value::<ActionRequest>(smuggled).is_err());
}

// --- properties ---

const FIELD_POOL: &[&str] = &[
    "type",
    "x",
    "y",
    "button",
    "from_x",
    "from_y",
    "to_x",
    "to_y",
    "duration_ms",
    "delta_x",
    "delta_y",
    "key",
    "keys",
    "text",
    "sensitive",
    "command",
    "action",
    "observe_after",
];

fn arb_tag() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::sample::select(SCHEMA.iter().map(|(t, _)| t.to_string()).collect::<Vec<_>>()),
        1 => "[a-z_]{0,14}",
        1 => any::<String>(),
    ]
}

fn arb_key() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::sample::select(FIELD_POOL.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        1 => "[a-zA-Z_]{0,10}",
        1 => any::<String>(),
    ]
}

fn arb_leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
        (0.0f64..=1.0).prop_map(Value::from),
        (0u32..40_000).prop_map(Value::from),
        prop::sample::select(vec![
            "primary",
            "secondary",
            "middle",
            "Enter",
            "Control",
            "c"
        ])
        .prop_map(Value::from),
        arb_tag().prop_map(Value::from),
        any::<String>().prop_map(Value::from),
    ]
}

fn arb_json() -> impl Strategy<Value = Value> {
    arb_leaf().prop_recursive(4, 48, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((arb_key(), inner), 0..8)
                .prop_map(|kv| Value::Object(kv.into_iter().collect())),
        ]
    })
}

/// Objects shaped like actions: a (mostly) real tag plus fields drawn
/// from the vocabulary and a few foreign names.
fn arb_action_like() -> impl Strategy<Value = Value> {
    (
        arb_tag(),
        prop::collection::vec((arb_key(), arb_json()), 0..7),
    )
        .prop_map(|(tag, fields)| {
            let mut map: serde_json::Map<String, Value> = fields.into_iter().collect();
            map.insert("type".into(), Value::from(tag));
            Value::Object(map)
        })
}

/// If anything parses, it was an object with a known tag and only that
/// variant's fields, and it serializes back to an equal action.
fn check_accepted_shape(input: &Value, parsed: &ComputerAction) -> Result<(), TestCaseError> {
    let obj = input
        .as_object()
        .ok_or_else(|| TestCaseError::fail(format!("non-object parsed: {input}")))?;
    let tag = obj.get("type").and_then(Value::as_str).unwrap_or("");
    let allowed = allowed_fields(tag)
        .ok_or_else(|| TestCaseError::fail(format!("unknown tag parsed: {input}")))?;
    for k in obj.keys() {
        prop_assert!(
            k == "type" || allowed.contains(&k.as_str()),
            "field {k:?} accepted: {input}"
        );
    }
    let back = serde_json::to_value(parsed).unwrap();
    prop_assert_eq!(&parse_value(&back).unwrap(), parsed);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_json_never_panics_and_only_exact_shapes_parse(v in arb_json()) {
        if let Ok(a) = parse_value(&v) {
            check_accepted_shape(&v, &a)?;
        }
        let _ = parse_text(&v.to_string());
    }

    #[test]
    fn action_like_objects_only_parse_with_their_own_fields(v in arb_action_like()) {
        let by_value = parse_value(&v);
        let by_text = parse_text(&v.to_string());
        // Same verdict on both paths (values may differ by one float ULP:
        // serde_json's default text parser is not correctly rounded).
        prop_assert_eq!(by_value.is_ok(), by_text.is_ok(), "paths disagree: {}", v);
        if let Ok(a) = by_value {
            check_accepted_shape(&v, &a)?;
        }
    }

    #[test]
    fn arbitrary_text_never_panics(s in any::<String>()) {
        let _ = parse_text(&s);
        let _ = serde_json::from_str::<ActionRequest>(&s);
    }

    #[test]
    fn duplicate_keys_in_text_never_parse(
        tag in prop::sample::select(SCHEMA.iter().map(|(t, _)| *t).collect::<Vec<_>>()),
        key in prop::sample::select(FIELD_POOL.to_vec()),
        a in arb_leaf(),
        b in arb_leaf(),
    ) {
        let text = format!(
            r#"{{"type":{},{}:{},{}:{}}}"#,
            Value::from(tag),
            Value::from(key),
            a,
            Value::from(key),
            b
        );
        prop_assert!(parse_text(&text).is_err(), "parsed: {}", text);
    }
}

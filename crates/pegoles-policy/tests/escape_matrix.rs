//! Release security matrix for Pegoles Guard: deterministic rows of
//! hostile input, each either unparseable at the wire or denied by
//! `evaluate`. Nothing here may be allowed, panic, or come back
//! `RequireApproval`. Allowed boundary rows pin the edges from the other
//! side so the caps stay exact rather than merely strict.

use pegoles_policy::{evaluate, PolicyContext};
use pegoles_protocol::{
    is_invisible_format, limits, ActionRequest, ComputerAction, ComputerId, Decision,
    PointerButton, PolicyVerdict, RiskLevel, TaskId,
};

fn verdict(action: ComputerAction) -> PolicyVerdict {
    let req = ActionRequest::new(TaskId::new(), ComputerId::new(), action);
    evaluate(&req, &PolicyContext::default())
}

#[track_caller]
fn assert_denied(action: ComputerAction) {
    let label = format!("{action:?}");
    let v = verdict(action);
    assert_eq!(v.decision, Decision::Deny, "allowed: {label:.200}");
    assert_eq!(v.risk, RiskLevel::Blocked, "{label:.200}");
    assert_reason_is_safe(&v);
}

#[track_caller]
fn assert_allowed(action: ComputerAction) {
    let label = format!("{action:?}");
    let v = verdict(action);
    assert_eq!(
        v.decision,
        Decision::Allow,
        "denied ({}): {label:.200}",
        v.reason
    );
    assert_eq!(v.risk, RiskLevel::Low, "{label:.200}");
}

/// Verdict reasons reach events and the UI: bounded, no raw hostile text.
#[track_caller]
fn assert_reason_is_safe(v: &PolicyVerdict) {
    assert!(v.reason.chars().count() <= 300, "reason too long");
    assert!(
        !v.reason
            .chars()
            .any(|c| c.is_control() || is_invisible_format(c)),
        "reason carries hidden characters: {:?}",
        v.reason
    );
}

const BUTTONS: [PointerButton; 3] = [
    PointerButton::Primary,
    PointerButton::Secondary,
    PointerButton::Middle,
];

/// Coordinates that must never reach the guest.
fn bad_coords() -> Vec<f64> {
    vec![
        f64::NAN,
        -f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        1e308,
        -1e308,
        -f64::MIN_POSITIVE,
        -1e-300,
        -0.000_001,
        1.0 + f64::EPSILON,
        1.000_001,
        2.0,
        -1.0,
        4096.0,
        1440.0,
        u32::MAX as f64,
    ]
}

/// In-range edges that must stay usable.
fn good_coords() -> Vec<f64> {
    vec![
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        1e-300,
        0.5,
        1.0 - f64::EPSILON,
        1.0,
    ]
}

/// Every pointer-positioned variant at (x, y).
fn pointer_actions(x: f64, y: f64) -> Vec<ComputerAction> {
    let mut out = vec![
        ComputerAction::MovePointer { x, y },
        ComputerAction::Scroll {
            x,
            y,
            delta_x: 0.0,
            delta_y: 1.0,
        },
    ];
    for button in BUTTONS {
        out.extend([
            ComputerAction::Click { x, y, button },
            ComputerAction::DoubleClick { x, y, button },
            ComputerAction::MouseDown { x, y, button },
            ComputerAction::MouseUp { x, y, button },
        ]);
    }
    out
}

fn drag(from: (f64, f64), to: (f64, f64), duration_ms: u32) -> ComputerAction {
    ComputerAction::Drag {
        from_x: from.0,
        from_y: from.1,
        to_x: to.0,
        to_y: to.1,
        button: PointerButton::Primary,
        duration_ms,
    }
}

fn type_text(text: impl Into<String>) -> ComputerAction {
    ComputerAction::TypeText {
        text: text.into(),
        sensitive: false,
    }
}

fn chord(keys: &[&str]) -> ComputerAction {
    ComputerAction::KeyChord {
        keys: keys.iter().map(|k| k.to_string()).collect(),
    }
}

// --- unsupported / unknown actions ---

#[test]
fn unknown_actions_never_reach_policy() {
    // The typed vocabulary cannot express them; the wire must refuse them
    // instead of mapping them onto something policy would allow.
    for json in [
        r#"{"type":"shell","command":"ls"}"#,
        r#"{"type":"run_command","command":"ls"}"#,
        r#"{"type":"open_url","url":"http://example.com"}"#,
        r#"{"type":"read_file","path":"/etc/passwd"}"#,
        r#"{"type":"write_file","path":"/tmp/x","content":"x"}"#,
        r#"{"type":"host_shell","command":"id"}"#,
        r#"{"type":"clipboard_set","text":"x"}"#,
        r#"{"type":"screenshot"}"#,
        r#"{"type":"type","text":"x"}"#,
        r#"{"type":"observe_screen","command":"ls"}"#,
        r#"{"type":"click","x":0.5,"y":0.5,"then":{"type":"shell"}}"#,
        r#"{"type":"type_text","text":"a","host":true}"#,
        r#"["click",0.5,0.5]"#,
        r#""observe_screen""#,
        r#"{"click":{"x":0.5,"y":0.5}}"#,
        r#"{"type":"click","x":0.1,"x":0.5,"y":0.5}"#,
    ] {
        assert!(
            serde_json::from_str::<ComputerAction>(json).is_err(),
            "parsed: {json}"
        );
    }
}

// --- pointer coordinates ---

#[test]
fn pointer_actions_deny_every_bad_coordinate() {
    for bad in bad_coords() {
        for good in good_coords() {
            for action in pointer_actions(bad, good)
                .into_iter()
                .chain(pointer_actions(good, bad))
            {
                assert_denied(action);
            }
        }
        for action in pointer_actions(bad, bad) {
            assert_denied(action);
        }
    }
}

#[test]
fn pointer_actions_allow_unit_square_edges() {
    for x in good_coords() {
        for y in good_coords() {
            for action in pointer_actions(x, y) {
                assert_allowed(action);
            }
        }
    }
}

// --- drag ---

#[test]
fn drag_denies_any_bad_endpoint() {
    let ok = (0.25, 0.75);
    for bad in bad_coords() {
        for (from, to) in [
            ((bad, ok.1), ok),
            ((ok.0, bad), ok),
            (ok, (bad, ok.1)),
            (ok, (ok.0, bad)),
            ((bad, bad), (bad, bad)),
        ] {
            assert_denied(drag(from, to, 400));
        }
    }
}

#[test]
fn drag_duration_is_capped() {
    let (a, b) = ((0.1, 0.1), (0.9, 0.9));
    assert_allowed(drag(a, b, 0));
    assert_allowed(drag(a, b, limits::MAX_DRAG_MS));
    for d in [limits::MAX_DRAG_MS + 1, 60_000, u32::MAX] {
        assert_denied(drag(a, b, d));
    }
    for button in BUTTONS {
        assert_denied(ComputerAction::Drag {
            from_x: f64::NAN,
            from_y: 0.5,
            to_x: 0.5,
            to_y: 0.5,
            button,
            duration_ms: 10,
        });
    }
}

// --- scroll ---

#[test]
fn scroll_deltas_are_finite_and_capped() {
    let cap = limits::MAX_SCROLL_UNITS;
    let bad = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        cap + 1e-9,
        -(cap + 1e-9),
        cap * 2.0,
        1e308,
    ];
    for d in bad {
        for (dx, dy) in [(d, 0.0), (0.0, d), (d, d)] {
            assert_denied(ComputerAction::Scroll {
                x: 0.5,
                y: 0.5,
                delta_x: dx,
                delta_y: dy,
            });
        }
    }
    for d in [0.0, -0.0, 0.4, -3.0, cap, -cap] {
        assert_allowed(ComputerAction::Scroll {
            x: 0.5,
            y: 0.5,
            delta_x: d,
            delta_y: -d,
        });
    }
}

// --- wait ---

#[test]
fn waits_over_cap_deny() {
    assert_allowed(ComputerAction::Wait { duration_ms: 0 });
    assert_allowed(ComputerAction::Wait {
        duration_ms: limits::MAX_WAIT_MS,
    });
    for d in [limits::MAX_WAIT_MS + 1, 120_000, u32::MAX] {
        assert_denied(ComputerAction::Wait { duration_ms: d });
    }
}

// --- typed text ---

#[test]
fn typed_text_is_capped_in_characters_not_bytes() {
    let cap = limits::MAX_TYPE_CHARS;
    assert_allowed(type_text("x".repeat(cap)));
    // Two bytes per char: the cap counts characters.
    assert_allowed(type_text("é".repeat(cap)));
    assert_denied(type_text("x".repeat(cap + 1)));
    assert_denied(type_text("é".repeat(cap + 1)));
    assert_denied(type_text("x".repeat(1 << 20)));
    assert_allowed(type_text(""));
}

#[test]
fn typed_text_denies_every_c0_and_c1_control_but_newline_and_tab() {
    for cp in (0x00u32..=0x1F).chain([0x7F]).chain(0x80..=0x9F) {
        let c = char::from_u32(cp).unwrap();
        if c == '\n' || c == '\t' {
            continue;
        }
        for text in [
            c.to_string(),
            format!("ls{c}"),
            format!("{c}ls"),
            format!("l{c}s"),
        ] {
            assert_denied(type_text(text));
        }
    }
    assert_allowed(type_text("line one\nline two\tcol"));
}

#[test]
fn typed_text_denies_bidi_zero_width_and_format_characters() {
    let hidden = [
        // Bidi embeddings, overrides, isolates and marks.
        '\u{202A}',
        '\u{202B}',
        '\u{202C}',
        '\u{202D}',
        '\u{202E}',
        '\u{2066}',
        '\u{2067}',
        '\u{2068}',
        '\u{2069}',
        '\u{200E}',
        '\u{200F}',
        '\u{061C}',
        // Zero width, word joiner, invisible operators, BOM.
        '\u{200B}',
        '\u{200C}',
        '\u{200D}',
        '\u{2060}',
        '\u{2061}',
        '\u{2062}',
        '\u{2063}',
        '\u{2064}',
        '\u{FEFF}',
        // Other Cf.
        '\u{00AD}',
        '\u{0600}',
        '\u{0605}',
        '\u{06DD}',
        '\u{070F}',
        '\u{0890}',
        '\u{08E2}',
        '\u{180E}',
        '\u{206A}',
        '\u{206F}',
        '\u{FFF9}',
        '\u{FFFB}',
        '\u{110BD}',
        '\u{110CD}',
        '\u{13430}',
        '\u{1343F}',
        '\u{1BCA0}',
        '\u{1D173}',
        '\u{1D17A}',
        '\u{E0001}',
        '\u{E0020}',
        '\u{E007F}',
        // Default-ignorable fillers/selectors and line separators.
        '\u{034F}',
        '\u{115F}',
        '\u{3164}',
        '\u{FE0F}',
        '\u{E0100}',
        '\u{2028}',
        '\u{2029}',
    ];
    for c in hidden {
        for text in [c.to_string(), format!("echo ok{c}"), format!("{c}echo ok")] {
            assert_denied(type_text(text.clone()));
            assert_denied(ComputerAction::TypeText {
                text,
                sensitive: true,
            });
        }
    }
    // The classic spoof: reads "exe.txt", is "txt.exe".
    assert_denied(type_text("open invoice\u{202E}txt.exe"));
}

#[test]
fn every_hidden_code_point_denies() {
    // Full sweep of Unicode scalar values: whatever the shared classifier
    // or `char::is_control` flags, the policy refuses to type.
    let hidden = (0..=0x10_FFFFu32)
        .filter_map(char::from_u32)
        .filter(|&c| is_invisible_format(c) || (c.is_control() && c != '\n' && c != '\t'));
    let mut n = 0;
    for c in hidden {
        assert_denied(type_text(format!("ok{c}ok")));
        n += 1;
    }
    assert!(n > 4_000, "sweep covered {n} code points");
}

#[test]
fn typed_key_material_denies_even_when_disguised() {
    for text in [
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----begin openssh private key-----",
        "AWS_SECRET_ACCESS_KEY=x",
        "sk-ant-api03-abc",
        // Invisible splitters cannot dodge the tripwire.
        "PRIVATE\u{200B}KEY-----",
        "sk\u{2060}-ant-api03-abc",
    ] {
        assert_denied(type_text(text));
    }
    assert_allowed(type_text("echo \"Pegoles is alive\" ção 漢字 🙂"));
}

// --- keys ---

#[test]
fn unknown_key_names_deny() {
    let huge = "A".repeat(1 << 20);
    for key in [
        "",
        " ",
        "Hyper",
        "F13",
        "F0",
        "Fn",
        "CapsLock",
        "Ctrl+C",
        "Control c",
        "ab",
        "é",
        "\u{0}",
        "\u{7f}",
        "\n",
        "Enter\n",
        "\tTab",
        "\u{85}Enter",
        "\u{A0}Enter",
        "Enter\u{2028}",
        "\u{200B}Enter",
        "Ent\u{200D}er",
        "\u{202E}retnE",
        "\u{FEFF}Tab",
        "BAC\u{212A}SPACE",
        huge.as_str(),
    ] {
        assert_denied(ComputerAction::KeyPress { key: key.into() });
        assert_denied(chord(&["Control", key]));
    }
}

#[test]
fn known_key_names_and_aliases_allow() {
    for key in [
        "Enter",
        "enter",
        "RETURN",
        " Return ",
        "Esc",
        "Tab",
        "Backspace",
        "Delete",
        "Space",
        "ArrowUp",
        "left",
        "PageDown",
        "pgup",
        "Home",
        "End",
        "F1",
        "f12",
        "a",
        "Z",
        "0",
        "+",
        "/",
    ] {
        assert_allowed(ComputerAction::KeyPress { key: key.into() });
    }
}

#[test]
fn chords_deny_empty_oversized_modifier_only_and_duplicates() {
    // Empty and oversized.
    assert_denied(chord(&[]));
    assert_denied(chord(&["Control", "Alt", "Shift", "Meta", "c"]));
    assert_denied(ComputerAction::KeyChord {
        keys: vec!["c".to_string(); 10_000],
    });
    // Modifier-only (with aliases).
    for keys in [
        &["Shift"][..],
        &["Control"],
        &["cmd"],
        &["Control", "Alt"],
        &["ctrl", "option", "shift", "super"],
        &["Meta", "Shift", "Control", "Alt"],
    ] {
        assert_denied(chord(keys));
    }
    // Duplicates, directly or through aliases.
    for keys in [
        &["Control", "Control", "c"][..],
        &["Control", "ctrl", "c"],
        &["cmd", "Meta", "q"],
        &["Return", "Enter"],
        &["c", "c"],
        &["Alt", "option", "Tab"],
    ] {
        assert_denied(chord(keys));
    }
    // Any unknown member poisons the chord.
    for keys in [
        &["Control", "Hyper"][..],
        &["Control", ""],
        &["", "c"],
        &["Control", "c\u{200B}"],
        &["Control\u{202E}", "c"],
    ] {
        assert_denied(chord(keys));
    }
}

#[test]
fn valid_chords_allow() {
    for keys in [
        &["Control", "c"][..],
        &["cmd", "shift", "z"],
        &["Enter"],
        &["Alt", "Tab"],
        &["Control", "Alt", "Shift", "Delete"],
        &["Meta", "Space"],
    ] {
        assert_allowed(chord(keys));
    }
}

// --- verdict shape ---

#[test]
fn no_row_requires_approval_and_reasons_stay_bounded() {
    let hostile_key = format!("\u{202E}{}\u{85}", "k".repeat(100_000));
    for action in [
        ComputerAction::KeyChord {
            keys: vec!["Control".into(), hostile_key.clone()],
        },
        ComputerAction::KeyPress {
            key: hostile_key.clone(),
        },
        type_text(hostile_key),
        ComputerAction::MovePointer {
            x: f64::NAN,
            y: 0.5,
        },
        ComputerAction::Wait {
            duration_ms: u32::MAX,
        },
    ] {
        let v = verdict(action);
        assert_eq!(v.decision, Decision::Deny);
        assert_reason_is_safe(&v);
    }
}

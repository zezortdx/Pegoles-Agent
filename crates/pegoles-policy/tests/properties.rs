//! Property tests for Pegoles Guard: for ANY action a planner can build,
//! `evaluate` returns (never panics), is deterministic, never answers
//! `RequireApproval`, and never allows input outside the release bounds.
//! The bounds below are restated independently of the engine on purpose.

use pegoles_policy::{evaluate, PolicyContext};
use pegoles_protocol::keys::{MODIFIERS, NAMED_KEYS};
use pegoles_protocol::{
    is_invisible_format, limits, normalize_key_name, ActionRequest, ComputerAction, ComputerId,
    Decision, PointerButton, RiskLevel, TaskId,
};
use proptest::prelude::*;

fn in_unit(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}

fn key_ok(key: &str) -> bool {
    key.is_ascii()
        && normalize_key_name(key).is_some_and(|n| {
            MODIFIERS.contains(&n.as_str())
                || NAMED_KEYS.contains(&n.as_str())
                || (n.len() == 1 && n.chars().all(|c| c.is_ascii_graphic()))
        })
}

fn text_ok(text: &str) -> bool {
    text.chars().count() <= limits::MAX_TYPE_CHARS
        && !text
            .chars()
            .any(|c| (c.is_control() && c != '\n' && c != '\t') || is_invisible_format(c))
}

/// What an allowed action must satisfy. Allow ⇒ within_bounds.
fn within_bounds(a: &ComputerAction) -> bool {
    match a {
        ComputerAction::ObserveScreen | ComputerAction::GetDisplayInfo => true,
        ComputerAction::MovePointer { x, y }
        | ComputerAction::Click { x, y, .. }
        | ComputerAction::DoubleClick { x, y, .. }
        | ComputerAction::MouseDown { x, y, .. }
        | ComputerAction::MouseUp { x, y, .. } => in_unit(*x) && in_unit(*y),
        ComputerAction::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            duration_ms,
            ..
        } => {
            in_unit(*from_x)
                && in_unit(*from_y)
                && in_unit(*to_x)
                && in_unit(*to_y)
                && *duration_ms <= limits::MAX_DRAG_MS
        }
        ComputerAction::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => {
            in_unit(*x)
                && in_unit(*y)
                && delta_x.is_finite()
                && delta_y.is_finite()
                && delta_x.abs() <= limits::MAX_SCROLL_UNITS
                && delta_y.abs() <= limits::MAX_SCROLL_UNITS
        }
        ComputerAction::KeyPress { key } => key_ok(key),
        ComputerAction::KeyChord { keys } => {
            (1..=4).contains(&keys.len())
                && keys.iter().all(|k| key_ok(k))
                && keys.iter().any(|k| !pegoles_protocol::is_modifier(k))
        }
        ComputerAction::TypeText { text, .. } => text_ok(text),
        ComputerAction::Wait { duration_ms } => *duration_ms <= limits::MAX_WAIT_MS,
    }
}

/// Variants judged on numbers alone: allowed exactly when in bounds.
fn numeric_only(a: &ComputerAction) -> bool {
    !matches!(
        a,
        ComputerAction::KeyPress { .. }
            | ComputerAction::KeyChord { .. }
            | ComputerAction::TypeText { .. }
    )
}

fn hostile_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        prop::sample::select(vec![
            f64::NAN,
            -f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::EPSILON,
            0.0,
            -0.0,
            1.0,
            1.0 + f64::EPSILON,
            1.0 - f64::EPSILON,
            limits::MAX_SCROLL_UNITS,
            -limits::MAX_SCROLL_UNITS,
            limits::MAX_SCROLL_UNITS + 1e-9,
        ]),
        0.0f64..=1.0,
        -2.0f64..3.0,
        -200.0f64..200.0,
        any::<f64>(),
    ]
}

fn hostile_u32() -> impl Strategy<Value = u32> {
    prop_oneof![
        prop::sample::select(vec![
            0,
            1,
            limits::MAX_DRAG_MS,
            limits::MAX_DRAG_MS + 1,
            limits::MAX_WAIT_MS,
            limits::MAX_WAIT_MS + 1,
            u32::MAX,
        ]),
        0u32..40_000,
        any::<u32>(),
    ]
}

/// Characters an attacker reaches for, mixed into ordinary text.
fn hostile_char() -> impl Strategy<Value = char> {
    prop_oneof![
        prop::sample::select(vec![
            '\u{0}',
            '\u{7}',
            '\u{1b}',
            '\r',
            '\n',
            '\t',
            '\u{7f}',
            '\u{85}',
            '\u{9b}',
            '\u{a0}',
            '\u{ad}',
            '\u{200b}',
            '\u{200d}',
            '\u{200e}',
            '\u{202e}',
            '\u{2066}',
            '\u{2069}',
            '\u{2028}',
            '\u{2062}',
            '\u{feff}',
            '\u{e0041}',
            '\u{fe0f}',
            '\u{3164}',
            '\u{212a}',
        ]),
        any::<char>(),
        prop::char::range(' ', '~'),
    ]
}

fn hostile_text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::collection::vec(hostile_char(), 0..48).prop_map(|v| v.into_iter().collect()),
        2 => "[ -~]{0,64}",
        1 => any::<String>(),
        // Around the cap.
        1 => (limits::MAX_TYPE_CHARS - 2..=limits::MAX_TYPE_CHARS + 2, any::<bool>())
            .prop_map(|(n, wide)| if wide { "é".repeat(n) } else { "x".repeat(n) }),
    ]
}

fn hostile_key() -> impl Strategy<Value = String> {
    let names: Vec<String> = MODIFIERS
        .iter()
        .chain(NAMED_KEYS)
        .map(|s| s.to_string())
        .chain(
            [
                "return", "esc", "ctrl", "cmd", "option", "a", "Z", "+", " ", "",
            ]
            .map(String::from),
        )
        .collect();
    prop_oneof![
        4 => prop::sample::select(names.clone()),
        // Known names with case games, padding and one hostile char.
        2 => (prop::sample::select(names), any::<bool>(), hostile_char(), 0usize..3)
            .prop_map(|(n, upper, c, pos)| {
                let n = if upper { n.to_uppercase() } else { n };
                match pos {
                    0 => format!("{c}{n}"),
                    1 => format!("{n}{c}"),
                    _ => format!(" {n} "),
                }
            }),
        1 => hostile_text(),
    ]
}

fn button() -> impl Strategy<Value = PointerButton> {
    prop::sample::select(vec![
        PointerButton::Primary,
        PointerButton::Secondary,
        PointerButton::Middle,
    ])
}

fn any_action() -> impl Strategy<Value = ComputerAction> {
    let f = hostile_f64;
    prop_oneof![
        Just(ComputerAction::ObserveScreen),
        Just(ComputerAction::GetDisplayInfo),
        (f(), f()).prop_map(|(x, y)| ComputerAction::MovePointer { x, y }),
        (f(), f(), button()).prop_map(|(x, y, button)| ComputerAction::Click { x, y, button }),
        (f(), f(), button()).prop_map(|(x, y, button)| ComputerAction::DoubleClick {
            x,
            y,
            button
        }),
        (f(), f(), button()).prop_map(|(x, y, button)| ComputerAction::MouseDown { x, y, button }),
        (f(), f(), button()).prop_map(|(x, y, button)| ComputerAction::MouseUp { x, y, button }),
        (f(), f(), f(), f(), button(), hostile_u32()).prop_map(
            |(from_x, from_y, to_x, to_y, button, duration_ms)| ComputerAction::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                duration_ms,
            }
        ),
        (f(), f(), f(), f()).prop_map(|(x, y, delta_x, delta_y)| ComputerAction::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        }),
        hostile_key().prop_map(|key| ComputerAction::KeyPress { key }),
        prop::collection::vec(hostile_key(), 0..7)
            .prop_map(|keys| ComputerAction::KeyChord { keys }),
        (hostile_text(), any::<bool>())
            .prop_map(|(text, sensitive)| ComputerAction::TypeText { text, sensitive }),
        hostile_u32().prop_map(|duration_ms| ComputerAction::Wait { duration_ms }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn evaluate_never_allows_out_of_bounds(action in any_action()) {
        let req = ActionRequest::new(TaskId::new(), ComputerId::new(), action.clone());
        let v = evaluate(&req, &PolicyContext::default());

        prop_assert_ne!(v.decision, Decision::RequireApproval);
        match v.decision {
            Decision::Allow => {
                prop_assert_eq!(v.risk, RiskLevel::Low);
                prop_assert!(within_bounds(&action), "allowed out of bounds: {:.300}", format!("{action:?}"));
            }
            Decision::Deny => prop_assert_eq!(v.risk, RiskLevel::Blocked),
            Decision::RequireApproval => unreachable!(),
        }
        if numeric_only(&action) {
            // Exact for numeric rules: no over-denial either.
            prop_assert_eq!(v.decision == Decision::Allow, within_bounds(&action), "{:?}", action);
        }
        // Deterministic: same request, same verdict.
        prop_assert_eq!(evaluate(&req, &PolicyContext::default()), v.clone());
        // Reasons stay bounded and free of hidden characters.
        prop_assert!(v.reason.chars().count() <= 300);
        prop_assert!(!v.reason.chars().any(|c| c.is_control() || is_invisible_format(c)), "{:?}", v.reason);
    }

    #[test]
    fn pointer_allows_exactly_the_unit_square(x in hostile_f64(), y in hostile_f64()) {
        let req = ActionRequest::new(TaskId::new(), ComputerId::new(), ComputerAction::MovePointer { x, y });
        let v = evaluate(&req, &PolicyContext::default());
        prop_assert_eq!(v.decision == Decision::Allow, in_unit(x) && in_unit(y));
    }
}

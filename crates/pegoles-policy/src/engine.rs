use pegoles_protocol::{limits, ActionRequest, ComputerAction, Decision, PolicyVerdict, RiskLevel};

/// Normalized agent coordinates must stay inside the unit square.
fn in_unit_square(x: f64, y: f64) -> bool {
    x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)
}

/// Task-scoped evaluation context. Empty today: every action in the
/// vocabulary is judged on its own shape. Future grants (e.g. a task that
/// may use the network) belong here, set by Core, never by a model.
#[derive(Clone, Debug, Default)]
pub struct PolicyContext {}

/// Deterministic policy entry point. The match is exhaustive: a new
/// action variant does not compile until it gets a rule here.
pub fn evaluate(req: &ActionRequest, _ctx: &PolicyContext) -> PolicyVerdict {
    use pegoles_protocol::keys::{normalize_key_name, validate_chord};
    match &req.action {
        ComputerAction::ObserveScreen => allow("observing the VM screen"),
        ComputerAction::GetDisplayInfo => allow("display size probe inside VM"),
        ComputerAction::MovePointer { x, y } => pointer(*x, *y, "pointer move inside VM"),
        ComputerAction::Click { x, y, .. }
        | ComputerAction::DoubleClick { x, y, .. }
        | ComputerAction::MouseDown { x, y, .. }
        | ComputerAction::MouseUp { x, y, .. } => pointer(*x, *y, "click inside VM"),
        ComputerAction::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            duration_ms,
            ..
        } => {
            if !in_unit_square(*from_x, *from_y) || !in_unit_square(*to_x, *to_y) {
                deny("drag coordinates outside 0.0..=1.0")
            } else if *duration_ms > limits::MAX_DRAG_MS {
                deny("drag duration exceeds safety cap")
            } else {
                allow("drag inside VM")
            }
        }
        ComputerAction::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => {
            if !in_unit_square(*x, *y) {
                deny("scroll coordinates outside 0.0..=1.0")
            } else if !delta_x.is_finite()
                || !delta_y.is_finite()
                || delta_x.abs() > limits::MAX_SCROLL_UNITS
                || delta_y.abs() > limits::MAX_SCROLL_UNITS
            {
                deny("scroll delta exceeds safety cap")
            } else {
                allow("scroll inside VM")
            }
        }
        ComputerAction::KeyPress { key } => match normalize_key_name(key) {
            Some(_) => allow("key press inside VM"),
            None => deny("unknown key name"),
        },
        ComputerAction::KeyChord { keys } => match validate_chord(keys) {
            Ok(_) => allow("key chord inside VM"),
            Err(reason) => deny(&reason),
        },
        ComputerAction::TypeText { text, .. } => {
            if text.chars().count() > limits::MAX_TYPE_CHARS {
                deny("typed text exceeds safety cap")
            } else if text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                deny("typed text contains control characters")
            } else if looks_like_private_key_material(text) {
                deny("refusing to type key material")
            } else {
                allow("typing inside VM")
            }
        }
        ComputerAction::Wait { duration_ms } => {
            if *duration_ms > limits::MAX_WAIT_MS {
                deny("wait exceeds safety cap")
            } else {
                allow("wait inside VM sequence")
            }
        }
    }
}

// --- helpers ---

fn pointer(x: f64, y: f64, reason: &str) -> PolicyVerdict {
    if in_unit_square(x, y) {
        allow(reason)
    } else {
        deny("pointer coordinates outside 0.0..=1.0")
    }
}

fn allow(reason: &str) -> PolicyVerdict {
    PolicyVerdict {
        decision: Decision::Allow,
        risk: RiskLevel::Low,
        reason: reason.to_string(),
    }
}

fn deny(reason: &str) -> PolicyVerdict {
    PolicyVerdict {
        decision: Decision::Deny,
        risk: RiskLevel::Blocked,
        reason: reason.to_string(),
    }
}

/// Tripwire, not a boundary: obvious private-key / cloud-credential
/// material is never typed. The real protection is that the VM has no
/// network and nothing typed can leave it.
fn looks_like_private_key_material(s: &str) -> bool {
    let u = s.to_uppercase();
    u.contains("PRIVATE KEY-----") || u.contains("AWS_SECRET_ACCESS_KEY") || u.contains("SK-ANT-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pegoles_protocol::{ComputerId, TaskId};

    fn req(action: ComputerAction) -> ActionRequest {
        ActionRequest::new(TaskId::new(), ComputerId::new(), action)
    }

    fn ctx() -> PolicyContext {
        PolicyContext::default()
    }

    #[test]
    fn observe_allows() {
        let v = evaluate(&req(ComputerAction::ObserveScreen), &ctx());
        assert_eq!(v.decision, Decision::Allow);
        assert_eq!(v.risk, RiskLevel::Low);
    }

    #[test]
    fn click_allows() {
        use pegoles_protocol::PointerButton;
        let v = evaluate(
            &req(ComputerAction::Click {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::Allow);
    }

    #[test]
    fn out_of_range_pointer_denies() {
        use pegoles_protocol::PointerButton;
        let cases = vec![
            ComputerAction::MovePointer { x: 1.5, y: 0.5 },
            ComputerAction::Click {
                x: f64::NAN,
                y: 0.5,
                button: PointerButton::Primary,
            },
            ComputerAction::Scroll {
                x: 0.5,
                y: 0.5,
                delta_x: 0.0,
                delta_y: 500.0,
            },
            ComputerAction::Drag {
                from_x: 0.1,
                from_y: 0.1,
                to_x: 0.9,
                to_y: 0.9,
                button: PointerButton::Primary,
                duration_ms: 60_000,
            },
            ComputerAction::KeyPress {
                key: "Hyper".into(),
            },
            ComputerAction::KeyChord {
                keys: vec!["Shift".into()],
            },
            ComputerAction::TypeText {
                text: "x".repeat(5000),
                sensitive: false,
            },
            ComputerAction::Wait {
                duration_ms: 120_000,
            },
        ];
        for action in &cases {
            let v = evaluate(&req(action.clone()), &ctx());
            assert_eq!(v.decision, Decision::Deny, "action: {action:?}");
        }
    }

    #[test]
    fn input_vocabulary_allows() {
        use pegoles_protocol::PointerButton;
        let cases = vec![
            ComputerAction::ObserveScreen,
            ComputerAction::GetDisplayInfo,
            ComputerAction::MovePointer { x: 0.0, y: 1.0 },
            ComputerAction::DoubleClick {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Secondary,
            },
            ComputerAction::MouseDown {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            },
            ComputerAction::MouseUp {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            },
            ComputerAction::Drag {
                from_x: 0.1,
                from_y: 0.1,
                to_x: 0.2,
                to_y: 0.2,
                button: PointerButton::Primary,
                duration_ms: 400,
            },
            ComputerAction::Scroll {
                x: 0.5,
                y: 0.5,
                delta_x: 0.0,
                delta_y: 3.0,
            },
            ComputerAction::KeyPress {
                key: "Enter".into(),
            },
            ComputerAction::KeyPress {
                key: "Return".into(),
            },
            ComputerAction::KeyChord {
                keys: vec!["Control".into(), "c".into()],
            },
            ComputerAction::TypeText {
                text: "echo \"Pegoles is alive\"".into(),
                sensitive: false,
            },
            ComputerAction::Wait { duration_ms: 500 },
        ];
        for action in cases {
            let v = evaluate(&req(action.clone()), &ctx());
            assert_eq!(v.decision, Decision::Allow, "action: {:?}", action);
        }
    }

    #[test]
    fn typing_key_material_and_control_chars_denies() {
        for text in [
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "export AWS_SECRET_ACCESS_KEY=abc",
            "sk-ant-api03-xyz",
            "ok\u{1b}[31m",
            "bell\u{7}",
        ] {
            let v = evaluate(
                &req(ComputerAction::TypeText {
                    text: text.into(),
                    sensitive: false,
                }),
                &ctx(),
            );
            assert_eq!(v.decision, Decision::Deny, "text: {text:?}");
        }
        let ok = evaluate(
            &req(ComputerAction::TypeText {
                text: "line one\nline two\tTAB unicode ção 漢字".into(),
                sensitive: false,
            }),
            &ctx(),
        );
        assert_eq!(ok.decision, Decision::Allow);
    }

    #[test]
    fn non_finite_scroll_deltas_deny() {
        for (dx, dy) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (f64::NEG_INFINITY, 1.0),
        ] {
            let v = evaluate(
                &req(ComputerAction::Scroll {
                    x: 0.5,
                    y: 0.5,
                    delta_x: dx,
                    delta_y: dy,
                }),
                &ctx(),
            );
            assert_eq!(v.decision, Decision::Deny);
        }
    }

    #[test]
    fn pointer_coordinates_sweep_matches_unit_square() {
        // Deterministic sweep incl. edges, negatives, and huge values.
        let samples = [
            -1e300, -1.0, -1e-9, 0.0, 1e-9, 0.25, 0.5, 0.999_999, 1.0, 1.000_001, 2.0, 1e300,
        ];
        for &x in &samples {
            for &y in &samples {
                let v = evaluate(&req(ComputerAction::MovePointer { x, y }), &ctx());
                let inside = (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y);
                let want = if inside {
                    Decision::Allow
                } else {
                    Decision::Deny
                };
                assert_eq!(v.decision, want, "({x}, {y})");
            }
        }
    }

    #[test]
    fn nothing_in_the_vocabulary_requires_approval_or_is_high_risk() {
        // Allowed actions are Low; denials are Blocked. RequireApproval is
        // reserved for future out-of-VM capabilities.
        use pegoles_protocol::PointerButton;
        let actions = [
            ComputerAction::ObserveScreen,
            ComputerAction::Click {
                x: 0.1,
                y: 0.1,
                button: PointerButton::Primary,
            },
            ComputerAction::KeyPress { key: "Tab".into() },
            ComputerAction::Wait { duration_ms: 10 },
        ];
        for a in actions {
            let v = evaluate(&req(a), &ctx());
            assert_ne!(v.decision, Decision::RequireApproval);
            assert_eq!(v.risk, RiskLevel::Low);
        }
    }
}

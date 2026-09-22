use pegoles_protocol::{
    limits, ActionRequest, ComputerAction, Decision, PolicyVerdict, RiskLevel, VirtualPath,
};

/// Normalized agent coordinates must stay inside the unit square.
fn in_unit_square(x: f64, y: f64) -> bool {
    x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)
}

/// Evaluation context. `workspace_root` is a guest path.
#[derive(Clone, Debug)]
pub struct PolicyContext {
    pub workspace_root: VirtualPath,
}

impl Default for PolicyContext {
    fn default() -> Self {
        Self {
            workspace_root: VirtualPath::new("/home/pegoles/workspace"),
        }
    }
}

/// Deterministic policy entry point.
pub fn evaluate(req: &ActionRequest, ctx: &PolicyContext) -> PolicyVerdict {
    use pegoles_protocol::keys::{normalize_key_name, validate_chord};
    match &req.action {
        ComputerAction::Screenshot | ComputerAction::ObserveScreen => {
            allow(RiskLevel::Low, "screenshot inside VM is always safe")
        }
        ComputerAction::GetDisplayInfo => allow(RiskLevel::Low, "display size probe inside VM"),
        ComputerAction::MovePointer { x, y } => {
            if in_unit_square(*x, *y) {
                allow(RiskLevel::Low, "pointer move inside VM")
            } else {
                deny(RiskLevel::Blocked, "pointer coordinates outside 0.0..=1.0")
            }
        }
        ComputerAction::Click { x, y, .. }
        | ComputerAction::DoubleClick { x, y, .. }
        | ComputerAction::MouseDown { x, y, .. }
        | ComputerAction::MouseUp { x, y, .. } => {
            if in_unit_square(*x, *y) {
                allow(RiskLevel::Low, "click inside VM")
            } else {
                deny(RiskLevel::Blocked, "pointer coordinates outside 0.0..=1.0")
            }
        }
        ComputerAction::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            duration_ms,
            ..
        } => {
            if !in_unit_square(*from_x, *from_y) || !in_unit_square(*to_x, *to_y) {
                deny(RiskLevel::Blocked, "drag coordinates outside 0.0..=1.0")
            } else if *duration_ms > limits::MAX_DRAG_MS {
                deny(RiskLevel::Blocked, "drag duration exceeds safety cap")
            } else {
                allow(RiskLevel::Low, "drag inside VM")
            }
        }
        ComputerAction::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => {
            if !in_unit_square(*x, *y) {
                deny(RiskLevel::Blocked, "scroll coordinates outside 0.0..=1.0")
            } else if delta_x.abs() > limits::MAX_SCROLL_UNITS
                || delta_y.abs() > limits::MAX_SCROLL_UNITS
            {
                deny(RiskLevel::Blocked, "scroll delta exceeds safety cap")
            } else {
                allow(RiskLevel::Low, "scroll inside VM")
            }
        }
        ComputerAction::KeyPress { key } => match normalize_key_name(key) {
            Some(_) => allow(RiskLevel::Low, "key press inside VM"),
            None => deny(RiskLevel::Blocked, "unknown key name"),
        },
        ComputerAction::KeyChord { keys } => match validate_chord(keys) {
            Ok(_) => allow(RiskLevel::Low, "key chord inside VM"),
            Err(reason) => deny(RiskLevel::Blocked, &reason),
        },
        ComputerAction::TypeText { text, .. } => {
            if text.chars().count() > limits::MAX_TYPE_CHARS {
                deny(RiskLevel::Blocked, "typed text exceeds safety cap")
            } else if looks_like_secret(text) {
                deny(
                    RiskLevel::Blocked,
                    "refusing to type a potential credential",
                )
            } else {
                allow(RiskLevel::Low, "typing inside VM")
            }
        }
        ComputerAction::Type { text } => {
            if looks_like_secret(text) {
                deny(
                    RiskLevel::Blocked,
                    "refusing to type a potential credential",
                )
            } else {
                allow(RiskLevel::Low, "typing inside VM")
            }
        }
        ComputerAction::Wait { duration_ms } => {
            if *duration_ms > limits::MAX_WAIT_MS {
                deny(RiskLevel::Blocked, "wait exceeds safety cap")
            } else {
                allow(RiskLevel::Low, "wait inside VM sequence")
            }
        }
        ComputerAction::OpenUrl { url } => {
            if is_http_url(url) {
                require_approval(
                    RiskLevel::Medium,
                    "network access from VM requires approval in Phase 1",
                )
            } else {
                deny(RiskLevel::Blocked, "non-http(s) URL scheme denied")
            }
        }
        ComputerAction::ReadFile { path } => {
            if is_host_path(path.as_str()) || looks_like_secret(path.as_str()) {
                deny(RiskLevel::Blocked, "host or credential path access denied")
            } else {
                allow(RiskLevel::Low, "guest file read")
            }
        }
        ComputerAction::WriteFile { path, .. } => {
            if is_host_path(path.as_str()) {
                deny(RiskLevel::Blocked, "host path write denied")
            } else if is_within_workspace(path.as_str(), ctx.workspace_root.as_str()) {
                allow(RiskLevel::Low, "write inside virtual workspace")
            } else {
                require_approval(RiskLevel::Medium, "write outside workspace needs approval")
            }
        }
        ComputerAction::Shell { command } => classify_shell(command),
    }
}

// --- helpers ---

fn allow(risk: RiskLevel, reason: &str) -> PolicyVerdict {
    PolicyVerdict {
        decision: Decision::Allow,
        risk,
        reason: reason.to_string(),
    }
}

fn deny(risk: RiskLevel, reason: &str) -> PolicyVerdict {
    PolicyVerdict {
        decision: Decision::Deny,
        risk,
        reason: reason.to_string(),
    }
}

fn require_approval(risk: RiskLevel, reason: &str) -> PolicyVerdict {
    PolicyVerdict {
        decision: Decision::RequireApproval,
        risk,
        reason: reason.to_string(),
    }
}

fn is_http_url(url: &str) -> bool {
    let u = url.trim().to_lowercase();
    u.starts_with("http://") || u.starts_with("https://")
}

/// Heuristic denylist for anything that smells like the host.
/// Conservative: deny first, refine with explicit Host Bridge later.
fn is_host_path(path: &str) -> bool {
    let p = path.trim().to_lowercase();
    p.starts_with("/users/")
        || p.starts_with("/etc/")
        || p.starts_with("/private/")
        || p.starts_with("/host")
        || p.starts_with("~/")
        || p.starts_with("~")
        || p.starts_with("c:\\")
        || p.starts_with("c:/")
        || p.contains("..")
        || p.contains("/.ssh/")
        || p.ends_with("/.ssh")
        || p.contains("\\.ssh\\")
}

fn looks_like_secret(s: &str) -> bool {
    let u = s.to_uppercase();
    u.contains("AWS_SECRET")
        || u.contains("AKIA")
        || u.contains("PRIVATE KEY")
        || u.contains(".PEM")
        || u.contains("GITHUB_TOKEN")
}

fn is_within_workspace(path: &str, workspace: &str) -> bool {
    let ws = workspace.trim_end_matches('/');
    path == ws || path.starts_with(&format!("{ws}/"))
}

fn shell_first_token(command: &str) -> String {
    let t = command.trim().trim_start_matches("sudo ").trim();
    t.split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string()
}

fn classify_shell(command: &str) -> PolicyVerdict {
    const SAFE: &[&str] = &[
        "ls",
        "pwd",
        "echo",
        "cat",
        "whoami",
        "date",
        "uname",
        "git",
        "python3",
        "python",
        "node",
        "npm",
        "lsb_release",
    ];
    const GATED: &[&str] = &[
        "curl",
        "wget",
        "ssh",
        "pip",
        "pip3",
        "apt",
        "apt-get",
        "npm",
        "npx",
        "chmod",
        "chown",
        "systemctl",
        "reboot",
        "shutdown",
    ];
    const BLOCKED_SUBSTR: &[&str] = &[
        "rm -rf",
        "rm -rf /",
        "mkfs",
        ":(){:|:&}",
        " dd ",
        "dd if=",
        "> /dev/sd",
        "/etc/passwd",
        "/etc/shadow",
        "~/.ssh",
    ];

    let lower = command.to_lowercase();
    for pat in BLOCKED_SUBSTR {
        if lower.contains(pat) {
            return deny(
                RiskLevel::Blocked,
                "destructive or credential-touching command denied",
            );
        }
    }
    if lower.contains("akexpress") || lower.contains("aws_secret_access_key") {
        return deny(RiskLevel::Blocked, "credential exfiltration pattern denied");
    }
    let tok = shell_first_token(command);
    if SAFE.contains(&tok.as_str()) {
        // `git` alone is safe-ish, but allow; network-y subcommands still pass
        // as Allow in Phase 1 unless destructive — documented limitation.
        return allow(RiskLevel::Low, "safe read-only/dev command inside VM");
    }
    if GATED.contains(&tok.as_str()) {
        return require_approval(
            RiskLevel::Medium,
            "network or privilege-adjacent command needs approval",
        );
    }
    require_approval(RiskLevel::High, "unknown command needs approval")
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
    fn screenshot_allows() {
        let v = evaluate(&req(ComputerAction::Screenshot), &ctx());
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
    fn write_inside_workspace_allows() {
        let v = evaluate(
            &req(ComputerAction::WriteFile {
                path: VirtualPath::new("/home/pegoles/workspace/notes.md"),
                content: "hi".into(),
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::Allow);
    }

    #[test]
    fn write_outside_workspace_gates() {
        let v = evaluate(
            &req(ComputerAction::WriteFile {
                path: VirtualPath::new("/tmp/tool-output.txt"),
                content: "hi".into(),
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::RequireApproval);
    }

    #[test]
    fn write_host_path_denies() {
        for p in [
            "/Users/alice/.ssh/id_rsa",
            "/etc/passwd",
            "C:\\Windows\\System32",
            "~/secrets",
            "/home/pegoles/workspace/../../etc/shadow",
        ] {
            let v = evaluate(
                &req(ComputerAction::WriteFile {
                    path: VirtualPath::new(p),
                    content: "x".into(),
                }),
                &ctx(),
            );
            assert_eq!(v.decision, Decision::Deny, "path: {p}");
            assert_eq!(v.risk, RiskLevel::Blocked);
        }
    }

    #[test]
    fn read_credential_path_denies() {
        let v = evaluate(
            &req(ComputerAction::ReadFile {
                path: VirtualPath::new("/home/pegoles/.ssh/id_rsa"),
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::Deny);
    }

    #[test]
    fn read_guest_file_allows() {
        let v = evaluate(
            &req(ComputerAction::ReadFile {
                path: VirtualPath::new("/home/pegoles/workspace/main.py"),
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::Allow);
    }

    #[test]
    fn safe_shell_allows() {
        for c in ["ls -la", "git status", "python3 --version", "echo hello"] {
            let v = evaluate(&req(ComputerAction::Shell { command: c.into() }), &ctx());
            assert_eq!(v.decision, Decision::Allow, "cmd: {c}");
        }
    }

    #[test]
    fn network_shell_gates() {
        for c in [
            "curl https://example.com",
            "apt-get install foo",
            "pip install bar",
        ] {
            let v = evaluate(&req(ComputerAction::Shell { command: c.into() }), &ctx());
            assert_eq!(v.decision, Decision::RequireApproval, "cmd: {c}");
        }
    }

    #[test]
    fn destructive_shell_denies() {
        for c in ["rm -rf /", "mkfs.ext4 /dev/sda1", "cat ~/.ssh/id_rsa"] {
            let v = evaluate(&req(ComputerAction::Shell { command: c.into() }), &ctx());
            assert_eq!(v.decision, Decision::Deny, "cmd: {c}");
            assert_eq!(v.risk, RiskLevel::Blocked);
        }
    }

    #[test]
    fn typing_secret_denies() {
        let v = evaluate(
            &req(ComputerAction::Type {
                text: "AWS_SECRET_ACCESS_KEY=xyz".into(),
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::Deny);
    }

    #[test]
    fn open_http_url_gates_and_other_schemes_deny() {
        let v = evaluate(
            &req(ComputerAction::OpenUrl {
                url: "https://example.com".into(),
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::RequireApproval);
        let v2 = evaluate(
            &req(ComputerAction::OpenUrl {
                url: "file:///etc/passwd".into(),
            }),
            &ctx(),
        );
        assert_eq!(v2.decision, Decision::Deny);
    }

    #[test]
    fn unknown_shell_requires_high_approval() {
        let v = evaluate(
            &req(ComputerAction::Shell {
                command: "terraform apply -auto-approve".into(),
            }),
            &ctx(),
        );
        assert_eq!(v.decision, Decision::RequireApproval);
        assert_eq!(v.risk, RiskLevel::High);
    }
}

//! The diagnostic report a person can send to whoever helps them when
//! setup or a task goes wrong ("Save a diagnostic report").
//!
//! It holds technical facts only: the app version and build, the system
//! check (OS, processor, memory, disk, virtualization, GPU), where setup
//! and onboarding stand, the backend and the computer's state, recent
//! failure *codes* and the guest's boot log tail. It never holds
//! screenshots, task titles or text, the agent's messages, typed text,
//! API keys, or anything read from the person's files. Every free-text
//! string passes through [`sanitize`] (home folder, user name, key- and
//! email-shaped strings), and failures carry their kind, never a message
//! that could echo what a task was about.

use serde_json::{json, Map, Value};

/// Report format version (bump when fields change meaning).
pub const REPORT_FORMAT: u32 = 1;
/// Events scanned for failures, newest first, and failures kept.
const MAX_FAILURES: usize = 30;
/// Lines of the guest boot log kept.
pub const BOOT_LOG_LINES: usize = 40;
/// Longest free-text value kept (a log line, a reason).
const MAX_TEXT: usize = 400;

/// Everything the report is built from (gathered by the command).
pub struct ReportFacts {
    pub created_at: String,
    pub app_version: &'static str,
    pub commit: Option<&'static str>,
    pub release_build: bool,
    pub system_check: Value,
    pub onboarding: Value,
    pub intelligence: Value,
    pub status: Value,
    /// Windows only: broker service and virtualization readiness.
    pub windows: Option<Value>,
    pub events: Vec<Value>,
    pub boot_log: Vec<String>,
    /// The person's home folder, replaced by `~` everywhere.
    pub home: Option<String>,
}

/// Replace what could identify the person or leak a secret.
pub fn sanitize(text: &str, home: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(home) = home.filter(|h| h.len() > 3) {
        out = out.replace(home, "~");
        let swapped: String = home
            .chars()
            .map(|c| match c {
                '\\' => '/',
                '/' => '\\',
                c => c,
            })
            .collect();
        out = out.replace(&swapped, "~");
        let user = home
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("");
        if user.chars().count() >= 3 {
            out = replace_word(&out, user, "<user>");
        }
    }
    let out = redact_tokens(&out);
    let mut out: String = out.chars().take(MAX_TEXT).collect();
    if text.chars().count() > MAX_TEXT {
        out.push('…');
    }
    out
}

/// `word` replaced only where it is not part of a longer word.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + word.len()..].chars().next();
        out.push_str(&rest[..at]);
        if before.is_some_and(is_word) || after.is_some_and(is_word) {
            out.push_str(word);
        } else {
            out.push_str(with);
        }
        rest = &rest[at + word.len()..];
    }
    out.push_str(rest);
    out
}

/// Key- and email-shaped tokens become placeholders.
fn redact_tokens(text: &str) -> String {
    text.split_inclusive(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '<' | '>')
    })
    .map(|chunk| {
        let token = chunk.trim_end_matches(|c: char| {
            c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '<' | '>')
        });
        let tail = &chunk[token.len()..];
        let secret = (token.starts_with("sk-")
            || token.starts_with("ghp_")
            || token.starts_with("github_pat_"))
            && token.len() >= 20;
        let email = token
            .split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.starts_with('.'));
        if secret {
            format!("<redacted>{tail}")
        } else if email {
            format!("<email>{tail}")
        } else {
            chunk.to_string()
        }
    })
    .collect()
}

/// Every string inside `value`, sanitized; keys unchanged.
fn sanitize_value(value: Value, home: Option<&str>) -> Value {
    match value {
        Value::String(s) => Value::String(sanitize(&s, home)),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(|v| sanitize_value(v, home)).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, sanitize_value(v, home)))
                .collect(),
        ),
        other => other,
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Failure codes from Core's events: what failed and when, never a message
/// that could carry what a task was about.
fn failures(events: &[Value]) -> Vec<Value> {
    let action = |e: &Value| {
        e.pointer("/request/action/type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    };
    let mut out: Vec<Value> = events
        .iter()
        .rev()
        .filter_map(|e| {
            let kind = e.get("type").and_then(Value::as_str)?;
            let at = text(e, "at");
            let entry = match kind {
                "action_failed" => json!({"code": "action_failed", "action": action(e), "at": at}),
                "action_denied" => json!({"code": "action_denied_by_policy", "action": action(e)}),
                "guest_runtime_error" => json!({"code": "guest_runtime_error", "detail": text(e, "message"), "at": at}),
                "guest_runtime_disconnected" => json!({"code": "guest_runtime_disconnected", "detail": text(e, "reason"), "at": at}),
                "guest_runtime_incompatible" => json!({"code": "guest_runtime_incompatible", "guest_version": e.get("guest_version"), "at": at}),
                "graphical_session_failed" => json!({"code": "graphical_session_failed", "detail": text(e, "reason"), "at": at}),
                "computer_state_changed" if text(e, "to").as_deref() == Some("error") => {
                    json!({"code": "computer_error", "from": text(e, "from"), "at": at})
                }
                "task_status_changed" => match text(e, "to").as_deref() {
                    Some(to @ ("failed" | "cancelled")) => json!({"code": format!("task_{to}"), "at": at}),
                    _ => return None,
                },
                _ => return None,
            };
            Some(entry)
        })
        .take(MAX_FAILURES)
        .collect();
    out.reverse();
    out
}

/// The status Core reports, minus what points at a task.
fn computer(status: Value) -> Value {
    let Value::Object(mut map) = status else {
        return Value::Null;
    };
    map.remove("active_task");
    Value::Object(map)
}

/// The report, as JSON (pure: every fact comes in through `facts`).
pub fn build(facts: ReportFacts) -> Value {
    let home = facts.home.as_deref();
    let mut report = Map::new();
    report.insert(
        "report".into(),
        json!({
            "format": REPORT_FORMAT,
            "created_at": facts.created_at,
            "about": "Technical facts for troubleshooting Pegoles. No screenshots, task text, agent messages, API keys or personal files; the home folder shows as ~.",
        }),
    );
    report.insert(
        "app".into(),
        json!({
            "version": facts.app_version,
            "commit": facts.commit,
            "build": if facts.release_build { "release" } else { "debug" },
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }),
    );
    report.insert("system".into(), facts.system_check);
    if let Some(windows) = facts.windows {
        report.insert("windows".into(), windows);
    }
    report.insert("onboarding".into(), facts.onboarding);
    report.insert("intelligence".into(), facts.intelligence);
    report.insert("computer".into(), computer(facts.status));
    report.insert(
        "recent_failures".into(),
        Value::Array(failures(&facts.events)),
    );
    let tail: Vec<String> = facts
        .boot_log
        .iter()
        .rev()
        .take(BOOT_LOG_LINES)
        .rev()
        .cloned()
        .collect();
    report.insert("boot_log_tail".into(), json!(tail));
    sanitize_value(Value::Object(report), home)
}

/// `Pegoles-report-YYYYMMDD-HHMMSS.json`, from a UTC timestamp.
pub fn file_name(now: chrono::DateTime<chrono::Utc>) -> String {
    format!("Pegoles-report-{}.json", now.format("%Y%m%d-%H%M%S"))
}

/// Where the report goes: the person's Downloads folder, else their home.
pub fn report_dir(
    home: Option<&std::path::Path>,
    fallback: &std::path::Path,
) -> std::path::PathBuf {
    match home {
        Some(h) if h.join("Downloads").is_dir() => h.join("Downloads"),
        Some(h) if h.is_dir() => h.to_path_buf(),
        _ => fallback.to_path_buf(),
    }
}

/// The person's home folder, from the environment the OS set.
pub fn home_dir() -> Option<std::path::PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(events: Vec<Value>, home: Option<&str>) -> ReportFacts {
        ReportFacts {
            created_at: "2026-09-28T13:00:00Z".into(),
            app_version: "0.1.0",
            commit: Some("abc123def456"),
            release_build: true,
            system_check: json!({"os_name": "Windows 11 Home (24H2)", "memory_bytes": 17179869184u64,
                "virtualization": {"state": "needs_enable", "technical": "VirtualMachinePlatform: Disabled"},
                "runtime_problem": "worker missing at C:\\Users\\ana\\AppData\\Local\\Pegoles\\x"}),
            onboarding: json!({"step": "check", "completed": false}),
            intelligence: json!({"provider": "local", "anthropic_key_configured": false}),
            status: json!({"backend": "real", "computer_state": "error", "active_task": "task-123"}),
            windows: Some(json!({"broker_installed": false})),
            events,
            boot_log: (0..60).map(|i| format!("line {i}")).collect(),
            home: home.map(str::to_string),
        }
    }

    fn task_events() -> Vec<Value> {
        vec![
            json!({"type": "task_created", "task_id": "t1", "title": "Pay my rent to Ana", "at": "a"}),
            json!({"type": "agent_message", "task_id": "t1", "kind": "progress", "text": "Opening the bank site", "at": "b"}),
            json!({"type": "action_started", "action_id": "x", "request": {"action": {"type": "type_text", "text": "hunter2"}}, "at": "c"}),
            json!({"type": "action_failed", "action_id": "x", "request": {"action": {"type": "type_text", "text": "hunter2"}}, "error": "key not mappable: hunter2", "at": "d"}),
            json!({"type": "action_denied", "request": {"action": {"type": "open_url", "url": "https://bank.example/ana"}}, "reason": "network: https://bank.example/ana"}),
            json!({"type": "guest_runtime_disconnected", "computer_id": "c", "reason": "vsock reset", "at": "e"}),
            json!({"type": "task_status_changed", "task_id": "t1", "from": "running", "to": "failed", "at": "f"}),
        ]
    }

    #[test]
    fn never_carries_task_text_messages_typed_text_or_urls() {
        let report = build(facts(task_events(), None));
        let text = report.to_string();
        for secret in [
            "Pay my rent",
            "Ana",
            "Opening the bank",
            "hunter2",
            "bank.example",
            "task-123",
        ] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        let failures = report["recent_failures"].as_array().unwrap();
        let codes: Vec<&str> = failures
            .iter()
            .map(|f| f["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            codes,
            [
                "action_failed",
                "action_denied_by_policy",
                "guest_runtime_disconnected",
                "task_failed"
            ]
        );
        assert_eq!(failures[0]["action"], "type_text");
    }

    #[test]
    fn has_the_facts_a_helper_needs() {
        let report = build(facts(vec![], None));
        assert_eq!(report["report"]["format"], REPORT_FORMAT);
        assert_eq!(report["app"]["version"], "0.1.0");
        assert_eq!(report["app"]["commit"], "abc123def456");
        assert_eq!(report["app"]["build"], "release");
        assert_eq!(report["system"]["virtualization"]["state"], "needs_enable");
        assert_eq!(report["windows"]["broker_installed"], false);
        assert_eq!(report["onboarding"]["step"], "check");
        assert_eq!(report["computer"]["computer_state"], "error");
        assert!(report["computer"].get("active_task").is_none());
        let tail = report["boot_log_tail"].as_array().unwrap();
        assert_eq!(tail.len(), BOOT_LOG_LINES);
        assert_eq!(tail.last().unwrap(), "line 59");
    }

    #[test]
    fn hides_the_home_folder_user_name_keys_and_emails() {
        let home = "C:\\Users\\ana";
        let report = build(facts(vec![], Some(home)));
        let problem = report["system"]["runtime_problem"].as_str().unwrap();
        assert_eq!(problem, "worker missing at ~\\AppData\\Local\\Pegoles\\x");
        assert_eq!(
            sanitize("/Users/pedro/Library/x", Some("/Users/pedro")),
            "~/Library/x"
        );
        assert_eq!(
            sanitize("owner pedro, not pedrosa", Some("/Users/pedro")),
            "owner <user>, not pedrosa"
        );
        assert_eq!(
            sanitize("key sk-ant-api03-abcdefghijklmnopqrstu end", None),
            "key <redacted> end"
        );
        assert_eq!(
            sanitize("mail me: ana@example.com.", None),
            "mail me: <email>"
        );
        assert!(sanitize(&"x".repeat(1000), None).chars().count() <= MAX_TEXT + 1);
    }

    #[test]
    fn names_the_file_by_time_and_prefers_downloads() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-28T13:04:05Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(file_name(now), "Pegoles-report-20260928-130405.json");
        let tmp = std::env::temp_dir().join(format!("pegoles-report-test-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("Downloads")).unwrap();
        assert_eq!(
            report_dir(Some(&tmp), std::path::Path::new("/fallback")),
            tmp.join("Downloads")
        );
        std::fs::remove_dir_all(tmp.join("Downloads")).unwrap();
        assert_eq!(
            report_dir(Some(&tmp), std::path::Path::new("/fallback")),
            tmp
        );
        assert_eq!(
            report_dir(None, std::path::Path::new("/fallback")),
            std::path::Path::new("/fallback")
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}

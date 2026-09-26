//! Structured actions: the ONLY language a model may speak to Pegoles.
//!
//! SECURITY INVARIANT (see docs/SECURITY.md):
//! 1. Model-generated commands never execute directly on the host.
//! 2. All agent actions are one of the typed variants below.
//! 3. Every structured action passes through Pegoles Policy.
//!
//! The vocabulary is deliberately the computer-use set only: observe,
//! pointer, keyboard, wait. Each variant has its own security meaning;
//! there is no generic "execute" primitive, no shell, no file access, no
//! URL opening, and never a host variant. A new capability must arrive as
//! a new typed variant with its own policy rule and guest implementation
//! (the guest runtime re-validates every primitive it receives).
//!
//! ## Coordinate system
//!
//! Pointer actions use NORMALIZED agent coordinates: `x`/`y` in
//! `0.0..=1.0`, origin at the guest display's top-left. The executor
//! converts them to guest pixels with the CURRENT display size
//! (`DisplayTransform`), so an action script is independent of the
//! guest resolution, viewport scaling, Retina/HiDPI factors, and window
//! resizes. Model coordinates are never macOS NSView pixels and never
//! host pixels. See `pegoles-computer/src/coords.rs`.
//!
//! ## Scroll semantics
//!
//! `Scroll.delta_x/delta_y` are LOGICAL scroll units (lines at the
//! current guest settings); positive `delta_y` scrolls content DOWN
//! (the page moves up, as with a mouse wheel pushed away). Adapters
//! convert to platform wheel deltas; platform quirks must not leak here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ActionId, ComputerId, TaskId};

/// Which pointer button an action uses. Guest semantics: primary is the
/// guest's main button regardless of host left/right configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    #[default]
    Primary,
    Secondary,
    Middle,
}

/// Structured action executed inside Pegoles Computer. NEVER the host.
/// Unknown `type` tags fail to deserialize: deny by construction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComputerAction {
    /// Capture the CURRENT guest framebuffer (VM display only, never the
    /// host screen) with frame metadata. On demand; no streaming.
    ObserveScreen,
    /// Guest display size without pixels (cheap layout probe).
    GetDisplayInfo,
    /// Glide the agent pointer to a normalized position.
    MovePointer { x: f64, y: f64 },
    Click {
        x: f64,
        y: f64,
        #[serde(default)]
        button: PointerButton,
    },
    DoubleClick {
        x: f64,
        y: f64,
        #[serde(default)]
        button: PointerButton,
    },
    MouseDown {
        x: f64,
        y: f64,
        #[serde(default)]
        button: PointerButton,
    },
    MouseUp {
        x: f64,
        y: f64,
        #[serde(default)]
        button: PointerButton,
    },
    /// Press-drag-release with natural interpolation over `duration_ms`.
    /// Never teleport: down at start, interpolated moves, up at end.
    Drag {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        #[serde(default)]
        button: PointerButton,
        duration_ms: u32,
    },
    /// Logical scroll at a normalized position.
    Scroll {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    },
    /// One named key (see `crate::keys::normalize_key_name`).
    KeyPress { key: String },
    /// Simultaneous chord, e.g. `["Control", "c"]`. Modifiers first.
    KeyChord { keys: Vec<String> },
    /// Type text through the guest input path (never the host
    /// clipboard). `sensitive` marks secret-broker input: audit and UI
    /// must redact the text (see `ActionRequest::redacted_for_event`).
    TypeText {
        text: String,
        #[serde(default)]
        sensitive: bool,
    },
    /// Do nothing for `duration_ms`. Cancellable like any action.
    Wait { duration_ms: u32 },
}

impl ComputerAction {
    /// Short verb for logs, UI labels, and audit rows.
    pub fn verb(&self) -> &'static str {
        match self {
            ComputerAction::ObserveScreen => "observe",
            ComputerAction::GetDisplayInfo => "display_info",
            ComputerAction::MovePointer { .. } => "move",
            ComputerAction::Click { .. } => "click",
            ComputerAction::DoubleClick { .. } => "double_click",
            ComputerAction::MouseDown { .. } => "mouse_down",
            ComputerAction::MouseUp { .. } => "mouse_up",
            ComputerAction::Drag { .. } => "drag",
            ComputerAction::Scroll { .. } => "scroll",
            ComputerAction::KeyPress { .. } => "key_press",
            ComputerAction::KeyChord { .. } => "key_chord",
            ComputerAction::TypeText { .. } => "type",
            ComputerAction::Wait { .. } => "wait",
        }
    }

    /// Human-readable one-liner for the activity timeline (never raw JSON).
    /// `redacted` forces secret-safe output for sensitive typing.
    pub fn describe(&self, redacted: bool) -> String {
        match self {
            ComputerAction::ObserveScreen => "Looking at the screen".to_string(),
            ComputerAction::GetDisplayInfo => "Checking display size".to_string(),
            ComputerAction::MovePointer { .. } => "Moving pointer".to_string(),
            ComputerAction::Click { .. } => "Clicking".to_string(),
            ComputerAction::DoubleClick { .. } => "Double-clicking".to_string(),
            ComputerAction::MouseDown { .. } => "Pressing pointer".to_string(),
            ComputerAction::MouseUp { .. } => "Releasing pointer".to_string(),
            ComputerAction::Drag { .. } => "Dragging".to_string(),
            ComputerAction::Scroll { .. } => "Scrolling".to_string(),
            ComputerAction::KeyPress { key } => format!("Pressing {key}"),
            ComputerAction::KeyChord { keys } => format!("Pressing {}", keys.join("+")),
            ComputerAction::TypeText { text, .. } => {
                if redacted {
                    "Typing".to_string()
                } else {
                    let short: String = text.chars().take(42).collect();
                    if text.chars().count() > 42 {
                        format!("Typing “{short}…”")
                    } else {
                        format!("Typing “{short}”")
                    }
                }
            }
            ComputerAction::Wait { .. } => "Waiting".to_string(),
        }
    }

    /// Whether this action carries secret text that audit/UI must redact.
    pub fn is_sensitive(&self) -> bool {
        matches!(
            self,
            ComputerAction::TypeText {
                sensitive: true,
                ..
            }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub action_id: ActionId,
    pub task_id: TaskId,
    pub computer_id: ComputerId,
    pub action: ComputerAction,
    /// Future agents may ask for a frame right after the action without
    /// chaining a second request. Observation-only, no extra policy hop.
    #[serde(default)]
    pub observe_after: bool,
    pub requested_at: DateTime<Utc>,
}

impl ActionRequest {
    pub fn new(task_id: TaskId, computer_id: ComputerId, action: ComputerAction) -> Self {
        Self {
            action_id: ActionId::new(),
            task_id,
            computer_id,
            action,
            observe_after: false,
            requested_at: Utc::now(),
        }
    }

    pub fn with_observe_after(mut self, observe_after: bool) -> Self {
        self.observe_after = observe_after;
        self
    }

    /// Copy safe for events/logs: clears secret text, keeps the shape.
    pub fn redacted_for_event(&self) -> Self {
        if !self.action.is_sensitive() {
            return self.clone();
        }
        let mut redacted = self.clone();
        redacted.action = ComputerAction::TypeText {
            text: String::new(),
            sensitive: true,
        };
        redacted
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    Executed,
    Blocked,
    NeedsApproval,
    /// Transport/backend fault or timeout (see `ActionResult.error`).
    Failed,
    /// Cancelled mid-flight (takeover, pause, stop, shutdown).
    Interrupted,
}

/// Structured result of one executed action. Never just a bool: future
/// agent reasoning needs timing, errors, and the resulting observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub action_id: ActionId,
    pub outcome: ActionOutcome,
    pub success: bool,
    pub message: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    /// Milliseconds from dispatch to completion (input + ack latency).
    pub duration_ms: u64,
    /// Machine-readable failure detail. `None` on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Frame captured via `observe_after`, if requested and available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resulting_frame_id: Option<crate::ids::FrameId>,
}

impl ActionResult {
    pub fn executed(
        action_id: ActionId,
        started_at: DateTime<Utc>,
        message: impl Into<String>,
    ) -> Self {
        let completed_at = Utc::now();
        let duration_ms = completed_at
            .signed_duration_since(started_at)
            .num_milliseconds()
            .max(0) as u64;
        Self {
            action_id,
            outcome: ActionOutcome::Executed,
            success: true,
            message: message.into(),
            started_at,
            completed_at,
            duration_ms,
            error: None,
            resulting_frame_id: None,
        }
    }

    pub fn failed(
        action_id: ActionId,
        started_at: DateTime<Utc>,
        outcome: ActionOutcome,
        error: impl Into<String>,
    ) -> Self {
        let completed_at = Utc::now();
        let duration_ms = completed_at
            .signed_duration_since(started_at)
            .num_milliseconds()
            .max(0) as u64;
        let error = error.into();
        Self {
            action_id,
            outcome,
            success: false,
            message: error.clone(),
            started_at,
            completed_at,
            duration_ms,
            error: Some(error),
            resulting_frame_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_uses_structured_tag() {
        let a = ComputerAction::Click {
            x: 0.1,
            y: 0.2,
            button: PointerButton::Primary,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["type"], "click");
    }

    #[test]
    fn vocabulary_has_no_execution_or_host_verbs() {
        // Guard: removed/forbidden verbs must not deserialize. A model
        // (or a webview) sending them gets a parse error, never an action.
        for verb in [
            "shell",
            "read_file",
            "write_file",
            "open_url",
            "host_shell",
            "execute_on_host",
            "raw_host_command",
            "type",
            "screenshot",
        ] {
            let json = format!(
                r#"{{"type":"{verb}","command":"ls","path":"/x","url":"http://x","text":"x","content":"x"}}"#
            );
            assert!(
                serde_json::from_str::<ComputerAction>(&json).is_err(),
                "{verb} must not be a ComputerAction"
            );
        }
    }

    #[test]
    fn legacy_click_json_still_parses_with_default_button() {
        // Wire compat: pre-Phase-5 integer pixels parse as normalized
        // floats; the button defaults to primary.
        let v: ComputerAction = serde_json::from_str(r#"{"type":"click","x":10,"y":20}"#).unwrap();
        assert_eq!(
            v,
            ComputerAction::Click {
                x: 10.0,
                y: 20.0,
                button: PointerButton::Primary,
            }
        );
    }

    #[test]
    fn sensitive_typing_redacts_for_events() {
        let req = ActionRequest::new(
            TaskId::new(),
            ComputerId::new(),
            ComputerAction::TypeText {
                text: "s3cr3t".into(),
                sensitive: true,
            },
        );
        let redacted = req.redacted_for_event();
        let json = serde_json::to_string(&redacted).unwrap();
        assert!(!json.contains("s3cr3t"));
        assert_eq!(redacted.action_id, req.action_id);
    }

    #[test]
    fn describe_never_emits_raw_protocol() {
        let req = ActionRequest::new(
            TaskId::new(),
            ComputerId::new(),
            ComputerAction::Drag {
                from_x: 0.1,
                from_y: 0.1,
                to_x: 0.9,
                to_y: 0.9,
                button: PointerButton::Primary,
                duration_ms: 400,
            },
        );
        let label = req.action.describe(false);
        assert_eq!(label, "Dragging");
        assert!(!label.contains('{'));
    }
}

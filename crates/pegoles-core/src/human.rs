//! Snapshot control authority. Only a native-confirmed desktop command grants a session.
use crate::ComputerRegistry;
use pegoles_computer::{action_to_input_ops, DisplayTransform, InputOp};
use pegoles_protocol::{ActionRequest, ComputerAction, Decision, PointerButton, TaskId};
use pegoles_protocol::{ComputerId, ComputerState, ControlOwner, FrameId, ObservedFrameMeta};
use std::time::{Duration, Instant};

pub const CAPTURE_TTL: Duration = Duration::from_secs(12);
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanAuthority {
    pub computer_id: ComputerId,
    pub frame_id: FrameId,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HumanInput {
    Move {
        x: f64,
        y: f64,
    },
    Down {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    Up {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    Scroll {
        x: f64,
        y: f64,
        dx: i32,
        dy: i32,
    },
    Key {
        key: String,
    },
    Chord {
        keys: Vec<String>,
    },
    Text {
        text: String,
    },
}
#[derive(Default)]
pub(crate) struct HumanState {
    pub generation: u64,
    pub session: bool,
    pub observed: Option<(ObservedFrameMeta, Instant)>,
    rate: std::collections::VecDeque<Instant>,
    sequence: u64,
}
impl ComputerRegistry {
    /// Called with the same Core lock used to validate authority and dispatch. No queue.
    pub fn human_input(&mut self, a: &HumanAuthority, input: HumanInput) -> Result<(), String> {
        let result = self.dispatch_human_input(a, input);
        if result.is_err() && a.generation == self.human.generation {
            let _ = self.return_control();
        }
        result
    }
    fn dispatch_human_input(
        &mut self,
        a: &HumanAuthority,
        input: HumanInput,
    ) -> Result<(), String> {
        if self.control_owner() != ControlOwner::User || !self.human.session {
            return Err("Human control is required".into());
        }
        self.validate_human_authority(a)?;
        while self
            .human
            .rate
            .front()
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(1))
        {
            self.human.rate.pop_front();
        }
        if self.human.rate.len() >= 60 {
            return Err("Human input rate exceeded".into());
        }
        self.human.rate.push_back(Instant::now());
        let key = |k: &str| {
            pegoles_protocol::keys::normalize_key_name(k)
                .filter(|v| v == k)
                .ok_or_else(|| "Canonical guest key required".to_string())
        };
        let action = match input {
            HumanInput::Move { x, y } => ComputerAction::MovePointer { x, y },
            HumanInput::Down { x, y, button } => ComputerAction::MouseDown { x, y, button },
            HumanInput::Up { x, y, button } => ComputerAction::MouseUp { x, y, button },
            HumanInput::Scroll { x, y, dx, dy } => ComputerAction::Scroll {
                x,
                y,
                delta_x: dx as f64,
                delta_y: dy as f64,
            },
            HumanInput::Key { key: k } => ComputerAction::KeyPress { key: key(&k)? },
            HumanInput::Chord { keys } => {
                if keys.len() > 4 {
                    return Err("At most four chord keys".into());
                }
                ComputerAction::KeyChord {
                    keys: keys.iter().map(|k| key(k)).collect::<Result<_, _>>()?,
                }
            }
            HumanInput::Text { text } => {
                if text.chars().count() > 256 {
                    return Err("Human text exceeds 256 characters".into());
                }
                ComputerAction::TypeText {
                    text,
                    sensitive: true,
                }
            }
        };
        let req = ActionRequest::new(TaskId::new(), a.computer_id, action);
        if pegoles_policy::evaluate(&req, &pegoles_policy::PolicyContext::default()).decision
            != Decision::Allow
        {
            return Err("Human input rejected by policy".into());
        }
        for op in action_to_input_ops(&req.action, &DisplayTransform::headless(a.width, a.height)) {
            match &op {
                InputOp::Down { button, point } => {
                    self.input_pressed.press_button(*button);
                    self.last_pointer_guest = *point;
                }
                InputOp::Up { button, point } => {
                    self.input_pressed.release_button(*button);
                    self.last_pointer_guest = *point;
                }
                InputOp::Move { point } | InputOp::Scroll { point, .. } => {
                    self.last_pointer_guest = *point
                }
                _ => {}
            }
            self.human.sequence = self.human.sequence.wrapping_add(1);
            let id = format!("human:{}:{}", self.human.generation, self.human.sequence);
            if !self
                .backend
                .as_deref_mut()
                .ok_or("Computer unavailable")?
                .input_execute(&id, &op)
                .ok
            {
                return Err("Guest input failed".into());
            }
        }
        Ok(())
    }
    pub fn agent_input_cancelled(&self) -> bool {
        self.agent_cancel.is_cancelled()
    }
    pub fn control_generation(&self) -> u64 {
        self.human.generation
    }
    pub fn snapshot_control_available(&self) -> bool {
        self.state() == Some(ComputerState::Running)
            && self.input_available()
            && self.input_status().frame_available
            && self.guest_pixel_size().is_ok()
    }
    pub fn validate_human_authority(&self, a: &HumanAuthority) -> Result<(), String> {
        if !self.snapshot_control_available()
            || self.computer_id().ok() != Some(a.computer_id)
            || a.generation != self.human.generation
            || self.guest_pixel_size().ok() != Some((a.width, a.height))
        {
            return Err("Control session is no longer valid".into());
        }
        let Some((m, at)) = &self.human.observed else {
            return Err("Capture the guest screen first".into());
        };
        if at.elapsed() >= CAPTURE_TTL
            || m.frame_id != a.frame_id
            || m.computer_id != a.computer_id
            || (m.width_px, m.height_px) != (a.width, a.height)
        {
            return Err("Capture a fresh guest screen".into());
        }
        Ok(())
    }
    /// Trusted Rust entry point; the desktop must confirm this exact authority natively first.
    pub fn take_snapshot_control(&mut self, a: &HumanAuthority) -> Result<(), String> {
        self.validate_human_authority(a)?;
        if self.control_owner() == ControlOwner::User {
            return Err("Control already belongs to the human".into());
        }
        self.release_pressed_state("human takeover");
        self.human.session = true;
        self.set_control(a.computer_id, ControlOwner::User, &mut Vec::new());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pegoles_protocol::PointerButton;
    fn rig() -> (ComputerRegistry, HumanAuthority) {
        let mut r = ComputerRegistry::new(crate::EventBus::new());
        let mut b = pegoles_computer::MockComputerBackend::new();
        b.set_input_enabled(true);
        r.create_default_on(Box::new(b)).unwrap();
        r.start().unwrap();
        let m = r.capture_frame("human-test", &mut Vec::new()).unwrap();
        let a = HumanAuthority {
            computer_id: m.computer_id,
            frame_id: m.frame_id,
            width: m.width_px,
            height: m.height_px,
            generation: r.control_generation(),
        };
        (r, a)
    }
    #[test]
    fn static_pixels_still_have_fresh_observation_authority() {
        let (mut r, a) = rig();
        r.human.observed.as_mut().unwrap().1 = Instant::now() - CAPTURE_TTL;
        assert!(r.take_snapshot_control(&a).is_err());
        let m = r.capture_frame("again", &mut Vec::new()).unwrap();
        assert_eq!(m.frame_id, a.frame_id);
        let mut wrong = a.clone();
        wrong.computer_id = ComputerId::new();
        assert!(r.take_snapshot_control(&wrong).is_err());
        r.take_snapshot_control(&a).unwrap();
        r.pump();
        assert_eq!(r.control_owner(), ControlOwner::User);
        r.return_control().unwrap();
        assert_eq!(r.control_owner(), ControlOwner::None);
        assert!(r.take_snapshot_control(&a).is_err());
    }
    #[test]
    fn human_dispatch_requires_exact_session_and_releases_held_input() {
        let (mut r, mut a) = rig();
        assert!(r
            .human_input(&a, HumanInput::Key { key: "Tab".into() })
            .is_err());
        r.take_snapshot_control(&a).unwrap();
        a.generation = r.control_generation();
        r.human_input(
            &a,
            HumanInput::Down {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            },
        )
        .unwrap();
        assert!(!r.input_status().pressed_clean);
        r.return_control().unwrap();
        assert!(r.input_status().pressed_clean);
        assert!(r
            .human_input(&a, HumanInput::Key { key: "Tab".into() })
            .is_err());
    }
    #[test]
    fn rate_limit_and_stop_revoke_human_authority() {
        let (mut r, mut a) = rig();
        r.take_snapshot_control(&a).unwrap();
        a.generation = r.control_generation();
        for _ in 0..60 {
            r.human_input(&a, HumanInput::Move { x: 0.1, y: 0.1 })
                .unwrap();
        }
        assert!(r
            .human_input(&a, HumanInput::Move { x: 0.1, y: 0.1 })
            .is_err());
        assert_eq!(r.control_owner(), ControlOwner::None);
        let (mut r, mut a) = rig();
        r.take_snapshot_control(&a).unwrap();
        a.generation = r.control_generation();
        r.cancel_agent_input("stop task");
        assert!(r
            .human_input(&a, HumanInput::Key { key: "Tab".into() })
            .is_err());
        assert_eq!(r.control_owner(), ControlOwner::None);
    }
    #[test]
    fn malformed_human_input_revokes_session_without_guest_dispatch() {
        for input in [
            HumanInput::Move {
                x: f64::NAN,
                y: 0.0,
            },
            HumanInput::Move { x: 1.1, y: 0.0 },
            HumanInput::Text {
                text: "x".repeat(257),
            },
            HumanInput::Key {
                key: "Unknown".into(),
            },
        ] {
            let (mut r, mut a) = rig();
            r.take_snapshot_control(&a).unwrap();
            a.generation = r.control_generation();
            assert!(r.human_input(&a, input).is_err());
            assert_eq!(r.control_owner(), ControlOwner::None);
        }
    }
}

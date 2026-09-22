//! `ViewportState` derivation: a PURE function of real backend facts.
//!
//! The ComputerViewport never guesses. Core gathers facts (image
//! preparation, computer state, guest handshake state, graphical session
//! report, display attachment, control owner) into [`ViewportFacts`] and
//! [`derive_viewport`] maps them to exactly one [`ViewportState`], plus a
//! machine-readable [`ViewportIssue`] explaining Error states (and the one
//! degraded Ready: display configured but no display adapter).
//!
//! Headless computers (no `DisplayConfig`) never invent a state: once the
//! guest is Ready the viewport is `Ready` and the status payload carries
//! `display_available: false`.

use pegoles_protocol::{
    ComputerState, ControlOwner, GraphicalSessionState, GuestRuntimeState, ViewportState,
};
use serde::{Deserialize, Serialize};

/// Everything the viewport depends on. Plain data, cheap to copy, built
/// by `ComputerRegistry::viewport_facts` from live backend truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportFacts {
    /// Base-image preparation (download/verify/extract) is running.
    pub image_preparing: bool,
    /// `None` = no computer created.
    pub computer: Option<ComputerState>,
    /// Backend has a guest control plane (vsock). Mock does not: its
    /// running computer has nothing further to wait for.
    pub guest_plane: bool,
    pub guest: GuestRuntimeState,
    /// The computer was created with a framebuffer (`DisplayConfig`).
    pub display_configured: bool,
    /// A display adapter exists on this host/process.
    pub display_backend_available: bool,
    pub graphical: GraphicalSessionState,
    /// The guest reported its graphical session at least once this boot.
    pub graphical_reported: bool,
    /// Native framebuffer view confirmed attached for this computer.
    pub display_attached: bool,
    /// The display adapter reported a failure this boot (cleared by a new
    /// geometry request or a new boot).
    pub display_failed: bool,
    pub control: ControlOwner,
}

impl ViewportFacts {
    /// Whether this computer can show a framebuffer at all.
    pub fn display_available(&self) -> bool {
        self.display_configured && self.display_backend_available
    }
}

/// Why the viewport is in `Error` (or degraded `Ready`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewportIssue {
    /// The VM itself is in `ComputerState::Error`.
    ComputerError,
    /// Guest runtime speaks an incompatible protocol version.
    GuestIncompatible,
    /// Handshake timeout or guest protocol violation.
    GuestError,
    /// The guest reported its compositor failed or exited.
    GraphicalSessionFailed,
    /// A display is configured but the guest reports no graphical session
    /// (e.g. compositor never configured inside the image).
    GraphicalSessionUnavailable,
    /// The native display adapter failed to show the framebuffer.
    DisplayFailed,
    /// Degraded, NOT an error: the computer has a framebuffer but this
    /// process has no display adapter, so it behaves as headless.
    DisplayBackendUnavailable,
}

/// Derived viewport: state + optional issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewportView {
    pub state: ViewportState,
    pub issue: Option<ViewportIssue>,
}

impl ViewportView {
    fn ok(state: ViewportState) -> Self {
        Self { state, issue: None }
    }

    fn error(issue: ViewportIssue) -> Self {
        Self {
            state: ViewportState::Error,
            issue: Some(issue),
        }
    }
}

/// Map facts to the one state the viewport shows. Pure and total.
pub fn derive_viewport(f: &ViewportFacts) -> ViewportView {
    use ComputerState as C;
    match f.computer {
        None | Some(C::Stopped) | Some(C::Stopping) => {
            if f.image_preparing {
                ViewportView::ok(ViewportState::Preparing)
            } else {
                ViewportView::ok(ViewportState::Off)
            }
        }
        Some(C::Starting) => ViewportView::ok(ViewportState::Starting),
        Some(C::Paused) => ViewportView::ok(ViewportState::Paused),
        Some(C::Error) => ViewportView::error(ViewportIssue::ComputerError),
        Some(C::Running) => derive_running(f),
    }
}

fn derive_running(f: &ViewportFacts) -> ViewportView {
    use GuestRuntimeState as G;
    if !f.guest_plane {
        // Guestless dev backend: nothing further can become ready.
        return headless_ready(f);
    }
    match f.guest {
        G::Ready => {}
        G::Incompatible => return ViewportView::error(ViewportIssue::GuestIncompatible),
        G::Error => return ViewportView::error(ViewportIssue::GuestError),
        G::Unavailable | G::Waiting | G::Connecting | G::Disconnected => {
            return ViewportView::ok(ViewportState::GuestConnecting)
        }
    }
    if !f.display_available() {
        return headless_ready(f);
    }
    match f.graphical {
        GraphicalSessionState::Failed => {
            return ViewportView::error(ViewportIssue::GraphicalSessionFailed)
        }
        GraphicalSessionState::Unavailable if f.graphical_reported => {
            return ViewportView::error(ViewportIssue::GraphicalSessionUnavailable)
        }
        _ => {}
    }
    if f.display_failed && !f.display_attached {
        return ViewportView::error(ViewportIssue::DisplayFailed);
    }
    if f.graphical != GraphicalSessionState::Ready || !f.display_attached {
        return ViewportView::ok(ViewportState::DisplayStarting);
    }
    match f.control {
        ControlOwner::None => ViewportView::ok(ViewportState::Ready),
        ControlOwner::User => ViewportView::ok(ViewportState::UserControlled),
        // Reserved for a future phase; no Phase 4 API can set it.
        ControlOwner::Agent => ViewportView::ok(ViewportState::AgentActive),
    }
}

/// Ready without a framebuffer. Flags the degraded case where a display
/// was configured but no adapter exists in this process.
fn headless_ready(f: &ViewportFacts) -> ViewportView {
    ViewportView {
        state: ViewportState::Ready,
        issue: (f.display_configured && !f.display_backend_available)
            .then_some(ViewportIssue::DisplayBackendUnavailable),
    }
}

/// The viewport ignoring who controls input: `Ready` here is the
/// precondition for (and the invariant of) user control.
pub fn derive_viewport_without_control(f: &ViewportFacts) -> ViewportView {
    let mut facts = *f;
    facts.control = ControlOwner::None;
    derive_viewport(&facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A graphical computer, fully ready, nobody in control.
    fn ready() -> ViewportFacts {
        ViewportFacts {
            image_preparing: false,
            computer: Some(ComputerState::Running),
            guest_plane: true,
            guest: GuestRuntimeState::Ready,
            display_configured: true,
            display_backend_available: true,
            graphical: GraphicalSessionState::Ready,
            graphical_reported: true,
            display_attached: true,
            display_failed: false,
            control: ControlOwner::None,
        }
    }

    fn state(f: ViewportFacts) -> ViewportState {
        derive_viewport(&f).state
    }

    #[test]
    fn off_and_preparing() {
        let mut f = ready();
        f.computer = None;
        assert_eq!(state(f), ViewportState::Off);
        f.image_preparing = true;
        assert_eq!(state(f), ViewportState::Preparing);
        f.computer = Some(ComputerState::Stopped);
        assert_eq!(state(f), ViewportState::Preparing);
        f.image_preparing = false;
        assert_eq!(state(f), ViewportState::Off);
        f.computer = Some(ComputerState::Stopping);
        assert_eq!(state(f), ViewportState::Off);
    }

    #[test]
    fn lifecycle_states_win_over_everything_below() {
        let mut f = ready();
        f.computer = Some(ComputerState::Starting);
        assert_eq!(state(f), ViewportState::Starting);
        f.computer = Some(ComputerState::Paused);
        f.control = ControlOwner::User;
        assert_eq!(state(f), ViewportState::Paused);
        f.computer = Some(ComputerState::Error);
        let v = derive_viewport(&f);
        assert_eq!(v.state, ViewportState::Error);
        assert_eq!(v.issue, Some(ViewportIssue::ComputerError));
        // Preparing an image never masks a running computer.
        let mut f = ready();
        f.image_preparing = true;
        assert_eq!(state(f), ViewportState::Ready);
    }

    #[test]
    fn guest_handshake_gates_everything() {
        for g in [
            GuestRuntimeState::Unavailable,
            GuestRuntimeState::Waiting,
            GuestRuntimeState::Connecting,
            GuestRuntimeState::Disconnected,
        ] {
            let mut f = ready();
            f.guest = g;
            assert_eq!(state(f), ViewportState::GuestConnecting, "{g:?}");
        }
        let mut f = ready();
        f.guest = GuestRuntimeState::Incompatible;
        assert_eq!(
            derive_viewport(&f).issue,
            Some(ViewportIssue::GuestIncompatible)
        );
        f.guest = GuestRuntimeState::Error;
        assert_eq!(derive_viewport(&f).issue, Some(ViewportIssue::GuestError));
    }

    #[test]
    fn headless_computer_is_ready_after_guest_ready() {
        let mut f = ready();
        f.display_configured = false;
        f.graphical = GraphicalSessionState::Unavailable;
        f.graphical_reported = false;
        f.display_attached = false;
        let v = derive_viewport(&f);
        assert_eq!(v, ViewportView::ok(ViewportState::Ready));
        assert!(!f.display_available());
        // A headless image that answers "no session" is still fine.
        f.graphical_reported = true;
        assert_eq!(derive_viewport(&f), ViewportView::ok(ViewportState::Ready));
        // ...but it still waits for the guest like everyone else.
        f.guest = GuestRuntimeState::Waiting;
        assert_eq!(state(f), ViewportState::GuestConnecting);
    }

    #[test]
    fn display_configured_without_adapter_is_degraded_ready() {
        let mut f = ready();
        f.display_backend_available = false;
        f.display_attached = false;
        let v = derive_viewport(&f);
        assert_eq!(v.state, ViewportState::Ready);
        assert_eq!(v.issue, Some(ViewportIssue::DisplayBackendUnavailable));
    }

    #[test]
    fn guestless_backend_is_ready_when_running() {
        let mut f = ready();
        f.guest_plane = false;
        f.guest = GuestRuntimeState::Unavailable;
        f.display_configured = false;
        assert_eq!(derive_viewport(&f), ViewportView::ok(ViewportState::Ready));
    }

    #[test]
    fn display_starting_until_session_ready_and_attached() {
        let mut f = ready();
        f.graphical = GraphicalSessionState::Unavailable;
        f.graphical_reported = false;
        assert_eq!(state(f), ViewportState::DisplayStarting);
        f.graphical = GraphicalSessionState::Starting;
        f.graphical_reported = true;
        assert_eq!(state(f), ViewportState::DisplayStarting);
        f.graphical = GraphicalSessionState::Ready;
        f.display_attached = false;
        assert_eq!(state(f), ViewportState::DisplayStarting);
        f.display_attached = true;
        assert_eq!(state(f), ViewportState::Ready);
    }

    #[test]
    fn graphical_failures_are_errors() {
        let mut f = ready();
        f.graphical = GraphicalSessionState::Failed;
        assert_eq!(
            derive_viewport(&f).issue,
            Some(ViewportIssue::GraphicalSessionFailed)
        );
        f.graphical = GraphicalSessionState::Unavailable;
        f.graphical_reported = true;
        assert_eq!(
            derive_viewport(&f).issue,
            Some(ViewportIssue::GraphicalSessionUnavailable)
        );
    }

    #[test]
    fn display_failure_is_error_until_attached() {
        let mut f = ready();
        f.display_failed = true;
        f.display_attached = false;
        assert_eq!(
            derive_viewport(&f).issue,
            Some(ViewportIssue::DisplayFailed)
        );
        f.graphical = GraphicalSessionState::Starting;
        assert_eq!(
            derive_viewport(&f).issue,
            Some(ViewportIssue::DisplayFailed)
        );
        // A later successful attach wins over a stale failure flag.
        let mut f = ready();
        f.display_failed = true;
        assert_eq!(state(f), ViewportState::Ready);
    }

    #[test]
    fn control_owner_maps_to_controlled_states() {
        let mut f = ready();
        f.control = ControlOwner::User;
        assert_eq!(state(f), ViewportState::UserControlled);
        f.control = ControlOwner::Agent;
        assert_eq!(state(f), ViewportState::AgentActive);
        assert_eq!(
            derive_viewport_without_control(&f).state,
            ViewportState::Ready
        );
        // Control never masks a lost display.
        f.control = ControlOwner::User;
        f.display_attached = false;
        assert_eq!(state(f), ViewportState::DisplayStarting);
    }

    #[test]
    fn issue_only_accompanies_error_or_degraded_ready() {
        // Exhaustive sweep over the fact space: every Error has an issue,
        // and every issue outside Error is the documented degraded Ready.
        let computers = [
            None,
            Some(ComputerState::Stopped),
            Some(ComputerState::Starting),
            Some(ComputerState::Running),
            Some(ComputerState::Paused),
            Some(ComputerState::Stopping),
            Some(ComputerState::Error),
        ];
        let guests = [
            GuestRuntimeState::Unavailable,
            GuestRuntimeState::Waiting,
            GuestRuntimeState::Connecting,
            GuestRuntimeState::Ready,
            GuestRuntimeState::Disconnected,
            GuestRuntimeState::Incompatible,
            GuestRuntimeState::Error,
        ];
        let sessions = [
            GraphicalSessionState::Unavailable,
            GraphicalSessionState::Starting,
            GraphicalSessionState::Ready,
            GraphicalSessionState::Failed,
        ];
        let bools = [false, true];
        let mut count = 0;
        for computer in computers {
            for guest in guests {
                for graphical in sessions {
                    for &configured in &bools {
                        for &adapter in &bools {
                            for &attached in &bools {
                                for &reported in &bools {
                                    for &failed in &bools {
                                        let f = ViewportFacts {
                                            image_preparing: false,
                                            computer,
                                            guest_plane: true,
                                            guest,
                                            display_configured: configured,
                                            display_backend_available: adapter,
                                            graphical,
                                            graphical_reported: reported,
                                            display_attached: attached,
                                            display_failed: failed,
                                            control: ControlOwner::None,
                                        };
                                        let v = derive_viewport(&f);
                                        count += 1;
                                        match v.state {
                                            ViewportState::Error => assert!(v.issue.is_some()),
                                            ViewportState::Ready => assert!(matches!(
                                                v.issue,
                                                None | Some(
                                                    ViewportIssue::DisplayBackendUnavailable
                                                )
                                            )),
                                            _ => assert_eq!(v.issue, None, "{f:?}"),
                                        }
                                        assert_ne!(v.state, ViewportState::AgentActive);
                                        assert_ne!(v.state, ViewportState::UserControlled);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(count, 7 * 7 * 4 * 32);
    }

    #[test]
    fn wire_shape_is_snake_case() {
        let v = ViewportView::error(ViewportIssue::GraphicalSessionFailed);
        let json = serde_json::to_value(v).unwrap();
        assert_eq!(json["state"], "error");
        assert_eq!(json["issue"], "graphical_session_failed");
    }
}

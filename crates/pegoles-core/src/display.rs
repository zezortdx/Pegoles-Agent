//! Display lifecycle coupling + control ownership (Phase 4).
//!
//! Core owns the installed `ComputerDisplayBackend` and couples it to the
//! computer lifecycle in ONE place:
//!
//! | Computer event            | Display                 | Control        |
//! |---------------------------|-------------------------|----------------|
//! | geometry from the UI      | attach (running) / move | —              |
//! | start / becomes Running   | auto-attach if a slot   | —              |
//! | pause                     | Dimmed (last frame)     | auto-return    |
//! | resume                    | Normal                  | —              |
//! | stop / reset / destroy    | detach                  | auto-return    |
//! | VM error / stops by itself| detach                  | auto-return    |
//! | display detached / failed | —                       | auto-return    |
//! | display no longer ready   | —                       | auto-return    |
//!
//! Readiness chain (all real facts, never timers): VM Running →
//! GuestRuntimeReady → GraphicalSessionReady (guest report) →
//! DisplayAttached (native fact) → DisplayReady (emitted ONCE per boot
//! when the session is Ready AND the view is attached).
//!
//! SECURITY: `ControlOwner::User` means a HUMAN routes input into the
//! isolated guest through the native view. `ControlOwner::Agent` is
//! reserved: no Phase 4 API can enter it (guard-tested). Nothing here can
//! synthesize input — the display trait has no such method.

use chrono::Utc;
use pegoles_computer::{
    ComputerDisplayBackend, DisplayBackendKind, DisplayEvent, DisplayGeometry, DisplayPresentation,
    GraphicalSessionInfo, UnavailableDisplay,
};
use pegoles_protocol::{
    AgentEvent, ComputerId, ComputerState, ControlOwner, DisplayConfig, ViewportState,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::error::{CoreError, Result};
use crate::registry::ComputerRegistry;
use crate::viewport::{
    derive_viewport, derive_viewport_without_control, ViewportFacts, ViewportIssue, ViewportView,
};

/// Geometry changes smaller than this (CSS px) are ResizeObserver jitter.
pub const GEOMETRY_EPSILON_PX: f64 = 0.5;

/// The webview's size in CSS px plus its device scale: the clamp/snap
/// frame for untrusted geometry from the UI. Supplied by the host
/// (Tauri reads it from the window), never by React.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayBounds {
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// What `set_display_geometry` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryOutcome {
    /// The framebuffer view was attached at this geometry.
    Attached,
    /// The attached view moved/resized.
    Updated,
    /// Jitter below `GEOMETRY_EPSILON_PX`: no native call made.
    Unchanged,
    /// Remembered, not applied: no computer running with a display yet
    /// (headless, stopped, starting, or no display adapter). Applied
    /// automatically once the computer can show a framebuffer.
    Deferred,
}

/// Core's record of the native view (confirmed by `DisplayEvent`s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DisplayLink {
    Detached,
    /// `attach` accepted; waiting for the native `Attached` fact.
    Attaching(ComputerId),
    Attached(ComputerId),
}

/// Display + control state owned by the registry.
pub(crate) struct DisplaySlot {
    pub(crate) backend: Box<dyn ComputerDisplayBackend>,
    pub(crate) link: DisplayLink,
    /// Last sanitized geometry the UI asked for (kept across stop/start so
    /// a mounted viewport re-attaches on the next boot).
    pub(crate) desired: Option<DisplayGeometry>,
    /// Last geometry actually handed to the native layer.
    pub(crate) applied: Option<DisplayGeometry>,
    pub(crate) presentation: DisplayPresentation,
    /// Sticky attach/native failure: blocks auto-attach until the UI sends
    /// a new geometry or the computer boots again.
    pub(crate) failure: Option<String>,
    /// Last best-effort error (diagnostics only).
    pub(crate) last_error: Option<String>,
    pub(crate) control: ControlOwner,
    /// Native answers still owed (FIFO contract, see `DisplayEvent`):
    /// one `Attached`/`Failed` per `attach`, one `Detached` per Core
    /// `detach` of a live/pending view. Lets Core tell the echo of its own
    /// earlier request from a new native fact (e.g. the `Detached` of the
    /// previous boot arriving after the next `attach`).
    pub(crate) awaiting_attach: u32,
    pub(crate) awaiting_detach: u32,
    /// VM start (this boot) for `DisplayReady.ready_in_ms`.
    pub(crate) boot_started_at: Option<Instant>,
    /// Set once per boot when DisplayReady is emitted.
    pub(crate) ready_ms: Option<u64>,
}

impl DisplaySlot {
    pub(crate) fn unavailable() -> Self {
        Self {
            backend: Box::new(UnavailableDisplay),
            link: DisplayLink::Detached,
            desired: None,
            applied: None,
            presentation: DisplayPresentation::Normal,
            failure: None,
            last_error: None,
            control: ControlOwner::None,
            awaiting_attach: 0,
            awaiting_detach: 0,
            boot_started_at: None,
            ready_ms: None,
        }
    }
}

/// Everything the UI needs to render the computer surface, in one
/// snapshot (flattened into `get_status`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComputerView {
    pub viewport_state: ViewportState,
    pub viewport_issue: Option<ViewportIssue>,
    /// A framebuffer exists for this computer AND this process can show
    /// it. `false` = headless (or no display adapter).
    pub display_available: bool,
    /// Guest framebuffer config (`null` = headless computer).
    pub display_config: Option<DisplayConfig>,
    pub display_backend: DisplayBackendKind,
    /// Native view confirmed attached.
    pub display_attached: bool,
    pub graphical_session: GraphicalSessionInfo,
    pub control_owner: ControlOwner,
    /// ms from VM start to DisplayReady (this boot), once reached.
    pub display_ready_ms: Option<u64>,
    /// Last display failure/diagnostic, if any.
    pub display_error: Option<String>,
}

/// Default framebuffer for a new computer: DesktopLarge when the ready
/// image ships a graphical stack AND the VM backend can attach a graphics
/// device AND this process has a display adapter to show it; otherwise
/// headless (never a graphics device nobody can see).
pub fn default_display_config(
    image_is_graphical: bool,
    backend_supports_graphics: bool,
    display_adapter_available: bool,
) -> Option<DisplayConfig> {
    if image_is_graphical && backend_supports_graphics && display_adapter_available {
        DisplayConfig::for_profile(pegoles_protocol::DisplayProfile::DesktopLarge)
    } else {
        None
    }
}

impl ComputerRegistry {
    pub(crate) fn emit(&self, event: AgentEvent, out: &mut Vec<AgentEvent>) {
        self.bus.publish(event.clone());
        out.push(event);
    }

    // --- installation / queries ---

    /// Install the platform display adapter (once, at app startup). A
    /// previously installed adapter is detached first (best effort).
    pub fn install_display_backend(&mut self, display: Box<dyn ComputerDisplayBackend>) {
        if self.display.link != DisplayLink::Detached {
            let mut out = Vec::new();
            // Best effort: failures are recorded in `last_error`.
            let _ = self.release_control(&mut out);
            let _ = self.detach_internal("display_replaced", &mut out);
        }
        self.display.backend = display;
        self.display.link = DisplayLink::Detached;
        self.display.applied = None;
        self.display.failure = None;
        self.display.awaiting_attach = 0;
        self.display.awaiting_detach = 0;
    }

    /// Which display adapter is installed (diagnostics / UI capability).
    pub fn display_kind(&self) -> DisplayBackendKind {
        self.display.backend.kind()
    }

    pub fn control_owner(&self) -> ControlOwner {
        self.display.control
    }

    /// Native view confirmed attached for the current computer.
    pub fn display_attached(&self) -> bool {
        match (self.display.link, self.current_id()) {
            (DisplayLink::Attached(attached), Some(id)) => attached == id,
            _ => false,
        }
    }

    pub fn display_ready_ms(&self) -> Option<u64> {
        self.display.ready_ms
    }

    /// Framebuffer config the computer was created with (None = headless).
    pub fn display_config(&self) -> Option<DisplayConfig> {
        self.backend
            .as_ref()
            .and_then(|b| b.config())
            .and_then(|c| c.display)
    }

    /// Whether this computer can show a framebuffer in this process.
    pub fn display_available(&self) -> bool {
        self.display_config().is_some() && self.display.backend.is_available()
    }

    pub fn graphical_session(&self) -> GraphicalSessionInfo {
        self.backend
            .as_ref()
            .map(|b| b.graphical_session())
            .unwrap_or_default()
    }

    /// Native framebuffer views alive (leak checks: 0 after detach).
    pub fn live_native_views(&self) -> usize {
        self.display.backend.live_native_views()
    }

    pub fn display_error(&self) -> Option<String> {
        self.display
            .failure
            .clone()
            .or_else(|| self.display.last_error.clone())
    }

    pub(crate) fn current_id(&self) -> Option<ComputerId> {
        self.backend.as_ref().and_then(|b| b.id())
    }

    /// Facts for the pure viewport derivation, given an already-read VM
    /// state (avoids a second native round-trip per poll).
    pub fn viewport_facts_for(
        &self,
        computer: Option<ComputerState>,
        image_preparing: bool,
    ) -> ViewportFacts {
        let backend = self.backend.as_ref().filter(|b| b.id().is_some());
        let session = self.graphical_session();
        ViewportFacts {
            image_preparing,
            computer: backend.and(computer),
            guest_plane: backend.map(|b| b.capabilities().vsock).unwrap_or(false),
            guest: self.guest_state(),
            display_configured: self.display_config().is_some(),
            display_backend_available: self.display.backend.is_available(),
            graphical: session.state,
            graphical_reported: session.reported,
            display_attached: self.display_attached(),
            display_failed: self.display.failure.is_some(),
            control: self.display.control,
        }
    }

    pub fn viewport_facts(&self, image_preparing: bool) -> ViewportFacts {
        self.viewport_facts_for(self.state(), image_preparing)
    }

    /// Derived viewport (pure function of live facts).
    pub fn viewport(&self, image_preparing: bool) -> ViewportView {
        derive_viewport(&self.viewport_facts(image_preparing))
    }

    /// One consistent snapshot for the UI.
    pub fn computer_view(&self, image_preparing: bool) -> ComputerView {
        self.computer_view_for(self.state(), image_preparing)
    }

    /// Same, for a VM state the caller already read (one native
    /// round-trip per poll instead of two).
    pub fn computer_view_for(
        &self,
        computer: Option<ComputerState>,
        image_preparing: bool,
    ) -> ComputerView {
        let facts = self.viewport_facts_for(computer, image_preparing);
        let view = derive_viewport(&facts);
        ComputerView {
            viewport_state: view.state,
            viewport_issue: view.issue,
            display_available: facts.display_available(),
            display_config: self.display_config(),
            display_backend: self.display_kind(),
            display_attached: facts.display_attached,
            graphical_session: self.graphical_session(),
            control_owner: self.display.control,
            display_ready_ms: self.display.ready_ms,
            display_error: self.display_error(),
        }
    }

    // --- UI-driven display commands ---

    /// Place the framebuffer where React measured its slot. Untrusted
    /// input: sanitized against host-provided bounds (finite, clamped,
    /// snapped), deduplicated against the last applied geometry.
    /// Attaches the view when the computer is running with a display and
    /// nothing is attached yet; otherwise moves the attached view.
    pub fn set_display_geometry(
        &mut self,
        geometry: DisplayGeometry,
        bounds: DisplayBounds,
    ) -> Result<GeometryOutcome> {
        let geometry = geometry
            .sanitize(bounds.width, bounds.height, bounds.scale)
            .map_err(CoreError::InvalidGeometry)?;
        // An explicit request is also the retry path after a failure.
        self.display.failure = None;
        self.display.desired = Some(geometry);
        let Some(id) = self.current_id() else {
            return Ok(GeometryOutcome::Deferred);
        };
        if !self.display_available() {
            return Ok(GeometryOutcome::Deferred);
        }
        match self.display.link {
            DisplayLink::Detached => {
                let state = self.state();
                if !matches!(state, Some(ComputerState::Running | ComputerState::Paused)) {
                    return Ok(GeometryOutcome::Deferred);
                }
                let mut out = Vec::new();
                self.attach_now(id, geometry, state == Some(ComputerState::Paused))?;
                // Native facts may already be queued (synchronous adapters).
                self.drain_display_events(&mut out);
                self.reconcile_display(state, &mut out);
                Ok(GeometryOutcome::Attached)
            }
            DisplayLink::Attaching(_) | DisplayLink::Attached(_) => {
                let unchanged = self
                    .display
                    .applied
                    .map(|applied| !applied.differs_from(&geometry, GEOMETRY_EPSILON_PX))
                    .unwrap_or(false);
                if unchanged {
                    return Ok(GeometryOutcome::Unchanged);
                }
                if let Err(e) = self.display.backend.set_geometry(geometry) {
                    self.display.last_error = Some(e.to_string());
                    return Err(CoreError::Computer(e));
                }
                self.display.applied = Some(geometry);
                Ok(GeometryOutcome::Updated)
            }
        }
    }

    /// Explicitly remove the framebuffer view (UI teardown, leak checks).
    /// Control returns to Pegoles first. The view comes back only when
    /// the UI sends a new geometry.
    pub fn detach_display(&mut self) -> Result<Vec<AgentEvent>> {
        let mut out = Vec::new();
        // Input goes back first; a native failure there is recorded and
        // does not block removing the view.
        let _ = self.release_control(&mut out);
        self.display.desired = None;
        self.detach_internal("explicit", &mut out)?;
        Ok(out)
    }

    /// Hand keyboard/pointer to the HUMAN (Take Control). From Ready
    /// (nothing owns it) or from Agent (takeover: the running agent
    /// sequence is cancelled and its pressed state released BEFORE the
    /// human gets the computer — never simultaneous conflicting control).
    /// Never sets Agent.
    pub fn take_control(&mut self) -> Result<ControlOwner> {
        let id = self.computer_id()?;
        if self.display.control == ControlOwner::User {
            return Ok(ControlOwner::User);
        }
        let facts = self.viewport_facts(false);
        let base = derive_viewport_without_control(&facts);
        if base.state != ViewportState::Ready || !facts.display_available() {
            let why = if facts.display_available() {
                format!("display is not ready (viewport: {:?})", base.state)
            } else {
                "this computer has no display to control".to_string()
            };
            return Err(CoreError::ControlUnavailable(why));
        }
        // Takeover from a running agent sequence: stop it cleanly first.
        if self.display.control == ControlOwner::Agent {
            self.cancel_agent_input("human takeover");
        }
        if let Err(e) = self.display.backend.set_interactive(true) {
            self.display.last_error = Some(e.to_string());
            return Err(CoreError::Computer(e));
        }
        let mut out = Vec::new();
        self.set_control(id, ControlOwner::User, &mut out);
        Ok(ControlOwner::User)
    }

    /// Give input back to Pegoles (Return to Pegoles). Idempotent.
    /// Ownership always returns to None; a native failure to stop routing
    /// input is reported after the state change (the view is then
    /// non-interactive from Core's point of view regardless).
    pub fn return_control(&mut self) -> Result<ControlOwner> {
        let mut out = Vec::new();
        self.release_control(&mut out)?;
        Ok(ControlOwner::None)
    }

    /// Drain native display facts into AgentEvents (DisplayAttached /
    /// DisplayDetached / control changes). Never blocks.
    pub fn pump_display(&mut self) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        self.drain_display_events(&mut out);
        out
    }

    /// Everything a poll needs: guest control plane, native display facts,
    /// then lifecycle reconciliation (auto-attach, dimming, auto-return of
    /// control, DisplayReady). Returns the events it published.
    pub fn pump(&mut self) -> Vec<AgentEvent> {
        let mut out = self.pump_guest();
        self.drain_display_events(&mut out);
        let state = self.state();
        self.reconcile_display(state, &mut out);
        out
    }

    // --- lifecycle hooks (called by registry lifecycle operations) ---

    /// A new boot began: fresh readiness clock, failures forgotten.
    pub(crate) fn display_on_started(&mut self) {
        self.display.boot_started_at = Some(Instant::now());
        self.display.ready_ms = None;
        self.display.failure = None;
    }

    /// The VM is going away (stop/reset/destroy): input back to Pegoles,
    /// view removed. `desired` survives stop/reset (the slot is still
    /// mounted) and is cleared by destroy.
    pub(crate) fn display_on_leaving(&mut self, reason: &str, out: &mut Vec<AgentEvent>) {
        // Best effort on the way out: the VM is going away regardless;
        // native failures are recorded in `last_error`.
        let _ = self.release_control(out);
        let _ = self.detach_internal(reason, out);
    }

    /// The boot ended (VM stopped): readiness clock cleared.
    pub(crate) fn display_on_stopped(&mut self) {
        self.display.boot_started_at = None;
        self.display.ready_ms = None;
    }

    /// Bring display + control in line with the computer's current state.
    pub(crate) fn reconcile_display(
        &mut self,
        state: Option<ComputerState>,
        out: &mut Vec<AgentEvent>,
    ) {
        let Some(id) = self.current_id() else {
            self.display_on_leaving("computer_destroyed", out);
            return;
        };
        let paused = match state {
            Some(ComputerState::Running) => false,
            Some(ComputerState::Paused) => true,
            // Transitional (start/pause/resume in flight): touch nothing.
            Some(ComputerState::Starting) => return,
            Some(ComputerState::Error) => {
                self.display_on_leaving("computer_error", out);
                return;
            }
            Some(ComputerState::Stopping) | Some(ComputerState::Stopped) | None => {
                self.display_on_leaving("computer_stopped", out);
                if state != Some(ComputerState::Stopping) {
                    self.display_on_stopped();
                }
                return;
            }
        };
        if paused {
            let _ = self.release_control(out);
        }
        self.sync_presentation(paused);
        if self.display.link == DisplayLink::Detached
            && self.display.failure.is_none()
            && self.display_available()
        {
            if let Some(geometry) = self.display.desired {
                // A failure is recorded as sticky `failure` (no retry loop).
                if self.attach_now(id, geometry, paused).is_ok() {
                    self.drain_display_events(out);
                }
            }
        }
        let facts = self.viewport_facts_for(state, false);
        let base = derive_viewport_without_control(&facts);
        let display_ready = base.state == ViewportState::Ready
            && facts.display_available()
            && facts.display_attached;
        // A HUMAN can only control through a ready, attached view: take
        // it back when the view goes. Agent input never uses the host
        // view (it travels the guest channel), so an agent session is
        // unaffected by view readiness.
        if self.display.control == ControlOwner::User && !display_ready {
            let _ = self.release_control(out);
        }
        if display_ready && self.display.ready_ms.is_none() {
            let ready_in_ms = self.ms_since_boot();
            self.display.ready_ms = Some(ready_in_ms);
            self.emit(
                AgentEvent::DisplayReady {
                    computer_id: id,
                    ready_in_ms,
                    at: Utc::now(),
                },
                out,
            );
        }
    }

    // --- internals ---

    fn ms_since_boot(&self) -> u64 {
        if let Some(start) = self.display.boot_started_at {
            return start.elapsed().as_millis() as u64;
        }
        self.backend
            .as_ref()
            .and_then(|b| b.instance())
            .and_then(|i| i.started_at.elapsed().ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn attach_now(
        &mut self,
        id: ComputerId,
        geometry: DisplayGeometry,
        paused: bool,
    ) -> Result<()> {
        if let Err(e) = self.display.backend.attach(id, geometry) {
            // Sticky until a new geometry or a new boot: no retry loop.
            self.display.failure = Some(e.to_string());
            return Err(CoreError::Computer(e));
        }
        self.display.link = DisplayLink::Attaching(id);
        self.display.awaiting_attach += 1;
        self.display.applied = Some(geometry);
        self.display.presentation = DisplayPresentation::Normal;
        if paused {
            self.sync_presentation(true);
        }
        Ok(())
    }

    fn sync_presentation(&mut self, paused: bool) {
        if self.display.link == DisplayLink::Detached {
            return;
        }
        let want = if paused {
            DisplayPresentation::Dimmed
        } else {
            DisplayPresentation::Normal
        };
        if self.display.presentation == want {
            return;
        }
        match self.display.backend.set_presentation(want) {
            Ok(()) => self.display.presentation = want,
            Err(e) => self.display.last_error = Some(e.to_string()),
        }
    }

    /// Remove the native view if Core believes one exists. Emits
    /// DisplayDetached only for a CONFIRMED attachment (exactly once; the
    /// native Detached fact that may follow is deduplicated).
    /// Core stops referencing the view either way; a native failure is
    /// recorded and returned.
    fn detach_internal(&mut self, reason: &str, out: &mut Vec<AgentEvent>) -> Result<()> {
        let link = self.display.link;
        if link == DisplayLink::Detached {
            return Ok(());
        }
        let native = self.display.backend.detach(reason);
        if native.is_ok() {
            self.display.awaiting_detach += 1;
        }
        self.mark_detached(link, reason, out);
        native.map_err(|e| {
            self.display.last_error = Some(e.to_string());
            CoreError::Computer(e)
        })
    }

    fn mark_detached(&mut self, link: DisplayLink, reason: &str, out: &mut Vec<AgentEvent>) {
        self.display.link = DisplayLink::Detached;
        self.display.applied = None;
        self.display.presentation = DisplayPresentation::Normal;
        if let DisplayLink::Attached(computer_id) = link {
            self.emit(
                AgentEvent::DisplayDetached {
                    computer_id,
                    reason: reason.to_string(),
                    at: Utc::now(),
                },
                out,
            );
        }
    }

    fn drain_display_events(&mut self, out: &mut Vec<AgentEvent>) {
        let events = self.display.backend.drain_events();
        let current = self.current_id();
        for event in events {
            match event {
                DisplayEvent::Attached { computer_id } => {
                    // FIFO: only the answer to the LATEST attach confirms.
                    self.display.awaiting_attach = self.display.awaiting_attach.saturating_sub(1);
                    let expected = self.display.awaiting_attach == 0
                        && matches!(
                            self.display.link,
                            DisplayLink::Attaching(id) if id == computer_id
                        );
                    if expected {
                        // FIFO sync point: every detach issued before this
                        // attach has been answered by now. Resetting here
                        // stops a missed echo from drifting forever.
                        self.display.awaiting_detach = 0;
                    }
                    if expected && current == Some(computer_id) {
                        self.display.link = DisplayLink::Attached(computer_id);
                        self.display.failure = None;
                        self.emit(
                            AgentEvent::DisplayAttached {
                                computer_id,
                                at: Utc::now(),
                            },
                            out,
                        );
                    }
                    // Stale/unsolicited facts are ignored (never trusted
                    // to resurrect a view Core already dropped).
                }
                DisplayEvent::Detached {
                    computer_id,
                    reason,
                } => {
                    if self.display.awaiting_detach > 0 {
                        // Echo of a detach Core already accounted for.
                        self.display.awaiting_detach -= 1;
                        continue;
                    }
                    let link = self.display.link;
                    let ours = matches!(
                        link,
                        DisplayLink::Attaching(id) | DisplayLink::Attached(id) if id == computer_id
                    );
                    if ours {
                        let _ = self.release_control(out);
                        // Native-initiated: do not auto-reattach; the UI
                        // sends a new geometry when it wants the view back.
                        self.display.desired = None;
                        self.mark_detached(link, &reason, out);
                    }
                }
                DisplayEvent::ReturnControlRequested { computer_id } => {
                    // The human asked (native pill / escape shortcut); Core
                    // decides — and returning control is always allowed.
                    if current == Some(computer_id) {
                        let _ = self.release_control(out);
                    }
                }
                DisplayEvent::Reconfigured { .. } => {
                    // Scanout size is informational here; the guest's own
                    // graphical-session report carries the size Core shows.
                }
                DisplayEvent::Failed {
                    computer_id,
                    message,
                } => {
                    // Either the answer to a pending attach or a failure of
                    // the live view; both leave Core without a view.
                    let answered_attach = self.display.awaiting_attach > 0;
                    self.display.awaiting_attach = self.display.awaiting_attach.saturating_sub(1);
                    let stale = answered_attach && self.display.awaiting_attach > 0;
                    if answered_attach && !stale {
                        // The latest attach failed: nothing exists for any
                        // later detach to remove, so no echo is owed.
                        self.display.awaiting_detach = 0;
                    }
                    if current == Some(computer_id) && !stale {
                        self.display.failure = Some(message.clone());
                        let link = self.display.link;
                        if link != DisplayLink::Detached {
                            let _ = self.release_control(out);
                            self.mark_detached(link, &format!("display_failed: {message}"), out);
                        }
                    }
                }
            }
        }
    }

    /// Input back to Pegoles (Return / auto-return). No-op when nobody
    /// controls. Ownership ALWAYS becomes None; a native failure to stop
    /// routing input is recorded in `last_error` and returned after the
    /// state change.
    pub(crate) fn release_control(&mut self, out: &mut Vec<AgentEvent>) -> Result<()> {
        if self.display.control == ControlOwner::None {
            return Ok(());
        }
        let native = if self.display.link != DisplayLink::Detached {
            self.display.backend.set_interactive(false)
        } else {
            Ok(())
        };
        let id = self.current_id().or(match self.display.link {
            DisplayLink::Attaching(id) | DisplayLink::Attached(id) => Some(id),
            DisplayLink::Detached => None,
        });
        match id {
            Some(id) => self.set_control(id, ControlOwner::None, out),
            None => self.display.control = ControlOwner::None,
        }
        native.map_err(|e| {
            self.display.last_error = Some(e.to_string());
            CoreError::Computer(e)
        })
    }

    /// The ONLY writer of `control`. Phase 4 callers pass `User` (take
    /// control) or `None` (return / auto-return); Phase 5's executor is
    /// the sole path that sets `Agent` (policy-checked structured
    /// actions only — see `crate::input`).
    pub(crate) fn set_control(
        &mut self,
        computer_id: ComputerId,
        to: ControlOwner,
        out: &mut Vec<AgentEvent>,
    ) {
        let from = self.display.control;
        if from == to {
            return;
        }
        self.display.control = to;
        self.emit(
            AgentEvent::ControlOwnershipChanged {
                computer_id,
                from,
                to,
                at: Utc::now(),
            },
            out,
        );
    }
}

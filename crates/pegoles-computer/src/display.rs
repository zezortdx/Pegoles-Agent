//! Display abstraction (Phase 4): how a computer's framebuffer reaches a
//! host window, independent of the hypervisor.
//!
//! ```text
//! ComputerDisplayBackend
//!   macOS    -> MacVirtualMachineDisplay  (native/macos/pegoles-macos-embed)
//!   Windows  -> WindowsHyperVDisplay      (FUTURE — not implemented)
//!   tests    -> TestDisplay
//!   elsewhere-> UnavailableDisplay
//! ```
//!
//! The shared UI only knows `ComputerViewport` + a geometry rectangle; it
//! never sees a native view type. React reports where the framebuffer
//! slot is (CSS px, relative to the webview's top-left) on layout changes
//! only — never per animation frame — and the native layer positions the
//! real framebuffer view there.
//!
//! SECURITY: this trait routes HUMAN input only (`set_interactive` lets
//! the person at the keyboard reach the isolated guest). There is
//! deliberately no API to synthesize pointer or keyboard events: model
//! input does not exist in Phase 4 and, when it arrives, goes through
//! structured actions + Pegoles Policy, never through this trait.

use pegoles_protocol::ComputerId;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{ComputerError, Result};

/// Rectangle in CSS pixels relative to the webview's top-left corner.
/// (WKWebView/WebView2 at zoom 1: 1 CSS px == 1 point / DIP.)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where (and whether) the framebuffer view should appear.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayGeometry {
    pub rect: DisplayRect,
    /// False when the viewport is unmounted/occluded (e.g. a modal covers
    /// it, or the user navigated away). The view is hidden, not destroyed.
    pub visible: bool,
    /// Native-side animation to the new rect (0 = jump). Lets a layout
    /// transition animate natively without per-frame IPC.
    #[serde(default)]
    pub animate_ms: u32,
}

/// Geometry rejection reasons (untrusted UI input is still validated).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryError {
    NotFinite,
    Negative,
    Empty,
    AnimationTooLong,
}

/// Longest native animation a geometry update may request.
pub const MAX_GEOMETRY_ANIMATION_MS: u32 = 1_000;

impl DisplayGeometry {
    /// Validate + normalize: finite, non-negative, non-empty, clamped to
    /// `bounds` (the webview size in CSS px), snapped to device pixels.
    pub fn sanitize(
        &self,
        bounds_width: f64,
        bounds_height: f64,
        scale: f64,
    ) -> std::result::Result<DisplayGeometry, GeometryError> {
        let r = self.rect;
        for v in [
            r.x,
            r.y,
            r.width,
            r.height,
            bounds_width,
            bounds_height,
            scale,
        ] {
            if !v.is_finite() {
                return Err(GeometryError::NotFinite);
            }
        }
        if r.width < 0.0 || r.height < 0.0 {
            return Err(GeometryError::Negative);
        }
        if self.animate_ms > MAX_GEOMETRY_ANIMATION_MS {
            return Err(GeometryError::AnimationTooLong);
        }
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let snap = |v: f64| (v * scale).round() / scale;
        let x0 = snap(r.x.clamp(0.0, bounds_width.max(0.0)));
        let y0 = snap(r.y.clamp(0.0, bounds_height.max(0.0)));
        let x1 = snap((r.x + r.width).clamp(0.0, bounds_width.max(0.0)));
        let y1 = snap((r.y + r.height).clamp(0.0, bounds_height.max(0.0)));
        let (w, h) = (x1 - x0, y1 - y0);
        if self.visible && (w < 1.0 || h < 1.0) {
            return Err(GeometryError::Empty);
        }
        Ok(DisplayGeometry {
            rect: DisplayRect {
                x: x0,
                y: y0,
                width: w.max(0.0),
                height: h.max(0.0),
            },
            visible: self.visible,
            animate_ms: self.animate_ms,
        })
    }

    /// Whether `next` differs enough from `self` to be worth a native
    /// update (sub-pixel jitter from ResizeObserver is dropped).
    pub fn differs_from(&self, next: &DisplayGeometry, epsilon: f64) -> bool {
        self.visible != next.visible
            || (self.rect.x - next.rect.x).abs() > epsilon
            || (self.rect.y - next.rect.y).abs() > epsilon
            || (self.rect.width - next.rect.width).abs() > epsilon
            || (self.rect.height - next.rect.height).abs() > epsilon
    }
}

/// Visual treatment of the native view (React cannot paint over it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPresentation {
    Normal,
    /// Paused computer: reduced luminance, last frame kept.
    Dimmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayBackendKind {
    /// No display on this host/backend (Mock, Linux, Windows today).
    Unavailable,
    /// macOS: native framebuffer view in the Pegoles window.
    MacVirtualMachine,
    /// FUTURE: Windows Hyper-V display. Not implemented.
    WindowsHyperV,
    /// Unit-test fake.
    Test,
}

/// Host-side attachment of the framebuffer view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DisplayAttachment {
    Detached,
    Attached {
        computer_id: ComputerId,
        visible: bool,
        interactive: bool,
    },
}

/// Facts the native display layer reports back to Core.
///
/// ORDERING CONTRACT (adapters must honor it; Core relies on it to tell
/// echoes of its own requests from new facts): facts are delivered in the
/// order the requests were issued (FIFO), and
/// - every successful `attach` is answered by exactly one `Attached` or
///   one `Failed`;
/// - every `detach` issued while a view exists or is being created is
///   answered by exactly one `Detached` (a `detach` with nothing to remove
///   emits nothing);
/// - a view removed by the native side on its own (window closed, view
///   lost) is reported as an unsolicited `Detached` / `Failed`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DisplayEvent {
    Attached {
        computer_id: ComputerId,
    },
    Detached {
        computer_id: ComputerId,
        reason: String,
    },
    /// The human asked (native pill button / escape shortcut) to hand
    /// input back. Core decides; the display only reports the request.
    ReturnControlRequested {
        computer_id: ComputerId,
    },
    /// Guest changed its scanout size.
    Reconfigured {
        computer_id: ComputerId,
        width_px: u32,
        height_px: u32,
    },
    Failed {
        computer_id: ComputerId,
        message: String,
    },
}

/// Platform display adapter. One instance per app window; at most one
/// computer attached at a time (Phase 4 owns a single computer).
///
/// Threading: implementations must be callable from any non-UI thread
/// and marshal native work onto the UI thread themselves.
pub trait ComputerDisplayBackend: Send {
    fn kind(&self) -> DisplayBackendKind;
    /// Whether this backend can show a framebuffer at all.
    fn is_available(&self) -> bool;
    fn attachment(&self) -> DisplayAttachment;
    /// Create (or re-show) the framebuffer view for `computer_id` at
    /// `geometry`. The computer must be running with a display device.
    fn attach(&mut self, computer_id: ComputerId, geometry: DisplayGeometry) -> Result<()>;
    fn set_geometry(&mut self, geometry: DisplayGeometry) -> Result<()>;
    /// Route HUMAN keyboard/pointer input to the guest (Take Control) or
    /// stop routing it (Return to Pegoles). Never synthesizes input.
    fn set_interactive(&mut self, interactive: bool) -> Result<()>;
    fn set_presentation(&mut self, presentation: DisplayPresentation) -> Result<()>;
    /// Destroy the native view (stop/destroy/explicit). Idempotent.
    fn detach(&mut self, reason: &str) -> Result<()>;
    /// Non-blocking drain of native facts since the last call.
    fn drain_events(&mut self) -> Vec<DisplayEvent>;
    /// Native framebuffer views currently alive (leak checks: must be 0
    /// after detach).
    fn live_native_views(&self) -> usize;
}

/// Backend for hosts without a display implementation. Every mutating
/// call fails explicitly — never a fake framebuffer.
#[derive(Debug, Default)]
pub struct UnavailableDisplay;

impl UnavailableDisplay {
    fn unavailable<T>() -> Result<T> {
        Err(ComputerError::DisplayUnavailable(
            "no display backend on this host".to_string(),
        ))
    }
}

impl ComputerDisplayBackend for UnavailableDisplay {
    fn kind(&self) -> DisplayBackendKind {
        DisplayBackendKind::Unavailable
    }
    fn is_available(&self) -> bool {
        false
    }
    fn attachment(&self) -> DisplayAttachment {
        DisplayAttachment::Detached
    }
    fn attach(&mut self, _computer_id: ComputerId, _geometry: DisplayGeometry) -> Result<()> {
        Self::unavailable()
    }
    fn set_geometry(&mut self, _geometry: DisplayGeometry) -> Result<()> {
        Self::unavailable()
    }
    fn set_interactive(&mut self, _interactive: bool) -> Result<()> {
        Self::unavailable()
    }
    fn set_presentation(&mut self, _presentation: DisplayPresentation) -> Result<()> {
        Self::unavailable()
    }
    fn detach(&mut self, _reason: &str) -> Result<()> {
        Ok(())
    }
    fn drain_events(&mut self) -> Vec<DisplayEvent> {
        Vec::new()
    }
    fn live_native_views(&self) -> usize {
        0
    }
}

/// One call recorded by [`TestDisplay`].
#[derive(Clone, Debug, PartialEq)]
pub enum TestDisplayCall {
    Attach {
        computer_id: ComputerId,
        geometry: DisplayGeometry,
    },
    SetGeometry(DisplayGeometry),
    SetInteractive(bool),
    SetPresentation(DisplayPresentation),
    Detach(String),
}

/// Inspectable state behind a [`TestDisplay`].
#[derive(Debug)]
pub struct TestDisplayState {
    pub calls: Vec<TestDisplayCall>,
    pub attachment: DisplayAttachment,
    pub presentation: DisplayPresentation,
    pub geometry: Option<DisplayGeometry>,
    pub live_views: usize,
    /// Events queued for the next `drain_events` (scripted or automatic).
    pub pending: VecDeque<DisplayEvent>,
    /// The next `attach` fails with this message (then clears).
    pub fail_next_attach: Option<String>,
    /// Queue `Attached`/`Detached` facts like the native layer does after
    /// it really created/removed the view (default true). Off = the test
    /// scripts every fact itself via `push_event`.
    pub auto_events: bool,
    pub available: bool,
}

/// Recording, scriptable display adapter for Core tests (no window, no
/// native view). Cloning shares state: the test keeps one handle and
/// installs the other into the registry. Mirrors the native contract:
/// one attached computer at a time, idempotent detach, facts reported
/// through `drain_events`, and NO way to synthesize input.
#[derive(Clone, Debug)]
pub struct TestDisplay {
    state: Arc<Mutex<TestDisplayState>>,
}

impl Default for TestDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl TestDisplay {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(TestDisplayState {
                calls: Vec::new(),
                attachment: DisplayAttachment::Detached,
                presentation: DisplayPresentation::Normal,
                geometry: None,
                live_views: 0,
                pending: VecDeque::new(),
                fail_next_attach: None,
                auto_events: true,
                available: true,
            })),
        }
    }

    /// Lock the shared state (poison-tolerant: a panicking test thread
    /// must not cascade into unrelated assertions).
    pub fn state(&self) -> MutexGuard<'_, TestDisplayState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue a native fact (e.g. `ReturnControlRequested`).
    pub fn push_event(&self, event: DisplayEvent) {
        self.state().pending.push_back(event);
    }

    pub fn calls(&self) -> Vec<TestDisplayCall> {
        self.state().calls.clone()
    }

    pub fn clear_calls(&self) {
        self.state().calls.clear();
    }

    pub fn fail_next_attach(&self, message: &str) {
        self.state().fail_next_attach = Some(message.to_string());
    }

    pub fn set_auto_events(&self, on: bool) {
        self.state().auto_events = on;
    }

    pub fn set_available(&self, available: bool) {
        self.state().available = available;
    }

    pub fn is_interactive(&self) -> bool {
        matches!(
            self.state().attachment,
            DisplayAttachment::Attached {
                interactive: true,
                ..
            }
        )
    }

    fn not_attached<T>() -> Result<T> {
        Err(ComputerError::DisplayUnavailable(
            "no framebuffer view attached".to_string(),
        ))
    }
}

impl ComputerDisplayBackend for TestDisplay {
    fn kind(&self) -> DisplayBackendKind {
        DisplayBackendKind::Test
    }
    fn is_available(&self) -> bool {
        self.state().available
    }
    fn attachment(&self) -> DisplayAttachment {
        self.state().attachment.clone()
    }
    fn attach(&mut self, computer_id: ComputerId, geometry: DisplayGeometry) -> Result<()> {
        let mut st = self.state();
        st.calls.push(TestDisplayCall::Attach {
            computer_id,
            geometry,
        });
        if !st.available {
            return Err(ComputerError::DisplayUnavailable(
                "test display disabled".to_string(),
            ));
        }
        if let Some(message) = st.fail_next_attach.take() {
            return Err(ComputerError::Backend(message));
        }
        if let DisplayAttachment::Attached {
            computer_id: other, ..
        } = st.attachment
        {
            if other != computer_id {
                return Err(ComputerError::DisplayUnavailable(
                    "another computer is attached".to_string(),
                ));
            }
        }
        let interactive = matches!(
            st.attachment,
            DisplayAttachment::Attached {
                interactive: true,
                ..
            }
        );
        let was_detached = st.attachment == DisplayAttachment::Detached;
        st.attachment = DisplayAttachment::Attached {
            computer_id,
            visible: geometry.visible,
            interactive,
        };
        st.geometry = Some(geometry);
        st.live_views = 1;
        if was_detached && st.auto_events {
            st.pending.push_back(DisplayEvent::Attached { computer_id });
        }
        Ok(())
    }
    fn set_geometry(&mut self, geometry: DisplayGeometry) -> Result<()> {
        let mut st = self.state();
        st.calls.push(TestDisplayCall::SetGeometry(geometry));
        match st.attachment {
            DisplayAttachment::Detached => Self::not_attached(),
            DisplayAttachment::Attached {
                computer_id,
                interactive,
                ..
            } => {
                st.attachment = DisplayAttachment::Attached {
                    computer_id,
                    visible: geometry.visible,
                    interactive,
                };
                st.geometry = Some(geometry);
                Ok(())
            }
        }
    }
    fn set_interactive(&mut self, interactive: bool) -> Result<()> {
        let mut st = self.state();
        st.calls.push(TestDisplayCall::SetInteractive(interactive));
        match st.attachment {
            DisplayAttachment::Detached if interactive => Self::not_attached(),
            DisplayAttachment::Detached => Ok(()),
            DisplayAttachment::Attached {
                computer_id,
                visible,
                ..
            } => {
                st.attachment = DisplayAttachment::Attached {
                    computer_id,
                    visible,
                    interactive,
                };
                Ok(())
            }
        }
    }
    fn set_presentation(&mut self, presentation: DisplayPresentation) -> Result<()> {
        let mut st = self.state();
        st.calls
            .push(TestDisplayCall::SetPresentation(presentation));
        if st.attachment == DisplayAttachment::Detached {
            return Self::not_attached();
        }
        st.presentation = presentation;
        Ok(())
    }
    fn detach(&mut self, reason: &str) -> Result<()> {
        let mut st = self.state();
        st.calls.push(TestDisplayCall::Detach(reason.to_string()));
        if let DisplayAttachment::Attached { computer_id, .. } = st.attachment {
            st.attachment = DisplayAttachment::Detached;
            st.live_views = 0;
            st.presentation = DisplayPresentation::Normal;
            if st.auto_events {
                st.pending.push_back(DisplayEvent::Detached {
                    computer_id,
                    reason: reason.to_string(),
                });
            }
        }
        Ok(())
    }
    fn drain_events(&mut self) -> Vec<DisplayEvent> {
        self.state().pending.drain(..).collect()
    }
    fn live_native_views(&self) -> usize {
        self.state().live_views
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo(x: f64, y: f64, w: f64, h: f64) -> DisplayGeometry {
        DisplayGeometry {
            rect: DisplayRect {
                x,
                y,
                width: w,
                height: h,
            },
            visible: true,
            animate_ms: 0,
        }
    }

    #[test]
    fn sanitize_clamps_and_snaps() {
        let g = geo(10.26, -4.0, 2000.0, 300.4)
            .sanitize(1200.0, 800.0, 2.0)
            .unwrap();
        assert_eq!(g.rect.x, 10.5);
        assert_eq!(g.rect.y, 0.0);
        assert_eq!(g.rect.width, 1200.0 - 10.5);
        assert_eq!(g.rect.height, 296.5);
    }

    #[test]
    fn sanitize_rejects_garbage() {
        assert_eq!(
            geo(f64::NAN, 0.0, 1.0, 1.0).sanitize(10.0, 10.0, 1.0),
            Err(GeometryError::NotFinite)
        );
        assert_eq!(
            geo(0.0, 0.0, -1.0, 5.0).sanitize(10.0, 10.0, 1.0),
            Err(GeometryError::Negative)
        );
        assert_eq!(
            geo(50.0, 50.0, 10.0, 10.0).sanitize(10.0, 10.0, 1.0),
            Err(GeometryError::Empty)
        );
        let mut slow = geo(0.0, 0.0, 5.0, 5.0);
        slow.animate_ms = MAX_GEOMETRY_ANIMATION_MS + 1;
        assert_eq!(
            slow.sanitize(10.0, 10.0, 1.0),
            Err(GeometryError::AnimationTooLong)
        );
    }

    #[test]
    fn hidden_geometry_may_be_empty() {
        let mut g = geo(0.0, 0.0, 0.0, 0.0);
        g.visible = false;
        assert!(g.sanitize(10.0, 10.0, 1.0).is_ok());
    }

    #[test]
    fn jitter_is_not_a_change() {
        let a = geo(10.0, 10.0, 100.0, 100.0);
        let b = geo(10.2, 10.0, 100.1, 100.0);
        assert!(!a.differs_from(&b, 0.5));
        let mut c = b;
        c.visible = false;
        assert!(a.differs_from(&c, 0.5));
    }

    /// SECURITY guard: the display adapter routes HUMAN input only. No
    /// function in this module (trait, adapters, test double) may look
    /// like a way to synthesize pointer/keyboard input.
    #[test]
    fn display_api_has_no_synthetic_input() {
        let src = include_str!("display.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or("");
        let names: Vec<String> = production
            .split("fn ")
            .skip(1)
            .map(|rest| {
                rest.chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
                    .to_lowercase()
            })
            .filter(|n| !n.is_empty())
            .collect();
        assert!(names.iter().any(|n| n == "set_interactive"), "scan broken");
        for name in &names {
            for forbidden in [
                "key",
                "inject",
                "click",
                "type_text",
                "pointer",
                "mouse",
                "synth",
                "scroll",
                "send_input",
                "press",
                "cursor",
                "touch",
            ] {
                assert!(
                    !name.contains(forbidden),
                    "display API grew an input-synthesis-like fn: {name}"
                );
            }
        }
    }

    #[test]
    fn test_display_records_and_reports_like_native() {
        let display = TestDisplay::new();
        let mut installed: Box<dyn ComputerDisplayBackend> = Box::new(display.clone());
        let id = ComputerId::new();
        installed.attach(id, geo(0.0, 0.0, 100.0, 60.0)).unwrap();
        assert_eq!(installed.live_native_views(), 1);
        assert_eq!(
            installed.drain_events(),
            vec![DisplayEvent::Attached { computer_id: id }]
        );
        installed.set_interactive(true).unwrap();
        assert!(display.is_interactive());
        installed
            .set_presentation(DisplayPresentation::Dimmed)
            .unwrap();
        installed.detach("stop").unwrap();
        installed.detach("stop").unwrap(); // idempotent, one fact only
        assert_eq!(installed.live_native_views(), 0);
        assert_eq!(installed.drain_events().len(), 1);
        assert!(installed.set_geometry(geo(0.0, 0.0, 1.0, 1.0)).is_err());
        assert!(installed.set_interactive(true).is_err());
        assert_eq!(display.calls().len(), 7);
        // Only one computer at a time.
        installed.attach(id, geo(0.0, 0.0, 10.0, 10.0)).unwrap();
        assert!(installed
            .attach(ComputerId::new(), geo(0.0, 0.0, 10.0, 10.0))
            .is_err());
    }

    #[test]
    fn wire_names_are_stable() {
        let names: Vec<String> = [
            DisplayBackendKind::Unavailable,
            DisplayBackendKind::MacVirtualMachine,
            DisplayBackendKind::WindowsHyperV,
            DisplayBackendKind::Test,
        ]
        .iter()
        .map(|k| {
            serde_json::to_value(k)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
        assert_eq!(
            names,
            [
                "unavailable",
                "mac_virtual_machine",
                "windows_hyper_v",
                "test"
            ]
        );
        let g: DisplayGeometry =
            serde_json::from_str(r#"{"rect":{"x":1,"y":2,"width":3,"height":4},"visible":true}"#)
                .unwrap();
        assert_eq!(g.animate_ms, 0);
    }

    #[test]
    fn unavailable_display_never_fakes() {
        let mut d = UnavailableDisplay;
        assert!(!d.is_available());
        assert!(d
            .attach(ComputerId::new(), geo(0.0, 0.0, 10.0, 10.0))
            .is_err());
        assert!(d.set_interactive(true).is_err());
        assert_eq!(d.live_native_views(), 0);
        assert!(d.detach("test").is_ok());
    }
}

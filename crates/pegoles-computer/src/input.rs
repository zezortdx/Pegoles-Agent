//! Platform-independent computer input + observation layer (Phase 5).
//!
//! ```text
//! ComputerAction (agent space, normalized)
//!   → action_to_input_ops (DisplayTransform to guest pixels)
//!     → ComputerInputBackend::execute (guest pixels only)
//!       → GuestChannel (vsock HostMessage::Input / GetFrame)
//!       → TestInput (tests/dev) / UnavailableInput / WindowsInputStub
//! ```
//!
//! SECURITY: this module contains NO host input API (no global event
//! synthesis, no host process control) and NO host screen capture. Guest
//! pixels travel to the isolated VM over the guest control plane only.
//! Guard tests below fail the build if host-input/capture tokens appear.

use std::collections::{HashMap, VecDeque};

use pegoles_guest_proto::{GuestButton, GuestInputOp};
use pegoles_protocol::{
    limits, ComputerAction, ComputerId, FrameEncoding, FrameId, ObservedFrameMeta, PointerButton,
};

use crate::coords::{AgentPoint, DisplayTransform, GuestPoint};
use crate::error::{ComputerError, Result};

/// Backend flavor. `GuestChannel` serves macOS AND Windows through the
/// same vsock guest protocol; only the transport below differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputBackendKind {
    Unavailable,
    Test,
    GuestChannel,
    WindowsHcs,
}

/// What an input backend can do (surfaced to Core/UI for honest gating).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputCapabilities {
    pub pointer: bool,
    pub keyboard: bool,
    pub screenshot: bool,
    pub max_text_len: usize,
    pub kind: InputBackendKind,
}

/// One primitive in GUEST pixels. Multi-step actions (drag path,
/// chord order) are expanded by the guest or the op itself — never by
/// teleporting: `Drag` carries its duration for guest-side interpolation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputOp {
    Move {
        point: GuestPoint,
    },
    Click {
        point: GuestPoint,
        button: PointerButton,
    },
    DoubleClick {
        point: GuestPoint,
        button: PointerButton,
    },
    Down {
        point: GuestPoint,
        button: PointerButton,
    },
    Up {
        point: GuestPoint,
        button: PointerButton,
    },
    Drag {
        from: GuestPoint,
        to: GuestPoint,
        button: PointerButton,
        duration_ms: u32,
    },
    Scroll {
        point: GuestPoint,
        /// Logical lines; +y scrolls content down.
        dx: i32,
        dy: i32,
    },
    KeyPress {
        key: String,
    },
    KeyChord {
        keys: Vec<String>,
    },
    TypeText {
        text: String,
    },
    CaptureFrame {
        request_id: String,
    },
    GetDisplayInfo,
}

impl InputOp {
    /// Short verb for audit rows.
    pub fn verb(&self) -> &'static str {
        match self {
            InputOp::Move { .. } => "move",
            InputOp::Click { .. } => "click",
            InputOp::DoubleClick { .. } => "double_click",
            InputOp::Down { .. } => "mouse_down",
            InputOp::Up { .. } => "mouse_up",
            InputOp::Drag { .. } => "drag",
            InputOp::Scroll { .. } => "scroll",
            InputOp::KeyPress { .. } => "key_press",
            InputOp::KeyChord { .. } => "key_chord",
            InputOp::TypeText { .. } => "type",
            InputOp::CaptureFrame { .. } => "capture",
            InputOp::GetDisplayInfo => "display_info",
        }
    }
}

/// Outcome of one backend primitive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOutcome {
    pub ok: bool,
    pub error: Option<String>,
    /// Backend round-trip time (send + guest ack).
    pub latency_ms: u64,
}

impl InputOutcome {
    pub fn ok(latency_ms: u64) -> Self {
        Self {
            ok: true,
            error: None,
            latency_ms,
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            latency_ms: 0,
        }
    }
}

/// Host-side pressed-state tracking: which guest buttons/modifiers this
/// host believes are DOWN. `release_all` generates the matching releases.
/// Checked on cancel, disconnect, pause, and ownership change so the
/// guest is never left with a stuck button or modifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PressedState {
    pub buttons: Vec<PointerButton>,
    pub modifiers: Vec<String>,
}

impl PressedState {
    pub fn is_clean(&self) -> bool {
        self.buttons.is_empty() && self.modifiers.is_empty()
    }

    pub fn press_button(&mut self, button: PointerButton) {
        if !self.buttons.contains(&button) {
            self.buttons.push(button);
        }
    }

    pub fn release_button(&mut self, button: PointerButton) {
        self.buttons.retain(|b| *b != button);
    }

    pub fn hold_modifier(&mut self, name: &str) {
        if !self.modifiers.iter().any(|m| m == name) {
            self.modifiers.push(name.to_string());
        }
    }

    pub fn release_modifier(&mut self, name: &str) {
        self.modifiers.retain(|m| m != name);
    }

    /// Release ops for everything held, at the given guest point.
    pub fn release_all_ops(&mut self, at: GuestPoint) -> Vec<InputOp> {
        let mut ops = Vec::new();
        for button in std::mem::take(&mut self.buttons) {
            ops.push(InputOp::Up { point: at, button });
        }
        self.modifiers.clear();
        ops
    }
}

/// Burst brake: at most N dispatches per rolling second. Injectable clock
/// (`now_ms`) keeps it unit-testable.
#[derive(Clone, Debug)]
pub struct ActionRateLimiter {
    per_sec: u32,
    window: VecDeque<u64>,
}

impl ActionRateLimiter {
    pub fn new(per_sec: u32) -> Self {
        Self {
            per_sec: per_sec.max(1),
            window: VecDeque::new(),
        }
    }

    pub fn check(&mut self, now_ms: u64) -> bool {
        while self
            .window
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= 1000)
        {
            self.window.pop_front();
        }
        if self.window.len() >= self.per_sec as usize {
            return false;
        }
        self.window.push_back(now_ms);
        true
    }
}

/// Lower one structured action to backend primitives using the CURRENT
/// display size. Exhaustive: only `Wait` (host-side timing) lowers to
/// nothing.
pub fn action_to_input_ops(action: &ComputerAction, t: &DisplayTransform) -> Vec<InputOp> {
    let pt = |x: f64, y: f64| t.agent_to_guest(AgentPoint { x, y });
    match action {
        ComputerAction::MovePointer { x, y } => vec![InputOp::Move { point: pt(*x, *y) }],
        ComputerAction::Click { x, y, button } => {
            vec![InputOp::Click {
                point: pt(*x, *y),
                button: *button,
            }]
        }
        ComputerAction::DoubleClick { x, y, button } => {
            vec![InputOp::DoubleClick {
                point: pt(*x, *y),
                button: *button,
            }]
        }
        ComputerAction::MouseDown { x, y, button } => {
            vec![InputOp::Down {
                point: pt(*x, *y),
                button: *button,
            }]
        }
        ComputerAction::MouseUp { x, y, button } => {
            vec![InputOp::Up {
                point: pt(*x, *y),
                button: *button,
            }]
        }
        ComputerAction::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
            duration_ms,
        } => {
            vec![InputOp::Drag {
                from: pt(*from_x, *from_y),
                to: pt(*to_x, *to_y),
                button: *button,
                duration_ms: (*duration_ms).min(limits::MAX_DRAG_MS),
            }]
        }
        ComputerAction::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => vec![InputOp::Scroll {
            point: pt(*x, *y),
            dx: (*delta_x)
                .round()
                .clamp(-limits::MAX_SCROLL_UNITS, limits::MAX_SCROLL_UNITS) as i32,
            dy: (*delta_y)
                .round()
                .clamp(-limits::MAX_SCROLL_UNITS, limits::MAX_SCROLL_UNITS) as i32,
        }],
        ComputerAction::KeyPress { key } => vec![InputOp::KeyPress { key: key.clone() }],
        ComputerAction::KeyChord { keys } => vec![InputOp::KeyChord { keys: keys.clone() }],
        ComputerAction::TypeText { text, .. } => vec![InputOp::TypeText { text: text.clone() }],
        // Pure host-side timing: no guest primitive.
        ComputerAction::Wait { .. } => Vec::new(),
        ComputerAction::ObserveScreen => {
            vec![InputOp::CaptureFrame {
                request_id: String::new(),
            }]
        }
        ComputerAction::GetDisplayInfo => vec![InputOp::GetDisplayInfo],
    }
}

/// Lower one backend op to guest-protocol primitives.
pub fn op_to_guest(op: &InputOp) -> Option<GuestInputOp> {
    let btn = |b: PointerButton| match b {
        PointerButton::Primary => GuestButton::Primary,
        PointerButton::Secondary => GuestButton::Secondary,
        PointerButton::Middle => GuestButton::Middle,
    };
    match op {
        InputOp::Move { point } => Some(GuestInputOp::Move {
            x: point.x,
            y: point.y,
        }),
        InputOp::Click { point, button } => Some(GuestInputOp::Click {
            x: point.x,
            y: point.y,
            button: btn(*button),
        }),
        InputOp::DoubleClick { point, button } => Some(GuestInputOp::DoubleClick {
            x: point.x,
            y: point.y,
            button: btn(*button),
        }),
        InputOp::Down { point, button } => Some(GuestInputOp::Press {
            x: point.x,
            y: point.y,
            button: btn(*button),
        }),
        InputOp::Up { point, button } => Some(GuestInputOp::Release {
            x: point.x,
            y: point.y,
            button: btn(*button),
        }),
        InputOp::Drag {
            from,
            to,
            button,
            duration_ms,
        } => Some(GuestInputOp::Drag {
            from_x: from.x,
            from_y: from.y,
            to_x: to.x,
            to_y: to.y,
            button: btn(*button),
            duration_ms: (*duration_ms).min(limits::MAX_DRAG_MS),
        }),
        InputOp::Scroll { point, dx, dy } => Some(GuestInputOp::Scroll {
            x: point.x,
            y: point.y,
            dx: *dx,
            dy: *dy,
        }),
        // A press is a TAP (down + up in one guest op). A bare
        // `Key { down: true }` stayed held and auto-repeated on hardware.
        InputOp::KeyPress { key } => Some(GuestInputOp::Chord {
            keys: vec![key.clone()],
        }),
        InputOp::KeyChord { keys } => Some(GuestInputOp::Chord { keys: keys.clone() }),
        InputOp::TypeText { text } => Some(GuestInputOp::Type { text: text.clone() }),
        InputOp::CaptureFrame { .. } | InputOp::GetDisplayInfo => None,
    }
}

/// Number of interpolation steps for a client-side drag preview path
/// (the guest itself interpolates over `duration_ms`; this bounds any
/// host-side preview/cursor path to the same budget).
pub fn drag_step_count(duration_ms: u32) -> usize {
    (duration_ms as usize / 16).clamp(2, limits::MAX_DRAG_STEPS)
}

/// Count pixels that differ inside a rectangle of two same-size RGBA
/// frames. The host's visual assertion path: click the fixture button,
/// capture again, require `changed > threshold` in the button region.
pub fn pixel_diff_in_rect(
    before: &[u8],
    after: &[u8],
    width_px: u32,
    rect: (u32, u32, u32, u32),
) -> Option<u64> {
    let (rx, ry, rw, rh) = rect;
    let height_px = before.len() as u64 / width_px as u64 / 4;
    if before.len() != after.len() || width_px == 0 || height_px == 0 {
        return None;
    }
    let height_px = height_px as u32;
    let mut changed = 0u64;
    for y in ry..(ry + rh).min(height_px) {
        for x in rx..(rx + rw).min(width_px) {
            let i = (y as usize * width_px as usize + x as usize) * 4;
            if before[i..i + 4] != after[i..i + 4] {
                changed += 1;
            }
        }
    }
    Some(changed)
}

/// Captured frame payload held out-of-band (never in events).
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub meta: ObservedFrameMeta,
    pub bytes: Vec<u8>,
}

/// Deduplicating frame store: repeated captures of an identical screen
/// reuse the previous frame id instead of minting new ones.
#[derive(Clone, Debug, Default)]
pub struct FrameCache {
    frames: HashMap<FrameId, CapturedFrame>,
    last_id: Option<FrameId>,
    order: VecDeque<FrameId>,
    capacity: usize,
}

impl FrameCache {
    pub fn new() -> Self {
        Self {
            frames: HashMap::new(),
            last_id: None,
            order: VecDeque::new(),
            capacity: 8,
        }
    }

    /// 64-bit FNV-1a over raw bytes. ~GB/s: a 5 MiB frame hashes in a few
    /// ms — cheap enough for on-demand capture, never for streaming.
    pub fn fingerprint(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    pub fn identical(a: &[u8], b: &[u8]) -> bool {
        a.len() == b.len() && Self::fingerprint(a) == Self::fingerprint(b)
    }

    /// Store a capture; returns the (possibly reused) frame id and whether
    /// the pixels were new.
    pub fn insert(
        &mut self,
        computer_id: ComputerId,
        width_px: u32,
        height_px: u32,
        encoding: FrameEncoding,
        bytes: Vec<u8>,
        capture_latency_ms: u64,
    ) -> (FrameId, bool) {
        if let Some(last) = self.last_id.and_then(|id| self.frames.get(&id)) {
            if last.meta.width_px == width_px
                && last.meta.height_px == height_px
                && last.meta.encoding == encoding
                && Self::identical(&last.bytes, &bytes)
            {
                return (last.meta.frame_id, false);
            }
        }
        let frame_id = FrameId::new();
        let byte_len = bytes.len() as u64;
        self.frames.insert(
            frame_id,
            CapturedFrame {
                meta: ObservedFrameMeta {
                    frame_id,
                    computer_id,
                    captured_at: chrono::Utc::now(),
                    width_px,
                    height_px,
                    encoding,
                    byte_len,
                    capture_latency_ms,
                },
                bytes,
            },
        );
        self.order.push_back(frame_id);
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.frames.remove(&old);
            }
        }
        self.last_id = Some(frame_id);
        (frame_id, true)
    }

    pub fn get(&self, id: &FrameId) -> Option<&CapturedFrame> {
        self.frames.get(id)
    }

    pub fn last(&self) -> Option<&CapturedFrame> {
        self.last_id.and_then(|id| self.frames.get(&id))
    }
}

/// Encode raw RGBA pixels as PNG (manual debugging path; model pipelines
/// consume raw bytes directly).
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(ComputerError::Backend("rgba stride mismatch".to_string()));
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
    }
    Ok(out)
}

/// Encode raw RGBA pixels as an RGB PNG tuned for latency: the guest
/// framebuffer is opaque (alpha carries nothing) and the fast deflate
/// mode keeps encoding off the critical path of every agent turn and
/// every preview refresh.
pub fn encode_png_rgb_fast(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(ComputerError::Backend("rgba stride mismatch".to_string()));
    }
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|px| [px[0], px[1], px[2]])
        .collect();
    let mut out = Vec::with_capacity(rgb.len() / 8);
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc
            .write_header()
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
        writer
            .write_image_data(&rgb)
            .map_err(|e| ComputerError::Backend(e.to_string()))?;
    }
    Ok(out)
}

/// Encode PNG bytes as base64 (Tauri/dev-panel transport).
pub fn base64_png(png: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(png)
}

/// Split bytes into base64 vsock chunks (each safely under 64 KiB frames).
pub fn chunk_bytes(bytes: &[u8]) -> Vec<String> {
    use base64::Engine;
    bytes
        .chunks(limits::FRAME_CHUNK_BYTES)
        .map(|c| base64::engine::general_purpose::STANDARD.encode(c))
        .collect()
}

/// Reassemble base64 chunks in order. Rejects oversized assemblies.
pub fn reassemble_chunks(chunks: &[String]) -> Result<Vec<u8>> {
    use base64::Engine;
    if chunks.len() as u32 > pegoles_guest_proto_cap() {
        return Err(ComputerError::Backend(
            "frame exceeds chunk cap".to_string(),
        ));
    }
    let mut out = Vec::new();
    for c in chunks {
        if c.len() > pegoles_guest_proto::MAX_FRAME_CHUNK_B64 {
            return Err(ComputerError::Backend("chunk exceeds bound".to_string()));
        }
        let raw = base64::engine::general_purpose::STANDARD
            .decode(c)
            .map_err(|e| ComputerError::Backend(format!("bad chunk encoding: {e}")))?;
        out.extend_from_slice(&raw);
    }
    Ok(out)
}

fn pegoles_guest_proto_cap() -> u32 {
    pegoles_guest_proto::MAX_FRAME_CHUNKS
}

/// Platform-independent input backend. Implementations must NEVER touch
/// host input devices or the host screen: guest pixels in, guest
/// acknowledgements out.
pub trait ComputerInputBackend: Send {
    fn kind(&self) -> InputBackendKind;
    fn is_available(&self) -> bool;
    fn capabilities(&self) -> InputCapabilities;
    fn execute(&mut self, op: &InputOp) -> InputOutcome;
    /// Release everything held (buttons, modifiers). Idempotent.
    fn release_all(&mut self);
    fn pressed_state(&self) -> PressedState;
    fn drain_events(&mut self) -> Vec<String>;
}

/// Always-unavailable backend (headless, mock, or pre-handshake).
pub struct UnavailableInput {
    reason: String,
}

impl UnavailableInput {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl ComputerInputBackend for UnavailableInput {
    fn kind(&self) -> InputBackendKind {
        InputBackendKind::Unavailable
    }

    fn is_available(&self) -> bool {
        false
    }

    fn capabilities(&self) -> InputCapabilities {
        InputCapabilities {
            pointer: false,
            keyboard: false,
            screenshot: false,
            max_text_len: 0,
            kind: InputBackendKind::Unavailable,
        }
    }

    fn execute(&mut self, _op: &InputOp) -> InputOutcome {
        InputOutcome::failed(format!("input unavailable: {}", self.reason))
    }

    fn release_all(&mut self) {}

    fn pressed_state(&self) -> PressedState {
        PressedState::default()
    }

    fn drain_events(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Recording backend for unit tests and the deterministic runner's
/// logic tests. Executes instantly with real pressed-state tracking.
pub struct TestInput {
    pub recorded: Vec<InputOp>,
    pub available: bool,
    pub latency_ms: u64,
    pub fail_with: Option<String>,
    /// When set, CaptureFrame returns this deterministic test pattern.
    pub frame_pattern: Option<(u32, u32)>,
    pub last_frame: Option<CapturedFrame>,
    pressed: PressedState,
    computer_id: ComputerId,
}

impl TestInput {
    pub fn new(computer_id: ComputerId) -> Self {
        Self {
            recorded: Vec::new(),
            available: true,
            latency_ms: 0,
            fail_with: None,
            frame_pattern: Some((64, 36)),
            last_frame: None,
            pressed: PressedState::default(),
            computer_id,
        }
    }

    /// Deterministic gradient pattern (top-left black → bottom-right
    /// white with a red diagonal) for cache/PNG/round-trip tests.
    pub fn test_pattern_rgba(width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                let r = if x == y * width / height.max(1) {
                    255
                } else {
                    0
                };
                px.push((x * 255 / width.max(1)) as u8);
                px.push((y * 255 / height.max(1)) as u8);
                px.push(128);
                px.push(r.max(64));
            }
        }
        px
    }
}

impl ComputerInputBackend for TestInput {
    fn kind(&self) -> InputBackendKind {
        InputBackendKind::Test
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn capabilities(&self) -> InputCapabilities {
        InputCapabilities {
            pointer: true,
            keyboard: true,
            screenshot: true,
            max_text_len: limits::MAX_TYPE_CHARS,
            kind: InputBackendKind::Test,
        }
    }

    fn execute(&mut self, op: &InputOp) -> InputOutcome {
        if !self.available {
            return InputOutcome::failed("test input disabled");
        }
        if let Some(reason) = self.fail_with.clone() {
            return InputOutcome::failed(reason);
        }
        match op {
            InputOp::Down { button, .. } => self.pressed.press_button(*button),
            InputOp::Up { button, .. } => self.pressed.release_button(*button),
            InputOp::CaptureFrame { .. } => {
                let (w, h) = self.frame_pattern.unwrap_or((64, 36));
                let bytes = Self::test_pattern_rgba(w, h);
                let mut cache = FrameCache::new();
                let (id, _) = cache.insert(
                    self.computer_id,
                    w,
                    h,
                    FrameEncoding::RawRgba,
                    bytes,
                    self.latency_ms,
                );
                self.last_frame = cache.get(&id).cloned();
            }
            _ => {}
        }
        self.recorded.push(op.clone());
        InputOutcome::ok(self.latency_ms)
    }

    fn release_all(&mut self) {
        let at = GuestPoint { x: 0, y: 0 };
        for op in self.pressed.release_all_ops(at) {
            self.recorded.push(op);
        }
    }

    fn pressed_state(&self) -> PressedState {
        self.pressed.clone()
    }

    fn drain_events(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Future Windows adapter skeleton. Honest by construction: reports
/// unavailable until the HCS input path lands. Shared action/protocol
/// layers stay platform-independent above it.
pub struct WindowsInputStub;

impl ComputerInputBackend for WindowsInputStub {
    fn kind(&self) -> InputBackendKind {
        InputBackendKind::WindowsHcs
    }

    fn is_available(&self) -> bool {
        false
    }

    fn capabilities(&self) -> InputCapabilities {
        InputCapabilities {
            pointer: false,
            keyboard: false,
            screenshot: false,
            max_text_len: 0,
            kind: InputBackendKind::WindowsHcs,
        }
    }

    fn execute(&mut self, _op: &InputOp) -> InputOutcome {
        InputOutcome::failed("Windows agent input lands with the HCS input backend")
    }

    fn release_all(&mut self) {}

    fn pressed_state(&self) -> PressedState {
        PressedState::default()
    }

    fn drain_events(&mut self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t1440() -> DisplayTransform {
        DisplayTransform::headless(1440, 900)
    }

    #[test]
    fn limits_mirror_protocol() {
        assert_eq!(limits::MAX_DRAG_MS, 10_000);
        assert_eq!(limits::MAX_SCROLL_UNITS, 100.0);
        assert_eq!(limits::MAX_TYPE_CHARS, 4096);
        assert_eq!(limits::MAX_WAIT_MS, 30_000);
        assert_eq!(limits::MAX_ACTIONS_PER_SEC, 20);
    }

    #[test]
    fn actions_lower_to_guest_pixels() {
        let t = t1440();
        let ops = action_to_input_ops(
            &ComputerAction::Click {
                x: 0.5,
                y: 0.5,
                button: PointerButton::Primary,
            },
            &t,
        );
        assert_eq!(
            ops,
            vec![InputOp::Click {
                point: GuestPoint { x: 720, y: 450 },
                button: PointerButton::Primary,
            }]
        );
        // Wait is host-side timing only.
        assert!(action_to_input_ops(&ComputerAction::Wait { duration_ms: 100 }, &t).is_empty());
    }

    #[test]
    fn drag_caps_duration_and_scroll_rounds() {
        let t = t1440();
        let ops = action_to_input_ops(
            &ComputerAction::Drag {
                from_x: 0.0,
                from_y: 0.0,
                to_x: 1.0,
                to_y: 1.0,
                button: PointerButton::Primary,
                duration_ms: 60_000,
            },
            &t,
        );
        assert!(matches!(
            ops[0],
            InputOp::Drag { duration_ms, .. } if duration_ms == limits::MAX_DRAG_MS
        ));
        let ops = action_to_input_ops(
            &ComputerAction::Scroll {
                x: 0.5,
                y: 0.5,
                delta_x: 0.4,
                delta_y: -2.6,
            },
            &t,
        );
        assert_eq!(
            ops[0],
            InputOp::Scroll {
                point: GuestPoint { x: 720, y: 450 },
                dx: 0,
                dy: -3,
            }
        );
    }

    #[test]
    fn ops_lower_to_guest_protocol() {
        let op = InputOp::Click {
            point: GuestPoint { x: 10, y: 20 },
            button: PointerButton::Secondary,
        };
        assert_eq!(
            op_to_guest(&op),
            Some(GuestInputOp::Click {
                x: 10,
                y: 20,
                button: GuestButton::Secondary
            })
        );
        assert_eq!(op_to_guest(&InputOp::GetDisplayInfo), None);
        // KeyPress is a tap: never a bare held key-down.
        assert_eq!(
            op_to_guest(&InputOp::KeyPress {
                key: "Enter".into()
            }),
            Some(GuestInputOp::Chord {
                keys: vec!["Enter".into()]
            })
        );
    }

    #[test]
    fn drag_steps_bounded() {
        assert_eq!(drag_step_count(0), 2);
        assert_eq!(drag_step_count(400), 25);
        assert_eq!(drag_step_count(60_000), limits::MAX_DRAG_STEPS);
    }

    #[test]
    fn pressed_state_tracks_and_releases() {
        let mut s = PressedState::default();
        assert!(s.is_clean());
        s.press_button(PointerButton::Primary);
        s.hold_modifier("Shift");
        assert!(!s.is_clean());
        let ops = s.release_all_ops(GuestPoint { x: 1, y: 2 });
        assert_eq!(ops.len(), 1);
        assert!(s.is_clean());
        // Idempotent: second release yields nothing.
        assert!(s.release_all_ops(GuestPoint { x: 1, y: 2 }).is_empty());
    }

    #[test]
    fn rate_limiter_brakes_bursts() {
        let mut r = ActionRateLimiter::new(3);
        assert!(r.check(0));
        assert!(r.check(100));
        assert!(r.check(200));
        assert!(!r.check(300));
        assert!(r.check(1001));
    }

    #[test]
    fn frame_cache_dedups_identical() {
        let mut c = FrameCache::new();
        let id = ComputerId::new();
        let bytes = TestInput::test_pattern_rgba(64, 36);
        let (a, fresh_a) = c.insert(id, 64, 36, FrameEncoding::RawRgba, bytes.clone(), 5);
        let (b, fresh_b) = c.insert(id, 64, 36, FrameEncoding::RawRgba, bytes, 6);
        assert!(fresh_a);
        assert!(!fresh_b);
        assert_eq!(a, b);
        let (d, fresh_d) = c.insert(id, 64, 36, FrameEncoding::RawRgba, vec![1, 2, 3, 4], 1);
        assert!(fresh_d);
        assert_ne!(a, d);
        assert!(c.get(&a).is_some());
    }

    #[test]
    fn pixel_diff_counts_changes() {
        let w = 8u32;
        let a = vec![0u8; (w * 4 * 4) as usize];
        let mut b = a.clone();
        // Change a 2x2 block at (1,1).
        for y in 1..3 {
            for x in 1..3 {
                let i = (y as usize * w as usize + x as usize) * 4;
                b[i] = 255;
            }
        }
        assert_eq!(pixel_diff_in_rect(&a, &b, w, (0, 0, 8, 4)), Some(4));
        assert_eq!(pixel_diff_in_rect(&a, &b, w, (0, 0, 1, 1)), Some(0));
        assert_eq!(pixel_diff_in_rect(&a, &b, w, (1, 1, 2, 2)), Some(4));
        assert_eq!(pixel_diff_in_rect(&a, &[1u8; 4], w, (0, 0, 1, 1)), None);
    }

    #[test]
    fn png_round_trips_test_pattern() {
        let bytes = TestInput::test_pattern_rgba(16, 9);
        let png = encode_png_rgba(16, 9, &bytes).expect("encode");
        assert!(png.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));
        assert!(encode_png_rgba(16, 9, &[0u8; 10]).is_err());
    }

    #[test]
    fn chunks_reassemble() {
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let chunks = chunk_bytes(&bytes);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.len() <= pegoles_guest_proto::MAX_FRAME_CHUNK_B64);
        }
        assert_eq!(reassemble_chunks(&chunks).unwrap(), bytes);
        assert!(reassemble_chunks(&["!!!".to_string()]).is_err());
    }

    #[test]
    fn test_input_records_and_tracks() {
        let mut b = TestInput::new(ComputerId::new());
        assert!(b.is_available());
        let down = InputOp::Down {
            point: GuestPoint { x: 1, y: 1 },
            button: PointerButton::Primary,
        };
        assert!(b.execute(&down).ok);
        assert!(!b.pressed_state().is_clean());
        b.release_all();
        assert!(b.pressed_state().is_clean());
        assert_eq!(b.recorded.len(), 2);
        b.fail_with = Some("boom".into());
        assert!(!b.execute(&down).ok);
    }

    #[test]
    fn unavailable_and_windows_stub_are_honest() {
        let mut u = UnavailableInput::new("headless");
        assert!(!u.is_available());
        assert!(!u.execute(&InputOp::GetDisplayInfo).ok);
        let mut w = WindowsInputStub;
        assert!(!w.is_available());
        let out = w.execute(&InputOp::GetDisplayInfo);
        assert!(!out.ok);
        assert!(out.error.unwrap().contains("HCS"));
    }

    #[test]
    fn no_host_input_or_capture_apis() {
        // Tripwire: guest-only module must never name host input/capture.
        // Tokens are split so this test's own literals don't self-match.
        let src = include_str!("input.rs");
        let tokens = [
            ["CG", "Event"].concat(),
            ["Send", "Input"].concat(),
            ["send", "input"].concat(),
            ["xdo", "tool"].concat(),
            ["en", "igo"].concat(),
            ["screen", "capture"].concat(),
            ["CGDisplayCreate", "Image"].concat(),
            ["Get", "DC"].concat(),
            ["Bit", "Blt"].concat(),
            ["XGet", "Image"].concat(),
            ["process", "::Command"].concat(),
            ["Clip", "board"].concat(),
            ["clip", "board"].concat(),
        ];
        for token in &tokens {
            assert!(!src.contains(token), "host capability leaked: {token}");
        }
    }
}

#[cfg(test)]
mod png_tests {
    use super::*;

    #[test]
    fn fast_rgb_png_round_trips_opaque_pixels() {
        let (w, h) = (64u32, 32u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % 251) as u8, (i % 7) as u8 * 30, 200, 255])
            .collect();
        let png_bytes = encode_png_rgb_fast(w, h, &rgba).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (w, h));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        for (i, px) in buf.chunks_exact(3).enumerate() {
            assert_eq!(px, &rgba[i * 4..i * 4 + 3]);
        }
        assert!(encode_png_rgb_fast(w, h, &rgba[..10]).is_err());
    }
}

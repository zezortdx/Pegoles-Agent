//! Coordinate spaces for Pegoles Eyes & Hands (Phase 5).
//!
//! Three spaces, one transform layer:
//!
//! ```text
//! Agent (normalized 0.0..=1.0, origin top-left)
//!   ↕ DisplayTransform::agent_to_guest
//! Guest (logical pixels, e.g. 1440x900, origin top-left)
//!   ↕ DisplayTransform::guest_to_native
//! Native (host view pixels: viewport rect + backing scale)
//! ```
//!
//! Model coordinates are NEVER host pixels and NEVER native view pixels:
//! every action enters in agent space and is converted with the CURRENT
//! guest size at execution time, so scripts survive resolution changes,
//! viewport scaling, Retina/HiDPI factors, window resizes, and future
//! mobile remote viewing.

use serde::{Deserialize, Serialize};

/// Normalized agent point. Must stay inside `0.0..=1.0` (checked by
/// Policy before execution; clamped defensively at conversion).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentPoint {
    pub x: f64,
    pub y: f64,
}

/// Guest display point in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestPoint {
    pub x: u32,
    pub y: u32,
}

/// Viewport rectangle of the native display view in host view pixels,
/// plus the backing scale factor (2.0 on Retina).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeViewport {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Backing pixels per view pixel (1.0 standard, 2.0 Retina, …).
    pub scale: f64,
}

/// Current geometry binding agent space to the guest framebuffer and the
/// native view showing it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayTransform {
    /// Guest framebuffer size in logical pixels.
    pub guest_width: u32,
    pub guest_height: u32,
    /// Where the guest image sits inside the native view (letterboxed).
    pub viewport: NativeViewport,
}

impl DisplayTransform {
    /// Identity-ish constructor for headless/logic-only use (no native view).
    pub fn headless(guest_width: u32, guest_height: u32) -> Self {
        Self {
            guest_width: guest_width.max(1),
            guest_height: guest_height.max(1),
            viewport: NativeViewport {
                x: 0.0,
                y: 0.0,
                width: guest_width.max(1) as f64,
                height: guest_height.max(1) as f64,
                scale: 1.0,
            },
        }
    }

    fn clamp01(v: f64) -> f64 {
        if v.is_nan() {
            0.0
        } else {
            v.clamp(0.0, 1.0)
        }
    }

    /// Agent -> guest pixels. Rounds to the nearest pixel and clamps to
    /// the framebuffer edges (last pixel index, never width).
    pub fn agent_to_guest(&self, p: AgentPoint) -> GuestPoint {
        let gx = (Self::clamp01(p.x) * self.guest_width as f64).round() as u32;
        let gy = (Self::clamp01(p.y) * self.guest_height as f64).round() as u32;
        GuestPoint {
            x: gx.min(self.guest_width.saturating_sub(1)),
            y: gy.min(self.guest_height.saturating_sub(1)),
        }
    }

    /// Guest pixels -> agent space (for cursor display + diagnostics).
    pub fn guest_to_agent(&self, p: GuestPoint) -> AgentPoint {
        AgentPoint {
            x: p.x as f64 / self.guest_width as f64,
            y: p.y as f64 / self.guest_height as f64,
        }
    }

    /// Guest pixels -> native backing pixels (where the native view draws
    /// that guest pixel: viewport offset + aspect-fit scale + backing scale).
    pub fn guest_to_native(&self, p: GuestPoint) -> (f64, f64) {
        let fit = (self.viewport.width / self.guest_width as f64)
            .min(self.viewport.height / self.guest_height as f64);
        let drawn_w = self.guest_width as f64 * fit;
        let drawn_h = self.guest_height as f64 * fit;
        let ox = self.viewport.x + (self.viewport.width - drawn_w) / 2.0;
        let oy = self.viewport.y + (self.viewport.height - drawn_h) / 2.0;
        (
            (ox + p.x as f64 * fit) * self.viewport.scale,
            (oy + p.y as f64 * fit) * self.viewport.scale,
        )
    }

    /// Native backing pixels -> guest pixels (for mapping native geometry
    /// reports back). Returns `None` outside the drawn image (letterbox).
    pub fn native_to_guest(&self, nx: f64, ny: f64) -> Option<GuestPoint> {
        let vx = nx / self.viewport.scale;
        let vy = ny / self.viewport.scale;
        let fit = (self.viewport.width / self.guest_width as f64)
            .min(self.viewport.height / self.guest_height as f64);
        if fit <= 0.0 {
            return None;
        }
        let drawn_w = self.guest_width as f64 * fit;
        let drawn_h = self.guest_height as f64 * fit;
        let ox = self.viewport.x + (self.viewport.width - drawn_w) / 2.0;
        let oy = self.viewport.y + (self.viewport.height - drawn_h) / 2.0;
        let gx = ((vx - ox) / fit).round() as i64;
        let gy = ((vy - oy) / fit).round() as i64;
        if gx < 0 || gy < 0 || gx >= self.guest_width as i64 || gy >= self.guest_height as i64 {
            return None;
        }
        Some(GuestPoint {
            x: gx as u32,
            y: gy as u32,
        })
    }

    /// Agent -> native backing pixels in one hop (cursor overlay path).
    pub fn agent_to_native(&self, p: AgentPoint) -> (f64, f64) {
        self.guest_to_native(self.agent_to_guest(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std() -> DisplayTransform {
        DisplayTransform::headless(1440, 900)
    }

    #[test]
    fn center_stays_center() {
        let g = std().agent_to_guest(AgentPoint { x: 0.5, y: 0.5 });
        assert_eq!(g, GuestPoint { x: 720, y: 450 });
    }

    #[test]
    fn corners_clamp_to_last_pixel() {
        let t = std();
        assert_eq!(
            t.agent_to_guest(AgentPoint { x: 0.0, y: 0.0 }),
            GuestPoint { x: 0, y: 0 }
        );
        assert_eq!(
            t.agent_to_guest(AgentPoint { x: 1.0, y: 1.0 }),
            GuestPoint { x: 1439, y: 899 }
        );
        // Out-of-range + NaN clamp defensively (policy rejects first).
        assert_eq!(
            t.agent_to_guest(AgentPoint { x: 1.5, y: -0.5 }),
            GuestPoint { x: 1439, y: 0 }
        );
        assert_eq!(
            t.agent_to_guest(AgentPoint {
                x: f64::NAN,
                y: f64::INFINITY
            }),
            GuestPoint { x: 0, y: 899 }
        );
    }

    #[test]
    fn guest_center_survives_viewport_sizes() {
        // A click at guest center must remain guest center regardless of
        // how the host shows the viewport (window resize, HiDPI…).
        let guest = GuestPoint { x: 720, y: 450 };
        for viewport in [
            NativeViewport {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
                scale: 1.0,
            },
            NativeViewport {
                x: 0.0,
                y: 0.0,
                width: 720.0,
                height: 450.0,
                scale: 1.0,
            },
            NativeViewport {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
                scale: 2.0,
            },
            NativeViewport {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
                scale: 2.0,
            },
        ] {
            let t = DisplayTransform {
                guest_width: 1440,
                guest_height: 900,
                viewport,
            };
            let (nx, ny) = t.guest_to_native(guest);
            let back = t.native_to_guest(nx, ny).expect("round-trips");
            assert_eq!(back, guest, "viewport: {viewport:?}");
        }
    }

    #[test]
    fn letterbox_is_outside_image() {
        // Tall narrow viewport: horizontal bars are not guest pixels.
        let t = DisplayTransform {
            guest_width: 1440,
            guest_height: 900,
            viewport: NativeViewport {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 900.0,
                scale: 1.0,
            },
        };
        assert!(t.native_to_guest(200.0, 10.0).is_none());
        // …but the drawn image interior maps back.
        let (nx, ny) = t.guest_to_native(GuestPoint { x: 720, y: 450 });
        assert_eq!(
            t.native_to_guest(nx, ny),
            Some(GuestPoint { x: 720, y: 450 })
        );
    }

    #[test]
    fn agent_native_round_trip() {
        let t = DisplayTransform {
            guest_width: 1440,
            guest_height: 900,
            viewport: NativeViewport {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
                scale: 2.0,
            },
        };
        let (nx, ny) = t.agent_to_native(AgentPoint { x: 0.25, y: 0.75 });
        // Retina: guest (360,675) at scale 2 → backing (720,1350).
        assert!((nx - 720.0).abs() < 1.0);
        assert!((ny - 1350.0).abs() < 1.0);
    }

    #[test]
    fn guest_agent_invertible() {
        let t = std();
        let g = GuestPoint { x: 100, y: 800 };
        let a = t.guest_to_agent(g);
        let back = t.agent_to_guest(a);
        assert!((back.x as i32 - g.x as i32).abs() <= 1);
        assert!((back.y as i32 - g.y as i32).abs() <= 1);
    }
}

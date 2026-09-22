//! Pegoles input test fixture (guest side, DEV ONLY).
//!
//! A fullscreen terminal target with stable PROPORTIONAL regions so
//! deterministic agent scripts can click/type/scroll/drag without vision:
//!
//! ```text
//! +-- PEGOLES INPUT TEST --------------------------------+
//! |  [ CLICK ME ]        clicks: 0                      |
//! |  [ TYPE HERE: _______ ]                             |
//! |  DRAG  A koje>>>  B                                 |
//! |  scroll: line0..lineN (wheel)                       |
//! +-----------------------------------------------------+
//! ```
//!
//! Every interaction appends JSONL to the fixture log (post-hoc
//! inspection) AND changes visible pixels (host verification via frame
//! diff — the real assertion path). Mouse via SGR 1006; pure layout +
//! render + parsers are unit-tested on the host.

use std::io::{Read, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x.saturating_add(self.w) && y >= self.y && y < self.y.saturating_add(self.h)
    }

    /// Normalized center (agent space) for deterministic scripts.
    pub fn center_norm(&self, term_w: u16, term_h: u16) -> (f64, f64) {
        (
            (self.x as f64 + self.w as f64 / 2.0) / term_w.max(1) as f64,
            (self.y as f64 + self.h as f64 / 2.0) / term_h.max(1) as f64,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Regions {
    pub click_btn: Rect,
    pub type_field: Rect,
    pub drag_src: Rect,
    pub drag_dst: Rect,
    pub scroll_list: Rect,
}

/// Proportional layout from terminal cells. Stable across sizes: the
/// host converts region centers to normalized agent coordinates.
pub fn layout(term_w: u16, term_h: u16) -> Regions {
    let w = term_w.max(40);
    let h = term_h.max(16);
    let fx = |p: f64| (w as f64 * p) as u16;
    let fy = |p: f64| (h as f64 * p) as u16;
    Regions {
        click_btn: Rect { x: fx(0.28), y: fy(0.30), w: fx(0.18).max(12), h: 3 },
        type_field: Rect { x: fx(0.28), y: fy(0.48), w: fx(0.44).max(20), h: 3 },
        drag_src: Rect { x: fx(0.08), y: fy(0.66), w: fx(0.16).max(10), h: 5 },
        drag_dst: Rect { x: fx(0.76), y: fy(0.66), w: fx(0.16).max(10), h: 5 },
        scroll_list: Rect { x: fx(0.52), y: fy(0.12), w: fx(0.20).max(14), h: fy(0.30).max(6) },
    }
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub clicks: u32,
    pub typed: String,
    pub type_focused: bool,
    pub submitted: Vec<String>,
    pub drag_armed: bool,
    pub drags_done: u32,
    pub scroll_offset: usize,
    pub events: u64,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub x: u16,
    pub y: u16,
    pub release: bool,
    pub drag: bool,
}

/// Parse one SGR 1006 mouse sequence (`\x1b[<b;x;yM/m`).
pub fn parse_sgr(seq: &str) -> Option<MouseEvent> {
    let body = seq.strip_prefix("\x1b[<")?;
    let final_m = body.ends_with('M') || body.ends_with('m');
    if !final_m {
        return None;
    }
    let release = body.ends_with('m');
    let nums: Vec<u16> = body[..body.len() - 1].split(';').filter_map(|n| n.parse().ok()).collect();
    if nums.len() != 3 {
        return None;
    }
    let (b, x, y) = (nums[0], nums[1].saturating_sub(1), nums[2].saturating_sub(1));
    let drag = b & 32 != 0;
    let code = b & !32;
    let button = match code {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        64 => MouseButton::WheelUp,
        65 => MouseButton::WheelDown,
        3 => return None, // release marker without button info
        _ => return None,
    };
    Some(MouseEvent { button, x, y, release, drag })
}

/// Apply a mouse event; returns a log line when something happened.
pub fn apply_mouse(state: &mut State, regions: &Regions, ev: &MouseEvent) -> Option<String> {
    state.events += 1;
    match ev.button {
        MouseButton::WheelUp => {
            state.scroll_offset = state.scroll_offset.saturating_sub(1);
            return Some(format!("scroll up offset={}", state.scroll_offset));
        }
        MouseButton::WheelDown => {
            state.scroll_offset = (state.scroll_offset + 1).min(40);
            return Some(format!("scroll down offset={}", state.scroll_offset));
        }
        _ => {}
    }
    if ev.release {
        if state.drag_armed && regions.drag_dst.contains(ev.x, ev.y) {
            state.drag_armed = false;
            state.drags_done += 1;
            return Some(format!("drag done total={}", state.drags_done));
        }
        state.drag_armed = false;
        return None;
    }
    if ev.drag {
        return None;
    }
    if regions.click_btn.contains(ev.x, ev.y) {
        state.clicks += 1;
        state.type_focused = false;
        return Some(format!("click total={}", state.clicks));
    }
    if regions.type_field.contains(ev.x, ev.y) {
        state.type_focused = true;
        return Some("focus type".to_string());
    }
    if regions.drag_src.contains(ev.x, ev.y) {
        state.drag_armed = true;
        return Some("drag armed".to_string());
    }
    state.type_focused = false;
    None
}

/// Apply a key (already decoded); returns a log line on submit.
pub fn apply_key(state: &mut State, key: &str) -> Option<String> {
    if !state.type_focused {
        return None;
    }
    if key == "\r" || key == "\n" {
        let line = std::mem::take(&mut state.typed);
        state.submitted.push(line.clone());
        state.events += 1;
        return Some(format!("submit {line:?}"));
    }
    if key == "\x7f" {
        state.typed.pop();
        state.events += 1;
        return None;
    }
    if key.chars().count() == 1 {
        let ch = key.chars().next().expect("counted");
        if !ch.is_control() && state.typed.len() < 256 {
            state.typed.push(ch);
            state.events += 1;
        }
    }
    None
}

const SCROLL_ITEMS: usize = 48;

fn pad_to(s: &str, w: usize) -> String {
    let mut out: String = s.chars().take(w).collect();
    while out.chars().count() < w {
        out.push(' ');
    }
    out
}

/// Render the fullscreen frame (pure; host-snapshot tested).
pub fn render(state: &State, regions: &Regions, term_w: u16, term_h: u16) -> String {
    let mut lines = vec![String::new(); term_h as usize];
    let title = " PEGOLES INPUT TEST (dev fixture) ";
    lines[0] = pad_to(title, term_w as usize);
    draw_box(&mut lines, regions.click_btn, &format!("[ CLICK ME: {} ]", state.clicks), state.clicks % 2 == 1);
    let field = format!("[ TYPE HERE: {} ]", state.typed);
    draw_box(&mut lines, regions.type_field, &field, state.type_focused);
    draw_box(&mut lines, regions.drag_src, "DRAG A", state.drag_armed);
    draw_box(&mut lines, regions.drag_dst, &format!("DROP B: {}", state.drags_done), false);
    draw_list(&mut lines, regions.scroll_list, state.scroll_offset);
    let status = format!(
        "events={} clicks={} drags={} submits={}",
        state.events,
        state.clicks,
        state.drags_done,
        state.submitted.len()
    );
    if let Some(last) = lines.last_mut() {
        *last = pad_to(&status, term_w as usize);
    }
    lines.join("\r\n")
}

fn draw_box(lines: &mut [String], r: Rect, label: &str, invert: bool) {
    let open = if invert { "\x1b[7m" } else { "" };
    let close = if invert { "\x1b[0m" } else { "" };
    for dy in 0..r.h {
        let y = r.y as usize + dy as usize;
        if y >= lines.len() {
            break;
        }
        let text = if dy == r.h / 2 { pad_to(label, r.w as usize) } else { " ".repeat(r.w as usize) };
        let row = &mut lines[y];
        while row.chars().count() < r.x as usize + r.w as usize {
            row.push(' ');
        }
        let mut chars: Vec<char> = row.chars().collect();
        for (i, c) in text.chars().enumerate() {
            if (r.x as usize + i) < chars.len() {
                chars[r.x as usize + i] = c;
            }
        }
        let mut rebuilt: String = chars.into_iter().collect();
        if invert {
            rebuilt = format!("{open}{rebuilt}{close}");
        }
        lines[y] = rebuilt;
    }
}

fn draw_list(lines: &mut [String], r: Rect, offset: usize) {
    for dy in 0..r.h {
        let y = r.y as usize + dy as usize;
        if y >= lines.len() {
            break;
        }
        let idx = offset + dy as usize;
        let text = if idx < SCROLL_ITEMS {
            pad_to(&format!("line{idx:02}"), r.w as usize)
        } else {
            " ".repeat(r.w as usize)
        };
        let row = &mut lines[y];
        while row.chars().count() < r.x as usize + r.w as usize {
            row.push(' ');
        }
        let mut chars: Vec<char> = row.chars().collect();
        for (i, c) in text.chars().enumerate() {
            if (r.x as usize + i) < chars.len() {
                chars[r.x as usize + i] = c;
            }
        }
        lines[y] = chars.into_iter().collect();
    }
}

fn log_line(path: &str, line: &str) {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "{{\"at\":{},\"msg\":{line:?}}}", now_secs());
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path);
    if let Ok(f) = f.as_mut() {
        let _ = f.write_all(s.as_bytes());
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn term_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            return (ws.ws_col, ws.ws_row.max(1));
        }
    }
    (80, 24)
}

struct RawGuard {
    orig: libc::termios,
}

impl RawGuard {
    fn enter() -> std::io::Result<Self> {
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut orig) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut raw = orig;
            raw.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
            raw.c_oflag &= !libc::OPOST;
            raw.c_cflag |= libc::CS8;
            raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
            raw.c_cc[libc::VMIN as usize] = 1;
            raw.c_cc[libc::VTIME as usize] = 0;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &raw) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self { orig })
        }
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &self.orig);
        }
    }
}

fn run() -> std::io::Result<()> {
    let log_path = std::env::var("PEGOLES_FIXTURE_LOG")
        .unwrap_or_else(|_| "/tmp/pegoles-fixture.log".to_string());
    let _raw = RawGuard::enter()?;
    let out = std::io::stdout();
    let mut out = out.lock();
    // Alternate screen + SGR mouse (1006) + any-event tracking.
    write!(out, "\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?1015h")?;
    out.flush()?;
    let mut state = State::new();
    let mut input: Vec<u8> = Vec::new();
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 256];
    loop {
        let (w, h) = term_size();
        let regions = layout(w, h);
        write!(out, "\x1b[H{}", render(&state, &regions, w, h))?;
        out.flush()?;
        let n = stdin.read(&mut buf)?;
        if n == 0 {
            break;
        }
        input.extend_from_slice(&buf[..n]);
        // Drain complete sequences: SGR mouse, single keys, quit on q/Ctrl-C.
        while !input.is_empty() {
            if input[0] == 0x1b {
                if input.len() >= 6 && input[1] == b'[' && input[2] == b'<' {
                    if let Some(end) = input.iter().position(|&b| b == b'M' || b == b'm') {
                        let seq = String::from_utf8_lossy(&input[..=end]).into_owned();
                        input.drain(..=end);
                        if let Some(ev) = parse_sgr(&seq) {
                            if let Some(line) = apply_mouse(&mut state, &regions, &ev) {
                                log_line(&log_path, &line);
                            }
                        }
                        continue;
                    }
                    break; // partial mouse sequence
                }
                input.drain(..1);
                continue;
            }
            let b = input.remove(0);
            if b == b'q' || b == 3 {
                write!(out, "\x1b[?1000l\x1b[?1006l\x1b[?1015l\x1b[?1049l")?;
                out.flush()?;
                return Ok(());
            }
            let key = (b as char).to_string();
            if let Some(line) = apply_key(&mut state, &key) {
                log_line(&log_path, &line);
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("fixture: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_are_proportional_and_inside() {
        for (w, h) in [(80, 24), (144, 54), (200, 60)] {
            let r = layout(w, h);
            for rect in [r.click_btn, r.type_field, r.drag_src, r.drag_dst, r.scroll_list] {
                assert!(rect.x + rect.w <= w, "{rect:?} in {w}x{h}");
                assert!(rect.y + rect.h <= h, "{rect:?} in {w}x{h}");
            }
            // Centers are stable normalized coordinates for scripts.
            let (cx, cy) = r.click_btn.center_norm(w, h);
            assert!((0.0..=1.0).contains(&cx) && (0.0..=1.0).contains(&cy));
        }
    }

    #[test]
    fn click_type_drag_scroll_flow() {
        let regions = layout(100, 30);
        let mut s = State::new();
        let (cx, cy) = (regions.click_btn.x, regions.click_btn.y);
        let line = apply_mouse(
            &mut s,
            &regions,
            &MouseEvent { button: MouseButton::Left, x: cx, y: cy, release: false, drag: false },
        );
        assert_eq!(s.clicks, 1);
        assert!(line.is_some());
        let (fx, fy) = (regions.type_field.x, regions.type_field.y);
        apply_mouse(
            &mut s,
            &regions,
            &MouseEvent { button: MouseButton::Left, x: fx, y: fy, release: false, drag: false },
        );
        assert!(s.type_focused);
        for ch in "hi".chars() {
            apply_key(&mut s, &ch.to_string());
        }
        assert_eq!(s.typed, "hi");
        let done = apply_key(&mut s, "\r");
        assert!(done.is_some());
        assert_eq!(s.submitted, vec!["hi".to_string()]);
        let (sx, sy) = (regions.drag_src.x, regions.drag_src.y);
        apply_mouse(
            &mut s,
            &regions,
            &MouseEvent { button: MouseButton::Left, x: sx, y: sy, release: false, drag: false },
        );
        assert!(s.drag_armed);
        let (dx, dy) = (regions.drag_dst.x, regions.drag_dst.y);
        apply_mouse(
            &mut s,
            &regions,
            &MouseEvent { button: MouseButton::Left, x: dx, y: dy, release: true, drag: false },
        );
        assert_eq!(s.drags_done, 1);
        apply_mouse(
            &mut s,
            &regions,
            &MouseEvent { button: MouseButton::WheelDown, x: 0, y: 0, release: false, drag: false },
        );
        assert_eq!(s.scroll_offset, 1);
    }

    #[test]
    fn sgr_sequences_parse() {
        let ev = parse_sgr("\x1b[<0;30;12M").unwrap();
        assert_eq!(ev.button, MouseButton::Left);
        assert_eq!((ev.x, ev.y), (29, 11));
        assert!(!ev.release);
        let ev = parse_sgr("\x1b[<0;30;12m").unwrap();
        assert!(ev.release);
        let ev = parse_sgr("\x1b[<65;10;10M").unwrap();
        assert_eq!(ev.button, MouseButton::WheelDown);
        assert!(parse_sgr("\x1b[<3;10;10M").is_none());
        assert!(parse_sgr("nope").is_none());
    }

    #[test]
    fn render_changes_pixels_on_interaction() {
        let regions = layout(100, 30);
        let before = render(&State::new(), &regions, 100, 30);
        let mut clicked = State::new();
        clicked.clicks = 1;
        let after = render(&clicked, &regions, 100, 30);
        assert_ne!(before, after);
        assert!(after.contains("CLICK ME: 1"));
    }
}

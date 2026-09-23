//! Guest-side input injection (Phase 5).
//!
//! The runtime owns ONE uinput device ("Pegoles Agent Input") with an
//! absolute touchscreen (ABS_X/ABS_Y, scaled by the host-provided display
//! size), relative wheel, pointer buttons, and a full keyboard. Absolute
//! positioning keeps agent clicks correct even when a human moved the
//! pointer through the native view in between.
//!
//! Layout: pure logic (key tables, compose plans, ABS scaling) is
//! platform-free and unit-tested on the host; the `device` submodule
//! holds the Linux-only uinput syscalls.
//!
//! Requirements (image side, see docs): /dev/uinput visible to the
//! runtime service, guest keymap `us(intl)` for dead-key accents.
//! Without them every op fails with a structured, honest error.

use pegoles_guest_proto::{GuestButton, GuestInputOp};

/// Canonical key name -> Linux evdev keycode (KEY_* from input-event-codes).
pub fn keycode(name: &str) -> Option<u16> {
    Some(match name {
        "Escape" => 1,
        "Backspace" => 14,
        "Tab" => 15,
        "Enter" => 28,
        "Control" => 29,
        "Shift" => 42,
        "Alt" => 56,
        "Space" => 57,
        "Meta" => 125,
        "Delete" => 111,
        "Insert" => 110,
        "Home" => 102,
        "End" => 107,
        "PageUp" => 104,
        "PageDown" => 109,
        "ArrowUp" => 103,
        "ArrowDown" => 108,
        "ArrowLeft" => 105,
        "ArrowRight" => 106,
        "F1" => 59,
        "F2" => 60,
        "F3" => 61,
        "F4" => 62,
        "F5" => 63,
        "F6" => 64,
        "F7" => 65,
        "F8" => 66,
        "F9" => 67,
        "F10" => 68,
        "F11" => 87,
        "F12" => 88,
        _ => return ascii_keycode(name),
    })
}

/// Single ASCII graphic / space -> evdev keycode + whether Shift is needed.
/// Letter codes are QWERTY positions (KEY_Q=16 …), NOT alphabetical.
pub fn ascii_keycode(name: &str) -> Option<u16> {
    if name.chars().count() != 1 {
        return None;
    }
    let c = name.chars().next().expect("counted");
    let lower = c.to_ascii_lowercase();
    if lower.is_ascii_lowercase() {
        return Some(match lower {
            'q' => 16,
            'w' => 17,
            'e' => 18,
            'r' => 19,
            't' => 20,
            'y' => 21,
            'u' => 22,
            'i' => 23,
            'o' => 24,
            'p' => 25,
            'a' => 30,
            's' => 31,
            'd' => 32,
            'f' => 33,
            'g' => 34,
            'h' => 35,
            'j' => 36,
            'k' => 37,
            'l' => 38,
            'z' => 44,
            'x' => 45,
            'c' => 46,
            'v' => 47,
            'b' => 48,
            'n' => 49,
            'm' => 50,
            _ => return None,
        });
    }
    Some(match c {
        '0' => 11,
        '1'..='9' => 2 + (c as u16 - '1' as u16),
        ' ' => 57,
        '-' => 12,
        '=' => 13,
        '[' => 26,
        ']' => 27,
        '\\' => 43,
        ';' => 39,
        '\'' => 40,
        ',' => 51,
        '.' => 52,
        '/' => 53,
        '`' => 41,
        '~' | '!' | '@' | '#' | '$' | '%' | '^' | '&' | '*' | '(' | ')' | '_' | '+' | '{' | '}'
        | '|' | ':' | '"' | '<' | '>' | '?' => shifted_symbol(c)?,
        _ => return None,
    })
}

fn shifted_symbol(c: char) -> Option<u16> {
    Some(match c {
        '~' => 41,
        '!' => 2,
        '@' => 3,
        '#' => 4,
        '$' => 5,
        '%' => 6,
        '^' => 7,
        '&' => 8,
        '*' => 9,
        '(' => 10,
        ')' => 11,
        '_' => 12,
        '+' => 13,
        '{' => 26,
        '}' => 27,
        '|' => 43,
        ':' => 39,
        '"' => 40,
        '<' => 51,
        '>' => 52,
        '?' => 53,
        _ => return None,
    })
}

/// Whether typing this char needs Shift held on a US base layout.
pub fn needs_shift(c: char) -> bool {
    c.is_ascii_uppercase()
        || matches!(
            c,
            '~' | '!'
                | '@'
                | '#'
                | '$'
                | '%'
                | '^'
                | '&'
                | '*'
                | '('
                | ')'
                | '_'
                | '+'
                | '{'
                | '}'
                | '|'
                | ':'
                | '"'
                | '<'
                | '>'
                | '?'
        )
}

/// Device name registered with the kernel (also the recognition key).
pub const DEVICE_NAME: &str = "Pegoles Agent Input";

/// True when `/proc/bus/input/devices` shows our device as recognized by
/// the input stack. Pure parser (host-tested); the file read is Linux.
pub fn device_recognized(proc_bus_input: &str, name: &str) -> bool {
    let mut current_name: Option<&str> = None;
    let mut matched = false;
    for line in proc_bus_input.lines().chain(std::iter::once("")) {
        let line = line.trim_end();
        if line.is_empty() {
            current_name = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("N: Name=") {
            let unquoted = rest
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(rest);
            current_name = Some(unquoted);
            if unquoted == name {
                matched = true;
            }
        }
    }
    let _ = current_name;
    matched
}

/// One physical key press: keycode + held modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyStroke {
    pub keycode: u16,
    pub shift: bool,
    pub right_alt: bool,
}

/// Dead keys on the `us(intl)` keymap (evdev codes of the dead key
/// itself; Shift/RightAlt select the dead variant).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeadKey {
    Acute,
    Grave,
    Circumflex,
    Tilde,
    Diaeresis,
}

impl DeadKey {
    fn stroke(self) -> KeyStroke {
        match self {
            // apostrophe key
            DeadKey::Acute => KeyStroke {
                keycode: 40,
                shift: false,
                right_alt: false,
            },
            // grave key
            DeadKey::Grave => KeyStroke {
                keycode: 41,
                shift: false,
                right_alt: false,
            },
            // Shift+6
            DeadKey::Circumflex => KeyStroke {
                keycode: 7,
                shift: true,
                right_alt: false,
            },
            // Shift+grave
            DeadKey::Tilde => KeyStroke {
                keycode: 41,
                shift: true,
                right_alt: false,
            },
            // Shift+apostrophe
            DeadKey::Diaeresis => KeyStroke {
                keycode: 40,
                shift: true,
                right_alt: false,
            },
        }
    }
}

/// Compose plan for one Unicode char: dead key(s) + base letter.
/// Covers Portuguese/Italian/Spanish/French accented letters (plus
/// German umlauts) on `us(intl)`. Anything else (ß, œ, æ, CJK, emoji,
/// controls) returns `None` -> structured Unsupported, never silent
/// substitution.
pub fn compose_plan(ch: char) -> Option<Vec<KeyStroke>> {
    if ch.is_ascii_graphic() || ch == ' ' {
        let keycode = ascii_keycode(&ch.to_string())?;
        return Some(vec![KeyStroke {
            keycode,
            shift: needs_shift(ch),
            right_alt: false,
        }]);
    }
    if ch == 'ç' {
        // us(intl): RightAlt+comma.
        return Some(vec![KeyStroke {
            keycode: 51,
            shift: false,
            right_alt: true,
        }]);
    }
    if ch == 'Ç' {
        return Some(vec![KeyStroke {
            keycode: 51,
            shift: true,
            right_alt: true,
        }]);
    }
    let (dead, base) = match ch {
        'á' | 'Á' => (DeadKey::Acute, 'a'),
        'é' | 'É' => (DeadKey::Acute, 'e'),
        'í' | 'Í' => (DeadKey::Acute, 'i'),
        'ó' | 'Ó' => (DeadKey::Acute, 'o'),
        'ú' | 'Ú' => (DeadKey::Acute, 'u'),
        'ý' | 'Ý' => (DeadKey::Acute, 'y'),
        'à' | 'À' => (DeadKey::Grave, 'a'),
        'è' | 'È' => (DeadKey::Grave, 'e'),
        'ì' | 'Ì' => (DeadKey::Grave, 'i'),
        'ò' | 'Ò' => (DeadKey::Grave, 'o'),
        'ù' | 'Ù' => (DeadKey::Grave, 'u'),
        'â' | 'Â' => (DeadKey::Circumflex, 'a'),
        'ê' | 'Ê' => (DeadKey::Circumflex, 'e'),
        'î' | 'Î' => (DeadKey::Circumflex, 'i'),
        'ô' | 'Ô' => (DeadKey::Circumflex, 'o'),
        'û' | 'Û' => (DeadKey::Circumflex, 'u'),
        'ã' | 'Ã' => (DeadKey::Tilde, 'a'),
        'õ' | 'Õ' => (DeadKey::Tilde, 'o'),
        'ñ' | 'Ñ' => (DeadKey::Tilde, 'n'),
        'ä' | 'Ä' => (DeadKey::Diaeresis, 'a'),
        'ë' | 'Ë' => (DeadKey::Diaeresis, 'e'),
        'ï' | 'Ï' => (DeadKey::Diaeresis, 'i'),
        'ö' | 'Ö' => (DeadKey::Diaeresis, 'o'),
        'ü' | 'Ü' => (DeadKey::Diaeresis, 'u'),
        'ÿ' => (DeadKey::Diaeresis, 'y'),
        'å' | 'Å' => {
            // us(intl): RightAlt+w.
            return Some(vec![
                KeyStroke {
                    keycode: 17,
                    shift: false,
                    right_alt: true,
                },
                KeyStroke {
                    keycode: ascii_keycode("a")?,
                    shift: ch.is_uppercase(),
                    right_alt: false,
                },
            ]);
        }
        _ => return None,
    };
    let base_code = ascii_keycode(&base.to_string())?;
    let mut plan = vec![dead.stroke()];
    plan.push(KeyStroke {
        keycode: base_code,
        shift: ch.is_uppercase(),
        right_alt: false,
    });
    Some(plan)
}

/// Scale guest pixels to touchscreen ABS units (0..=32767).
pub fn scale_to_abs(value: u32, extent: u32) -> i32 {
    if extent == 0 {
        return 0;
    }
    ((value as u64 * 32767) / extent as u64).min(32767) as i32
}

/// Validate an op against the runtime's static bounds (mirrors the host
/// policy caps; defense in depth — the guest never trusts the host).
pub fn validate_op(op: &GuestInputOp) -> Result<(), String> {
    if !op.is_bounded() {
        return Err("op exceeds guest bounds".to_string());
    }
    match op {
        GuestInputOp::Key { key, .. } => {
            if pegoles_key_known(key) {
                Ok(())
            } else {
                Err(format!("unknown key: {key}"))
            }
        }
        GuestInputOp::Chord { keys } => {
            if keys.len() > 4 || keys.is_empty() {
                return Err("chord must hold 1..=4 keys".to_string());
            }
            for k in keys {
                if !pegoles_key_known(k) {
                    return Err(format!("unknown key: {k}"));
                }
            }
            Ok(())
        }
        GuestInputOp::Type { text } => {
            for ch in text.chars() {
                if compose_plan(ch).is_none() {
                    return Err(format!("unsupported character: {ch:?}"));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Canonical key names the guest accepts (mirrors protocol vocabulary).
fn pegoles_key_known(name: &str) -> bool {
    if name.chars().count() == 1 {
        return name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_graphic() || c == ' ');
    }
    matches!(
        name,
        "Enter"
            | "Escape"
            | "Tab"
            | "Backspace"
            | "Delete"
            | "Insert"
            | "Space"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
            | "Shift"
            | "Control"
            | "Alt"
            | "Meta"
            | "F1"
            | "F2"
            | "F3"
            | "F4"
            | "F5"
            | "F6"
            | "F7"
            | "F8"
            | "F9"
            | "F10"
            | "F11"
            | "F12"
    )
}

pub fn button_code(button: GuestButton) -> u16 {
    match button {
        GuestButton::Primary => 272,   // BTN_LEFT
        GuestButton::Secondary => 273, // BTN_RIGHT
        GuestButton::Middle => 274,    // BTN_MIDDLE
    }
}

/// Linux-only uinput device. All syscalls live here; everything above is
/// pure and host-tested.
#[cfg(target_os = "linux")]
pub mod device {
    use super::*;
    use pegoles_guest_proto::GuestDisplaySize;

    const UI_SET_EVBIT: u64 = 0x40045564;
    const UI_SET_KEYBIT: u64 = 0x40045565;
    const UI_SET_RELBIT: u64 = 0x40045566;
    const UI_SET_ABSBIT: u64 = 0x40045567;
    const UI_DEV_CREATE: u64 = 0x5501;
    const UI_DEV_DESTROY: u64 = 0x5502;

    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const EV_REL: u16 = 0x02;
    const EV_ABS: u16 = 0x03;
    const SYN_REPORT: u16 = 0;
    const REL_WHEEL: u16 = 0x08;
    const REL_HWHEEL: u16 = 0x06;
    const ABS_X: u16 = 0x00;
    const ABS_Y: u16 = 0x01;

    // Keyboard keys only (KEY_ESC..=KEY_MICMUTE region). The BTN_* block
    // at 0x100.. must stay clear apart from the three mouse buttons:
    // BTN_TOOL_PEN / BTN_STYLUS make udev tag the device as a tablet and
    // libinput then rejects it ("missing tablet capabilities").
    const KEYBOARD_KEY_LAST: u16 = 0xff;

    #[repr(C)]
    struct UinputId {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
    }

    #[repr(C)]
    struct UinputUserDev {
        name: [u8; 80],
        id: UinputId,
        ff_effects_max: u32,
        absmax: [i32; 64],
        absmin: [i32; 64],
        absfuzz: [i32; 64],
        absflat: [i32; 64],
    }

    // The kernel's legacy setup path rejects any write whose size differs
    // from `struct uinput_user_dev` (80 + 8 + 4 + 4 * 64 * 4) with EINVAL.
    const _: () = assert!(std::mem::size_of::<UinputUserDev>() == 1116);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Timeval {
        tv_sec: i64,
        tv_usec: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct InputEvent {
        time: Timeval,
        type_: u16,
        code: u16,
        value: i32,
    }

    fn ioctl(fd: i32, req: u64, arg: i64) -> std::io::Result<()> {
        let ret = unsafe { libc::ioctl(fd, req, arg) };
        if ret < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Open /dev/uinput and create the Pegoles agent device.
    ///
    /// # Errors
    /// Returns a human/actionable message when the device is missing or
    /// the service lacks access (udev rule / input group — image side).
    pub fn open() -> Result<AgentDevice, String> {
        use std::os::unix::io::IntoRawFd;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| {
                format!(
                    "uinput unavailable (/dev/uinput: {e}; image needs udev rule + input group)"
                )
            })?;
        let fd = file.into_raw_fd();
        let setup = |what: &str, req: u64, code: i64| {
            ioctl(fd, req, code)
                .map_err(|e| format!("uinput setup failed at {what} (code {code}): {e}"))
        };
        let result = (|| {
            setup("evbit-syn", UI_SET_EVBIT, EV_SYN as i64)?;
            setup("evbit-key", UI_SET_EVBIT, EV_KEY as i64)?;
            setup("evbit-rel", UI_SET_EVBIT, EV_REL as i64)?;
            setup("evbit-abs", UI_SET_EVBIT, EV_ABS as i64)?;
            for code in [272u16, 273, 274] {
                setup("keybit-button", UI_SET_KEYBIT, code as i64)?;
            }
            for code in 1..=KEYBOARD_KEY_LAST {
                // Skip unknown/hazardous codes silently: errors here are
                // non-fatal (the bit just stays clear).
                let _ = ioctl(fd, UI_SET_KEYBIT, code as i64);
            }
            setup("relbit-wheel", UI_SET_RELBIT, REL_WHEEL as i64)?;
            setup("relbit-hwheel", UI_SET_RELBIT, REL_HWHEEL as i64)?;
            setup("absbit-x", UI_SET_ABSBIT, ABS_X as i64)?;
            setup("absbit-y", UI_SET_ABSBIT, ABS_Y as i64)?;
            let mut dev: UinputUserDev = unsafe { std::mem::zeroed() };
            let name = super::DEVICE_NAME.as_bytes();
            dev.name[..name.len()].copy_from_slice(name);
            dev.id = UinputId {
                bustype: 0x03,
                vendor: 0x5067,
                product: 0x0001,
                version: 1,
            };
            dev.absmin[ABS_X as usize] = 0;
            dev.absmax[ABS_X as usize] = 32767;
            dev.absmin[ABS_Y as usize] = 0;
            dev.absmax[ABS_Y as usize] = 32767;
            let ptr = &dev as *const UinputUserDev as *const u8;
            let bytes =
                unsafe { std::slice::from_raw_parts(ptr, std::mem::size_of::<UinputUserDev>()) };
            let written =
                unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
            if written != bytes.len() as isize {
                return Err(format!(
                    "uinput setup failed at write-dev (wrote {written}/{}, {})",
                    bytes.len(),
                    std::io::Error::last_os_error()
                ));
            }
            ioctl(fd, UI_DEV_CREATE, 0)
                .map_err(|e| format!("uinput setup failed at dev-create: {e}"))?;
            Ok(())
        })();
        if let Err(e) = result {
            unsafe { libc::close(fd) };
            return Err(format!("uinput setup failed: {e}"));
        }
        Ok(AgentDevice { fd })
    }

    pub struct AgentDevice {
        fd: i32,
    }

    fn pause_ms(ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    impl AgentDevice {
        fn emit(&self, type_: u16, code: u16, value: i32) {
            let ev = InputEvent {
                time: Timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                type_,
                code,
                value,
            };
            unsafe {
                libc::write(
                    self.fd,
                    &ev as *const InputEvent as *const libc::c_void,
                    std::mem::size_of::<InputEvent>(),
                );
            }
        }

        fn sync(&self) {
            self.emit(EV_SYN, SYN_REPORT, 0);
        }

        fn key(&self, code: u16, down: bool) {
            self.emit(EV_KEY, code, down as i32);
            self.sync();
        }

        fn abs_xy(&self, x: i32, y: i32) {
            self.emit(EV_ABS, ABS_X, x);
            self.emit(EV_ABS, ABS_Y, y);
            self.sync();
        }

        fn stroke(&self, s: KeyStroke) {
            if s.shift {
                self.key(42, true);
            }
            if s.right_alt {
                self.key(100, true);
            }
            self.key(s.keycode, true);
            self.key(s.keycode, false);
            if s.right_alt {
                self.key(100, false);
            }
            if s.shift {
                self.key(42, false);
            }
        }

        /// Press + paced release: an instant pair can be debounced away.
        fn click(&self, code: u16) {
            self.key(code, true);
            pause_ms(pegoles_guest_proto::CLICK_HOLD_MS);
            self.key(code, false);
        }

        fn abs_for(&self, x: u32, y: u32, display: GuestDisplaySize) {
            self.abs_xy(
                scale_to_abs(x, display.width_px),
                scale_to_abs(y, display.height_px),
            );
        }

        /// Execute one validated op. `display` scales absolute motion.
        pub fn execute(&self, op: &GuestInputOp, display: GuestDisplaySize) -> Result<(), String> {
            validate_op(op)?;
            match op {
                GuestInputOp::Move { x, y } => {
                    self.abs_for(*x, *y, display);
                }
                GuestInputOp::Press { x, y, button } => {
                    self.abs_for(*x, *y, display);
                    self.key(button_code(*button), true);
                }
                GuestInputOp::Release { x, y, button } => {
                    self.abs_for(*x, *y, display);
                    self.key(button_code(*button), false);
                }
                GuestInputOp::Click { x, y, button } => {
                    self.abs_for(*x, *y, display);
                    self.click(button_code(*button));
                }
                GuestInputOp::DoubleClick { x, y, button } => {
                    self.abs_for(*x, *y, display);
                    let code = button_code(*button);
                    self.click(code);
                    pause_ms(pegoles_guest_proto::DOUBLE_CLICK_GAP_MS);
                    self.click(code);
                }
                GuestInputOp::Drag {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    button,
                    duration_ms,
                } => {
                    let code = button_code(*button);
                    self.abs_for(*from_x, *from_y, display);
                    self.key(code, true);
                    let steps = ((*duration_ms as u64 / 16).clamp(2, 64)) as i64;
                    let (x0, y0) = (
                        scale_to_abs(*from_x, display.width_px),
                        scale_to_abs(*from_y, display.height_px),
                    );
                    let (x1, y1) = (
                        scale_to_abs(*to_x, display.width_px),
                        scale_to_abs(*to_y, display.height_px),
                    );
                    let step_ms = *duration_ms as u64 / steps as u64;
                    for i in 1..=steps {
                        let x = x0 + (x1 - x0) * i as i32 / steps as i32;
                        let y = y0 + (y1 - y0) * i as i32 / steps as i32;
                        self.abs_xy(x, y);
                        std::thread::sleep(std::time::Duration::from_millis(step_ms));
                    }
                    self.key(code, false);
                }
                GuestInputOp::Scroll { x, y, dx, dy } => {
                    self.abs_for(*x, *y, display);
                    // Linux REL_WHEEL +1 = wheel up = content up; our
                    // +dy means content DOWN, hence the negation.
                    for _ in 0..dx.unsigned_abs() {
                        self.emit(EV_REL, REL_HWHEEL, dx.signum());
                        self.sync();
                    }
                    for _ in 0..dy.unsigned_abs() {
                        self.emit(EV_REL, REL_WHEEL, -dy.signum());
                        self.sync();
                    }
                }
                GuestInputOp::Key { key, down } => {
                    let code = keycode(key).ok_or_else(|| format!("unknown key: {key}"))?;
                    self.key(code, *down);
                }
                GuestInputOp::Chord { keys } => {
                    // Canonical order: modifiers down first (in listed
                    // order), main key tap, modifiers released reversed.
                    let mut mods = Vec::new();
                    let mut main = None;
                    for k in keys {
                        let code = keycode(k).ok_or_else(|| format!("unknown key: {k}"))?;
                        if matches!(k.as_str(), "Shift" | "Control" | "Alt" | "Meta") {
                            mods.push(code);
                        } else if main.is_none() {
                            main = Some(code);
                        } else {
                            return Err("chord holds one main key".to_string());
                        }
                    }
                    if main.is_none() && mods.is_empty() {
                        return Err("chord needs a key".to_string());
                    }
                    for m in &mods {
                        self.key(*m, true);
                    }
                    // A lone modifier (KeyPress "Shift") is a modifier tap.
                    if let Some(main) = main {
                        self.key(main, true);
                        self.key(main, false);
                    }
                    for m in mods.iter().rev() {
                        self.key(*m, false);
                    }
                }
                GuestInputOp::Type { text } => {
                    for ch in text.chars() {
                        let plan = compose_plan(ch)
                            .ok_or_else(|| format!("unsupported character: {ch:?}"))?;
                        for s in plan {
                            self.stroke(s);
                            pause_ms(pegoles_guest_proto::KEY_STROKE_PACING_MS);
                        }
                    }
                }
            }
            Ok(())
        }

        /// Release every button + modifier (cancellation / disconnect).
        pub fn release_all(&self) {
            for code in [272u16, 273, 274, 42, 29, 56, 125, 100] {
                self.key(code, false);
            }
        }
    }

    impl Drop for AgentDevice {
        fn drop(&mut self) {
            self.release_all();
            unsafe {
                libc::ioctl(self.fd, UI_DEV_DESTROY as u64, 0);
                libc::close(self.fd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evdev_letters_and_digits() {
        assert_eq!(keycode("a"), Some(30));
        assert_eq!(keycode("q"), Some(16));
        assert_eq!(keycode("e"), Some(18));
        assert_eq!(keycode("z"), Some(44));
        assert_eq!(keycode("A"), Some(30));
        assert_eq!(keycode("1"), Some(2));
        assert_eq!(keycode("0"), Some(11));
        assert_eq!(keycode("Enter"), Some(28));
        assert_eq!(keycode("Escape"), Some(1));
        assert_eq!(keycode("Meta"), Some(125));
        assert_eq!(keycode("F11"), Some(87));
        assert_eq!(keycode("Hyper"), None);
        assert!(needs_shift('A'));
        assert!(needs_shift('?'));
        assert!(!needs_shift('a'));
    }

    #[test]
    fn portuguese_italian_accents_compose() {
        // "Pegolés à italiana ção" building blocks.
        let plan = compose_plan('é').unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].keycode, 40); // dead acute on apostrophe
        assert_eq!(plan[1].keycode, 18); // e (QWERTY position)
        assert!(compose_plan('à').is_some());
        assert!(compose_plan('ã').is_some());
        assert!(compose_plan('ç').is_some());
        assert!(compose_plan('ñ').is_some());
        assert!(compose_plan('ü').is_some());
        assert!(compose_plan('Õ').unwrap()[1].shift);
        // Not silently substitutable: honest Unsupported.
        assert!(compose_plan('ß').is_none());
        assert!(compose_plan('œ').is_none());
        assert!(compose_plan('中').is_none());
        assert!(compose_plan('\n').is_none());
    }

    #[test]
    fn every_typable_char_fits_the_host_budget() {
        // The host budgets TypeText as chars * MAX_STROKES_PER_CHAR paced
        // strokes; a longer plan would time out on long text.
        let accents = "áàâãäéèêëíìîïóòôõöúùûüçñÁÀÂÃÄÉÈÊËÍÌÎÏÓÒÔÕÖÚÙÛÜÇÑ";
        for ch in (' '..='~').chain(accents.chars()) {
            if let Some(plan) = compose_plan(ch) {
                assert!(
                    plan.len() as u64 <= pegoles_guest_proto::MAX_STROKES_PER_CHAR,
                    "{ch:?} needs {} strokes",
                    plan.len()
                );
            }
        }
    }

    #[test]
    fn abs_scaling_covers_screen() {
        assert_eq!(scale_to_abs(0, 1440), 0);
        assert_eq!(scale_to_abs(1440, 1440), 32767);
        assert_eq!(scale_to_abs(720, 1440), 16383);
        assert_eq!(scale_to_abs(5, 0), 0);
    }

    #[test]
    fn op_validation_is_defense_in_depth() {
        assert!(validate_op(&GuestInputOp::Move { x: 1, y: 2 }).is_ok());
        assert!(validate_op(&GuestInputOp::Type {
            text: "echo hi".into()
        })
        .is_ok());
        assert!(validate_op(&GuestInputOp::Type { text: "ß".into() }).is_err());
        assert!(validate_op(&GuestInputOp::Key {
            key: "Hyper".into(),
            down: true
        })
        .is_err());
        assert!(validate_op(&GuestInputOp::Chord {
            keys: vec![
                "Control".into(),
                "Control".into(),
                "c".into(),
                "x".into(),
                "y".into()
            ],
        })
        .is_err());
    }

    #[test]
    fn button_codes_are_linux_btn() {
        assert_eq!(button_code(GuestButton::Primary), 272);
        assert_eq!(button_code(GuestButton::Secondary), 273);
        assert_eq!(button_code(GuestButton::Middle), 274);
    }

    #[test]
    fn recognition_parses_proc_bus_input() {
        let sample = "I: Bus=0006 Vendor=0000 Product=0000 Version=0000\n\
                      N: Name=\"Pegoles Agent Input\"\n\
                      P: Phys=\n\
                      \n\
                      I: Bus=0011 Vendor=0002 Product=0008 Version=0000\n\
                      N: Name=\"AT Translated Set 2 keyboard\"\n";
        assert!(device_recognized(sample, DEVICE_NAME));
        assert!(!device_recognized(sample, "Nope"));
        assert!(!device_recognized("", DEVICE_NAME));
        assert!(!device_recognized(
            "N: Name=\"Pegoles Agent Inpu\"\n",
            DEVICE_NAME
        ));
    }
}

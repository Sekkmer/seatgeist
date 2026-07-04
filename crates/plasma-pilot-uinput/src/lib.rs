use std::{
    fs::{File, OpenOptions},
    io::Write,
    mem,
    os::fd::AsRawFd,
    path::Path,
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};

const UINPUT_PATH: &str = "/dev/uinput";
const UINPUT_IOCTL_BASE: u8 = b'U';
const UINPUT_MAX_NAME_SIZE: usize = 80;
const DEVICE_SETTLE_MS: u64 = 100;

const IOC_NRBITS: u8 = 8;
const IOC_TYPEBITS: u8 = 8;
const IOC_SIZEBITS: u8 = 14;

const IOC_NRSHIFT: u8 = 0;
const IOC_TYPESHIFT: u8 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u8 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u8 = IOC_SIZESHIFT + IOC_SIZEBITS;

const IOC_NONE: u8 = 0;
const IOC_WRITE: u8 = 1;

const fn ioc(dir: u8, ty: u8, nr: u8, size: usize) -> libc::c_ulong {
    ((dir as libc::c_ulong) << IOC_DIRSHIFT)
        | ((ty as libc::c_ulong) << IOC_TYPESHIFT)
        | ((nr as libc::c_ulong) << IOC_NRSHIFT)
        | ((size as libc::c_ulong) << IOC_SIZESHIFT)
}

const fn io(ty: u8, nr: u8) -> libc::c_ulong {
    ioc(IOC_NONE, ty, nr, 0)
}

const fn iow<T>(ty: u8, nr: u8) -> libc::c_ulong {
    ioc(IOC_WRITE, ty, nr, mem::size_of::<T>())
}

const UI_DEV_CREATE: libc::c_ulong = io(UINPUT_IOCTL_BASE, 1);
const UI_DEV_DESTROY: libc::c_ulong = io(UINPUT_IOCTL_BASE, 2);
const UI_DEV_SETUP: libc::c_ulong = iow::<UinputSetup>(UINPUT_IOCTL_BASE, 3);
const UI_ABS_SETUP: libc::c_ulong = iow::<UinputAbsSetup>(UINPUT_IOCTL_BASE, 4);
const UI_SET_EVBIT: libc::c_ulong = iow::<libc::c_int>(UINPUT_IOCTL_BASE, 100);
const UI_SET_KEYBIT: libc::c_ulong = iow::<libc::c_int>(UINPUT_IOCTL_BASE, 101);
const UI_SET_RELBIT: libc::c_ulong = iow::<libc::c_int>(UINPUT_IOCTL_BASE, 102);
const UI_SET_ABSBIT: libc::c_ulong = iow::<libc::c_int>(UINPUT_IOCTL_BASE, 103);

const BUS_USB: u16 = 0x03;
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MAX_VALUE: i32 = 32_767;

const KEY_ESC: u16 = 1;
const KEY_1: u16 = 2;
const KEY_2: u16 = 3;
const KEY_3: u16 = 4;
const KEY_4: u16 = 5;
const KEY_5: u16 = 6;
const KEY_6: u16 = 7;
const KEY_7: u16 = 8;
const KEY_8: u16 = 9;
const KEY_9: u16 = 10;
const KEY_0: u16 = 11;
const KEY_MINUS: u16 = 12;
const KEY_EQUAL: u16 = 13;
const KEY_BACKSPACE: u16 = 14;
const KEY_TAB: u16 = 15;
const KEY_Q: u16 = 16;
const KEY_W: u16 = 17;
const KEY_E: u16 = 18;
const KEY_R: u16 = 19;
const KEY_T: u16 = 20;
const KEY_Y: u16 = 21;
const KEY_U: u16 = 22;
const KEY_I: u16 = 23;
const KEY_O: u16 = 24;
const KEY_P: u16 = 25;
const KEY_LEFTBRACE: u16 = 26;
const KEY_RIGHTBRACE: u16 = 27;
const KEY_ENTER: u16 = 28;
const KEY_LEFTCTRL: u16 = 29;
const KEY_A: u16 = 30;
const KEY_S: u16 = 31;
const KEY_D: u16 = 32;
const KEY_F: u16 = 33;
const KEY_G: u16 = 34;
const KEY_H: u16 = 35;
const KEY_J: u16 = 36;
const KEY_K: u16 = 37;
const KEY_L: u16 = 38;
const KEY_SEMICOLON: u16 = 39;
const KEY_APOSTROPHE: u16 = 40;
const KEY_GRAVE: u16 = 41;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_BACKSLASH: u16 = 43;
const KEY_Z: u16 = 44;
const KEY_X: u16 = 45;
const KEY_C: u16 = 46;
const KEY_V: u16 = 47;
const KEY_B: u16 = 48;
const KEY_N: u16 = 49;
const KEY_M: u16 = 50;
const KEY_COMMA: u16 = 51;
const KEY_DOT: u16 = 52;
const KEY_SLASH: u16 = 53;
const KEY_LEFTALT: u16 = 56;
const KEY_SPACE: u16 = 57;
const KEY_F1: u16 = 59;
const KEY_F2: u16 = 60;
const KEY_F3: u16 = 61;
const KEY_F4: u16 = 62;
const KEY_F5: u16 = 63;
const KEY_F6: u16 = 64;
const KEY_F7: u16 = 65;
const KEY_F8: u16 = 66;
const KEY_F9: u16 = 67;
const KEY_F10: u16 = 68;
const KEY_F11: u16 = 87;
const KEY_F12: u16 = 88;
const KEY_HOME: u16 = 102;
const KEY_UP: u16 = 103;
const KEY_PAGEUP: u16 = 104;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_END: u16 = 107;
const KEY_DOWN: u16 = 108;
const KEY_PAGEDOWN: u16 = 109;
const KEY_INSERT: u16 = 110;
const KEY_DELETE: u16 = 111;
const KEY_LEFTMETA: u16 = 125;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

const SUPPORTED_KEY_CODES: &[u16] = &[
    KEY_ESC,
    KEY_1,
    KEY_2,
    KEY_3,
    KEY_4,
    KEY_5,
    KEY_6,
    KEY_7,
    KEY_8,
    KEY_9,
    KEY_0,
    KEY_MINUS,
    KEY_EQUAL,
    KEY_BACKSPACE,
    KEY_TAB,
    KEY_Q,
    KEY_W,
    KEY_E,
    KEY_R,
    KEY_T,
    KEY_Y,
    KEY_U,
    KEY_I,
    KEY_O,
    KEY_P,
    KEY_LEFTBRACE,
    KEY_RIGHTBRACE,
    KEY_ENTER,
    KEY_LEFTCTRL,
    KEY_A,
    KEY_S,
    KEY_D,
    KEY_F,
    KEY_G,
    KEY_H,
    KEY_J,
    KEY_K,
    KEY_L,
    KEY_SEMICOLON,
    KEY_APOSTROPHE,
    KEY_GRAVE,
    KEY_LEFTSHIFT,
    KEY_BACKSLASH,
    KEY_Z,
    KEY_X,
    KEY_C,
    KEY_V,
    KEY_B,
    KEY_N,
    KEY_M,
    KEY_COMMA,
    KEY_DOT,
    KEY_SLASH,
    KEY_LEFTALT,
    KEY_SPACE,
    KEY_F1,
    KEY_F2,
    KEY_F3,
    KEY_F4,
    KEY_F5,
    KEY_F6,
    KEY_F7,
    KEY_F8,
    KEY_F9,
    KEY_F10,
    KEY_F11,
    KEY_F12,
    KEY_HOME,
    KEY_UP,
    KEY_PAGEUP,
    KEY_LEFT,
    KEY_RIGHT,
    KEY_END,
    KEY_DOWN,
    KEY_PAGEDOWN,
    KEY_INSERT,
    KEY_DELETE,
    KEY_LEFTMETA,
];

#[repr(C)]
#[derive(Clone, Copy)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UinputSetup {
    id: InputId,
    name: [libc::c_char; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputAbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UinputAbsSetup {
    code: u16,
    absinfo: InputAbsInfo,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyStroke {
    code: u16,
    shift: bool,
}

struct UinputKeyboard {
    file: File,
    created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

struct UinputPointer {
    file: File,
    bounds: PointerBounds,
    created: bool,
}

impl UinputKeyboard {
    fn create() -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(UINPUT_PATH)
            .with_context(|| format!("open {UINPUT_PATH} for uinput keyboard"))?;
        let keyboard = Self {
            file,
            created: false,
        };
        keyboard.setup_bits()?;
        keyboard.setup_device()?;
        let mut keyboard = keyboard;
        ioctl_noarg(keyboard.file.as_raw_fd(), UI_DEV_CREATE).context("create uinput device")?;
        keyboard.created = true;
        thread::sleep(Duration::from_millis(DEVICE_SETTLE_MS));
        Ok(keyboard)
    }

    fn setup_bits(&self) -> Result<()> {
        let fd = self.file.as_raw_fd();
        ioctl_int(fd, UI_SET_EVBIT, EV_KEY).context("enable EV_KEY")?;
        ioctl_int(fd, UI_SET_EVBIT, EV_SYN).context("enable EV_SYN")?;
        for code in SUPPORTED_KEY_CODES {
            ioctl_int(fd, UI_SET_KEYBIT, *code).with_context(|| format!("enable key {code}"))?;
        }
        Ok(())
    }

    fn setup_device(&self) -> Result<()> {
        let mut name = [0 as libc::c_char; UINPUT_MAX_NAME_SIZE];
        for (index, byte) in b"PlasmaPilot Virtual Keyboard".iter().enumerate() {
            name[index] = *byte as libc::c_char;
        }
        let setup = UinputSetup {
            id: InputId {
                bustype: BUS_USB,
                vendor: 0x5050,
                product: 0x0001,
                version: 1,
            },
            name,
            ff_effects_max: 0,
        };
        ioctl_ptr(self.file.as_raw_fd(), UI_DEV_SETUP, &setup).context("setup uinput device")
    }

    fn press_key(&mut self, code: u16) -> Result<()> {
        self.emit(EV_KEY, code, 1)?;
        self.sync()
    }

    fn release_key(&mut self, code: u16) -> Result<()> {
        self.emit(EV_KEY, code, 0)?;
        self.sync()
    }

    fn tap_key(&mut self, code: u16) -> Result<()> {
        self.press_key(code)?;
        self.release_key(code)
    }

    fn type_stroke(&mut self, stroke: KeyStroke) -> Result<()> {
        if stroke.shift {
            self.press_key(KEY_LEFTSHIFT)?;
        }
        self.tap_key(stroke.code)?;
        if stroke.shift {
            self.release_key(KEY_LEFTSHIFT)?;
        }
        Ok(())
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<()> {
        let event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        let bytes = event_as_bytes(&event);
        self.file.write_all(bytes).context("write input event")
    }

    fn sync(&mut self) -> Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }
}

impl UinputPointer {
    fn create(bounds: PointerBounds) -> Result<Self> {
        validate_pointer_bounds(bounds)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(UINPUT_PATH)
            .with_context(|| format!("open {UINPUT_PATH} for uinput pointer"))?;
        let pointer = Self {
            file,
            bounds,
            created: false,
        };
        pointer.setup_bits()?;
        pointer.setup_device()?;
        let mut pointer = pointer;
        ioctl_noarg(pointer.file.as_raw_fd(), UI_DEV_CREATE).context("create uinput pointer")?;
        pointer.created = true;
        thread::sleep(Duration::from_millis(DEVICE_SETTLE_MS));
        Ok(pointer)
    }

    fn setup_bits(&self) -> Result<()> {
        let fd = self.file.as_raw_fd();
        ioctl_int(fd, UI_SET_EVBIT, EV_KEY).context("enable pointer EV_KEY")?;
        ioctl_int(fd, UI_SET_EVBIT, EV_SYN).context("enable pointer EV_SYN")?;
        ioctl_int(fd, UI_SET_EVBIT, EV_REL).context("enable pointer EV_REL")?;
        ioctl_int(fd, UI_SET_EVBIT, EV_ABS).context("enable pointer EV_ABS")?;
        ioctl_int(fd, UI_SET_KEYBIT, BTN_LEFT).context("enable BTN_LEFT")?;
        ioctl_int(fd, UI_SET_KEYBIT, BTN_RIGHT).context("enable BTN_RIGHT")?;
        ioctl_int(fd, UI_SET_KEYBIT, BTN_MIDDLE).context("enable BTN_MIDDLE")?;
        ioctl_int(fd, UI_SET_RELBIT, REL_WHEEL).context("enable REL_WHEEL")?;
        ioctl_int(fd, UI_SET_RELBIT, REL_HWHEEL).context("enable REL_HWHEEL")?;
        ioctl_int(fd, UI_SET_ABSBIT, ABS_X).context("enable ABS_X")?;
        ioctl_int(fd, UI_SET_ABSBIT, ABS_Y).context("enable ABS_Y")?;
        self.setup_abs_axis(ABS_X)?;
        self.setup_abs_axis(ABS_Y)?;
        Ok(())
    }

    fn setup_abs_axis(&self, code: u16) -> Result<()> {
        let setup = UinputAbsSetup {
            code,
            absinfo: InputAbsInfo {
                value: 0,
                minimum: 0,
                maximum: ABS_MAX_VALUE,
                fuzz: 0,
                flat: 0,
                resolution: 0,
            },
        };
        ioctl_ptr(self.file.as_raw_fd(), UI_ABS_SETUP, &setup)
            .with_context(|| format!("setup absolute axis {code}"))
    }

    fn setup_device(&self) -> Result<()> {
        let mut name = [0 as libc::c_char; UINPUT_MAX_NAME_SIZE];
        for (index, byte) in b"PlasmaPilot Virtual Pointer".iter().enumerate() {
            name[index] = *byte as libc::c_char;
        }
        let setup = UinputSetup {
            id: InputId {
                bustype: BUS_USB,
                vendor: 0x5050,
                product: 0x0002,
                version: 1,
            },
            name,
            ff_effects_max: 0,
        };
        ioctl_ptr(self.file.as_raw_fd(), UI_DEV_SETUP, &setup).context("setup uinput pointer")
    }

    fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        let (abs_x, abs_y) = map_pointer_point(x, y, self.bounds)?;
        self.emit(EV_ABS, ABS_X, abs_x)?;
        self.emit(EV_ABS, ABS_Y, abs_y)?;
        self.sync()
    }

    fn click(&mut self, x: f64, y: f64, button: PointerButton, clicks: u8) -> Result<()> {
        self.move_to(x, y)?;
        let code = button_code(button);
        for _ in 0..clicks {
            self.emit(EV_KEY, code, 1)?;
            self.sync()?;
            self.emit(EV_KEY, code, 0)?;
            self.sync()?;
        }
        Ok(())
    }

    fn scroll(&mut self, vertical: i32, horizontal: i32) -> Result<()> {
        if vertical == 0 && horizontal == 0 {
            bail!("scroll request must include a non-zero delta");
        }
        if vertical != 0 {
            self.emit(EV_REL, REL_WHEEL, vertical)?;
        }
        if horizontal != 0 {
            self.emit(EV_REL, REL_HWHEEL, horizontal)?;
        }
        self.sync()
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<()> {
        let event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        let bytes = event_as_bytes(&event);
        self.file.write_all(bytes).context("write pointer event")
    }

    fn sync(&mut self) -> Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }
}

impl Drop for UinputPointer {
    fn drop(&mut self) {
        if self.created {
            let _ = ioctl_noarg(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

impl Drop for UinputKeyboard {
    fn drop(&mut self) {
        if self.created {
            let _ = ioctl_noarg(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

pub fn available() -> bool {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(UINPUT_PATH)
        .is_ok()
}

pub fn type_text(text: &str) -> Result<()> {
    if text.is_empty() {
        bail!("text must be non-empty");
    }
    let strokes = text
        .chars()
        .map(char_to_stroke)
        .collect::<Result<Vec<_>>>()?;
    let mut keyboard = UinputKeyboard::create()?;
    for stroke in strokes {
        keyboard.type_stroke(stroke)?;
    }
    Ok(())
}

pub fn key_combo(combo: &str) -> Result<usize> {
    let codes = parse_key_combo(combo)?;
    let mut keyboard = UinputKeyboard::create()?;
    for code in &codes {
        keyboard.press_key(*code)?;
    }
    for code in codes.iter().rev() {
        keyboard.release_key(*code)?;
    }
    Ok(codes.len())
}

pub fn move_pointer(x: f64, y: f64, bounds: PointerBounds) -> Result<()> {
    let mut pointer = UinputPointer::create(bounds)?;
    pointer.move_to(x, y)
}

pub fn click_pointer(
    x: f64,
    y: f64,
    bounds: PointerBounds,
    button: PointerButton,
    clicks: u8,
) -> Result<()> {
    if clicks == 0 || clicks > 2 {
        bail!("click count must be 1 or 2");
    }
    let mut pointer = UinputPointer::create(bounds)?;
    pointer.click(x, y, button, clicks)
}

pub fn scroll_pointer(vertical: i32, horizontal: i32, bounds: PointerBounds) -> Result<()> {
    let mut pointer = UinputPointer::create(bounds)?;
    pointer.scroll(vertical, horizontal)
}

fn validate_pointer_bounds(bounds: PointerBounds) -> Result<()> {
    if bounds.width < 2 || bounds.height < 2 {
        bail!("pointer bounds must be at least 2x2 pixels");
    }
    Ok(())
}

fn map_pointer_point(x: f64, y: f64, bounds: PointerBounds) -> Result<(i32, i32)> {
    validate_pointer_bounds(bounds)?;
    if !x.is_finite() || !y.is_finite() {
        bail!("pointer coordinates must be finite");
    }
    let max_x = f64::from(bounds.min_x) + f64::from(bounds.width - 1);
    let max_y = f64::from(bounds.min_y) + f64::from(bounds.height - 1);
    if x < f64::from(bounds.min_x) || x > max_x || y < f64::from(bounds.min_y) || y > max_y {
        bail!(
            "pointer coordinate {},{} is outside physical desktop bounds {},{} {}x{}",
            x,
            y,
            bounds.min_x,
            bounds.min_y,
            bounds.width,
            bounds.height
        );
    }
    let normalized_x = (x - f64::from(bounds.min_x)) / f64::from(bounds.width - 1);
    let normalized_y = (y - f64::from(bounds.min_y)) / f64::from(bounds.height - 1);
    Ok((
        (normalized_x * f64::from(ABS_MAX_VALUE)).round() as i32,
        (normalized_y * f64::from(ABS_MAX_VALUE)).round() as i32,
    ))
}

fn button_code(button: PointerButton) -> u16 {
    match button {
        PointerButton::Left => BTN_LEFT,
        PointerButton::Middle => BTN_MIDDLE,
        PointerButton::Right => BTN_RIGHT,
    }
}

fn ioctl_noarg(fd: libc::c_int, request: libc::c_ulong) -> Result<()> {
    // SAFETY: ioctl is called with a valid uinput file descriptor and a no-argument request
    // defined by linux/uinput.h. The kernel owns all validation of the request.
    let result = unsafe { libc::ioctl(fd, request) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("uinput ioctl");
    }
    Ok(())
}

fn ioctl_int(fd: libc::c_int, request: libc::c_ulong, value: u16) -> Result<()> {
    // SAFETY: ioctl is called with a valid uinput file descriptor, a request taking an int
    // bit value, and a plain integer copied by value.
    let result = unsafe { libc::ioctl(fd, request, libc::c_int::from(value)) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("uinput ioctl");
    }
    Ok(())
}

fn ioctl_ptr<T>(fd: libc::c_int, request: libc::c_ulong, value: &T) -> Result<()> {
    // SAFETY: value points to a repr(C) structure that lives for the duration of the ioctl call.
    // The request code's size matches T through the iow::<T> helper above.
    let result = unsafe { libc::ioctl(fd, request, value as *const T) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("uinput ioctl");
    }
    Ok(())
}

fn event_as_bytes(event: &InputEvent) -> &[u8] {
    // SAFETY: InputEvent is repr(C), initialized, and viewed as bytes only for a single write.
    unsafe {
        std::slice::from_raw_parts(
            (event as *const InputEvent).cast::<u8>(),
            mem::size_of::<InputEvent>(),
        )
    }
}

fn char_to_stroke(character: char) -> Result<KeyStroke> {
    let stroke = match character {
        'a'..='z' => KeyStroke {
            code: KEY_A + (character as u16 - 'a' as u16),
            shift: false,
        },
        'A'..='Z' => KeyStroke {
            code: KEY_A + (character as u16 - 'A' as u16),
            shift: true,
        },
        '1' => key(KEY_1),
        '2' => key(KEY_2),
        '3' => key(KEY_3),
        '4' => key(KEY_4),
        '5' => key(KEY_5),
        '6' => key(KEY_6),
        '7' => key(KEY_7),
        '8' => key(KEY_8),
        '9' => key(KEY_9),
        '0' => key(KEY_0),
        '!' => shifted(KEY_1),
        '@' => shifted(KEY_2),
        '#' => shifted(KEY_3),
        '$' => shifted(KEY_4),
        '%' => shifted(KEY_5),
        '^' => shifted(KEY_6),
        '&' => shifted(KEY_7),
        '*' => shifted(KEY_8),
        '(' => shifted(KEY_9),
        ')' => shifted(KEY_0),
        '-' => key(KEY_MINUS),
        '_' => shifted(KEY_MINUS),
        '=' => key(KEY_EQUAL),
        '+' => shifted(KEY_EQUAL),
        '[' => key(KEY_LEFTBRACE),
        '{' => shifted(KEY_LEFTBRACE),
        ']' => key(KEY_RIGHTBRACE),
        '}' => shifted(KEY_RIGHTBRACE),
        '\\' => key(KEY_BACKSLASH),
        '|' => shifted(KEY_BACKSLASH),
        ';' => key(KEY_SEMICOLON),
        ':' => shifted(KEY_SEMICOLON),
        '\'' => key(KEY_APOSTROPHE),
        '"' => shifted(KEY_APOSTROPHE),
        '`' => key(KEY_GRAVE),
        '~' => shifted(KEY_GRAVE),
        ',' => key(KEY_COMMA),
        '<' => shifted(KEY_COMMA),
        '.' => key(KEY_DOT),
        '>' => shifted(KEY_DOT),
        '/' => key(KEY_SLASH),
        '?' => shifted(KEY_SLASH),
        ' ' => key(KEY_SPACE),
        '\n' => key(KEY_ENTER),
        '\t' => key(KEY_TAB),
        other => bail!(
            "unsupported character for uinput US keyboard mapping: U+{:04X}",
            other as u32
        ),
    };
    Ok(stroke)
}

fn key(code: u16) -> KeyStroke {
    KeyStroke { code, shift: false }
}

fn shifted(code: u16) -> KeyStroke {
    KeyStroke { code, shift: true }
}

fn parse_key_combo(combo: &str) -> Result<Vec<u16>> {
    let parts = combo
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        bail!("key combo must contain at least one key");
    }
    if parts.len() > 8 {
        bail!("key combo may contain at most 8 keys");
    }
    parts
        .iter()
        .map(|part| key_name_to_code(part))
        .collect::<Result<Vec<_>>>()
}

fn key_name_to_code(name: &str) -> Result<u16> {
    let normalized = name.trim().to_ascii_lowercase().replace(['-', '_'], "");
    let code = match normalized.as_str() {
        "ctrl" | "control" => KEY_LEFTCTRL,
        "alt" => KEY_LEFTALT,
        "shift" => KEY_LEFTSHIFT,
        "super" | "meta" | "win" | "windows" => KEY_LEFTMETA,
        "enter" | "return" => KEY_ENTER,
        "tab" => KEY_TAB,
        "space" => KEY_SPACE,
        "esc" | "escape" => KEY_ESC,
        "backspace" => KEY_BACKSPACE,
        "delete" | "del" => KEY_DELETE,
        "insert" | "ins" => KEY_INSERT,
        "home" => KEY_HOME,
        "end" => KEY_END,
        "pageup" | "pgup" => KEY_PAGEUP,
        "pagedown" | "pgdown" => KEY_PAGEDOWN,
        "up" => KEY_UP,
        "down" => KEY_DOWN,
        "left" => KEY_LEFT,
        "right" => KEY_RIGHT,
        "f1" => KEY_F1,
        "f2" => KEY_F2,
        "f3" => KEY_F3,
        "f4" => KEY_F4,
        "f5" => KEY_F5,
        "f6" => KEY_F6,
        "f7" => KEY_F7,
        "f8" => KEY_F8,
        "f9" => KEY_F9,
        "f10" => KEY_F10,
        "f11" => KEY_F11,
        "f12" => KEY_F12,
        "a" => KEY_A,
        "b" => KEY_B,
        "c" => KEY_C,
        "d" => KEY_D,
        "e" => KEY_E,
        "f" => KEY_F,
        "g" => KEY_G,
        "h" => KEY_H,
        "i" => KEY_I,
        "j" => KEY_J,
        "k" => KEY_K,
        "l" => KEY_L,
        "m" => KEY_M,
        "n" => KEY_N,
        "o" => KEY_O,
        "p" => KEY_P,
        "q" => KEY_Q,
        "r" => KEY_R,
        "s" => KEY_S,
        "t" => KEY_T,
        "u" => KEY_U,
        "v" => KEY_V,
        "w" => KEY_W,
        "x" => KEY_X,
        "y" => KEY_Y,
        "z" => KEY_Z,
        "0" => KEY_0,
        "1" => KEY_1,
        "2" => KEY_2,
        "3" => KEY_3,
        "4" => KEY_4,
        "5" => KEY_5,
        "6" => KEY_6,
        "7" => KEY_7,
        "8" => KEY_8,
        "9" => KEY_9,
        _ => bail!("unsupported key name in combo: {name}"),
    };
    Ok(code)
}

pub fn uinput_path() -> &'static Path {
    Path::new(UINPUT_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_ascii_text_to_us_evdev_strokes() {
        assert_eq!(char_to_stroke('a').expect("a maps"), key(KEY_A));
        assert_eq!(char_to_stroke('A').expect("A maps"), shifted(KEY_A));
        assert_eq!(char_to_stroke('!').expect("! maps"), shifted(KEY_1));
        assert_eq!(char_to_stroke('\n').expect("newline maps"), key(KEY_ENTER));
    }

    #[test]
    fn rejects_unsupported_text() {
        let err = char_to_stroke('é').expect_err("non-US character is rejected");
        assert!(err.to_string().contains("unsupported character"));
    }

    #[test]
    fn parses_key_combo_names() {
        assert_eq!(
            parse_key_combo("Ctrl+Shift+L").expect("combo parses"),
            vec![KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_L]
        );
        assert_eq!(
            parse_key_combo("Super+Space").expect("combo parses"),
            vec![KEY_LEFTMETA, KEY_SPACE]
        );
    }

    #[test]
    fn rejects_empty_key_combo() {
        let err = parse_key_combo(" + ").expect_err("empty combo is rejected");
        assert!(err.to_string().contains("at least one key"));
    }

    #[test]
    fn maps_pointer_points_to_absolute_range() {
        let bounds = PointerBounds {
            min_x: 0,
            min_y: 0,
            width: 7680,
            height: 4320,
        };
        assert_eq!(
            map_pointer_point(0.0, 0.0, bounds).expect("origin maps"),
            (0, 0)
        );
        assert_eq!(
            map_pointer_point(7679.0, 4319.0, bounds).expect("max maps"),
            (ABS_MAX_VALUE, ABS_MAX_VALUE)
        );
        assert_eq!(
            map_pointer_point(3840.0, 2160.0, bounds).expect("center maps"),
            (16_386, 16_387)
        );
    }

    #[test]
    fn rejects_pointer_outside_bounds() {
        let bounds = PointerBounds {
            min_x: 100,
            min_y: 200,
            width: 640,
            height: 480,
        };
        let err = map_pointer_point(99.0, 200.0, bounds).expect_err("x below bounds");
        assert!(err.to_string().contains("outside physical desktop bounds"));
    }
}

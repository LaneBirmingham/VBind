//! Keyboard and mouse output through `SendInput`. Keys go out as scan codes,
//! so games that read raw or DirectInput keyboards see them like real keys.
//! Input lands in whichever window has focus; Windows drops it for windows
//! running as administrator when VBind is not.

use std::time::Instant;
use vbind_core::keyboard::{KEYS, KeyboardReport, MouseAccumulator};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};

/// Marks input VBind sent, in `dwExtraInfo`.
const VBIND_MARK: usize = 0x5642_494E; // "VBIN"
const WHEEL_DELTA: i32 = 120;

/// The keyboard and mouse as one output. Dropping it releases everything held.
pub struct KeyboardMouse {
    keys: Vec<bool>,
    buttons: [bool; 5],
    mouse: MouseAccumulator,
    last: Option<Instant>,
}

impl Default for KeyboardMouse {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardMouse {
    pub fn new() -> KeyboardMouse {
        KeyboardMouse { keys: vec![false; KEYS.len()], buttons: [false; 5], mouse: MouseAccumulator::default(), last: None }
    }

    /// Send what changed since the last report, and mouse movement for the
    /// time since then.
    pub fn update(&mut self, r: &KeyboardReport, mouse_speed: f64, wheel_speed: f64) -> Result<(), String> {
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last = Some(now);

        let mut batch = Vec::new();
        for (i, (&want, held)) in r.keys.iter().zip(self.keys.iter_mut()).enumerate() {
            if want != *held {
                batch.push(key_input(i, !want));
                *held = want;
            }
        }
        for (i, (&want, held)) in r.buttons.iter().zip(self.buttons.iter_mut()).enumerate() {
            if want != *held {
                batch.push(button_input(i, !want));
                *held = want;
            }
        }
        let [dx, dy, wheel] = self.mouse.step(&r.mouse, dt, mouse_speed, wheel_speed);
        if dx != 0 || dy != 0 {
            batch.push(mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE));
        }
        if wheel != 0 {
            batch.push(mouse_input(0, 0, (wheel * WHEEL_DELTA) as u32, MOUSEEVENTF_WHEEL));
        }
        send(&batch)
    }

    /// Let go of every key and button.
    pub fn release_all(&mut self) {
        let mut batch = Vec::new();
        for (i, held) in self.keys.iter_mut().enumerate() {
            if std::mem::take(held) {
                batch.push(key_input(i, true));
            }
        }
        for (i, held) in self.buttons.iter_mut().enumerate() {
            if std::mem::take(held) {
                batch.push(button_input(i, true));
            }
        }
        let _ = send(&batch);
        self.last = None;
    }
}

impl Drop for KeyboardMouse {
    fn drop(&mut self) {
        self.release_all();
    }
}

fn key_input(i: usize, up: bool) -> INPUT {
    let (_, scan, extended, _) = KEYS[i];
    let mut flags = KEYEVENTF_SCANCODE;
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: scan, dwFlags: KEYBD_EVENT_FLAGS(flags.0), time: 0, dwExtraInfo: VBIND_MARK },
        },
    }
}

fn button_input(i: usize, up: bool) -> INPUT {
    let (flags, data) = match (i, up) {
        (0, false) => (MOUSEEVENTF_LEFTDOWN, 0),
        (0, true) => (MOUSEEVENTF_LEFTUP, 0),
        (1, false) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (1, true) => (MOUSEEVENTF_RIGHTUP, 0),
        (2, false) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (2, true) => (MOUSEEVENTF_MIDDLEUP, 0),
        (x, false) => (MOUSEEVENTF_XDOWN, x as u32 - 2), // XBUTTON1 = 1, XBUTTON2 = 2
        (x, true) => (MOUSEEVENTF_XUP, x as u32 - 2),
    };
    mouse_input(0, 0, data, flags)
}

fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: VBIND_MARK } },
    }
}

fn send(batch: &[INPUT]) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }
    // SAFETY: the inputs are fully initialised and the size matches INPUT.
    let sent = unsafe { SendInput(batch, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == batch.len() {
        Ok(())
    } else {
        Err("Windows blocked keyboard/mouse input (is the game running as administrator?)".into())
    }
}

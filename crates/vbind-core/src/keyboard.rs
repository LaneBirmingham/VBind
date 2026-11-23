//! The `keyboard` output: keys, mouse buttons and mouse movement, for games
//! that only listen to a keyboard and mouse (for example a free-look key held
//! while the mouse moves).
//!
//! Profiles name controls `key.<name>` (`key.lalt`, `key.w`, `key.f1`),
//! `mouse.<button>` (`mouse.left`) and `mouse.x`, `mouse.y`, `mouse.wheel`.
//! Keys are sent as scan codes, which games read the same way as a real
//! keyboard, so names follow key positions on a US layout.

use crate::output::ControlKind;

/// A key: (name, scan code, extended, label).
pub type Key = (&'static str, u16, bool, &'static str);

/// Every key, in display order.
pub const KEYS: &[Key] = &[
    ("esc", 0x01, false, "Esc"),
    ("f1", 0x3B, false, "F1"),
    ("f2", 0x3C, false, "F2"),
    ("f3", 0x3D, false, "F3"),
    ("f4", 0x3E, false, "F4"),
    ("f5", 0x3F, false, "F5"),
    ("f6", 0x40, false, "F6"),
    ("f7", 0x41, false, "F7"),
    ("f8", 0x42, false, "F8"),
    ("f9", 0x43, false, "F9"),
    ("f10", 0x44, false, "F10"),
    ("f11", 0x57, false, "F11"),
    ("f12", 0x58, false, "F12"),
    ("f13", 0x64, false, "F13"),
    ("f14", 0x65, false, "F14"),
    ("f15", 0x66, false, "F15"),
    ("f16", 0x67, false, "F16"),
    ("f17", 0x68, false, "F17"),
    ("f18", 0x69, false, "F18"),
    ("f19", 0x6A, false, "F19"),
    ("f20", 0x6B, false, "F20"),
    ("f21", 0x6C, false, "F21"),
    ("f22", 0x6D, false, "F22"),
    ("f23", 0x6E, false, "F23"),
    ("f24", 0x76, false, "F24"),
    ("grave", 0x29, false, "`"),
    ("1", 0x02, false, "1"),
    ("2", 0x03, false, "2"),
    ("3", 0x04, false, "3"),
    ("4", 0x05, false, "4"),
    ("5", 0x06, false, "5"),
    ("6", 0x07, false, "6"),
    ("7", 0x08, false, "7"),
    ("8", 0x09, false, "8"),
    ("9", 0x0A, false, "9"),
    ("0", 0x0B, false, "0"),
    ("minus", 0x0C, false, "-"),
    ("equals", 0x0D, false, "="),
    ("backspace", 0x0E, false, "Backspace"),
    ("tab", 0x0F, false, "Tab"),
    ("q", 0x10, false, "Q"),
    ("w", 0x11, false, "W"),
    ("e", 0x12, false, "E"),
    ("r", 0x13, false, "R"),
    ("t", 0x14, false, "T"),
    ("y", 0x15, false, "Y"),
    ("u", 0x16, false, "U"),
    ("i", 0x17, false, "I"),
    ("o", 0x18, false, "O"),
    ("p", 0x19, false, "P"),
    ("lbracket", 0x1A, false, "["),
    ("rbracket", 0x1B, false, "]"),
    ("backslash", 0x2B, false, "\\"),
    ("capslock", 0x3A, false, "Caps Lock"),
    ("a", 0x1E, false, "A"),
    ("s", 0x1F, false, "S"),
    ("d", 0x20, false, "D"),
    ("f", 0x21, false, "F"),
    ("g", 0x22, false, "G"),
    ("h", 0x23, false, "H"),
    ("j", 0x24, false, "J"),
    ("k", 0x25, false, "K"),
    ("l", 0x26, false, "L"),
    ("semicolon", 0x27, false, ";"),
    ("apostrophe", 0x28, false, "'"),
    ("enter", 0x1C, false, "Enter"),
    ("lshift", 0x2A, false, "Left Shift"),
    ("z", 0x2C, false, "Z"),
    ("x", 0x2D, false, "X"),
    ("c", 0x2E, false, "C"),
    ("v", 0x2F, false, "V"),
    ("b", 0x30, false, "B"),
    ("n", 0x31, false, "N"),
    ("m", 0x32, false, "M"),
    ("comma", 0x33, false, ","),
    ("period", 0x34, false, "."),
    ("slash", 0x35, false, "/"),
    ("rshift", 0x36, false, "Right Shift"),
    ("lctrl", 0x1D, false, "Left Ctrl"),
    ("lwin", 0x5B, true, "Left Windows"),
    ("lalt", 0x38, false, "Left Alt"),
    ("space", 0x39, false, "Space"),
    ("ralt", 0x38, true, "Right Alt"),
    ("rwin", 0x5C, true, "Right Windows"),
    ("menu", 0x5D, true, "Menu"),
    ("rctrl", 0x1D, true, "Right Ctrl"),
    ("printscreen", 0x37, true, "Print Screen"),
    ("scrolllock", 0x46, false, "Scroll Lock"),
    ("insert", 0x52, true, "Insert"),
    ("home", 0x47, true, "Home"),
    ("pageup", 0x49, true, "Page Up"),
    ("delete", 0x53, true, "Delete"),
    ("end", 0x4F, true, "End"),
    ("pagedown", 0x51, true, "Page Down"),
    ("up", 0x48, true, "Up"),
    ("left", 0x4B, true, "Left"),
    ("down", 0x50, true, "Down"),
    ("right", 0x4D, true, "Right"),
    ("numlock", 0x45, false, "Num Lock"),
    ("numpad_divide", 0x35, true, "Numpad /"),
    ("numpad_multiply", 0x37, false, "Numpad *"),
    ("numpad_minus", 0x4A, false, "Numpad -"),
    ("numpad_plus", 0x4E, false, "Numpad +"),
    ("numpad_enter", 0x1C, true, "Numpad Enter"),
    ("numpad_decimal", 0x53, false, "Numpad ."),
    ("numpad0", 0x52, false, "Numpad 0"),
    ("numpad1", 0x4F, false, "Numpad 1"),
    ("numpad2", 0x50, false, "Numpad 2"),
    ("numpad3", 0x51, false, "Numpad 3"),
    ("numpad4", 0x4B, false, "Numpad 4"),
    ("numpad5", 0x4C, false, "Numpad 5"),
    ("numpad6", 0x4D, false, "Numpad 6"),
    ("numpad7", 0x47, false, "Numpad 7"),
    ("numpad8", 0x48, false, "Numpad 8"),
    ("numpad9", 0x49, false, "Numpad 9"),
];

/// Other names people write for keys.
const KEY_ALIASES: &[(&str, &str)] = &[
    ("alt", "lalt"),
    ("ctrl", "lctrl"),
    ("control", "lctrl"),
    ("shift", "lshift"),
    ("win", "lwin"),
    ("escape", "esc"),
    ("return", "enter"),
    ("del", "delete"),
    ("ins", "insert"),
    ("pgup", "pageup"),
    ("pgdn", "pagedown"),
    ("tilde", "grave"),
    ("backtick", "grave"),
];

/// Mouse buttons: (name, label).
pub const MOUSE_BUTTONS: &[(&str, &str)] =
    &[("left", "Left mouse button"), ("right", "Right mouse button"), ("middle", "Middle mouse button"), ("x1", "Mouse back button"), ("x2", "Mouse forward button")];

/// Mouse movement: (name, label). Values run -1 to 1 and set a speed.
pub const MOUSE_AXES: &[(&str, &str)] = &[("x", "Mouse left/right"), ("y", "Mouse up/down"), ("wheel", "Mouse wheel")];

/// Default mouse speed at full deflection, in pixels per second.
pub const DEFAULT_MOUSE_SPEED: f64 = 1200.0;
/// Default wheel speed at full deflection, in notches per second.
pub const DEFAULT_WHEEL_SPEED: f64 = 8.0;

/// A control on the keyboard output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyboardControl {
    /// Index into [`KEYS`].
    Key(u8),
    /// Index into [`MOUSE_BUTTONS`].
    MouseButton(u8),
    /// Index into [`MOUSE_AXES`].
    MouseAxis(u8),
}

impl KeyboardControl {
    pub fn parse(name: &str) -> Result<KeyboardControl, String> {
        if let Some(k) = name.strip_prefix("key.") {
            let k = k.to_ascii_lowercase();
            let k = KEY_ALIASES.iter().find(|a| a.0 == k).map_or(k.as_str(), |a| a.1);
            return KEYS.iter().position(|x| x.0 == k).map(|i| KeyboardControl::Key(i as u8)).ok_or_else(|| {
                let hint = crate::expr::suggest(k, KEYS.iter().map(|x| x.0)).map(|s| format!(" Did you mean 'key.{s}'?")).unwrap_or_default();
                format!("no key '{k}'.{hint} Keys are named by US layout position: key.a, key.1, key.f1, key.lalt, key.space, key.up, key.numpad5")
            });
        }
        if let Some(m) = name.strip_prefix("mouse.") {
            if let Some(i) = MOUSE_BUTTONS.iter().position(|x| x.0 == m) {
                return Ok(KeyboardControl::MouseButton(i as u8));
            }
            if let Some(i) = MOUSE_AXES.iter().position(|x| x.0 == m) {
                return Ok(KeyboardControl::MouseAxis(i as u8));
            }
            return Err(format!("no mouse control '{m}'. Use mouse.left, mouse.right, mouse.middle, mouse.x1, mouse.x2, mouse.x, mouse.y or mouse.wheel"));
        }
        Err(format!("keyboard has no control '{name}'. Use key.<name> (key.lalt, key.w), a mouse button (mouse.left) or mouse movement (mouse.x, mouse.y, mouse.wheel)"))
    }

    pub fn id(&self) -> String {
        match *self {
            KeyboardControl::Key(i) => format!("key.{}", KEYS[i as usize].0),
            KeyboardControl::MouseButton(i) => format!("mouse.{}", MOUSE_BUTTONS[i as usize].0),
            KeyboardControl::MouseAxis(i) => format!("mouse.{}", MOUSE_AXES[i as usize].0),
        }
    }

    pub fn label(&self) -> String {
        match *self {
            KeyboardControl::Key(i) => KEYS[i as usize].3.to_string(),
            KeyboardControl::MouseButton(i) => MOUSE_BUTTONS[i as usize].1.to_string(),
            KeyboardControl::MouseAxis(i) => MOUSE_AXES[i as usize].1.to_string(),
        }
    }

    pub fn kind(&self) -> ControlKind {
        match self {
            KeyboardControl::MouseAxis(_) => ControlKind::Axis,
            _ => ControlKind::Button,
        }
    }

    pub fn order(&self) -> u32 {
        match *self {
            KeyboardControl::Key(i) => i as u32,
            KeyboardControl::MouseButton(i) => 1000 + i as u32,
            KeyboardControl::MouseAxis(i) => 1100 + i as u32,
        }
    }

    pub fn all() -> Vec<KeyboardControl> {
        let mut v: Vec<KeyboardControl> = (0..KEYS.len() as u8).map(KeyboardControl::Key).collect();
        v.extend((0..MOUSE_BUTTONS.len() as u8).map(KeyboardControl::MouseButton));
        v.extend((0..MOUSE_AXES.len() as u8).map(KeyboardControl::MouseAxis));
        v
    }
}

/// One frame for the keyboard output.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardReport {
    /// Held keys, by index into [`KEYS`].
    pub keys: Vec<bool>,
    /// Held mouse buttons, by index into [`MOUSE_BUTTONS`].
    pub buttons: [bool; 5],
    /// Mouse x, y and wheel, `-1..=1`; a fraction of the output's speed.
    pub mouse: [f64; 3],
}

impl Default for KeyboardReport {
    fn default() -> Self {
        KeyboardReport { keys: vec![false; KEYS.len()], buttons: [false; 5], mouse: [0.0; 3] }
    }
}

impl KeyboardReport {
    pub fn reset(&mut self) {
        self.keys.iter_mut().for_each(|k| *k = false);
        self.buttons = [false; 5];
        self.mouse = [0.0; 3];
    }

    pub fn set(&mut self, c: KeyboardControl, value: f64) {
        let on = crate::expr::truthy(value);
        match c {
            KeyboardControl::Key(i) => self.keys[i as usize] |= on,
            KeyboardControl::MouseButton(i) => self.buttons[i as usize] |= on,
            KeyboardControl::MouseAxis(i) => self.mouse[i as usize] = crate::math::clamp(if value.is_finite() { value } else { 0.0 }, -1.0, 1.0),
        }
    }

    /// Held keys and buttons as ids, for display.
    pub fn held(&self) -> Vec<String> {
        let keys = self.keys.iter().enumerate().filter(|(_, h)| **h).map(|(i, _)| KeyboardControl::Key(i as u8).id());
        let buttons = self.buttons.iter().enumerate().filter(|(_, h)| **h).map(|(i, _)| KeyboardControl::MouseButton(i as u8).id());
        keys.chain(buttons).collect()
    }
}

/// Turns mouse speeds into whole pixel and wheel steps, carrying fractions
/// between ticks so slow movement still moves.
#[derive(Clone, Debug, Default)]
pub struct MouseAccumulator {
    carry: [f64; 3],
}

impl MouseAccumulator {
    /// Steps to send now for `mouse` held for `dt_s` seconds. Wheel steps are
    /// whole notches; a positive y moves the pointer down, a positive wheel scrolls up.
    pub fn step(&mut self, mouse: &[f64; 3], dt_s: f64, mouse_speed: f64, wheel_speed: f64) -> [i32; 3] {
        let mut out = [0; 3];
        for i in 0..3 {
            if mouse[i] == 0.0 {
                self.carry[i] = 0.0;
                continue;
            }
            // a leftover fraction from the other direction is not owed
            if self.carry[i] * mouse[i] < 0.0 {
                self.carry[i] = 0.0;
            }
            let speed = if i == 2 { wheel_speed } else { mouse_speed };
            let total = self.carry[i] + mouse[i] * speed * dt_s.clamp(0.0, 0.1);
            let whole = total.trunc();
            self.carry[i] = total - whole;
            out[i] = whole as i32;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_and_mouse() {
        assert_eq!(KeyboardControl::parse("key.alt").unwrap().id(), "key.lalt");
        assert_eq!(KeyboardControl::parse("key.W").unwrap().id(), "key.w");
        assert_eq!(KeyboardControl::parse("mouse.y").unwrap().kind(), ControlKind::Axis);
        assert!(KeyboardControl::parse("key.lat").unwrap_err().contains("Did you mean 'key.lalt'"));
        assert!(KeyboardControl::parse("pad.a").is_err());
        for c in KeyboardControl::all() {
            assert_eq!(KeyboardControl::parse(&c.id()).unwrap(), c, "{}", c.id());
        }
    }

    #[test]
    fn profiles_map_keys_and_mouse() {
        let text = |settings: &str| {
            format!(
                r#"{{ "vbind": 1, "id": "t", "name": "T",
                     "devices": {{ "s": {{ "match": [{{ "vid": "046D" }}] }} }},
                     "outputs": {{ "kb": {{ "type": "keyboard", "settings": {settings} }} }},
                     "map": {{ "kb.key.lalt": "s.hat1.x != 0 || s.hat1.y != 0", "kb.mouse.x": "s.hat1.x", "kb.mouse.y": "-s.hat1.y" }} }}"#
            )
        };
        let c = crate::profile::Profile::from_json(&text(r#"{ "mouse_speed": 600 }"#)).unwrap().compile().unwrap();
        assert_eq!(c.outputs[0].mouse_speed(), 600.0);
        assert_eq!(c.outputs[0].wheel_speed(), DEFAULT_WHEEL_SPEED);
        let mut r = crate::output::Report::neutral(crate::output::OutputKind::Keyboard, 0, 0);
        for m in &c.mappings {
            r.set(m.control, if m.key == "kb.mouse.y" { -1.0 } else { 1.0 });
        }
        let crate::output::Report::Keyboard(k) = r else { panic!() };
        assert_eq!(k.held(), vec!["key.lalt".to_string()]);
        assert_eq!(k.mouse, [1.0, -1.0, 0.0]);
        let errs = crate::profile::Profile::from_json(&text(r#"{ "speed": 1 }"#)).unwrap().compile().unwrap_err();
        assert!(errs[0].message.contains("mouse_speed and wheel_speed"), "{errs:?}");
    }

    #[test]
    fn mouse_carries_fractions() {
        let mut m = MouseAccumulator::default();
        // 128 px/s for 1/512 s is a quarter pixel per tick: one pixel every fourth tick
        let moved: i32 = (0..10).map(|_| m.step(&[1.0, 0.0, 0.0], 1.0 / 512.0, 128.0, 8.0)[0]).sum();
        assert_eq!(moved, 2);
        assert_eq!(m.step(&[-1.0, 0.5, 0.0], 0.01, 1000.0, 8.0), [-10, 5, 0]);
    }
}

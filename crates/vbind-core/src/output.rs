//! Virtual output devices: their controls and the reports sent to drivers.

use crate::expr::HatDir;
use crate::math;
use crate::state::{MAX_AXES, MAX_BUTTONS, MAX_HATS};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Kinds of virtual device VBind can create.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OutputKind {
    /// Virtual wired Xbox 360 controller through ViGEmBus.
    Xbox360,
    /// Virtual joystick (flight stick) through the vJoy driver: 8 axes,
    /// up to 128 buttons and 4 hats.
    Vjoy,
}

impl OutputKind {
    pub fn name(self) -> &'static str {
        match self {
            OutputKind::Xbox360 => "xbox360",
            OutputKind::Vjoy => "vjoy",
        }
    }
}

/// How an output control interprets its expression value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// `-1..=1`
    Axis,
    /// `0..=1`
    Trigger,
    /// pressed when `>= 0.5`
    Button,
    /// one direction of a hat; pressed when `>= 0.5`
    HatDir,
}

/// Xbox 360 controls in display order: (name, kind, XUSB button bit, label).
pub const XBOX_CONTROLS: &[(&str, ControlKind, u16, &str)] = &[
    ("lx", ControlKind::Axis, 0, "Left stick X"),
    ("ly", ControlKind::Axis, 0, "Left stick Y"),
    ("rx", ControlKind::Axis, 0, "Right stick X"),
    ("ry", ControlKind::Axis, 0, "Right stick Y"),
    ("lt", ControlKind::Trigger, 0, "Left trigger"),
    ("rt", ControlKind::Trigger, 0, "Right trigger"),
    ("a", ControlKind::Button, 0x1000, "A"),
    ("b", ControlKind::Button, 0x2000, "B"),
    ("x", ControlKind::Button, 0x4000, "X"),
    ("y", ControlKind::Button, 0x8000, "Y"),
    ("lb", ControlKind::Button, 0x0100, "Left bumper"),
    ("rb", ControlKind::Button, 0x0200, "Right bumper"),
    ("back", ControlKind::Button, 0x0020, "Back / View"),
    ("start", ControlKind::Button, 0x0010, "Start / Menu"),
    ("guide", ControlKind::Button, 0x0400, "Guide"),
    ("ls", ControlKind::Button, 0x0040, "Left stick click"),
    ("rs", ControlKind::Button, 0x0080, "Right stick click"),
    ("dpad_up", ControlKind::Button, 0x0001, "D-pad up"),
    ("dpad_down", ControlKind::Button, 0x0002, "D-pad down"),
    ("dpad_left", ControlKind::Button, 0x0004, "D-pad left"),
    ("dpad_right", ControlKind::Button, 0x0008, "D-pad right"),
];

/// A control on a virtual output device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OutputControl {
    /// Index into [`XBOX_CONTROLS`].
    Xbox(u8),
    VjoyAxis(u8),
    VjoyButton(u8),
    VjoyHat(u8, HatDir),
}

impl OutputControl {
    pub fn parse(kind: OutputKind, name: &str) -> Result<OutputControl, String> {
        match kind {
            OutputKind::Xbox360 => {
                let alias = match name {
                    "lthumb" => "ls",
                    "rthumb" => "rs",
                    "view" => "back",
                    "menu" => "start",
                    other => other,
                };
                XBOX_CONTROLS
                    .iter()
                    .position(|c| c.0 == alias)
                    .map(|i| OutputControl::Xbox(i as u8))
                    .ok_or_else(|| {
                        let names: Vec<&str> = XBOX_CONTROLS.iter().map(|c| c.0).collect();
                        let hint = crate::expr::suggest(name, names.iter().copied())
                            .map(|s| format!(" Did you mean '{s}'?"))
                            .unwrap_or_default();
                        format!("xbox360 has no control '{name}'.{hint} Controls: {}", names.join(", "))
                    })
            }
            OutputKind::Vjoy => {
                if let Some(i) = crate::state::AXIS_NAMES.iter().position(|a| *a == name) {
                    return Ok(OutputControl::VjoyAxis(i as u8));
                }
                if let Some(n) = name.strip_prefix('b').and_then(|n| n.parse::<usize>().ok()) {
                    if (1..=MAX_BUTTONS).contains(&n) {
                        return Ok(OutputControl::VjoyButton((n - 1) as u8));
                    }
                    return Err(format!("vjoy buttons run from b1 to b{MAX_BUTTONS}"));
                }
                if let Some(rest) = name.strip_prefix("hat") {
                    let (num, dir) = rest.split_once('.').unwrap_or((rest, ""));
                    let idx: usize = if num.is_empty() { 1 } else { num.parse().unwrap_or(0) };
                    if !(1..=MAX_HATS).contains(&idx) {
                        return Err(format!("vjoy hats run from hat1 to hat{MAX_HATS}"));
                    }
                    let d = match dir {
                        "up" => HatDir::Up,
                        "right" => HatDir::Right,
                        "down" => HatDir::Down,
                        "left" => HatDir::Left,
                        _ => {
                            return Err(format!(
                                "vjoy hat outputs are hat{idx}.up, hat{idx}.right, hat{idx}.down and hat{idx}.left; two neighbours together make a diagonal"
                            ));
                        }
                    };
                    return Ok(OutputControl::VjoyHat((idx - 1) as u8, d));
                }
                Err(format!(
                    "vjoy has no control '{name}'. Use an axis (x, y, z, rx, ry, rz, slider1, slider2), a button (b1..b{MAX_BUTTONS}) or a hat direction (hat1.up)"
                ))
            }
        }
    }

    pub fn id(&self) -> String {
        match *self {
            OutputControl::Xbox(i) => XBOX_CONTROLS[i as usize].0.to_string(),
            OutputControl::VjoyAxis(a) => crate::state::AXIS_NAMES[a as usize].to_string(),
            OutputControl::VjoyButton(b) => format!("b{}", b as u16 + 1),
            OutputControl::VjoyHat(h, d) => format!("hat{}.{}", h + 1, d.name()),
        }
    }

    pub fn kind(&self) -> ControlKind {
        match *self {
            OutputControl::Xbox(i) => XBOX_CONTROLS[i as usize].1,
            OutputControl::VjoyAxis(_) => ControlKind::Axis,
            OutputControl::VjoyButton(_) => ControlKind::Button,
            OutputControl::VjoyHat(..) => ControlKind::HatDir,
        }
    }

    /// Default human label ("Right trigger", "Button 5").
    pub fn default_label(&self) -> String {
        match *self {
            OutputControl::Xbox(i) => XBOX_CONTROLS[i as usize].3.to_string(),
            OutputControl::VjoyAxis(a) => format!("Axis {}", crate::state::AXIS_NAMES[a as usize].to_uppercase()),
            OutputControl::VjoyButton(b) => format!("Button {}", b as u16 + 1),
            OutputControl::VjoyHat(h, d) => format!("Hat {} {}", h + 1, d.name()),
        }
    }

    /// Position in the device's canonical display order.
    pub fn order(&self) -> u32 {
        match *self {
            OutputControl::Xbox(i) => i as u32,
            OutputControl::VjoyAxis(a) => a as u32,
            OutputControl::VjoyButton(b) => 100 + b as u32,
            OutputControl::VjoyHat(h, d) => 1000 + h as u32 * 10 + d as u32,
        }
    }
}

/// Every control an output kind offers, in display order.
pub fn all_controls(kind: OutputKind) -> Vec<OutputControl> {
    match kind {
        OutputKind::Xbox360 => (0..XBOX_CONTROLS.len() as u8).map(OutputControl::Xbox).collect(),
        OutputKind::Vjoy => {
            let mut v: Vec<OutputControl> = (0..MAX_AXES as u8).map(OutputControl::VjoyAxis).collect();
            v.extend((0..32u8).map(OutputControl::VjoyButton));
            for h in 0..MAX_HATS as u8 {
                for d in [HatDir::Up, HatDir::Right, HatDir::Down, HatDir::Left] {
                    v.push(OutputControl::VjoyHat(h, d));
                }
            }
            v
        }
    }
}

/// A frame for a virtual Xbox 360 controller, in XUSB report units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct XboxReport {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

/// A frame for a vJoy device in normalized units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VjoyReport {
    /// `-1..=1` per axis slot.
    pub axes: [f64; MAX_AXES],
    #[serde(with = "serde_big_array")]
    #[schemars(with = "Vec<bool>")]
    pub buttons: [bool; MAX_BUTTONS],
    /// Hat angles in hundredths of a degree, `None` when centered.
    pub hats: [Option<u32>; MAX_HATS],
}

impl Default for VjoyReport {
    fn default() -> Self {
        Self { axes: [0.0; MAX_AXES], buttons: [false; MAX_BUTTONS], hats: [None; MAX_HATS] }
    }
}

mod serde_big_array {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use crate::state::MAX_BUTTONS;

    pub fn serialize<S: Serializer>(v: &[bool; MAX_BUTTONS], s: S) -> Result<S::Ok, S::Error> {
        v.as_slice().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[bool; MAX_BUTTONS], D::Error> {
        let v: Vec<bool> = Vec::deserialize(d)?;
        let mut out = [false; MAX_BUTTONS];
        for (o, b) in out.iter_mut().zip(v) {
            *o = b;
        }
        Ok(out)
    }
}

/// One output device's frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    Xbox(XboxReport),
    Vjoy(VjoyReport),
}

impl Report {
    pub fn neutral(kind: OutputKind) -> Report {
        match kind {
            OutputKind::Xbox360 => Report::Xbox(XboxReport::default()),
            OutputKind::Vjoy => Report::Vjoy(VjoyReport::default()),
        }
    }

    /// Write one evaluated control value into this report.
    pub fn set(&mut self, control: OutputControl, value: f64) {
        let on = crate::expr::truthy(value);
        match (self, control) {
            (Report::Xbox(r), OutputControl::Xbox(i)) => {
                let (name, kind, bit, _) = XBOX_CONTROLS[i as usize];
                match kind {
                    ControlKind::Button | ControlKind::HatDir => {
                        if on {
                            r.buttons |= bit;
                        }
                    }
                    ControlKind::Trigger => {
                        let t = math::to_trigger(value);
                        if name == "lt" {
                            r.left_trigger = t;
                        } else {
                            r.right_trigger = t;
                        }
                    }
                    ControlKind::Axis => {
                        let s = math::to_stick(value);
                        match name {
                            "lx" => r.thumb_lx = s,
                            "ly" => r.thumb_ly = s,
                            "rx" => r.thumb_rx = s,
                            _ => r.thumb_ry = s,
                        }
                    }
                }
            }
            (Report::Vjoy(r), OutputControl::VjoyAxis(a)) => r.axes[a as usize] = math::clamp(value, -1.0, 1.0),
            (Report::Vjoy(r), OutputControl::VjoyButton(b)) => r.buttons[b as usize] = on,
            (Report::Vjoy(r), OutputControl::VjoyHat(h, d)) => {
                if on {
                    r.hats[h as usize] = Some(combine_hat(r.hats[h as usize], d));
                }
            }
            _ => {}
        }
    }
}

/// Add a pressed cardinal direction to a hat angle, producing diagonals for
/// neighbours and cancelling opposites to the newer direction.
fn combine_hat(current: Option<u32>, dir: HatDir) -> u32 {
    let new = dir.angle();
    match current {
        None => new,
        Some(cur) => {
            let d = math::hat_angle_distance(cur, new);
            if d == 9000 {
                // neighbours: midpoint, handling wrap-around between left and up
                let (a, b) = (cur.min(new), cur.max(new));
                if b - a > 18000 { (b + (36000 - b + a) / 2) % 36000 } else { (a + b) / 2 }
            } else {
                new
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbox_report_fields() {
        let mut r = Report::neutral(OutputKind::Xbox360);
        r.set(OutputControl::parse(OutputKind::Xbox360, "rt").unwrap(), 1.0);
        r.set(OutputControl::parse(OutputKind::Xbox360, "lx").unwrap(), -1.0);
        r.set(OutputControl::parse(OutputKind::Xbox360, "a").unwrap(), 1.0);
        r.set(OutputControl::parse(OutputKind::Xbox360, "dpad_up").unwrap(), 0.4);
        r.set(OutputControl::parse(OutputKind::Xbox360, "rthumb").unwrap(), 1.0);
        let Report::Xbox(x) = r else { panic!() };
        assert_eq!(x.right_trigger, 255);
        assert_eq!(x.thumb_lx, -32768);
        assert_eq!(x.buttons, 0x1000 | 0x0080);
    }

    #[test]
    fn vjoy_hat_diagonals() {
        let up = OutputControl::parse(OutputKind::Vjoy, "hat1.up").unwrap();
        let right = OutputControl::parse(OutputKind::Vjoy, "hat1.right").unwrap();
        let left = OutputControl::parse(OutputKind::Vjoy, "hat1.left").unwrap();
        let mut r = Report::neutral(OutputKind::Vjoy);
        r.set(up, 1.0);
        r.set(right, 1.0);
        let Report::Vjoy(v) = &r else { panic!() };
        assert_eq!(v.hats[0], Some(4500));
        let mut r = Report::neutral(OutputKind::Vjoy);
        r.set(up, 1.0);
        r.set(left, 1.0);
        let Report::Vjoy(v) = &r else { panic!() };
        assert_eq!(v.hats[0], Some(31500));
    }

    #[test]
    fn helpful_output_errors() {
        let e = OutputControl::parse(OutputKind::Xbox360, "rtt").unwrap_err();
        assert!(e.contains("Did you mean 'rt'"), "{e}");
        assert!(OutputControl::parse(OutputKind::Vjoy, "hat1.up_left").is_err());
        assert!(OutputControl::parse(OutputKind::Vjoy, "b129").is_err());
    }
}

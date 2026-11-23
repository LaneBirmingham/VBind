//! Convert a "Galactic Racer HOSAS / HOTAS Bridge" `hosas_config.json` into a
//! VBind profile.
//!
//! Supported: Podracer Style (HOSAS), Traditional HOTAS and Single Stick
//! modes, learned Xbox button bindings (buttons and hats), digital pitch,
//! inversion, deadzones, response curves, Hybrid Preserve Thrust, and the
//! Additional Input (twist) modes Pitch, Fine steering and Move Camera.
//! Anything else is reported as a warning rather than silently dropped.

use indexmap::IndexMap;
use serde_json::Value;
use vbind_core::profile::{DeviceMatch, DeviceRole, OutputDef, Profile, UsbId, VarDef};
use vbind_core::output::OutputKind;
use vbind_core::state::{DeviceInfo, Guid};

pub struct Conversion {
    pub profile: Profile,
    /// Role to the exact device the bridge used on this PC.
    pub assignments: Vec<(String, Guid)>,
    pub warnings: Vec<String>,
}

pub struct Options {
    pub id: String,
    pub name: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { id: "galactic-racer-dual-hosas".into(), name: "Galactic Racer - Dual HOSAS".into() }
    }
}

/// Galactic Racer's default Xbox layout, used to label outputs.
const GAME_ACTIONS: &[(&str, &str)] = &[
    ("rt", "Accelerate"),
    ("lt", "Brake / reverse"),
    ("lx", "Steer"),
    ("ly", "Pitch"),
    ("rx", "Move camera"),
    ("a", "Boost"),
    ("x", "Advanced maneuver"),
    ("b", "Malfunction recovery"),
    ("rs", "Toggle boost mode"),
    ("lb", "Activate ability"),
    ("rb", "Change camera"),
    ("dpad_up", "Ability 1"),
    ("dpad_left", "Ability 2"),
    ("dpad_right", "Ability 3"),
    ("dpad_down", "Ability 4"),
    ("start", "Pause"),
    ("back", "View build"),
];

/// Bridge binding keys to Xbox controls.
const BUTTON_KEYS: &[(&str, &str)] = &[
    ("A", "a"),
    ("B", "b"),
    ("X", "x"),
    ("Y", "y"),
    ("LB", "lb"),
    ("RB", "rb"),
    ("BACK", "back"),
    ("START", "start"),
    ("LTHUMB", "ls"),
    ("RTHUMB", "rs"),
    ("DUP", "dpad_up"),
    ("DDOWN", "dpad_down"),
    ("DLEFT", "dpad_left"),
    ("DRIGHT", "dpad_right"),
];

const AXES: [&str; 8] = vbind_core::state::AXIS_NAMES;

fn num(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}
fn int(v: &Value, key: &str, default: i64) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(default)
}
fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn fmt_num(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

/// Bridge axis tuning profile, as an expression around `inner`.
struct Tune {
    deadzone: f64,
    curve: f64,
    full: f64,
    weight: f64,
    two_stage: bool,
}

impl Tune {
    fn read(cfg: &Value, key: &str, default_dz: f64) -> Tune {
        let p = cfg.get("axis_tuning").and_then(|t| t.get(key)).cloned().unwrap_or(Value::Null);
        let mut dz = num(&p, "deadzone", default_dz);
        if !(0.0..=40.0).contains(&dz) {
            dz = 6.0;
        }
        let mut curve = num(&p, "curve", 1.0);
        if !(0.2..=3.0).contains(&curve) {
            curve = 1.0;
        }
        let mut full = num(&p, "full_output", 100.0);
        if !(50.0..=100.0).contains(&full) {
            full = 100.0;
        }
        let weight = match p.get("weighting").and_then(Value::as_f64) {
            Some(w) => w.clamp(0.0, 100.0) / 100.0,
            None if curve == 1.0 => 0.5,
            None => 0.0,
        };
        Tune { deadzone: dz / 100.0, curve, full: full / 100.0, weight, two_stage: flag(&p, "two_stage") }
    }

    fn apply(&self, inner: &str, warnings: &mut Vec<String>, what: &str) -> String {
        if self.two_stage {
            warnings.push(format!("{what}: two-stage response curves are not converted; using its single-stage settings"));
        }
        let mut e = if self.deadzone > 0.0 { format!("deadzone({inner}, {})", fmt_num(self.deadzone)) } else { inner.to_string() };
        if self.curve != 1.0 || self.full < 1.0 {
            e = format!("curve({e}, {}, {}, {})", fmt_num(self.curve), fmt_num(self.weight), fmt_num(self.full));
        }
        e
    }

    fn is_identity(&self) -> bool {
        self.deadzone == 0.0 && self.curve == 1.0 && self.full >= 1.0
    }
}

struct Roles<'a> {
    lookup: &'a dyn Fn(Guid) -> Option<DeviceInfo>,
    roles: IndexMap<String, (Guid, DeviceRole)>,
}

impl Roles<'_> {
    /// Role name for a bridge DirectInput GUID, adding a new role when needed.
    fn role_for(&mut self, guid_text: &str, preferred: &str, label: &str, name_hint: &str) -> Option<String> {
        let guid: Guid = guid_text.parse().ok()?;
        if let Some((name, _)) = self.roles.iter().find(|(_, (g, _))| *g == guid) {
            return Some(name.clone());
        }
        let name = if self.roles.contains_key(preferred) {
            (2..).map(|i| format!("{preferred}{i}")).find(|n| !self.roles.contains_key(n)).unwrap()
        } else {
            preferred.to_string()
        };
        let info = (self.lookup)(guid);
        let matcher = match &info {
            Some(DeviceInfo { vid: Some(v), pid: Some(p), .. }) => DeviceMatch { vid: Some(UsbId(*v)), pid: Some(UsbId(*p)), name: None },
            _ if !name_hint.is_empty() => DeviceMatch { vid: None, pid: None, name: Some(name_hint.to_string()) },
            _ => DeviceMatch::default(),
        };
        self.roles.insert(name.clone(), (guid, DeviceRole { label: Some(label.to_string()), matches: vec![matcher], optional: false }));
        Some(name)
    }
}

fn hat_dir(angle: i64) -> &'static str {
    match ((angle.rem_euclid(36000) + 2250) / 4500) % 8 {
        0 => "up",
        1 => "up_right",
        2 => "right",
        3 => "down_right",
        4 => "down",
        5 => "down_left",
        6 => "left",
        _ => "up_left",
    }
}

pub fn convert(cfg: &Value, lookup: &dyn Fn(Guid) -> Option<DeviceInfo>, opts: &Options) -> Result<Conversion, String> {
    if !cfg.is_object() || cfg.get("control_mode").is_none() {
        return Err("this does not look like a HOSAS Bridge hosas_config.json (no control_mode)".into());
    }
    let mut warnings = Vec::new();
    let mut roles = Roles { lookup, roles: IndexMap::new() };
    let mut vars: IndexMap<String, VarDef> = IndexMap::new();
    let mut map: IndexMap<String, String> = IndexMap::new();
    let mode = text(cfg, "control_mode").to_uppercase();
    let backend = text(cfg, "input_backend");
    if backend == "WinMM" {
        warnings.push("the bridge used WinMM device numbers; VBind reads DirectInput, so check the device roles".into());
    }
    let global_dz = num(cfg, "deadzone", 6.0);

    let axis_ref = |role: &str, axis: i64, invert: bool| -> String {
        let a = AXES.get(axis.clamp(0, 7) as usize).copied().unwrap_or("y");
        format!("{}{role}.{a}", if invert { "-" } else { "" })
    };

    let mut steer_terms: Vec<String> = Vec::new();
    let mut pitch_terms: Vec<String> = Vec::new();
    let mut camera_terms: Vec<String> = Vec::new();

    match mode.as_str() {
        "HOSAS" => {
            let left = roles
                .role_for(text(cfg, "left_di_guid"), "left", "Left hand stick", "")
                .ok_or("the bridge config has no Left Input DirectInput device")?;
            let right = roles
                .role_for(text(cfg, "right_di_guid"), "right", "Right hand stick", "")
                .ok_or("the bridge config has no Right Input DirectInput device")?;
            let lt = Tune::read(cfg, "hosas_left", global_dz);
            let rt = Tune::read(cfg, "hosas_right", global_dz);
            let l_src = axis_ref(&left, int(cfg, "left_axis", 1), flag(cfg, "left_invert"));
            let r_src = axis_ref(&right, int(cfg, "right_axis", 1), flag(cfg, "right_invert"));
            vars.insert("l".into(), VarDef::Labeled { expr: lt.apply(&l_src, &mut warnings, "Left Input"), label: Some("Left stick thrust".into()) });
            vars.insert("r".into(), VarDef::Labeled { expr: rt.apply(&r_src, &mut warnings, "Right Input"), label: Some("Right stick thrust".into()) });
            let preserve = cfg.get("preserve_thrust").and_then(Value::as_bool).unwrap_or(true);
            let drive_raw = if preserve { "hosas_drive(l, r)".to_string() } else { "hosas_drive(l, r, 0)".to_string() };
            let thrust = Tune::read(cfg, "hosas_thrust", 0.0);
            let steer_t = Tune::read(cfg, "hosas_steering", 0.0);
            vars.insert(
                "drive".into(),
                VarDef::Labeled {
                    expr: if thrust.is_identity() { drive_raw } else { thrust.apply(&drive_raw, &mut warnings, "Thrust Output") },
                    label: Some(if preserve { "Thrust, stronger stick wins" } else { "Thrust, average of sticks" }.into()),
                },
            );
            let steer_raw = "clamp(l - r, -1, 1)";
            steer_terms.push(if steer_t.is_identity() { steer_raw.to_string() } else { steer_t.apply(steer_raw, &mut warnings, "Steering Output") });
            if flag(cfg, "left_throttle_mode") || flag(cfg, "right_throttle_mode") {
                warnings.push("a HOSAS input had 'Throttle axis' checked; VBind reads it as a centered axis, which is what the bridge's DirectInput path does".into());
            }
        }
        "SINGLE" | "HOTAS" => {
            let stick = roles
                .role_for(text(cfg, "hotas_stick_di_guid"), "stick", "Stick", "")
                .ok_or("the bridge config has no stick DirectInput device")?;
            let tx = Tune::read(cfg, "hotas_x", global_dz);
            let ty = Tune::read(cfg, "hotas_y", global_dz);
            let x = tx.apply(&axis_ref(&stick, int(cfg, "hotas_stick_x_axis", 0), flag(cfg, "hotas_stick_invert_x")), &mut warnings, "Stick X");
            let y = ty.apply(&axis_ref(&stick, int(cfg, "hotas_stick_y_axis", 1), flag(cfg, "hotas_stick_invert_y")), &mut warnings, "Stick Y");
            steer_terms.push(x);
            if mode == "SINGLE" {
                vars.insert("drive".into(), VarDef::Labeled { expr: y, label: Some("Stick forward/back".into()) });
            } else {
                vars.insert("pitch".into(), VarDef::Labeled { expr: y, label: Some("Stick pitch".into()) });
                pitch_terms.push("pitch".into());
                let throttle = roles
                    .role_for(text(cfg, "hotas_throttle_di_guid"), "throttle", "Throttle", "")
                    .ok_or("the bridge config has no throttle DirectInput device")?;
                let t_src = axis_ref(&throttle, int(cfg, "hotas_throttle_axis", 1), flag(cfg, "hotas_throttle_invert"));
                vars.insert("travel".into(), VarDef::Labeled { expr: format!("lever({t_src})"), label: Some("Throttle travel".into()) });
                let threshold = (num(cfg, "hotas_brake_threshold", 5.0) / 100.0).clamp(0.0, 0.5);
                let zone = (num(cfg, "hotas_brake_zone", 20.0) / 100.0).clamp(0.01, 0.5);
                let lever = match text(cfg, "hotas_brake_behavior") {
                    "Off" => "travel".to_string(),
                    "Analog Brake Zone" | "Centered Stick" => {
                        let f = if text(cfg, "hotas_brake_behavior") == "Centered Stick" { 0.5 } else { zone };
                        format!("travel <= {f} ? clamp(travel / {f} - 1, -1, 0) : clamp((travel - {f}) / {}, 0, 1)", fmt_num(1.0 - f), f = fmt_num(f))
                    }
                    _ => format!("travel <= {} ? -1 : travel", fmt_num(threshold)),
                };
                let tt = Tune::read(cfg, "hotas_throttle", 0.0);
                let drive = if tt.is_identity() {
                    lever
                } else {
                    // The bridge curves only the forward part.
                    vars.insert("lever_drive".into(), VarDef::Expr(lever));
                    format!("lever_drive > 0 ? {} : lever_drive", tt.apply("lever_drive", &mut warnings, "Throttle"))
                };
                vars.insert("drive".into(), VarDef::Labeled { expr: drive, label: Some("Throttle drive".into()) });
            }
        }
        "HOTAT" => return Err("Podracer Style (HOTAT) configs are not converted yet; ask your agent to write the profile from the bridge settings".into()),
        "WHEEL" => return Err("Driving Wheel configs are not converted yet; ask your agent to write the profile from the bridge settings".into()),
        other => return Err(format!("unknown bridge control mode '{other}'")),
    }

    // Additional Inputs (twist axes).
    for (key, label) in [("twist_left", "Additional Input 1"), ("twist_right", "Additional Input 2")] {
        let tw = cfg.get(key).cloned().unwrap_or(Value::Null);
        let tw_mode = text(&tw, "mode");
        if tw_mode.is_empty() || tw_mode == "Off" {
            continue;
        }
        let Some(role) = roles.role_for(text(&tw, "di_guid"), "aux", label, "") else {
            warnings.push(format!("{label} ({tw_mode}) has no DirectInput device and was skipped"));
            continue;
        };
        if text(&tw, "axis_type") == "Lever" {
            warnings.push(format!("{label}: lever-type additional inputs are not converted"));
            continue;
        }
        let src = axis_ref(&role, int(&tw, "axis", 0), flag(&tw, "invert"));
        let tune = Tune::read(cfg, key, num(&tw, "deadzone", 6.0));
        let var = format!("{key}_value");
        vars.insert(var.clone(), VarDef::Labeled { expr: tune.apply(&src, &mut warnings, label), label: Some(format!("{label} ({tw_mode})")) });
        match tw_mode {
            "Fine steering" => steer_terms.push(format!("{var} * {}", fmt_num(num(&tw, "steer_strength", 30.0) / 100.0))),
            "Pitch" => pitch_terms.push(var),
            "Move Camera" => {
                let sens = num(&tw, "camera_sensitivity", 100.0) / 100.0;
                let cdz = num(&tw, "camera_deadzone", 6.0) / 100.0;
                camera_terms.push(format!("deadzone({src}, {}) * {}", fmt_num(cdz), fmt_num(sens)));
            }
            other => warnings.push(format!("{label}: mode '{other}' is not converted")),
        }
    }
    if !matches!(text(cfg, "pedal_mode"), "" | "Off") {
        warnings.push(format!("Rudder Pedals mode '{}' is not converted", text(cfg, "pedal_mode")));
    }

    // Learned bindings.
    let bindings = cfg.get("bindings").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut source = |b: &Value, key: &str, warnings: &mut Vec<String>| -> Option<String> {
        let guid = text(b, "di_guid");
        if guid.is_empty() {
            warnings.push(format!("binding {key} uses a WinMM device number and was skipped"));
            return None;
        }
        let role = roles.role_for(guid, "dev", "Extra device", text(b, "device_name"))?;
        match text(b, "kind") {
            "button" => Some(format!("{role}.b{}", int(b, "button", 0) + 1)),
            "pov" => Some(format!("{role}.hat{}.{}", int(b, "pov_index", 0) + 1, hat_dir(int(b, "pov", 0)))),
            other => {
                warnings.push(format!("binding {key} has unknown kind '{other}'"));
                None
            }
        }
    };
    let mut pitch_up = None;
    let mut pitch_down = None;
    let mut manual_lt = None;
    let mut manual_rt = None;
    for (key, b) in &bindings {
        if let Some((_, ctrl)) = BUTTON_KEYS.iter().find(|(k, _)| k == key) {
            if let Some(src) = source(b, key, &mut warnings) {
                map.insert(format!("pad.{ctrl}"), src);
            }
            continue;
        }
        match key.as_str() {
            "PITCHUP" => pitch_up = source(b, key, &mut warnings),
            "PITCHDOWN" => pitch_down = source(b, key, &mut warnings),
            "LT" | "BRAKE" => manual_lt = source(b, key, &mut warnings),
            "RT" => manual_rt = source(b, key, &mut warnings),
            "WALKMODE" => warnings.push("Walking Toggle is not converted; ask your agent to add a walking layer with toggle()".into()),
            "LOOKLEFT" | "LOOKRIGHT" | "LOOKBACK" => warnings.push(format!("{key} sends a keyboard key, which VBind does not output yet")),
            "WHEELHANDBRAKE" => {}
            other => warnings.push(format!("binding {other} is not converted")),
        }
    }

    // Analog outputs.
    let mut rt = "max(drive, 0)".to_string();
    let mut lt = "max(-drive, 0)".to_string();
    if let Some(s) = manual_rt {
        rt = format!("max({rt}, {s})");
    }
    if let Some(s) = manual_lt {
        lt = format!("max({lt}, {s})");
    }
    let mut analog: IndexMap<String, String> = IndexMap::new();
    analog.insert("pad.rt".into(), rt);
    analog.insert("pad.lt".into(), lt);
    if steer_terms.len() == 1 && steer_terms[0] == "clamp(l - r, -1, 1)" {
        vars.insert("steer".into(), VarDef::Labeled { expr: steer_terms[0].clone(), label: Some("Steering, left minus right".into()) });
        analog.insert("pad.lx".into(), "steer".into());
    } else {
        analog.insert("pad.lx".into(), format!("clamp({}, -1, 1)", steer_terms.join(" + ")));
    }
    let pitch_analog = if pitch_terms.is_empty() { None } else { Some(format!("clamp({}, -1, 1)", pitch_terms.join(" + "))) };
    let ly = match (pitch_up, pitch_down, pitch_analog) {
        (None, None, a) => a,
        (up, down, a) => {
            let u = up.unwrap_or_else(|| "0".into());
            let d = down.unwrap_or_else(|| "0".into());
            Some(format!("{u} and not {d} ? 1 : ({d} and not {u} ? -1 : ({u} and {d} ? 0 : {}))", a.unwrap_or_else(|| "0".into())))
        }
    };
    if let Some(ly) = ly {
        analog.insert("pad.ly".into(), ly);
    }
    if !camera_terms.is_empty() {
        analog.insert("pad.rx".into(), format!("clamp({}, -1, 1)", camera_terms.join(" + ")));
    }
    analog.extend(map);
    let map = analog;

    let mut labels = IndexMap::new();
    for key in map.keys() {
        let ctrl = key.trim_start_matches("pad.");
        if let Some((_, action)) = GAME_ACTIONS.iter().find(|(c, _)| *c == ctrl) {
            labels.insert(key.clone(), action.to_string());
        }
    }

    let mut devices = IndexMap::new();
    let mut assignments = Vec::new();
    for (name, (guid, role)) in roles.roles {
        if role.matches.iter().all(DeviceMatch::is_empty) {
            warnings.push(format!("device role '{name}' could not be identified; assign it in VBind"));
        }
        assignments.push((name.clone(), guid));
        devices.insert(name, role);
    }
    let mut outputs = IndexMap::new();
    outputs.insert("pad".to_string(), OutputDef { kind: OutputKind::Xbox360, label: Some("Virtual Xbox 360".into()), vjoy_id: None });

    let polling = int(cfg, "polling_hz", 500);
    let profile = Profile {
        schema: None,
        vbind: 1,
        id: opts.id.clone(),
        name: opts.name.clone(),
        description: format!(
            "Converted from the Galactic Racer HOSAS Bridge ({} mode). Both sticks forward to accelerate, push one further than the other to steer, both back to brake.",
            match mode.as_str() {
                "HOSAS" => "Podracer Style (HOSAS)",
                "SINGLE" => "Single Stick",
                _ => "Traditional HOTAS",
            }
        ),
        game: Some("STAR WARS: Galactic Racer".into()),
        author: None,
        devices,
        outputs,
        vars,
        map,
        labels,
        polling_hz: if (100..=1000).contains(&polling) && polling != 500 { Some(polling as u32) } else { None },
    };
    if let Err(errs) = profile.compile() {
        return Err(format!(
            "the converted profile did not validate: {}",
            errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")
        ));
    }
    Ok(Conversion { profile, assignments, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vbind_core::output::Report;
    use vbind_core::state::DeviceState;

    const CONFIG: &str = include_str!("../tests/fixtures/hosas_config.json");

    fn lookup(g: Guid) -> Option<DeviceInfo> {
        let s = g.to_string();
        let (vid, pid, name) = if s.starts_with("F32C3250") {
            (0x044F, 0xB10A, "T.16000M Joystick")
        } else if s.starts_with("D66B73C0") {
            (0x046D, 0xC215, "Logitech Extreme 3D")
        } else {
            return None;
        };
        Some(DeviceInfo { instance: g, vid: Some(vid), pid: Some(pid), name: name.into(), caps: Default::default() })
    }

    fn reference_drive(l: f64, r: f64) -> f64 {
        vbind_core::math::hosas_drive(l, r, true)
    }

    #[test]
    fn converts_the_real_galactic_racer_config() {
        let cfg: Value = serde_json::from_str(CONFIG).unwrap();
        let c = convert(&cfg, &lookup, &Options::default()).unwrap();
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        let p = &c.profile;
        assert_eq!(p.devices.keys().collect::<Vec<_>>(), ["left", "right"]);
        assert_eq!(p.vars["l"].expr(), "deadzone(-left.y, 0.06)");
        assert_eq!(p.vars["r"].expr(), "deadzone(-right.y, 0.06)");
        assert_eq!(p.vars["drive"].expr(), "hosas_drive(l, r)");
        let expect = [
            ("pad.a", "left.b1"),
            ("pad.x", "right.b1"),
            ("pad.b", "right.b3"),
            ("pad.rs", "right.b4"),
            ("pad.lb", "left.b2"),
            ("pad.rb", "right.hat1.up"),
            ("pad.dpad_up", "left.hat1.up"),
            ("pad.dpad_right", "left.hat1.right"),
            ("pad.dpad_down", "left.hat1.down"),
            ("pad.dpad_left", "left.hat1.left"),
            ("pad.back", "right.b11"),
            ("pad.start", "right.b12"),
        ];
        for (k, v) in expect {
            assert_eq!(p.map.get(k).map(String::as_str), Some(v), "{k}");
        }
        assert_eq!(p.map.len(), expect.len() + 3);
        assert_eq!(p.labels["pad.a"], "Boost");
        let guids: Vec<String> = c.assignments.iter().map(|(r, g)| format!("{r}={g}")).collect();
        assert_eq!(guids, ["left=F32C3250-6366-11F1-8002-444553540000", "right=D66B73C0-5D02-11F1-8001-444553540000"]);
    }

    /// Run the converted profile against the bridge's own math over a grid
    /// of stick positions and compare every Xbox output.
    #[test]
    fn matches_bridge_output_math() {
        let cfg: Value = serde_json::from_str(CONFIG).unwrap();
        let c = convert(&cfg, &lookup, &Options::default()).unwrap();
        let compiled = c.profile.compile().unwrap();
        let mut rt = compiled.runtime();
        let steps: Vec<f64> = (-20..=20).map(|i| i as f64 / 20.0).collect();
        for &ly in &steps {
            for &ry in &steps {
                let mut left = DeviceState::default();
                let mut right = DeviceState::default();
                left.axes[1] = ly;
                right.axes[1] = ry;
                compiled.evaluate(&[Some(&left), Some(&right)], &mut rt);
                let Report::Xbox(x) = &rt.reports[0] else { panic!() };
                // Bridge: invert, 6% deadzone per side, preserve thrust, steer = l - r.
                let l = vbind_core::math::deadzone(-ly, 0.06);
                let r = vbind_core::math::deadzone(-ry, 0.06);
                let drive = reference_drive(l, r);
                let (want_rt, want_lt) = if drive >= 0.0 {
                    ((drive.clamp(0.0, 1.0) * 255.0).round() as u8, 0)
                } else {
                    (0, ((-drive).clamp(0.0, 1.0) * 255.0).round() as u8)
                };
                assert_eq!(x.right_trigger, want_rt, "RT at L={ly} R={ry}");
                assert_eq!(x.left_trigger, want_lt, "LT at L={ly} R={ry}");
                assert_eq!(x.thumb_lx, vbind_core::math::to_stick((l - r).clamp(-1.0, 1.0)), "LX at L={ly} R={ry}");
                assert_eq!(x.thumb_ly, 0);
            }
        }
    }

    #[test]
    fn buttons_and_hats_drive_the_right_xbox_bits() {
        let cfg: Value = serde_json::from_str(CONFIG).unwrap();
        let compiled = convert(&cfg, &lookup, &Options::default()).unwrap().profile.compile().unwrap();
        let mut rt = compiled.runtime();
        let cases: &[(bool, usize, Option<u32>, u16)] = &[
            (true, 0, None, 0x1000),        // left trigger -> A
            (false, 0, None, 0x4000),       // right trigger -> X
            (false, 2, None, 0x2000),       // right b3 -> B
            (false, 3, None, 0x0080),       // right b4 -> RS click
            (true, 1, None, 0x0100),        // left b2 -> LB
            (false, 10, None, 0x0020),      // right b11 -> Back
            (false, 11, None, 0x0010),      // right b12 -> Start
            (true, 99, Some(0), 0x0001),    // left hat up -> D-pad up
            (true, 99, Some(9000), 0x0008), // left hat right -> D-pad right
            (true, 99, Some(4500), 0x0000), // diagonal presses nothing
            (false, 99, Some(0), 0x0200),   // right hat up -> RB
        ];
        for &(on_left, button, hat, want) in cases {
            let mut left = DeviceState::default();
            let mut right = DeviceState::default();
            let dev = if on_left { &mut left } else { &mut right };
            if button < 128 {
                dev.buttons[button] = true;
            }
            dev.hats[0] = hat;
            compiled.evaluate(&[Some(&left), Some(&right)], &mut rt);
            let Report::Xbox(x) = &rt.reports[0] else { panic!() };
            assert_eq!(x.buttons, want, "left={on_left} button={button} hat={hat:?}");
        }
    }
}

//! Device layout cards: a shareable description of what a device's controls
//! are called and where they sit, so the UI can label and draw it. A card can
//! also carry pictures of the device (views) that an agent draws.

use crate::drawing::{MAX_VIEWS, View};
use crate::expr::InputControl;
use crate::profile::{Diagnostic, DeviceMatch, is_valid_id};
use crate::state::DeviceInfo;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const LAYOUT_FORMAT: u32 = 1;

/// Outline the UI draws behind the control markers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Flight stick seen from the side: grip on a base.
    Stick,
    /// Throttle quadrant.
    Throttle,
    Gamepad,
    Wheel,
    Pedals,
    /// A plain panel.
    #[default]
    Generic,
}

/// One physical control.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlInfo {
    /// What the control is, for example "Trigger" or "Thumb button (top of grip)".
    pub label: String,
    /// Where it sits, for grouping: "grip", "head", "base", "throttle".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Position on the generic shape drawing, `[x, y]` from 0 to 100 (top left is 0, 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    /// Which view the control is drawn on (a view id). May be left out when there is one view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// Where the control sits on its view, `[x, y]` in view units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    /// Radius of the control's marker in view units, for unusually big or small controls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

impl ControlInfo {
    pub fn new(label: impl Into<String>) -> ControlInfo {
        ControlInfo { label: label.into(), group: None, pos: None, view: None, at: None, size: None }
    }
}

/// A device layout card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "VBind device layout")]
pub struct DeviceLayout {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Layout format version. Always 1 for now.
    pub vbind_layout: u32,
    /// Lowercase id, for example "thrustmaster-t16000m".
    pub id: String,
    /// Product name shown in the UI.
    pub name: String,
    /// Which devices this layout describes.
    #[serde(rename = "match")]
    pub matches: Vec<DeviceMatch>,
    #[serde(default)]
    pub shape: Shape,
    /// Controls by id: axes (`x`, `y`, `rz`, `slider1`), buttons (`b1`), hats (`hat1`).
    pub controls: IndexMap<String, ControlInfo>,
    /// Controls the driver reports that do not physically exist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    /// Pictures of the device, drawn from kit parts and SVG paths. Controls are
    /// placed on them with `view` and `at`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<View>,
}

impl DeviceLayout {
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut errs = Vec::new();
        let d = |path: String, message: String| Diagnostic { path, message, pos: None };
        if self.vbind_layout != LAYOUT_FORMAT {
            errs.push(d("vbind_layout".into(), format!("unsupported layout format; use {LAYOUT_FORMAT}")));
        }
        if !is_valid_id(&self.id) {
            errs.push(d("id".into(), "use lowercase letters, digits and dashes".into()));
        }
        if self.matches.is_empty() || self.matches.iter().any(DeviceMatch::is_empty) {
            errs.push(d("match".into(), "give at least one vid/pid/name to recognise the device".into()));
        }
        for (key, info) in &self.controls {
            if let Err(m) = parse_layout_control(key) {
                errs.push(d(format!("controls.{key}"), m));
            }
            if let Some([x, y]) = info.pos {
                if !(0.0..=100.0).contains(&x) || !(0.0..=100.0).contains(&y) {
                    errs.push(d(format!("controls.{key}.pos"), "positions run from 0 to 100".into()));
                }
            }
        }
        if self.views.len() > MAX_VIEWS {
            errs.push(d("views".into(), format!("a layout can have at most {MAX_VIEWS} views")));
        }
        for (i, v) in self.views.iter().enumerate() {
            if self.views[..i].iter().any(|o| o.id == v.id) {
                errs.push(d(format!("views[{i}].id"), format!("two views are called '{}'", v.id)));
            }
            v.validate(&format!("views[{i}]"), &mut errs);
        }
        for (key, info) in &self.controls {
            let here = format!("controls.{key}");
            if let Some(view) = &info.view
                && !self.views.iter().any(|v| &v.id == view)
            {
                errs.push(d(format!("{here}.view"), format!("there is no view '{view}'")));
            }
            if let Some([x, y]) = info.at {
                match self.view_of(info) {
                    None if info.view.is_some() => {}
                    None if self.views.is_empty() => errs.push(d(format!("{here}.at"), "add a view before placing controls with at".into())),
                    None => errs.push(d(format!("{here}.view"), "say which view this control is on".into())),
                    Some(v) => {
                        let [w, h] = v.size;
                        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 || x > w || y > h {
                            errs.push(d(format!("{here}.at"), format!("[{x}, {y}] is outside the {w} x {h} view '{}'", v.id)));
                        }
                    }
                }
            }
            if let Some(r) = info.size
                && !(r.is_finite() && r > 0.0)
            {
                errs.push(d(format!("{here}.size"), "size is a marker radius above 0".into()));
            }
        }
        for key in &self.hidden {
            if let Err(m) = parse_layout_control(key) {
                errs.push(d(format!("hidden.{key}"), m));
            }
        }
        errs
    }

    /// The view a control is drawn on: its own, or the only view.
    pub fn view_of(&self, info: &ControlInfo) -> Option<&View> {
        match &info.view {
            Some(id) => self.views.iter().find(|v| &v.id == id),
            None if self.views.len() == 1 => self.views.first(),
            None => None,
        }
    }

    pub fn matches(&self, device: &DeviceInfo) -> bool {
        self.matches.iter().any(|m| m.matches(device))
    }

    /// Label for an input control. Hat directions use the hat's entry plus the
    /// direction: "Thumb hat up".
    pub fn label(&self, control: &InputControl) -> Option<String> {
        let base = &self.controls.get(&layout_key(control))?.label;
        Some(match control {
            InputControl::HatDir(_, d) => format!("{base} {}", d.name().replace('_', "-")),
            InputControl::HatX(_) => format!("{base} left/right"),
            InputControl::HatY(_) => format!("{base} up/down"),
            _ => base.clone(),
        })
    }
}

/// The layout key a control is described under (`hat1.up` lives under `hat1`).
pub fn layout_key(control: &InputControl) -> String {
    match control {
        InputControl::HatDir(h, _) | InputControl::HatX(h) | InputControl::HatY(h) => format!("hat{}", h + 1),
        other => other.id(),
    }
}

fn parse_layout_control(key: &str) -> Result<(), String> {
    if let Some(n) = key.strip_prefix("hat") {
        return match n.parse::<usize>() {
            Ok(1..=4) => Ok(()),
            _ => Err("hats are hat1 to hat4".into()),
        };
    }
    InputControl::parse(&[key]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_labels() {
        let l: DeviceLayout = serde_json::from_str(
            r#"{"vbind_layout":1,"id":"t16000m","name":"T.16000M","match":[{"vid":"044F","pid":"B10A"}],
                "shape":"stick","controls":{"b1":{"label":"Trigger","pos":[30,40]},"hat1":{"label":"Hat"},"y":{"label":"Pitch"}}}"#,
        )
        .unwrap();
        assert!(l.validate().is_empty());
        assert_eq!(l.label(&InputControl::Button(0)).as_deref(), Some("Trigger"));
        assert_eq!(l.label(&InputControl::HatDir(0, crate::expr::HatDir::Up)).as_deref(), Some("Hat up"));
        let mut bad = l.clone();
        bad.controls.insert("b999".into(), ControlInfo { pos: Some([120.0, 0.0]), ..ControlInfo::new("x") });
        assert_eq!(bad.validate().len(), 2);
    }

    #[test]
    fn places_controls_on_views() {
        let mut l: DeviceLayout = serde_json::from_str(
            r#"{"vbind_layout":1,"id":"s","name":"S","match":[{"vid":"1234"}],
                "views":[{"id":"side","label":"Side","size":[100,200],"parts":[{"kit":"grip.straight","box":[30,40,40,150]}]}],
                "controls":{"b1":{"label":"Trigger","at":[30,60]},"b2":{"label":"Thumb","view":"top","at":[5,5]},"b3":{"label":"Far","at":[150,10]}}}"#,
        )
        .unwrap();
        let errs: Vec<_> = l.validate().into_iter().map(|d| d.path).collect();
        assert_eq!(errs, ["controls.b2.view", "controls.b3.at"]);
        assert_eq!(l.view_of(&l.controls["b1"]).map(|v| v.id.as_str()), Some("side"));
        l.views.push(l.views[0].clone());
        let errs: Vec<_> = l.validate().into_iter().map(|d| d.path).collect();
        assert!(errs.contains(&"views[1].id".to_string()) && errs.contains(&"controls.b1.view".to_string()), "{errs:?}");
    }
}

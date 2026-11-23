//! Profiles: the shareable JSON file that describes a whole mapping.
//!
//! ```json
//! {
//!   "vbind": 1,
//!   "id": "my-game-dual-sticks",
//!   "name": "My Game - two sticks",
//!   "devices": { "left": { "match": [{ "vid": "044F", "pid": "B10A" }] } },
//!   "outputs": { "pad": { "type": "xbox360" } },
//!   "vars": { "thrust": "deadzone(-left.y, 0.06)" },
//!   "map": { "pad.rt": "max(thrust, 0)", "pad.a": "left.b1" },
//!   "labels": { "pad.a": "Boost" }
//! }
//! ```

use crate::expr::{self, EvalState, Env, InputRef, Node, Resolver, Scope};
use crate::integration::Integration;
use crate::output::{ControlKind, OutputControl, OutputKind, Report};
use crate::state::{DeviceInfo, DeviceState};
use indexmap::IndexMap;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

pub const PROFILE_FORMAT: u32 = 1;

/// A USB vendor or product ID, written as hex: `"044F"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UsbId(pub u16);

impl fmt::Display for UsbId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04X}", self.0)
    }
}

impl Serialize for UsbId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for UsbId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
        u16::from_str_radix(t, 16)
            .map(UsbId)
            .map_err(|_| serde::de::Error::custom(format!("'{s}' is not a hex USB ID like \"044F\"")))
    }
}

impl JsonSchema for UsbId {
    fn schema_name() -> Cow<'static, str> {
        "UsbId".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(0[xX])?[0-9A-Fa-f]{1,4}$",
            "description": "USB vendor or product ID in hex, for example \"044F\"."
        })
    }
}

/// How to recognise a physical device on any PC. Every field given must match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceMatch {
    /// USB vendor ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vid: Option<UsbId>,
    /// USB product ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<UsbId>,
    /// Case-insensitive part of the product name, for example "T.16000M".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl DeviceMatch {
    pub fn is_empty(&self) -> bool {
        self.vid.is_none() && self.pid.is_none() && self.name.as_deref().is_none_or(str::is_empty)
    }

    pub fn matches(&self, d: &DeviceInfo) -> bool {
        if self.is_empty() {
            return false;
        }
        if let Some(v) = self.vid {
            if d.vid != Some(v.0) {
                return false;
            }
        }
        if let Some(p) = self.pid {
            if d.pid != Some(p.0) {
                return false;
            }
        }
        if let Some(n) = &self.name {
            if !d.name.to_lowercase().contains(&n.to_lowercase()) {
                return false;
            }
        }
        true
    }
}

/// A physical device the profile needs, named by its role ("left", "throttle").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceRole {
    /// Shown in the UI, for example "Left hand stick".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Devices that can fill this role; any one entry matching is enough.
    #[serde(rename = "match", default)]
    pub matches: Vec<DeviceMatch>,
    /// When true the profile can run without this device (its controls read 0).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// A virtual device the profile creates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputDef {
    #[serde(rename = "type")]
    pub kind: OutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// vJoy device number (1-16). Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vjoy_id: Option<u8>,
    /// For `integration` outputs: the installed integration's id, for example "space-engineers-2".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<String>,
    /// For `integration` outputs: settings sent to the game-side add-on,
    /// overriding the integration's defaults.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub settings: IndexMap<String, serde_json::Value>,
}

/// A named intermediate value: an expression string, or an object with a label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum VarDef {
    Expr(String),
    Labeled {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

impl VarDef {
    pub fn expr(&self) -> &str {
        match self {
            VarDef::Expr(e) => e,
            VarDef::Labeled { expr, .. } => expr,
        }
    }
    pub fn label(&self) -> Option<&str> {
        match self {
            VarDef::Expr(_) => None,
            VarDef::Labeled { label, .. } => label.as_deref(),
        }
    }
}

/// A complete VBind mapping profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "VBind profile")]
pub struct Profile {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Profile format version. Always 1 for now.
    pub vbind: u32,
    /// Lowercase id used for the file name and commands, for example "galactic-racer-dual-hosas".
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Physical devices by role name. Expressions read them as `role.control`.
    pub devices: IndexMap<String, DeviceRole>,
    /// Virtual devices by name. Map keys write them as `output.control`.
    pub outputs: IndexMap<String, OutputDef>,
    /// Named intermediate values, usable in any expression by name.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub vars: IndexMap<String, VarDef>,
    /// Output control (`pad.rt`) to expression.
    pub map: IndexMap<String, String>,
    /// What each output does in the game, for example `"pad.a": "Boost"`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub labels: IndexMap<String, String>,
    /// Memory hints shown next to a mapping in the picture view and the trainer,
    /// for example `"pad.a": "Index finger, like a gas pedal"`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub hints: IndexMap<String, String>,
    /// Polling rate while this profile runs (100-1000, default 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polling_hz: Option<u32>,
}

/// One problem found while checking a profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Diagnostic {
    /// Where the problem is, for example `map.pad.rt` or `devices.left`.
    pub path: String,
    pub message: String,
    /// Character offset inside the expression, when the problem is in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<usize>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.pos {
            Some(p) => write!(f, "{}: {} (at character {})", self.path, self.message, p + 1),
            None => write!(f, "{}: {}", self.path, self.message),
        }
    }
}

fn diag(path: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic { path: path.into(), message: message.into(), pos: None }
}

const RESERVED: &[&str] = &["true", "false", "and", "or", "not", "pi"];

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
}

#[derive(Clone, Debug)]
pub struct CompiledVar {
    pub name: String,
    pub label: Option<String>,
    pub expr: String,
    pub node: Node,
    pub inputs: Vec<InputRef>,
    pub vars: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct CompiledOutput {
    pub name: String,
    pub kind: OutputKind,
    pub label: Option<String>,
    pub vjoy_id: u8,
    /// The integration definition, for `integration` outputs.
    pub integration: Option<Arc<Integration>>,
    /// Settings JSON for the integration's settings channel, with mapped
    /// settings at their neutral (0) value.
    pub settings_json: Option<String>,
    /// The profile's fixed setting values, kept for rebuilding the JSON when
    /// a mapped setting changes.
    pub settings: IndexMap<String, serde_json::Value>,
    /// Indices of the integration settings the profile maps, in order.
    pub live_settings: Vec<usize>,
}

/// Map keys address integration settings as `output.settings.<setting>`.
pub const SETTINGS_PREFIX: &str = "settings.";

impl CompiledOutput {
    pub fn parse_control(&self, name: &str) -> Result<OutputControl, String> {
        if let (Some(i), Some(key)) = (&self.integration, name.strip_prefix(SETTINGS_PREFIX)) {
            let Some(idx) = i.setting_index(key) else {
                let names: Vec<&str> = i.settings.keys().map(String::as_str).collect();
                let hint = expr::suggest(key, names.iter().copied()).map(|s| format!(" Did you mean '{s}'?")).unwrap_or_default();
                let list = if names.is_empty() { "it has none".to_string() } else { format!("settings: {}", names.join(", ")) };
                return Err(format!("{} has no setting '{key}'.{hint} ({list})", i.name));
            };
            if i.channels.settings.is_none() {
                return Err(format!("{} has no settings channel, so its settings cannot change while playing", i.name));
            }
            crate::integration::check_setting_mappable(&i.settings[idx])?;
            return Ok(OutputControl::IntegrationSetting(idx as u16));
        }
        match &self.integration {
            Some(i) => i.control_index(name).map(|x| OutputControl::Integration(x as u16)).ok_or_else(|| {
                let names: Vec<&str> = i.controls.keys().map(String::as_str).collect();
                let hint = expr::suggest(name, names.iter().copied()).map(|s| format!(" Did you mean '{s}'?")).unwrap_or_default();
                format!("{} has no control '{name}'.{hint} Controls: {}", i.name, names.join(", "))
            }),
            None => OutputControl::parse(self.kind, name),
        }
    }

    fn integration_control(&self, c: OutputControl) -> Option<(&String, &crate::integration::IntegrationControl)> {
        match (c, &self.integration) {
            (OutputControl::Integration(i), Some(def)) => def.controls.get_index(i as usize),
            _ => None,
        }
    }

    fn integration_setting(&self, c: OutputControl) -> Option<(&String, &crate::integration::SettingDef)> {
        match (c, &self.integration) {
            (OutputControl::IntegrationSetting(i), Some(def)) => def.settings.get_index(i as usize),
            _ => None,
        }
    }

    pub fn control_id(&self, c: OutputControl) -> String {
        if let Some((id, _)) = self.integration_setting(c) {
            return format!("{SETTINGS_PREFIX}{id}");
        }
        self.integration_control(c).map(|(id, _)| id.clone()).unwrap_or_else(|| c.id())
    }

    pub fn control_label(&self, c: OutputControl) -> String {
        if let Some((_, s)) = self.integration_setting(c) {
            return s.label.clone();
        }
        self.integration_control(c).map(|(_, x)| x.label.clone()).unwrap_or_else(|| c.default_label())
    }

    /// A setting with two choices or on/off acts like a button; any other
    /// setting takes the raw value, shown like an axis.
    pub fn control_kind(&self, c: OutputControl) -> ControlKind {
        if let Some((_, s)) = self.integration_setting(c) {
            let two_way = s.choices.len() == 2 || (s.choices.is_empty() && s.default.is_boolean());
            return if two_way { ControlKind::Button } else { ControlKind::Axis };
        }
        self.integration_control(c).map(|(_, x)| x.kind).unwrap_or_else(|| c.kind())
    }

    /// Keyboard outputs: mouse pixels per second at full deflection.
    pub fn mouse_speed(&self) -> f64 {
        self.settings.get("mouse_speed").and_then(|v| v.as_f64()).unwrap_or(crate::keyboard::DEFAULT_MOUSE_SPEED)
    }

    /// Keyboard outputs: wheel notches per second at full deflection.
    pub fn wheel_speed(&self) -> f64 {
        self.settings.get("wheel_speed").and_then(|v| v.as_f64()).unwrap_or(crate::keyboard::DEFAULT_WHEEL_SPEED)
    }

    pub fn neutral_report(&self) -> Report {
        Report::neutral(
            self.kind,
            self.integration.as_ref().map_or(0, |i| i.controls.len()),
            self.integration.as_ref().map_or(0, |i| i.settings.len()),
        )
    }

    /// Settings JSON for the current values of the mapped settings
    /// (`values` is the report's per-setting values).
    pub fn live_settings_json(&self, values: &[f64]) -> Result<Option<String>, String> {
        let Some(def) = &self.integration else { return Ok(None) };
        let live: Vec<(usize, f64)> = self.live_settings.iter().map(|&i| (i, values.get(i).copied().unwrap_or(0.0))).collect();
        def.settings_json_live(&self.settings, &live).map_err(|e| e.into_iter().map(|d| d.message).collect::<Vec<_>>().join("; "))
    }

    /// Default display name: "Virtual Xbox 360", "Space Engineers 2".
    pub fn default_label(&self) -> String {
        match (self.kind, &self.integration) {
            (OutputKind::Xbox360, _) => "Virtual Xbox 360".into(),
            (OutputKind::Vjoy, _) => format!("Virtual stick (vJoy {})", self.vjoy_id),
            (OutputKind::Integration, Some(i)) => i.name.clone(),
            (OutputKind::Integration, None) => "Game integration".into(),
            (OutputKind::Keyboard, _) => "Keyboard and mouse".into(),
        }
    }
}

/// Looks up installed integrations by id while compiling a profile.
pub trait IntegrationLookup {
    fn integration(&self, id: &str) -> Option<Arc<Integration>>;
    /// Ids of every installed integration, for suggestions.
    fn integration_ids(&self) -> Vec<String>;
}

/// No integrations installed.
pub struct NoIntegrations;

impl IntegrationLookup for NoIntegrations {
    fn integration(&self, _: &str) -> Option<Arc<Integration>> {
        None
    }
    fn integration_ids(&self) -> Vec<String> {
        Vec::new()
    }
}

impl IntegrationLookup for Vec<Arc<Integration>> {
    fn integration(&self, id: &str) -> Option<Arc<Integration>> {
        self.iter().find(|i| i.id == id).cloned()
    }
    fn integration_ids(&self) -> Vec<String> {
        self.iter().map(|i| i.id.clone()).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Mapping {
    /// `pad.rt`
    pub key: String,
    pub output: usize,
    pub control: OutputControl,
    pub expr: String,
    pub node: Node,
    pub inputs: Vec<InputRef>,
    pub vars: Vec<usize>,
}

/// A checked profile ready to run.
#[derive(Clone, Debug)]
pub struct CompiledProfile {
    pub id: String,
    pub name: String,
    pub roles: Vec<String>,
    pub optional_roles: Vec<bool>,
    pub outputs: Vec<CompiledOutput>,
    pub vars: Vec<CompiledVar>,
    /// Variable indices in evaluation (dependency) order.
    pub var_order: Vec<usize>,
    pub mappings: Vec<Mapping>,
    pub toggle_slots: usize,
    pub polling_hz: u32,
}

struct ProfileScope<'a> {
    roles: Vec<&'a str>,
    vars: Vec<&'a str>,
}

impl Scope for ProfileScope<'_> {
    fn role(&self, name: &str) -> Option<usize> {
        self.roles.iter().position(|r| *r == name)
    }
    fn var(&self, name: &str) -> Option<usize> {
        self.vars.iter().position(|v| *v == name)
    }
    fn role_names(&self) -> Vec<&str> {
        self.roles.clone()
    }
    fn var_names(&self) -> Vec<&str> {
        self.vars.clone()
    }
}

impl Profile {
    pub fn from_json(text: &str) -> Result<Profile, Diagnostic> {
        serde_json::from_str(text).map_err(|e| Diagnostic {
            path: format!("line {} column {}", e.line(), e.column()),
            message: e.to_string(),
            pos: None,
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("profile serializes")
    }

    /// Check everything and compile with no integrations installed.
    pub fn compile(&self) -> Result<CompiledProfile, Vec<Diagnostic>> {
        self.compile_with(&NoIntegrations)
    }

    /// Check everything and compile, reporting every problem found.
    pub fn compile_with(&self, integrations: &dyn IntegrationLookup) -> Result<CompiledProfile, Vec<Diagnostic>> {
        let mut errors = Vec::new();
        if self.vbind != PROFILE_FORMAT {
            errors.push(diag("vbind", format!("unsupported profile format {}; this VBind reads format {PROFILE_FORMAT}", self.vbind)));
        }
        if !is_valid_id(&self.id) {
            errors.push(diag("id", "use 1-64 lowercase letters, digits and dashes, for example \"my-game-hotas\""));
        }
        if self.name.trim().is_empty() {
            errors.push(diag("name", "give the profile a name"));
        }
        if let Some(hz) = self.polling_hz {
            if !(100..=1000).contains(&hz) {
                errors.push(diag("polling_hz", "use a rate between 100 and 1000"));
            }
        }
        let fn_names: Vec<&str> = expr::FUNCTIONS.iter().map(|f| f.1).collect();
        let check_name = |kind: &str, name: &str, errors: &mut Vec<Diagnostic>, path: String| {
            if !is_ident(name) {
                errors.push(diag(path, format!("{kind} names use lowercase letters, digits and _, starting with a letter")));
            } else if RESERVED.contains(&name) || fn_names.contains(&name) {
                errors.push(diag(path, format!("'{name}' is a reserved word; pick another {kind} name")));
            }
        };

        if self.devices.is_empty() {
            errors.push(diag("devices", "a profile needs at least one device role"));
        }
        for (name, role) in &self.devices {
            let path = format!("devices.{name}");
            check_name("device role", name, &mut errors, path.clone());
            if role.matches.is_empty() {
                errors.push(diag(format!("{path}.match"), "add at least one way to recognise the device, for example {\"vid\": \"044F\", \"pid\": \"B10A\"}"));
            }
            for (i, m) in role.matches.iter().enumerate() {
                if m.is_empty() {
                    errors.push(diag(format!("{path}.match[{i}]"), "give a vid, pid or name"));
                }
            }
        }
        if self.outputs.is_empty() {
            errors.push(diag("outputs", "a profile needs at least one output, for example {\"pad\": {\"type\": \"xbox360\"}}"));
        }
        let mut vjoy_ids = Vec::new();
        let mut resolved: Vec<(Option<Arc<Integration>>, Option<String>)> = Vec::new();
        let mut used_integrations: Vec<&str> = Vec::new();
        let mut keyboard_outputs = 0;
        for (name, out) in &self.outputs {
            let path = format!("outputs.{name}");
            check_name("output", name, &mut errors, path.clone());
            if self.devices.contains_key(name) {
                errors.push(diag(path.clone(), format!("'{name}' is already a device role")));
            }
            match out.kind {
                OutputKind::Vjoy => {
                    let id = out.vjoy_id.unwrap_or(1);
                    if !(1..=16).contains(&id) {
                        errors.push(diag(format!("{path}.vjoy_id"), "vJoy devices are numbered 1 to 16"));
                    } else if vjoy_ids.contains(&id) {
                        errors.push(diag(format!("{path}.vjoy_id"), format!("vJoy device {id} is used twice")));
                    }
                    vjoy_ids.push(id);
                }
                OutputKind::Xbox360 | OutputKind::Integration | OutputKind::Keyboard => {
                    if out.kind == OutputKind::Keyboard {
                        if keyboard_outputs > 0 {
                            errors.push(diag(path.clone(), "there is one keyboard and mouse; use one keyboard output"));
                        }
                        keyboard_outputs += 1;
                    }
                    if out.vjoy_id.is_some() {
                        errors.push(diag(format!("{path}.vjoy_id"), "vjoy_id only applies to vjoy outputs"));
                    }
                }
            }
            let mut entry = (None, None);
            if out.kind == OutputKind::Integration {
                match out.integration.as_deref() {
                    None => errors.push(diag(format!("{path}.integration"), "name the integration, for example \"integration\": \"space-engineers-2\"")),
                    Some(id) => match integrations.integration(id) {
                        None => {
                            let ids = integrations.integration_ids();
                            let hint = expr::suggest(id, ids.iter().map(String::as_str)).map(|s| format!(" Did you mean '{s}'?")).unwrap_or_default();
                            let have = if ids.is_empty() { "none are installed".to_string() } else { format!("installed: {}", ids.join(", ")) };
                            errors.push(diag(format!("{path}.integration"), format!("no integration '{id}' is installed.{hint} ({have})")));
                        }
                        Some(def) => {
                            if used_integrations.contains(&id) {
                                errors.push(diag(format!("{path}.integration"), format!("integration '{id}' is used twice; one output drives it")));
                            }
                            used_integrations.push(id);
                            match def.settings_json(&out.settings) {
                                Ok(json) => entry = (Some(def), json),
                                Err(d) => errors.extend(d.into_iter().map(|e| diag(format!("{path}.{}", e.path), e.message))),
                            }
                        }
                    },
                }
            } else {
                if out.integration.is_some() {
                    errors.push(diag(format!("{path}.integration"), "integration only applies to integration outputs"));
                }
                if out.kind == OutputKind::Keyboard {
                    for (k, v) in &out.settings {
                        match (k.as_str(), v.as_f64()) {
                            ("mouse_speed", Some(n)) if (1.0..=20_000.0).contains(&n) => {}
                            ("wheel_speed", Some(n)) if (0.1..=200.0).contains(&n) => {}
                            ("mouse_speed", _) => errors.push(diag(format!("{path}.settings.{k}"), "mouse_speed is pixels per second at full deflection, 1 to 20000")),
                            ("wheel_speed", _) => errors.push(diag(format!("{path}.settings.{k}"), "wheel_speed is wheel notches per second at full deflection, 0.1 to 200")),
                            _ => errors.push(diag(format!("{path}.settings.{k}"), "keyboard outputs take mouse_speed and wheel_speed")),
                        }
                    }
                } else if !out.settings.is_empty() {
                    errors.push(diag(format!("{path}.settings"), "settings only apply to integration and keyboard outputs"));
                }
            }
            resolved.push(entry);
        }
        for name in self.vars.keys() {
            let path = format!("vars.{name}");
            check_name("variable", name, &mut errors, path.clone());
            if self.devices.contains_key(name) || self.outputs.contains_key(name) {
                errors.push(diag(path, format!("'{name}' is already a device or output name")));
            }
        }

        let scope = ProfileScope {
            roles: self.devices.keys().map(String::as_str).collect(),
            vars: self.vars.keys().map(String::as_str).collect(),
        };
        let mut resolver = Resolver::new(&scope);
        let mut compile_expr = |path: String, src: &str, errors: &mut Vec<Diagnostic>| -> Option<Node> {
            match expr::parse(src).and_then(|ast| resolver.resolve(&ast)) {
                Ok(node) => Some(node),
                Err(e) => {
                    errors.push(Diagnostic { path, message: e.message, pos: Some(e.pos) });
                    None
                }
            }
        };

        let mut vars = Vec::new();
        for (name, def) in &self.vars {
            if let Some(node) = compile_expr(format!("vars.{name}"), def.expr(), &mut errors) {
                let (mut inputs, mut deps) = (Vec::new(), Vec::new());
                node.collect_refs(&mut inputs, &mut deps);
                vars.push(CompiledVar {
                    name: name.clone(),
                    label: def.label().map(str::to_string),
                    expr: def.expr().to_string(),
                    node,
                    inputs,
                    vars: deps,
                });
            } else {
                vars.push(CompiledVar {
                    name: name.clone(),
                    label: None,
                    expr: def.expr().to_string(),
                    node: Node::Const(0.0),
                    inputs: vec![],
                    vars: vec![],
                });
            }
        }

        let mut outputs: Vec<CompiledOutput> = self
            .outputs
            .iter()
            .zip(resolved)
            .map(|((name, o), (integration, settings_json))| CompiledOutput {
                name: name.clone(),
                kind: o.kind,
                label: o.label.clone(),
                vjoy_id: o.vjoy_id.unwrap_or(1),
                integration,
                settings_json,
                settings: o.settings.clone(),
                live_settings: Vec::new(),
            })
            .collect();

        let parse_key = |key: &str| -> Result<(usize, OutputControl), String> {
            let Some((out, control)) = key.split_once('.') else {
                return Err("write output keys as output.control, for example pad.rt".into());
            };
            let Some(idx) = outputs.iter().position(|o| o.name == out) else {
                let names: Vec<&str> = outputs.iter().map(|o| o.name.as_str()).collect();
                let hint = expr::suggest(out, names.iter().copied())
                    .map(|s| format!(" Did you mean '{s}'?"))
                    .unwrap_or_default();
                return Err(format!("no output named '{out}'.{hint}"));
            };
            if outputs[idx].kind == OutputKind::Integration && outputs[idx].integration.is_none() {
                return Err(format!("output '{out}' has no integration installed yet"));
            }
            outputs[idx].parse_control(control).map(|c| (idx, c))
        };

        let mut mappings: Vec<Mapping> = Vec::new();
        if self.map.is_empty() {
            errors.push(diag("map", "map at least one output control"));
        }
        for (key, src) in &self.map {
            let path = format!("map.{key}");
            let target = match parse_key(key) {
                Ok(t) => t,
                Err(m) => {
                    errors.push(diag(path, m));
                    continue;
                }
            };
            if let Some(prev) = mappings.iter().find(|m| m.output == target.0 && m.control == target.1) {
                errors.push(diag(path.clone(), format!("same output as '{}'; combine them in one expression, for example with max()", prev.key)));
                continue;
            }
            if let Some(node) = compile_expr(path, src, &mut errors) {
                let (mut inputs, mut deps) = (Vec::new(), Vec::new());
                node.collect_refs(&mut inputs, &mut deps);
                mappings.push(Mapping {
                    key: key.clone(),
                    output: target.0,
                    control: target.1,
                    expr: src.clone(),
                    node,
                    inputs,
                    vars: deps,
                });
            }
        }
        for key in self.labels.keys() {
            if let Err(m) = parse_key(key) {
                errors.push(diag(format!("labels.{key}"), m));
            }
        }
        for (key, hint) in &self.hints {
            if let Err(m) = parse_key(key) {
                errors.push(diag(format!("hints.{key}"), m));
            } else if hint.chars().count() > 160 {
                errors.push(diag(format!("hints.{key}"), "keep hints under 160 characters"));
            }
        }

        // Settings the map drives: not also fixed under the output's
        // settings, and the channel must hold their longest values.
        for (oi, o) in outputs.iter_mut().enumerate() {
            let Some(def) = o.integration.clone() else { continue };
            let mut live: Vec<(usize, &str)> = mappings
                .iter()
                .filter(|m| m.output == oi)
                .filter_map(|m| match m.control {
                    OutputControl::IntegrationSetting(i) => Some((i as usize, m.key.as_str())),
                    _ => None,
                })
                .collect();
            if live.is_empty() {
                continue;
            }
            live.sort_by_key(|l| l.0);
            for &(i, key) in &live {
                let id = &def.settings.get_index(i).expect("parsed setting").0;
                if o.settings.contains_key(*id) {
                    errors.push(diag(format!("map.{key}"), format!("outputs.{}.settings.{id} also sets it; keep one", o.name)));
                }
            }
            let at = |pick: &dyn Fn(usize) -> f64| live.iter().map(|&(i, _)| (i, pick(i))).collect::<Vec<_>>();
            if let Err(d) = def.settings_json_live(&o.settings, &at(&|i| crate::integration::longest_setting_input(&def.settings[i]))) {
                errors.extend(d.into_iter().map(|e| diag(format!("outputs.{}.{}", o.name, e.path), e.message)));
            }
            o.live_settings = live.iter().map(|l| l.0).collect();
            if let Ok(json) = def.settings_json_live(&o.settings, &at(&|_| 0.0)) {
                o.settings_json = json;
            }
        }
        let toggle_slots = resolver.toggle_slots;

        // Variables must not depend on themselves, directly or indirectly.
        let var_order = match topo_order(&vars) {
            Ok(order) => order,
            Err(cycle) => {
                errors.push(diag(format!("vars.{}", vars[cycle[0]].name), format!(
                    "variables depend on each other in a loop: {}",
                    cycle.iter().chain(std::iter::once(&cycle[0])).map(|i| vars[*i].name.as_str()).collect::<Vec<_>>().join(" -> ")
                )));
                Vec::new()
            }
        };

        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(CompiledProfile {
            id: self.id.clone(),
            name: self.name.clone(),
            roles: self.devices.keys().cloned().collect(),
            optional_roles: self.devices.values().map(|r| r.optional).collect(),
            outputs,
            vars,
            var_order,
            mappings,
            toggle_slots,
            polling_hz: self.polling_hz.unwrap_or(500),
        })
    }

    /// Label for an output key: the game action if given.
    pub fn label_for(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }

    /// Memory hint for an output key.
    pub fn hint_for(&self, key: &str) -> Option<&str> {
        self.hints.get(key).map(String::as_str)
    }
}

fn topo_order(vars: &[CompiledVar]) -> Result<Vec<usize>, Vec<usize>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        None,
        Active,
        Done,
    }
    fn visit(i: usize, vars: &[CompiledVar], marks: &mut [Mark], stack: &mut Vec<usize>, out: &mut Vec<usize>) -> Result<(), Vec<usize>> {
        match marks[i] {
            Mark::Done => return Ok(()),
            Mark::Active => {
                let start = stack.iter().position(|s| *s == i).unwrap_or(0);
                return Err(stack[start..].to_vec());
            }
            Mark::None => {}
        }
        marks[i] = Mark::Active;
        stack.push(i);
        for &d in &vars[i].vars {
            visit(d, vars, marks, stack, out)?;
        }
        stack.pop();
        marks[i] = Mark::Done;
        out.push(i);
        Ok(())
    }
    let mut marks = vec![Mark::None; vars.len()];
    let mut out = Vec::with_capacity(vars.len());
    for i in 0..vars.len() {
        visit(i, vars, &mut marks, &mut Vec::new(), &mut out)?;
    }
    Ok(out)
}

/// Mutable state for running one compiled profile.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub eval: EvalState,
    /// Latest variable values, by variable index.
    pub vars: Vec<f64>,
    /// Latest mapped values, by mapping index.
    pub values: Vec<f64>,
    /// Latest report per output.
    pub reports: Vec<Report>,
}

impl CompiledProfile {
    pub fn runtime(&self) -> Runtime {
        Runtime {
            eval: EvalState::new(self.toggle_slots),
            vars: vec![0.0; self.vars.len()],
            values: vec![0.0; self.mappings.len()],
            reports: self.outputs.iter().map(CompiledOutput::neutral_report).collect(),
        }
    }

    /// Run one tick. `devices[i]` is the state of role `i`, `None` if absent.
    pub fn evaluate(&self, devices: &[Option<&DeviceState>], rt: &mut Runtime) {
        for &i in &self.var_order {
            let v = {
                let env = Env { devices, vars: &rt.vars };
                expr::eval(&self.vars[i].node, &env, &mut rt.eval)
            };
            rt.vars[i] = v;
        }
        for r in rt.reports.iter_mut() {
            r.reset();
        }
        for (i, m) in self.mappings.iter().enumerate() {
            let env = Env { devices, vars: &rt.vars };
            let v = expr::eval(&m.node, &env, &mut rt.eval);
            rt.values[i] = v;
            rt.reports[m.output].set(m.control, v);
        }
    }

    /// Every input this profile reads, as (role, control) pairs.
    pub fn used_inputs(&self) -> Vec<InputRef> {
        let mut all: Vec<InputRef> = Vec::new();
        for r in self.vars.iter().flat_map(|v| &v.inputs).chain(self.mappings.iter().flat_map(|m| &m.inputs)) {
            if !all.contains(r) {
                all.push(*r);
            }
        }
        all.sort();
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Profile {
        Profile::from_json(
            r#"{
              "vbind": 1, "id": "test", "name": "Test",
              "devices": {
                "left": {"match": [{"vid": "044F", "pid": "B10A"}]},
                "right": {"match": [{"vid": "046D", "pid": "C215"}]}
              },
              "outputs": {"pad": {"type": "xbox360"}},
              "vars": {
                "l": "deadzone(-left.y, 0.06)",
                "r": {"expr": "deadzone(-right.y, 0.06)", "label": "Right thrust"},
                "drive": "hosas_drive(l, r)"
              },
              "map": {
                "pad.rt": "max(drive, 0)",
                "pad.lt": "max(-drive, 0)",
                "pad.lx": "clamp(l - r, -1, 1)",
                "pad.a": "left.b1",
                "pad.dpad_up": "left.hat.up"
              },
              "labels": {"pad.a": "Boost"}
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn compiles_and_evaluates() {
        let p = sample();
        let c = p.compile().unwrap();
        assert_eq!(c.var_order.last(), Some(&2));
        let mut rt = c.runtime();
        let mut left = DeviceState::default();
        let mut right = DeviceState::default();
        left.axes[1] = -1.0; // pushed fully forward
        right.axes[1] = -0.53; // ~half forward after the 6% deadzone
        left.buttons[0] = true;
        c.evaluate(&[Some(&left), Some(&right)], &mut rt);
        let Report::Xbox(x) = &rt.reports[0] else { panic!() };
        assert_eq!(x.right_trigger, 255);
        assert_eq!(x.left_trigger, 0);
        assert_eq!(x.thumb_lx, crate::math::to_stick(1.0 - crate::math::deadzone(0.53, 0.06)));
        assert_eq!(x.buttons, 0x1000);
    }

    #[test]
    fn reports_every_problem() {
        let mut p = sample();
        p.id = "Bad Id".into();
        p.map.insert("pad.rtt".into(), "1".into());
        p.map.insert("stick.x".into(), "1".into());
        p.map.insert("pad.b".into(), "lfet.b1".into());
        p.vars.insert("loop_a".into(), VarDef::Expr("loop_b".into()));
        p.vars.insert("loop_b".into(), VarDef::Expr("loop_a".into()));
        let errs = p.compile().unwrap_err();
        let paths: Vec<&str> = errs.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"id"));
        assert!(paths.contains(&"map.pad.rtt"));
        assert!(paths.contains(&"map.stick.x"));
        assert!(paths.contains(&"map.pad.b"));
        assert!(errs.iter().any(|d| d.message.contains("loop")), "{errs:?}");
    }

    #[test]
    fn rejects_unknown_fields_and_bad_ids() {
        assert!(Profile::from_json(r#"{"vbind":1,"id":"x","name":"x","devices":{},"outputs":{},"map":{},"extra":1}"#).is_err());
        assert!(Profile::from_json(r#"{"vbind":1,"id":"x","name":"x","devices":{"a":{"match":[{"vid":"zz"}]}},"outputs":{},"map":{}}"#).is_err());
    }

    #[test]
    fn device_matching() {
        let d = DeviceInfo {
            instance: crate::state::Guid(1),
            vid: Some(0x044F),
            pid: Some(0xB10A),
            name: "T.16000M Joystick".into(),
            caps: Default::default(),
        };
        let m = |vid: Option<u16>, pid: Option<u16>, name: Option<&str>| DeviceMatch {
            vid: vid.map(UsbId),
            pid: pid.map(UsbId),
            name: name.map(str::to_string),
        };
        assert!(m(Some(0x044F), Some(0xB10A), None).matches(&d));
        assert!(m(None, None, Some("t.16000m")).matches(&d));
        assert!(!m(Some(0x046D), None, None).matches(&d));
        assert!(!m(None, None, None).matches(&d));
    }

    #[test]
    fn round_trips_json() {
        let p = sample();
        let again = Profile::from_json(&p.to_json()).unwrap();
        assert_eq!(p, again);
    }
}

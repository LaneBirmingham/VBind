//! Game integrations: optional, per-game add-ons that let a profile drive a
//! game directly instead of through a virtual controller.
//!
//! An integration is a data file plus, usually, a game-side add-on written by
//! someone else (a plugin, mod or adapter that runs inside the game). VBind
//! ships no integrations. It knows how to talk to them: an integration file
//! describes a fixed-layout frame in named shared memory, the controls in
//! that frame, optional JSON channels for status, telemetry and settings, and
//! the steps to install the game-side files and launch the game.
//!
//! ```json
//! {
//!   "vbind_integration": 1,
//!   "id": "my-game",
//!   "name": "My Game",
//!   "frame": {
//!     "shared_memory": "Local\\MyGame_Input",
//!     "size": 16,
//!     "constants": [{ "at": 0, "type": "u32", "value": 1 }]
//!   },
//!   "controls": {
//!     "pitch": { "label": "Pitch", "kind": "axis", "at": 4, "type": "f32" },
//!     "fire":  { "label": "Fire", "kind": "button", "at": 8, "type": "u64", "bit": 0 }
//!   }
//! }
//! ```

use crate::output::ControlKind;
use crate::profile::{Diagnostic, is_valid_id};
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const INTEGRATION_FORMAT: u32 = 1;
/// Largest frame or channel VBind will map.
pub const MAX_SHARED_BYTES: u32 = 64 * 1024;
pub const MAX_CONTROLS: usize = 256;
/// Template for the game's install folder in install and launch steps.
pub const GAME_DIR: &str = "{game_dir}";

fn diag(path: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic { path: path.into(), message: message.into(), pos: None }
}

/// The game an integration is for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GameInfo {
    pub name: String,
    /// Steam app id. VBind finds the install folder (`{game_dir}`) and the
    /// installed build from Steam's library files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<u32>,
    /// Game process name, for example "SpaceEngineers2.exe".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    /// Steam build ids the game-side add-on is known to work with. VBind warns
    /// when the installed build is not listed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tested_builds: Vec<String>,
}

/// Little-endian field types in a shared-memory frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
}

impl FieldType {
    pub fn width(self) -> u32 {
        match self {
            FieldType::U8 | FieldType::I8 => 1,
            FieldType::U16 | FieldType::I16 => 2,
            FieldType::U32 | FieldType::I32 | FieldType::F32 => 4,
            FieldType::U64 | FieldType::I64 | FieldType::F64 => 8,
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, FieldType::F32 | FieldType::F64)
    }

    fn int_range(self) -> (f64, f64) {
        match self {
            FieldType::U8 => (0.0, u8::MAX as f64),
            FieldType::U16 => (0.0, u16::MAX as f64),
            FieldType::U32 => (0.0, u32::MAX as f64),
            FieldType::U64 => (0.0, u64::MAX as f64),
            FieldType::I8 => (i8::MIN as f64, i8::MAX as f64),
            FieldType::I16 => (i16::MIN as f64, i16::MAX as f64),
            FieldType::I32 => (i32::MIN as f64, i32::MAX as f64),
            FieldType::I64 => (i64::MIN as f64, i64::MAX as f64),
            FieldType::F32 => (f32::MIN as f64, f32::MAX as f64),
            FieldType::F64 => (f64::MIN, f64::MAX),
        }
    }

    /// Write `v` at `at`, rounding and saturating for integer types.
    pub fn write(self, buf: &mut [u8], at: usize, v: f64) {
        let v = if v.is_finite() { v } else { 0.0 };
        let (lo, hi) = self.int_range();
        let r = v.round().clamp(lo, hi);
        let w = self.width() as usize;
        let bytes: [u8; 8] = match self {
            FieldType::U8 => pad(&(r as u8).to_le_bytes()),
            FieldType::U16 => pad(&(r as u16).to_le_bytes()),
            FieldType::U32 => pad(&(r as u32).to_le_bytes()),
            FieldType::U64 => pad(&(r as u64).to_le_bytes()),
            FieldType::I8 => pad(&(r as i8).to_le_bytes()),
            FieldType::I16 => pad(&(r as i16).to_le_bytes()),
            FieldType::I32 => pad(&(r as i32).to_le_bytes()),
            FieldType::I64 => pad(&(r as i64).to_le_bytes()),
            FieldType::F32 => pad(&(v.clamp(lo, hi) as f32).to_le_bytes()),
            FieldType::F64 => pad(&v.to_le_bytes()),
        };
        buf[at..at + w].copy_from_slice(&bytes[..w]);
    }
}

fn pad(b: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..b.len()].copy_from_slice(b);
    out
}

/// A value written into the frame whenever a profile drives it, such as a
/// version number or an "input enabled" flag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameConstant {
    /// Byte offset in the frame.
    pub at: u32,
    #[serde(rename = "type")]
    pub ty: FieldType,
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The block of named shared memory the game-side add-on reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameDef {
    /// Name of the shared memory, for example "Local\\Kontrol_Input_space-engineers-2".
    pub shared_memory: String,
    /// Frame size in bytes. Every byte is zero while no profile drives it.
    pub size: u32,
    /// Fixed values written while a profile runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<FrameConstant>,
}

/// One control the game-side add-on reads from the frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationControl {
    /// What it does in the game, for example "Pitch" or "Toggle dampeners".
    pub label: String,
    /// Longer explanation for people and agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Heading the window groups controls under, for example "Flight".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// `axis` (-1..1), `trigger` (0..1) or `button` (on/off).
    pub kind: ControlKind,
    /// Byte offset in the frame.
    pub at: u32,
    #[serde(rename = "type")]
    pub ty: FieldType,
    /// For buttons packed into an integer field: the bit to set (0 = lowest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit: Option<u8>,
    /// For integer fields: the stored values for -1 and 1 (axis), 0 and 1
    /// (trigger) or off and on (button). Floats store the value itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Buttons: keep the control on for at least this long after a press, so
    /// a quick tap is not missed by a game that reads less often than VBind writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse_ms: Option<u32>,
}

/// A small JSON document in named shared memory, UTF-8, ending at the first zero byte.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JsonChannel {
    pub shared_memory: String,
    pub size: u32,
}

/// Where the game-side add-on reports whether it is loaded and working.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusChannel {
    pub shared_memory: String,
    pub size: u32,
    /// JSON pointer to the state text, for example "/state".
    pub state: String,
    /// States that mean the add-on is working.
    pub ok_states: Vec<String>,
    /// JSON pointer to a Unix time in milliseconds the add-on refreshes. A
    /// status older than `stale_after_ms` means the game is no longer running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_ms: Option<String>,
    #[serde(default = "default_stale_ms")]
    pub stale_after_ms: u64,
    /// JSON pointers to text worth showing when something is wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
}

fn default_stale_ms() -> u64 {
    5000
}

/// A telemetry value to show, read from the telemetry channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Readout {
    pub label: String,
    /// JSON pointer into the telemetry document, for example "/Values/Linear Speed".
    pub pointer: String,
}

/// A setting the game-side add-on reads from the settings channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingDef {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Value used when a profile does not set one.
    pub default: Value,
    /// Allowed values, when only some are valid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Channels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<JsonChannel>,
    /// VBind writes the profile's settings here as one JSON object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<JsonChannel>,
}

/// One step that puts the game-side files in place. Every destination must
/// be inside `{game_dir}`; uninstalling undoes each step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallStep {
    /// Copy a file from the integration's `files` folder into a folder.
    Copy {
        file: String,
        /// Destination folder, for example "{game_dir}\\Game2".
        to: String,
    },
    /// Write a small text file. Any file already there is kept aside and put
    /// back on uninstall.
    WriteText { path: String, text: String },
}

/// How to start the game with the add-on loaded.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    /// Extra launch arguments; `{game_dir}` is replaced. With a Steam app id
    /// the game starts through Steam (`steam -applaunch <id> <args>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// Where the integration and its game-side add-on come from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A game integration file (`integration.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "VBind game integration")]
pub struct Integration {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Integration format version. Always 1 for now.
    pub vbind_integration: u32,
    /// Lowercase id, also the folder name and the profile's `integration` value.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<GameInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    pub frame: FrameDef,
    /// Controls by id. Profiles map them as `output.id`.
    pub controls: IndexMap<String, IntegrationControl>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub channels: Channels,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub settings: IndexMap<String, SettingDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readouts: Vec<Readout>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub install: Vec<InstallStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<Launch>,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

fn is_control_id(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && s.len() <= 48
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_shm_name(path: &str, name: &str, errors: &mut Vec<Diagnostic>) {
    let bare = name.strip_prefix("Local\\").or_else(|| name.strip_prefix("Global\\")).unwrap_or(name);
    if bare.is_empty() || bare.len() > 200 || bare.contains('\\') || bare.chars().any(char::is_control) {
        errors.push(diag(path, "name the shared memory like \"Local\\\\Vendor_Input_game\" (one optional Local\\ or Global\\ prefix, no other backslashes)"));
    }
}

fn check_size(path: &str, size: u32, errors: &mut Vec<Diagnostic>) {
    if size == 0 || size > MAX_SHARED_BYTES {
        errors.push(diag(path, format!("size must be 1 to {MAX_SHARED_BYTES} bytes")));
    }
}

/// A destination template: must start with `{game_dir}` and never climb out of it.
fn check_game_path(path: &str, value: &str, errors: &mut Vec<Diagnostic>) {
    let Some(rest) = value.strip_prefix(GAME_DIR) else {
        errors.push(diag(path, format!("must start with {GAME_DIR}; integrations only change files inside the game's folder")));
        return;
    };
    if rest.split(['\\', '/']).any(|p| p == ".." || p.contains(':')) || rest.contains('{') {
        errors.push(diag(path, "use a plain path inside the game folder, without '..'"));
    }
}

fn check_pointer(path: &str, p: &str, errors: &mut Vec<Diagnostic>) {
    if !p.is_empty() && !p.starts_with('/') {
        errors.push(diag(path, "write a JSON pointer, for example \"/Values/Speed\""));
    }
}

impl Integration {
    pub fn from_json(text: &str) -> Result<Integration, Diagnostic> {
        serde_json::from_str(text).map_err(|e| Diagnostic {
            path: format!("line {} column {}", e.line(), e.column()),
            message: e.to_string(),
            pos: None,
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("integration serializes")
    }

    pub fn steam_app_id(&self) -> Option<u32> {
        self.game.as_ref().and_then(|g| g.steam_app_id)
    }

    pub fn control_index(&self, id: &str) -> Option<usize> {
        self.controls.get_index_of(id)
    }

    /// Files the install steps copy, in order.
    pub fn files(&self) -> Vec<&str> {
        self.install
            .iter()
            .filter_map(|s| match s {
                InstallStep::Copy { file, .. } => Some(file.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Check everything, reporting every problem found.
    pub fn validate(&self) -> Result<(), Vec<Diagnostic>> {
        let mut errors = Vec::new();
        if self.vbind_integration != INTEGRATION_FORMAT {
            errors.push(diag("vbind_integration", format!("unsupported integration format {}; this VBind reads format {INTEGRATION_FORMAT}", self.vbind_integration)));
        }
        if !is_valid_id(&self.id) {
            errors.push(diag("id", "use 1-64 lowercase letters, digits and dashes, for example \"space-engineers-2\""));
        }
        if self.name.trim().is_empty() {
            errors.push(diag("name", "give the integration a name"));
        }
        check_shm_name("frame.shared_memory", &self.frame.shared_memory, &mut errors);
        check_size("frame.size", self.frame.size, &mut errors);
        let size = self.frame.size;

        // Every byte (or bit) of the frame has one owner.
        let mut owners: Vec<(u32, u32, Option<u8>, String)> = Vec::new();
        let mut claim = |path: String, at: u32, ty: FieldType, bit: Option<u8>, errors: &mut Vec<Diagnostic>| {
            let end = at as u64 + ty.width() as u64;
            if end > size as u64 {
                errors.push(diag(path, format!("bytes {at}..{end} run past the {size}-byte frame")));
                return;
            }
            for (a, b, obit, owner) in &owners {
                let overlaps = at < *b && *a < end as u32;
                let same_field_bits = *a == at && *b == end as u32 && obit.is_some() && bit.is_some();
                if overlaps && !(same_field_bits && *obit != bit) {
                    errors.push(diag(path.clone(), format!("overlaps {owner}")));
                    return;
                }
            }
            owners.push((at, end as u32, bit, path));
        };
        for (i, c) in self.frame.constants.iter().enumerate() {
            claim(format!("frame.constants[{i}]"), c.at, c.ty, None, &mut errors);
        }
        if self.controls.is_empty() {
            errors.push(diag("controls", "add at least one control"));
        }
        if self.controls.len() > MAX_CONTROLS {
            errors.push(diag("controls", format!("at most {MAX_CONTROLS} controls")));
        }
        for (id, c) in &self.controls {
            let path = format!("controls.{id}");
            if !is_control_id(id) {
                errors.push(diag(path.clone(), "control ids use lowercase letters, digits and _, starting with a letter"));
            }
            if c.label.trim().is_empty() {
                errors.push(diag(format!("{path}.label"), "give the control a label"));
            }
            match c.kind {
                ControlKind::HatDir => errors.push(diag(format!("{path}.kind"), "use axis, trigger or button")),
                ControlKind::Axis | ControlKind::Trigger => {
                    if c.bit.is_some() {
                        errors.push(diag(format!("{path}.bit"), "only buttons can be a single bit"));
                    }
                    if c.pulse_ms.is_some() {
                        errors.push(diag(format!("{path}.pulse_ms"), "only buttons pulse"));
                    }
                    if !c.ty.is_float() && c.range.is_none() {
                        errors.push(diag(format!("{path}.range"), "integer fields need a range, for example [-32768, 32767]"));
                    }
                }
                ControlKind::Button => {
                    if let Some(b) = c.bit {
                        if c.ty.is_float() {
                            errors.push(diag(format!("{path}.bit"), "bits need an integer type such as u64"));
                        } else if b as u32 >= c.ty.width() * 8 {
                            errors.push(diag(format!("{path}.bit"), format!("a {} field has bits 0 to {}", serde_json::to_value(c.ty).unwrap().as_str().unwrap_or(""), c.ty.width() * 8 - 1)));
                        }
                        if c.range.is_some() {
                            errors.push(diag(format!("{path}.range"), "a bit is either set or clear; drop the range"));
                        }
                    }
                    if c.pulse_ms.is_some_and(|p| p > 5000) {
                        errors.push(diag(format!("{path}.pulse_ms"), "keep pulses under 5000 ms"));
                    }
                }
            }
            claim(path, c.at, c.ty, c.bit, &mut errors);
        }

        if let Some(s) = &self.channels.status {
            check_shm_name("channels.status.shared_memory", &s.shared_memory, &mut errors);
            check_size("channels.status.size", s.size, &mut errors);
            check_pointer("channels.status.state", &s.state, &mut errors);
            if let Some(p) = &s.updated_ms {
                check_pointer("channels.status.updated_ms", p, &mut errors);
            }
            for (i, p) in s.messages.iter().enumerate() {
                check_pointer(&format!("channels.status.messages[{i}]"), p, &mut errors);
            }
        }
        for (name, ch) in [("telemetry", &self.channels.telemetry), ("settings", &self.channels.settings)] {
            if let Some(ch) = ch {
                check_shm_name(&format!("channels.{name}.shared_memory"), &ch.shared_memory, &mut errors);
                check_size(&format!("channels.{name}.size"), ch.size, &mut errors);
            }
        }
        if !self.readouts.is_empty() && self.channels.telemetry.is_none() {
            errors.push(diag("readouts", "readouts need a telemetry channel"));
        }
        for (i, r) in self.readouts.iter().enumerate() {
            check_pointer(&format!("readouts[{i}].pointer"), &r.pointer, &mut errors);
        }
        if !self.settings.is_empty() && self.channels.settings.is_none() {
            errors.push(diag("settings", "settings need a settings channel"));
        }
        for (key, s) in &self.settings {
            if let Err(m) = check_setting_value(s, &s.default) {
                errors.push(diag(format!("settings.{key}.default"), m));
            }
        }

        let needs_game_dir = !self.install.is_empty() || self.launch.as_ref().is_some_and(|l| l.args.iter().any(|a| a.contains(GAME_DIR)));
        if needs_game_dir && self.steam_app_id().is_none() {
            errors.push(diag("game.steam_app_id", "install steps and {game_dir} need the game's Steam app id"));
        }
        if self.launch.is_some() && self.steam_app_id().is_none() {
            errors.push(diag("launch", "launching needs the game's Steam app id"));
        }
        for (i, step) in self.install.iter().enumerate() {
            let path = format!("install[{i}]");
            match step {
                InstallStep::Copy { file, to } => {
                    if file.is_empty() || file.contains(['\\', '/', ':']) || file.starts_with('.') {
                        errors.push(diag(format!("{path}.file"), "name a file in the integration's files folder, for example \"Adapter.dll\""));
                    }
                    check_game_path(&format!("{path}.to"), to, &mut errors);
                }
                InstallStep::WriteText { path: p, text } => {
                    check_game_path(&format!("{path}.path"), p, &mut errors);
                    if p.trim_start_matches(GAME_DIR).trim_matches(['\\', '/']).is_empty() {
                        errors.push(diag(format!("{path}.path"), "name a file, not the game folder"));
                    }
                    if text.len() > 4096 {
                        errors.push(diag(format!("{path}.text"), "keep written files under 4 KB"));
                    }
                }
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }

    pub fn setting_index(&self, id: &str) -> Option<usize> {
        self.settings.get_index_of(id)
    }

    /// The settings JSON for a profile: defaults, overridden by `overrides`.
    pub fn settings_json(&self, overrides: &IndexMap<String, Value>) -> Result<Option<String>, Vec<Diagnostic>> {
        self.settings_json_live(overrides, &[])
    }

    /// The settings JSON with some settings driven by mapped values, as
    /// (setting index, expression value) pairs. Live values win over `overrides`.
    pub fn settings_json_live(&self, overrides: &IndexMap<String, Value>, live: &[(usize, f64)]) -> Result<Option<String>, Vec<Diagnostic>> {
        let mut errors = Vec::new();
        for (k, v) in overrides {
            match self.settings.get(k) {
                None => {
                    let known: Vec<&str> = self.settings.keys().map(String::as_str).collect();
                    let hint = crate::expr::suggest(k, known.iter().copied()).map(|s| format!(" Did you mean '{s}'?")).unwrap_or_default();
                    errors.push(diag(format!("settings.{k}"), format!("{} has no setting '{k}'.{hint}", self.name)));
                }
                Some(def) => {
                    if let Err(m) = check_setting_value(def, v) {
                        errors.push(diag(format!("settings.{k}"), m));
                    }
                }
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let Some(ch) = &self.channels.settings else {
            return Ok(None);
        };
        let mut obj: serde_json::Map<String, Value> =
            self.settings.iter().map(|(k, d)| (k.clone(), overrides.get(k).cloned().unwrap_or_else(|| d.default.clone()))).collect();
        for &(i, v) in live {
            if let Some((k, def)) = self.settings.get_index(i) {
                obj.insert(k.clone(), setting_from_number(def, v));
            }
        }
        let text = serde_json::to_string(&obj).unwrap_or_default();
        if text.len() >= ch.size as usize {
            return Err(vec![diag("settings", format!("the settings come to {} bytes; the channel holds {}", text.len(), ch.size - 1))]);
        }
        Ok(Some(text))
    }
}

/// Why a setting cannot be driven by an expression, if it cannot.
pub fn check_setting_mappable(def: &SettingDef) -> Result<(), String> {
    if def.choices.is_empty() && !def.default.is_number() && !def.default.is_boolean() {
        return Err(format!("{} takes text, so a mapping cannot drive it; set it under the output's settings instead", def.label));
    }
    Ok(())
}

/// The setting value an expression value stands for: a choice picks by
/// index (0 is the first, so `toggle(b)` flips between the first two), an
/// on/off setting turns on at 0.5, and a number is clamped to its range.
pub fn setting_from_number(def: &SettingDef, v: f64) -> Value {
    let v = if v.is_finite() { v } else { 0.0 };
    if !def.choices.is_empty() {
        let i = (v.round().max(0.0) as usize).min(def.choices.len() - 1);
        return def.choices[i].clone();
    }
    if def.default.is_boolean() {
        return Value::Bool(crate::expr::truthy(v));
    }
    let n = v.clamp(def.min.unwrap_or(f64::MIN), def.max.unwrap_or(f64::MAX));
    if def.default.is_i64() || def.default.is_u64() {
        Value::from(n.round() as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::from(0), Value::Number)
    }
}

/// An expression value that gives a mapped setting its longest JSON text,
/// for checking the settings channel is big enough.
pub fn longest_setting_input(def: &SettingDef) -> f64 {
    if !def.choices.is_empty() {
        return def.choices.iter().enumerate().max_by_key(|(_, c)| c.to_string().len()).map_or(0.0, |(i, _)| i as f64);
    }
    if def.default.is_boolean() {
        return 0.0;
    }
    // a long negative fraction; clamping keeps it inside the setting's range
    -1e15 - 0.123_456_789_012_345
}

fn check_setting_value(def: &SettingDef, v: &Value) -> Result<(), String> {
    if !def.choices.is_empty() && !def.choices.contains(v) {
        let list: Vec<String> = def.choices.iter().map(|c| c.to_string()).collect();
        return Err(format!("{} must be one of {}", def.label, list.join(", ")));
    }
    if def.min.is_some() || def.max.is_some() {
        let Some(n) = v.as_f64() else { return Err(format!("{} takes a number", def.label)) };
        if def.min.is_some_and(|m| n < m) || def.max.is_some_and(|m| n > m) {
            return Err(format!("{} runs from {} to {}", def.label, def.min.map_or("any".into(), |m| m.to_string()), def.max.map_or("any".into(), |m| m.to_string())));
        }
    }
    let same_type = std::mem::discriminant(v) == std::mem::discriminant(&def.default)
        || (v.is_number() && def.default.is_number());
    if !same_type {
        return Err(format!("{} takes a value like {}", def.label, def.default));
    }
    Ok(())
}

/// Builds frames from control values. Keeps press times so buttons can pulse.
#[derive(Clone, Debug)]
pub struct FrameEncoder {
    buf: Vec<u8>,
    held_since: Vec<Option<u64>>,
}

impl FrameEncoder {
    pub fn new(def: &Integration) -> FrameEncoder {
        FrameEncoder { buf: vec![0; def.frame.size as usize], held_since: vec![None; def.controls.len()] }
    }

    /// A frame of zeros: what the game sees when no profile drives it.
    pub fn idle(def: &Integration) -> Vec<u8> {
        vec![0; def.frame.size as usize]
    }

    /// Encode `values` (by control index) at time `now_ms`.
    pub fn encode(&mut self, def: &Integration, values: &[f64], now_ms: u64) -> &[u8] {
        self.buf.iter_mut().for_each(|b| *b = 0);
        for c in &def.frame.constants {
            c.ty.write(&mut self.buf, c.at as usize, c.value);
        }
        for (i, c) in def.controls.values().enumerate() {
            let v = values.get(i).copied().unwrap_or(0.0);
            let at = c.at as usize;
            match c.kind {
                ControlKind::Axis | ControlKind::Trigger => {
                    let (lo_in, v) = if c.kind == ControlKind::Axis {
                        (-1.0, crate::math::clamp(v, -1.0, 1.0))
                    } else {
                        (0.0, crate::math::clamp(v, 0.0, 1.0))
                    };
                    let out = match c.range {
                        Some([lo, hi]) => lo + (v - lo_in) / (1.0 - lo_in) * (hi - lo),
                        None => v,
                    };
                    c.ty.write(&mut self.buf, at, out);
                }
                ControlKind::Button | ControlKind::HatDir => {
                    let pressed = crate::expr::truthy(v);
                    let since = &mut self.held_since[i];
                    if pressed && since.is_none() {
                        *since = Some(now_ms);
                    }
                    let pulsing = since.is_some_and(|t| now_ms < t + c.pulse_ms.unwrap_or(0) as u64);
                    if !pressed && !pulsing {
                        *since = None;
                    }
                    let on = pressed || pulsing;
                    match c.bit {
                        Some(b) => {
                            if on {
                                let w = c.ty.width() as usize;
                                let mut word = [0u8; 8];
                                word[..w].copy_from_slice(&self.buf[at..at + w]);
                                let x = u64::from_le_bytes(word) | (1u64 << b);
                                self.buf[at..at + w].copy_from_slice(&x.to_le_bytes()[..w]);
                            }
                        }
                        None => {
                            let [off, on_v] = c.range.unwrap_or([0.0, 1.0]);
                            c.ty.write(&mut self.buf, at, if on { on_v } else { off });
                        }
                    }
                }
            }
        }
        &self.buf
    }
}

/// Read a zero-terminated UTF-8 JSON document from a channel's bytes.
pub fn parse_json_channel(bytes: &[u8]) -> Option<Value> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    serde_json::from_slice(&bytes[..end]).ok()
}

/// Encode a JSON text for a channel: the text then zero bytes.
pub fn json_channel_bytes(text: &str, size: u32) -> Vec<u8> {
    let mut out = vec![0u8; size as usize];
    let n = text.len().min(size as usize - 1);
    out[..n].copy_from_slice(&text.as_bytes()[..n]);
    out
}

/// What the status channel says about the game-side add-on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddonState {
    /// No status: the game is not running, or the add-on did not load.
    NotRunning,
    /// Loaded and working.
    Active,
    /// Loaded but reporting a state other than the working ones.
    Other,
    /// The last status is older than `stale_after_ms`: the game has closed.
    Stale,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AddonStatus {
    pub state: AddonState,
    /// The add-on's own state text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
}

pub fn interpret_status(ch: &StatusChannel, doc: Option<&Value>, now_ms: u64) -> AddonStatus {
    let Some(doc) = doc else {
        return AddonStatus { state: AddonState::NotRunning, reported: None, messages: Vec::new(), age_ms: None };
    };
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let reported = doc.pointer(&ch.state).map(text);
    let age_ms = ch.updated_ms.as_ref().and_then(|p| doc.pointer(p)).and_then(Value::as_u64).map(|t| now_ms.saturating_sub(t));
    let messages = ch.messages.iter().filter_map(|p| doc.pointer(p)).map(text).filter(|s| !s.is_empty()).collect();
    let state = if age_ms.is_some_and(|a| a > ch.stale_after_ms) {
        AddonState::Stale
    } else if reported.as_ref().is_some_and(|r| ch.ok_states.iter().any(|o| o.eq_ignore_ascii_case(r))) {
        AddonState::Active
    } else {
        AddonState::Other
    };
    AddonStatus { state, reported, messages, age_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> Integration {
        Integration::from_json(
            r#"{
              "vbind_integration": 1, "id": "test-game", "name": "Test Game",
              "game": { "name": "Test Game", "steam_app_id": 42 },
              "frame": {
                "shared_memory": "Local\\Test_Input",
                "size": 149,
                "constants": [{ "at": 0, "type": "u32", "value": 9 }, { "at": 4, "type": "u8", "value": 1 }]
              },
              "controls": {
                "pitch": { "label": "Pitch", "kind": "axis", "at": 5, "type": "f32" },
                "lift":  { "label": "Lift", "kind": "axis", "at": 25, "type": "f32" },
                "fire":  { "label": "Fire", "kind": "button", "at": 133, "type": "u64", "bit": 11 },
                "damp":  { "label": "Dampeners", "kind": "button", "at": 141, "type": "u64", "bit": 6, "pulse_ms": 150 },
                "lights": { "label": "Lights", "kind": "button", "at": 141, "type": "u64", "bit": 7, "pulse_ms": 150 }
              },
              "channels": {
                "status": { "shared_memory": "Local\\Test_Status", "size": 512, "state": "/state", "ok_states": ["Active"],
                            "updated_ms": "/timestampUnixMilliseconds", "messages": ["/errorTitle"] },
                "settings": { "shared_memory": "Local\\Test_Settings", "size": 512 }
              },
              "settings": {
                "mode": { "label": "Flight model", "default": "Direct", "choices": ["Direct", "Reticle"] },
                "gain": { "label": "Gain", "default": 12, "min": 1, "max": 20 }
              },
              "install": [
                { "action": "copy", "file": "Adapter.dll", "to": "{game_dir}\\Bin" },
                { "action": "write_text", "path": "{game_dir}\\Bin\\steam_appid.txt", "text": "42" }
              ],
              "launch": { "args": ["-plugins:{game_dir}\\Bin\\Adapter.dll"] }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn valid_sample() {
        sample().validate().unwrap();
    }

    #[test]
    fn encodes_kontrol_style_frame() {
        let d = sample();
        let mut e = FrameEncoder::new(&d);
        let f = e.encode(&d, &[-0.5, 2.0, 1.0, 1.0, 0.0], 1000).to_vec();
        assert_eq!(f.len(), 149);
        assert_eq!(u32::from_le_bytes(f[0..4].try_into().unwrap()), 9);
        assert_eq!(f[4], 1);
        assert_eq!(f32::from_le_bytes(f[5..9].try_into().unwrap()), -0.5);
        assert_eq!(f32::from_le_bytes(f[25..29].try_into().unwrap()), 1.0, "axes clamp");
        assert_eq!(u64::from_le_bytes(f[133..141].try_into().unwrap()), 1 << 11);
        assert_eq!(u64::from_le_bytes(f[141..149].try_into().unwrap()), 1 << 6);
        // released 10 ms later: the dampener pulse holds until 150 ms
        let f = e.encode(&d, &[0.0, 0.0, 0.0, 0.0, 0.0], 1010).to_vec();
        assert_eq!(u64::from_le_bytes(f[141..149].try_into().unwrap()), 1 << 6);
        assert_eq!(u64::from_le_bytes(f[133..141].try_into().unwrap()), 0, "fire does not pulse");
        let f = e.encode(&d, &[0.0; 5], 1151).to_vec();
        assert_eq!(u64::from_le_bytes(f[141..149].try_into().unwrap()), 0);
        // a new press pulses again
        let f = e.encode(&d, &[0.0, 0.0, 0.0, 1.0, 1.0], 2000).to_vec();
        assert_eq!(u64::from_le_bytes(f[141..149].try_into().unwrap()), (1 << 6) | (1 << 7));
    }

    #[test]
    fn integer_ranges() {
        let mut d = sample();
        d.controls.insert("thr".into(), IntegrationControl {
            label: "Throttle".into(), description: None, group: None, kind: ControlKind::Trigger,
            at: 29, ty: FieldType::U8, bit: None, range: Some([0.0, 255.0]), pulse_ms: None,
        });
        d.validate().unwrap();
        let mut e = FrameEncoder::new(&d);
        let f = e.encode(&d, &[0.0, 0.0, 0.0, 0.0, 0.0, 0.5], 0).to_vec();
        assert_eq!(f[29], 128);
    }

    #[test]
    fn catches_layout_mistakes() {
        let mut d = sample();
        d.controls.get_mut("lift").unwrap().at = 7; // overlaps pitch
        d.controls.get_mut("lights").unwrap().bit = Some(6); // same bit as damp
        d.controls.get_mut("fire").unwrap().at = 145; // runs past the frame
        if let InstallStep::Copy { to, .. } = &mut d.install[0] {
            *to = "C:\\Windows".into();
        }
        let errs = d.validate().unwrap_err();
        let paths: Vec<&str> = errs.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"controls.lift"), "{errs:?}");
        assert!(paths.contains(&"controls.lights"), "{errs:?}");
        assert!(paths.contains(&"controls.fire"), "{errs:?}");
        assert!(paths.contains(&"install[0].to"), "{errs:?}");
    }

    #[test]
    fn settings_merge_and_check() {
        let d = sample();
        let mut o = IndexMap::new();
        o.insert("mode".to_string(), Value::from("Reticle"));
        let t = d.settings_json(&o).unwrap().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&t).unwrap(), serde_json::json!({"mode": "Reticle", "gain": 12}));
        o.insert("gain".to_string(), Value::from(50));
        o.insert("mdoe".to_string(), Value::from("x"));
        let errs = d.settings_json(&o).unwrap_err();
        assert_eq!(errs.len(), 2);
        assert!(errs.iter().any(|e| e.message.contains("Did you mean 'mode'")));
    }

    fn profile_with(map: &str, settings: &str) -> Result<crate::profile::CompiledProfile, Vec<Diagnostic>> {
        let text = format!(
            r#"{{ "vbind": 1, "id": "t", "name": "T",
                 "devices": {{ "s": {{ "match": [{{ "vid": "044F" }}] }} }},
                 "outputs": {{ "g": {{ "type": "integration", "integration": "test-game", "settings": {settings} }} }},
                 "map": {map} }}"#
        );
        crate::profile::Profile::from_json(&text).unwrap().compile_with(&vec![std::sync::Arc::new(sample())])
    }

    #[test]
    fn mapped_settings_change_live() {
        let c = profile_with(r#"{ "g.fire": "s.b1", "g.settings.mode": "toggle(s.b2)", "g.settings.gain": "s.slider1 * 30" }"#, r#"{}"#).unwrap();
        let o = &c.outputs[0];
        assert_eq!(o.live_settings, vec![0, 1]);
        let m = c.mappings.iter().find(|m| m.key == "g.settings.mode").unwrap();
        assert_eq!(o.control_id(m.control), "settings.mode");
        assert_eq!(o.control_label(m.control), "Flight model");
        assert_eq!(o.control_kind(m.control), ControlKind::Button);
        // starts on the first choice; gain clamps to its range
        let json = |v: &[f64]| serde_json::from_str::<Value>(&o.live_settings_json(v).unwrap().unwrap()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(o.settings_json.as_deref().unwrap()).unwrap(), serde_json::json!({"mode": "Direct", "gain": 1}));
        assert_eq!(json(&[1.0, 30.0]), serde_json::json!({"mode": "Reticle", "gain": 20}));
        assert_eq!(json(&[0.4, 7.6]), serde_json::json!({"mode": "Direct", "gain": 8}));
    }

    #[test]
    fn mapped_settings_are_checked() {
        let errs = profile_with(r#"{ "g.settings.mdoe": "s.b1" }"#, "{}").unwrap_err();
        assert!(errs[0].message.contains("Did you mean 'mode'"), "{errs:?}");
        let errs = profile_with(r#"{ "g.settings.mode": "s.b1" }"#, r#"{ "mode": "Reticle" }"#).unwrap_err();
        assert!(errs[0].message.contains("also sets it"), "{errs:?}");
        // unmapped settings keep their fixed value
        let c = profile_with(r#"{ "g.settings.mode": "s.b1" }"#, r#"{ "gain": 5 }"#).unwrap();
        let t = c.outputs[0].live_settings_json(&[1.0, 0.0]).unwrap().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&t).unwrap(), serde_json::json!({"mode": "Reticle", "gain": 5}));
    }

    #[test]
    fn reads_status() {
        let d = sample();
        let ch = d.channels.status.as_ref().unwrap();
        let bytes = json_channel_bytes(r#"{"state":"Active","timestampUnixMilliseconds":1000}"#, 512);
        let doc = parse_json_channel(&bytes);
        assert_eq!(interpret_status(ch, doc.as_ref(), 2000).state, AddonState::Active);
        assert_eq!(interpret_status(ch, doc.as_ref(), 9000).state, AddonState::Stale);
        assert_eq!(interpret_status(ch, None, 0).state, AddonState::NotRunning);
        let doc = serde_json::json!({"state": "Error", "errorTitle": "Incompatible Game Version", "timestampUnixMilliseconds": 1000});
        let s = interpret_status(ch, Some(&doc), 1500);
        assert_eq!(s.state, AddonState::Other);
        assert_eq!(s.messages, vec!["Incompatible Game Version".to_string()]);
    }
}

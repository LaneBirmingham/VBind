//! Snapshot types for physical device input and their identities.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;

pub const MAX_AXES: usize = 8;
pub const MAX_HATS: usize = 4;
pub const MAX_BUTTONS: usize = 128;

/// Axis names in DirectInput data-format order. Profiles refer to axes by
/// these names (`left.y`, `right.rz`, ...).
pub const AXIS_NAMES: [&str; MAX_AXES] = ["x", "y", "z", "rx", "ry", "rz", "slider1", "slider2"];

/// One poll of a physical device.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceState {
    /// Normalized axes in `-1.0..=1.0`. Axes the device lacks read 0.
    pub axes: [f64; MAX_AXES],
    /// Hat angles in hundredths of a degree clockwise from up; `None` when centered.
    pub hats: [Option<u32>; MAX_HATS],
    /// Button states. Index 0 is "button 1" as printed on most sticks and in
    /// the Windows game controller panel.
    pub buttons: [bool; MAX_BUTTONS],
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            axes: [0.0; MAX_AXES],
            hats: [None; MAX_HATS],
            buttons: [false; MAX_BUTTONS],
        }
    }
}

/// What a device has, as reported by the driver.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceCapabilities {
    /// Which of the eight axis slots exist.
    pub axes: [bool; MAX_AXES],
    pub hat_count: u8,
    pub button_count: u8,
}

/// A 128-bit Windows GUID, formatted the usual way
/// (`F32C3250-6366-11F1-8002-444553540000`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub u128);

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.0;
        write!(
            f,
            "{:08X}-{:04X}-{:04X}-{:04X}-{:012X}",
            (v >> 96) as u32,
            (v >> 80) as u16,
            (v >> 64) as u16,
            (v >> 48) as u16,
            v & 0xFFFF_FFFF_FFFF
        )
    }
}

impl Serialize for Guid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Guid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Guid {
    fn schema_name() -> Cow<'static, str> {
        "Guid".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description": "Windows device instance GUID, for example F32C3250-6366-11F1-8002-444553540000"
        })
    }
}

impl std::str::FromStr for Guid {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex: String = s
            .trim()
            .trim_matches(|c| c == '{' || c == '}')
            .chars()
            .filter(|c| *c != '-')
            .collect();
        if hex.len() != 32 {
            return Err(format!("not a GUID: {s}"));
        }
        u128::from_str_radix(&hex, 16)
            .map(Guid)
            .map_err(|_| format!("not a GUID: {s}"))
    }
}

/// Identity of a connected physical device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceInfo {
    /// Per-PC instance GUID. Stable on one machine, meaningless elsewhere.
    pub instance: Guid,
    /// USB vendor ID, when the device is HID.
    pub vid: Option<u16>,
    /// USB product ID, when the device is HID.
    pub pid: Option<u16>,
    /// Product name reported by the driver.
    pub name: String,
    pub caps: DeviceCapabilities,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_round_trips() {
        let s = "F32C3250-6366-11F1-8002-444553540000";
        let g: Guid = s.parse().unwrap();
        assert_eq!(g.to_string(), s);
        let braced: Guid = "{d66b73c0-5d02-11f1-8001-444553540000}".parse().unwrap();
        assert_eq!(braced.to_string(), "D66B73C0-5D02-11F1-8001-444553540000");
    }
}

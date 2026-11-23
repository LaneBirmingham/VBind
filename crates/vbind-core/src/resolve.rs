//! Matching a profile's device roles to the devices plugged into this PC.

use crate::profile::Profile;
use crate::state::{DeviceInfo, Guid};
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoleStatus {
    /// You (or your agent) picked this exact device for the role on this PC.
    Assigned,
    /// Exactly one connected device matches the role.
    Matched,
    /// More than one connected device matches; pick one with an assignment.
    Ambiguous,
    /// No connected device fills the role.
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoleResolution {
    pub role: String,
    pub status: RoleStatus,
    /// The device filling the role, when assigned or matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Guid>,
    /// Devices that match, when ambiguous.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Guid>,
    /// Every connected device that fits the role's match rules, taken or not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matching: Vec<Guid>,
    pub optional: bool,
    /// Set when an assigned device is not plugged in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl RoleResolution {
    pub fn is_ready(&self) -> bool {
        self.device.is_some() || self.optional
    }
}

/// Resolve every role. Explicit assignments win; then roles with exactly one
/// matching device not already taken, repeating until nothing changes.
pub fn resolve_roles(
    profile: &Profile,
    devices: &[DeviceInfo],
    assignments: &IndexMap<String, Guid>,
) -> Vec<RoleResolution> {
    let connected = |g: &Guid| devices.iter().any(|d| d.instance == *g);
    let mut taken: Vec<Guid> = Vec::new();
    let mut out: Vec<RoleResolution> = profile
        .devices
        .iter()
        .map(|(name, role)| RoleResolution {
            role: name.clone(),
            status: RoleStatus::Missing,
            device: None,
            candidates: Vec::new(),
            matching: devices.iter().filter(|d| role.matches.iter().any(|m| m.matches(d))).map(|d| d.instance).collect(),
            optional: role.optional,
            note: None,
        })
        .collect();

    for r in out.iter_mut() {
        if let Some(g) = assignments.get(&r.role) {
            if connected(g) && !taken.contains(g) {
                r.status = RoleStatus::Assigned;
                r.device = Some(*g);
                taken.push(*g);
            } else if !connected(g) {
                r.note = Some(format!("assigned device {g} is not connected"));
            }
        }
    }

    loop {
        let mut progressed = false;
        for (i, (_, role)) in profile.devices.iter().enumerate() {
            if out[i].device.is_some() {
                continue;
            }
            let cands: Vec<Guid> = devices
                .iter()
                .filter(|d| !taken.contains(&d.instance) && role.matches.iter().any(|m| m.matches(d)))
                .map(|d| d.instance)
                .collect();
            match cands.len() {
                0 => {
                    out[i].status = RoleStatus::Missing;
                    out[i].candidates.clear();
                }
                1 => {
                    out[i].status = RoleStatus::Matched;
                    out[i].device = Some(cands[0]);
                    out[i].candidates.clear();
                    taken.push(cands[0]);
                    progressed = true;
                }
                _ => {
                    out[i].status = RoleStatus::Ambiguous;
                    out[i].candidates = cands;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{DeviceMatch, DeviceRole, UsbId};

    fn dev(id: u128, vid: u16, pid: u16) -> DeviceInfo {
        DeviceInfo { instance: Guid(id), vid: Some(vid), pid: Some(pid), name: format!("dev {id}"), caps: Default::default() }
    }

    fn profile(roles: &[(&str, u16, u16)]) -> Profile {
        let mut p = Profile::from_json(r#"{"vbind":1,"id":"t","name":"t","devices":{},"outputs":{"pad":{"type":"xbox360"}},"map":{}}"#).unwrap();
        for (name, vid, pid) in roles {
            p.devices.insert(
                name.to_string(),
                DeviceRole { label: None, matches: vec![DeviceMatch { vid: Some(UsbId(*vid)), pid: Some(UsbId(*pid)), name: None }], optional: false },
            );
        }
        p
    }

    #[test]
    fn unique_matches() {
        let p = profile(&[("left", 1, 1), ("right", 2, 2)]);
        let r = resolve_roles(&p, &[dev(10, 2, 2), dev(11, 1, 1)], &IndexMap::new());
        assert_eq!(r[0].device, Some(Guid(11)));
        assert_eq!(r[1].device, Some(Guid(10)));
        assert!(r.iter().all(|x| x.status == RoleStatus::Matched));
    }

    #[test]
    fn identical_devices_need_one_assignment() {
        let p = profile(&[("left", 1, 1), ("right", 1, 1)]);
        let devs = [dev(10, 1, 1), dev(11, 1, 1)];
        let r = resolve_roles(&p, &devs, &IndexMap::new());
        assert_eq!(r[0].status, RoleStatus::Ambiguous);
        assert_eq!(r[0].candidates.len(), 2);
        let mut a = IndexMap::new();
        a.insert("right".to_string(), Guid(10));
        let r = resolve_roles(&p, &devs, &a);
        assert_eq!(r[1].status, RoleStatus::Assigned);
        assert_eq!(r[0].status, RoleStatus::Matched);
        assert_eq!(r[0].device, Some(Guid(11)));
    }

    #[test]
    fn unplugged_assignment_falls_back_to_matching() {
        let p = profile(&[("left", 1, 1)]);
        let mut a = IndexMap::new();
        a.insert("left".to_string(), Guid(99));
        let r = resolve_roles(&p, &[dev(10, 1, 1)], &a);
        assert_eq!(r[0].status, RoleStatus::Matched);
        assert!(r[0].note.is_some());
        let r = resolve_roles(&p, &[], &a);
        assert_eq!(r[0].status, RoleStatus::Missing);
    }
}

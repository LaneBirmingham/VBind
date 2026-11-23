//! On-disk state: profiles, device layouts, device assignments and settings.
//!
//! Everything lives in `%APPDATA%\VBind`:
//! - `profiles\<id>.json`: shareable profiles
//! - `layouts\<id>.json`: shareable device layout cards
//! - `assignments.json`: which physical device fills which role on this PC
//! - `settings.json`: app settings, including the profile to start with

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use vbind_core::layout::DeviceLayout;
use vbind_core::profile::{Diagnostic, Profile};
use vbind_core::state::Guid;

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VBIND_HOME") {
        return PathBuf::from(dir);
    }
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    Path::new(&base).join("VBind")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Profile to run whenever VBind starts.
    pub active_profile: Option<String>,
    /// Port for the local API. VBind falls back to any free port if taken.
    pub port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { active_profile: None, port: 47821 }
    }
}

/// A profile file as found on disk; `profile` is `None` when it does not parse.
#[derive(Clone, Debug)]
pub struct StoredProfile {
    pub id: String,
    pub path: PathBuf,
    pub profile: Option<Profile>,
    pub load_error: Option<Diagnostic>,
}

pub struct Store {
    dir: PathBuf,
    profiles: RwLock<Vec<StoredProfile>>,
    layouts: RwLock<Vec<DeviceLayout>>,
    assignments: RwLock<IndexMap<String, IndexMap<String, Guid>>>,
    settings: RwLock<Settings>,
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

impl Store {
    pub fn open(dir: PathBuf) -> Store {
        let _ = fs::create_dir_all(dir.join("profiles"));
        let _ = fs::create_dir_all(dir.join("layouts"));
        let store = Store {
            settings: RwLock::new(read_json(&dir.join("settings.json"))),
            assignments: RwLock::new(read_json(&dir.join("assignments.json"))),
            profiles: RwLock::new(Vec::new()),
            layouts: RwLock::new(Vec::new()),
            dir,
        };
        store.reload();
        store
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Re-read profiles and layouts from disk (files may be edited by hand).
    pub fn reload(&self) {
        let mut profiles = Vec::new();
        for path in json_files(&self.dir.join("profiles")) {
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            let text = fs::read_to_string(&path).unwrap_or_default();
            match Profile::from_json(&text) {
                Ok(p) => profiles.push(StoredProfile { id: p.id.clone(), path, profile: Some(p), load_error: None }),
                Err(e) => profiles.push(StoredProfile { id, path, profile: None, load_error: Some(e) }),
            }
        }
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        *self.profiles.write().unwrap() = profiles;

        let mut layouts = Vec::new();
        for path in json_files(&self.dir.join("layouts")) {
            if let Ok(l) = fs::read_to_string(&path).map_err(|_| ()).and_then(|t| serde_json::from_str::<DeviceLayout>(&t).map_err(|_| ())) {
                layouts.push(l);
            }
        }
        layouts.sort_by(|a, b| a.id.cmp(&b.id));
        *self.layouts.write().unwrap() = layouts;
    }

    pub fn profiles(&self) -> Vec<StoredProfile> {
        self.profiles.read().unwrap().clone()
    }

    pub fn profile(&self, id: &str) -> Option<Profile> {
        self.profiles.read().unwrap().iter().find(|p| p.id == id).and_then(|p| p.profile.clone())
    }

    pub fn profile_exists(&self, id: &str) -> bool {
        self.profiles.read().unwrap().iter().any(|p| p.id == id)
    }

    /// Save a profile that has already been validated.
    pub fn save_profile(&self, p: &Profile) -> Result<(), String> {
        let path = self.dir.join("profiles").join(format!("{}.json", p.id));
        write_atomic(&path, &(p.to_json() + "\n"))?;
        let mut list = self.profiles.write().unwrap();
        list.retain(|s| s.id != p.id);
        list.push(StoredProfile { id: p.id.clone(), path, profile: Some(p.clone()), load_error: None });
        list.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }

    pub fn delete_profile(&self, id: &str) -> Result<bool, String> {
        let mut list = self.profiles.write().unwrap();
        let Some(pos) = list.iter().position(|p| p.id == id) else { return Ok(false) };
        let entry = list.remove(pos);
        fs::remove_file(&entry.path).map_err(|e| format!("cannot delete {}: {e}", entry.path.display()))?;
        drop(list);
        let mut a = self.assignments.write().unwrap();
        if a.shift_remove(id).is_some() {
            let text = serde_json::to_string_pretty(&*a).unwrap();
            drop(a);
            write_atomic(&self.dir.join("assignments.json"), &text)?;
        }
        Ok(true)
    }

    pub fn layouts(&self) -> Vec<DeviceLayout> {
        self.layouts.read().unwrap().clone()
    }

    pub fn save_layout(&self, l: &DeviceLayout) -> Result<(), String> {
        let path = self.dir.join("layouts").join(format!("{}.json", l.id));
        write_atomic(&path, &(serde_json::to_string_pretty(l).unwrap() + "\n"))?;
        let mut list = self.layouts.write().unwrap();
        list.retain(|x| x.id != l.id);
        list.push(l.clone());
        list.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }

    pub fn delete_layout(&self, id: &str) -> Result<bool, String> {
        let path = self.dir.join("layouts").join(format!("{id}.json"));
        let mut list = self.layouts.write().unwrap();
        let before = list.len();
        list.retain(|x| x.id != id);
        if list.len() == before {
            return Ok(false);
        }
        fs::remove_file(&path).map_err(|e| format!("cannot delete {}: {e}", path.display()))?;
        Ok(true)
    }

    pub fn assignments(&self, profile: &str) -> IndexMap<String, Guid> {
        self.assignments.read().unwrap().get(profile).cloned().unwrap_or_default()
    }

    /// Set (or with `None`, clear) the device for one role on this PC.
    pub fn set_assignment(&self, profile: &str, role: &str, device: Option<Guid>) -> Result<(), String> {
        let mut all = self.assignments.write().unwrap();
        let entry = all.entry(profile.to_string()).or_default();
        match device {
            Some(g) => {
                entry.insert(role.to_string(), g);
            }
            None => {
                entry.shift_remove(role);
            }
        }
        if entry.is_empty() {
            all.shift_remove(profile);
        }
        let text = serde_json::to_string_pretty(&*all).unwrap();
        drop(all);
        write_atomic(&self.dir.join("assignments.json"), &text)
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let mut s = self.settings.write().unwrap();
        f(&mut s);
        let copy = s.clone();
        drop(s);
        write_atomic(&self.dir.join("settings.json"), &serde_json::to_string_pretty(&copy).unwrap())?;
        Ok(copy)
    }
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    rd.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect()
}

/// Information written for the CLI and MCP server to find a running VBind.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
}

pub fn runtime_file() -> PathBuf {
    data_dir().join("runtime.json")
}

pub fn read_runtime() -> Option<RuntimeInfo> {
    fs::read_to_string(runtime_file()).ok().and_then(|t| serde_json::from_str(&t).ok())
}

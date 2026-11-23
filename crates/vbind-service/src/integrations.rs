//! Installed game integrations: `%APPDATA%\VBind\integrations\<id>\` holds
//! `integration.json`, the game-side files in `files\`, and `installed.json`
//! once those files are put into the game.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use vbind_core::integration::{AddonStatus, GAME_DIR, InstallStep, Integration, interpret_status, parse_json_channel};
use vbind_core::profile::Diagnostic;
use vbind_io::steam::{self, SteamApp};

/// Largest set of files one integration may carry.
pub const MAX_FILES_BYTES: u64 = 256 * 1024 * 1024;
const BACKUP_SUFFIX: &str = ".vbind-backup";

#[derive(Clone, Debug)]
pub struct StoredIntegration {
    pub id: String,
    pub dir: PathBuf,
    pub def: Option<Arc<Integration>>,
    pub errors: Vec<Diagnostic>,
}

/// What VBind changed in the game folder, so uninstalling can undo it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InstallRecord {
    pub game_dir: PathBuf,
    #[serde(default)]
    pub build_id: Option<String>,
    pub installed_ms: u64,
    pub files: Vec<InstalledFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledFile {
    pub path: PathBuf,
    /// Where a file VBind replaced was kept, to put back on uninstall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
}

pub struct Integrations {
    root: PathBuf,
    list: RwLock<Vec<StoredIntegration>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn load_dir(dir: &Path) -> Option<StoredIntegration> {
    let id = dir.file_name()?.to_str()?.to_string();
    let path = dir.join("integration.json");
    let text = fs::read_to_string(&path).ok()?;
    Some(match Integration::from_json(&text) {
        Ok(def) => {
            let mut errors = def.validate().err().unwrap_or_default();
            if def.id != id {
                errors.push(Diagnostic { path: "id".into(), message: format!("the id '{}' does not match its folder '{id}'", def.id), pos: None });
            }
            for f in def.files() {
                if !dir.join("files").join(f).is_file() {
                    errors.push(Diagnostic { path: "install".into(), message: format!("files\\{f} is missing"), pos: None });
                }
            }
            if errors.is_empty() {
                StoredIntegration { id, dir: dir.to_path_buf(), def: Some(Arc::new(def)), errors }
            } else {
                StoredIntegration { id, dir: dir.to_path_buf(), def: None, errors }
            }
        }
        Err(e) => StoredIntegration { id, dir: dir.to_path_buf(), def: None, errors: vec![e] },
    })
}

/// Valid integration definitions in a data folder, without starting anything.
pub fn load_defs(data_dir: &Path) -> Vec<Arc<Integration>> {
    Integrations::open(data_dir).defs()
}

impl Integrations {
    pub fn open(data_dir: &Path) -> Integrations {
        let i = Integrations { root: data_dir.join("integrations"), list: RwLock::new(Vec::new()) };
        i.reload();
        i
    }

    pub fn reload(&self) {
        let mut list: Vec<StoredIntegration> = fs::read_dir(&self.root)
            .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).filter_map(|e| load_dir(&e.path())).collect())
            .unwrap_or_default();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        *self.list.write().unwrap() = list;
    }

    pub fn list(&self) -> Vec<StoredIntegration> {
        self.list.read().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<StoredIntegration> {
        self.list.read().unwrap().iter().find(|i| i.id == id).cloned()
    }

    /// Valid definitions, for compiling profiles.
    pub fn defs(&self) -> Vec<Arc<Integration>> {
        self.list.read().unwrap().iter().filter_map(|i| i.def.clone()).collect()
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// Import from a folder holding `integration.json` (and a `files` folder),
    /// or from an `integration.json` path.
    pub fn import(&self, src: &Path, replace: bool) -> Result<Arc<Integration>, ImportError> {
        let (src_dir, json_path) = if src.is_dir() {
            (src.to_path_buf(), src.join("integration.json"))
        } else {
            (src.parent().map(Path::to_path_buf).unwrap_or_default(), src.to_path_buf())
        };
        let text = fs::read_to_string(&json_path).map_err(|e| ImportError::Bad(format!("cannot read {}: {e}", json_path.display())))?;
        let def = Integration::from_json(&text).map_err(|d| ImportError::Invalid(vec![d]))?;
        def.validate().map_err(ImportError::Invalid)?;
        let files_src = src_dir.join("files");
        let mut missing = Vec::new();
        for f in def.files() {
            if !files_src.join(f).is_file() {
                missing.push(Diagnostic { path: "install".into(), message: format!("{} is missing", files_src.join(f).display()), pos: None });
            }
        }
        if !missing.is_empty() {
            return Err(ImportError::Invalid(missing));
        }
        let dest = self.dir(&def.id);
        if dest.join("integration.json").exists() && !replace {
            return Err(ImportError::Exists(def.id.clone()));
        }
        if dest == src_dir {
            self.reload();
            return Ok(Arc::new(def));
        }
        let mut total = 0u64;
        let mut copies = Vec::new();
        if files_src.is_dir() {
            for e in fs::read_dir(&files_src).map_err(|e| ImportError::Bad(e.to_string()))?.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_file() {
                    total += e.metadata().map(|m| m.len()).unwrap_or(0);
                    copies.push(p);
                }
            }
        }
        if total > MAX_FILES_BYTES {
            return Err(ImportError::Bad(format!("the files come to {} MB; integrations carry at most {} MB", total >> 20, MAX_FILES_BYTES >> 20)));
        }
        let files_dest = dest.join("files");
        if files_dest.is_dir() {
            fs::remove_dir_all(&files_dest).map_err(|e| ImportError::Bad(format!("cannot clear {}: {e}", files_dest.display())))?;
        }
        fs::create_dir_all(&files_dest).map_err(|e| ImportError::Bad(format!("cannot create {}: {e}", files_dest.display())))?;
        for p in copies {
            let to = files_dest.join(p.file_name().unwrap());
            fs::copy(&p, &to).map_err(|e| ImportError::Bad(format!("cannot copy {}: {e}", p.display())))?;
        }
        write_text(&dest.join("integration.json"), &(def.to_json() + "\n")).map_err(ImportError::Bad)?;
        self.reload();
        Ok(Arc::new(def))
    }

    /// Replace just the definition, keeping the files already imported.
    pub fn save_def(&self, def: &Integration) -> Result<(), ImportError> {
        def.validate().map_err(ImportError::Invalid)?;
        let dest = self.dir(&def.id);
        let missing: Vec<Diagnostic> = def
            .files()
            .into_iter()
            .filter(|f| !dest.join("files").join(f).is_file())
            .map(|f| Diagnostic { path: "install".into(), message: format!("files\\{f} has not been imported; import the folder that holds it", ), pos: None })
            .collect();
        if !missing.is_empty() {
            return Err(ImportError::Invalid(missing));
        }
        write_text(&dest.join("integration.json"), &(def.to_json() + "\n")).map_err(ImportError::Bad)?;
        self.reload();
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let dir = self.dir(id);
        if !dir.exists() {
            return Ok(false);
        }
        if self.record(id).is_some() {
            return Err(format!("'{id}' is still installed in the game; uninstall it first"));
        }
        fs::remove_dir_all(&dir).map_err(|e| format!("cannot delete {}: {e}", dir.display()))?;
        self.reload();
        Ok(true)
    }

    pub fn record(&self, id: &str) -> Option<InstallRecord> {
        fs::read_to_string(self.dir(id).join("installed.json")).ok().and_then(|t| serde_json::from_str(&t).ok())
    }

    fn save_record(&self, id: &str, r: Option<&InstallRecord>) -> Result<(), String> {
        let path = self.dir(id).join("installed.json");
        match r {
            Some(r) => write_text(&path, &serde_json::to_string_pretty(r).unwrap()),
            None => match fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("cannot remove {}: {e}", path.display())),
                _ => Ok(()),
            },
        }
    }

    /// Put the game-side files in place.
    pub fn install(&self, id: &str) -> Result<Value, String> {
        let s = self.get(id).ok_or_else(|| format!("no integration '{id}'"))?;
        let def = s.def.clone().ok_or_else(|| format!("'{id}' has problems: {}", describe(&s.errors)))?;
        let app = game(&def)?;
        if app.updating() {
            return Err(format!("Steam is still installing or updating {}; try again once it finishes", def.game.as_ref().map_or(id, |g| g.name.as_str())));
        }
        let mut record = self.record(id).unwrap_or_default();
        record.game_dir = app.dir.clone();
        record.build_id = app.build_id.clone();
        record.installed_ms = now_ms();
        let mut done = Vec::new();
        for step in &def.install {
            let (dest, bytes) = match step {
                InstallStep::Copy { file, to } => {
                    let bytes = fs::read(s.dir.join("files").join(file)).map_err(|e| format!("cannot read files\\{file}: {e}"))?;
                    (expand(to, &app.dir).join(file), bytes)
                }
                InstallStep::WriteText { path, text } => (expand(path, &app.dir), text.as_bytes().to_vec()),
            };
            let known = record.files.iter().position(|f| f.path == dest);
            let mut backup = known.and_then(|i| record.files[i].backup.clone());
            if known.is_none() && dest.is_file() && fs::read(&dest).ok().as_deref() != Some(bytes.as_slice()) {
                // A file VBind did not put there: keep it aside.
                let b = PathBuf::from(format!("{}{BACKUP_SUFFIX}", dest.display()));
                fs::copy(&dest, &b).map_err(|e| format!("cannot back up {}: {e}", dest.display()))?;
                backup = Some(b);
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            clear_readonly(&dest);
            fs::write(&dest, &bytes).map_err(|e| {
                let hint = if e.raw_os_error() == Some(32) { " (close the game first)" } else { "" };
                format!("cannot write {}: {e}{hint}", dest.display())
            })?;
            match known {
                Some(i) => record.files[i].backup = backup,
                None => record.files.push(InstalledFile { path: dest.clone(), backup }),
            }
            done.push(dest);
        }
        self.save_record(id, Some(&record))?;
        Ok(json!({ "installed": id, "game_dir": app.dir, "build_id": app.build_id, "files": done }))
    }

    /// Remove what install put in the game folder and restore replaced files.
    pub fn uninstall(&self, id: &str) -> Result<Value, String> {
        let Some(record) = self.record(id) else {
            return Ok(json!({ "uninstalled": id, "files": [] }));
        };
        let mut removed = Vec::new();
        let mut errors = Vec::new();
        for f in record.files.iter().rev() {
            clear_readonly(&f.path);
            match fs::remove_file(&f.path) {
                Ok(()) => removed.push(f.path.clone()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    let hint = if e.raw_os_error() == Some(32) { " (close the game first)" } else { "" };
                    errors.push(format!("cannot remove {}: {e}{hint}", f.path.display()));
                    continue;
                }
            }
            if let Some(b) = &f.backup
                && let Err(e) = fs::rename(b, &f.path)
            {
                errors.push(format!("cannot restore {}: {e}", f.path.display()));
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        self.save_record(id, None)?;
        Ok(json!({ "uninstalled": id, "files": removed }))
    }

    /// Install state against the game folder as it is now.
    pub fn install_state(&self, s: &StoredIntegration) -> Value {
        let Some(def) = &s.def else { return json!({ "installed": false }) };
        let record = self.record(&s.id);
        let app = def.steam_app_id().and_then(steam::find_app);
        let tested = def.game.as_ref().filter(|g| !g.tested_builds.is_empty()).and_then(|g| {
            app.as_ref().and_then(|a| a.build_id.as_ref()).map(|b| g.tested_builds.iter().any(|t| t == b))
        });
        let mut current = true;
        if let (Some(r), Some(app)) = (&record, &app) {
            for step in &def.install {
                let (dest, want) = match step {
                    InstallStep::Copy { file, to } => (expand(to, &app.dir).join(file), fs::read(s.dir.join("files").join(file)).ok()),
                    InstallStep::WriteText { path, text } => (expand(path, &app.dir), Some(text.as_bytes().to_vec())),
                };
                if !r.files.iter().any(|f| f.path == dest) || fs::read(&dest).ok() != want {
                    current = false;
                }
            }
        }
        json!({
            "installed": record.is_some(),
            "current": record.is_some() && current,
            "installed_build": record.as_ref().and_then(|r| r.build_id.clone()),
            "game": app.as_ref().map(|a| json!({
                "dir": a.dir, "build_id": a.build_id, "updating": a.updating(), "name": a.name,
            })),
            "game_found": app.is_some(),
            "tested_build": tested,
            "needs_install": !def.install.is_empty(),
        })
    }

    pub fn launch(&self, id: &str) -> Result<Value, String> {
        let s = self.get(id).ok_or_else(|| format!("no integration '{id}'"))?;
        let def = s.def.clone().ok_or_else(|| format!("'{id}' has problems: {}", describe(&s.errors)))?;
        let launch = def.launch.clone().ok_or_else(|| format!("{} has no launch settings; start the game as usual", def.name))?;
        let app = game(&def)?;
        if !def.install.is_empty() && self.record(id).is_none() {
            return Err(format!("install {} first", def.name));
        }
        let args: Vec<String> = launch.args.iter().map(|a| a.replace(GAME_DIR, &app.dir.display().to_string())).collect();
        steam::launch(app.app_id, &args)?;
        Ok(json!({ "launched": id, "steam_app_id": app.app_id, "args": args }))
    }

    /// Zero every integration's input frame. Run at start-up so a game never
    /// keeps acting on input a crashed VBind left behind.
    pub fn clear_frames(&self) {
        for d in self.defs() {
            let _ = vbind_io::shm::clear_existing(&d.frame.shared_memory, d.frame.size);
        }
    }
}

/// Live status and telemetry from the game-side add-on.
pub fn live(def: &Integration) -> Value {
    let read = |name: &str, size: u32| vbind_io::shm::read_existing(name, size).ok().flatten().and_then(|b| parse_json_channel(&b));
    let status: Option<AddonStatus> = def.channels.status.as_ref().map(|ch| interpret_status(ch, read(&ch.shared_memory, ch.size).as_ref(), now_ms()));
    let telemetry = def.channels.telemetry.as_ref().and_then(|ch| read(&ch.shared_memory, ch.size));
    let readouts: Vec<Value> = def
        .readouts
        .iter()
        .map(|r| {
            let v = telemetry.as_ref().and_then(|t| t.pointer(&r.pointer)).cloned();
            json!({ "label": r.label, "value": v })
        })
        .collect();
    json!({ "status": status, "readouts": readouts, "telemetry": telemetry })
}

fn game(def: &Integration) -> Result<SteamApp, String> {
    let id = def.steam_app_id().ok_or_else(|| format!("{} does not name a Steam app id", def.name))?;
    let name = def.game.as_ref().map_or(def.name.as_str(), |g| g.name.as_str());
    steam::find_app(id).ok_or_else(|| format!("{name} (Steam app {id}) is not installed through Steam on this PC"))
}

fn expand(template: &str, game_dir: &Path) -> PathBuf {
    let rest = template.trim_start_matches(GAME_DIR).trim_start_matches(['\\', '/']);
    if rest.is_empty() { game_dir.to_path_buf() } else { game_dir.join(rest.replace('/', "\\")) }
}

fn clear_readonly(p: &Path) {
    if let Ok(m) = fs::metadata(p) {
        let mut perm = m.permissions();
        if perm.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = fs::set_permissions(p, perm);
        }
    }
}

fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

fn describe(d: &[Diagnostic]) -> String {
    d.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("; ")
}

#[derive(Debug)]
pub enum ImportError {
    Invalid(Vec<Diagnostic>),
    Exists(String),
    Bad(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_game_paths() {
        let g = Path::new("F:\\Games\\SE2");
        assert_eq!(expand("{game_dir}\\Game2", g), PathBuf::from("F:\\Games\\SE2\\Game2"));
        assert_eq!(expand("{game_dir}/Game2/steam_appid.txt", g), PathBuf::from("F:\\Games\\SE2\\Game2\\steam_appid.txt"));
        assert_eq!(expand("{game_dir}", g), PathBuf::from("F:\\Games\\SE2"));
    }

    #[test]
    fn imports_and_validates_files() {
        let tmp = std::env::temp_dir().join(format!("vbind-int-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let src = tmp.join("src");
        fs::create_dir_all(src.join("files")).unwrap();
        let def = r#"{"vbind_integration":1,"id":"demo","name":"Demo","game":{"name":"Demo","steam_app_id":1},
          "frame":{"shared_memory":"Local\\Demo","size":8},
          "controls":{"a":{"label":"A","kind":"button","at":0,"type":"u8"}},
          "install":[{"action":"copy","file":"Addon.dll","to":"{game_dir}"}]}"#;
        fs::write(src.join("integration.json"), def).unwrap();
        let ints = Integrations::open(&tmp.join("data"));
        assert!(matches!(ints.import(&src, false), Err(ImportError::Invalid(_))), "missing file is caught");
        fs::write(src.join("files").join("Addon.dll"), b"dll").unwrap();
        ints.import(&src, false).unwrap();
        assert!(matches!(ints.import(&src, false), Err(ImportError::Exists(_))));
        assert_eq!(ints.defs().len(), 1);
        assert!(tmp.join("data").join("integrations").join("demo").join("files").join("Addon.dll").is_file());
        assert!(ints.delete("demo").unwrap());
        assert!(ints.defs().is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }
}

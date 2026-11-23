//! Finding Steam games: install folders, installed builds, and launching
//! with extra arguments.

use std::path::{Path, PathBuf};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::HSTRING;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SteamApp {
    pub app_id: u32,
    pub name: Option<String>,
    /// The game's install folder.
    pub dir: PathBuf,
    /// Steam's build id of the installed version.
    pub build_id: Option<String>,
    /// Steam's state flags; 4 means fully installed and up to date.
    pub state_flags: Option<u32>,
}

impl SteamApp {
    /// True while Steam is downloading or applying an update.
    pub fn updating(&self) -> bool {
        self.state_flags.is_some_and(|f| f != 4)
    }
}

fn reg_string(key: &str, value: &str) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

/// Steam's own folder.
pub fn steam_dir() -> Option<PathBuf> {
    reg_string("Software\\Valve\\Steam", "SteamPath").map(|p| PathBuf::from(p.replace('/', "\\"))).filter(|p| p.is_dir())
}

/// `steam.exe`.
pub fn steam_exe() -> Option<PathBuf> {
    reg_string("Software\\Valve\\Steam", "SteamExe")
        .map(|p| PathBuf::from(p.replace('/', "\\")))
        .filter(|p| p.is_file())
        .or_else(|| steam_dir().map(|d| d.join("steam.exe")).filter(|p| p.is_file()))
}

/// Every `"key" "value"` pair in a Valve KeyValues text, in order.
pub fn vdf_pairs(text: &str) -> Vec<(String, String)> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(match n {
                                    'n' => '\n',
                                    't' => '\t',
                                    other => other,
                                });
                            }
                        }
                        '"' => break,
                        other => s.push(other),
                    }
                }
                tokens.push(Some(s));
            }
            '{' | '}' => tokens.push(None),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < tokens.len() {
        match (&tokens[i], &tokens[i + 1]) {
            (Some(k), Some(v)) => {
                out.push((k.clone(), v.clone()));
                i += 2;
            }
            _ => i += 1,
        }
    }
    out
}

fn vdf_get<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
}

/// Library folders from `libraryfolders.vdf`, Steam's own folder first.
pub fn library_folders(steam: &Path) -> Vec<PathBuf> {
    let mut out = vec![steam.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for (k, v) in vdf_pairs(&text) {
            if k.eq_ignore_ascii_case("path") {
                let p = PathBuf::from(v);
                if !out.iter().any(|o| o.as_os_str().eq_ignore_ascii_case(p.as_os_str())) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Parse an `appmanifest_<id>.acf` found in `library`.
pub fn parse_manifest(app_id: u32, library: &Path, text: &str) -> Option<SteamApp> {
    let pairs = vdf_pairs(text);
    let dir = vdf_get(&pairs, "installdir")?;
    Some(SteamApp {
        app_id,
        name: vdf_get(&pairs, "name").map(str::to_string),
        dir: library.join("steamapps").join("common").join(dir),
        build_id: vdf_get(&pairs, "buildid").map(str::to_string),
        state_flags: vdf_get(&pairs, "StateFlags").and_then(|f| f.parse().ok()),
    })
}

/// Find an installed Steam game.
pub fn find_app(app_id: u32) -> Option<SteamApp> {
    let steam = steam_dir()?;
    library_folders(&steam).into_iter().find_map(|lib| {
        let text = std::fs::read_to_string(lib.join("steamapps").join(format!("appmanifest_{app_id}.acf"))).ok()?;
        parse_manifest(app_id, &lib, &text).filter(|a| a.dir.is_dir())
    })
}

/// Start a game through Steam with extra launch arguments.
pub fn launch(app_id: u32, args: &[String]) -> Result<(), String> {
    let exe = steam_exe().ok_or("Steam is not installed")?;
    std::process::Command::new(exe)
        .arg("-applaunch")
        .arg(app_id.to_string())
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start Steam: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_library_and_manifest() {
        let lib = r#""libraryfolders"
        {
            "0" { "path" "C:\\Program Files (x86)\\Steam" "apps" { "228980" "0" } }
            "1" { "path" "F:\\SteamLibrary" // comment
                  "apps" { "1133870" "123" } }
        }"#;
        let paths: Vec<String> = vdf_pairs(lib).into_iter().filter(|(k, _)| k == "path").map(|(_, v)| v).collect();
        assert_eq!(paths, vec!["C:\\Program Files (x86)\\Steam", "F:\\SteamLibrary"]);
        let acf = r#""AppState" { "appid" "1133870" "name" "Space Engineers 2" "StateFlags" "4" "installdir" "SpaceEngineers2" "buildid" "26536832" }"#;
        let app = parse_manifest(1133870, Path::new("F:\\SteamLibrary"), acf).unwrap();
        assert_eq!(app.dir, PathBuf::from("F:\\SteamLibrary\\steamapps\\common\\SpaceEngineers2"));
        assert_eq!(app.build_id.as_deref(), Some("26536832"));
        assert!(!app.updating());
    }
}

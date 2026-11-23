//! Photos of devices, kept only on this PC.
//!
//! A person (or their agent) can line a photo up with a layout's view so the
//! picture view shows the real device under the same control markers. Photos
//! live beside the layouts, never inside them, so exporting or sharing a
//! layout never shares a photo:
//!
//! `photos\<layout>\<view>.<ext>` and `photos\<layout>\photos.json`

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Largest photo accepted.
pub const MAX_PHOTO_BYTES: usize = 15 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhotoInfo {
    /// File name inside the layout's photo folder.
    pub file: String,
    pub mime: String,
    /// Pixel size of the image.
    pub width: u32,
    pub height: u32,
    /// Where the image sits in the view: `[x, y, width, height]` in view units.
    pub fit: [f64; 4],
    /// Changes whenever the photo or its fit changes, so the window reloads it.
    pub rev: u64,
}

pub struct Photos {
    dir: PathBuf,
}

fn kind(data: &[u8]) -> Option<(&'static str, &'static str)> {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(("png", "image/png"))
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("jpg", "image/jpeg"))
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else {
        None
    }
}

/// The largest box with the image's proportions that fits the view, centred.
pub fn contain(view: [f64; 2], width: u32, height: u32) -> [f64; 4] {
    let (vw, vh) = (view[0], view[1]);
    let (iw, ih) = (width.max(1) as f64, height.max(1) as f64);
    let s = (vw / iw).min(vh / ih);
    let (w, h) = (iw * s, ih * s);
    [(vw - w) / 2.0, (vh - h) / 2.0, w, h]
}

fn now_rev() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Photos {
    pub fn new(data_dir: &Path) -> Photos {
        Photos { dir: data_dir.join("photos") }
    }

    fn folder(&self, layout: &str) -> PathBuf {
        self.dir.join(layout)
    }

    /// Photos for a layout's views.
    pub fn list(&self, layout: &str) -> IndexMap<String, PhotoInfo> {
        fs::read_to_string(self.folder(layout).join("photos.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn write_list(&self, layout: &str, list: &IndexMap<String, PhotoInfo>) -> Result<(), String> {
        let folder = self.folder(layout);
        fs::create_dir_all(&folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
        let path = folder.join("photos.json");
        fs::write(&path, serde_json::to_string_pretty(list).unwrap()).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Store a new photo for a view (fitted to the view unless `fit` is given),
    /// or with `data` None, move the existing one.
    pub fn set(&self, layout: &str, view: &str, view_size: [f64; 2], data: Option<&[u8]>, fit: Option<[f64; 4]>) -> Result<PhotoInfo, String> {
        let mut list = self.list(layout);
        let info = match data {
            Some(bytes) => {
                if bytes.len() > MAX_PHOTO_BYTES {
                    return Err(format!("photos can be at most {} MB", MAX_PHOTO_BYTES / 1024 / 1024));
                }
                let (ext, mime) = kind(bytes).ok_or("the photo must be a PNG, JPEG or WebP image")?;
                let size = imagesize::blob_size(bytes).map_err(|e| format!("cannot read the photo's size: {e}"))?;
                let (width, height) = (size.width as u32, size.height as u32);
                if let Some(old) = list.get(view) {
                    let _ = fs::remove_file(self.folder(layout).join(&old.file));
                }
                let folder = self.folder(layout);
                fs::create_dir_all(&folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
                let file = format!("{view}.{ext}");
                fs::write(folder.join(&file), bytes).map_err(|e| format!("cannot save the photo: {e}"))?;
                PhotoInfo { file, mime: mime.into(), width, height, fit: fit.unwrap_or_else(|| contain(view_size, width, height)), rev: now_rev() }
            }
            None => {
                let mut info = list.get(view).cloned().ok_or_else(|| format!("view '{view}' has no photo yet; send one first"))?;
                info.fit = fit.ok_or("give a fit: [x, y, width, height]")?;
                info.rev = now_rev();
                info
            }
        };
        let [x, y, w, h] = info.fit;
        if ![x, y, w, h].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
            return Err("fit is [x, y, width, height] with width and height above 0".into());
        }
        list.insert(view.to_string(), info.clone());
        self.write_list(layout, &list)?;
        Ok(info)
    }

    pub fn read(&self, layout: &str, view: &str) -> Option<(Vec<u8>, String)> {
        let info = self.list(layout).shift_remove(view)?;
        fs::read(self.folder(layout).join(&info.file)).ok().map(|b| (b, info.mime))
    }

    pub fn delete(&self, layout: &str, view: &str) -> Result<bool, String> {
        let mut list = self.list(layout);
        let Some(info) = list.shift_remove(view) else { return Ok(false) };
        let _ = fs::remove_file(self.folder(layout).join(&info.file));
        self.write_list(layout, &list)?;
        Ok(true)
    }

    /// Remove every photo of a layout.
    pub fn delete_all(&self, layout: &str) {
        let _ = fs::remove_dir_all(self.folder(layout));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 2 x 1 pixel PNG.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x7B, 0x40,
        0xE8, 0xDD, 0x00, 0x00, 0x00, 0x0E, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x56, 0xC7, 0x2F, 0x0D, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn stores_fits_and_deletes() {
        let dir = std::env::temp_dir().join(format!("vbind-photos-{}", std::process::id()));
        let photos = Photos::new(&dir);
        assert_eq!(contain([100.0, 100.0], 2, 1), [0.0, 25.0, 100.0, 50.0]);
        let info = photos.set("stick", "side", [100.0, 200.0], Some(PNG), None).unwrap();
        assert_eq!((info.width, info.height, info.fit), (2, 1, [0.0, 75.0, 100.0, 50.0]));
        assert_eq!(photos.read("stick", "side").unwrap().1, "image/png");
        let moved = photos.set("stick", "side", [100.0, 200.0], None, Some([1.0, 2.0, 3.0, 4.0])).unwrap();
        assert_eq!(moved.fit, [1.0, 2.0, 3.0, 4.0]);
        assert!(photos.set("stick", "side", [1.0, 1.0], Some(b"GIF89a"), None).is_err());
        assert!(photos.delete("stick", "side").unwrap());
        assert!(photos.read("stick", "side").is_none());
        let _ = fs::remove_dir_all(dir);
    }
}

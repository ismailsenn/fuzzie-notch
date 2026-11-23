//! The file shelf (files dropped on the notch) and saving it, together with the
//! clipboard history, to `shelf.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Controls::IImageList;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHGetImageList, SHFILEINFOW, SHGFI_SYSICONINDEX, SHIL_EXTRALARGE};
use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

use crate::clipboard::{self, SavedClip};
use crate::image::{self, Image};
use crate::settings;

pub struct ShelfFile {
    pub path: PathBuf,
    pub name: String,
    pub icon: Option<Arc<Image>>,
}

#[derive(Default)]
pub struct Shelf {
    pub files: Vec<ShelfFile>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    files: Vec<PathBuf>,
    clips: Vec<SavedClip>,
}

fn path() -> PathBuf {
    settings::data_dir().join("shelf.json")
}

impl Shelf {
    pub fn add(&mut self, path: PathBuf) {
        if self.files.iter().any(|f| f.path == path) {
            return;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        let icon = file_icon(&path).map(Arc::new);
        self.files.push(ShelfFile { path, name, icon });
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.files.len() {
            self.files.remove(i);
        }
    }
}

/// Loads the shelf and clipboard history saved by [`save`].
pub fn load(include_files: bool, include_clips: bool) -> (Shelf, clipboard::History) {
    let saved: Saved = std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut shelf = Shelf::default();
    if include_files {
        for f in saved.files.into_iter().filter(|p| p.exists()) {
            shelf.add(f);
        }
    }
    let clips = if include_clips { clipboard::History::load(saved.clips) } else { clipboard::History::default() };
    (shelf, clips)
}

pub fn save(shelf: &Shelf, clips: &clipboard::History) {
    let saved = Saved { files: shelf.files.iter().map(|f| f.path.clone()).collect(), clips: clips.save() };
    let p = path();
    let tmp = p.with_extension("json.tmp");
    if let Ok(text) = serde_json::to_string_pretty(&saved) {
        let _ = std::fs::create_dir_all(settings::data_dir());
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

/// The 48 px shell icon Explorer shows for this file.
fn file_icon(path: &Path) -> Option<Image> {
    unsafe {
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            &HSTRING::from(path.as_os_str()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        );
        if ok == 0 {
            return None;
        }
        let list: IImageList = SHGetImageList(SHIL_EXTRALARGE as i32).ok()?;
        // ILD_TRANSPARENT = 1
        let icon = list.GetIcon(info.iIcon, 1).ok()?;
        let img = image::from_hicon(icon);
        let _ = DestroyIcon(icon);
        img
    }
}

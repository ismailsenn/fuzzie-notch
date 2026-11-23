//! Clipboard history: text and images copied anywhere in Windows.
//! Respects the "don't record this" flags password managers set.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::Win32::System::Threading::Sleep;

use crate::image::{self, Image};
use crate::settings;

const THUMB: u32 = 44;

pub enum ClipData {
    Text(String),
    Image { path: PathBuf, w: u32, h: u32, hash: u64 },
}

pub struct Clip {
    pub data: ClipData,
    /// "HH:MM" when it was copied.
    pub time: String,
    /// Unix seconds when it was copied; it is dropped a day later.
    pub at: u64,
    pub thumb: Option<Arc<Image>>,
}

/// Clips older than this are removed, so the history does not pile up.
const KEEP_SECS: u64 = 24 * 60 * 60;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Clip {
    pub fn label(&self) -> String {
        match &self.data {
            ClipData::Text(t) => {
                let line = t.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
                let mut s: String = line.chars().take(60).collect();
                if line.chars().count() > 60 {
                    s.push('…');
                }
                s
            }
            ClipData::Image { w, h, .. } => format!("Image  {w}×{h}"),
        }
    }

    fn same(&self, other: &ClipData) -> bool {
        match (&self.data, other) {
            (ClipData::Text(a), ClipData::Text(b)) => a == b,
            (ClipData::Image { hash: a, .. }, ClipData::Image { hash: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// How a clip is stored in shelf.json.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SavedClip {
    Text {
        text: String,
        time: String,
        #[serde(default)]
        at: u64,
    },
    Image {
        path: PathBuf,
        w: u32,
        h: u32,
        time: String,
        #[serde(default)]
        at: u64,
    },
}

#[derive(Default)]
pub struct History {
    pub items: Vec<Clip>,
}

fn clips_dir() -> PathBuf {
    settings::data_dir().join("clips")
}

impl History {
    pub fn start(hwnd: HWND) {
        unsafe {
            let _ = AddClipboardFormatListener(hwnd);
        }
    }

    /// Reads the clipboard after WM_CLIPBOARDUPDATE. Returns true if the list changed.
    pub fn on_update(&mut self, hwnd: HWND, max: usize, time: String) -> bool {
        let Some(data) = (unsafe { read(hwnd) }) else { return false };
        // Copying something that is already in the list leaves it where it is.
        // (A duplicate image was written to the same hash-named file, so nothing to clean up.)
        if self.items.iter().any(|c| c.same(&data)) {
            return false;
        }
        let thumb = thumb_for(&data);
        self.items.insert(0, Clip { data, time, at: now_secs(), thumb });
        self.trim(max);
        true
    }

    pub fn trim(&mut self, max: usize) {
        while self.items.len() > max.max(1) {
            if let Some(c) = self.items.pop() {
                delete_file(&c);
            }
        }
    }

    /// Drops clips copied more than a day ago. Returns true if any went.
    pub fn expire(&mut self) -> bool {
        let now = now_secs();
        let before = self.items.len();
        self.items.retain(|c| {
            let keep = c.at + KEEP_SECS > now;
            if !keep {
                delete_file(c);
            }
            keep
        });
        self.items.len() != before
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.items.len() {
            delete_file(&self.items.remove(i));
        }
    }

    pub fn clear(&mut self) {
        for c in self.items.drain(..) {
            delete_file(&c);
        }
    }

    /// Puts item `i` back on the clipboard.
    pub fn restore(&self, hwnd: HWND, i: usize) {
        let Some(clip) = self.items.get(i) else { return };
        unsafe {
            if !open(hwnd) {
                return;
            }
            let _ = EmptyClipboard();
            match &clip.data {
                ClipData::Text(t) => {
                    let wide: Vec<u16> = t.encode_utf16().chain([0]).collect();
                    set(CF_UNICODETEXT.0 as u32, bytes_of(&wide));
                }
                ClipData::Image { path, .. } => {
                    if let Ok(png) = std::fs::read(path) {
                        if let Some((w, h, px)) = image::decode_png(&png) {
                            set(CF_DIB.0 as u32, &image::encode_dib(w, h, &px));
                        }
                        set(RegisterClipboardFormatW(w!("PNG")), &png);
                    }
                }
            }
            let _ = CloseClipboard();
        }
    }

    pub fn save(&self) -> Vec<SavedClip> {
        self.items
            .iter()
            .map(|c| match &c.data {
                ClipData::Text(text) => SavedClip::Text { text: text.clone(), time: c.time.clone(), at: c.at },
                ClipData::Image { path, w, h, .. } => {
                    SavedClip::Image { path: path.clone(), w: *w, h: *h, time: c.time.clone(), at: c.at }
                }
            })
            .collect()
    }

    pub fn load(saved: Vec<SavedClip>) -> Self {
        let mut items = Vec::new();
        let now = now_secs();
        for s in saved {
            let (data, time, at) = match s {
                SavedClip::Text { text, time, at } => (ClipData::Text(text), time, at),
                SavedClip::Image { path, w, h, time, at } => {
                    let Ok(bytes) = std::fs::read(&path) else { continue };
                    (ClipData::Image { path, w, h, hash: image::hash(&bytes) }, time, at)
                }
            };
            // Clips saved before copy times were kept get a day from now.
            let at = if at == 0 { now } else { at };
            let thumb = thumb_for(&data);
            items.push(Clip { data, time, at, thumb });
        }
        let mut h = Self { items };
        h.expire();
        h
    }
}

fn delete_file(c: &Clip) {
    if let ClipData::Image { path, .. } = &c.data {
        let _ = std::fs::remove_file(path);
    }
}

fn thumb_for(data: &ClipData) -> Option<Arc<Image>> {
    let ClipData::Image { path, .. } = data else { return None };
    let (w, h, px) = image::decode_png(&std::fs::read(path).ok()?)?;
    Some(Arc::new(Image::from_straight(THUMB, THUMB, image::thumbnail(w, h, &px, THUMB))))
}

fn bytes_of(v: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) }
}

/// Puts plain text on the clipboard.
pub fn set_text(hwnd: HWND, text: &str) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    unsafe {
        if open(hwnd) {
            let _ = EmptyClipboard();
            set(13, bytes_of(&wide)); // CF_UNICODETEXT
            let _ = CloseClipboard();
        }
    }
}

/// Plain text on the clipboard, if any.
pub fn get_text(hwnd: HWND) -> Option<String> {
    unsafe {
        if !open(hwnd) {
            return None;
        }
        let bytes = get(13);
        let _ = CloseClipboard();
        let wide: Vec<u16> = bytes?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        Some(String::from_utf16_lossy(&wide))
    }
}

unsafe fn open(hwnd: HWND) -> bool {
    // Another app may hold the clipboard for a moment.
    for _ in 0..6 {
        if OpenClipboard(Some(hwnd)).is_ok() {
            return true;
        }
        Sleep(15);
    }
    false
}

unsafe fn set(format: u32, bytes: &[u8]) {
    let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return };
    let p = GlobalLock(mem);
    if p.is_null() {
        let _ = GlobalFree(Some(mem));
        return;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
    let _ = GlobalUnlock(mem);
    if SetClipboardData(format, Some(HANDLE(mem.0))).is_err() {
        let _ = GlobalFree(Some(mem));
    }
}

unsafe fn get(format: u32) -> Option<Vec<u8>> {
    IsClipboardFormatAvailable(format).ok()?;
    let h = GetClipboardData(format).ok()?;
    let mem = HGLOBAL(h.0);
    let p = GlobalLock(mem);
    if p.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(p as *const u8, GlobalSize(mem)).to_vec();
    let _ = GlobalUnlock(mem);
    Some(bytes)
}

fn format(name: PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

unsafe fn read(hwnd: HWND) -> Option<ClipData> {
    if !open(hwnd) {
        return None;
    }
    let data = read_open();
    let _ = CloseClipboard();
    data
}

unsafe fn read_open() -> Option<ClipData> {
    // Password managers and others mark copies that must not be recorded.
    if IsClipboardFormatAvailable(format(w!("ExcludeClipboardContentFromMonitorProcessing"))).is_ok()
        || IsClipboardFormatAvailable(format(w!("Clipboard Viewer Ignore"))).is_ok()
    {
        return None;
    }
    if let Some(v) = get(format(w!("CanIncludeInClipboardHistory"))) {
        if v.len() >= 4 && u32::from_le_bytes(v[..4].try_into().unwrap()) == 0 {
            return None;
        }
    }
    if let Some(bytes) = get(CF_UNICODETEXT.0 as u32) {
        let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        let text = String::from_utf16_lossy(&wide[..end]);
        return (!text.trim().is_empty()).then_some(ClipData::Text(text));
    }
    let png_format = format(w!("PNG"));
    let decoded = get(png_format)
        .and_then(|b| image::decode_png(&b))
        .or_else(|| get(CF_DIB.0 as u32).and_then(|b| image::decode_dib(&b)))?;
    let (w, h, px) = decoded;
    let png = image::encode_png(w, h, &px)?;
    let hash = image::hash(&png);
    let dir = clips_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{hash:016x}.png"));
    std::fs::write(&path, &png).ok()?;
    Some(ClipData::Image { path, w, h, hash })
}

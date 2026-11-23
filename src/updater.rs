//! Updates from this project's GitHub releases. A little while after start-up
//! and then every few hours, the latest release is looked up; when it is newer,
//! its `Fuzzie.exe` is downloaded, checked against the SHA-256 GitHub publishes
//! for it, swapped in for the running exe, and Fuzzie restarts on the new one.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

use serde_json::Value;
use windows::core::HSTRING;
use windows::Foundation::Uri;
use windows::Storage::Streams::DataReader;
use windows::Web::Http::HttpClient;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Where releases are published.
const REPO: &str = "ismailsenn/Fuzzie";
/// The release asset that is the app itself.
const ASSET: &str = "Fuzzie.exe";
const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(6 * 3600);

/// Posted to the notch window when a new exe is in place: restart on it.
pub const WM_UPDATED: u32 = WM_APP + 9;

static ENABLED: AtomicBool = AtomicBool::new(true);
static WINDOW: AtomicIsize = AtomicIsize::new(0);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Starts the background checks; `hwnd` gets [`WM_UPDATED`] after an update.
pub fn start(hwnd: HWND) {
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    cleanup();
    let _ = std::thread::Builder::new().name("updater".into()).stack_size(512 * 1024).spawn(|| {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        std::thread::sleep(FIRST_CHECK);
        loop {
            if ENABLED.load(Ordering::Relaxed) && check().is_some() {
                let hwnd = HWND(WINDOW.load(Ordering::Relaxed) as *mut core::ffi::c_void);
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_UPDATED, WPARAM(0), LPARAM(0));
                }
                return;
            }
            std::thread::sleep(EVERY);
        }
    });
}

/// Starts the freshly installed exe (it waits for this one to quit).
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--after-update").spawn();
    }
}

fn paths() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    Some((exe, dir.join("Fuzzie.update.tmp"), dir.join("Fuzzie.old.exe")))
}

/// Removes what the last update left behind.
fn cleanup() {
    if let Some((_, tmp, old)) = paths() {
        let _ = std::fs::remove_file(tmp);
        let _ = std::fs::remove_file(old);
    }
}

/// "v1.2.3" / "1.2.3" → (1, 2, 3).
fn version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split(['-', '+']).next()?;
    let mut it = s.split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// Looks for a newer release and installs it; Some(()) once the new exe is in place.
fn check() -> Option<()> {
    let latest = get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let rel: Value = serde_json::from_slice(&latest).ok()?;
    if rel["draft"].as_bool() == Some(true) || rel["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let newest = version(rel["tag_name"].as_str()?)?;
    if newest <= version(env!("CARGO_PKG_VERSION"))? {
        return None;
    }
    let asset = rel["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(ASSET))?;
    let url = asset["browser_download_url"].as_str()?;
    let size = asset["size"].as_u64()?;
    // Only install what GitHub vouches for: no published digest, no update.
    let want = asset["digest"].as_str()?.strip_prefix("sha256:")?.to_ascii_lowercase();
    let bytes = get(url)?;
    if bytes.len() as u64 != size || !bytes.starts_with(b"MZ") || sha256_hex(&bytes)? != want {
        return None;
    }
    install(&bytes)
}

/// Writes the new exe beside the running one and swaps them (a running exe can
/// be renamed, not overwritten).
fn install(bytes: &[u8]) -> Option<()> {
    let (exe, tmp, old) = paths()?;
    std::fs::write(&tmp, bytes).ok()?;
    let _ = std::fs::remove_file(&old);
    if std::fs::rename(&exe, &old).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    if std::fs::rename(&tmp, &exe).is_err() {
        // Put the old one back so Fuzzie still starts.
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    Some(())
}

/// GET `url` (redirects followed) and return the body.
fn get(url: &str) -> Option<Vec<u8>> {
    let client = HttpClient::new().ok()?;
    if let Ok(h) = client.DefaultRequestHeaders() {
        if let Ok(ua) = h.UserAgent() {
            let _ = ua.TryParseAdd(&HSTRING::from(concat!("Fuzzie/", env!("CARGO_PKG_VERSION"))));
        }
        let _ = h.TryAppendWithoutValidation(&HSTRING::from("Accept"), &HSTRING::from("application/vnd.github+json, application/octet-stream"));
    }
    let resp = client.GetAsync(&Uri::CreateUri(&HSTRING::from(url)).ok()?).ok()?.join().ok()?;
    if !resp.IsSuccessStatusCode().ok()? {
        return None;
    }
    let buf = resp.Content().ok()?.ReadAsBufferAsync().ok()?.join().ok()?;
    let n = buf.Length().ok()? as usize;
    let reader = DataReader::FromBuffer(&buf).ok()?;
    let mut out = vec![0u8; n];
    reader.ReadBytes(&mut out).ok()?;
    Some(out)
}

fn sha256_hex(data: &[u8]) -> Option<String> {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut out = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) }.ok().ok()?;
    Some(out.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(version("0.2"), Some((0, 2, 0)));
        assert_eq!(version("v1.0.0-beta"), Some((1, 0, 0)));
        assert!(version("v0.10.0") > version("v0.9.9"));
        assert_eq!(version("latest"), None);
    }

    #[test]
    fn hashes() {
        assert_eq!(sha256_hex(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}

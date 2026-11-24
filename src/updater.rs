//! Updates from this project's GitHub releases. Right after start-up and then
//! every minute, a cheap probe asks which release is the latest; when it is
//! newer, the release is looked up, its `Fuzzie.exe` downloaded, checked
//! against the SHA-256 GitHub publishes for it, swapped in for the running
//! exe, and Fuzzie restarts on the new one.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
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
const REPO: &str = "ismailsenn/fuzzie-notch";
/// The release asset that is the app itself.
const ASSET: &str = "Fuzzie.exe";
/// Probe soon after start, then every minute, so a new release is installed
/// about a minute after it is published. The probe is a HEAD request to the
/// release page (not the API, which allows only 60 calls an hour); the API
/// is only asked once the probe has seen a newer tag, and anyway every
/// [`FULL_EVERY`] in case the probe stops working.
const FIRST_CHECK: Duration = Duration::from_secs(5);
const EVERY: Duration = Duration::from_secs(60);
const FULL_EVERY: u32 = 15;

/// Posted to the notch window when a new exe is in place: restart on it.
pub const WM_UPDATED: u32 = WM_APP + 9;

static ENABLED: AtomicBool = AtomicBool::new(true);
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// One check or install at a time.
static BUSY: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<Status> = Mutex::new(Status::Idle);

/// What the Updates page shows.
#[derive(Clone, PartialEq, Debug)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    /// A newer release (its tag) can be installed.
    Available(String),
    Installing,
    Failed,
}

pub fn status() -> Status {
    STATUS.lock().map(|s| s.clone()).unwrap_or(Status::Idle)
}

fn set_status(s: Status) {
    if let Ok(mut g) = STATUS.lock() {
        *g = s;
    }
}

/// A release newer than this build.
struct Release {
    tag: String,
    url: String,
    size: u64,
    sha256: String,
}

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
        let mut n = 0u32;
        loop {
            if ENABLED.load(Ordering::Relaxed) {
                // A newer tag seen by the probe, the first round, or one in a while.
                let ask = n % FULL_EVERY == 0 || probe().is_some_and(|tag| is_newer(&tag));
                if ask && run(true) {
                    return;
                }
                n += 1;
            }
            std::thread::sleep(EVERY);
        }
    });
}

/// "Check for updates": looks for a release, installs nothing.
pub fn check_now() {
    spawn(false);
}

/// "Install update": downloads and installs the newest release, then restarts.
pub fn install_now() {
    spawn(true);
}

fn spawn(install: bool) {
    let _ = std::thread::Builder::new().name("update-now".into()).stack_size(512 * 1024).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        run(install);
    });
}

/// One check (and install, if asked); true once the new exe is in place and
/// the notch window has been told to restart on it.
fn run(install: bool) -> bool {
    if BUSY.swap(true, Ordering::AcqRel) {
        return false;
    }
    set_status(Status::Checking);
    let done = match latest() {
        None => {
            set_status(Status::Failed);
            false
        }
        Some(None) => {
            set_status(Status::UpToDate);
            false
        }
        Some(Some(rel)) if !install => {
            set_status(Status::Available(rel.tag));
            false
        }
        Some(Some(rel)) => {
            set_status(Status::Installing);
            if download(&rel).is_some() {
                let hwnd = HWND(WINDOW.load(Ordering::Relaxed) as *mut core::ffi::c_void);
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_UPDATED, WPARAM(0), LPARAM(0));
                }
                true
            } else {
                set_status(Status::Failed);
                false
            }
        }
    };
    BUSY.store(false, Ordering::Release);
    done
}

static RELAUNCH_EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Starts the freshly installed exe, retrying briefly if Windows Defender is scanning it.
/// Returns true once the new process has been spawned.
pub fn relaunch() -> bool {
    let Some(exe) = RELAUNCH_EXE.get().cloned().or_else(|| std::env::current_exe().ok()) else {
        return false;
    };
    for _ in 0..10 {
        if std::process::Command::new(&exe).arg("--after-update").spawn().is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
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

/// The newest release: None if GitHub could not be asked, Some(None) when
/// this build is already the newest.
fn latest() -> Option<Option<Release>> {
    let body = get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let rel: Value = serde_json::from_slice(&body).ok()?;
    Some(newer(&rel))
}

/// The latest release's tag, from where the release page redirects to
/// (".../releases/tag/v1.2.3"). Cheap: a HEAD request, no API call.
fn probe() -> Option<String> {
    use windows::Web::Http::{HttpMethod, HttpRequestMessage};
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    // The stamp keeps any cache from answering with an old redirect.
    let uri = Uri::CreateUri(&HSTRING::from(format!("https://github.com/{REPO}/releases/latest?t={stamp}"))).ok()?;
    let client = HttpClient::new().ok()?;
    if let Ok(h) = client.DefaultRequestHeaders() {
        if let Ok(ua) = h.UserAgent() {
            let _ = ua.TryParseAdd(&HSTRING::from(concat!("Fuzzie/", env!("CARGO_PKG_VERSION"))));
        }
    }
    let req = HttpRequestMessage::Create(&HttpMethod::Head().ok()?, &uri).ok()?;
    let resp = client.SendRequestAsync(&req).ok()?.join().ok()?;
    let url = resp.RequestMessage().ok()?.RequestUri().ok()?.Path().ok()?.to_string();
    tag_from_path(&url)
}

/// ".../releases/tag/v1.2.3" → "v1.2.3".
fn tag_from_path(path: &str) -> Option<String> {
    let (_, tag) = path.rsplit_once("/releases/tag/")?;
    let tag = tag.trim_end_matches('/');
    version(tag).map(|_| tag.to_string())
}

fn is_newer(tag: &str) -> bool {
    matches!((version(tag), version(env!("CARGO_PKG_VERSION"))), (Some(a), Some(b)) if a > b)
}

fn newer(rel: &Value) -> Option<Release> {
    if rel["draft"].as_bool() == Some(true) || rel["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let tag = rel["tag_name"].as_str()?;
    if version(tag)? <= version(env!("CARGO_PKG_VERSION"))? {
        return None;
    }
    let asset = rel["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(ASSET))?;
    Some(Release {
        tag: tag.to_string(),
        url: asset["browser_download_url"].as_str()?.to_string(),
        size: asset["size"].as_u64()?,
        // Only install what GitHub vouches for: no published digest, no update.
        sha256: asset["digest"].as_str()?.strip_prefix("sha256:")?.to_ascii_lowercase(),
    })
}

/// Downloads, verifies and installs `rel`; Some(()) once the new exe is in place.
fn download(rel: &Release) -> Option<()> {
    let bytes = get(&rel.url)?;
    if bytes.len() as u64 != rel.size || !bytes.starts_with(b"MZ") || sha256_hex(&bytes)? != rel.sha256 {
        return None;
    }
    install(&bytes)
}

/// Writes the new exe beside the running one and swaps them (a running exe can
/// be renamed, not overwritten).
fn install(bytes: &[u8]) -> Option<()> {
    let (exe, tmp, old) = paths()?;
    let _ = RELAUNCH_EXE.set(exe.clone());
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
    // Tell Windows Explorer / Taskbar to refresh the cached exe icon immediately.
    unsafe {
        use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
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
    fn tags_from_redirects() {
        assert_eq!(tag_from_path("/ismailsenn/fuzzie-notch/releases/tag/v0.2.7").as_deref(), Some("v0.2.7"));
        assert_eq!(tag_from_path("/ismailsenn/fuzzie-notch/releases"), None);
        assert!(is_newer("v999.0.0"));
        assert!(!is_newer("v0.0.1"));
    }

    /// Asks GitHub for real; run with `cargo test --release -- --ignored`.
    #[test]
    #[ignore]
    fn probe_live() {
        let tag = probe();
        println!("latest: {tag:?}");
        assert!(tag.is_some());
    }

    #[test]
    fn hashes() {
        assert_eq!(sha256_hex(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}

//! Usage stats for the Stats page:
//! Spotify listening time (per year, song and artist), key presses and
//! clicks, the daily streak, and friend counters (filled in by the relay later).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WM_XBUTTONDOWN,
};

use crate::settings;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Year {
    pub secs: f64,
    /// "Title — Artist" -> seconds.
    pub songs: HashMap<String, f64>,
    pub artists: HashMap<String, f64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Friend {
    pub bonks_sent: u64,
    pub bonks_received: u64,
    pub kisses_sent: u64,
    pub kisses_received: u64,
    pub pies_sent: u64,
    pub pies_received: u64,
    pub pokes_sent: u64,
    pub pokes_received: u64,
    pub rps_played: u64,
    pub rps_won: u64,
    pub rps_lost: u64,
    pub races_won: u64,
    pub visits_made: u64,
    pub visits_received: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub presses: u64,
    pub listen_secs: f64,
    pub years: BTreeMap<i32, Year>,
    /// Consecutive days Fuzzie ran on.
    pub streak: u32,
    pub best_streak: u32,
    /// Day number of the last day counted for the streak.
    pub last_day: i64,
    pub friend: Friend,
}

/// Key presses and clicks counted since the last flush into `Stats`.
static PENDING: AtomicU64 = AtomicU64::new(0);
/// Keys currently held down, so auto-repeat is not counted.
static DOWN: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

fn state() -> &'static Mutex<Stats> {
    static S: OnceLock<Mutex<Stats>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(load()))
}

fn path() -> PathBuf {
    settings::data_dir().join("stats.json")
}

fn load() -> Stats {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Loads the stats, counts today for the streak and starts counting input.
/// Must run on a thread with a message loop (the low-level hooks need one).
pub fn init() {
    touch_day();
    unsafe {
        let module = GetModuleHandleW(None).ok();
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module.map(Into::into), 0);
        let _ = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module.map(Into::into), 0);
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let vk = (k.vkCode & 255) as usize;
        let (slot, bit) = (&DOWN[vk / 64], 1u64 << (vk % 64));
        match wp.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if slot.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
                    PENDING.fetch_add(1, Ordering::Relaxed);
                    crate::deskfox::hit(false);
                }
            }
            WM_KEYUP | WM_SYSKEYUP => {
                slot.fetch_and(!bit, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn mouse_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wp.0 as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) {
        PENDING.fetch_add(1, Ordering::Relaxed);
        crate::deskfox::hit(true);
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn day_number(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn today() -> (i32, i64) {
    let t = unsafe { GetLocalTime() };
    (t.wYear as i32, day_number(t.wYear as i32, t.wMonth as u32, t.wDay as u32))
}

/// Counts today toward the streak (call at start and now and then).
pub fn touch_day() {
    let (_, day) = today();
    let mut s = state().lock().unwrap();
    if s.last_day == day {
        return;
    }
    s.streak = if s.last_day == day - 1 { s.streak + 1 } else { 1 };
    s.best_streak = s.best_streak.max(s.streak);
    s.last_day = day;
}

/// Adds listening time for the current song.
pub fn add_listening(title: &str, artist: &str, secs: f64) {
    if secs <= 0.0 || title.is_empty() {
        return;
    }
    let (year, _) = today();
    let mut s = state().lock().unwrap();
    s.listen_secs += secs;
    let y = s.years.entry(year).or_default();
    y.secs += secs;
    let song = if artist.is_empty() { title.to_string() } else { format!("{title} \u{2014} {artist}") };
    *y.songs.entry(song).or_default() += secs;
    if !artist.is_empty() {
        *y.artists.entry(artist.to_string()).or_default() += secs;
    }
}

/// A copy of the current stats, including presses not yet flushed.
pub fn snapshot() -> Stats {
    let mut s = state().lock().unwrap().clone();
    s.presses += PENDING.load(Ordering::Relaxed);
    s
}

pub fn save() {
    let text = {
        let mut s = state().lock().unwrap();
        s.presses += PENDING.swap(0, Ordering::Relaxed);
        serde_json::to_string(&*s)
    };
    let Ok(text) = text else { return };
    let p = path();
    let tmp = p.with_extension("json.tmp");
    let _ = std::fs::create_dir_all(settings::data_dir());
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// "1 day 7 hours 34 minutes 42 seconds": every unit from the largest non-zero one down.
pub fn duration_words(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let parts = [(total / 86400, "day"), (total / 3600 % 24, "hour"), (total / 60 % 60, "minute"), (total % 60, "second")];
    let start = parts.iter().position(|(n, _)| *n > 0).unwrap_or(3);
    parts[start..]
        .iter()
        .map(|(n, unit)| format!("{n} {unit}{}", if *n == 1 { "" } else { "s" }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 90882 -> "90,882"
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(duration_words(0.0), "0 seconds");
        assert_eq!(duration_words(61.0), "1 minute 1 second");
        assert_eq!(duration_words(113_682.0), "1 day 7 hours 34 minutes 42 seconds");
        assert_eq!(duration_words(4140.0), "1 hour 9 minutes 0 seconds");
        assert_eq!(thousands(90882), "90,882");
        assert_eq!(thousands(300000), "300,000");
        assert_eq!(thousands(7), "7");
    }

    #[test]
    fn days() {
        assert_eq!(day_number(1970, 1, 1), 0);
        assert_eq!(day_number(2026, 3, 1) - day_number(2026, 2, 28), 1);
        assert_eq!(day_number(2024, 3, 1) - day_number(2024, 2, 28), 2);
    }
}

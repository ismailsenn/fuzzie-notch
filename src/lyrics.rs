//! Song lyrics from lrclib.net (free, no key). Synced (LRC) lyrics are preferred;
//! plain lyrics are kept for a future lyrics view.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use windows::core::HSTRING;
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

#[derive(Clone, Default)]
pub struct Lyrics {
    /// The track key from `MediaInfo::key`.
    pub key: String,
    /// (start ms, line); empty when nothing was found.
    /// Shared, so the notch can read it every frame without copying.
    pub lines: Arc<Vec<(u64, String)>>,
    pub synced: bool,
}

impl Lyrics {
    /// Index of the line being sung at `pos_ms`; -1 before the first line.
    pub fn index_at(&self, pos_ms: u64) -> isize {
        self.lines.partition_point(|(t, _)| *t <= pos_ms) as isize - 1
    }
}

#[derive(Default)]
pub struct State {
    /// Bumped on each request so a slow, stale answer cannot overwrite a newer one.
    generation: u64,
    pub current: Option<Lyrics>,
}

pub type Shared = Arc<Mutex<State>>;

fn cache() -> &'static Mutex<HashMap<String, Lyrics>> {
    static C: OnceLock<Mutex<HashMap<String, Lyrics>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Looks up lyrics for a track in the background and calls `notify` when done.
pub fn request(shared: &Shared, key: String, title: &str, artist: &str, duration_ms: u64, notify: impl Fn() + Send + 'static) {
    let generation = {
        let mut s = shared.lock().unwrap();
        s.generation += 1;
        s.current = None;
        s.generation
    };
    if let Some(hit) = cache().lock().unwrap().get(&key).cloned() {
        shared.lock().unwrap().current = Some(hit);
        notify();
        return;
    }
    let (shared, title, artist) = (shared.clone(), title.to_string(), artist.to_string());
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let (lines, synced) = find(&title, &artist, duration_ms).unwrap_or_default();
        let lyrics = Lyrics { key: key.clone(), lines: Arc::new(lines), synced };
        {
            let mut c = cache().lock().unwrap();
            // Plenty for a listening session; keeps memory flat over days.
            if c.len() >= 200 {
                c.clear();
            }
            c.insert(key, lyrics.clone());
        }
        let mut s = shared.lock().unwrap();
        if s.generation == generation {
            s.current = Some(lyrics);
            drop(s);
            notify();
        }
    });
}

fn find(title: &str, artist: &str, duration_ms: u64) -> Option<(Vec<(u64, String)>, bool)> {
    let mut plain = None;
    for (t, a) in candidates(title, artist) {
        if duration_ms > 0 {
            let url = format!(
                "https://lrclib.net/api/get?track_name={}&artist_name={}&duration={}",
                enc(&t),
                enc(&a),
                duration_ms / 1000
            );
            if let Some(rec) = get_json(&url) {
                match from_record(&rec) {
                    Some(r) if r.1 => return Some(r),
                    Some(r) => plain = plain.or(Some(r)),
                    None => {}
                }
            }
        }
        let url = format!("https://lrclib.net/api/search?track_name={}&artist_name={}", enc(&t), enc(&a));
        if let Some(Value::Array(list)) = get_json(&url) {
            // Prefer synced lyrics whose duration is closest to the track.
            let dur = duration_ms as f64 / 1000.0;
            let mut best: Option<(f64, &Value)> = None;
            for rec in &list {
                if rec["syncedLyrics"].as_str().is_none_or(str::is_empty) {
                    continue;
                }
                let d = rec["duration"].as_f64().unwrap_or(0.0);
                let score = if dur > 0.0 { (d - dur).abs() } else { 0.0 };
                if dur > 0.0 && score > 8.0 {
                    continue;
                }
                if best.is_none_or(|(b, _)| score < b) {
                    best = Some((score, rec));
                }
            }
            if let Some(r) = best.and_then(|(_, rec)| from_record(rec)) {
                return Some(r);
            }
            if plain.is_none() {
                plain = list.iter().find_map(from_record);
            }
        }
    }
    plain
}

fn from_record(rec: &Value) -> Option<(Vec<(u64, String)>, bool)> {
    if let Some(s) = rec["syncedLyrics"].as_str().filter(|s| !s.is_empty()) {
        let lines = parse_lrc(s);
        if !lines.is_empty() {
            return Some((lines, true));
        }
    }
    let p = rec["plainLyrics"].as_str().filter(|s| !s.is_empty())?;
    Some((p.lines().map(|l| (0, l.trim().to_string())).collect(), false))
}

/// Title/artist pairs to try, from most to least likely.
fn candidates(title: &str, artist: &str) -> Vec<(String, String)> {
    let t = clean_title(title);
    let a = main_artist(artist);
    let mut out = vec![(t.clone(), a.clone())];
    // Videos are often titled "Artist - Song" with the channel as artist.
    if let Some((left, right)) = t.split_once(" - ") {
        out.push((clean_title(right), main_artist(left)));
    }
    out.dedup();
    out
}

fn clean_title(title: &str) -> String {
    const NOISE: &[&str] = &[
        "official", "video", "lyric", "audio", "remaster", "feat", "ft.", "visualizer", "hd", "4k", "mv", "live",
    ];
    let mut out = String::new();
    let mut rest = title;
    // Drop (...) and [...] groups that only carry noise.
    while let Some(start) = rest.find(['(', '[']) {
        let close = if rest.as_bytes()[start] == b'(' { ')' } else { ']' };
        let Some(len) = rest[start..].find(close) else { break };
        let group = rest[start + 1..start + len].to_lowercase();
        out.push_str(&rest[..start]);
        if !NOISE.iter().any(|n| group.contains(n)) {
            out.push_str(&rest[start..=start + len]);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    for cut in [" feat. ", " ft. ", " | "] {
        if let Some(i) = out.to_ascii_lowercase().find(cut) {
            out.truncate(i);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn main_artist(artist: &str) -> String {
    let lower = artist.to_ascii_lowercase(); // ASCII keeps byte offsets valid
    let mut end = artist.len();
    for sep in [", ", " & ", " x ", " feat", " ft."] {
        if let Some(i) = lower.find(sep) {
            end = end.min(i);
        }
    }
    artist[..end].trim().to_string()
}

fn parse_lrc(s: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for line in s.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            match parse_ts(&stripped[..end]) {
                Some(ms) => times.push(ms),
                None => break,
            }
            rest = &stripped[end + 1..];
        }
        for t in times {
            out.push((t, rest.trim().to_string()));
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

/// "mm:ss.xx" -> milliseconds.
fn parse_ts(s: &str) -> Option<u64> {
    let (m, sec) = s.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let sec: f64 = sec.trim().parse().ok()?;
    Some(m * 60_000 + (sec * 1000.0).round() as u64)
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn get_json(url: &str) -> Option<Value> {
    let client = HttpClient::new().ok()?;
    if let Ok(h) = client.DefaultRequestHeaders().and_then(|h| h.UserAgent()) {
        let _ = h.TryParseAdd(&HSTRING::from(concat!("Fuzzie/", env!("CARGO_PKG_VERSION"))));
    }
    let resp = client.GetAsync(&Uri::CreateUri(&HSTRING::from(url)).ok()?).ok()?.join().ok()?;
    if !resp.IsSuccessStatusCode().ok()? {
        return None;
    }
    let body = resp.Content().ok()?.ReadAsStringAsync().ok()?.join().ok()?;
    serde_json::from_str(&body.to_string()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc() {
        let l = parse_lrc("[00:01.50]one\n[00:03.00][00:10.00]two\nbad line");
        assert_eq!(l, vec![(1500, "one".into()), (3000, "two".into()), (10000, "two".into())]);
        let ly = Lyrics { key: String::new(), lines: Arc::new(l), synced: true };
        assert_eq!(ly.index_at(0), -1);
        assert_eq!(ly.index_at(2000), 0);
        assert_eq!(ly.index_at(4000), 1);
    }

    #[test]
    fn titles() {
        assert_eq!(clean_title("Song (Official Video) [HD]"), "Song");
        assert_eq!(clean_title("Song (Acoustic)"), "Song (Acoustic)");
        assert_eq!(clean_title("Song feat. Someone"), "Song");
        assert_eq!(main_artist("A, B & C"), "A");
        assert_eq!(enc("a b/ç"), "a%20b%2F%C3%A7");
    }
}

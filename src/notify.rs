//! Windows notifications, read straight from the database Windows keeps for
//! the notification centre (`wpndatabase.db`). The file is only ever opened
//! read-only, through the SQLite that ships with Windows (`winsqlite3.dll`),
//! and only re-read when it changes. New toasts are queued for the notch.

use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

/// One notification: the app that sent it, its title and its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub app: String,
    pub title: String,
    pub body: String,
}

pub type Queue = Arc<Mutex<VecDeque<Notice>>>;

/// Starts watching for new notifications; they are pushed onto the queue.
pub fn start() -> Queue {
    let q: Queue = Default::default();
    let out = q.clone();
    let _ = std::thread::Builder::new().name("notify".into()).stack_size(256 * 1024).spawn(move || watch(out));
    q
}

const POLL: Duration = Duration::from_millis(800);

fn watch(out: Queue) {
    let Some(db) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"Microsoft\Windows\Notifications\wpndatabase.db")) else { return };
    let Some(lib) = Sqlite::load() else { return };
    let wal = PathBuf::from(format!("{}-wal", db.display()));
    let stamp = || {
        let m = |p: &PathBuf| std::fs::metadata(p).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        (m(&db), m(&wal))
    };
    let mut seen = stamp();
    // Start after whatever is there now: old notifications are not news.
    let mut last = loop {
        if let Some(id) = lib.max_id(&db) {
            break id;
        }
        std::thread::sleep(Duration::from_secs(5));
    };
    loop {
        std::thread::sleep(POLL);
        let now = stamp();
        if now == seen {
            continue;
        }
        seen = now;
        if let Some(rows) = lib.since(&db, last) {
            for (id, handler, payload) in rows {
                last = last.max(id);
                if let Some(n) = parse(&handler, &payload) {
                    let mut q = out.lock().unwrap();
                    if q.len() < 8 {
                        q.push_back(n);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- SQLite

type Db = *mut c_void;
type Stmt = *mut c_void;
const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_OPEN_READONLY: c_int = 1;
const SQLITE_OPEN_URI: c_int = 0x40;

/// The few SQLite calls we need, from the copy in Windows.
struct Sqlite {
    open: unsafe extern "system" fn(*const c_char, *mut Db, c_int, *const c_char) -> c_int,
    close: unsafe extern "system" fn(Db) -> c_int,
    busy: unsafe extern "system" fn(Db, c_int) -> c_int,
    prepare: unsafe extern "system" fn(Db, *const c_char, c_int, *mut Stmt, *mut *const c_char) -> c_int,
    bind: unsafe extern "system" fn(Stmt, c_int, i64) -> c_int,
    step: unsafe extern "system" fn(Stmt) -> c_int,
    int: unsafe extern "system" fn(Stmt, c_int) -> i64,
    blob: unsafe extern "system" fn(Stmt, c_int) -> *const c_void,
    bytes: unsafe extern "system" fn(Stmt, c_int) -> c_int,
    text: unsafe extern "system" fn(Stmt, c_int) -> *const c_char,
    finalize: unsafe extern "system" fn(Stmt) -> c_int,
}

impl Sqlite {
    fn load() -> Option<Self> {
        unsafe {
            let lib = LoadLibraryW(w!("winsqlite3.dll")).ok()?;
            macro_rules! f {
                ($name:expr) => {
                    std::mem::transmute(GetProcAddress(lib, $name)?)
                };
            }
            Some(Sqlite {
                open: f!(s!("sqlite3_open_v2")),
                close: f!(s!("sqlite3_close")),
                busy: f!(s!("sqlite3_busy_timeout")),
                prepare: f!(s!("sqlite3_prepare_v2")),
                bind: f!(s!("sqlite3_bind_int64")),
                step: f!(s!("sqlite3_step")),
                int: f!(s!("sqlite3_column_int64")),
                blob: f!(s!("sqlite3_column_blob")),
                bytes: f!(s!("sqlite3_column_bytes")),
                text: f!(s!("sqlite3_column_text")),
                finalize: f!(s!("sqlite3_finalize")),
            })
        }
    }

    /// Runs `sql` (with `arg` bound to ?1) on a read-only connection and hands each row to `row`.
    fn query(&self, path: &PathBuf, sql: &str, arg: i64, mut row: impl FnMut(Stmt)) -> Option<()> {
        let uri = CString::new(format!("file:{}?mode=ro", path.display().to_string().replace('\\', "/"))).ok()?;
        let sql = CString::new(sql).ok()?;
        unsafe {
            let mut db: Db = std::ptr::null_mut();
            let rc = (self.open)(uri.as_ptr(), &mut db, SQLITE_OPEN_READONLY | SQLITE_OPEN_URI, std::ptr::null());
            if rc != SQLITE_OK {
                if !db.is_null() {
                    (self.close)(db);
                }
                return None;
            }
            (self.busy)(db, 300);
            let mut st: Stmt = std::ptr::null_mut();
            let ok = (self.prepare)(db, sql.as_ptr(), -1, &mut st, std::ptr::null_mut()) == SQLITE_OK;
            if ok {
                (self.bind)(st, 1, arg);
                while (self.step)(st) == SQLITE_ROW {
                    row(st);
                }
            }
            (self.finalize)(st);
            (self.close)(db);
            ok.then_some(())
        }
    }

    fn max_id(&self, path: &PathBuf) -> Option<i64> {
        let mut id = 0;
        self.query(path, "SELECT IFNULL(MAX(Id), 0) FROM Notification WHERE ?1 = ?1", 0, |st| id = unsafe { (self.int)(st, 0) })?;
        Some(id)
    }

    /// Toasts newer than `last`: (id, app id, XML payload).
    fn since(&self, path: &PathBuf, last: i64) -> Option<Vec<(i64, String, String)>> {
        let mut rows = Vec::new();
        let sql = "SELECT n.Id, IFNULL(h.PrimaryId, ''), n.Payload FROM Notification n \
                   LEFT JOIN NotificationHandler h ON h.RecordId = n.HandlerId \
                   WHERE n.Id > ?1 AND n.Type = 'toast' ORDER BY n.Id";
        self.query(path, sql, last, |st| unsafe {
            let id = (self.int)(st, 0);
            let t = (self.text)(st, 1);
            let handler = if t.is_null() { String::new() } else { CStr::from_ptr(t).to_string_lossy().into_owned() };
            let p = (self.blob)(st, 2);
            let n = (self.bytes)(st, 2).max(0) as usize;
            let payload = if p.is_null() { String::new() } else { String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, n)).into_owned() };
            rows.push((id, handler, payload));
        })?;
        Some(rows)
    }
}

// ---------------------------------------------------------------- parsing

/// Title and text from a toast's XML; None when it has nothing to show.
fn parse(handler: &str, xml: &str) -> Option<Notice> {
    let mut texts = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<text") {
        rest = &rest[i + 5..];
        // `<text>` or `<text attr=...>`, not e.g. `<textbox`.
        if !rest.starts_with('>') && !rest.starts_with(' ') && !rest.starts_with('/') {
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        if rest[..gt].ends_with('/') {
            rest = &rest[gt + 1..];
            continue;
        }
        let body = &rest[gt + 1..];
        let Some(end) = body.find("</text>") else { break };
        let t = unescape(&body[..end]);
        let t = t.trim();
        if !t.is_empty() {
            texts.push(t.to_string());
        }
        rest = &body[end..];
    }
    let mut it = texts.into_iter();
    let title = it.next()?;
    let body: Vec<String> = it.collect();
    Some(Notice { app: app_name(handler), title, body: body.join(" \u{00B7} ") })
}

fn unescape(s: &str) -> String {
    let s = s.replace("<![CDATA[", "").replace("]]>", "");
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';').filter(|&k| k <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A readable app name from its id: "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"
/// → "Windows Terminal", "com.squirrel.Discord.Discord" → "Discord",
/// "C:\...\app.exe" → "app".
fn app_name(id: &str) -> String {
    let id = id.trim();
    let base = if id.contains('\\') || id.to_ascii_lowercase().ends_with(".exe") {
        let file = id.rsplit(['\\', '/']).next().unwrap_or(id);
        file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file).to_string()
    } else {
        let id = id.split('!').next().unwrap_or(id);
        let id = id.split('_').next().unwrap_or(id);
        id.rsplit('.').find(|p| !p.is_empty()).unwrap_or(id).to_string()
    };
    // "WindowsTerminal" → "Windows Terminal".
    let mut out = String::with_capacity(base.len() + 4);
    let chars: Vec<char> = base.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() && chars[i - 1].is_lowercase() {
            out.push(' ');
        }
        out.push(c);
    }
    if out.is_empty() {
        "Notification".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_xml() {
        let xml = r#"<toast><visual><binding template="ToastGeneric"><text hint-maxLines="1">Ana &amp; Bo</text><text>Hi there &#x1F44B;</text><text placement="attribution">via web</text><image src="x"/></binding></visual></toast>"#;
        let n = parse("com.squirrel.Discord.Discord", xml).unwrap();
        assert_eq!(n.app, "Discord");
        assert_eq!(n.title, "Ana & Bo");
        assert_eq!(n.body, "Hi there \u{1F44B} \u{00B7} via web");
        assert!(parse("x", "<toast><visual><binding/></visual></toast>").is_none());
    }

    #[test]
    fn app_names() {
        assert_eq!(app_name("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"), "Windows Terminal");
        assert_eq!(app_name("Chrome"), "Chrome");
        assert_eq!(app_name(r"C:\Program Files\Foo\foo.exe"), "foo");
    }
}

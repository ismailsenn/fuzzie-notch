//! Friends over the Fuzzie relay (`server/`): an account (name + friend code,
//! with a secret key kept in `%APPDATA%\Fuzzie\social.json`), presence, what
//! friends are playing, pokes and listening along.
//!
//! One background thread owns the WebSocket. The UI reads [`snapshot`] and
//! asks for things with the small `send`-style functions; incoming pokes and
//! the like become notch notices.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use windows::core::HSTRING;
use windows::Foundation::{TypedEventHandler, Uri};
use windows::Networking::Sockets::{IWebSocket, MessageWebSocket, MessageWebSocketMessageReceivedEventArgs, SocketMessageType, WebSocketClosedEventArgs};
use windows::Storage::Streams::{DataWriter, UnicodeEncoding};
use windows::Web::Http::{HttpClient, HttpStringContent};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::notify::{Notice, Queue};

/// The relay; `FUZZIE_RELAY` overrides it (e.g. `http://127.0.0.1:8787` for `wrangler dev`).
const RELAY: &str = "https://fuzzie-relay.fuzzie-app.workers.dev";
const PING_EVERY: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Conn {
    #[default]
    Off,
    Connecting,
    Online,
    /// Could not reach the relay; retrying.
    Retrying,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Position when sent (ms) and how old that is, in relay time.
    pub pos: u64,
    pub dur: u64,
    /// Local time the position was valid.
    pub at: Option<Instant>,
}

impl Track {
    /// Position now, moving on while it plays.
    pub fn position_ms(&self) -> u64 {
        let p = match (self.playing, self.at) {
            (true, Some(at)) => self.pos + at.elapsed().as_millis() as u64,
            _ => self.pos,
        };
        if self.dur > 0 {
            p.min(self.dur)
        } else {
            p
        }
    }

    pub fn key(&self) -> String {
        format!("{}\u{1}{}", self.artist, self.title)
    }
}

/// Built-in companion bot code (always online, cannot be removed).
pub const BOT_CODE: &str = "FUZ-BOT001";

pub fn is_bot(code: &str) -> bool {
    code == BOT_CODE
}

pub fn bot_friend() -> Friend {
    Friend { code: BOT_CODE.into(), name: "Fuzzie Bot".into(), skin: 2, acc: 0, online: true, np: None, is_bot: true }
}

#[derive(Clone, Debug, Default)]
pub struct Friend {
    pub code: String,
    pub name: String,
    pub skin: usize,
    pub acc: usize,
    pub online: bool,
    pub np: Option<Track>,
    pub is_bot: bool,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub conn: Conn,
    pub code: String,
    pub name: String,
    pub friends: Vec<Friend>,
    /// Friend requests: (code, name).
    pub incoming: Vec<(String, String)>,
    pub outgoing: Vec<(String, String)>,
    /// Last error from the relay and when it came.
    pub error: Option<(String, Instant)>,
    /// The friend we listen along with.
    pub along: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Identity {
    code: String,
    key: String,
}

/// What we tell the relay about ourselves (resent after every reconnect).
#[derive(Clone, PartialEq, Default)]
struct Profile {
    name: String,
    skin: usize,
    acc: usize,
    np: Option<(String, String, bool, u64, u64)>,
    /// When `np` was sent.
    np_at: Option<Instant>,
    share: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static NOTICES: OnceLock<Queue> = OnceLock::new();
static PROFILE: Mutex<Profile> = Mutex::new(Profile { name: String::new(), skin: 0, acc: 0, np: None, np_at: None, share: true });
static OUT: Mutex<Option<Sender<String>>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(false);
static CHANGED: AtomicBool = AtomicBool::new(false);

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

fn edit(f: impl FnOnce(&mut State)) {
    if let Ok(mut s) = state().lock() {
        f(&mut s);
    }
    CHANGED.store(true, Ordering::Release);
}

/// A copy of what the UI shows (always includes the built-in Fuzzie Bot).
pub fn snapshot() -> State {
    let mut st = state().lock().map(|s| s.clone()).unwrap_or_default();
    if !st.friends.iter().any(|f| f.is_bot) {
        let pos = st.friends.iter().position(|f| !f.online).unwrap_or(st.friends.len());
        st.friends.insert(pos, bot_friend());
    }
    st
}

/// True once after anything shown changed.
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::AcqRel)
}

fn send(v: Value) {
    if let Some(tx) = OUT.lock().ok().and_then(|o| o.clone()) {
        let _ = tx.send(v.to_string());
    }
}

pub fn add(code: &str) {
    send(json!({"t": "add", "code": code.trim()}));
}
pub fn accept(code: &str) {
    send(json!({"t": "accept", "code": code}));
}
pub fn decline(code: &str) {
    send(json!({"t": "decline", "code": code}));
}
pub fn remove(code: &str) {
    if is_bot(code) {
        return;
    }
    if snapshot().along.as_deref() == Some(code) {
        stop_along();
    }
    send(json!({"t": "remove", "code": code}));
}

pub fn poke(code: &str) {
    crate::stats::record_poke(true);
    if is_bot(code) {
        bot_reply(false);
        return;
    }
    send(json!({"t": "poke", "to": code}));
}

pub fn kiss(code: &str) {
    crate::stats::record_kiss(true);
    if is_bot(code) {
        bot_reply(true);
        return;
    }
    send(json!({"t": "kiss", "to": code}));
}

/// Fuzzie Bot replies back with a poke or kiss after a short, natural delay.
fn bot_reply(is_kiss: bool) {
    let Some(q) = NOTICES.get().cloned() else { return };
    let _ = std::thread::Builder::new().name("bot-reply".into()).stack_size(256 * 1024).spawn(move || {
        std::thread::sleep(Duration::from_millis(1100));
        if is_kiss {
            crate::stats::record_kiss(false);
            notice(&q, "Fuzzie Bot sent you a kiss!".into(), "Beep boop \u{2665}".into(), Some(crate::mascot::Look::bot()));
        } else {
            crate::stats::record_poke(false);
            notice(&q, "Fuzzie Bot poked you back!".into(), "Beep boop!".into(), Some(crate::mascot::Look::bot()));
        }
    });
}

/// Starts or stops listening along with `code`.
pub fn toggle_along(code: &str) {
    if snapshot().along.as_deref() == Some(code) {
        stop_along();
    } else {
        stop_along();
        edit(|s| s.along = Some(code.to_string()));
        send(json!({"t": "join", "to": code}));
    }
}

pub fn stop_along() {
    let mut was = None;
    edit(|s| was = s.along.take());
    if let Some(code) = was {
        send(json!({"t": "leave", "to": code}));
    }
}

/// Name, look and sharing choice; sent when they change.
pub fn set_profile(name: &str, skin: usize, acc: usize, share: bool) {
    let Ok(mut p) = PROFILE.lock() else { return };
    let name = clean_name(name);
    if p.name == name && p.skin == skin && p.acc == acc && p.share == share {
        return;
    }
    let share_changed = p.share != share;
    p.name = name;
    p.skin = skin;
    p.acc = acc;
    p.share = share;
    let hello = hello(&p);
    let np = share_changed.then(|| np_msg(&p));
    drop(p);
    send(hello);
    if let Some(m) = np {
        send(m);
    }
}

/// What we play now (None when nothing plays); sent on track or play/pause
/// changes and when the position jumps.
pub fn set_playing(np: Option<(String, String, bool, u64, u64)>) {
    let Ok(mut p) = PROFILE.lock() else { return };
    let same = match (&p.np, &np, p.np_at) {
        (Some(a), Some(b), Some(at)) => {
            // Where the last sent position should be by now.
            let expect = if a.2 { a.3 + at.elapsed().as_millis() as u64 } else { a.3 };
            a.0 == b.0 && a.1 == b.1 && a.2 == b.2 && (b.3 as i64 - expect as i64).abs() < 3000
        }
        (None, None, _) => true,
        _ => false,
    };
    if same {
        return;
    }
    p.np = np;
    p.np_at = Some(Instant::now());
    let m = np_msg(&p);
    drop(p);
    send(m);
}

fn hello(p: &Profile) -> Value {
    json!({"t": "hello", "name": p.name, "skin": p.skin, "acc": p.acc})
}

fn np_msg(p: &Profile) -> Value {
    match (&p.np, p.share) {
        (Some((title, artist, playing, pos, dur)), true) => {
            json!({"t": "np", "np": {"title": title, "artist": artist, "playing": playing, "pos": pos, "dur": dur}})
        }
        _ => json!({"t": "np", "np": null}),
    }
}

pub fn clean_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(24).collect()
}

/// "fuz a4g 7b9" → "FUZ-A4G7B9"; None if it cannot be a friend code.
pub fn clean_code(s: &str) -> Option<String> {
    let mut t: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    if t.len() == 9 && t.starts_with("FUZ") {
        t.drain(..3);
    }
    let ok = t.len() == 6 && t.chars().all(|c| "23456789ABCDEFGHJKMNPQRSTUVWXYZ".contains(c));
    ok.then(|| format!("FUZ-{t}"))
}

/// Turns the connection on or off (Settings → Social → "Online").
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Release);
    if !on {
        stop_along();
    }
}

/// Starts the connection thread; notices go to `notices`.
pub fn start(notices: Queue) {
    let _ = NOTICES.set(notices.clone());
    let _ = std::thread::Builder::new().name("social".into()).stack_size(512 * 1024).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        run(notices);
    });
}

fn relay() -> String {
    std::env::var("FUZZIE_RELAY").unwrap_or_else(|_| RELAY.to_string()).trim_end_matches('/').to_string()
}

fn id_path() -> std::path::PathBuf {
    // A test relay gets its own account.
    let name = if std::env::var("FUZZIE_RELAY").is_ok() { "social-test.json" } else { "social.json" };
    crate::settings::data_dir().join(name)
}

fn identity() -> Option<Identity> {
    if let Some(id) = std::fs::read(id_path()).ok().and_then(|b| serde_json::from_slice::<Identity>(&b).ok()) {
        return Some(id);
    }
    let name = PROFILE.lock().ok()?.name.clone();
    let body = post(&format!("{}/register", relay()), &json!({ "name": name }).to_string())?;
    let id: Identity = serde_json::from_str(&body).ok()?;
    let _ = std::fs::create_dir_all(crate::settings::data_dir());
    std::fs::write(id_path(), serde_json::to_vec(&json!({"code": id.code, "key": id.key})).ok()?).ok()?;
    Some(id)
}

fn run(notices: Queue) {
    let mut wait = Duration::from_secs(2);
    loop {
        if !ENABLED.load(Ordering::Acquire) {
            edit(|s| {
                if s.conn != Conn::Off {
                    *s = State::default();
                }
            });
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        edit(|s| s.conn = if s.conn == Conn::Retrying { Conn::Retrying } else { Conn::Connecting });
        let started = Instant::now();
        let ok = identity().map(|id| session(&id, &notices)).unwrap_or(false);
        if !ENABLED.load(Ordering::Acquire) {
            continue;
        }
        edit(|s| {
            s.conn = Conn::Retrying;
            s.friends.iter_mut().for_each(|f| f.online = false);
        });
        // A connection that lasted resets the back-off.
        if ok && started.elapsed() > Duration::from_secs(30) {
            wait = Duration::from_secs(2);
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(Duration::from_secs(60));
    }
}

/// One connection, until it drops or Social is turned off. True if it connected.
fn session(id: &Identity, notices: &Queue) -> bool {
    let (tx_in, rx_in) = channel::<String>();
    let closed = Arc::new(AtomicBool::new(false));
    let Some((ws, writer)) = open(id, tx_in, closed.clone()) else { return false };

    let (tx_out, rx_out) = channel::<String>();
    if let Ok(mut o) = OUT.lock() {
        *o = Some(tx_out);
    }
    let write = |text: &str| -> bool { writer.WriteString(&HSTRING::from(text)).is_ok() && writer.StoreAsync().and_then(|op| op.join()).is_ok() };
    let p = PROFILE.lock().map(|p| p.clone()).unwrap_or_default();
    let mut alive = write(&hello(&p).to_string()) && write(&np_msg(&p).to_string());
    if let Some(code) = snapshot().along {
        alive &= write(&json!({"t": "join", "to": code}).to_string());
    }
    edit(|s| {
        s.conn = Conn::Online;
        s.code = id.code.clone();
    });

    let mut last_ping = Instant::now();
    while alive && !closed.load(Ordering::Acquire) && ENABLED.load(Ordering::Acquire) {
        while let Ok(m) = rx_in.try_recv() {
            handle(&m, notices);
        }
        match rx_out.recv_timeout(Duration::from_millis(150)) {
            Ok(m) => alive = write(&m),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_ping.elapsed() > PING_EVERY {
            alive = write("ping");
            last_ping = Instant::now();
        }
    }
    if let Ok(mut o) = OUT.lock() {
        *o = None;
    }
    let _ = ws.CloseWithStatus(1000, &HSTRING::from("bye"));
    true
}

fn open(id: &Identity, tx_in: Sender<String>, closed: Arc<AtomicBool>) -> Option<(MessageWebSocket, DataWriter)> {
    let ws = MessageWebSocket::new().ok()?;
    ws.Control().ok()?.SetMessageType(SocketMessageType::Utf8).ok()?;
    ws.SetRequestHeader(&HSTRING::from("Authorization"), &HSTRING::from(format!("Bearer {}:{}", id.code, id.key))).ok()?;
    ws.MessageReceived(&TypedEventHandler::<MessageWebSocket, MessageWebSocketMessageReceivedEventArgs>::new(move |_, args| {
        if let Ok(args) = args.ok() {
            if let Ok(r) = args.GetDataReader() {
                let _ = r.SetUnicodeEncoding(UnicodeEncoding::Utf8);
                if let Ok(s) = r.UnconsumedBufferLength().and_then(|n| r.ReadString(n)) {
                    let _ = tx_in.send(s.to_string());
                }
            }
        }
        Ok(())
    }))
    .ok()?;
    ws.Closed(&TypedEventHandler::<IWebSocket, WebSocketClosedEventArgs>::new(move |_, _| {
        closed.store(true, Ordering::Release);
        Ok(())
    }))
    .ok()?;
    let url = relay().replacen("http", "ws", 1) + "/ws";
    ws.ConnectAsync(&Uri::CreateUri(&HSTRING::from(url)).ok()?).ok()?.join().ok()?;
    let writer = DataWriter::CreateDataWriter(&ws.OutputStream().ok()?).ok()?;
    writer.SetUnicodeEncoding(UnicodeEncoding::Utf8).ok()?;
    Some((ws, writer))
}

/// Removes this PC's friends account from the relay (best effort, a few seconds at most).
pub fn delete_account() {
    let Some(id) = std::fs::read(id_path()).ok().and_then(|b| serde_json::from_slice::<Identity>(&b).ok()) else { return };
    let url = format!("{}/delete", relay());
    let (tx, rx) = channel();
    let _ = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let ok = (|| -> Option<()> {
            let client = HttpClient::new().ok()?;
            let msg = windows::Web::Http::HttpRequestMessage::Create(&windows::Web::Http::HttpMethod::Post().ok()?, &Uri::CreateUri(&HSTRING::from(url)).ok()?).ok()?;
            msg.Headers().ok()?.TryAppendWithoutValidation(&HSTRING::from("Authorization"), &HSTRING::from(format!("Bearer {}:{}", id.code, id.key))).ok()?;
            client.SendRequestAsync(&msg).ok()?.join().ok()?;
            Some(())
        })();
        let _ = tx.send(ok);
    });
    let _ = rx.recv_timeout(Duration::from_secs(6));
}

fn post(url: &str, body: &str) -> Option<String> {
    let client = HttpClient::new().ok()?;
    let content = HttpStringContent::CreateFromStringWithEncodingAndMediaType(&HSTRING::from(body), UnicodeEncoding::Utf8, &HSTRING::from("application/json")).ok()?;
    let resp = client.PostAsync(&Uri::CreateUri(&HSTRING::from(url)).ok()?, &content).ok()?.join().ok()?;
    if !resp.IsSuccessStatusCode().ok()? {
        return None;
    }
    Some(resp.Content().ok()?.ReadAsStringAsync().ok()?.join().ok()?.to_string())
}

fn notice(q: &Queue, title: String, body: String, fox: Option<crate::mascot::Look>) {
    if let Ok(mut q) = q.lock() {
        q.push_back(Notice { app: "Fuzzie friends".into(), title, body, fox });
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

/// One message from the relay.
fn handle(text: &str, q: &Queue) {
    if text == "pong" {
        return;
    }
    let Ok(m) = serde_json::from_str::<Value>(text) else { return };
    match m["t"].as_str() {
        Some("state") => {
            let now = n(&m["now"]);
            let friends: Vec<Friend> = m["friends"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|f| Friend {
                            code: s(&f["code"]),
                            name: s(&f["name"]),
                            skin: n(&f["skin"]) as usize,
                            acc: n(&f["acc"]) as usize,
                            online: f["online"].as_bool() == Some(true),
                            is_bot: false,
                            np: f["np"].as_object().map(|_| {
                                let np = &f["np"];
                                // How long ago the position was sampled, by the relay's clock.
                                let age = now.saturating_sub(n(&np["at"])).min(60 * 60 * 1000);
                                Track {
                                    title: s(&np["title"]),
                                    artist: s(&np["artist"]),
                                    playing: np["playing"].as_bool() == Some(true),
                                    pos: n(&np["pos"]),
                                    dur: n(&np["dur"]),
                                    at: Instant::now().checked_sub(Duration::from_millis(age)),
                                }
                            }),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let pairs = |v: &Value| -> Vec<(String, String)> { v.as_array().map(|a| a.iter().map(|x| (s(&x["code"]), s(&x["name"]))).collect()).unwrap_or_default() };
            let incoming = pairs(&m["incoming"]);
            let mut new_requests = Vec::new();
            let mut new_friends = Vec::new();
            edit(|st| {
                for (c, name) in &incoming {
                    if !st.incoming.iter().any(|(x, _)| x == c) && st.conn == Conn::Online && !st.code.is_empty() && st.name_known() {
                        new_requests.push(name.clone());
                    }
                }
                for f in &friends {
                    if st.outgoing.iter().any(|(x, _)| *x == f.code) {
                        new_friends.push(f.name.clone());
                    }
                }
                st.name = s(&m["me"]["name"]);
                st.code = s(&m["me"]["code"]);
                st.friends = friends;
                // Online first, then those playing something, then by name.
                st.friends.sort_by_key(|f| (!f.online, f.np.is_none(), f.name.to_lowercase()));
                st.incoming = incoming;
                st.outgoing = pairs(&m["outgoing"]);
                // Listening along ends when that friend goes away.
                if let Some(c) = &st.along {
                    if !st.friends.iter().any(|f| &f.code == c && f.online) {
                        st.along = None;
                    }
                }
            });
            for name in new_requests {
                notice(q, format!("{name} wants to be friends"), "Accept in Settings \u{2192} Social".into(), None);
            }
            for name in new_friends {
                notice(q, format!("{name} accepted your request"), String::new(), None);
            }
        }
        Some("poke") => {
            crate::stats::record_poke(false);
            notice(q, format!("{} poked you", s(&m["name"])), String::new(), Some(crate::mascot::Look::new(0, n(&m["skin"]) as usize, n(&m["acc"]) as usize, 0)));
        }
        Some("kiss") => {
            crate::stats::record_kiss(false);
            notice(q, format!("{} sent you a kiss", s(&m["name"])), String::new(), Some(crate::mascot::Look::new(0, n(&m["skin"]) as usize, n(&m["acc"]) as usize, 0)));
        }
        Some("join") => notice(q, format!("{} is listening along", s(&m["name"])), String::new(), None),
        Some("leave") => notice(q, format!("{} stopped listening along", s(&m["name"])), String::new(), None),
        Some("error") => {
            let msg = s(&m["msg"]);
            edit(|st| st.error = Some((msg, Instant::now())));
        }
        _ => {}
    }
}

/// Sample friends for `--snapshot` pictures.
pub fn demo() {
    let t = |title: &str, artist: &str| Some(Track { title: title.into(), artist: artist.into(), playing: true, pos: 60_000, dur: 200_000, at: Some(Instant::now()) });
    let f = |name: &str, code: &str, skin, acc, online, np| Friend { code: code.into(), name: name.into(), skin, acc, online, np, is_bot: false };
    edit(|s| {
        s.conn = Conn::Online;
        s.code = "FUZ-A4G7B9".into();
        s.name = "Ayla".into();
        s.friends = vec![
            f("Bora", "FUZ-VS7DEF", 5, 3, true, t("Midnight City", "M83")),
            f("Deniz", "FUZ-K2Q8MN", 12, 0, true, None),
            f("Ece", "FUZ-P9X3RT", 30, 31, false, None),
            f("Mert", "FUZ-H7W2JC", 2, 1, false, None),
        ];
        s.incoming = vec![("FUZ-Z3Y4XW".into(), "Kaan".into())];
        s.outgoing = vec![("FUZ-M4N5PQ".into(), "Lale".into())];
        s.along = Some("FUZ-VS7DEF".into());
    });
}

/// The friend we listen along with, as last heard.
pub fn along_friend() -> Option<Friend> {
    let s = state().lock().ok()?;
    let code = s.along.as_ref()?;
    s.friends.iter().find(|f| &f.code == code).cloned()
}

impl State {
    /// The first state message has arrived (so requests in it are not "new").
    fn name_known(&self) -> bool {
        !self.name.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(clean_code("fuz-a4g7b9").as_deref(), Some("FUZ-A4G7B9"));
        assert_eq!(clean_code(" a4g 7b9 ").as_deref(), Some("FUZ-A4G7B9"));
        assert_eq!(clean_code("FUZ-A4G7B0"), None);
        assert_eq!(clean_code("FUZ-A4G7"), None);
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("  Ayla\n "), "Ayla");
        assert_eq!(clean_name(&"x".repeat(40)).len(), 24);
    }

    #[test]
    fn track_position() {
        let t = Track { playing: false, pos: 5000, dur: 4000, ..Default::default() };
        assert_eq!(t.position_ms(), 4000);
    }
}

//! The notch window: placement, mouse handling, pages, animation and the message loop.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{w, Error, Result, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::System::DataExchange::RemoveClipboardFormatListener;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetLastInputInfo, ReleaseCapture, SetCapture, TrackMouseEvent, LASTINPUTINFO,
    TME_LEAVE, TRACKMOUSEEVENT, VK_CAPITAL, VK_LBUTTON,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowRect, SW_HIDE,
    LoadCursorW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetTimer,
    ShowWindow, TranslateMessage, IDC_ARROW, MSG, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_APP, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use crate::dragdrop::{self, DragEvent};
use crate::frameclock::FrameClock;
use crate::media::{self, Cmd, MediaInfo};
use crate::render::{self, approach, glide, Edge, Hit, Hud, LyricsStatus, Renderer, Scene, Theme, CANVAS_W, CLIP_ROWS, PAGE_COUNT};
use crate::settings::{self, IdleStyle, NotchStyle, OpenMode, Placement, Settings};
use crate::settings_win::{self, Key};
use crate::{audio, autostart, clipboard, deskfox, lyrics, shelf, stats, tray};

// Not exported by the WindowsAndMessaging module in windows 0.62.
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
const WM_CAPTURECHANGED: u32 = 0x0215;
const TIMER_CLOCK: usize = 2;
/// Volume / Caps Lock / hover polling and live redraws while music plays.
const TIMER_POLL: usize = 3;
/// Frames come from the vblank clock: ~60 fps while something eases, ~30 fps
/// while only the mascot (or equaliser) moves, none when the notch is still.
/// (WM_APP + 1 is media, + 2 the tray icon.)
const WM_FRAME: u32 = WM_APP + 4;
const FRAME_FAST: u32 = 16;
const FRAME_LIVE: u32 = 33;
/// How long click feedback (a ripple, "Copied") stays.
const FLASH_SECS: f32 = 1.1;
/// How long scrolled lyrics stay put before following the singer again.
const LYRIC_SCROLL_HOLD: Duration = Duration::from_secs(4);
/// `--snapshot DIR`: render each view to PNG files after start-up, then quit.
const TIMER_SNAPSHOT: usize = 5;
/// Ends the see-through spell of a right-click.
const TIMER_GHOST: usize = 6;
/// Opacity while see-through.
pub const GHOST_ALPHA: u8 = 105;
const HUD_TIME: Duration = Duration::from_millis(1600);
/// How long a notification stays in the notch.
const NOTICE_TIME: Duration = Duration::from_millis(5000);

struct App {
    hwnd: HWND,
    renderer: Renderer,
    settings: Settings,
    media: Arc<Mutex<MediaInfo>>,
    media_tx: mpsc::Sender<Cmd>,
    lyrics: lyrics::Shared,
    audio: Option<audio::Watcher>,
    shelf: shelf::Shelf,
    clips: clipboard::History,
    caps: Option<bool>,
    pos: POINT,
    /// Desired state (true = expanded) and the animated value 0..1.
    open: bool,
    anim: f32,
    page: usize,
    /// Animated page position (slides toward `page`).
    page_pos: f32,
    lyrics_mode: bool,
    /// Lines the reader scrolled the lyrics by (target, and eased), and when;
    /// after a few seconds the view follows the singer again.
    lyric_scroll: f32,
    lyric_scroll_pos: f32,
    lyric_scrolled_at: Option<Instant>,
    /// Right-clicked: see-through and letting clicks pass until the timer ends it.
    ghost: bool,
    /// The last frame was part of an animation (when it ends, one clean full frame).
    was_fast: bool,
    /// Animated music layout: 0 two columns, 1 wide (lyrics off).
    wide: f32,
    clip_scroll: usize,
    drag_over: bool,
    /// HUD being shown, until when, and its animated value 0..1.
    hud: Option<Hud>,
    /// New Windows notifications waiting to be shown.
    notices: crate::notify::Queue,
    hud_until: Option<Instant>,
    hud_anim: f32,
    /// True while any tween is still moving (frames run at FRAME_FAST).
    animating: bool,
    last_tick: Instant,
    tracking_leave: bool,
    hits: Vec<(D2D_RECT_F, Hit)>,
    /// The hit area under the pointer, and the eased hover highlight per area.
    hot: Option<Hit>,
    hover: Vec<(Hit, f32)>,
    /// The area just clicked and its feedback left (1 → 0).
    flash: Option<(Hit, f32)>,
    /// Dragging the volume bar (its rectangle).
    vol_drag: Option<D2D_RECT_F>,
    /// The volume slider beside the speaker: 0 folded, 1 open. It opens while
    /// the speaker or slider is hovered and folds a moment after.
    vol_open: f32,
    vol_seen: Option<Instant>,
    /// Mouse wheel over the page: accumulated delta, last event and last page flip.
    wheel_acc: i32,
    wheel_at: Instant,
    flipped_at: Instant,
    /// Where the left button went down, and on which hit area.
    press: Option<((f32, f32), usize)>,
    /// Dragging the notch in "Custom" placement: cursor and notch position at the start.
    moving: Option<(POINT, (i32, i32))>,
    /// The mascot is napping (no input for a while).
    sleeping: bool,
    /// Hidden because a full-screen app is in front.
    hidden: bool,
    /// Interval of the frame clock in ms (0 = off).
    frame_ms: u32,
    clock: FrameClock,
    /// For adding listening time between polls, and saving stats now and then.
    last_poll: Instant,
    stats_saved: Instant,
    started: Instant,
    /// Listening along: the friend's track we last opened, and the one we
    /// already jumped to their position in.
    along_opened: String,
    along_synced: String,
    /// Friends page: 0 the list, 1 add a friend, 2 requests; the friend code
    /// being typed, whether the box has the keyboard, and a short note.
    friend_view: u8,
    friend_tab: u8,
    friend_input: String,
    friend_typing: bool,
    friend_note: Option<(String, Instant)>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app state. Returns None if state is missing or already
/// borrowed (re-entrant messages fall through to the default handler).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn taskbar_created() -> u32 {
    static MSG_ID: OnceLock<u32> = OnceLock::new();
    *MSG_ID.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

pub fn run(settings: Settings) -> Result<()> {
    let show_lyrics = settings.show_lyrics;
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let class = w!("FuzzieNotchWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(Error::from_thread());
        }
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("Fuzzie"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;

        let renderer = Renderer::new(dpi_scale(hwnd, &settings))?;
        let media = Arc::new(Mutex::new(MediaInfo::default()));
        let lyrics = lyrics::Shared::default();
        let media_tx = media::spawn(hwnd, media.clone(), lyrics.clone());
        let audio = audio::Watcher::new().map_err(|e| eprintln!("audio watcher failed: {e}")).ok();
        let (shelf, clips) = shelf::load(!settings.start_with_empty_shelf, settings.remember_after_restart);
        if settings.clipboard_history {
            clipboard::History::start(hwnd);
        }
        if let Err(e) = dragdrop::register(hwnd, on_drag) {
            eprintln!("drag and drop unavailable: {e}");
        }
        if let Err(e) = tray::add(hwnd) {
            eprintln!("tray icon failed: {e}");
        }
        taskbar_created();

        let mut app = App {
            hwnd,
            renderer,
            settings,
            media,
            media_tx,
            lyrics,
            audio,
            shelf,
            clips,
            caps: None,
            pos: POINT::default(),
            open: false,
            anim: 0.0,
            page: render::PAGE_MUSIC,
            page_pos: render::PAGE_MUSIC as f32,
            lyrics_mode: show_lyrics,
            lyric_scroll: 0.0,
            lyric_scroll_pos: 0.0,
            lyric_scrolled_at: None,
            ghost: false,
            was_fast: false,
            wide: if show_lyrics { 0.0 } else { 1.0 },
            clip_scroll: 0,
            drag_over: false,
            hud: None,
            notices: crate::notify::start(),
            hud_until: None,
            hud_anim: 0.0,
            animating: false,
            last_tick: Instant::now(),
            tracking_leave: false,
            hits: Vec::new(),
            hot: None,
            hover: Vec::new(),
            flash: None,
            vol_drag: None,
            vol_open: 0.0,
            vol_seen: None,
            wheel_acc: 0,
            wheel_at: Instant::now(),
            flipped_at: Instant::now(),
            press: None,
            moving: None,
            sleeping: false,
            hidden: false,
            frame_ms: 0,
            clock: FrameClock::new(hwnd, WM_FRAME),
            last_poll: Instant::now(),
            stats_saved: Instant::now(),
            started: Instant::now(),
            along_opened: String::new(),
            along_synced: String::new(),
            friend_view: 0,
            friend_tab: 0,
            friend_input: String::new(),
            friend_typing: false,
            friend_note: None,
        };
        app.redraw();
        NOTCH.store(hwnd.0 as isize, std::sync::atomic::Ordering::Relaxed);
        APP.with(|a| *a.borrow_mut() = Some(app));

        SetTimer(Some(hwnd), TIMER_CLOCK, 1000, None);
        SetTimer(Some(hwnd), TIMER_POLL, 100, None);
        stats::init();
        with_app(|a| {
            deskfox::sync(&a.settings);
            crate::updater::set_enabled(a.settings.auto_update);
            if !std::env::args().any(|x| x == "--snapshot") {
                crate::updater::start(a.hwnd);
                social_sync(&a.settings);
                crate::social::start(a.notices.clone());
            }
        });
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if std::env::args().any(|a| a == "--snapshot") {
            crate::social::demo();
            SetTimer(Some(hwnd), TIMER_SNAPSHOT, 5000, None);
        }
        // `--settings [page] [scroll] [years]` opens the settings window (for testing).
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--settings") {
            let num = |k: usize| args.get(i + k).and_then(|v| v.parse::<f32>().ok());
            match num(1) {
                Some(page) => {
                    let tab = if args.iter().any(|a| a == "acc") { 1 } else if args.iter().any(|a| a == "options") { 2 } else { 0 };
                    settings_win::open_at(page as usize, num(2).unwrap_or(0.0), args.iter().any(|a| a == "years"), tab)
                }
                None => settings_win::open(),
            }
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn dpi_scale(hwnd: HWND, settings: &Settings) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    dpi as f32 / 96.0 * settings.scale()
}

impl App {
    fn edge(&self) -> Edge {
        let st = &self.settings;
        if st.style == NotchStyle::Floating || st.placement == Placement::Custom {
            Edge::None
        } else if st.placement.is_bottom() {
            Edge::Bottom
        } else {
            Edge::Top
        }
    }

    /// Whether the notch sits on the lower half of its screen.
    fn lower_half(&self) -> bool {
        match self.edge() {
            Edge::Bottom => true,
            Edge::Top => false,
            Edge::None => unsafe {
                use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                if !GetMonitorInfoW(MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
                    return false;
                }
                let k = self.renderer.scale();
                let body = &self.renderer.body;
                let cy = self.pos.y as f32 + (body.top + body.bottom) / 2.0 * k;
                cy > (mi.rcMonitor.top + mi.rcMonitor.bottom) as f32 / 2.0
            },
        }
    }

    /// The side of the screen an attached corner notch is glued to: -1 left, 1 right, 0 none.
    fn side(&self) -> i8 {
        if self.edge() == Edge::None {
            return 0;
        }
        match self.settings.placement {
            Placement::TopLeft | Placement::BottomLeft => -1,
            Placement::TopRight | Placement::BottomRight => 1,
            _ => 0,
        }
    }

    /// Places the window so the notch body sits where the settings say.
    /// Runs after every draw because the body size changes while animating.
    fn position(&mut self) {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe {
            let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTOPRIMARY);
            if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
                return;
            }
        }
        let (m, work) = (mi.rcMonitor, mi.rcWork);
        let s = self.renderer.scale();
        let b = self.renderer.body;
        let px = |v: f32| (v * s).round() as i32;
        // An edge pressed against the screen side must land on a fully covered
        // pixel, or its half-transparent anti-aliased column shows a gap there:
        // round a left/top edge inwards up, a right/bottom edge inwards down.
        let lo = |v: f32| (v * s - 0.01).ceil() as i32;
        let hi = |v: f32| (v * s + 0.01).floor() as i32;
        let gap = if self.settings.style == NotchStyle::Floating { px(10.0) } else { 0 };
        // Attached corner placements sit flush against the side of the screen too.
        let side = if self.settings.style == NotchStyle::Floating { px(16.0) } else { 0 };
        let canvas_w = self.renderer.width;
        let (x, y) = match self.settings.placement {
            Placement::TopCenter => (m.left + (m.right - m.left - canvas_w) / 2, m.top + gap),
            Placement::TopLeft => (m.left + side - lo(b.left), m.top + gap),
            Placement::TopRight => (m.right - side - hi(b.right), m.top + gap),
            Placement::BottomLeft => (work.left + side - lo(b.left), work.bottom - gap - hi(b.bottom)),
            Placement::BottomRight => (work.right - side - hi(b.right), work.bottom - gap - hi(b.bottom)),
            Placement::Custom => {
                let (cx, cy) = self.custom_pos(&m);
                (cx - px(CANVAS_W / 2.0), cy)
            }
        };
        self.pos = POINT { x, y };
    }

    /// Top-centre of the notch body in "Custom" placement.
    fn custom_pos(&self, m: &windows::Win32::Foundation::RECT) -> (i32, i32) {
        if self.settings.custom_x < 0 {
            ((m.left + m.right) / 2, m.top + 40)
        } else {
            (self.settings.custom_x, self.settings.custom_y)
        }
    }

    /// Advances every tween by the time since the last frame.
    fn tick(&mut self) {
        let now = Instant::now();
        let real = now.duration_since(self.last_tick).as_secs_f32();
        self.last_tick = now;
        // After a still period the first step is one frame, not the whole pause.
        let dt = if self.animating { real.min(0.05) } else { 1.0 / 60.0 };
        let mut moving = false;
        // Opening, closing and the HUD glide at a steady pace; the renderer eases
        // them in and out, so they start and stop softly.
        moving |= glide(&mut self.anim, if self.open { 1.0 } else { 0.0 }, dt, 0.3);
        moving |= glide(&mut self.hud_anim, if self.hud_until.is_some() { 1.0 } else { 0.0 }, dt, 0.25);
        moving |= approach(&mut self.page_pos, self.page as f32, dt, 12.0);
        // Scrolled lyrics drift back to the sung line after a few idle seconds.
        if self.lyric_scrolled_at.is_some_and(|t| now.duration_since(t) > LYRIC_SCROLL_HOLD) {
            self.lyric_scrolled_at = None;
            self.lyric_scroll = 0.0;
        }
        moving |= approach(&mut self.lyric_scroll_pos, self.lyric_scroll, dt, 14.0);
        // Lyrics on/off: a steady half-second glide, eased in the layout.
        moving |= glide(&mut self.wide, if self.lyrics_mode { 0.0 } else { 1.0 }, dt, 0.5);
        let hot = self.hot;
        if matches!(hot, Some(Hit::Volume | Hit::VolumeBar)) || self.vol_drag.is_some() {
            self.vol_seen = Some(now);
        }
        let vol = self.vol_seen.is_some_and(|t| now.duration_since(t) < Duration::from_millis(600));
        if !vol {
            self.vol_seen = None;
        }
        moving |= approach(&mut self.vol_open, if vol { 1.0 } else { 0.0 }, dt, 14.0);
        // Keep frames coming until the fold delay runs out.
        moving |= vol && self.vol_open > 0.0;
        if let Some(h) = hot {
            if !self.hover.iter().any(|e| e.0 == h) {
                self.hover.push((h, 0.0));
            }
        }
        for e in &mut self.hover {
            moving |= approach(&mut e.1, if Some(e.0) == hot { 1.0 } else { 0.0 }, dt, 18.0);
        }
        self.hover.retain(|e| e.1 > 0.0 || Some(e.0) == hot);
        // Click feedback runs out over FLASH_SECS.
        if let Some((h, v)) = self.flash {
            let v = v - dt / FLASH_SECS;
            self.flash = (v > 0.0).then_some((h, v));
            moving = true;
        }
        self.animating = moving;
    }

    /// Starts the fast frame timer; the tweens move on the next frames.
    fn start_anim(&mut self) {
        if !self.animating {
            self.animating = true;
            self.last_tick = Instant::now();
        }
        self.set_frame(FRAME_FAST);
    }

    fn set_frame(&mut self, ms: u32) {
        self.frame_ms = ms;
        self.clock.set(ms);
    }

    fn redraw(&mut self) {
        if self.hidden {
            // Nothing to show behind a full-screen app: no frames at all.
            self.set_frame(0);
            return;
        }
        let t_start = Instant::now();
        self.tick();
        let media = self.media.lock().unwrap().clone();
        let social = crate::social::snapshot();
        let position_ms = media.position_ms();
        // Show each line slightly early so it does not lag the singer.
        let at = position_ms + 250;
        let (lyrics_status, lyric_lines, lyric_index, lyric_t) = {
            let state = self.lyrics.lock().unwrap();
            match state.current.as_ref().filter(|l| l.key == media.key()) {
                None => (LyricsStatus::Loading, None, -1, 1.0),
                Some(l) if !l.synced || l.lines.is_empty() => (LyricsStatus::Missing, None, -1, 1.0),
                Some(l) => {
                    let i = l.index_at(at);
                    let t = if i >= 0 { (at - l.lines[i as usize].0) as f32 / 450.0 } else { 1.0 };
                    (LyricsStatus::Found, Some(l.lines.clone()), i, t.min(1.0))
                }
            }
        };
        let lyric_lines: &[(u64, String)] = lyric_lines.as_deref().map_or(&[], |v| v.as_slice());
        let (clock, date) = clock_and_date(&self.settings);
        let s = &self.settings;
        let theme = Theme {
            background: settings::parse_hex(&s.background, [0.0; 3]),
            opacity: s.opacity(),
            accent: settings::parse_hex(&s.accent, [0.78, 0.0, 0.94]),
            style: s.bg_style,
            grad_strength: s.gradient_strength.clamp(0.0, 1.0),
            grad_soft: s.gradient_softness.clamp(0.0, 1.0),
        };
        let scene = Scene {
            open: self.anim,
            page: self.page,
            page_pos: self.page_pos,
            clock: &clock,
            date: &date,
            media: &media,
            position_ms,
            lyric_lines,
            lyric_index,
            lyric_t,
            lyrics_status,
            lyrics_mode: self.lyrics_mode,
            lyric_scroll: self.lyric_scroll_pos,
            low: self.lower_half(),
            social: &social,
            social_on: self.settings.social_on,
            friend_view: self.friend_view,
            friend_tab: self.friend_tab,
            friend_input: &self.friend_input,
            friend_typing: self.friend_typing,
            friend_note: self.friend_note.as_ref().filter(|n| n.1.elapsed().as_secs() < 8).map(|n| n.0.as_str()),
            wide: self.wide,
            volume: self.audio.as_ref().and_then(|a| a.app_state()),
            vol_open: self.vol_open,
            shelf: &self.shelf.files,
            clips: &self.clips.items,
            clip_scroll: self.clip_scroll,
            drag_over: self.drag_over,
            hud: self.hud.as_ref().map(|h| (h, self.hud_anim)),
            time: self.started.elapsed().as_secs_f32(),
            theme,
            edge: self.edge(),
            side: self.side(),
            minimal: s.idle_style == IdleStyle::Minimal,
            visualizer: s.audio_visualizer,
            mascot: s.show_mascot.then_some((s.skin, s.acc, self.sleeping)),
            hover: &self.hover,
            flash: self.flash,
        };
        let t_draw = Instant::now();
        match self.renderer.draw(&scene) {
            Ok(hits) => self.hits = hits,
            Err(e) => {
                // Rebuild the target so one bad frame cannot freeze the notch.
                eprintln!("draw failed: {e}; rebuilding the renderer");
                if let Ok(r) = Renderer::new(dpi_scale(self.hwnd, &self.settings)) {
                    self.renderer = r;
                }
            }
        }
        let t_drawn = Instant::now();
        self.position();
        // Easing and lyric scrolls get ~64 fps; the mascot's idle animation
        // (blinks, ear twitches, breathing, effects) and the equaliser ~32 fps.
        let fast = self.animating || (media.playing && lyric_t < 1.0) || self.renderer.settling.get() || self.vol_drag.is_some();
        // An animation just ended: one more frame that wipes and resends the
        // whole canvas, so no faded leftovers of the open notch can linger.
        let settle = self.was_fast && !fast;
        self.was_fast = fast;
        if settle {
            self.renderer.full_refresh();
        }
        let frame_ms = if fast || settle {
            FRAME_FAST
        } else if self.renderer.live.get() {
            FRAME_LIVE
        } else {
            0
        };
        self.set_frame(frame_ms);
        if let Err(e) = self.renderer.present(self.hwnd, self.pos) {
            eprintln!("present failed: {e}");
        }
        fps_probe([t_start, t_draw, t_drawn, Instant::now()]);
    }

    fn snapshot(&mut self) {
        let args: Vec<String> = std::env::args().collect();
        let Some(dir) = args.iter().position(|a| a == "--snapshot").and_then(|i| args.get(i + 1)) else { return };
        let dir = std::path::PathBuf::from(dir);
        // FUZZIE_DEMO=1: a made-up song instead of what is really playing.
        let demo = std::env::var("FUZZIE_DEMO").is_ok();
        let save = |a: &mut App, name: &str| {
            if demo {
                // Lyrics shown are from a public-domain song (Robert Burns, 1788).
                let m = if name.starts_with("music_wide") {
                    media::MediaInfo::demo("Blinding Lights", "The Weeknd", 200_000)
                } else {
                    media::MediaInfo::demo("Auld Lang Syne", "Traditional", 214_000)
                };
                let lines = ["Should auld acquaintance be forgot,", "and never brought to mind?", "Should auld acquaintance be forgot,", "and auld lang syne?", "For auld lang syne, my dear,"];
                let lines: Vec<(u64, String)> = lines.iter().enumerate().map(|(i, l)| (40_000 + i as u64 * 6_000, l.to_string())).collect();
                a.lyrics.lock().unwrap().current = Some(lyrics::Lyrics { key: m.key(), lines: Arc::new(lines), synced: true });
                *a.media.lock().unwrap() = m;
            }
            a.redraw();
            let (w, h) = (a.renderer.width as u32, a.renderer.height as u32);
            if let Some(png) = crate::image::encode_png(w, h, a.renderer.pixels()) {
                let _ = std::fs::write(dir.join(name), png);
            }
        };
        self.hot = None;
        self.hover.clear();
        self.open = false;
        self.anim = 0.0;
        save(self, "closed.png");
        // A notification in the collapsed notch.
        self.hud = Some(Hud::Notice(crate::notify::Notice { app: "Discord".into(), title: "Ada Lovelace".into(), body: "Are we still on for tonight? I'll bring the notes.".into(), fox: None }));
        self.hud_until = Some(Instant::now() + Duration::from_secs(60));
        self.hud_anim = 1.0;
        save(self, "notice.png");
        self.hud = None;
        self.hud_until = None;
        self.hud_anim = 0.0;
        self.open = true;
        self.anim = 1.0;
        {
            let (id, name) = {
                let m = self.media.lock().unwrap();
                (m.app_id.clone(), m.source.clone())
            };
            if let Some(a) = self.audio.as_mut() {
                a.track_app(&id, &name);
            }
        }
        for (page, name) in [(0, "music.png"), (1, "shelf.png"), (2, "friends.png")] {
            self.page = page;
            self.page_pos = page as f32;
            save(self, name);
        }
        // The friends page: adding a friend (a code half typed) and the requests.
        self.page = 2;
        self.page_pos = 2.0;
        self.friend_view = 1;
        self.friend_input = "FUZ-7KQ".into();
        self.friend_typing = true;
        save(self, "friends_add.png");
        self.friend_view = 2;
        self.friend_typing = false;
        self.friend_input.clear();
        save(self, "friends_requests.png");
        self.friend_tab = 1;
        save(self, "friends_sent.png");
        self.friend_tab = 0;
        self.friend_view = 0;
        // The music page with the volume slider open.
        self.page = 0;
        self.page_pos = 0.0;
        self.hot = Some(Hit::VolumeBar);
        self.vol_open = 1.0;
        self.hover = vec![(Hit::VolumeBar, 1.0)];
        save(self, "music_vol.png");
        self.page = 1;
        // Half-way through sliding from music to shelf, with the pill gliding.
        self.hot = None;
        self.hover.clear();
        self.vol_open = 0.0;
        self.vol_seen = None;
        self.page_pos = 0.5;
        self.animating = false;
        save(self, "slide.png");
        // Half-way into the wide music layout, hovering play and the volume bar.
        self.page = 0;
        self.page_pos = 0.0;
        self.lyrics_mode = false;
        self.wide = 0.5;
        self.hot = Some(Hit::VolumeBar);
        self.vol_open = 1.0;
        self.hover = vec![(Hit::PlayPause, 1.0), (Hit::VolumeBar, 1.0)];
        save(self, "music_half.png");
        // The wide music layout (lyrics off).
        self.hot = None;
        self.hover.clear();
        self.vol_open = 0.0;
        self.vol_seen = None;
        self.wide = 1.0;
        save(self, "music_wide.png");
        // Part-way open: the content must stay inside the growing body.
        self.anim = 0.72;
        self.open = false;
        save(self, "opening.png");
        self.anim = 1.0;
        self.open = true;
        // Corner placements, glued to two edges (not saved to the settings file).
        for (placement, name) in [(Placement::TopRight, "top_right"), (Placement::BottomLeft, "bottom_left")] {
            self.settings.placement = placement;
            self.settings.style = NotchStyle::Attached;
            self.anim = 0.0;
            save(self, &format!("{name}_closed.png"));
            self.anim = 1.0;
            save(self, &format!("{name}_open.png"));
        }
        deskfox::snapshot(&dir, &self.settings);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    fn set_open(&mut self, open: bool) {
        if self.open != open {
            self.open = open;
            if !open {
                self.press = None;
            }
            self.start_anim();
        }
    }

    fn set_page(&mut self, page: usize) {
        let page = page.min(PAGE_COUNT - 1);
        if page != self.page {
            self.page = page;
            self.start_anim();
        }
    }

    fn show_hud(&mut self, hud: Hud) {
        // A notification on screen is not cut short by a volume or caps flash.
        if matches!(self.hud, Some(Hud::Notice(_))) && self.hud_until.is_some() {
            return;
        }
        self.hud = Some(hud);
        self.hud_until = Some(Instant::now() + HUD_TIME);
        self.start_anim();
    }

    /// Shows the next waiting notification once the notch is free for it.
    fn show_notices(&mut self) {
        let mut q = self.notices.lock().unwrap();
        if !self.settings.show_notifications || self.hidden {
            q.clear();
            return;
        }
        let busy = self.hud_until.is_some() && matches!(self.hud, Some(Hud::Notice(_)));
        if busy || self.open {
            return;
        }
        if let Some(n) = q.pop_front() {
            drop(q);
            self.hud = Some(Hud::Notice(n));
            self.hud_until = Some(Instant::now() + NOTICE_TIME);
            self.start_anim();
        }
    }

    /// Shares what plays, follows a friend we listen along with, and says
    /// whether the friends page needs redrawing.
    fn social_tick(&mut self) -> bool {
        let changed = crate::social::take_changed();
        let (np, mine, my_pos, playing) = {
            let m = self.media.lock().unwrap();
            let np = (m.available && !m.title.is_empty()).then(|| (m.title.clone(), m.artist.clone(), m.playing, m.position_ms(), m.duration_ms));
            (np, m.key(), m.position_ms(), m.playing)
        };
        crate::social::set_playing(np);
        if let Some(t) = crate::social::along_friend().and_then(|f| f.np) {
            let key = t.key();
            if key != mine {
                if self.along_opened != key {
                    self.along_opened = key;
                    self.along_synced.clear();
                    open_in_spotify(&t.artist, &t.title);
                }
            } else if self.along_synced != key && playing {
                self.along_synced = key;
                let want = t.position_ms();
                if (want as i64 - my_pos as i64).abs() > 3000 {
                    let _ = self.media_tx.send(Cmd::Seek(want));
                }
            }
        } else if !self.along_opened.is_empty() {
            self.along_opened.clear();
            self.along_synced.clear();
        }
        changed && self.page == crate::render::PAGE_FRIENDS
    }

    /// Every 100 ms: HUD sources, HUD expiry, hover, and live redraws while music plays.
    fn on_poll(&mut self) {
        let mut redraw = false;
        if let Some(event) = self.audio.as_mut().and_then(|w| w.poll()) {
            redraw = true;
            match event {
                audio::Event::Volume { level, muted } if self.settings.hud_volume => self.show_hud(Hud::Volume { level, muted }),
                audio::Event::Device(name) if self.settings.hud_audio_device => self.show_hud(Hud::Device(name)),
                _ => {}
            }
        }
        let caps = unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0;
        if self.caps.replace(caps).is_some_and(|old| old != caps) && self.settings.hud_capslock {
            self.show_hud(Hud::CapsLock(caps));
        }
        if self.hud_until.is_some_and(|t| Instant::now() >= t) {
            self.hud_until = None;
            self.start_anim();
        }
        self.show_notices();
        redraw |= self.social_tick();
        self.poll_hover();
        if self.anim > 0.0 && self.page == 0 {
            let (id, name) = {
                let m = self.media.lock().unwrap();
                (m.app_id.clone(), m.source.clone())
            };
            if let Some(a) = self.audio.as_mut() {
                let before = a.app_state();
                a.track_app(&id, &name);
                redraw |= a.app_state() != before;
            }
        }
        let playing = self.media.lock().unwrap().playing;
        let now = Instant::now();
        let dt = now.duration_since(self.last_poll).as_secs_f64().min(1.0);
        self.last_poll = now;
        if playing {
            let m = self.media.lock().unwrap();
            if m.source == "Spotify" {
                stats::add_listening(&m.title, &m.artist, dt);
            }
        }
        if now.duration_since(self.stats_saved) >= Duration::from_secs(30) {
            self.stats_saved = now;
            stats::save();
        }
        let st = &self.settings;
        let sleeping = st.nap_enabled && idle_secs() >= st.nap_after_secs && (st.nap_while_music || !playing);
        if sleeping != self.sleeping {
            self.sleeping = sleeping;
            redraw = true;
        }
        let hide = self.settings.hide_in_fullscreen && fullscreen_app_in_front(self.hwnd);
        if hide != self.hidden {
            self.hidden = hide;
            redraw = true; // restarts or stops the mascot's frame timer
            unsafe {
                let _ = ShowWindow(self.hwnd, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
        }
        deskfox::set_state(self.sleeping, self.hidden);
        // While the frame timer runs, it draws soon anyway.
        if (playing || redraw) && self.frame_ms == 0 {
            self.redraw();
        }
    }

    /// Hover detection by cursor position. Mouse messages alone are not enough:
    /// a full-screen app holding mouse capture (games, Blender ...) swallows them.
    fn poll_hover(&mut self) {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let s = self.renderer.scale();
        let b = self.renderer.body;
        let x = (pt.x - self.pos.x) as f32 / s;
        let y = (pt.y - self.pos.y) as f32 / s;
        // A little slack below the body so the edge does not flicker.
        let inside = !self.ghost && x >= b.left && x <= b.right && y >= b.top - 1.0 && y <= b.bottom + 4.0;
        if inside && self.settings.open_mode == OpenMode::Hover {
            self.set_open(true);
        } else if !inside && !self.tracking_leave && self.open && !self.drag_over && self.vol_drag.is_none() && !self.friend_typing {
            self.hot = None;
            self.set_open(false);
        }
    }

    /// Converts client-area mouse coordinates to logical canvas coordinates.
    fn logical(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xffff) as i16 as f32;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
        let s = self.renderer.scale();
        (x / s, y / s)
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.anim < 0.9 {
            return None;
        }
        // Later entries are drawn on top, so search from the end.
        self.hits.iter().rposition(|(r, _)| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom)
    }

    /// Returns shelf paths to drag out, if a drag started (it must run outside the borrow).
    fn on_mouse_move(&mut self, lp: LPARAM) -> Option<Vec<PathBuf>> {
        if !self.tracking_leave {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
            self.tracking_leave = true;
        }
        if self.settings.open_mode == OpenMode::Hover {
            self.set_open(true);
        }
        if self.drag_move() {
            return None;
        }
        let (x, y) = self.logical(lp);
        if let Some(r) = self.vol_drag {
            self.set_volume_at(r, x);
            return None;
        }
        if let Some(((px, py), i)) = self.press {
            let held = unsafe { GetKeyState(VK_LBUTTON.0 as i32) } < 0;
            if held && ((x - px).abs() > 5.0 || (y - py).abs() > 5.0) {
                self.press = None;
                if let Some((_, Hit::ShelfFile(f))) = self.hits.get(i) {
                    return self.shelf.files.get(*f).map(|f| vec![f.path.clone()]);
                }
            }
        }
        let hot = self.hit_at(x, y).map(|i| self.hits[i].1).filter(|&h| h != Hit::ClipList && h != Hit::LyricList);
        if hot != self.hot {
            self.hot = hot;
            self.start_anim();
        }
        None
    }

    fn on_mouse_leave(&mut self) {
        self.tracking_leave = false;
        if self.vol_drag.is_some() {
            return;
        }
        self.hot = None;
        if !self.drag_over && !self.friend_typing {
            self.set_open(false);
        }
        self.start_anim();
    }

    /// Sets the media app's volume from a pointer x over the volume bar `r`.
    fn set_volume_at(&mut self, r: D2D_RECT_F, x: f32) {
        let f = ((x - r.left) / (r.right - r.left).max(1.0)).clamp(0.0, 1.0);
        if let Some(a) = self.audio.as_mut() {
            a.set_app_level(f);
        }
        self.redraw();
    }

    fn on_button_down(&mut self, lp: LPARAM) {
        let (x, y) = self.logical(lp);
        if let Some(i) = self.hit_at(x, y) {
            if let (r, Hit::VolumeBar) = self.hits[i] {
                self.vol_drag = Some(r);
                self.hot = Some(Hit::VolumeBar);
                unsafe {
                    SetCapture(self.hwnd);
                }
                self.set_volume_at(r, x);
                return;
            }
        }
        self.press = self.hit_at(x, y).map(|i| ((x, y), i));
        if self.press.is_none() && self.settings.placement == Placement::Custom {
            let mut pt = POINT::default();
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            unsafe {
                let _ = GetCursorPos(&mut pt);
                SetCapture(self.hwnd);
                let _ = GetMonitorInfoW(MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi);
            }
            self.moving = Some((pt, self.custom_pos(&mi.rcMonitor)));
        }
    }

    /// Moves the notch while it is dragged in "Custom" placement.
    fn drag_move(&mut self) -> bool {
        let Some((start, (sx, sy))) = self.moving else { return false };
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        self.settings.custom_x = sx + pt.x - start.x;
        self.settings.custom_y = (sy + pt.y - start.y).max(0);
        self.redraw();
        true
    }

    /// Returns true if the settings window should open (done outside the borrow).
    fn on_click(&mut self, lp: LPARAM) -> bool {
        if self.vol_drag.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
            self.start_anim();
            return false;
        }
        if self.moving.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
            let _ = self.settings.save();
            return false;
        }
        let (x, y) = self.logical(lp);
        let pressed = self.press.take().map(|(_, i)| i);
        let Some(i) = self.hit_at(x, y) else {
            // A click opens the notch; clicks inside an open notch never close
            // it (moving the pointer away does).
            if self.settings.open_mode == OpenMode::Click && !self.open {
                self.set_open(true);
            }
            return false;
        };
        if pressed.is_some_and(|p| p != i) {
            return false;
        }
        let (r, hit) = self.hits[i];
        if self.friend_typing && !matches!(hit, Hit::CodeBox | Hit::PasteCode | Hit::SendCode) {
            self.friend_typing = false;
        }
        if !matches!(hit, Hit::Seek | Hit::VolumeBar | Hit::ClipList | Hit::LyricList | Hit::Page(_)) {
            self.flash = Some((hit, 1.0));
            self.start_anim();
        }
        let send = |a: &App, c| {
            let _ = a.media_tx.send(c);
        };
        match hit {
            Hit::Prev => send(self, Cmd::Prev),
            Hit::PlayPause => send(self, Cmd::PlayPause),
            Hit::Next => send(self, Cmd::Next),
            Hit::Seek => {
                let f = ((x - r.left) / (r.right - r.left)).clamp(0.0, 1.0);
                let ms = (self.media.lock().unwrap().duration_ms as f32 * f) as u64;
                send(self, Cmd::Seek(ms));
            }
            Hit::Shuffle => {
                let on = self.media.lock().unwrap().shuffle == Some(true);
                send(self, Cmd::Shuffle(!on));
            }
            Hit::Volume => {
                if let Some(a) = self.audio.as_mut() {
                    a.toggle_app_mute();
                }
            }
            Hit::Lyrics => {
                self.lyrics_mode = !self.lyrics_mode;
                self.start_anim();
            }
            Hit::Settings => return true,
            Hit::Page(p) => self.set_page(p),
            Hit::NextPage => self.set_page(self.page + 1),
            Hit::ShelfFile(f) => {
                if let Some(file) = self.shelf.files.get(f) {
                    open_path(&file.path);
                }
            }
            Hit::ShelfClear => {
                self.shelf.files.clear();
                self.save_store();
            }
            Hit::Clip(c) => self.clips.restore(self.hwnd, c),
            Hit::ClipClear => {
                self.clips.clear();
                self.clip_scroll = 0;
                self.save_store();
            }
            Hit::LyricLine(k) => {
                let ms = self.lyrics.lock().unwrap().current.as_ref().and_then(|l| l.lines.get(k).map(|e| e.0));
                if let Some(ms) = ms {
                    send(self, Cmd::Seek(ms));
                    // Back to following the singer, who now starts at this line.
                    self.lyric_scroll = 0.0;
                    self.lyric_scroll_pos = 0.0;
                    self.lyric_scrolled_at = None;
                }
            }
            Hit::AddFriend => self.friend_view = 1,
            Hit::Requests => {
                self.friend_view = 2;
                // Open on whichever side has something.
                let so = crate::social::snapshot();
                self.friend_tab = u8::from(so.incoming.is_empty() && !so.outgoing.is_empty());
            }
            Hit::ReqTab(t) => self.friend_tab = t,
            Hit::FriendsBack => {
                self.friend_view = 0;
                self.friend_typing = false;
            }
            Hit::CopyCode => {
                let code = crate::social::snapshot().code;
                if !code.is_empty() {
                    crate::clipboard::set_text(self.hwnd, &code);
                    self.friend_note = Some((format!("Copied {code}: send it to a friend"), Instant::now()));
                }
            }
            Hit::CodeBox => self.start_typing(),
            Hit::PasteCode => {
                if let Some(t) = crate::clipboard::get_text(self.hwnd) {
                    self.friend_input = t.chars().filter(|c| !c.is_control()).take(16).collect::<String>().trim().to_string();
                }
                self.start_typing();
            }
            Hit::SendCode => self.send_code(),
            Hit::Accept(i) => {
                if let Some((c, _)) = crate::social::snapshot().incoming.get(i) {
                    crate::social::accept(c);
                }
            }
            Hit::Decline(i) => {
                if let Some((c, _)) = crate::social::snapshot().incoming.get(i) {
                    crate::social::decline(c);
                }
            }
            Hit::Cancel(i) => {
                if let Some((c, _)) = crate::social::snapshot().outgoing.get(i) {
                    crate::social::decline(c);
                }
            }
            Hit::Poke(i) => {
                if let Some(f) = crate::social::snapshot().friends.get(i) {
                    crate::social::poke(&f.code);
                }
            }
            Hit::Along(i) => {
                if let Some(f) = crate::social::snapshot().friends.get(i) {
                    crate::social::toggle_along(&f.code);
                }
            }
            Hit::VolumeBar | Hit::ClipList | Hit::LyricList => {}
        }
        self.redraw();
        false
    }

    /// Gives the notch the keyboard so a friend code can be typed in.
    fn start_typing(&mut self) {
        self.friend_typing = true;
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(self.hwnd);
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(self.hwnd));
        }
        self.start_anim();
    }

    /// Sends a friend request to the code in the box.
    fn send_code(&mut self) {
        let text = self.friend_input.trim().to_string();
        if text.is_empty() {
            self.friend_note = Some(("Type or paste your friend's code first".into(), Instant::now()));
            return;
        }
        match crate::social::clean_code(&text) {
            Some(c) if c == crate::social::snapshot().code => {
                self.friend_note = Some(("That's your own code".into(), Instant::now()));
            }
            Some(c) => {
                crate::social::add(&c);
                self.friend_note = Some((format!("Request sent to {c}"), Instant::now()));
                self.friend_input.clear();
                self.friend_typing = false;
            }
            None => self.friend_note = Some(("Friend codes look like FUZ-A4G7B9".into(), Instant::now())),
        }
    }

    /// A key typed while the friend code box has the keyboard.
    fn on_char(&mut self, c: u32) {
        if !self.friend_typing {
            return;
        }
        match c {
            0x08 => {
                self.friend_input.pop();
            }
            0x0D => self.send_code(),
            0x1B => self.friend_typing = false,
            // Ctrl+V
            0x16 => {
                if let Some(t) = crate::clipboard::get_text(self.hwnd) {
                    self.friend_input.extend(t.chars().filter(|c| !c.is_control()));
                }
            }
            c if c >= 0x20 => {
                if let Some(ch) = char::from_u32(c) {
                    self.friend_input.push(ch);
                }
            }
            _ => return,
        }
        self.friend_input = self.friend_input.chars().take(16).collect();
        self.redraw();
    }

    fn on_right_click(&mut self, lp: LPARAM) {
        let (x, y) = self.logical(lp);
        match self.hit_at(x, y).map(|i| self.hits[i].1) {
            Some(Hit::ShelfFile(f)) => self.shelf.remove(f),
            Some(Hit::Clip(c)) => {
                self.clips.remove(c);
                self.clip_scroll = self.clip_scroll.min(self.clips.items.len().saturating_sub(CLIP_ROWS));
            }
            _ => {
                self.start_ghost();
                return;
            }
        }
        self.save_store();
        self.redraw();
    }

    /// Right-click on the notch: it folds, turns see-through and lets clicks
    /// reach what is behind it for a little while (Settings → General).
    fn start_ghost(&mut self) {
        self.ghost = true;
        self.hot = None;
        self.set_open(false);
        set_click_through(self.hwnd, true);
        self.renderer.set_ghost(GHOST_ALPHA); self.renderer.full_refresh();
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_GHOST, self.settings.ghost_ms(), None);
        }
        self.start_anim();
    }

    fn end_ghost(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(self.hwnd), TIMER_GHOST);
        }
        self.ghost = false;
        set_click_through(self.hwnd, false);
        self.renderer.set_ghost(255);
        self.renderer.full_refresh();
        self.redraw();
    }

    fn on_wheel(&mut self, wp: WPARAM, lp: LPARAM) {
        // Wheel coordinates are in screen space.
        let sx = (lp.0 & 0xffff) as i16 as i32 - self.pos.x;
        let sy = ((lp.0 >> 16) & 0xffff) as i16 as i32 - self.pos.y;
        let s = self.renderer.scale();
        let (x, y) = (sx as f32 / s, sy as f32 / s);
        if self.anim < 0.9 {
            return;
        }
        let delta = ((wp.0 >> 16) & 0xffff) as i16;
        let over = |hit: Hit, pad_l: f32, pad_y: f32| {
            self.hits.iter().any(|(r, h)| *h == hit && x >= r.left - pad_l && x <= r.right + 4.0 && y >= r.top - pad_y && y <= r.bottom + pad_y)
        };
        // Over the speaker or its slider: 2% per notch (opens the slider too).
        if over(Hit::Volume, 4.0, 4.0) || over(Hit::VolumeBar, 0.0, 4.0) {
            if let Some(a) = self.audio.as_mut() {
                if let Some((level, _)) = a.app_state() {
                    a.set_app_level(level + delta as f32 / 120.0 * 0.02);
                }
            }
            self.vol_seen = Some(Instant::now());
            self.start_anim();
            return;
        }
        // The clipboard list scrolls itself when it has more rows than fit.
        let max = self.clips.items.len().saturating_sub(CLIP_ROWS);
        if max > 0 && over(Hit::ClipList, 0.0, 0.0) {
            self.clip_scroll = if delta < 0 { (self.clip_scroll + 1).min(max) } else { self.clip_scroll.saturating_sub(1) };
            self.redraw();
            return;
        }
        // Over the lyrics: scroll through the song (one line per notch; touchpads
        // scroll smoothly).
        if over(Hit::LyricList, 0.0, 0.0) {
            let (index, count) = {
                let state = self.lyrics.lock().unwrap();
                let media = self.media.lock().unwrap();
                match state.current.as_ref().filter(|l| l.key == media.key() && l.synced) {
                    Some(l) => (l.index_at(media.position_ms() + 250), l.lines.len() as isize),
                    None => (-1, 0),
                }
            };
            if count > 0 {
                let lo = -(index + 1) as f32;
                let hi = (count - 1 - index).max(0) as f32;
                self.lyric_scroll = (self.lyric_scroll - delta as f32 / 120.0).clamp(lo, hi);
                self.lyric_scrolled_at = Some(Instant::now());
                self.start_anim();
                return;
            }
        }
        // Anywhere else: flip pages. Touchpads send small deltas, so they add up;
        // a short pause after each flip keeps one swipe from skipping a page.
        let now = Instant::now();
        if now.duration_since(self.wheel_at) > Duration::from_millis(400) {
            self.wheel_acc = 0;
        }
        self.wheel_at = now;
        if now.duration_since(self.flipped_at) < Duration::from_millis(260) {
            self.wheel_acc = 0;
            return;
        }
        self.wheel_acc += delta as i32;
        if self.wheel_acc.abs() >= 100 {
            let next = if self.wheel_acc < 0 { self.page + 1 } else { self.page.saturating_sub(1) };
            self.wheel_acc = 0;
            if next != self.page && next < PAGE_COUNT {
                self.flipped_at = now;
                self.set_page(next);
            }
        }
    }

    fn on_clipboard(&mut self) {
        if !self.settings.clipboard_history {
            return;
        }
        let (clock, _) = clock_and_date(&Settings { show_seconds: false, ..self.settings.clone() });
        let time = clock.trim_end_matches(" AM").trim_end_matches(" PM").to_string();
        if self.clips.on_update(self.hwnd, self.settings.clipboard_max, time) {
            self.clip_scroll = 0;
            self.save_store();
            self.redraw();
        }
    }

    fn save_store(&self) {
        shelf::save(&self.shelf, &self.clips);
    }

    fn on_dpi_changed(&mut self) {
        match Renderer::new(dpi_scale(self.hwnd, &self.settings)) {
            Ok(r) => self.renderer = r,
            Err(e) => eprintln!("could not rebuild renderer: {e}"),
        }
        self.redraw();
    }
}

/// Lets clicks fall through `hwnd` to whatever is behind it (`on`), or catches them again.
pub fn set_click_through(hwnd: HWND, on: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TRANSPARENT};
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let t = WS_EX_TRANSPARENT.0 as isize;
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, if on { ex | t } else { ex & !t });
    }
}

/// A copy of the current settings (for the settings window to draw).
pub fn settings_snapshot() -> Option<Settings> {
    with_app(|a| a.settings.clone())
}

/// Changes a setting, applies its side effects, saves, and redraws.
pub fn change_setting(key: Key, f: impl FnOnce(&mut Settings)) {
    with_app(|a| {
        let before = a.settings.clone();
        f(&mut a.settings);
        match key {
            Key::StartWithWindows => {
                if !autostart::set(a.settings.start_with_windows) {
                    a.settings.start_with_windows = before.start_with_windows;
                }
            }
            Key::Scale => {
                if (before.scale - a.settings.scale).abs() > 0.001 {
                    a.on_dpi_changed();
                }
            }
            Key::ClipHistory => unsafe {
                if a.settings.clipboard_history {
                    clipboard::History::start(a.hwnd);
                } else {
                    let _ = RemoveClipboardFormatListener(a.hwnd);
                }
            },
            Key::ShowLyrics => {
                a.lyrics_mode = a.settings.show_lyrics;
                a.start_anim();
            }
            Key::ClipMax => {
                a.clips.trim(a.settings.clipboard_max);
                a.save_store();
            }
            Key::HideFullscreen if !a.settings.hide_in_fullscreen && a.hidden => {
                a.hidden = false;
                unsafe {
                    let _ = ShowWindow(a.hwnd, SW_SHOWNOACTIVATE);
                }
            }
            _ => {}
        }
        if let Err(e) = a.settings.save() {
            eprintln!("could not save settings: {e}");
        }
        deskfox::sync(&a.settings);
        crate::updater::set_enabled(a.settings.auto_update);
        if !std::env::args().any(|x| x == "--snapshot") {
            social_sync(&a.settings);
        }
        a.redraw();
    });
}

/// Closes Fuzzie (the notch window ends the message loop).
pub fn quit() {
    let hwnd = HWND(NOTCH.load(std::sync::atomic::Ordering::Relaxed) as *mut core::ffi::c_void);
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(hwnd), windows::Win32::UI::WindowsAndMessaging::WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// The notch window, for [`quit`] (callable from anywhere on the UI thread).
static NOTCH: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Opens the friend's song in Spotify (the app if installed, else the web).
fn open_in_spotify(artist: &str, title: &str) {
    let q: String = format!("{artist} {title}").chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let enc: String = q
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    let app = HSTRING::from(format!("spotify:search:{enc}"));
    let web = HSTRING::from(format!("https://open.spotify.com/search/{enc}"));
    unsafe {
        // ShellExecute returns a value > 32 on success.
        if ShellExecuteW(None, w!("open"), &app, None, None, SW_SHOWNORMAL).0 as isize <= 32 {
            ShellExecuteW(None, w!("open"), &web, None, None, SW_SHOWNORMAL);
        }
    }
}

/// Tells the friends connection who we are and whether to be online.
fn social_sync(s: &Settings) {
    let name = if s.social_name.trim().is_empty() { "Fuzzie" } else { s.social_name.as_str() };
    crate::social::set_profile(name, s.skin, s.acc, s.share_music);
    crate::social::set_enabled(s.social_on);
}

/// Remembers where the desktop fox was dragged to.
pub fn save_desk_pos(x: i32, y: i32) {
    with_app(|a| {
        a.settings.desk_x = x;
        a.settings.desk_y = y;
        if let Err(e) = a.settings.save() {
            eprintln!("could not save settings: {e}");
        }
    });
}

pub fn open_data_folder() {
    let dir = settings::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    open_path(&dir);
}

/// With `FUZZIE_FPS=1` set: prints every 5 s how many frames were drawn, the
/// longest gap between two (to check that animations run evenly) and the mean
/// time spent per frame preparing, drawing and presenting.
fn fps_probe(t: [Instant; 4]) {
    use std::cell::Cell;
    thread_local! {
        static ON: bool = std::env::var_os("FUZZIE_FPS").is_some();
        static STATE: Cell<(Option<Instant>, Option<Instant>, u32, f32, [f32; 3])> = const { Cell::new((None, None, 0, 0.0, [0.0; 3])) };
    }
    if !ON.with(|o| *o) {
        return;
    }
    STATE.with(|st| {
        let now = t[0];
        let (start, last, n, gap, mut spent) = st.get();
        let gap = last.map_or(gap, |l| gap.max(now.duration_since(l).as_secs_f32() * 1000.0));
        for i in 0..3 {
            spent[i] += t[i + 1].duration_since(t[i]).as_secs_f32() * 1000.0;
        }
        let start = start.unwrap_or(now);
        if now.duration_since(start).as_secs_f32() >= 5.0 {
            let k = (n + 1) as f32;
            println!(
                "frames: {:.1} fps, longest gap {:.0} ms, per frame: prep {:.2} ms, draw {:.2} ms, present {:.2} ms",
                n as f32 / 5.0,
                gap,
                spent[0] / k,
                spent[1] / k,
                spent[2] / k
            );

            st.set((Some(now), Some(now), 0, 0.0, [0.0; 3]));
        } else {
            st.set((Some(start), Some(now), n + 1, gap, spent));
        }
    });
}

/// Seconds since the last keyboard or mouse input anywhere.
fn idle_secs() -> u32 {
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut li).as_bool() {
            return 0;
        }
        GetTickCount().wrapping_sub(li.dwTime) / 1000
    }
}

/// True when the foreground window covers its whole monitor (games, videos).
fn fullscreen_app_in_front(own: HWND) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == own {
            return false;
        }
        let mut buf = [0u16; 64];
        let n = GetClassNameW(fg, &mut buf) as usize;
        let class = String::from_utf16_lossy(&buf[..n]);
        // The desktop and the shell are not apps.
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "FuzzieSettings") {
            return false;
        }
        let mut r = windows::Win32::Foundation::RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTOPRIMARY), &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

fn on_drag(event: DragEvent) {
    with_app(|a| {
        match event {
            DragEvent::Enter => {
                a.drag_over = true;
                a.set_page(render::PAGE_SHELF);
                a.set_open(true);
            }
            DragEvent::Leave => a.drag_over = false,
            DragEvent::Drop(paths) => {
                a.drag_over = false;
                for p in paths {
                    a.shelf.add(p);
                }
                a.save_store();
            }
        }
        a.redraw();
    });
}

fn open_path(path: &std::path::Path) {
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(path.as_os_str()), None, None, SW_SHOWNORMAL);
    }
}

/// ("12:23 AM", "Monday, November 23") following the clock settings.
fn clock_and_date(s: &Settings) -> (String, String) {
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
    ];
    let t = unsafe { GetLocalTime() };
    let (h, m, sec) = (t.wHour, t.wMinute, t.wSecond);
    let hour = if s.clock_24h { h } else { (h + 11) % 12 + 1 };
    let mut clock = if s.clock_24h { format!("{hour:02}:{m:02}") } else { format!("{hour}:{m:02}") };
    if s.show_seconds {
        clock.push_str(&format!(":{sec:02}"));
    }
    if !s.clock_24h {
        clock.push_str(if h < 12 { " AM" } else { " PM" });
    }
    let day = DAYS[t.wDayOfWeek as usize % 7];
    let month = MONTHS[(t.wMonth as usize).clamp(1, 12) - 1];
    (clock, format!("{day}, {month} {}", t.wDay))
}

fn handle_tray_menu(hwnd: HWND) {
    let Some(state) = with_app(|a| tray::MenuState {
        minimal: a.settings.idle_style == IdleStyle::Minimal,
        lyrics: a.settings.show_lyrics,
        desk_fox: a.settings.desk_fox,
        desk_lock: a.settings.desk_lock,
        autostart: a.settings.start_with_windows,
    }) else {
        return;
    };
    // The menu runs a modal loop, so no app borrow may be held here.
    match tray::show_menu(hwnd, &state) {
        Some(tray::MenuCmd::MinimalNotch) => {
            let style = if state.minimal { IdleStyle::Clock } else { IdleStyle::Minimal };
            change_setting(Key::IdleStyle, |s| s.idle_style = style);
            settings_win::refresh();
        }
        Some(tray::MenuCmd::Lyrics) => {
            change_setting(Key::ShowLyrics, |s| s.show_lyrics = !state.lyrics);
            settings_win::refresh();
        }
        Some(tray::MenuCmd::DeskFox) => {
            change_setting(Key::DeskFox, |s| s.desk_fox = !state.desk_fox);
            settings_win::refresh();
        }
        Some(tray::MenuCmd::DeskLock) => {
            change_setting(Key::DeskLock, |s| s.desk_lock = !state.desk_lock);
            settings_win::refresh();
        }
        Some(tray::MenuCmd::Autostart) => {
            change_setting(Key::StartWithWindows, |s| s.start_with_windows = !state.autostart);
            settings_win::refresh();
        }
        Some(tray::MenuCmd::Settings) => settings_win::open(),
        Some(tray::MenuCmd::Quit) => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        None => {}
    }
}


unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_app(|a| match wp.0 {
                TIMER_CLOCK => {
                    stats::touch_day();
                    if a.clips.expire() {
                        a.save_store();
                        a.redraw();
                    }
                    if a.frame_ms == 0 {
                        a.redraw();
                    }
                }
                TIMER_POLL => a.on_poll(),
                TIMER_SNAPSHOT => a.snapshot(),
                TIMER_GHOST => a.end_ghost(),
                _ => {}
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(Some(paths)) = with_app(|a| a.on_mouse_move(lp)) {
                // Modal: runs its own message loop until the drop ends.
                dragdrop::drag_out(&paths);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(|a| a.on_mouse_leave());
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            with_app(|a| a.on_button_down(lp));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if with_app(|a| a.on_click(lp)) == Some(true) {
                settings_win::open();
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            with_app(|a| a.on_right_click(lp));
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_CHAR => {
            with_app(|a| a.on_char(wp.0 as u32));
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_KILLFOCUS => {
            with_app(|a| {
                if a.friend_typing {
                    a.friend_typing = false;
                    a.redraw();
                }
            });
            LRESULT(0)
        }
        WM_FRAME => {
            with_app(|a| {
                a.clock.consumed();
                a.redraw();
            });
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            with_app(|a| {
                if a.vol_drag.take().is_some() {
                    a.start_anim();
                }
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            with_app(|a| a.on_wheel(wp, lp));
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE => {
            with_app(|a| a.on_clipboard());
            LRESULT(0)
        }
        media::WM_MEDIA_CHANGED => {
            with_app(|a| a.redraw());
            LRESULT(0)
        }
        tray::WM_TRAY => {
            let event = (lp.0 & 0xffff) as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                handle_tray_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            with_app(|a| a.on_dpi_changed());
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            with_app(|a| a.redraw());
            deskfox::keep_on_screen();
            LRESULT(0)
        }
        crate::updater::WM_UPDATED => {
            // A new version is in place: hand over to it.
            crate::updater::relaunch();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if !crate::wipe::pending() {
                with_app(|a| a.save_store());
                stats::save();
            }
            deskfox::close();
            tray::remove(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        m if m == taskbar_created() => {
            // Explorer restarted: the tray icon must be added again.
            let _ = tray::add(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

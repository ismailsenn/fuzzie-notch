//! The settings window: a sidebar of pages and custom-drawn controls
//! (toggles, segmented choices, sliders, a colour editor, mascot looks, stats).
//! Every change is applied to the notch immediately and saved.
//!
//! Layout numbers are in "units": 1 unit = 0.8 logical pixels, so the window
//! is 930 × 750 units (744 × 600 logical pixels).

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D_RECT_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE, ID2D1BitmapRenderTarget, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget, ID2D1LinearGradientBrush, ID2D1RadialGradientBrush, ID2D1SolidColorBrush,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_EXTEND_MODE_CLAMP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
};
use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, IsIconic, LoadCursorW,
    RegisterClassExW, KillTimer, SetForegroundWindow, SetTimer, ShowWindow, HTCAPTION, HTCLIENT, IDC_ARROW,
    SW_RESTORE, SW_SHOW, WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_APP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSEXW,
    WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
};
use windows_numerics::Vector2;

use crate::render::{approach, ease, ellipse, format, font_exists, lerp, luminance, mix, rgba};
use crate::settings::{self, BgStyle, IdleStyle, NotchStyle, OpenMode, Placement, Settings};
use crate::stats::{self, Stats};
use crate::frameclock::FrameClock;
use crate::{clipboard, mascot, window};

/// Logical pixels per layout unit is 1 / UNIT.
const UNIT: f32 = 1.25;
const WIN_W: f32 = 930.0;
const WIN_H: f32 = 750.0;
const SIDEBAR: f32 = 248.0;
const LEFT: f32 = 290.0;
const RIGHT: f32 = 880.0;
const CONTENT_TOP: f32 = 85.0;
const WM_MOUSELEAVE: u32 = 0x02A3;

const BG: [f32; 3] = [0.122, 0.122, 0.137];
const SIDE_BG: [f32; 3] = [0.09, 0.09, 0.1];
const CARD: [f32; 3] = [0.168, 0.168, 0.185];
const FIELD: [f32; 3] = [0.19, 0.19, 0.205];
const FG: [f32; 3] = [0.94, 0.94, 0.96];
const NAV: [f32; 3] = [0.78, 0.78, 0.8];
const DIM: [f32; 3] = [0.6, 0.6, 0.64];
const OFF: [f32; 3] = [0.29, 0.29, 0.31];
const PINK: [f32; 3] = [1.0, 0.6, 0.76];

pub const DEFAULT_BACKGROUND: &str = "#000000";
pub const DEFAULT_ACCENT: &str = "#FF8A3D";

/// Background / accent pairs offered as colour presets.
const PRESETS: [(&str, &str); 11] = [
    ("#000000", "#FF8A3D"),
    ("#2A2A2E", "#5AB0FF"),
    ("#14213D", "#5AB0FF"),
    ("#3A1D5C", "#E8A0FF"),
    ("#123B26", "#7CE08A"),
    ("#4A1420", "#FFB08A"),
    ("#8A4A1C", "#FFE0B0"),
    ("#F2F2F5", "#FF7A30"),
    ("#000000", "#9B30D0"),
    ("#000000", "#C902F0"),
    ("#1E0A1E", "#FF2020"),
];

const PAGES: [(&str, &str); 12] = [
    ("APP", "General"),
    ("APP", "Appearance"),
    ("APP", "Customize"),
    ("APP", "Desktop Fuzzie"),
    ("FEATURES", "Music & Lyrics"),
    ("FEATURES", "Shelf"),
    ("FEATURES", "Reminders"),
    ("FEATURES", "HUD & Notifications"),
    ("FRIENDS", "Social"),
    ("FRIENDS", "Stats"),
    ("FUZZIE", "Updates"),
    ("FUZZIE", "About"),
];
const PAGE_CUSTOMIZE: usize = 2;
const PAGE_DESK: usize = 3;
const PAGE_STATS: usize = 9;
const PAGE_UPDATES: usize = 10;
const PAGE_SOCIAL: usize = 8;
/// A timer for live numbers on the Stats page; frames for tweens (~60 fps) and
/// for the animated foxes on Customize and About (~30 fps) come from the vblank clock.
const TIMER_STATS: usize = 1;
/// The foxes' next blink, and their eyes opening again.
const TIMER_BLINK: usize = 2;
const TIMER_EYES: usize = 3;
const WM_FRAME: u32 = WM_APP + 5;
const FRAME_FAST: u32 = 16;
/// The two live foxes (sidebar and the Customize stage): 25 fps, only while
/// the window is in front (behind other windows they hold still and blink).
const FRAME_LIVE: u32 = 40;

/// A setting the window can change.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    StartWithWindows,
    HideFullscreen,
    OpenMode,
    IdleStyle,
    Clock24,
    Seconds,
    Scale,
    Style,
    Placement,
    BgStyle,
    GradStrength,
    GradSoft,
    Glass,
    Colors,
    ShowMascot,
    Nap,
    NapAfter,
    NapWhileMusic,
    DeskFox,
    DeskSize,
    GhostSecs,
    DeskKeys,
    DeskClicks,
    DeskBlink,
    DeskNap,
    DeskAnim,
    DeskLock,
    DeskGlow,
    DeskAcc,
    DeskTaskbar,
    Skin,
    ShowLyrics,
    Visualizer,
    EmptyShelf,
    ClipHistory,
    ClipMax,
    Remember,
    HudVolume,
    HudCaps,
    HudWifi,
    HudDevice,
    Notifications,
    AutoUpdate,
    MessagePreview,
    SocialOn,
    ShareMusic,
    SocialName,
}

const NAP_SECS: [u32; 5] = [10, 30, 60, 120, 300];
const CLIP_RANGE: (f32, f32) = (5.0, 50.0);

fn get_bool(s: &Settings, k: Key) -> bool {
    match k {
        Key::StartWithWindows => s.start_with_windows,
        Key::HideFullscreen => s.hide_in_fullscreen,
        Key::Clock24 => s.clock_24h,
        Key::Seconds => s.show_seconds,
        Key::Glass => s.glass,
        Key::ShowMascot => s.show_mascot,
        Key::Nap => s.nap_enabled,
        Key::NapWhileMusic => s.nap_while_music,
        Key::DeskFox => s.desk_fox,
        Key::DeskKeys => s.desk_keys,
        Key::DeskClicks => s.desk_clicks,
        Key::DeskBlink => s.desk_blink,
        Key::DeskNap => s.desk_nap,
        Key::DeskAnim => s.desk_anim,
        Key::DeskLock => s.desk_lock,
        Key::DeskGlow => s.desk_glow,
        Key::DeskAcc => s.desk_acc,
        Key::DeskTaskbar => s.desk_on_taskbar,
        Key::ShowLyrics => s.show_lyrics,
        Key::Visualizer => s.audio_visualizer,
        Key::EmptyShelf => s.start_with_empty_shelf,
        Key::ClipHistory => s.clipboard_history,
        Key::Remember => s.remember_after_restart,
        Key::HudVolume => s.hud_volume,
        Key::HudCaps => s.hud_capslock,
        Key::HudWifi => s.hud_wifi,
        Key::HudDevice => s.hud_audio_device,
        Key::Notifications => s.show_notifications,
        Key::AutoUpdate => s.auto_update,
        Key::MessagePreview => s.show_message_preview,
        Key::SocialOn => s.social_on,
        Key::ShareMusic => s.share_music,
        _ => false,
    }
}

fn set_bool(s: &mut Settings, k: Key, v: bool) {
    let f = match k {
        Key::StartWithWindows => &mut s.start_with_windows,
        Key::HideFullscreen => &mut s.hide_in_fullscreen,
        Key::Clock24 => &mut s.clock_24h,
        Key::Seconds => &mut s.show_seconds,
        Key::Glass => &mut s.glass,
        Key::ShowMascot => &mut s.show_mascot,
        Key::Nap => &mut s.nap_enabled,
        Key::NapWhileMusic => &mut s.nap_while_music,
        Key::DeskFox => &mut s.desk_fox,
        Key::DeskKeys => &mut s.desk_keys,
        Key::DeskClicks => &mut s.desk_clicks,
        Key::DeskBlink => &mut s.desk_blink,
        Key::DeskNap => &mut s.desk_nap,
        Key::DeskAnim => &mut s.desk_anim,
        Key::DeskLock => &mut s.desk_lock,
        Key::DeskGlow => &mut s.desk_glow,
        Key::DeskAcc => &mut s.desk_acc,
        Key::DeskTaskbar => &mut s.desk_on_taskbar,
        Key::ShowLyrics => &mut s.show_lyrics,
        Key::Visualizer => &mut s.audio_visualizer,
        Key::EmptyShelf => &mut s.start_with_empty_shelf,
        Key::ClipHistory => &mut s.clipboard_history,
        Key::Remember => &mut s.remember_after_restart,
        Key::HudVolume => &mut s.hud_volume,
        Key::HudCaps => &mut s.hud_capslock,
        Key::HudWifi => &mut s.hud_wifi,
        Key::HudDevice => &mut s.hud_audio_device,
        Key::Notifications => &mut s.show_notifications,
        Key::AutoUpdate => &mut s.auto_update,
        Key::MessagePreview => &mut s.show_message_preview,
        Key::SocialOn => &mut s.social_on,
        Key::ShareMusic => &mut s.share_music,
        _ => return,
    };
    *f = v;
}

fn get_choice(s: &Settings, k: Key) -> usize {
    match k {
        Key::OpenMode => (s.open_mode == OpenMode::Click) as usize,
        Key::IdleStyle => (s.idle_style == IdleStyle::Minimal) as usize,
        Key::Style => (s.style == NotchStyle::Floating) as usize,
        Key::Placement => match s.placement {
            Placement::TopCenter => 0,
            Placement::TopLeft => 1,
            Placement::TopRight => 2,
            Placement::BottomLeft => 3,
            Placement::BottomRight => 4,
            Placement::Custom => 5,
        },
        Key::BgStyle => match s.bg_style {
            BgStyle::Solid => 0,
            BgStyle::Gradient => 1,
            BgStyle::Album => 2,
        },
        Key::NapAfter => NAP_SECS.iter().position(|&v| v >= s.nap_after_secs).unwrap_or(4),
        _ => 0,
    }
}

fn set_choice(s: &mut Settings, k: Key, i: usize) {
    match k {
        Key::OpenMode => s.open_mode = if i == 1 { OpenMode::Click } else { OpenMode::Hover },
        Key::IdleStyle => s.idle_style = if i == 1 { IdleStyle::Minimal } else { IdleStyle::Clock },
        Key::Style => s.style = if i == 1 { NotchStyle::Floating } else { NotchStyle::Attached },
        Key::Placement => {
            s.placement = [Placement::TopCenter, Placement::TopLeft, Placement::TopRight, Placement::BottomLeft, Placement::BottomRight, Placement::Custom][i.min(5)]
        }
        Key::BgStyle => s.bg_style = [BgStyle::Solid, BgStyle::Gradient, BgStyle::Album][i.min(2)],
        Key::NapAfter => s.nap_after_secs = NAP_SECS[i.min(4)],
        _ => {}
    }
}

/// Slider position (0..1) and its label.
fn slider_value(s: &Settings, k: Key) -> (f32, String) {
    match k {
        Key::ClipMax => {
            let v = (s.clipboard_max as f32).clamp(CLIP_RANGE.0, CLIP_RANGE.1);
            ((v - CLIP_RANGE.0) / (CLIP_RANGE.1 - CLIP_RANGE.0), format!("{}", v as usize))
        }
        Key::GhostSecs => {
            let v = s.ghost_secs.clamp(1.0, 5.0);
            ((v - 1.0) / 4.0, format!("{v:.1} s"))
        }
        Key::DeskSize => {
            let v = s.desk_size();
            ((v - 0.25) / 1.75, format!("{}%", (v * 100.0).round() as i32))
        }
        Key::GradStrength | Key::GradSoft => {
            let v = if k == Key::GradStrength { s.gradient_strength } else { s.gradient_softness }.clamp(0.0, 1.0);
            (v, format!("{}%", (v * 100.0).round() as i32))
        }
        _ => (((s.scale() - 0.5) / 1.0).clamp(0.0, 1.0), format!("{}%", (s.scale() * 100.0).round() as i32)),
    }
}

fn set_slider(s: &mut Settings, k: Key, f: f32) {
    match k {
        Key::ClipMax => s.clipboard_max = (CLIP_RANGE.0 + f * (CLIP_RANGE.1 - CLIP_RANGE.0)).round() as usize,
        Key::GradStrength => s.gradient_strength = (f * 100.0).round() / 100.0,
        Key::GhostSecs => s.ghost_secs = ((1.0 + f * 4.0) * 2.0).round() / 2.0, // 0.5 s steps
        Key::DeskSize => s.desk_scale = ((0.25 + f * 1.75) * 20.0).round() / 20.0, // 5% steps
        Key::GradSoft => s.gradient_softness = (f * 100.0).round() / 100.0,
        _ => s.scale = ((0.5 + f) * 20.0).round() / 20.0, // 5% steps
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ColorTarget {
    Background,
    Accent,
}

fn color_of(s: &Settings, t: ColorTarget) -> [f32; 3] {
    match t {
        ColorTarget::Background => settings::parse_hex(&s.background, [0.0; 3]),
        ColorTarget::Accent => settings::parse_hex(&s.accent, [0.79, 0.01, 0.94]),
    }
}

fn hex(c: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", b(c[0]), b(c[1]), b(c[2]))
}

fn set_color(t: ColorTarget, c: [f32; 3]) {
    let h = hex(c);
    window::change_setting(Key::Colors, |s| match t {
        ColorTarget::Background => s.background = h,
        ColorTarget::Accent => s.accent = h,
    });
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// A random number below `n` from the clock (good enough for "surprise me").
fn random_below(seed: u64, n: usize) -> usize {
    let mut x = seed | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x % n.max(1) as u64) as usize
}

fn rgb_to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let [r, g, b] = c;
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max <= 0.0 { 0.0 } else { d / max }, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

enum Item {
    Section(&'static str),
    Toggle(&'static str, Key),
    Segment(&'static str, Key, &'static [&'static str]),
    Grid(&'static str, Key, &'static [&'static str]),
    Slider(&'static str, Key),
    Button(&'static str, Act, f32),
    Colors,
    ColorRow(&'static str, ColorTarget),
    /// Big live preview of the fox with skin and accessory pickers.
    Hero,
    /// Skins / Accessories / Options tabs of the character screen.
    Tabs,
    SkinGrid,
    AccGrid,
    MascotCard,
    Text(String),
    Row(&'static str, String),
    StatCard(&'static str, &'static str, String, Option<Act>),
    YearBreakdown,
    Divider,
    FriendTable,
    RpsTable,
    /// A one-line text box.
    Input(&'static str, Field),
    /// A person: name, a line under it, and buttons on the right.
    Person(String, String, Vec<(&'static str, Act)>),
}

/// Text boxes on the Social page.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Field {
    Name,
    Code,
}

impl Field {
    fn max(self) -> usize {
        match self {
            Field::Name => 24,
            Field::Code => 16,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    Page(usize),
    Close,
    Toggle(Key),
    Choose(Key, usize),
    Slider(Key),
    Preset(usize),
    EditColor(ColorTarget),
    CopyColor(ColorTarget),
    PasteColor(ColorTarget),
    Sv(ColorTarget),
    Hue(ColorTarget),
    RestoreColors,
    ResetPosition,
    ResetDesk,
    Skin(usize),
    Acc(usize),
    Tab(usize),
    StepSkin(i64),
    StepAcc(i64),
    Random,
    OpenFolder,
    Focus(Field),
    CopyCode,
    SendRequest,
    Accept(usize),
    Decline(usize),
    Cancel(usize),
    Remove(usize),
    CheckUpdate,
    InstallUpdate,
    DeleteData,
    Uninstall,
    ToggleYears,
}

struct Fonts {
    logo: IDWriteTextFormat,
    group: IDWriteTextFormat,
    nav: IDWriteTextFormat,
    title: IDWriteTextFormat,
    section: IDWriteTextFormat,
    label: IDWriteTextFormat,
    seg: IDWriteTextFormat,
    text: IDWriteTextFormat,
    value: IDWriteTextFormat,
    field: IDWriteTextFormat,
    card_name: IDWriteTextFormat,
    name_c: IDWriteTextFormat,
    tab: IDWriteTextFormat,
    small_c: IDWriteTextFormat,
    big: IDWriteTextFormat,
    stat_label: IDWriteTextFormat,
    stat_sub: IDWriteTextFormat,
    stat_value: IDWriteTextFormat,
    pink_head: IDWriteTextFormat,
    pink_sub: IDWriteTextFormat,
    pink_item: IDWriteTextFormat,
    table_head: IDWriteTextFormat,
    table_val: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_big: IDWriteTextFormat,
}

pub struct SettingsWindow {
    hwnd: HWND,
    factory: ID2D1Factory,
    rt: Option<ID2D1HwndRenderTarget>,
    f: Fonts,
    page: usize,
    scroll: f32,
    content_h: f32,
    hits: Vec<(D2D_RECT_F, Act)>,
    /// The control under the pointer, and the eased hover highlight per control.
    hot: Option<Act>,
    hover: Vec<(Act, f32)>,
    /// The button just pressed and its feedback left (1 → 0).
    flash: Option<(Act, f32)>,
    /// A dangerous button clicked once, waiting for the second click.
    confirm: Option<(Act, Instant)>,
    /// The text box being typed in and its text.
    typing: Option<(Field, String)>,
    /// A short message under "Add a friend" and when it was set.
    note: Option<(String, Instant)>,
    /// The accent colour of the frame being drawn.
    accent_now: [f32; 3],
    /// Eased toggle knobs and segment highlights, per setting.
    toggles: Vec<(Key, f32)>,
    choices: Vec<(Key, f32)>,
    /// Eased sidebar highlight (page index) and character tab highlight.
    nav_pos: f32,
    tab_pos: f32,
    /// 0..1 since the page (or character tab) changed: the content fades and rises in.
    page_t: f32,
    /// Seconds since the last frame (for the tweens), and whether any tween moves.
    dt: f32,
    last_frame: Instant,
    moving: bool,
    /// Interval of the frame clock in ms (0 = off).
    frame_ms: u32,
    clock: FrameClock,
    dragging: Option<(D2D_RECT_F, Act)>,
    tracking: bool,
    want_close: bool,
    /// Colour whose inline picker is open, and its HSV (kept so hue survives black).
    editing: Option<ColorTarget>,
    hsv: (f32, f32, f32),
    years_open: bool,
    /// Character screen tab: 0 skins, 1 accessories, 2 options.
    tab: usize,
    /// The foxes hold still; their eyes shut for a moment every few seconds.
    eyes_shut: bool,
    blinks: u32,
    /// Drives the live foxes.
    started: Instant,
    /// Gradient brushes by their stops; tied to `rt`.
    gradients: HashMap<String, ID2D1RadialGradientBrush>,
    linears: HashMap<String, ID2D1LinearGradientBrush>,
    /// The page as last drawn, without the two live foxes; while nothing eases
    /// a frame is this bitmap plus those foxes.
    cache: Option<ID2D1BitmapRenderTarget>,
    /// The page changed since `cache` was drawn.
    dirty: std::cell::Cell<bool>,
    /// Drawing into `cache`: leave out the live foxes.
    skip_live: bool,
    /// The Customize stage (window coordinates) when it is on screen.
    stage: Option<D2D_RECT_F>,
}

impl Drop for SettingsWindow {
    fn drop(&mut self) {
        // Let go of everything tied to the GPU so the memory goes with the window.
        if let Some(c) = self.cache.take() {
            mascot::release(&c, &self.factory);
        }
        if let Some(rt) = self.rt.take() {
            mascot::release(&rt, &self.factory);
        }
        self.gradients.clear();
        self.linears.clear();
        SOLID.with(|b| b.borrow_mut().clear());
    }
}

thread_local! {
    static WIN: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
    /// One solid brush per render target (the window and the cached page), recoloured per shape.
    static SOLID: RefCell<Vec<(usize, ID2D1SolidColorBrush)>> = const { RefCell::new(Vec::new()) };
}

fn with_win<R>(f: impl FnOnce(&mut SettingsWindow) -> R) -> Option<R> {
    WIN.with(|w| w.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// Opens the settings window, or brings it to the front.
pub fn open() {
    if let Some(hwnd) = with_win(|w| w.hwnd) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    match SettingsWindow::create() {
        Ok(w) => {
            let hwnd = w.hwnd;
            WIN.with(|c| *c.borrow_mut() = Some(w));
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
        }
        Err(e) => eprintln!("settings window failed: {e}"),
    }
}

/// Physical pixels per layout unit.
fn scale_of(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0 / UNIT
}

impl SettingsWindow {
    fn create() -> Result<Self> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class = w!("FuzzieSettings");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hIcon: crate::tray::app_icon(false),
                hIconSm: crate::tray::app_icon(true),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc); // fails harmlessly if already registered

            // Centre on the primary monitor at its DPI.
            let mon = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let r = mi.rcWork;
            let hwnd = CreateWindowExW(
                WS_EX_APPWINDOW,
                class,
                w!("Fuzzie Settings"),
                WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
                r.left,
                r.top,
                100,
                100,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?;
            let s = scale_of(hwnd);
            let (w, h) = ((WIN_W * s) as i32, (WIN_H * s) as i32);
            let x = r.left + ((r.right - r.left) - w) / 2;
            let y = r.top + ((r.bottom - r.top) - h) / 2;
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(hwnd, None, x, y, w, h, windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER);
            // Rounded corners, dark frame (Windows 11; ignored elsewhere).
            let round: u32 = 2;
            let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
            let dark: u32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
            let border = COLORREF(0x00383838);
            let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_BORDER_COLOR, &border as *const _ as _, 4);
            // Live numbers on the Stats page.
            SetTimer(Some(hwnd), TIMER_STATS, 1000, None);
            SetTimer(Some(hwnd), TIMER_BLINK, 3000, None);

            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let text = w!("Segoe UI Variable Text");
            let icon_font = if font_exists(&dw, w!("Segoe Fluent Icons")) { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") };
            let (semi, normal) = (DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_WEIGHT_NORMAL);
            let (l, c, r) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING);
            let f = Fonts {
                logo: format(&dw, text, 23.0, semi, l)?,
                group: format(&dw, text, 13.0, semi, l)?,
                nav: format(&dw, text, 17.5, normal, l)?,
                title: format(&dw, text, 29.0, semi, l)?,
                section: format(&dw, text, 13.0, semi, l)?,
                label: format(&dw, text, 17.5, normal, l)?,
                seg: format(&dw, text, 15.5, normal, c)?,
                text: format(&dw, text, 14.5, normal, l)?,
                value: format(&dw, text, 15.0, normal, r)?,
                field: format(&dw, text, 15.5, normal, l)?,
                card_name: format(&dw, text, 15.5, semi, c)?,
                name_c: format(&dw, text, 21.0, semi, c)?,
                tab: format(&dw, text, 16.0, semi, c)?,
                small_c: format(&dw, text, 13.5, normal, c)?,
                big: format(&dw, text, 21.0, semi, l)?,
                stat_label: format(&dw, text, 13.0, semi, l)?,
                stat_sub: format(&dw, text, 16.0, normal, l)?,
                stat_value: format(&dw, text, 28.0, semi, l)?,
                pink_head: format(&dw, text, 16.5, semi, l)?,
                pink_sub: format(&dw, text, 13.5, semi, l)?,
                pink_item: format(&dw, text, 15.0, normal, l)?,
                table_head: format(&dw, text, 14.0, normal, c)?,
                table_val: format(&dw, text, 21.0, semi, c)?,
                icon: format(&dw, icon_font, 15.0, normal, c)?,
                icon_big: format(&dw, icon_font, 21.0, normal, c)?,
            };
            Ok(Self {
                hwnd,
                factory,
                rt: None,
                cache: None,
                dirty: std::cell::Cell::new(true),
                skip_live: false,
                stage: None,
                f,
                page: 0,
                scroll: 0.0,
                content_h: 0.0,
                hits: Vec::new(),
                hot: None,
                hover: Vec::new(),
                flash: None,
                typing: None,
                note: None,
                confirm: None,
                accent_now: [0.78, 0.0, 0.94],
                toggles: Vec::new(),
                choices: Vec::new(),
                nav_pos: 0.0,
                tab_pos: 0.0,
                page_t: 1.0,
                dt: 1.0 / 60.0,
                last_frame: Instant::now(),
                moving: false,
                frame_ms: 0,
                clock: FrameClock::new(hwnd, WM_FRAME),
                dragging: None,
                tracking: false,
                want_close: false,
                editing: None,
                hsv: (0.0, 0.0, 0.0),
                years_open: false,
                tab: 0,
                eyes_shut: false,
                blinks: 0,
                started: Instant::now(),
                gradients: HashMap::new(),
                linears: HashMap::new(),
            })
        }
    }

    fn items(&self, st: &Stats) -> Vec<Item> {
        use Item::*;
        let t = |s: &str| Text(s.to_string());
        match self.page {
            0 => vec![
                Section("STARTUP"),
                Toggle("Start with Windows", Key::StartWithWindows),
                Toggle("Hide in full screen", Key::HideFullscreen),
                Toggle("Update automatically", Key::AutoUpdate),
                Section("OPENING"),
                Segment("Open the notch on", Key::OpenMode, &["Hover", "Click"]),
                Section("COLLAPSED NOTCH"),
                Segment("Look", Key::IdleStyle, &["Clock", "Minimal"]),
                t("Minimal is a thin bar, even while music plays."),
                Section("RIGHT CLICK"),
                Slider("See-through time", Key::GhostSecs),
                t("Right-click the notch or Desktop Fuzzie to click what's behind it."),
                Section("DATE & TIME"),
                Toggle("24-hour clock", Key::Clock24),
                Toggle("Show seconds", Key::Seconds),
            ],
            1 => vec![
                Section("SIZE"),
                Slider("Size", Key::Scale),
                Section("POSITION"),
                Segment("Style", Key::Style, &["Attached", "Floating"]),
                Grid("Position", Key::Placement, &["Top center", "Top left", "Top right", "Bottom left", "Bottom right", "Custom (drag)"]),
                Button("Reset position", Act::ResetPosition, 190.0),
                Section("BACKGROUND"),
                Segment("Style", Key::BgStyle, &["Solid", "Gradient", "Album"]),
                Slider("Gradient strength", Key::GradStrength),
                Slider("Gradient softness", Key::GradSoft),
                Section("GLASS"),
                Toggle("Glass effect", Key::Glass),
                Section("COLOR"),
                Colors,
                ColorRow("Background color", ColorTarget::Background),
                ColorRow("Accent color", ColorTarget::Accent),
                Button("Restore default colors", Act::RestoreColors, 237.0),
            ],
            PAGE_CUSTOMIZE => {
                let mut v = vec![Hero, Tabs];
                match self.tab {
                    0 => v.push(SkinGrid),
                    1 => v.push(AccGrid),
                    _ => v.extend([
                        Section("MASCOT"),
                        Toggle("Mascot", Key::ShowMascot),
                        Toggle("Nap when I'm away", Key::Nap),
                        Segment("Start napping after", Key::NapAfter, &["10 s", "30 s", "1 min", "2 min", "5 min"]),
                        Toggle("Nap while music plays", Key::NapWhileMusic),
                        Section("DESKTOP FUZZIE"),
                        Toggle("Fox on the desktop", Key::DeskFox),
                        t("More options are on the Desktop Fuzzie page."),
                    ]),
                }
                v
            }
            PAGE_DESK => vec![
                Section("DESKTOP FUZZIE"),
                Toggle("Fox on the desktop", Key::DeskFox),
                t("Sits on the taskbar and taps a paw on every key press and click."),
                Section("SIZE"),
                Slider("Size", Key::DeskSize),
                Section("LOOK"),
                Toggle("Wear the accessory", Key::DeskAcc),
                Toggle("Glow effects", Key::DeskGlow),
                Section("TAPPING"),
                Toggle("Tap on key presses", Key::DeskKeys),
                Toggle("Tap on clicks", Key::DeskClicks),
                Section("LIFE"),
                Toggle("Blink now and then", Key::DeskBlink),
                Toggle("Sleep when I'm away", Key::DeskNap),
                Toggle("Animations", Key::DeskAnim),
                t("Breathing, tail and skin effects. Uses a little CPU."),
                t("Uses the nap time from Customize \u{2192} Options."),
                Section("POSITION"),
                Toggle("Sit on the taskbar", Key::DeskTaskbar),
                Toggle("Lock in place", Key::DeskLock),
                t("Drag to move (unless locked). On the taskbar it slides sideways."),
                Button("Reset position", Act::ResetDesk, 190.0),
            ],
            4 => vec![Toggle("Show lyrics", Key::ShowLyrics), Toggle("Audio visualizer", Key::Visualizer)],
            5 => vec![
                Toggle("Start with an empty shelf", Key::EmptyShelf),
                Toggle("Clipboard history", Key::ClipHistory),
                Slider("Items to keep", Key::ClipMax),
                Toggle("Remember after restart", Key::Remember),
            ],
            6 => vec![t("Reminders are coming in the next update.")],
            7 => vec![
                Section("HUD & POPUPS"),
                Toggle("Volume", Key::HudVolume),
                Toggle("Caps Lock", Key::HudCaps),
                Toggle("Wi-Fi", Key::HudWifi),
                Toggle("Headphones & audio devices", Key::HudDevice),
                Section("NOTIFICATIONS"),
                Toggle("Show notifications", Key::Notifications),
                Toggle("Show message preview", Key::MessagePreview),
            ],
            8 => self.social_items(),
            PAGE_STATS => {
                let mut v = vec![StatCard("LISTENING", "Total time listening on Spotify", stats::duration_words(st.listen_secs), Some(Act::ToggleYears))];
                if self.years_open {
                    v.push(YearBreakdown);
                }
                v.push(StatCard("PRESSES", "Total key presses and clicks", stats::thousands(st.presses), None));
                v.push(StatCard(
                    "STREAK",
                    "Days in a row with Fuzzie",
                    format!("{} day{}  \u{00B7}  best {}", st.streak, if st.streak == 1 { "" } else { "s" }, st.best_streak),
                    None,
                ));
                v.extend([Divider, FriendTable, RpsTable]);
                v
            }
            10 => {
                use crate::updater::Status;
                let status = crate::updater::status();
                let line = match &status {
                    Status::Idle => "Not checked yet".to_string(),
                    Status::Checking => "Checking\u{2026}".to_string(),
                    Status::UpToDate => "You're up to date".to_string(),
                    Status::Available(tag) => format!("{} is available", tag),
                    Status::Installing => "Installing\u{2026} Fuzzie will restart".to_string(),
                    Status::Failed => "Couldn't reach GitHub. Try again later.".to_string(),
                };
                let mut v = vec![
                    Row("Version", env!("CARGO_PKG_VERSION").to_string()),
                    Row("Status", line),
                    Button("Check for updates", Act::CheckUpdate, 210.0),
                ];
                if matches!(status, Status::Available(_)) {
                    v.push(Button("Install update", Act::InstallUpdate, 210.0));
                }
                v.extend([Section("AUTOMATIC"), Toggle("Update automatically", Key::AutoUpdate)]);
                v.push(t("New versions install by themselves within minutes of release."));
                v
            }
            _ => vec![
                MascotCard,
                Row("Version", env!("CARGO_PKG_VERSION").to_string()),
                t("A desktop notch for Windows: music, lyrics, shelf, clipboard and a fox."),
                Button("Open data folder", Act::OpenFolder, 190.0),
                Section("YOUR DATA"),
                t("Settings, stats, the shelf, clipboard history and your friends account."),
                Button(if self.armed(Act::DeleteData) { "Click again to delete" } else { "Delete all my data" }, Act::DeleteData, 230.0),
                t("Fuzzie starts again as if new. This cannot be undone."),
                Button(if self.armed(Act::Uninstall) { "Click again to uninstall" } else { "Uninstall Fuzzie" }, Act::Uninstall, 230.0),
                t("Deletes all your data, stops starting with Windows and removes Fuzzie.exe."),
            ],
        }
    }

    fn target(&mut self) -> Result<ID2D1HwndRenderTarget> {
        if let Some(rt) = &self.rt {
            return Ok(rt.clone());
        }
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U { width: rc.right as u32, height: rc.bottom as u32 },
                // Do not block the UI thread (and the notch's frames) waiting for vsync.
                presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
            };
            // Software: the page is drawn once into a cached bitmap and only the
            // two live foxes are redrawn, so the GPU driver (tens of MB) is not
            // worth loading. Everything is released when the window closes.
            let rt = self.factory.CreateHwndRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES { r#type: windows::Win32::Graphics::Direct2D::D2D1_RENDER_TARGET_TYPE_SOFTWARE, ..Default::default() }, &props)?;
            let dpi = 96.0 * scale_of(self.hwnd);
            rt.SetDpi(dpi, dpi);
            rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.rt = Some(rt.clone());
            Ok(rt)
        }
    }

    fn paint(&mut self) {
        let Some(s) = window::settings_snapshot() else { return };
        let Ok(rt) = self.target() else { return };
        let st = stats::snapshot();
        self.tick();
        let mut ok = true;
        unsafe {
            if self.moving {
                // Everything eases: draw it all straight to the window.
                self.dirty.set(true);
                self.hits.clear();
                self.skip_live = false;
                rt.BeginDraw();
                if let Err(e) = self.draw(&rt, &s, &st) {
                    eprintln!("settings draw failed: {e}");
                }
                ok = rt.EndDraw(None, None).is_ok();
            } else {
                if self.dirty.replace(false) || self.cache.is_none() {
                    if self.cache.is_none() {
                        self.cache = rt.CreateCompatibleRenderTarget(None, None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE).ok();
                    }
                    if let Some(c) = self.cache.clone() {
                        self.hits.clear();
                        self.skip_live = true;
                        self.stage = None;
                        c.BeginDraw();
                        if let Err(e) = self.draw(&c, &s, &st) {
                            eprintln!("settings draw failed: {e}");
                        }
                        ok = c.EndDraw(None, None).is_ok();
                        self.skip_live = false;
                    }
                }
                if let Some(bmp) = self.cache.as_ref().and_then(|c| c.GetBitmap().ok()) {
                    rt.BeginDraw();
                    rt.DrawBitmap(&bmp, None, 1.0, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None);
                    if let Err(e) = self.draw_live(&rt, &s) {
                        eprintln!("settings draw failed: {e}");
                    }
                    ok &= rt.EndDraw(None, None).is_ok();
                }
            }
            if !ok {
                // Device lost: rebuild the targets on the next paint.
                self.drop_targets();
            }
        }
        // Fast frames while something eases, ~30 fps for the two live foxes
        // (sidebar and Customize stage); the other foxes hold still and blink
        // from a timer. A minimised window gets no frames.
        let shown = unsafe { !IsIconic(self.hwnd).as_bool() };
        let front = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } == self.hwnd;
        self.set_frame(if !shown { 0 } else if self.moving { FRAME_FAST } else if front { FRAME_LIVE } else { 0 });
    }

    /// Steps the hover highlights and the page fade; the knobs and highlights
    /// step while they are drawn.
    fn tick(&mut self) {
        let now = Instant::now();
        let real = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.dt = if self.moving { real.min(0.05) } else { 1.0 / 60.0 };
        let dt = self.dt;
        let mut moving = false;
        if let Some(h) = self.hot {
            if !self.hover.iter().any(|e| e.0 == h) {
                self.hover.push((h, 0.0));
            }
        }
        let hot = self.hot;
        for e in &mut self.hover {
            moving |= approach(&mut e.1, if Some(e.0) == hot { 1.0 } else { 0.0 }, dt, 18.0);
        }
        self.hover.retain(|e| e.1 > 0.0 || Some(e.0) == hot);
        if let Some((a, v)) = self.flash {
            let v = v - dt / 0.9;
            self.flash = (v > 0.0).then_some((a, v));
            moving = true;
        }
        moving |= approach(&mut self.page_t, 1.0, dt, 9.0);
        moving |= approach(&mut self.nav_pos, self.page as f32, dt, 14.0);
        moving |= approach(&mut self.tab_pos, self.tab as f32, dt, 14.0);
        self.moving = moving;
    }

    /// Forgets the render targets and everything made for them.
    fn drop_targets(&mut self) {
        if let Some(c) = self.cache.take() {
            mascot::release(&c, &self.factory);
        }
        if let Some(rt) = self.rt.take() {
            mascot::release(&rt, &self.factory);
        }
        self.gradients.clear();
        self.linears.clear();
        SOLID.with(|b| b.borrow_mut().clear());
        self.dirty.set(true);
    }

    /// The two live foxes over the cached page: the sidebar one, and the
    /// Customize stage when it shows.
    fn draw_live(&mut self, rt: &ID2D1RenderTarget, s: &Settings) -> Result<()> {
        self.sidebar_fox(rt, s)?;
        if let Some(st) = self.stage {
            let clip = D2D_RECT_F { left: st.left.max(SIDEBAR), top: st.top.max(0.0), right: st.right.min(WIN_W), bottom: st.bottom.min(WIN_H) };
            if clip.right > clip.left && clip.bottom > clip.top {
                unsafe { rt.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                let r = self.stage_live(rt, s, st);
                unsafe { rt.PopAxisAlignedClip() };
                r?;
            }
        }
        Ok(())
    }

    fn sidebar_fox(&self, rt: &ID2D1RenderTarget, s: &Settings) -> Result<()> {
        // Clipped so wide accessories (wings, rings) stay clear of the name.
        unsafe {
            rt.PushAxisAlignedClip(&D2D_RECT_F { left: 0.0, top: 0.0, right: 76.0, bottom: 96.0 }, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        let r = mascot::draw(rt, &self.factory, s.skin, s.acc, 44.0, 56.0, 42.0, 1.0, false, self.time());
        unsafe {
            rt.PopAxisAlignedClip();
        }
        r
    }

    /// The moving part of the Customize stage: bokeh drifting up, and the fox.
    fn stage_live(&self, rt: &ID2D1RenderTarget, s: &Settings, stage: D2D_RECT_F) -> Result<()> {
        let t = self.time();
        let (sw, sh) = (stage.right - stage.left, stage.bottom - stage.top);
        let sx = (stage.left + stage.right) / 2.0;
        let floor = stage.bottom - 30.0;
        unsafe {
            for i in 0..8 {
                let u = (t * 0.045 + i as f32 * 0.137).fract();
                let bx = stage.left + ((i as f32 * 0.383 + 0.11).fract()) * sw + (t * 0.5 + i as f32).sin() * 6.0;
                let by = stage.bottom + 20.0 - u * (sh + 40.0);
                let br = 5.0 + (i % 3) as f32 * 5.0;
                let a = 0.09 * (u * std::f32::consts::PI).sin();
                rt.FillEllipse(&ellipse(D2D_RECT_F { left: bx - br, top: by - br, right: bx + br, bottom: by + br }), &Self::brush(rt, FG, a)?);
            }
        }
        let size = 158.0;
        mascot::draw(rt, &self.factory, s.skin, s.acc, sx, floor - 0.47 * size, size, 1.0, false, t)
    }

    fn set_frame(&mut self, ms: u32) {
        self.frame_ms = ms;
        self.clock.set(ms);
    }

    /// Starts frames for a tween that begins now.
    fn kick(&mut self) {
        if !self.moving {
            self.moving = true;
            self.last_frame = Instant::now();
        }
        self.set_frame(FRAME_FAST);
        self.invalidate();
    }

    /// Press feedback left on `act` (1 just pressed → 0).
    fn fl(&self, act: Act) -> f32 {
        self.flash.filter(|f| f.0 == act).map_or(0.0, |f| f.1)
    }

    /// Hover highlight of `act`, 0..1.
    fn hv(&self, act: Act) -> f32 {
        self.hover.iter().find(|e| e.0 == act).map_or(0.0, |e| e.1)
    }

    /// Eased value for `key` in `list`, stepped toward `target` (starts there).
    fn tween(list: &mut Vec<(Key, f32)>, key: Key, target: f32, dt: f32, moving: &mut bool) -> f32 {
        match list.iter_mut().find(|e| e.0 == key) {
            Some(e) => {
                *moving |= approach(&mut e.1, target, dt, 16.0);
                e.1
            }
            None => {
                list.push((key, target));
                target
            }
        }
    }

    fn time(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    /// How shut the foxes' eyes are.
    fn blink(&self) -> f32 {
        if self.eyes_shut { 1.0 } else { 0.0 }
    }

    /// Shuts the eyes for two frames (open again 120 ms later) and picks the next blink.
    fn on_blink_timer(&mut self) {
        self.blinks = self.blinks.wrapping_add(1);
        let shown = unsafe { !IsIconic(self.hwnd).as_bool() };
        if shown {
            self.eyes_shut = true;
            self.invalidate();
            unsafe { SetTimer(Some(self.hwnd), TIMER_EYES, 120, None) };
        }
        // 2.5..6.5 s, scrambled from the blink count.
        let h = self.blinks.wrapping_mul(2654435761) >> 16;
        unsafe { SetTimer(Some(self.hwnd), TIMER_BLINK, 2500 + h % 4000, None) };
    }

    fn on_eyes_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_EYES);
        }
        self.eyes_shut = false;
        self.invalidate();
    }

    /// The shared solid brush of `rt`, set to colour `c` with alpha `a`.
    fn brush(rt: &ID2D1RenderTarget, c: [f32; 3], a: f32) -> Result<ID2D1SolidColorBrush> {
        use windows::core::Interface;
        let key = rt.as_raw() as usize;
        let b = match SOLID.with(|b| b.borrow().iter().find(|e| e.0 == key).map(|e| e.1.clone())) {
            Some(b) => b,
            None => {
                let b = unsafe { rt.CreateSolidColorBrush(&rgba(c, a), None)? };
                SOLID.with(|s| {
                    let mut s = s.borrow_mut();
                    if s.len() >= 2 {
                        s.remove(0);
                    }
                    s.push((key, b.clone()));
                });
                b
            }
        };
        unsafe { b.SetColor(&rgba(c, a)) };
        Ok(b)
    }

    fn text_a(rt: &ID2D1RenderTarget, s: &str, f: &IDWriteTextFormat, r: D2D_RECT_F, c: [f32; 3], a: f32) -> Result<()> {
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            rt.DrawText(&wide, f, &r, &Self::brush(rt, c, a)?, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        }
        Ok(())
    }

    fn text(rt: &ID2D1RenderTarget, s: &str, f: &IDWriteTextFormat, r: D2D_RECT_F, c: [f32; 3]) -> Result<()> {
        Self::text_a(rt, s, f, r, c, 1.0)
    }

    fn rounded(rt: &ID2D1RenderTarget, r: D2D_RECT_F, radius: f32, c: [f32; 3], a: f32) -> Result<()> {
        unsafe {
            rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, &Self::brush(rt, c, a)?);
        }
        Ok(())
    }

    fn draw(&mut self, rt: &ID2D1RenderTarget, s: &Settings, st: &Stats) -> Result<()> {
        let accent = color_of(s, ColorTarget::Accent);
        self.accent_now = accent;
        unsafe {
            rt.Clear(Some(&D2D1_COLOR_F { r: BG[0], g: BG[1], b: BG[2], a: 1.0 }));
        }
        Self::rounded(rt, D2D_RECT_F { left: 0.0, top: 0.0, right: SIDEBAR, bottom: WIN_H }, 0.0, SIDE_BG, 1.0)?;
        if !self.skip_live {
            self.sidebar_fox(rt, s)?;
        }
        Self::text(rt, "Fuzzie", &self.f.logo, D2D_RECT_F { left: 79.0, top: 34.0, right: SIDEBAR, bottom: 70.0 }, FG)?;

        // Sidebar: group labels with a rule, then the pages. The highlight glides
        // to the selected page.
        let mut ys = [0.0f32; PAGES.len()];
        let mut y = 111.0; // centre of the first group label
        let mut group = "";
        for (i, (g, _)) in PAGES.iter().enumerate() {
            if *g != group {
                if !group.is_empty() {
                    y += 39.0 - 42.0;
                }
                group = g;
                Self::text(rt, g, &self.f.group, D2D_RECT_F { left: 24.0, top: y - 11.0, right: 140.0, bottom: y + 11.0 }, accent)?;
                let line_x = 34.0 + g.len() as f32 * 8.8;
                Self::rounded(rt, D2D_RECT_F { left: line_x, top: y, right: 228.0, bottom: y + 1.0 }, 0.0, FG, 0.1)?;
                y += 37.0;
            }
            ys[i] = y;
            y += 42.0;
        }
        let np = self.nav_pos.clamp(0.0, (PAGES.len() - 1) as f32);
        let (i0, i1) = (np.floor() as usize, (np.ceil() as usize).min(PAGES.len() - 1));
        let hy = lerp(ys[i0], ys[i1], np - i0 as f32);
        Self::rounded(rt, D2D_RECT_F { left: 14.0, top: hy - 20.0, right: 233.0, bottom: hy + 20.0 }, 8.0, FG, 0.07)?;
        Self::rounded(rt, D2D_RECT_F { left: 12.0, top: hy - 12.0, right: 16.0, bottom: hy + 12.0 }, 2.0, accent, 1.0)?;
        for (i, (_, name)) in PAGES.iter().enumerate() {
            let y = ys[i];
            let r = D2D_RECT_F { left: 14.0, top: y - 20.0, right: 233.0, bottom: y + 20.0 };
            let hot = self.hv(Act::Page(i));
            if hot > 0.005 && i != self.page {
                Self::rounded(rt, r, 8.0, FG, 0.04 * hot)?;
            }
            let sel = (1.0 - (i as f32 - np).abs()).clamp(0.0, 1.0);
            let c = mix(NAV, FG, sel.max(hot * 0.5));
            Self::text(rt, name, &self.f.nav, D2D_RECT_F { left: 33.0 + 3.0 * hot, ..r }, c)?;
            self.hits.push((r, Act::Page(i)));
        }

        // Content, clipped and scrolled.
        let clip = D2D_RECT_F { left: SIDEBAR, top: 0.0, right: WIN_W, bottom: WIN_H };
        unsafe {
            rt.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        // A new page rises a little and fades in from the background.
        let rise = (1.0 - ease(self.page_t)) * 16.0;
        let top = -self.scroll + rise;
        let title = D2D_RECT_F { left: LEFT - 4.0, top: top + 36.0, right: RIGHT - 40.0, bottom: top + 78.0 };
        Self::text(rt, PAGES[self.page].1, &self.f.title, title, FG)?;
        let mut y = top + CONTENT_TOP;
        for (i, item) in self.items(st).iter().enumerate() {
            y = self.draw_item(rt, s, st, item, y, i == 0, accent)?;
        }
        self.content_h = y - top + 40.0;
        // Content can shrink (e.g. switching tabs): keep the scroll in range.
        self.scroll = self.scroll.min((self.content_h - WIN_H).max(0.0));
        let veil = 1.0 - ease(self.page_t);
        if veil > 0.005 {
            Self::rounded(rt, clip, 0.0, BG, veil)?;
        }
        unsafe {
            rt.PopAxisAlignedClip();
        }
        // Scrollbar.
        if self.content_h > WIN_H {
            let track = WIN_H - 40.0;
            let th = (track * WIN_H / self.content_h).max(40.0);
            let ty = 20.0 + (track - th) * self.scroll / (self.content_h - WIN_H);
            Self::rounded(rt, D2D_RECT_F { left: WIN_W - 17.0, top: ty, right: WIN_W - 12.0, bottom: ty + th }, 2.5, FG, 0.28)?;
        }
        // Close button, fixed in the corner.
        let close = D2D_RECT_F { left: WIN_W - 53.0, top: 13.0, right: WIN_W - 21.0, bottom: 45.0 };
        let hot = self.hv(Act::Close);
        if hot > 0.005 {
            unsafe {
                rt.FillEllipse(&ellipse(close), &Self::brush(rt, FG, 0.1 * hot)?);
            }
        }
        Self::text(rt, "\u{E711}", &self.f.icon, close, mix(NAV, FG, hot))?;
        self.hits.push((close, Act::Close));
        Ok(())
    }

    fn toggle(&mut self, rt: &ID2D1RenderTarget, on: bool, cy: f32, act: Act, accent: [f32; 3]) -> Result<()> {
        let r = D2D_RECT_F { left: RIGHT - 56.0, top: cy - 15.0, right: RIGHT, bottom: cy + 15.0 };
        let hot = self.hv(act);
        // The knob slides and the track colour blends as the setting flips.
        let t = match act {
            Act::Toggle(k) => Self::tween(&mut self.toggles, k, if on { 1.0 } else { 0.0 }, self.dt, &mut self.moving),
            _ => on as u8 as f32,
        };
        let te = ease(t);
        Self::rounded(rt, r, 15.0, mix(OFF, accent, te), 1.0 - 0.15 * hot)?;
        let kx = lerp(r.left + 15.0, r.right - 15.0, te);
        // A little stretch mid-way, like a real switch.
        let stretch = 4.0 * (te * std::f32::consts::PI).sin();
        let kr = 11.0 + 0.8 * hot;
        let knob = D2D_RECT_F { left: kx - kr - stretch / 2.0, top: cy - kr, right: kx + kr + stretch / 2.0, bottom: cy + kr };
        Self::rounded(rt, knob, kr, [1.0; 3], 1.0)?;
        self.hits.push((r, act));
        Ok(())
    }

    fn button(&mut self, rt: &ID2D1RenderTarget, label: &str, r: D2D_RECT_F, act: Act) -> Result<()> {
        Self::rounded(rt, r, 8.0, FIELD, 1.0)?;
        let hot = self.hv(act);
        if hot > 0.005 {
            Self::rounded(rt, r, 8.0, FG, 0.07 * hot)?;
        }
        let f = self.fl(act);
        let label = if f > 0.0 {
            Self::rounded(rt, r, 8.0, self.accent_now, 0.35 * f)?;
            match act {
                Act::CopyColor(_) => "\u{2713} Copied",
                Act::PasteColor(_) => "\u{2713} Pasted",
                _ => label,
            }
        } else {
            label
        };
        Self::text(rt, label, &self.f.seg, r, FG)?;
        self.hits.push((r, act));
        Ok(())
    }

    /// Card with an accent tick and an upper-case label; returns the label centre.
    fn card(rt: &ID2D1RenderTarget, f: &Fonts, r: D2D_RECT_F, label: &str, accent: [f32; 3]) -> Result<f32> {
        Self::rounded(rt, r, 18.0, CARD, 1.0)?;
        let cy = r.top + 33.0;
        Self::rounded(rt, D2D_RECT_F { left: r.left, top: cy - 12.0, right: r.left + 4.0, bottom: cy + 12.0 }, 1.5, accent, 1.0)?;
        Self::text(rt, label, &f.stat_label, D2D_RECT_F { left: r.left + 25.0, top: cy - 11.0, right: r.right, bottom: cy + 11.0 }, accent)?;
        Ok(cy)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_item(&mut self, rt: &ID2D1RenderTarget, s: &Settings, st: &Stats, item: &Item, y: f32, first: bool, accent: [f32; 3]) -> Result<f32> {
        let width = RIGHT - LEFT;
        Ok(match item {
            Item::Section(name) => {
                let pad = if first { 22.0 } else { 24.0 };
                Self::text(rt, name, &self.f.section, D2D_RECT_F { left: LEFT, top: y + pad, right: RIGHT, bottom: y + pad + 22.0 }, DIM)?;
                Self::rounded(rt, D2D_RECT_F { left: LEFT, top: y + pad + 25.0, right: RIGHT, bottom: y + pad + 26.0 }, 0.0, FG, 0.12)?;
                y + pad + 29.0
            }
            Item::Toggle(label, key) => {
                Self::text(rt, label, &self.f.label, D2D_RECT_F { left: LEFT, top: y, right: RIGHT - 80.0, bottom: y + 48.0 }, FG)?;
                self.toggle(rt, get_bool(s, *key), y + 24.0, Act::Toggle(*key), accent)?;
                y + 48.0
            }
            Item::Segment(label, key, opts) => {
                Self::text(rt, label, &self.f.label, D2D_RECT_F { left: LEFT, top: y, right: RIGHT - 300.0, bottom: y + 52.0 }, FG)?;
                let per = match opts.len() {
                    2 => 110.0,
                    3 => 86.0,
                    _ => 70.0,
                };
                let total = per * opts.len() as f32 + 8.0;
                let bx = D2D_RECT_F { left: RIGHT - total, top: y + 6.0, right: RIGHT, bottom: y + 46.0 };
                Self::rounded(rt, bx, 10.0, FIELD, 1.0)?;
                let sel = get_choice(s, *key);
                let pos = Self::tween(&mut self.choices, *key, sel as f32, self.dt, &mut self.moving);
                let slot = |f: f32| D2D_RECT_F { left: bx.left + 4.0 + per * f, top: bx.top + 4.0, right: bx.left + 4.0 + per * (f + 1.0), bottom: bx.bottom - 4.0 };
                for i in 0..opts.len() {
                    let r = slot(i as f32);
                    let hot = self.hv(Act::Choose(*key, i));
                    if hot > 0.005 && i != sel {
                        Self::rounded(rt, r, 8.0, FG, 0.06 * hot)?;
                    }
                    self.hits.push((r, Act::Choose(*key, i)));
                }
                Self::rounded(rt, slot(pos), 8.0, FG, 0.16)?;
                for (i, o) in opts.iter().enumerate() {
                    let near = (1.0 - (i as f32 - pos).abs()).clamp(0.0, 1.0);
                    Self::text(rt, o, &self.f.seg, slot(i as f32), mix(DIM, FG, near))?;
                }
                y + 52.0
            }
            Item::Grid(label, key, opts) => {
                Self::text(rt, label, &self.f.label, D2D_RECT_F { left: LEFT, top: y, right: RIGHT, bottom: y + 40.0 }, FG)?;
                let sel = get_choice(s, *key);
                let cw = (width - 16.0) / 3.0;
                for (i, o) in opts.iter().enumerate() {
                    let (col, row) = ((i % 3) as f32, (i / 3) as f32);
                    let r = D2D_RECT_F { left: LEFT + col * (cw + 8.0), top: y + 44.0 + row * 48.0, right: LEFT + col * (cw + 8.0) + cw, bottom: y + 84.0 + row * 48.0 };
                    Self::rounded(rt, r, 8.0, FIELD, 1.0)?;
                    let hot = self.hv(Act::Choose(*key, i));
                    if i == sel || hot > 0.005 {
                        Self::rounded(rt, r, 8.0, FG, if i == sel { 0.08 } else { 0.05 * hot })?;
                    }
                    if i == sel {
                        unsafe {
                            rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: 8.0, radiusY: 8.0 }, &Self::brush(rt, accent, 1.0)?, 2.0, None);
                        }
                    }
                    Self::text(rt, o, &self.f.seg, r, if i == sel { FG } else { DIM })?;
                    self.hits.push((r, Act::Choose(*key, i)));
                }
                y + 44.0 + opts.len().div_ceil(3) as f32 * 48.0 + 4.0
            }
            Item::Slider(label, key) => {
                Self::text(rt, label, &self.f.label, D2D_RECT_F { left: LEFT, top: y, right: RIGHT, bottom: y + 48.0 }, FG)?;
                let cy = y + 25.0;
                let track = D2D_RECT_F { left: RIGHT - 323.0, top: cy - 2.0, right: RIGHT - 62.0, bottom: cy + 2.0 };
                let (f, label) = slider_value(s, *key);
                let done = D2D_RECT_F { right: track.left + (track.right - track.left) * f, ..track };
                Self::rounded(rt, track, 2.0, FG, 0.2)?;
                Self::rounded(rt, done, 2.0, accent, 1.0)?;
                let k = D2D_RECT_F { left: done.right - 10.0, top: cy - 10.0, right: done.right + 10.0, bottom: cy + 10.0 };
                unsafe {
                    rt.FillEllipse(&ellipse(k), &Self::brush(rt, [1.0; 3], 1.0)?);
                }
                Self::text(rt, &label, &self.f.value, D2D_RECT_F { left: RIGHT - 60.0, top: y, right: RIGHT - 4.0, bottom: y + 50.0 }, DIM)?;
                self.hits.push((D2D_RECT_F { top: cy - 14.0, bottom: cy + 14.0, ..track }, Act::Slider(*key)));
                y + 50.0
            }
            Item::Button(label, act, w) => {
                let r = D2D_RECT_F { left: LEFT, top: y + 12.0, right: LEFT + w, bottom: y + 51.0 };
                self.button(rt, label, r, *act)?;
                y + 60.0
            }
            Item::Colors => {
                let size = 52.0;
                let gap = 13.0;
                let per_row = ((width + gap) / (size + gap)).floor() as usize;
                let count = PRESETS.len() + 1;
                for i in 0..count {
                    let (col, row) = ((i % per_row) as f32, (i / per_row) as f32);
                    let l = LEFT + 3.0 + col * (size + gap);
                    let t = y + 13.0 + row * (size + gap);
                    let r = D2D_RECT_F { left: l, top: t, right: l + size, bottom: t + size };
                    let act = if i == PRESETS.len() { Act::EditColor(ColorTarget::Background) } else { Act::Preset(i) };
                    let hot = self.hv(act);
                    unsafe {
                        if i == PRESETS.len() {
                            let small = D2D_RECT_F { left: l + 4.0, top: t + 4.0, right: l + size - 4.0, bottom: t + size - 4.0 };
                            rt.DrawEllipse(&ellipse(small), &Self::brush(rt, FG, 0.45 + 0.25 * hot)?, 1.5, None);
                            Self::text(rt, "\u{E710}", &self.f.icon_big, small, FG)?;
                            self.hits.push((r, Act::EditColor(ColorTarget::Background)));
                            continue;
                        }
                        let (bg, ac) = PRESETS[i];
                        let (bgc, acc) = (settings::parse_hex(bg, [0.0; 3]), settings::parse_hex(ac, [1.0; 3]));
                        let selected = s.background.eq_ignore_ascii_case(bg) && s.accent.eq_ignore_ascii_case(ac);
                        let g = if selected { 4.0 } else { 1.5 * (1.0 - hot) };
                        let inner = D2D_RECT_F { left: l + g, top: t + g, right: l + size - g, bottom: t + size - g };
                        rt.FillEllipse(&ellipse(inner), &Self::brush(rt, bgc, 1.0)?);
                        let border = if luminance(bgc) > 0.6 { 0.0 } else { 0.22 };
                        rt.DrawEllipse(&ellipse(inner), &Self::brush(rt, FG, border + 0.25 * hot)?, 1.0, None);
                        let dot = D2D_RECT_F { left: l + 31.0, top: t + 31.0, right: l + 45.0, bottom: t + 45.0 };
                        rt.FillEllipse(&ellipse(dot), &Self::brush(rt, acc, 1.0)?);
                        if selected {
                            let ring = D2D_RECT_F { left: l - 0.5, top: t - 0.5, right: l + size + 0.5, bottom: t + size + 0.5 };
                            rt.DrawEllipse(&ellipse(ring), &Self::brush(rt, accent, 1.0)?, 3.0, None);
                        }
                    }
                    self.hits.push((r, Act::Preset(i)));
                }
                y + 13.0 + count.div_ceil(per_row) as f32 * (size + gap) + 4.0
            }
            Item::ColorRow(label, target) => {
                let cy = y + 26.0;
                Self::text(rt, label, &self.f.label, D2D_RECT_F { left: LEFT, top: cy - 20.0, right: RIGHT - 400.0, bottom: cy + 8.0 }, FG)?;
                let c = color_of(s, *target);
                let open = self.editing == Some(*target);
                let sw = D2D_RECT_F { left: RIGHT - 409.0, top: cy - 16.0, right: RIGHT - 377.0, bottom: cy + 16.0 };
                unsafe {
                    rt.FillEllipse(&ellipse(sw), &Self::brush(rt, c, 1.0)?);
                    let ring = if open { (accent, 2.5) } else { (DIM, 1.5) };
                    rt.DrawEllipse(&ellipse(sw), &Self::brush(rt, ring.0, 1.0)?, ring.1, None);
                }
                self.hits.push((sw, Act::EditColor(*target)));
                let field = D2D_RECT_F { left: RIGHT - 365.0, top: cy - 20.0, right: RIGHT - 177.0, bottom: cy + 19.0 };
                Self::rounded(rt, field, 8.0, FIELD, 1.0)?;
                Self::text(rt, &hex(c), &self.f.field, D2D_RECT_F { left: field.left + 13.0, ..field }, FG)?;
                self.hits.push((field, Act::EditColor(*target)));
                self.button(rt, "Copy", D2D_RECT_F { left: RIGHT - 167.0, top: field.top, right: RIGHT - 87.0, bottom: field.bottom }, Act::CopyColor(*target))?;
                self.button(rt, "Paste", D2D_RECT_F { left: RIGHT - 79.0, top: field.top, right: RIGHT, bottom: field.bottom }, Act::PasteColor(*target))?;
                if open {
                    self.picker(rt, *target, y + 59.0)?;
                    y + 59.0 + 209.0
                } else {
                    y + 52.0
                }
            }
            Item::Hero => self.hero(rt, s, y, accent)?,
            Item::Tabs => self.tabs(rt, y, accent)?,
            Item::SkinGrid => self.grid(rt, s, y, 0, accent)?,
            Item::AccGrid => self.grid(rt, s, y, 1, accent)?,
            Item::MascotCard => {
                let r = D2D_RECT_F { left: LEFT, top: y + 8.0, right: RIGHT, bottom: y + 128.0 };
                Self::rounded(rt, r, 18.0, CARD, 1.0)?;
                unsafe {
                    rt.PushAxisAlignedClip(&r, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                }
                mascot::draw_still(rt, &self.factory, s.skin, s.acc, LEFT + 80.0, y + 76.0, 80.0, 1.0, false, self.blink())?;
                unsafe {
                    rt.PopAxisAlignedClip();
                }
                let skin = mascot::get(s.skin);
                Self::text(rt, skin.name, &self.f.big, D2D_RECT_F { left: LEFT + 160.0, top: y + 40.0, right: RIGHT, bottom: y + 68.0 }, FG)?;
                let sub = if s.acc % mascot::ACCESSORIES.len() == 0 { "Your fox".to_string() } else { format!("Your fox  \u{00B7}  {}", mascot::acc_name(s.acc)) };
                Self::text(rt, &sub, &self.f.text, D2D_RECT_F { left: LEFT + 160.0, top: y + 68.0, right: RIGHT, bottom: y + 92.0 }, DIM)?;
                y + 136.0
            }
            Item::Text(t) => {
                Self::text(rt, t, &self.f.text, D2D_RECT_F { left: LEFT, top: y + 6.0, right: RIGHT, bottom: y + 32.0 }, DIM)?;
                y + 36.0
            }
            Item::Input(label, field) => {
                Self::text(rt, label, &self.f.label, D2D_RECT_F { left: LEFT, top: y, right: RIGHT - 300.0, bottom: y + 52.0 }, FG)?;
                let r = D2D_RECT_F { left: RIGHT - 280.0, top: y + 8.0, right: RIGHT, bottom: y + 46.0 };
                Self::rounded(rt, r, 8.0, FIELD, 1.0)?;
                let typing = self.typing.as_ref().filter(|t| t.0 == *field).map(|t| t.1.clone());
                if typing.is_some() {
                    Self::rounded(rt, r, 8.0, accent, 0.18)?;
                } else if self.hv(Act::Focus(*field)) > 0.005 {
                    Self::rounded(rt, r, 8.0, FG, 0.05 * self.hv(Act::Focus(*field)))?;
                }
                let (text, color) = match (typing, field) {
                    (Some(t), _) => {
                        let caret = self.started.elapsed().as_millis() / 530 % 2 == 0;
                        (format!("{t}{}", if caret { "|" } else { " " }), FG)
                    }
                    (None, Field::Name) if !s.social_name.is_empty() => (s.social_name.clone(), FG),
                    (None, Field::Name) => ("Fuzzie".to_string(), DIM),
                    (None, Field::Code) => ("FUZ-XXXXXX".to_string(), DIM),
                };
                Self::text(rt, &text, &self.f.field, D2D_RECT_F { left: r.left + 13.0, right: r.right - 8.0, ..r }, color)?;
                self.hits.push((r, Act::Focus(*field)));
                y + 54.0
            }
            Item::Person(name, sub, buttons) => {
                let mut x = RIGHT;
                for (label, act) in buttons.iter().rev() {
                    let r = D2D_RECT_F { left: x - 104.0, top: y + 9.0, right: x, bottom: y + 45.0 };
                    self.button(rt, label, r, *act)?;
                    x = r.left - 10.0;
                }
                Self::text(rt, name, &self.f.label, D2D_RECT_F { left: LEFT, top: y + 3.0, right: x - 8.0, bottom: y + 29.0 }, FG)?;
                Self::text(rt, sub, &self.f.text, D2D_RECT_F { left: LEFT, top: y + 28.0, right: x - 8.0, bottom: y + 50.0 }, DIM)?;
                y + 56.0
            }
            Item::Row(label, value) => {
                Self::text(rt, label, &self.f.label, D2D_RECT_F { left: LEFT, top: y, right: RIGHT, bottom: y + 48.0 }, FG)?;
                Self::text(rt, value, &self.f.value, D2D_RECT_F { left: LEFT, top: y, right: RIGHT, bottom: y + 48.0 }, DIM)?;
                y + 48.0
            }
            Item::StatCard(label, sub, value, act) => {
                let r = D2D_RECT_F { left: LEFT + 2.0, top: y + 8.0, right: RIGHT, bottom: y + 131.0 };
                if let Some(a) = act {
                    let hot = self.hv(*a);
                    self.hits.push((r, *a));
                    Self::card(rt, &self.f, r, label, accent)?;
                    if hot > 0.005 {
                        Self::rounded(rt, r, 18.0, FG, 0.03 * hot)?;
                    }
                } else {
                    Self::card(rt, &self.f, r, label, accent)?;
                }
                let x = r.left + 25.0;
                Self::text(rt, sub, &self.f.stat_sub, D2D_RECT_F { left: x, top: r.top + 46.0, right: r.right, bottom: r.top + 70.0 }, DIM)?;
                Self::text(rt, value, &self.f.stat_value, D2D_RECT_F { left: x, top: r.top + 78.0, right: r.right - 20.0, bottom: r.top + 118.0 }, FG)?;
                y + 141.0
            }
            Item::YearBreakdown => {
                let mut cy = y + 22.0;
                // Top songs and artists start over every New Year (local time).
                let this_year = stats::current_year();
                for (year, data) in st.years.iter().filter(|(y, _)| **y == this_year) {
                    let line = |cy: f32, x: f32| D2D_RECT_F { left: x, top: cy - 12.0, right: RIGHT, bottom: cy + 12.0 };
                    Self::text(rt, &format!("{year}: {}", stats::duration_words(data.secs)), &self.f.pink_head, line(cy, LEFT + 10.0), PINK)?;
                    cy += 27.0;
                    Self::text(rt, &format!("Top songs  ({} different songs)", data.songs.len()), &self.f.pink_sub, line(cy, LEFT + 27.0), PINK)?;
                    let mut songs: Vec<_> = data.songs.iter().collect();
                    songs.sort_by(|a, b| b.1.total_cmp(a.1));
                    for (i, (name, secs)) in songs.iter().take(5).enumerate() {
                        cy += 24.0;
                        Self::text(rt, &format!("{}. {}   {}", i + 1, name, stats::duration_words(**secs)), &self.f.pink_item, line(cy, LEFT + 35.0), PINK)?;
                    }
                    cy += 30.0;
                    Self::text(rt, "Top artists", &self.f.pink_sub, line(cy, LEFT + 27.0), PINK)?;
                    let mut artists: Vec<_> = data.artists.iter().collect();
                    artists.sort_by(|a, b| b.1.total_cmp(a.1));
                    for (i, (name, secs)) in artists.iter().take(5).enumerate() {
                        cy += 24.0;
                        Self::text(rt, &format!("{}. {}   {}", i + 1, name, stats::duration_words(**secs)), &self.f.pink_item, line(cy, LEFT + 35.0), PINK)?;
                    }
                    cy += 34.0;
                }
                if !st.years.contains_key(&this_year) {
                    Self::text(rt, "Nothing yet: play something on Spotify.", &self.f.pink_item, D2D_RECT_F { left: LEFT + 10.0, top: cy - 12.0, right: RIGHT, bottom: cy + 12.0 }, PINK)?;
                    cy += 30.0;
                }
                cy - 4.0
            }
            Item::Divider => {
                Self::rounded(rt, D2D_RECT_F { left: LEFT + 2.0, top: y + 26.0, right: RIGHT, bottom: y + 27.0 }, 0.0, FG, 0.12)?;
                y + 42.0
            }
            Item::FriendTable => {
                let r = D2D_RECT_F { left: LEFT + 2.0, top: y + 8.0, right: RIGHT, bottom: y + 297.0 };
                Self::card(rt, &self.f, r, "WITH YOUR FRIEND", accent)?;
                let (c1, c2) = (RIGHT - 210.0, RIGHT - 74.0);
                let cell = |cx: f32, cy: f32, h: f32| D2D_RECT_F { left: cx - 70.0, top: cy - h, right: cx + 70.0, bottom: cy + h };
                Self::text(rt, "Sent", &self.f.table_head, cell(c1, r.top + 61.0, 11.0), DIM)?;
                Self::text(rt, "Received", &self.f.table_head, cell(c2, r.top + 61.0, 11.0), DIM)?;
                let f = &st.friend;
                let rows = [("Bonks", f.bonks_sent, f.bonks_received), ("Kisses", f.kisses_sent, f.kisses_received), ("Pies", f.pies_sent, f.pies_received), ("Pokes", f.pokes_sent, f.pokes_received)];
                for (i, (name, sent, got)) in rows.iter().enumerate() {
                    let cy = r.top + 104.0 + i as f32 * 50.0;
                    if i == 3 {
                        Self::rounded(rt, D2D_RECT_F { left: r.left + 25.0, top: cy - 25.0, right: r.right - 25.0, bottom: cy - 24.0 }, 0.0, FG, 0.1)?;
                    }
                    Self::text(rt, name, &self.f.label, D2D_RECT_F { left: r.left + 25.0, top: cy - 14.0, right: c1 - 70.0, bottom: cy + 14.0 }, FG)?;
                    Self::text(rt, &stats::thousands(*sent), &self.f.table_val, cell(c1, cy, 15.0), FG)?;
                    Self::text(rt, &stats::thousands(*got), &self.f.table_val, cell(c2, cy, 15.0), FG)?;
                }
                y + 307.0
            }
            Item::RpsTable => {
                let r = D2D_RECT_F { left: LEFT + 2.0, top: y + 8.0, right: RIGHT, bottom: y + 223.0 };
                Self::card(rt, &self.f, r, "ROCK PAPER SCISSORS", accent)?;
                let f = &st.friend;
                let rows = [("Games played", f.rps_played), ("Wins", f.rps_won), ("Losses", f.rps_lost)];
                for (i, (name, v)) in rows.iter().enumerate() {
                    let cy = r.top + 80.0 + i as f32 * 50.0;
                    Self::text(rt, name, &self.f.label, D2D_RECT_F { left: r.left + 25.0, top: cy - 14.0, right: r.right, bottom: cy + 14.0 }, FG)?;
                    Self::text(rt, &stats::thousands(*v), &self.f.table_val, D2D_RECT_F { left: RIGHT - 144.0, top: cy - 15.0, right: RIGHT - 4.0, bottom: cy + 15.0 }, FG)?;
                }
                y + 233.0
            }
        })
    }

    /// Radial gradient brush, kept between frames (creating one is costly at 30 fps).
    #[allow(clippy::too_many_arguments)]
    fn radial(&mut self, rt: &ID2D1RenderTarget, cx: f32, cy: f32, rx: f32, ry: f32, stops: &[(f32, [f32; 3], f32)]) -> Result<ID2D1RadialGradientBrush> {
        let key = format!("{stops:?}");
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: Vector2 { X: cx, Y: cy }, gradientOriginOffset: Vector2 { X: 0.0, Y: 0.0 }, radiusX: rx, radiusY: ry };
        unsafe {
            if let Some(b) = self.gradients.get(&key) {
                b.SetCenter(props.center);
                b.SetRadiusX(rx);
                b.SetRadiusY(ry);
                b.SetOpacity(1.0);
                return Ok(b.clone());
            }
            let v: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|&(p, c, a)| D2D1_GRADIENT_STOP { position: p, color: rgba(c, a) }).collect();
            let coll = rt.CreateGradientStopCollection(&v, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let b = rt.CreateRadialGradientBrush(&props, None, &coll)?;
            if self.gradients.len() > 64 {
                self.gradients.clear();
            }
            self.gradients.insert(key, b.clone());
            Ok(b)
        }
    }

    /// Character screen: a stage with the big animated fox, and pickers beside it.
    fn hero(&mut self, rt: &ID2D1RenderTarget, s: &Settings, y: f32, accent: [f32; 3]) -> Result<f32> {
        let r = D2D_RECT_F { left: LEFT, top: y + 8.0, right: RIGHT, bottom: y + 318.0 };
        Self::rounded(rt, r, 18.0, CARD, 1.0)?;
        let stage = D2D_RECT_F { left: r.left + 12.0, top: r.top + 12.0, right: r.left + 300.0, bottom: r.bottom - 12.0 };
        let sx = (stage.left + stage.right) / 2.0;
        let floor = stage.bottom - 30.0;
        unsafe {
            let glow = self.radial(rt, sx, floor - 110.0, 200.0, 180.0, &[(0.0, mix3(accent, CARD, 0.45), 1.0), (0.6, mix3(accent, BG, 0.82), 1.0), (1.0, SIDE_BG, 1.0)])?;
            rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: stage, radiusX: 14.0, radiusY: 14.0 }, &glow);
            rt.PushAxisAlignedClip(&stage, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            // Floor: a spotlight and a soft shadow under the paws.
            let spot = self.radial(rt, sx, floor, 120.0, 22.0, &[(0.0, accent, 0.28), (1.0, accent, 0.0)])?;
            rt.FillEllipse(&ellipse(D2D_RECT_F { left: sx - 120.0, top: floor - 22.0, right: sx + 120.0, bottom: floor + 22.0 }), &spot);
            let shadow = self.radial(rt, sx, floor, 70.0, 10.0, &[(0.0, [0.0; 3], 0.45), (1.0, [0.0; 3], 0.0)])?;
            rt.FillEllipse(&ellipse(D2D_RECT_F { left: sx - 70.0, top: floor - 10.0, right: sx + 70.0, bottom: floor + 10.0 }), &shadow);
            // The bokeh and the fox move: drawn over the cached page every frame.
            if self.skip_live {
                self.stage = Some(stage);
            } else {
                self.stage_live(rt, s, stage)?;
            }
            rt.PopAxisAlignedClip();
        }

        // Pickers.
        let (px, pr) = (stage.right + 22.0, r.right - 22.0);
        let skin = mascot::get(s.skin);
        let (ns, na) = (mascot::SKINS.len(), mascot::ACCESSORIES.len());
        let sub = format!("{} / {}  \u{00B7}  {}", s.skin % ns + 1, ns, skin.tags());
        self.selector(rt, "SKIN", skin.name, &sub, px, pr, r.top + 22.0, Act::StepSkin(-1), Act::StepSkin(1), accent)?;
        let (aname, moving) = mascot::ACCESSORIES[s.acc % na];
        let sub = format!("{} / {}{}", s.acc % na + 1, na, if moving { "  \u{00B7}  Animated" } else { "" });
        self.selector(rt, "ACCESSORY", aname, &sub, px, pr, r.top + 122.0, Act::StepAcc(-1), Act::StepAcc(1), accent)?;

        let hot = self.hv(Act::Random);
        let lift = 2.0 * hot;
        let b = D2D_RECT_F { left: px, top: r.bottom - 66.0 - lift, right: pr, bottom: r.bottom - 22.0 - lift };
        Self::rounded(rt, b, 12.0, accent, 0.85 + 0.15 * hot)?;
        let f = self.fl(Act::Random);
        if f > 0.0 {
            Self::rounded(rt, b, 12.0, [1.0; 3], 0.3 * f)?;
        }
        let fg = if luminance(accent) > 0.6 { [0.08; 3] } else { [1.0; 3] };
        Self::text(rt, "\u{E8B1}", &self.f.icon, D2D_RECT_F { left: b.left + 18.0, right: b.left + 44.0, ..b }, fg)?;
        Self::text(rt, "Surprise me", &self.f.tab, D2D_RECT_F { left: b.left + 20.0, ..b }, fg)?;
        self.hits.push((b, Act::Random));
        Ok(y + 326.0)
    }

    /// "LABEL" over a ‹ name › stepper and a small caption.
    #[allow(clippy::too_many_arguments)]
    fn selector(&mut self, rt: &ID2D1RenderTarget, label: &str, name: &str, sub: &str, px: f32, pr: f32, top: f32, prev: Act, next: Act, accent: [f32; 3]) -> Result<()> {
        Self::text(rt, label, &self.f.stat_label, D2D_RECT_F { left: px + 2.0, top, right: pr, bottom: top + 20.0 }, accent)?;
        let row = D2D_RECT_F { left: px, top: top + 26.0, right: pr, bottom: top + 72.0 };
        Self::rounded(rt, row, 12.0, FIELD, 1.0)?;
        for (i, act) in [prev, next].into_iter().enumerate() {
            let x = if i == 0 { row.left + 5.0 } else { row.right - 41.0 };
            let b = D2D_RECT_F { left: x, top: row.top + 5.0, right: x + 36.0, bottom: row.bottom - 5.0 };
            Self::rounded(rt, b, 9.0, FG, 0.05 + 0.09 * self.hv(act))?;
            let f = self.fl(act);
            if f > 0.0 {
                Self::rounded(rt, b, 9.0, accent, 0.6 * f)?;
            }
            Self::text(rt, if i == 0 { "\u{E76B}" } else { "\u{E76C}" }, &self.f.icon, b, FG)?;
            self.hits.push((b, act));
        }
        Self::text(rt, name, &self.f.name_c, D2D_RECT_F { left: row.left + 42.0, right: row.right - 42.0, ..row }, FG)?;
        Self::text(rt, sub, &self.f.small_c, D2D_RECT_F { left: px, top: row.bottom + 4.0, right: pr, bottom: row.bottom + 24.0 }, DIM)?;
        Ok(())
    }

    fn tabs(&mut self, rt: &ID2D1RenderTarget, y: f32, accent: [f32; 3]) -> Result<f32> {
        let bx = D2D_RECT_F { left: LEFT, top: y + 14.0, right: RIGHT, bottom: y + 62.0 };
        Self::rounded(rt, bx, 13.0, FIELD, 1.0)?;
        let labels = [format!("Skins  {}", mascot::SKINS.len()), format!("Accessories  {}", mascot::ACCESSORIES.len() - 1), "Options".to_string()];
        let per = (bx.right - bx.left - 8.0) / 3.0;
        let slot = |f: f32| D2D_RECT_F { left: bx.left + 4.0 + per * f, top: bx.top + 4.0, right: bx.left + 4.0 + per * (f + 1.0), bottom: bx.bottom - 4.0 };
        for i in 0..labels.len() {
            let hot = self.hv(Act::Tab(i));
            if hot > 0.005 && i != self.tab {
                Self::rounded(rt, slot(i as f32), 10.0, FG, 0.07 * hot)?;
            }
        }
        let pos = self.tab_pos.clamp(0.0, 2.0);
        Self::rounded(rt, slot(pos), 10.0, accent, 1.0)?;
        let on_accent = if luminance(accent) > 0.6 { [0.08; 3] } else { [1.0; 3] };
        for (i, l) in labels.iter().enumerate() {
            let r = slot(i as f32);
            let near = (1.0 - (i as f32 - pos).abs()).clamp(0.0, 1.0);
            Self::text(rt, l, &self.f.tab, r, mix(NAV, on_accent, near))?;
            self.hits.push((r, Act::Tab(i)));
        }
        Ok(y + 70.0)
    }

    /// Cards for every skin (`tab` 0) or accessory (`tab` 1), each with a live fox.
    fn grid(&mut self, rt: &ID2D1RenderTarget, s: &Settings, y: f32, tab: usize, accent: [f32; 3]) -> Result<f32> {
        let (ns, na) = (mascot::SKINS.len(), mascot::ACCESSORIES.len());
        let n = if tab == 0 { ns } else { na };
        let (cols, gap, ch) = (4usize, 14.0, 172.0);
        let cw = (RIGHT - LEFT - gap * (cols - 1) as f32) / cols as f32;
        for i in 0..n {
            let (col, row) = ((i % cols) as f32, (i / cols) as f32);
            let l = LEFT + col * (cw + gap);
            let top = y + 10.0 + row * (ch + gap);
            if top + ch < 0.0 || top > WIN_H {
                continue;
            }
            let h = self.hv(if tab == 0 { Act::Skin(i) } else { Act::Acc(i) });
            let lift = 3.0 * h;
            let r = D2D_RECT_F { left: l, top: top - lift, right: l + cw, bottom: top + ch - lift };
            let selected = if tab == 0 { i == s.skin % ns } else { i == s.acc % na };
            Self::rounded(rt, r, 16.0, CARD, 1.0)?;
            if h > 0.01 {
                Self::rounded(rt, r, 16.0, FG, 0.05 * h)?;
            }
            let (cx, cy) = ((r.left + r.right) / 2.0, r.top + 78.0);
            unsafe {
                rt.PushAxisAlignedClip(&r, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                if selected || h > 0.01 {
                    let a = if selected { 0.4 } else { 0.22 * h };
                    let g = self.radial(rt, cx, cy, 70.0, 64.0, &[(0.0, accent, 1.0), (1.0, accent, 0.0)])?;
                    g.SetOpacity(a);
                    rt.FillEllipse(&ellipse(D2D_RECT_F { left: cx - 70.0, top: cy - 64.0, right: cx + 70.0, bottom: cy + 64.0 }), &g);
                }
                let (skin, acc) = if tab == 0 { (i, s.acc) } else { (s.skin, i) };
                mascot::draw_still(rt, &self.factory, skin, acc, cx, cy + 6.0, 72.0 + 8.0 * h, 1.0, false, self.blink())?;
                rt.PopAxisAlignedClip();
                if selected {
                    rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: 16.0, radiusY: 16.0 }, &Self::brush(rt, accent, 1.0)?, 2.5, None);
                    let c = D2D_RECT_F { left: r.right - 30.0, top: r.top + 8.0, right: r.right - 8.0, bottom: r.top + 30.0 };
                    rt.FillEllipse(&ellipse(c), &Self::brush(rt, accent, 1.0)?);
                    Self::text(rt, "\u{E73E}", &self.f.icon, c, if luminance(accent) > 0.6 { [0.08; 3] } else { [1.0; 3] })?;
                } else if h > 0.01 {
                    rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: 16.0, radiusY: 16.0 }, &Self::brush(rt, FG, 0.18 * h)?, 1.5, None);
                }
            }
            // A picked card: a ring pulses out from it.
            let f = self.fl(if tab == 0 { Act::Skin(i) } else { Act::Acc(i) });
            if f > 0.0 {
                let g = 7.0 * (1.0 - f);
                let pr = D2D_RECT_F { left: r.left - g, top: r.top - g, right: r.right + g, bottom: r.bottom + g };
                unsafe { rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: pr, radiusX: 16.0 + g, radiusY: 16.0 + g }, &Self::brush(rt, accent, f)?, 2.0, None) };
            }
            let (name, animated) = if tab == 0 { (mascot::SKINS[i].name, mascot::SKINS[i].animated()) } else { mascot::ACCESSORIES[i] };
            if animated {
                Self::text_a(rt, "\u{2726}", &self.f.text, D2D_RECT_F { left: r.left + 12.0, top: r.top + 8.0, right: r.left + 40.0, bottom: r.top + 30.0 }, accent, 0.9)?;
            }
            let nr = D2D_RECT_F { left: r.left + 6.0, top: r.bottom - 36.0, right: r.right - 6.0, bottom: r.bottom - 12.0 };
            Self::text(rt, name, &self.f.card_name, nr, if selected { FG } else { NAV })?;
            self.hits.push((r, if tab == 0 { Act::Skin(i) } else { Act::Acc(i) }));
        }
        Ok(y + 10.0 + n.div_ceil(cols) as f32 * (ch + gap))
    }

    /// Saturation/value square and hue bar for the colour being edited.
    fn picker(&mut self, rt: &ID2D1RenderTarget, target: ColorTarget, top: f32) -> Result<()> {
        let sv = D2D_RECT_F { left: LEFT, top, right: LEFT + 400.0, bottom: top + 161.0 };
        let hue = D2D_RECT_F { left: LEFT, top: top + 175.0, right: LEFT + 400.0, bottom: top + 193.0 };
        let (h, s, v) = self.hsv;
        let linears = &mut self.linears;
        unsafe {
            // Kept between frames: the stops never change, only the positions.
            let mut grad = |stops: &[D2D1_GRADIENT_STOP], from: Vector2, to: Vector2| -> Result<ID2D1LinearGradientBrush> {
                let key = format!("{:?}", stops.iter().map(|s| (s.position, s.color.r, s.color.g, s.color.b, s.color.a)).collect::<Vec<_>>());
                if let Some(b) = linears.get(&key) {
                    b.SetStartPoint(from);
                    b.SetEndPoint(to);
                    return Ok(b.clone());
                }
                let coll = rt.CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
                let b = rt.CreateLinearGradientBrush(&D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: from, endPoint: to }, None, &coll)?;
                linears.insert(key, b.clone());
                Ok(b)
            };
            let stop = |p: f32, c: [f32; 3], a: f32| D2D1_GRADIENT_STOP { position: p, color: rgba(c, a) };
            rt.FillRectangle(&sv, &Self::brush(rt, hsv_to_rgb(h, 1.0, 1.0), 1.0)?);
            let white = grad(&[stop(0.0, [1.0; 3], 1.0), stop(1.0, [1.0; 3], 0.0)], Vector2 { X: sv.left, Y: 0.0 }, Vector2 { X: sv.right, Y: 0.0 })?;
            rt.FillRectangle(&sv, &white);
            let black = grad(&[stop(0.0, [0.0; 3], 0.0), stop(1.0, [0.0; 3], 1.0)], Vector2 { X: 0.0, Y: sv.top }, Vector2 { X: 0.0, Y: sv.bottom })?;
            rt.FillRectangle(&sv, &black);
            let hs: Vec<_> = (0..=6).map(|i| stop(i as f32 / 6.0, hsv_to_rgb(i as f32 * 60.0, 1.0, 1.0), 1.0)).collect();
            let rainbow = grad(&hs, Vector2 { X: hue.left, Y: 0.0 }, Vector2 { X: hue.right, Y: 0.0 })?;
            rt.FillRectangle(&hue, &rainbow);

            let (kx, ky) = (sv.left + s * (sv.right - sv.left), sv.top + (1.0 - v) * (sv.bottom - sv.top));
            let knob = D2D_RECT_F { left: kx - 8.0, top: ky - 8.0, right: kx + 8.0, bottom: ky + 8.0 };
            rt.DrawEllipse(&ellipse(knob), &Self::brush(rt, [0.0; 3], 0.6)?, 4.5, None);
            rt.DrawEllipse(&ellipse(knob), &Self::brush(rt, [1.0; 3], 1.0)?, 2.5, None);
            let hx = hue.left + h / 360.0 * (hue.right - hue.left);
            let marker = D2D_RECT_F { left: hx - 3.0, top: hue.top - 3.0, right: hx + 3.0, bottom: hue.bottom + 3.0 };
            Self::rounded(rt, marker, 3.0, [1.0; 3], 1.0)?;
        }
        self.hits.push((sv, Act::Sv(target)));
        self.hits.push((D2D_RECT_F { top: hue.top - 4.0, bottom: hue.bottom + 4.0, ..hue }, Act::Hue(target)));
        Ok(())
    }

    fn logical(&self, lp: LPARAM) -> (f32, f32) {
        let s = scale_of(self.hwnd);
        (((lp.0 & 0xffff) as i16) as f32 / s, (((lp.0 >> 16) & 0xffff) as i16) as f32 / s)
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<usize> {
        self.hits.iter().rposition(|(r, _)| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom)
    }

    fn invalidate(&self) {
        self.dirty.set(true);
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Applies a drag on a slider or the colour picker at (x, y).
    fn drag_to(&mut self, r: D2D_RECT_F, act: Act, x: f32, y: f32) {
        let fx = ((x - r.left) / (r.right - r.left)).clamp(0.0, 1.0);
        let fy = ((y - r.top) / (r.bottom - r.top)).clamp(0.0, 1.0);
        match act {
            Act::Slider(k) => window::change_setting(k, |s| set_slider(s, k, fx)),
            Act::Sv(t) => {
                self.hsv.1 = fx;
                self.hsv.2 = 1.0 - fy;
                set_color(t, hsv_to_rgb(self.hsv.0, self.hsv.1, self.hsv.2));
            }
            Act::Hue(t) => {
                self.hsv.0 = fx * 360.0;
                set_color(t, hsv_to_rgb(self.hsv.0, self.hsv.1, self.hsv.2));
            }
            _ => {}
        }
    }

    /// A dangerous button clicked once in the last 5 seconds.
    fn armed(&self, act: Act) -> bool {
        self.confirm.is_some_and(|(a, t)| a == act && t.elapsed() < std::time::Duration::from_secs(5))
    }

    /// The Social page.
    fn social_items(&self) -> Vec<Item> {
        use crate::social::Conn;
        use Item::*;
        let so = crate::social::snapshot();
        let mut v = vec![Section("YOU"), Toggle("Online with friends", Key::SocialOn)];
        let code = match so.conn {
            Conn::Online if !so.code.is_empty() => so.code.clone(),
            Conn::Off => "Offline".into(),
            Conn::Retrying => "Can't reach the Fuzzie server, retrying\u{2026}".into(),
            _ => "Connecting\u{2026}".into(),
        };
        v.push(Row("Your friend code", code));
        if !so.code.is_empty() {
            v.push(Button("Copy my code", Act::CopyCode, 190.0));
        }
        v.push(Input("Name", Field::Name));
        v.push(Toggle("Share what I'm playing", Key::ShareMusic));
        v.push(Section("ADD A FRIEND"));
        v.push(Input("Friend code", Field::Code));
        v.push(Button("Send request", Act::SendRequest, 190.0));
        let fresh = |t: Instant| t.elapsed() < std::time::Duration::from_secs(8);
        let error = so.error.as_ref().filter(|e| fresh(e.1));
        let note = self.note.as_ref().filter(|n| fresh(n.1));
        match (error, note) {
            (Some(e), Some(n)) if n.1 > e.1 => v.push(Text(n.0.clone())),
            (Some(e), _) => v.push(Text(e.0.clone())),
            (None, Some(n)) => v.push(Text(n.0.clone())),
            _ => {}
        }
        if !so.incoming.is_empty() || !so.outgoing.is_empty() {
            v.push(Section("REQUESTS"));
            for (i, (c, n)) in so.incoming.iter().enumerate() {
                v.push(Person(n.clone(), format!("{c}  \u{00B7}  wants to be friends"), vec![("Accept", Act::Accept(i)), ("Decline", Act::Decline(i))]));
            }
            for (i, (c, n)) in so.outgoing.iter().enumerate() {
                v.push(Person(n.clone(), format!("{c}  \u{00B7}  waiting for them"), vec![("Cancel", Act::Cancel(i))]));
            }
        }
        v.push(Section("FRIENDS"));
        if so.friends.is_empty() {
            v.push(Text("No friends yet: share your code, or add theirs above.".into()));
        }
        for (i, f) in so.friends.iter().enumerate() {
            let sub = match (&f.np, f.online) {
                (Some(t), true) => format!("{}  \u{00B7}  \u{266A} {} \u{2014} {}", f.code, t.title, t.artist),
                (None, true) => format!("{}  \u{00B7}  online", f.code),
                _ => format!("{}  \u{00B7}  offline", f.code),
            };
            v.push(Person(f.name.clone(), sub, vec![("Remove", Act::Remove(i))]));
        }
        v.push(Text("Friends see your name, your Fuzzie and, if shared, the song you play.".into()));
        v
    }

    fn on_char(&mut self, c: u32) {
        let Some((field, mut buf)) = self.typing.clone() else { return };
        match c {
            0x08 => {
                buf.pop();
            }
            0x0D => {
                self.commit_typing();
                return;
            }
            0x1B => {
                self.typing = None;
                self.invalidate();
                return;
            }
            // Ctrl+V
            0x16 => {
                if let Some(t) = clipboard::get_text(self.hwnd) {
                    buf.extend(t.chars().filter(|c| !c.is_control()));
                }
            }
            c if c >= 0x20 => {
                if let Some(ch) = char::from_u32(c) {
                    buf.push(ch);
                }
            }
            _ => return,
        }
        let buf: String = buf.chars().take(field.max()).collect();
        self.typing = Some((field, buf));
        self.invalidate();
    }

    /// Saves the name or sends the friend request typed in.
    fn commit_typing(&mut self) {
        let Some((field, buf)) = self.typing.take() else { return };
        match field {
            Field::Name => {
                let name = crate::social::clean_name(&buf);
                if !name.is_empty() {
                    window::change_setting(Key::SocialName, |s| s.social_name = name);
                }
            }
            Field::Code if buf.trim().is_empty() => {}
            Field::Code => match crate::social::clean_code(&buf) {
                Some(c) => {
                    crate::social::add(&c);
                    self.note = Some((format!("Request sent to {c}"), Instant::now()));
                }
                None => self.note = Some(("Friend codes look like FUZ-A4G7B9".into(), Instant::now())),
            },
        }
        self.invalidate();
    }

    fn on_down(&mut self, lp: LPARAM) {
        let (x, y) = self.logical(lp);
        // Clicking anywhere but the box being typed in (or Send) finishes typing.
        if let Some(field) = self.typing.as_ref().map(|t| t.0) {
            let hit = self.hit_at(x, y).map(|i| self.hits[i].1);
            if hit != Some(Act::Focus(field)) && hit != Some(Act::SendRequest) {
                self.commit_typing();
            }
        }
        let Some(i) = self.hit_at(x, y) else { return };
        let (r, act) = self.hits[i];
        let current = window::settings_snapshot();
        if !matches!(act, Act::Page(_) | Act::Slider(_) | Act::Sv(_) | Act::Hue(_) | Act::Tab(_)) {
            self.flash = Some((act, 1.0));
            self.kick();
        }
        match act {
            Act::Page(p) => {
                if p != self.page {
                    self.page = p;
                    self.scroll = 0.0;
                    self.page_t = 0.0;
                    self.hover.clear();
                    self.kick();
                }
            }
            Act::Close => self.want_close = true,
            Act::Toggle(k) => {
                window::change_setting(k, |s| {
                    let v = !get_bool(s, k);
                    set_bool(s, k, v);
                });
            }
            Act::Choose(k, c) => window::change_setting(k, |s| set_choice(s, k, c)),
            Act::Slider(_) | Act::Sv(_) | Act::Hue(_) => {
                self.dragging = Some((r, act));
                unsafe {
                    SetCapture(self.hwnd);
                }
                self.drag_to(r, act, x, y);
            }
            Act::Preset(p) => {
                let (bg, ac) = PRESETS[p];
                window::change_setting(Key::Colors, |s| {
                    s.background = bg.into();
                    s.accent = ac.into();
                });
                self.editing = None;
            }
            Act::EditColor(t) => {
                if self.editing == Some(t) {
                    self.editing = None;
                } else if let Some(s) = &current {
                    self.editing = Some(t);
                    self.hsv = rgb_to_hsv(color_of(s, t));
                }
            }
            Act::CopyColor(t) => {
                if let Some(s) = &current {
                    clipboard::set_text(self.hwnd, &hex(color_of(s, t)));
                }
            }
            Act::PasteColor(t) => {
                if let Some(text) = clipboard::get_text(self.hwnd) {
                    let t_hex = text.trim().trim_start_matches('#');
                    if t_hex.len() == 6 && t_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                        let c = settings::parse_hex(&format!("#{t_hex}"), [0.0; 3]);
                        set_color(t, c);
                        if self.editing == Some(t) {
                            self.hsv = rgb_to_hsv(c);
                        }
                    }
                }
            }
            Act::RestoreColors => {
                window::change_setting(Key::Colors, |s| {
                    s.background = DEFAULT_BACKGROUND.into();
                    s.accent = DEFAULT_ACCENT.into();
                });
                if let Some(t) = self.editing {
                    if let Some(s) = window::settings_snapshot() {
                        self.hsv = rgb_to_hsv(color_of(&s, t));
                    }
                }
            }
            Act::ResetPosition => window::change_setting(Key::Placement, |s| {
                s.placement = Placement::TopCenter;
                s.style = NotchStyle::Attached;
                s.custom_x = -1;
                s.custom_y = -1;
            }),
            Act::ResetDesk => crate::deskfox::reset_position(),
            Act::Skin(k) => window::change_setting(Key::Skin, |s| s.skin = k),
            Act::Acc(k) => window::change_setting(Key::Skin, |s| s.acc = k),
            Act::Tab(k) => {
                if k != self.tab {
                    self.tab = k;
                    self.page_t = 0.3;
                    self.kick();
                }
            }
            Act::StepSkin(d) => window::change_setting(Key::Skin, |s| s.skin = (s.skin as i64 + d).rem_euclid(mascot::SKINS.len() as i64) as usize),
            Act::StepAcc(d) => window::change_setting(Key::Skin, |s| s.acc = (s.acc as i64 + d).rem_euclid(mascot::ACCESSORIES.len() as i64) as usize),
            Act::Random => {
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
                window::change_setting(Key::Skin, |s| {
                    let (ns, na) = (mascot::SKINS.len(), mascot::ACCESSORIES.len());
                    // Always land on something new.
                    s.skin = (s.skin % ns + 1 + random_below(seed, ns - 1)) % ns;
                    s.acc = (s.acc % na + 1 + random_below(seed / 977, na - 1)) % na;
                });
            }
            Act::OpenFolder => window::open_data_folder(),
            Act::Focus(f) => {
                if self.typing.as_ref().map(|t| t.0) != Some(f) {
                    let start = match f {
                        Field::Name => current.as_ref().map(|c| c.social_name.clone()).unwrap_or_default(),
                        Field::Code => String::new(),
                    };
                    self.typing = Some((f, start));
                }
                self.invalidate();
            }
            Act::CopyCode => {
                let code = crate::social::snapshot().code;
                if !code.is_empty() {
                    clipboard::set_text(self.hwnd, &code);
                    self.note = Some((format!("Copied {code}: send it to a friend"), Instant::now()));
                }
            }
            Act::SendRequest => {
                if self.typing.as_ref().map(|t| t.0) == Some(Field::Code) {
                    self.commit_typing();
                } else {
                    self.note = Some(("Type your friend's code in the box first".into(), Instant::now()));
                    self.invalidate();
                }
            }
            Act::Accept(i) => {
                if let Some((c, _)) = crate::social::snapshot().incoming.get(i) {
                    crate::social::accept(c);
                }
            }
            Act::Decline(i) => {
                if let Some((c, _)) = crate::social::snapshot().incoming.get(i) {
                    crate::social::decline(c);
                }
            }
            Act::Cancel(i) => {
                if let Some((c, _)) = crate::social::snapshot().outgoing.get(i) {
                    crate::social::decline(c);
                }
            }
            Act::Remove(i) => {
                if let Some(f) = crate::social::snapshot().friends.get(i) {
                    crate::social::remove(&f.code);
                }
            }
            Act::DeleteData | Act::Uninstall => {
                if self.armed(act) {
                    self.confirm = None;
                    if act == Act::DeleteData {
                        crate::wipe::delete_data();
                    } else {
                        crate::wipe::uninstall();
                    }
                } else {
                    self.confirm = Some((act, Instant::now()));
                    self.invalidate();
                }
            }
            Act::CheckUpdate => crate::updater::check_now(),
            Act::InstallUpdate => crate::updater::install_now(),
            Act::ToggleYears => self.years_open = !self.years_open,
        }
        // Knobs, highlights and fades ease toward the new state.
        self.kick();
    }

    fn on_move(&mut self, lp: LPARAM) {
        let (x, y) = self.logical(lp);
        if let Some((r, act)) = self.dragging {
            self.drag_to(r, act, x, y);
            self.invalidate();
            return;
        }
        if !self.tracking {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
            self.tracking = true;
        }
        let hot = self.hit_at(x, y).map(|i| self.hits[i].1);
        if hot != self.hot {
            self.hot = hot;
            self.kick();
        }
    }

    fn on_wheel(&mut self, wp: WPARAM) {
        let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
        let max = (self.content_h - WIN_H).max(0.0);
        self.scroll = (self.scroll - delta / 120.0 * 70.0).clamp(0.0, max);
        self.invalidate();
    }

    /// Lets the window be dragged by its empty top strip.
    fn hit_test(&self, lp: LPARAM) -> LRESULT {
        let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut pt);
        }
        let s = scale_of(self.hwnd);
        let (x, y) = (pt.x as f32 / s, pt.y as f32 / s);
        if y < 90.0 && self.hit_at(x, y).is_none() {
            LRESULT(HTCAPTION as isize)
        } else {
            LRESULT(HTCLIENT as isize)
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled = match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            with_win(|w| w.paint());
            let _ = EndPaint(hwnd, &ps);
            Some(LRESULT(0))
        }
        WM_SIZE => {
            with_win(|w| {
                if let Some(rt) = &w.rt {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let _ = rt.Resize(&D2D_SIZE_U { width: rc.right as u32, height: rc.bottom as u32 });
                }
                if let Some(c) = w.cache.take() {
                    mascot::release(&c, &w.factory);
                }
                w.dirty.set(true);
            });
            Some(LRESULT(0))
        }
        WM_DPICHANGED => {
            with_win(|w| w.drop_targets());
            let r = &*(lp.0 as *const RECT);
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER);
            Some(LRESULT(0))
        }
        WM_TIMER => with_win(|w| match wp.0 {
            TIMER_BLINK => w.on_blink_timer(),
            TIMER_EYES => w.on_eyes_timer(),
            _ if w.page == PAGE_STATS || w.page == PAGE_UPDATES || w.page == PAGE_SOCIAL || w.confirm.is_some() => w.invalidate(),
            _ => {}
        })
        .map(|_| LRESULT(0)),
        // Coming to the front restarts the live foxes; going behind stops them.
        windows::Win32::UI::WindowsAndMessaging::WM_ACTIVATE => {
            with_win(|w| w.invalidate());
            None
        }
        WM_FRAME => with_win(|w| {
            w.clock.consumed();
            if w.moving {
                w.invalidate();
            } else {
                // Only the live foxes move: the cached page stays.
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        })
        .map(|_| LRESULT(0)),
        WM_NCHITTEST => with_win(|w| w.hit_test(lp)),
        WM_LBUTTONDOWN => with_win(|w| w.on_down(lp)).map(|_| LRESULT(0)),
        windows::Win32::UI::WindowsAndMessaging::WM_CHAR => with_win(|w| w.on_char(wp.0 as u32)).map(|_| LRESULT(0)),
        WM_MOUSEMOVE => with_win(|w| w.on_move(lp)).map(|_| LRESULT(0)),
        WM_LBUTTONUP => with_win(|w| {
            if w.dragging.take().is_some() {
                let _ = ReleaseCapture();
            }
        })
        .map(|_| LRESULT(0)),
        WM_MOUSELEAVE => with_win(|w| {
            w.tracking = false;
            w.hot = None;
            w.kick();
        })
        .map(|_| LRESULT(0)),
        WM_MOUSEWHEEL => with_win(|w| w.on_wheel(wp)).map(|_| LRESULT(0)),
        WM_DESTROY => {
            WIN.with(|c| {
                if let Ok(mut g) = c.try_borrow_mut() {
                    *g = None;
                }
            });
            // The window's memory is free again: hand the pages back to Windows.
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
            Some(LRESULT(0))
        }
        _ => None,
    };
    // Destroying the window runs with no borrow held.
    if with_win(|w| std::mem::take(&mut w.want_close)) == Some(true) {
        let _ = DestroyWindow(hwnd);
    }
    handled.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
}

/// Opens the window on a page, scrolled down by `scroll` units, with a
/// character-screen tab (for testing).
pub fn open_at(page: usize, scroll: f32, years: bool, tab: usize) {
    open();
    with_win(|w| {
        w.page = page.min(PAGES.len() - 1);
        w.nav_pos = w.page as f32;
        w.scroll = scroll;
        w.years_open = years;
        w.tab = tab.min(2);
        w.tab_pos = w.tab as f32;
        w.invalidate();
    });
}

/// Repaints the settings window, if open (after a change from elsewhere).
pub fn refresh() {
    with_win(|w| w.invalidate());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        for c in [[0.79, 0.01, 0.94], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.2, 0.6, 0.3]] {
            let (h, s, v) = rgb_to_hsv(c);
            let back = hsv_to_rgb(h, s, v);
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 0.002, "{c:?} -> {back:?}");
            }
        }
        assert_eq!(hex([0.788, 0.008, 0.941]), "#C902F0");
    }
}

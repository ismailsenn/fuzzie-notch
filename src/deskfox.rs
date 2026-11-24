//! Fuzzie on the desktop, Bongo Cat style: the customised fox sits on the
//! taskbar and taps it with one paw, then the other, on every key press and
//! click. It is a small layered window; clicks on its transparent pixels go
//! through, and it can be dragged (along the taskbar, or anywhere).
//!
//! The fox holds still and moves the way Bongo Cat does, in poses rather than
//! tweens: a tap is two frames (paw down, then up 110 ms later), a blink two
//! frames (eyes shut, then open). Falling asleep or waking up is one frame.
//! Nothing runs in between, so the window costs nothing while you are idle.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Instant;


use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{ID2D1DCRenderTarget, ID2D1Factory};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, MonitorFromPoint, ReleaseDC,
    SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassExW, SetTimer, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, IDC_HAND, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_numerics::Matrix3x2;

use crate::mascot;
use crate::settings::Settings;
use crate::window;

/// Head width in logical pixels at 100 % size; the window is sized around it.
const BASE_SIZE: f32 = 100.0;
/// Window size and the fox's centre, in head widths.
const W_K: f32 = 1.9;
const H_K: f32 = 1.85;
const FOX_Y_K: f32 = 1.15;
/// Where a pressed paw lands, in head widths below the centre: the taskbar's top edge.
const PAW_LINE_K: f32 = 0.47;

const WM_DESK_HIT: u32 = WM_APP + 7;
const WM_DESK_RESYNC: u32 = WM_APP + 8;
/// The next blink; lifting the left / right paw; the eyes opening (or shutting again).
const TIMER_BLINK: usize = 1;
const TIMER_PAW: usize = 2; // + side
const TIMER_EYES: usize = 4;
/// Frames for the animations, or only for the drifting "zZZ" while asleep.
const TIMER_ANIM: usize = 5;
/// Ends the see-through spell of a right-click.
const TIMER_GHOST: usize = 6;
const ANIM_MS: u32 = 33;
const ZZZ_MS: u32 = 50;
const PAW_MS: u32 = 110;
const BLINK_MS: u32 = 120;

/// The window, for the input hooks (0 = no desktop fox).
static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
/// Hook that tells us when another window (the taskbar, say) comes forward.
static FG_HOOK: AtomicIsize = AtomicIsize::new(0);
/// Steps back above the taskbar shortly after something else came forward.
const TIMER_RAISE: usize = 7;
/// Re-asserts "above the taskbar" every few seconds, in case a hook was missed.
const TIMER_KEEP_UP: usize = 8;
const KEEP_UP_MS: u32 = 3000;

/// Another window became the foreground one: the taskbar may now cover the
/// fox, so put it back on top a moment later (after the shell reorders).
unsafe extern "system" fn on_foreground(
    _: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let raw = HWND_RAW.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe { SetTimer(Some(HWND(raw as *mut core::ffi::c_void)), TIMER_RAISE, 60, None) };
    }
}

fn raise(hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}
static ON_KEYS: AtomicBool = AtomicBool::new(true);
static ON_CLICKS: AtomicBool = AtomicBool::new(true);
/// Locked in place: dragging does nothing.
static LOCKED: AtomicBool = AtomicBool::new(false);
/// How long a right-click leaves the fox see-through (ms).
static GHOST_MS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(2000);

struct Desk {
    hwnd: HWND,
    factory: ID2D1Factory,
    rt: ID2D1DCRenderTarget,
    dc: HDC,
    bitmap: HBITMAP,
    bits: *mut u8,
    old: HGDIOBJ,
    width: i32,
    height: i32,
    scale: f32,
    /// Head width in logical pixels.
    size: f32,
    /// Each paw (left, right) is down on the taskbar.
    down: [bool; 2],
    next: usize,
    eyes_shut: bool,
    /// Eye changes left in the current blink (3 for a double blink).
    blink_left: u8,
    blinks: u32,
    skin: usize,
    acc: usize,
    blink_on: bool,
    nap_on: bool,
    anim_on: bool,
    /// Right-clicked: see-through, letting clicks pass to what is behind.
    ghost: bool,
    glow_on: bool,
    acc_on: bool,
    /// Interval of TIMER_ANIM now (0 = off).
    anim_ms: u32,
    started: Instant,
    on_taskbar: bool,
    sleeping: bool,
    hidden: bool,
    pos: POINT,
    /// Dragging: cursor and window position when the button went down.
    drag: Option<(POINT, POINT)>,
    raised: Instant,
    /// Paws to draw instead of the live ones (for `--snapshot`).
    forced: Option<[f32; 2]>,
}

thread_local! {
    static DESK: RefCell<Option<Desk>> = const { RefCell::new(None) };
}

fn with_desk<R>(f: impl FnOnce(&mut Desk) -> R) -> Option<R> {
    DESK.with(|d| d.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

/// A key press (`click` false) or a click: the next paw taps. Called from the input hooks.
pub fn hit(click: bool) {
    let h = HWND_RAW.load(Ordering::Relaxed);
    let wanted = if click { ON_CLICKS.load(Ordering::Relaxed) } else { ON_KEYS.load(Ordering::Relaxed) };
    if h != 0 && wanted {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_DESK_HIT, WPARAM(0), LPARAM(0));
        }
    }
}

/// Shows, updates or removes the desktop fox to match the settings.
pub fn sync(s: &Settings) {
    ON_KEYS.store(s.desk_keys, Ordering::Relaxed);
    ON_CLICKS.store(s.desk_clicks, Ordering::Relaxed);
    LOCKED.store(s.desk_lock, Ordering::Relaxed);
    GHOST_MS.store(s.ghost_ms(), Ordering::Relaxed);
    let exists = DESK.with(|d| d.borrow().is_some());
    if !s.desk_fox {
        if exists {
            close();
        }
        return;
    }
    if !exists {
        if let Err(e) = open(s) {
            eprintln!("desktop fox failed: {e}");
        }
        return;
    }
    let rebuild = with_desk(|d| {
        let size = BASE_SIZE * s.desk_size();
        if (scale_for(d.hwnd, s) - d.scale).abs() > 0.001 || (size - d.size).abs() > 0.01 || s.desk_on_taskbar != d.on_taskbar {
            return true;
        }
        let looks = (d.skin, d.acc, d.glow_on, d.acc_on) != (s.skin, s.acc, s.desk_glow, s.desk_acc);
        d.skin = s.skin;
        d.acc = s.acc;
        d.glow_on = s.desk_glow;
        d.acc_on = s.desk_acc;
        d.blink_on = s.desk_blink;
        d.nap_on = s.desk_nap;
        let anim = d.anim_on != s.desk_anim;
        d.anim_on = s.desk_anim;
        d.update_anim();
        if anim {
            d.draw();
        }
        if !d.nap_on && d.sleeping {
            d.sleeping = false;
            d.draw();
        } else if looks {
            d.draw();
        }
        d.schedule_blink();
        false
    });
    if rebuild == Some(true) {
        close();
        if let Err(e) = open(s) {
            eprintln!("desktop fox failed: {e}");
        }
    }
}

/// Napping and hiding follow the notch (no input for a while; a full-screen app in front).
pub fn set_state(sleeping: bool, hidden: bool) {
    with_desk(|d| {
        let sleeping = sleeping && d.nap_on;
        if d.hidden != hidden {
            d.hidden = hidden;
            unsafe {
                let _ = ShowWindow(d.hwnd, if hidden { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
            if !hidden {
                d.draw();
            }
            d.update_anim();
        }
        if d.sleeping != sleeping {
            d.sleeping = sleeping;
            d.eyes_shut = false;
            d.blink_left = 0;
            d.schedule_blink();
            d.update_anim();
            d.draw();
        }
    });
}

pub fn close() {
    HWND_RAW.store(0, Ordering::Relaxed);
    let desk = DESK.with(|d| d.borrow_mut().take());
    if let Some(d) = desk {
        let hwnd = d.hwnd;
        drop(d);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn scale_for(hwnd: HWND, _s: &Settings) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    dpi as f32 / 96.0
}

/// The whole screen and its work area (without the taskbar) of the monitor
/// nearest to `p`.
fn monitor_at(p: POINT) -> (RECT, RECT) {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let m = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(m, &mut mi);
    }
    (mi.rcMonitor, mi.rcWork)
}

/// The primary monitor's work area.
fn work_area() -> RECT {
    monitor_at(POINT::default()).1
}

/// Window top for sitting on the taskbar of `work`: pressed paws land on its top edge.
fn taskbar_y_in(work: &RECT, scale: f32, size: f32) -> i32 {
    work.bottom - ((FOX_Y_K + PAW_LINE_K) * size * scale).round() as i32
}

fn taskbar_y(scale: f32, size: f32) -> i32 {
    taskbar_y_in(&work_area(), scale, size)
}

/// Keeps a window of `w` × `h` at `p` inside the monitor nearest to `probe`: on
/// the taskbar it sits on that monitor's taskbar, otherwise anywhere on its screen.
fn clamp_to_screen(p: POINT, probe: POINT, w: i32, h: i32, on_taskbar: bool, scale: f32, size: f32) -> POINT {
    let (mon, work) = monitor_at(probe);
    if on_taskbar {
        POINT { x: p.x.clamp(work.left, (work.right - w).max(work.left)), y: taskbar_y_in(&work, scale, size) }
    } else {
        POINT { x: p.x.clamp(mon.left, (mon.right - w).max(mon.left)), y: p.y.clamp(mon.top, (mon.bottom - h).max(mon.top)) }
    }
}

fn open(s: &Settings) -> Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let class = w!("FuzzieDesk");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            hCursor: LoadCursorW(None, IDC_HAND)?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        // Made where it will sit, so it takes that monitor's scale.
        let (x0, y0) = if s.desk_x == -1 { (0, 0) } else { (s.desk_x, if s.desk_y == -1 { 0 } else { s.desk_y }) };
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class,
            w!("Fuzzie"),
            WS_POPUP,
            x0,
            y0,
            1,
            1,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;
        let scale = scale_for(hwnd, s);
        let size = BASE_SIZE * s.desk_size();
        let (width, height) = ((W_K * size * scale).round() as i32, (H_K * size * scale).round() as i32);

        // Drawn with the notch's render target, bound to this window's bitmap per frame.
        let (factory, rt) = crate::render::shared_target()?;
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        ReleaseDC(None, screen);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, bitmap.into());

        let work = work_area();
        let x = if s.desk_x == -1 && s.desk_y == -1 { work.right - width - 24 } else { s.desk_x };
        let y = if s.desk_y == -1 { taskbar_y(scale, size) } else { s.desk_y };
        // Back on a screen that is still there (monitors come and go).
        let POINT { x, y } = if s.desk_x == -4000 {
            POINT { x, y } // --snapshot: kept off screen
        } else {
            clamp_to_screen(POINT { x, y }, POINT { x: x + width / 2, y: y + height / 2 }, width, height, s.desk_on_taskbar, scale, size)
        };
        let mut d = Desk {
            hwnd,
            factory,
            rt,
            dc,
            bitmap,
            bits: bits as *mut u8,
            old,
            width,
            height,
            scale,
            size,
            down: [false; 2],
            next: 0,
            eyes_shut: false,
            blink_left: 0,
            blinks: 0,
            skin: s.skin,
            acc: s.acc,
            blink_on: s.desk_blink,
            nap_on: s.desk_nap,
            anim_on: s.desk_anim,
            ghost: false,
            glow_on: s.desk_glow,
            acc_on: s.desk_acc,
            anim_ms: 0,
            started: Instant::now(),
            on_taskbar: s.desk_on_taskbar,
            sleeping: false,
            hidden: false,
            pos: POINT { x, y },
            drag: None,
            raised: Instant::now(),
            forced: None,
        };
        d.draw();
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // Always above the taskbar, like a sticker on it.
        if FG_HOOK.load(Ordering::Relaxed) == 0 {
            use windows::Win32::UI::Accessibility::SetWinEventHook;
            use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS};
            let hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
            FG_HOOK.store(hook.0 as isize, Ordering::Relaxed);
        }
        SetTimer(Some(hwnd), TIMER_KEEP_UP, KEEP_UP_MS, None);
        d.schedule_blink();
        d.update_anim();
        DESK.with(|x| *x.borrow_mut() = Some(d));
        HWND_RAW.store(hwnd.0 as isize, Ordering::Relaxed);
    }
    Ok(())
}

impl Desk {
    /// The next blink in 2.5..6.5 s (none while asleep, hidden or switched off).
    fn schedule_blink(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_BLINK);
            if self.blink_on && !self.sleeping && !self.hidden {
                let r = (Instant::now().duration_since(self.raised).as_nanos() as u64).wrapping_mul(6364136223846793005) >> 40;
                let ms = 2500 + (r % 4000) as u32;
                SetTimer(Some(self.hwnd), TIMER_BLINK, ms, None);
            }
        }
    }

    fn on_blink_timer(&mut self) {
        if self.blink_on && !self.sleeping && !self.hidden {
            self.blinks += 1;
            // Shut now; every fourth blink is a double one (shut, open, shut, open).
            self.eyes_shut = true;
            self.blink_left = if self.blinks % 4 == 0 { 3 } else { 1 };
            unsafe { SetTimer(Some(self.hwnd), TIMER_EYES, BLINK_MS, None) };
            self.draw();
        }
        self.schedule_blink();
    }

    fn on_eyes_timer(&mut self) {
        self.eyes_shut = !self.eyes_shut;
        self.blink_left = self.blink_left.saturating_sub(1);
        if self.blink_left == 0 {
            self.eyes_shut = false;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_EYES);
            }
        }
        self.draw();
    }

    fn on_paw_timer(&mut self, side: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_PAW + side);
        }
        self.down[side] = false;
        self.draw();
    }

    fn on_hit(&mut self) {
        if self.hidden {
            return;
        }
        let side = self.next;
        self.next ^= 1;
        let now = Instant::now();
        self.down[side] = true;
        unsafe { SetTimer(Some(self.hwnd), TIMER_PAW + side, PAW_MS, None) };
        if self.sleeping {
            self.sleeping = false; // the notch confirms on its next poll
            self.schedule_blink();
        }
        // A click on the taskbar brings it to the front once the click is done:
        // step back above it just after.
        self.raised = now;
        unsafe { SetTimer(Some(self.hwnd), TIMER_RAISE, 150, None) };
        self.draw();
    }

    /// Runs frames while animating (or asleep, for the "zZZ"); none otherwise.
    fn update_anim(&mut self) {
        let ms = if self.hidden || self.forced.is_some() {
            0
        } else if self.anim_on {
            ANIM_MS
        } else if self.sleeping {
            ZZZ_MS
        } else {
            0
        };
        if ms != self.anim_ms {
            self.anim_ms = ms;
            unsafe {
                if ms == 0 {
                    let _ = KillTimer(Some(self.hwnd), TIMER_ANIM);
                } else {
                    SetTimer(Some(self.hwnd), TIMER_ANIM, ms, None);
                }
            }
        }
    }

    fn draw(&mut self) {
        if self.hidden {
            return;
        }
        let paws = self.forced.unwrap_or(self.down.map(|d| if d { 1.0 } else { 0.0 }));
        let blink = if self.eyes_shut { 1.0 } else { 0.0 };
        let (cx, cy) = (W_K * self.size / 2.0, FOX_Y_K * self.size);
        unsafe {
            crate::render::lend_target();
            self.rt.SetDpi(96.0 * self.scale, 96.0 * self.scale);
            if self.rt.BindDC(self.dc, &RECT { left: 0, top: 0, right: self.width, bottom: self.height }).is_err() {
                return;
            }
            self.rt.BeginDraw();
            self.rt.SetTransform(&Matrix3x2::identity());
            self.rt.Clear(Some(&D2D1_COLOR_F::default()));
            let acc = if self.acc_on { self.acc } else { 0 };
            let _ = mascot::draw_bongo(&self.rt, &self.factory, self.skin, acc, cx, cy, self.size, self.sleeping, paws, blink, self.started.elapsed().as_secs_f32(), self.anim_on, self.glow_on);
            if self.rt.EndDraw(None, None).is_err() {
                return;
            }
            let alpha = if self.ghost { window::GHOST_ALPHA } else { 255 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
            let size = SIZE { cx: self.width, cy: self.height };
            let zero = POINT::default();
            let _ = UpdateLayeredWindow(self.hwnd, None, Some(&self.pos), Some(&size), Some(self.dc), Some(&zero), COLORREF(0), Some(&blend), ULW_ALPHA);
        }
    }

    /// Puts the fox back inside the screen it is on (after a display change).
    fn keep_on_screen(&mut self) {
        let probe = POINT { x: self.pos.x + self.width / 2, y: self.pos.y + self.height / 2 };
        let p = clamp_to_screen(self.pos, probe, self.width, self.height, self.on_taskbar, self.scale, self.size);
        if p.x != self.pos.x || p.y != self.pos.y {
            self.pos = p;
            self.move_to();
        }
    }

    /// Moves the window without redrawing it.
    fn move_to(&self) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, self.pos.x, self.pos.y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER);
        }
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        unsafe {
            for t in [TIMER_BLINK, TIMER_PAW, TIMER_PAW + 1, TIMER_EYES, TIMER_ANIM, TIMER_GHOST, TIMER_RAISE, TIMER_KEEP_UP] {
                let _ = KillTimer(Some(self.hwnd), t);
            }
            let hook = FG_HOOK.swap(0, Ordering::Relaxed);
            if hook != 0 {
                let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(windows::Win32::UI::Accessibility::HWINEVENTHOOK(hook as *mut core::ffi::c_void));
            }
        }
        // (The render target is shared with the notch, so its brushes stay.)
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// `--snapshot DIR`: the desktop fox resting, tapping left, tapping right, blinking, asleep and in the helmet.
pub fn snapshot(dir: &std::path::Path, s: &Settings) {
    let mut s = s.clone();
    s.desk_fox = true;
    s.desk_on_taskbar = false;
    s.desk_x = -4000;
    s.desk_y = -4000;
    close();
    sync(&s);
    with_desk(|d| {
        let cases: [(&str, [f32; 2], usize, bool, bool); 6] = [
            ("desk_rest.png", [0.0, 0.0], s.acc, false, false),
            ("desk_left.png", [1.0, 0.0], s.acc, false, false),
            ("desk_right.png", [0.3, 1.0], s.acc, false, false),
            ("desk_blink.png", [0.0, 0.0], s.acc, false, true),
            ("desk_sleep.png", [0.0, 0.0], s.acc, true, false),
            ("desk_helmet.png", [0.0, 1.0], 26, false, false),
        ];
        for (name, paws, acc, sleeping, blink) in cases {
            d.forced = Some(paws);
            d.acc = acc;
            d.sleeping = sleeping;
            d.eyes_shut = blink;
            d.draw();
            let px = unsafe { std::slice::from_raw_parts(d.bits, (d.width * d.height * 4) as usize) };
            if let Some(png) = crate::image::encode_png(d.width as u32, d.height as u32, px) {
                let _ = std::fs::write(dir.join(name), png);
            }
        }
        // The shinobi pack: every skin bare, then every accessory on one of them.
        let pack = &crate::mascot::PACKS[1];
        let first = pack.skins.start;
        let skins = pack.skins.len();
        let (acc0, accs) = (pack.accs.start, pack.accs.len());
        d.forced = Some([0.0, 0.0]);
        d.sleeping = false;
        d.eyes_shut = false;
        for (k, (skin, acc)) in (0..skins).map(|i| (first + i, 0)).chain((0..accs).map(|i| (first + i % skins, acc0 + i))).enumerate() {
            d.skin = skin;
            d.acc = acc;
            d.draw();
            let px = unsafe { std::slice::from_raw_parts(d.bits, (d.width * d.height * 4) as usize) };
            if let Some(png) = crate::image::encode_png(d.width as u32, d.height as u32, px) {
                let _ = std::fs::write(dir.join(format!("shinobi_{k:02}.png")), png);
            }
        }
    });
    close();
    // The app icon: the default fox's head at each icon size.
    for px in [16, 20, 24, 32, 40, 48, 64, 96, 128, 256] {
        let k = px as f32;
        if let Some(png) = fox_png(px, |rt, f| mascot::draw_head(rt, f, 0, k * ICON_X, k * ICON_Y, k * ICON_SIZE)) {
            let _ = std::fs::write(dir.join(format!("icon_{px}.png")), png);
        }
    }
    if std::env::var_os("FUZZIE_PACK").is_some() {
        pack_frames(&dir.join("pack"));
    }
    if std::env::var_os("FUZZIE_WEB_FRAMES").is_some() {
        web_frames(&dir.join("web"));
    }
    // A gallery of looks for the web page: whole foxes, skin + accessory.
    let looks = [(0, 0), (1, 22), (3, 9), (4, 4), (5, 24), (6, 7), (8, 13), (9, 17), (10, 25), (11, 5), (14, 8), (16, 16), (19, 6), (21, 19), (25, 15), (27, 18), (29, 21), (31, 20), (36, 11), (38, 3), (45, 14), (23, 26)];
    for (skin, acc) in looks {
        let px = 220;
        let k = px as f32;
        if let Some(png) = fox_png(px, |rt, f| mascot::draw_still(rt, f, skin, acc, k * 0.5, k * 0.56, k * 0.42, 1.0, false, 0.0)) {
            let _ = std::fs::write(dir.join(format!("fox_{skin}_{acc}.png")), png);
        }
    }
}

/// Animation frames for the web page: every skin (without the shinobi pack)
/// on its own, and every accessory's back and front parts, on one loop of
/// [`WEB_LOOP`] seconds, so the page can stack any skin with any accessory.
fn web_frames(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let px = 200;
    let k = px as f32;
    // Only the classic pack is shown on the page.
    let classic = &mascot::PACKS[0];
    let skins = classic.skins.end;
    let save = |name: String, skin: usize, acc: usize, layers: u8, t: f32| {
        if let Some(png) = fox_png(px, |rt, f| mascot::draw_layers(rt, f, skin, acc, k * 0.5, k * 0.56, k * 0.42, t, layers)) {
            let _ = std::fs::write(dir.join(name), png);
        }
    };
    // The desktop fox's poses, large.
    let big = 240;
    let (size, cx) = (big as f32 / W_K, big as f32 / 2.0);
    let cy = FOX_Y_K * size + (big as f32 - H_K * size) / 2.0;
    for (name, paws, sleeping, blink) in [("rest", [0.0, 0.0], false, 0.0), ("left", [1.0, 0.0], false, 0.0), ("right", [0.0, 1.0], false, 0.0), ("blink", [0.0, 0.0], false, 1.0), ("sleep", [0.0, 0.0], true, 0.0)] {
        if let Some(png) = fox_png(big, |rt, f| mascot::draw_bongo(rt, f, 0, 0, cx, cy, size, sleeping, paws, blink, 1.2, false, true)) {
            let _ = std::fs::write(dir.with_file_name(format!("bongo_{name}.png")), png);
        }
    }
    // A few frames past the loop, for the page builder to blend into its start.
    for i in 0..WEB_FRAMES + 8 {
        // Starting one blink period in puts the seam on a blink and the ear twitch mid-loop.
        let t = WEB_LOOP + i as f32 * WEB_LOOP / WEB_FRAMES as f32;
        for skin in 0..skins {
            save(format!("skin_{skin}_{i:02}.png"), skin, 0, 1, t);
        }
        for acc in classic.accs.clone() {
            save(format!("accb_{acc}_{i:02}.png"), 0, acc, 2, t);
            save(format!("accf_{acc}_{i:02}.png"), 0, acc, 4, t);
        }
    }
}

/// Large stills of the shinobi pack for checking the drawings by eye:
/// every skin bare, and every pack accessory on a plain fox and on a pack skin.
fn pack_frames(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let px = 360;
    let k = px as f32;
    let pack = &mascot::PACKS[1];
    let (first, acc0) = (pack.skins.start, pack.accs.start);
    let save = |name: String, skin: usize, acc: usize, t: f32, zoom: bool| {
        let (cy, size) = if zoom { (k * 0.42, k * 0.92) } else { (k * 0.56, k * 0.4) };
        if let Some(png) = fox_png(px, |rt, f| if t < 2.0 { mascot::draw_still(rt, f, skin, acc, k * 0.5, cy, size, 1.0, false, 0.0) } else { mascot::draw(rt, f, skin, acc, k * 0.5, cy, size, 1.0, false, t) }) {
            let _ = std::fs::write(dir.join(name), png);
        }
    };
    for s in pack.skins.clone() {
        save(format!("skin_{:02}.png", s - first), s, 0, 1.3, false);
        save(format!("face_{:02}.png", s - first), s, 0, 1.3, true);
    }
    for a in pack.accs.clone() {
        save(format!("acc_{:02}.png", a - acc0), 0, a, 1.3, false);
        save(format!("accs_{:02}.png", a - acc0), first + (a - acc0) % pack.skins.len(), a, 2.1, false);
    }
}

/// One blink period, so the loop closes on the same blink.
const WEB_LOOP: f32 = 4.3;
const WEB_FRAMES: usize = 52;

/// Draws with `paint` into a `px` square on a clear background, as PNG.
fn fox_png(px: i32, paint: impl FnOnce(&windows::Win32::Graphics::Direct2D::ID2D1RenderTarget, &ID2D1Factory) -> windows::core::Result<()>) -> Option<Vec<u8>> {
    unsafe {
        let (factory, rt) = crate::render::shared_target().ok()?;
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        ReleaseDC(None, screen);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: px,
                biHeight: -px,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(dc, bitmap.into());
        crate::render::lend_target();
        rt.SetDpi(96.0, 96.0);
        let mut out = None;
        if rt.BindDC(dc, &RECT { left: 0, top: 0, right: px, bottom: px }).is_ok() {
            rt.BeginDraw();
            rt.SetTransform(&Matrix3x2::identity());
            rt.Clear(Some(&D2D1_COLOR_F::default()));
            let _ = paint(&rt, &factory);
            if rt.EndDraw(None, None).is_ok() {
                let data = std::slice::from_raw_parts(bits as *const u8, (px * px * 4) as usize);
                out = crate::image::encode_png(px as u32, px as u32, data);
            }
        }
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        out
    }
}

/// Where the head sits in the icon square (fractions of its side).
const ICON_X: f32 = 0.5;
const ICON_Y: f32 = 0.55;
const ICON_SIZE: f32 = 0.7;

/// Monitors or the taskbar changed: keep the fox on a screen.
pub fn keep_on_screen() {
    with_desk(|d| d.keep_on_screen());
}

/// Puts the fox back on the taskbar at the bottom right (Settings button).
pub fn reset_position() {
    let pos = with_desk(|d| {
        let work = work_area();
        d.pos = POINT { x: work.right - d.width - 24, y: taskbar_y(d.scale, d.size) };
        d.move_to();
        d.pos
    });
    if let Some(p) = pos {
        window::save_desk_pos(p.x, p.y);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_DESK_HIT => {
            with_desk(|d| d.on_hit());
            LRESULT(0)
        }
        WM_TIMER => {
            match wp.0 {
                TIMER_BLINK => with_desk(|d| d.on_blink_timer()),
                TIMER_EYES => with_desk(|d| d.on_eyes_timer()),
                TIMER_ANIM => with_desk(|d| d.draw()),
                TIMER_RAISE => with_desk(|d| {
                    let _ = KillTimer(Some(d.hwnd), TIMER_RAISE);
                    if !d.hidden { raise(d.hwnd) }
                }),
                TIMER_KEEP_UP => with_desk(|d| if !d.hidden { raise(d.hwnd) }),
                TIMER_GHOST => with_desk(|d| {
                    let _ = KillTimer(Some(d.hwnd), TIMER_GHOST);
                    d.ghost = false;
                    window::set_click_through(d.hwnd, false);
                    d.draw();
                }),
                t if t == TIMER_PAW || t == TIMER_PAW + 1 => with_desk(|d| d.on_paw_timer(t - TIMER_PAW)),
                _ => None,
            };
            LRESULT(0)
        }
        WM_LBUTTONDOWN if LOCKED.load(Ordering::Relaxed) => LRESULT(0),
        WM_LBUTTONDOWN => {
            let mut cur = POINT::default();
            let _ = GetCursorPos(&mut cur);
            with_desk(|d| d.drag = Some((cur, d.pos)));
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            with_desk(|d| {
                if let Some((from, start)) = d.drag {
                    let mut cur = POINT::default();
                    let _ = GetCursorPos(&mut cur);
                    // Follows the pointer, kept inside the screen under it.
                    let want = POINT { x: start.x + cur.x - from.x, y: start.y + cur.y - from.y };
                    let p = clamp_to_screen(want, cur, d.width, d.height, d.on_taskbar, d.scale, d.size);
                    if p.x != d.pos.x || p.y != d.pos.y {
                        d.pos = p;
                        d.move_to();
                    }
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let moved = with_desk(|d| d.drag.take().map(|(_, start)| (start, d.pos))).flatten();
            if let Some((start, pos)) = moved {
                if start.x != pos.x || start.y != pos.y {
                    window::save_desk_pos(pos.x, pos.y);
                    // Another monitor may have another scale: rebuild at its size.
                    let _ = PostMessageW(Some(hwnd), WM_DESK_RESYNC, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        // Right-click: see-through for a moment, so what is behind can be clicked.
        windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP => {
            with_desk(|d| {
                d.ghost = true;
                window::set_click_through(d.hwnd, true);
                d.draw();
                SetTimer(Some(d.hwnd), TIMER_GHOST, GHOST_MS.load(Ordering::Relaxed), None);
            });
            LRESULT(0)
        }
        WM_DESK_RESYNC => {
            if let Some(s) = window::settings_snapshot() {
                sync(&s);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

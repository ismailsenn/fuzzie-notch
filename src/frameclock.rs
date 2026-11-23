//! A frame clock that ticks on the display's vertical blank (`DwmFlush`), so
//! animations step at an even pace. `SetTimer` only fires on the 15.6 ms system
//! tick and drifts against the display, which shows as stutter.
//!
//! A helper thread posts `msg` to the window every n-th vblank, n chosen so the
//! frame time is close to the one asked for (and never shorter). It sleeps on a
//! condition variable while no frames are wanted, so an idle notch costs nothing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmFlush, DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Wanted frame time in ms: 0 = stopped, `QUIT` = thread exits.
const QUIT: u32 = u32::MAX;

struct Shared {
    ms: Mutex<u32>,
    wake: Condvar,
    /// A frame message is posted and not handled yet (never queue more than one).
    pending: AtomicBool,
}

pub struct FrameClock {
    shared: Arc<Shared>,
    ms: u32,
}

impl FrameClock {
    pub fn new(hwnd: HWND, msg: u32) -> Self {
        let shared = Arc::new(Shared { ms: Mutex::new(0), wake: Condvar::new(), pending: AtomicBool::new(false) });
        let s = shared.clone();
        let hwnd_raw = hwnd.0 as isize; // HWND is not Send; pass the raw value.
        std::thread::spawn(move || run(&s, hwnd_raw, msg));
        Self { shared, ms: 0 }
    }

    /// Frames about every `ms` milliseconds; 0 stops them.
    pub fn set(&mut self, ms: u32) {
        if ms != self.ms {
            self.ms = ms;
            *self.shared.ms.lock().unwrap() = ms;
            self.shared.wake.notify_one();
        }
    }

    /// Call when the frame message arrives, so the next one can be posted.
    pub fn consumed(&self) {
        self.shared.pending.store(false, Ordering::Release);
    }
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        *self.shared.ms.lock().unwrap() = QUIT;
        self.shared.wake.notify_one();
    }
}

/// Milliseconds between vblanks of the main display (60 Hz if unknown).
fn refresh_period() -> f32 {
    let mut info = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    if unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.is_ok() {
        let (num, den) = (info.rateRefresh.uiNumerator, info.rateRefresh.uiDenominator);
        if num > 0 && den > 0 {
            let hz = num as f32 / den as f32;
            if (20.0..=500.0).contains(&hz) {
                return 1000.0 / hz;
            }
        }
    }
    1000.0 / 60.0
}

fn run(s: &Shared, hwnd_raw: isize, msg: u32) {
    let mut period = refresh_period();
    // When the next frame is due; None right after waking up.
    let mut due: Option<Instant> = None;
    let mut posted = Instant::now();
    let mut quick = 0u32;
    loop {
        let ms = {
            let mut g = s.ms.lock().unwrap();
            if *g == 0 {
                while *g == 0 {
                    g = s.wake.wait(g).unwrap();
                }
                // The display may have changed while idle.
                due = None;
                period = refresh_period();
            }
            *g
        };
        if ms == QUIT {
            return;
        }
        // Every n-th vblank: 60 Hz -> 1 (fast) or 2 (33 ms); 144 Hz -> 2 or 5.
        let n = (ms as f32 / period).round().max(1.0);
        let frame = Duration::from_secs_f32(n * period / 1000.0);
        // Sleep most of the way, then wait for the vblank itself: the thread
        // wakes about once per frame, not on every vblank. Sleep can overrun by a
        // 15.6 ms system tick, hence the margin.
        if let Some(d) = due {
            let margin = Duration::from_secs_f32((15.6 + period * 0.5) / 1000.0);
            if let Some(wait) = d.checked_duration_since(Instant::now()).and_then(|w| w.checked_sub(margin)) {
                std::thread::sleep(wait);
            }
        }
        let before = Instant::now();
        let synced = unsafe { DwmFlush() }.is_ok();
        let now = Instant::now();
        // DwmFlush returns at once when nothing is composed (display off, no
        // DWM): after a few of those in a row, pace by sleeping instead.
        quick = if now.duration_since(before) < Duration::from_micros(300) { quick + 1 } else { 0 };
        if !synced || quick >= 3 {
            std::thread::sleep(Duration::from_millis(ms as u64));
        }
        let now = Instant::now();
        // Post on the vblank closest to when the frame is due.
        if due.is_none_or(|d| now + Duration::from_secs_f32(period * 0.5 / 1000.0) >= d) {
            due = Some(now + frame);
            // A message the window never handled (it was busy, or re-entered)
            // must not stop the clock for good: send another after a while.
            if s.pending.load(Ordering::Acquire) && posted.elapsed() > Duration::from_millis(200) {
                s.pending.store(false, Ordering::Release);
            }
            if !s.pending.swap(true, Ordering::AcqRel) {
                posted = now;
                let ok = unsafe { PostMessageW(Some(HWND(hwnd_raw as *mut _)), msg, WPARAM(0), LPARAM(0)) };
                if ok.is_err() {
                    s.pending.store(false, Ordering::Release);
                }
            }
        }
    }
}

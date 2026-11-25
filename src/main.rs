// Release builds run without a console window; debug builds keep it for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod autostart;
mod clipboard;
mod dragdrop;
mod deskfox;
mod frameclock;
mod image;
mod lyrics;
mod mascot;
mod notify;
mod media;
mod render;
mod settings;
mod settings_win;
mod shelf;
mod social;
mod stats;
mod tray;
mod updater;
mod window;
mod wipe;

use std::sync::atomic::{AtomicIsize, Ordering};
use windows::core::{w, Result};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

static INSTANCE_MUTEX: AtomicIsize = AtomicIsize::new(0);

/// Releases the single-instance mutex right before relaunching into an updated exe.
pub fn release_instance_mutex() {
    let raw = INSTANCE_MUTEX.swap(0, Ordering::AcqRel);
    if raw != 0 {
        unsafe {
            let _ = CloseHandle(HANDLE(raw as *mut core::ffi::c_void));
        }
    }
}

fn main() -> Result<()> {
    let after_update = std::env::args().any(|a| a == "--after-update");
    let snapshot = std::env::args().any(|a| a == "--snapshot");
    unsafe {
        // One instance per user session; save GetLastError() immediately after CreateMutexW.
        let mut mutex = CreateMutexW(None, true, w!("Fuzzie_SingleInstance_Mutex"))?;
        let mut already = GetLastError() == ERROR_ALREADY_EXISTS;
        // Started by an update: if the old Fuzzie hasn't released the mutex yet, wait for it.
        if after_update && already {
            for _ in 0..100 {
                let _ = CloseHandle(mutex);
                std::thread::sleep(std::time::Duration::from_millis(100));
                mutex = CreateMutexW(None, true, w!("Fuzzie_SingleInstance_Mutex"))?;
                already = GetLastError() == ERROR_ALREADY_EXISTS;
                if !already {
                    break;
                }
            }
        }
        // `--snapshot` renders to files and quits, so it may run beside the real one.
        if already && !snapshot {
            let _ = CloseHandle(mutex);
            return Ok(());
        }
        INSTANCE_MUTEX.store(mutex.0 as isize, Ordering::Release);
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // OLE (not just COM) so the notch can accept dropped files.
        OleInitialize(None)?;
    }

    let settings = settings::Settings::load();
    if !snapshot && settings.start_with_windows {
        // Refresh autostart in the background so UI startup never waits on schtasks.
        let _ = std::thread::Builder::new().name("autostart".into()).spawn(|| {
            autostart::set(true);
        });
    }
    let r = window::run(settings);
    wipe::finish();
    r
}

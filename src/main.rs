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

use windows::core::{w, Result};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

fn main() -> Result<()> {
    unsafe {
        // One instance per user session; the handle lives until the process exits.
        let mut _mutex = CreateMutexW(None, true, w!("Fuzzie_SingleInstance_Mutex"))?;
        // Started by an update: the old Fuzzie is still closing, wait for it.
        if std::env::args().any(|a| a == "--after-update") {
            for _ in 0..100 {
                if GetLastError() != ERROR_ALREADY_EXISTS {
                    break;
                }
                let _ = windows::Win32::Foundation::CloseHandle(_mutex);
                std::thread::sleep(std::time::Duration::from_millis(100));
                _mutex = CreateMutexW(None, true, w!("Fuzzie_SingleInstance_Mutex"))?;
            }
        }
        // `--snapshot` renders to files and quits, so it may run beside the real one.
        if GetLastError() == ERROR_ALREADY_EXISTS && !std::env::args().any(|a| a == "--snapshot") {
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // OLE (not just COM) so the notch can accept dropped files.
        OleInitialize(None)?;
    }

    let settings = settings::Settings::load();
    if settings.start_with_windows {
        // Refresh the registry entry in case the exe moved.
        autostart::set(true);
    }
    let r = window::run(settings);
    wipe::finish();
    r
}

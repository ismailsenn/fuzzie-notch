//! "Delete all my data" and "Uninstall Fuzzie". Both close the app first and
//! do the work after the message loop ends, so nothing is saved back.

use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicU8, Ordering};

static MODE: AtomicU8 = AtomicU8::new(0);
const WIPE: u8 = 1;
const UNINSTALL: u8 = 2;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Erases everything and starts again fresh.
pub fn delete_data() {
    MODE.store(WIPE, Ordering::Release);
    crate::window::quit();
}

/// Erases everything and removes the exe.
pub fn uninstall() {
    MODE.store(UNINSTALL, Ordering::Release);
    crate::window::quit();
}

/// True while closing to wipe: nothing should be written any more.
pub fn pending() -> bool {
    MODE.load(Ordering::Acquire) != 0
}

/// Runs after the app window is gone.
pub fn finish() {
    let mode = MODE.load(Ordering::Acquire);
    if mode == 0 {
        return;
    }
    // The friends account on the relay (its friends see it disappear).
    crate::social::delete_account();
    crate::autostart::set(false);
    let _ = std::fs::remove_dir_all(crate::settings::data_dir());
    match mode {
        WIPE => {
            crate::release_instance_mutex();
            let _ = crate::updater::relaunch();
        }
        UNINSTALL => remove_exe(),
        _ => {}
    }
}

/// A running exe cannot delete itself: a hidden cmd waits for this process to
/// end, then deletes the exe and anything an update left beside it.
fn remove_exe() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let q = |p: std::path::PathBuf| format!("\"{}\"", p.display());
    // The exe stays locked a moment after the process ends: retry for up to ~30 s.
    let me = q(exe.clone());
    let cmd = format!(
        "for /l %i in (1,1,30) do @(if exist {me} (ping 127.0.0.1 -n 2 > nul & del /f /q {me} 2> nul)) & del /f /q {} {} 2> nul",
        q(dir.join("Fuzzie.old.exe")),
        q(dir.join("Fuzzie.update.tmp"))
    );
    let _ = std::process::Command::new("cmd.exe").arg("/C").raw_arg(&cmd).creation_flags(CREATE_NO_WINDOW).spawn();
}

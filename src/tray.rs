use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, POINT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, LoadImageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON,
    SM_CYICON, SM_CYSMICON,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, SetForegroundWindow,
    TrackPopupMenu, IDI_APPLICATION, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
};

/// Callback message for tray icon mouse events.
pub const WM_TRAY: u32 = WM_APP + 2;

pub enum MenuCmd {
    MinimalNotch,
    Lyrics,
    DeskFox,
    DeskLock,
    Autostart,
    Settings,
    Quit,
}

/// Which switches in the menu are ticked.
pub struct MenuState {
    pub minimal: bool,
    pub lyrics: bool,
    pub desk_fox: bool,
    pub desk_lock: bool,
    pub autostart: bool,
}

/// Fuzzie's own icon (resource 1, embedded by build.rs); the Windows default if missing.
pub fn app_icon(small: bool) -> HICON {
    unsafe {
        let (w, h) = if small {
            (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
        } else {
            (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON))
        };
        let module = GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
        LoadImageW(module, PCWSTR(1 as *const u16), IMAGE_ICON, w, h, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default()
    }
}

fn base(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

pub fn add(hwnd: HWND) -> Result<()> {
    unsafe {
        let mut nid = base(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = app_icon(true);
        for (dst, src) in nid.szTip.iter_mut().zip("Fuzzie".encode_utf16()) {
            *dst = src;
        }
        Shell_NotifyIconW(NIM_ADD, &nid).ok()
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &base(hwnd));
    }
}

/// Shows the tray menu at the cursor and returns the chosen item.
pub fn show_menu(hwnd: HWND, st: &MenuState) -> Option<MenuCmd> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | check(st.minimal), 1, w!("Minimal notch"));
        let _ = AppendMenuW(menu, MF_STRING | check(st.lyrics), 2, w!("Lyrics"));
        let _ = AppendMenuW(menu, MF_STRING | check(st.desk_fox), 5, w!("Desktop Fuzzie"));
        if st.desk_fox {
            let _ = AppendMenuW(menu, MF_STRING | check(st.desk_lock), 7, w!("Lock Desktop Fuzzie"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING | check(st.autostart), 6, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_STRING, 4, w!("Settings"));
        let _ = AppendMenuW(menu, MF_STRING, 3, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu closes when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None).0;
        let _ = DestroyMenu(menu);
        match id {
            1 => Some(MenuCmd::MinimalNotch),
            2 => Some(MenuCmd::Lyrics),
            5 => Some(MenuCmd::DeskFox),
            7 => Some(MenuCmd::DeskLock),
            6 => Some(MenuCmd::Autostart),
            3 => Some(MenuCmd::Quit),
            4 => Some(MenuCmd::Settings),
            _ => None,
        }
    }
}

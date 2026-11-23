//! Watches the default speaker: volume / mute changes and output device switches.
//! Also sets the volume of the app that plays the media (its audio session).
//! Polled from the UI thread (cheap COM calls), so no callbacks are needed.

use std::time::{Duration, Instant};

use windows::core::{Interface, Result, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eConsole, eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub enum Event {
    Volume { level: f32, muted: bool },
    Device(String),
}

pub struct Watcher {
    devices: IMMDeviceEnumerator,
    current: Option<(String, IAudioEndpointVolume)>,
    last: Option<(f32, bool)>,
    ticks: u32,
    app: Option<AppVolume>,
    app_last: Option<(f32, bool)>,
}

/// The audio sessions of the app that plays the media.
struct AppVolume {
    id: String,
    sessions: Vec<ISimpleAudioVolume>,
    found: Instant,
}

impl Watcher {
    pub fn new() -> Result<Self> {
        let devices = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        Ok(Self { devices, current: None, last: None, ticks: 0, app: None, app_last: None })
    }

    /// Call about every 100 ms. Returns at most one event.
    pub fn poll(&mut self) -> Option<Event> {
        self.ticks = self.ticks.wrapping_add(1);
        // Checking the default device is slower, so do it once a second.
        if self.current.is_none() || self.ticks % 10 == 0 {
            match self.default_device() {
                Ok((id, name, volume)) => {
                    let switched = self.current.as_ref().is_some_and(|(old, _)| *old != id);
                    if self.current.as_ref().is_none_or(|(old, _)| *old != id) {
                        self.current = Some((id, volume));
                        self.last = None;
                    }
                    if switched {
                        return Some(Event::Device(name));
                    }
                }
                Err(_) => {
                    self.current = None;
                    return None;
                }
            }
        }
        let (_, volume) = self.current.as_ref()?;
        let level = unsafe { volume.GetMasterVolumeLevelScalar() }.ok()?;
        let muted = unsafe { volume.GetMute() }.ok()?.as_bool();
        match self.last.replace((level, muted)) {
            Some((l, m)) if (l - level).abs() > 0.004 || m != muted => Some(Event::Volume { level, muted }),
            _ => None,
        }
    }

    /// Follows the app that plays the media (its GSMTC app id, and its friendly
    /// name as a fallback), so the music page's volume control sets that app
    /// only. Call while the music page can be seen: the session list is
    /// refreshed every 2 s or when the app changes, the level on every call.
    pub fn track_app(&mut self, app_id: &str, friendly: &str) {
        let stale = self.app.as_ref().is_none_or(|a| a.id != app_id || a.found.elapsed() > Duration::from_secs(2));
        if stale {
            let sessions = self.app_sessions(&app_keys(app_id, friendly)).unwrap_or_default();
            self.app = Some(AppVolume { id: app_id.to_string(), sessions, found: Instant::now() });
        }
        self.app_last = self
            .app
            .as_ref()
            .and_then(|a| a.sessions.first())
            .and_then(|v| unsafe { Some((v.GetMasterVolume().ok()?, v.GetMute().ok()?.as_bool())) });
    }

    /// Last known (level 0..1, muted) of the media app, if it has an audio session.
    pub fn app_state(&self) -> Option<(f32, bool)> {
        self.app_last
    }

    /// Mutes or unmutes the media app.
    pub fn toggle_app_mute(&mut self) {
        let Some((level, muted)) = self.app_last else { return };
        for v in self.app.iter().flat_map(|a| &a.sessions) {
            let _ = unsafe { v.SetMute(!muted, std::ptr::null()) };
        }
        self.app_last = Some((level, !muted));
    }

    /// Sets the media app's volume (0..1); unmutes when raised above zero.
    pub fn set_app_level(&mut self, level: f32) {
        let level = level.clamp(0.0, 1.0);
        let Some((_, muted)) = self.app_last else { return };
        let unmute = muted && level > 0.0;
        for v in self.app.iter().flat_map(|a| &a.sessions) {
            unsafe {
                let _ = v.SetMasterVolume(level, std::ptr::null());
                if unmute {
                    let _ = v.SetMute(false, std::ptr::null());
                }
            }
        }
        self.app_last = Some((level, muted && !unmute));
    }

    /// Volume controls of the default speaker's sessions whose process matches
    /// `keys`; the ones playing now come first (their level is the one shown).
    fn app_sessions(&self, keys: &[String]) -> Result<Vec<ISimpleAudioVolume>> {
        let mut out = Vec::new();
        if keys.is_empty() {
            return Ok(out);
        }
        unsafe {
            let dev: IMMDevice = self.devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let manager = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)?;
            let list = manager.GetSessionEnumerator()?;
            for i in 0..list.GetCount()? {
                let Ok(control) = list.GetSession(i) else { continue };
                let Ok(c2) = control.cast::<IAudioSessionControl2>() else { continue };
                let Ok(pid) = c2.GetProcessId() else { continue };
                let Some(exe) = exe_stem(pid) else { continue };
                if !keys.iter().any(|k| *k == exe || (exe.len() >= 4 && k.starts_with(&exe))) {
                    continue;
                }
                if let Ok(v) = control.cast::<ISimpleAudioVolume>() {
                    if control.GetState().is_ok_and(|st| st == AudioSessionStateActive) {
                        out.insert(0, v);
                    } else {
                        out.push(v);
                    }
                }
            }
        }
        Ok(out)
    }

    fn default_device(&self) -> Result<(String, String, IAudioEndpointVolume)> {
        unsafe {
            let dev: IMMDevice = self.devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let raw = dev.GetId()?;
            let id = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0 as _));
            let name = dev
                .OpenPropertyStore(STGM_READ)
                .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
                .map(|v| v.to_string())
                .unwrap_or_else(|_| "Audio device".into());
            let volume = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)?;
            Ok((id, name, volume))
        }
    }
}

/// Lower-case executable name, without ".exe", of a process.
fn exe_stem(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut n = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n);
        let _ = CloseHandle(h);
        r.ok()?;
        let path = String::from_utf16_lossy(&buf[..n as usize]).to_lowercase();
        let file = path.rsplit('\\').next()?;
        Some(file.strip_suffix(".exe").unwrap_or(file).to_string())
    }
}

/// Names a media app's process may have, from its app id ("Spotify.exe",
/// "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "MSEdge", "C:\x\vlc.exe")
/// and its friendly name (Firefox reports a hash as its id).
fn app_keys(app_id: &str, friendly: &str) -> Vec<String> {
    let id = app_id.to_lowercase();
    let tail = id.rsplit(['\\', '/']).next().unwrap_or(&id);
    let mut keys: Vec<String> = Vec::new();
    for part in tail.split('!') {
        let part = part.strip_suffix(".exe").unwrap_or(part);
        // A store package: "publisher.name_hash".
        let part = part.split('_').next().unwrap_or(part);
        for seg in std::iter::once(part).chain(part.split('.')) {
            if seg.len() >= 3 && !keys.iter().any(|k| k == seg) {
                keys.push(seg.to_string());
            }
        }
    }
    let f: String = friendly.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if f.len() >= 3 && !keys.contains(&f) {
        keys.push(f);
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::app_keys;

    #[test]
    fn keys() {
        let has = |id: &str, name: &str, k: &str| app_keys(id, name).iter().any(|x| x == k);
        assert!(has("Spotify.exe", "Spotify", "spotify"));
        assert!(has("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Spotify", "spotify"));
        assert!(has("MSEdge", "Edge", "msedge"));
        assert!(has("308046B0AF4A39CB", "Firefox", "firefox"));
        assert!(has(r"C:\Apps\vlc.exe", "VLC", "vlc"));
    }
}

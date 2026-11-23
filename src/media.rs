//! Now-playing info and media controls through Windows' media session API
//! (the same one the volume flyout uses). Works with Spotify, browsers, etc.

use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{Interface, Result};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat,
    BitmapTransform, ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::IRandomAccessStream;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::image::Image;
use crate::lyrics;

/// Posted to the notch window whenever [`MediaInfo`] or the lyrics change.
pub const WM_MEDIA_CHANGED: u32 = WM_APP + 1;
/// Album art is decoded to a square of this many pixels.
pub const ART_SIZE: u32 = 96;


#[derive(Clone, Default)]
pub struct MediaInfo {
    pub available: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    /// Friendly name of the app that is playing ("Spotify", "Chrome" ...).
    pub source: String,
    /// The app's own id (AppUserModelId), to find its audio session.
    pub app_id: String,
    pub duration_ms: u64,
    position_ms: u64,
    sampled: Option<Instant>,
    pub art: Option<Arc<Image>>,
    /// None when the app does not report shuffle.
    pub shuffle: Option<bool>,
}

/// A drawn cover for the demo song: a striped sun setting over a road.
fn demo_art() -> Image {
    let n = ART_SIZE as usize;
    let mut px = vec![0u8; n * n * 4];
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 / n as f32, y as f32 / n as f32);
            // Sky: deep plum at the top to warm orange at the horizon.
            let t = (fy / 0.66).min(1.0);
            let mut c = [lerp(0.20, 1.00, t), lerp(0.08, 0.52, t), lerp(0.30, 0.22, t)];
            // The sun, cut by stripes near its bottom.
            let (dx, dy) = (fx - 0.5, fy - 0.56);
            let stripe = fy > 0.5 && ((fy - 0.5) * 40.0) as i32 % 2 == 1;
            if dx * dx + dy * dy < 0.07 && fy < 0.66 && !stripe {
                c = [1.0, lerp(0.86, 0.62, (fy - 0.3) / 0.36), 0.35];
            }
            // Ground and a road narrowing to the horizon.
            if fy >= 0.66 {
                let k = (fy - 0.66) / 0.34;
                c = [0.16, 0.10, 0.20];
                if dx.abs() < 0.03 + 0.42 * k {
                    c = [0.28, 0.22, 0.32];
                    if dx.abs() < 0.012 + 0.02 * k && ((fy * 30.0) as i32) % 2 == 0 {
                        c = [1.0, 0.82, 0.45];
                    }
                }
            }
            let i = (y * n + x) * 4;
            px[i] = (c[2] * 255.0) as u8;
            px[i + 1] = (c[1] * 255.0) as u8;
            px[i + 2] = (c[0] * 255.0) as u8;
            px[i + 3] = 255;
        }
    }
    Image::from_straight(ART_SIZE, ART_SIZE, px)
}

impl MediaInfo {
    /// A well-known song for `--snapshot` pictures (`FUZZIE_DEMO=1`), with a drawn cover.
    pub fn demo(title: &str, artist: &str, duration_ms: u64) -> Self {
        Self {
            available: true,
            playing: true,
            title: title.into(),
            artist: artist.into(),
            source: "Spotify".into(),
            duration_ms,
            position_ms: 49_000,
            shuffle: Some(true),
            art: Some(Arc::new(demo_art())),
            ..Default::default()
        }
    }

    /// Identifies the track; lyrics and art are matched against it.
    pub fn key(&self) -> String {
        format!("{}\u{1}{}", self.artist, self.title)
    }

    /// Current playback position, extrapolated since the last sample.
    pub fn position_ms(&self) -> u64 {
        let mut p = self.position_ms;
        if self.playing {
            if let Some(s) = self.sampled {
                p += s.elapsed().as_millis() as u64;
            }
        }
        if self.duration_ms > 0 { p.min(self.duration_ms) } else { p }
    }

    /// True when the change is worth a redraw (ignores normal position drift).
    fn differs(&self, o: &Self) -> bool {
        self.available != o.available
            || self.playing != o.playing
            || self.title != o.title
            || self.artist != o.artist
            || self.source != o.source
            || self.duration_ms != o.duration_ms
            || self.shuffle != o.shuffle
            || self.art.as_ref().map(|a| a.id) != o.art.as_ref().map(|a| a.id)
            || self.position_ms().abs_diff(o.position_ms()) > 1500
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Prev,
    PlayPause,
    Next,
    Seek(u64),
    Shuffle(bool),
}

/// Starts the media thread. It polls twice a second and runs commands as they arrive.
pub fn spawn(hwnd: HWND, shared: Arc<Mutex<MediaInfo>>, lyrics: lyrics::Shared) -> mpsc::Sender<Cmd> {
    let (tx, rx) = mpsc::channel::<Cmd>();
    let hwnd_raw = hwnd.0 as isize; // HWND is not Send; pass the raw value.
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let notify = move || unsafe {
            let _ = PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_MEDIA_CHANGED, WPARAM(0), LPARAM(0));
        };
        let mut manager: Option<Manager> = None;
        let mut key = String::new();
        let mut art: Option<Arc<Image>> = None;
        let mut art_tries = 0u32;
        let mut art_checked = Instant::now();
        loop {
            if manager.is_none() {
                manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
            }
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(cmd) => {
                    if let Some(m) = &manager {
                        if let Err(e) = run(m, cmd) {
                            eprintln!("media command {cmd:?} failed: {e}");
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            let session = manager.as_ref().and_then(|m| m.GetCurrentSession().ok());
            let mut info = session.as_ref().and_then(|s| read(s).ok()).unwrap_or_default();

            let new_key = info.key();
            if new_key != key {
                key = new_key;
                art = None;
                // Apps often update the thumbnail a moment after the title, so try a few times.
                art_tries = 4;
                if info.available && !info.title.is_empty() {
                    lyrics::request(&lyrics, key.clone(), &info.title, &info.artist, info.duration_ms, notify);
                }
            }
            // Keep retrying every few seconds while a track has no art yet.
            if art_tries == 0 && art.is_none() && info.available && art_checked.elapsed() > Duration::from_secs(3) {
                art_tries = 1;
            }
            if art_tries > 0 {
                art_tries -= 1;
                art_checked = Instant::now();
                if let Some(pixels) = session.as_ref().and_then(|s| read_art(s).ok()) {
                    if art.as_ref().map(|a| &a.pixels) != Some(&pixels) {
                        art = Some(Arc::new(Image::from_premultiplied(ART_SIZE, ART_SIZE, pixels)));
                    }
                }
            }
            info.art = art.clone();

            let changed = {
                let mut cur = shared.lock().unwrap();
                let changed = cur.differs(&info);
                *cur = info;
                changed
            };
            if changed {
                notify();
            }
        }
    });
    tx
}

fn read(session: &Session) -> Result<MediaInfo> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let playback = session.GetPlaybackInfo()?;
    let status = playback.PlaybackStatus()?;
    let shuffle = playback.IsShuffleActive().and_then(|r| r.Value()).ok();
    let playing = status == Status::Playing;
    let timeline = session.GetTimelineProperties()?;
    let ms = |ticks: i64| (ticks.max(0) / 10_000) as u64;
    let duration_ms = ms(timeline.EndTime()?.Duration - timeline.StartTime()?.Duration);
    let mut position_ms = ms(timeline.Position()?.Duration);
    if playing {
        // The position is a snapshot from LastUpdatedTime; move it forward to now.
        let updated = timeline.LastUpdatedTime()?.UniversalTime;
        let now = unsafe { GetSystemTimeAsFileTime() };
        let now = ((now.dwHighDateTime as i64) << 32) | now.dwLowDateTime as i64;
        if updated > 0 && now > updated {
            position_ms += ms(now - updated);
        }
    }
    if duration_ms > 0 {
        position_ms = position_ms.min(duration_ms);
    }
    let app_id = session.SourceAppUserModelId()?.to_string();
    Ok(MediaInfo {
        available: true,
        playing,
        title: props.Title()?.to_string(),
        artist: clean_artist(&props.Artist()?.to_string()),
        source: friendly_source(&app_id),
        app_id,
        duration_ms,
        position_ms,
        sampled: Some(Instant::now()),
        art: None,
        shuffle,
    })
}

/// Decodes the session thumbnail into a centred square of `ART_SIZE` pixels.
fn read_art(session: &Session) -> Result<Vec<u8>> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let stream = props.Thumbnail()?.OpenReadAsync()?.join()?;
    let decoder = BitmapDecoder::CreateAsync(&stream.cast::<IRandomAccessStream>()?)?.join()?;
    let (w, h) = (decoder.PixelWidth()?.max(1), decoder.PixelHeight()?.max(1));
    // Scale the short side to ART_SIZE, then crop the middle.
    let s = ART_SIZE as f64 / w.min(h) as f64;
    let (sw, sh) = (((w as f64 * s).round() as u32).max(ART_SIZE), ((h as f64 * s).round() as u32).max(ART_SIZE));
    let t = BitmapTransform::new()?;
    t.SetScaledWidth(sw)?;
    t.SetScaledHeight(sh)?;
    t.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    t.SetBounds(BitmapBounds { X: (sw - ART_SIZE) / 2, Y: (sh - ART_SIZE) / 2, Width: ART_SIZE, Height: ART_SIZE })?;
    let data = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &t,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?;
    Ok(data.DetachPixelData()?.to_vec())
}

fn run(m: &Manager, cmd: Cmd) -> Result<()> {
    let session = m.GetCurrentSession()?;
    let op = match cmd {
        Cmd::Prev => session.TrySkipPreviousAsync()?,
        Cmd::PlayPause => session.TryTogglePlayPauseAsync()?,
        Cmd::Next => session.TrySkipNextAsync()?,
        Cmd::Seek(ms) => session.TryChangePlaybackPositionAsync(ms as i64 * 10_000)?,
        Cmd::Shuffle(on) => session.TryChangeShuffleActiveAsync(on)?,
    };
    op.join()?;
    Ok(())
}

/// YouTube channels show up as "Artist - Topic" or "ArtistVEVO".
fn clean_artist(a: &str) -> String {
    let a = a.trim();
    let a = a.strip_suffix(" - Topic").unwrap_or(a);
    let a = a.strip_suffix("VEVO").unwrap_or(a);
    a.trim().to_string()
}

fn friendly_source(app_id: &str) -> String {
    let lower = app_id.to_lowercase();
    const KNOWN: &[(&str, &str)] = &[
        ("spotify", "Spotify"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("vivaldi", "Vivaldi"),
        ("zen", "Zen"),
        ("deezer", "Deezer"),
        ("tidal", "TIDAL"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
        ("vlc", "VLC"),
        ("foobar2000", "foobar2000"),
        ("musicbee", "MusicBee"),
        ("aimp", "AIMP"),
        ("discord", "Discord"),
        ("zunemusic", "Media Player"),
        ("youtube", "YouTube Music"),
    ];
    if let Some((_, name)) = KNOWN.iter().find(|(k, _)| lower.contains(k)) {
        return name.to_string();
    }
    // "Publisher.App_hash!App" or "C:\...\app.exe" -> "app"
    let base = app_id.rsplit(['\\', '!']).next().unwrap_or(app_id);
    base.trim_end_matches(".exe").trim_end_matches(".EXE").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources() {
        assert_eq!(friendly_source("Spotify.exe"), "Spotify");
        assert_eq!(friendly_source("MSEdge"), "Edge");
        assert_eq!(friendly_source("C:\\Apps\\Player.exe"), "Player");
    }

    #[test]
    fn artists() {
        assert_eq!(clean_artist("Meyra - Topic"), "Meyra");
        assert_eq!(clean_artist("ArtistVEVO"), "Artist");
    }
}

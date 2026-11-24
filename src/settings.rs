use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BgStyle {
    Solid,
    /// Fades from the background colour into the accent colour at the bottom.
    #[default]
    Gradient,
    /// Like Gradient, but the colour comes from the album art.
    Album,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenMode {
    /// Expand when the pointer enters the notch.
    #[default]
    Hover,
    /// Expand on click.
    Click,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleStyle {
    /// Collapsed notch shows the clock when nothing plays.
    #[default]
    Clock,
    /// A thin bar only.
    Minimal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotchStyle {
    /// Glued to the screen edge.
    #[default]
    Attached,
    /// Floats a little away from the edge, all corners rounded.
    Floating,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    TopCenter,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Wherever the user dragged it (`custom_x`, `custom_y`).
    Custom,
}

impl Placement {
    pub fn is_bottom(self) -> bool {
        matches!(self, Placement::BottomLeft | Placement::BottomRight)
    }
}

/// User settings, stored as `%APPDATA%\Fuzzie\settings.json`.
/// Unknown or missing fields fall back to defaults, so old files keep working.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub start_with_windows: bool,
    /// Install new versions from GitHub releases by itself.
    pub auto_update: bool,
    pub open_mode: OpenMode,
    pub clock_24h: bool,
    pub show_seconds: bool,
    /// Extra size multiplier on top of the monitor's DPI scale.
    pub scale: f32,
    /// Background colour as `#RRGGBB`.
    pub background: String,
    pub background_opacity: f32,
    /// Accent colour as `#RRGGBB`.
    pub accent: String,
    /// Show the current synced lyric line in the expanded notch.
    pub show_lyrics: bool,
    /// Right-click the notch or the desktop fox: it turns see-through and lets
    /// clicks through for this many seconds (1..5).
    pub ghost_secs: f32,
    pub hud_volume: bool,
    pub hud_capslock: bool,
    pub hud_audio_device: bool,
    pub bg_style: BgStyle,
    pub clipboard_history: bool,
    pub clipboard_max: usize,
    /// Keep the clipboard history across restarts.
    pub remember_after_restart: bool,
    pub hide_in_fullscreen: bool,
    pub idle_style: IdleStyle,
    pub style: NotchStyle,
    pub placement: Placement,
    /// Top-centre of the notch body in screen pixels, for `Placement::Custom`.
    pub custom_x: i32,
    pub custom_y: i32,
    /// Translucent background.
    pub glass: bool,
    pub show_mascot: bool,
    /// Skin and accessory glow effects for the notch / UI mascot (independent of Desktop Fuzzie).
    pub mascot_glow: bool,
    pub nap_enabled: bool,
    pub nap_after_secs: u32,
    pub nap_while_music: bool,
    pub desk_fox: bool,
    /// Top-left of the desktop fox window in screen pixels (-1, -1 = bottom right).
    pub desk_x: i32,
    pub desk_y: i32,
    /// Desktop fox size, 0.25..2.0 (1 = 100 px head).
    pub desk_scale: f32,
    pub desk_keys: bool,
    pub desk_clicks: bool,
    pub desk_blink: bool,
    pub desk_nap: bool,
    /// Breathing, tail and skin / accessory effects (costs a little CPU).
    pub desk_anim: bool,
    /// Desktop fox locked in place (cannot be dragged).
    pub desk_lock: bool,
    /// Skin and accessory glows on the desktop fox.
    pub desk_glow: bool,
    /// The desktop fox wears the chosen accessory.
    pub desk_acc: bool,
    /// Sits on the taskbar (dragging moves it sideways only).
    pub desk_on_taskbar: bool,
    /// Gradient background: how much accent colour reaches the bottom (0..1),
    /// and how gently it fades in (0 = from the middle, 1 = from the very top).
    pub gradient_strength: f32,
    pub gradient_softness: f32,
    /// Index into `mascot::SPECIES`.
    pub species: usize,
    /// Index into `mascot::SKINS`.
    pub skin: usize,
    /// Head/face accessory index into `mascot::ACCESSORIES` (0 = none).
    pub acc: usize,
    /// Body/back/gear accessory index into `mascot::ACCESSORIES` (0 = none).
    pub acc2: usize,
    pub audio_visualizer: bool,
    pub start_with_empty_shelf: bool,
    pub hud_brightness: bool,
    pub hud_wifi: bool,
    pub show_notifications: bool,
    pub show_message_preview: bool,
    /// Connected to Fuzzie friends.
    pub social_on: bool,
    /// Friends see the song playing here.
    pub share_music: bool,
    /// Name friends see ("" = "Fuzzie").
    pub social_name: String,
    /// Size of the settings window (1 = normal), set by dragging its edges.
    pub settings_zoom: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_with_windows: true,
            auto_update: true,
            open_mode: OpenMode::Hover,
            clock_24h: true,
            show_seconds: false,
            scale: 0.9,
            background: "#000000".into(),
            background_opacity: 1.0,
            accent: "#FF8A3D".into(),
            show_lyrics: true,
            ghost_secs: 2.0,
            hud_volume: true,
            hud_capslock: true,
            hud_audio_device: true,
            bg_style: BgStyle::Gradient,
            clipboard_history: true,
            clipboard_max: 50,
            remember_after_restart: true,
            hide_in_fullscreen: true,
            idle_style: IdleStyle::Clock,
            style: NotchStyle::Attached,
            placement: Placement::TopCenter,
            custom_x: -1,
            custom_y: -1,
            glass: false,
            show_mascot: true,
            mascot_glow: true,
            nap_enabled: true,
            nap_after_secs: 60,
            nap_while_music: true,
            desk_fox: true,
            desk_x: -1,
            desk_y: -1,
            desk_scale: 0.5,
            desk_keys: true,
            desk_clicks: true,
            desk_blink: true,
            desk_nap: true,
            desk_anim: true,
            desk_lock: false,
            social_on: true,
            share_music: true,
            social_name: String::new(),
            settings_zoom: 1.0,
            desk_glow: true,
            desk_acc: true,
            desk_on_taskbar: true,
            gradient_strength: 0.3,
            gradient_softness: 1.0,
            species: 0,
            skin: 0,
            acc: 0,
            acc2: 0,
            audio_visualizer: true,
            start_with_empty_shelf: false,
            hud_brightness: true,
            hud_wifi: true,
            show_notifications: true,
            show_message_preview: true,
        }
    }
}

pub fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("Fuzzie")
}

impl Settings {
    fn path() -> PathBuf {
        data_dir().join("settings.json")
    }

    /// Loads settings; writes the defaults on first run so the file can be edited.
    pub fn load() -> Self {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(text) => {
                let mut s: Self = serde_json::from_str(&text).unwrap_or_else(|e| {
                    eprintln!("settings.json is invalid ({e}); using defaults");
                    Self::default()
                });
                // The first default accent; matches the preset now.
                if s.accent.eq_ignore_ascii_case("#C800F0") {
                    s.accent = "#C902F0".into();
                }
                s
            }
            Err(_) => {
                let s = Self::default();
                if let Err(e) = s.save() {
                    eprintln!("could not write settings: {e}");
                }
                s
            }
        }
    }

    /// Atomic save: write a temp file, then rename over the old one.
    pub fn save(&self) -> io::Result<()> {
        fs::create_dir_all(data_dir())?;
        let path = Self::path();
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(tmp, path)
    }

    /// Background opacity: fully opaque unless the glass effect is on.
    pub fn opacity(&self) -> f32 {
        if self.glass { 0.72 } else { 1.0 }
    }

    pub fn scale(&self) -> f32 {
        self.scale.clamp(0.5, 1.5)
    }

    pub fn desk_size(&self) -> f32 {
        self.desk_scale.clamp(0.25, 2.0)
    }

    /// How long a right-click leaves the notch / desktop fox see-through, in ms.
    pub fn ghost_ms(&self) -> u32 {
        (self.ghost_secs.clamp(1.0, 5.0) * 1000.0) as u32
    }
}

/// Parses `#RRGGBB` into 0..1 floats; falls back to the given colour.
pub fn parse_hex(s: &str, fallback: [f32; 3]) -> [f32; 3] {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return fallback;
    }
    let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map(|v| v as f32 / 255.0);
    match (ch(0), ch(2), ch(4)) {
        (Ok(r), Ok(g), Ok(b)) => [r, g, b],
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#FF0000", [0.0; 3]), [1.0, 0.0, 0.0]);
        assert_eq!(parse_hex("00ff00", [0.0; 3]), [0.0, 1.0, 0.0]);
        assert_eq!(parse_hex("#xyz", [0.5; 3]), [0.5; 3]);
    }

    #[test]
    fn partial_json_uses_defaults() {
        let s: Settings = serde_json::from_str(r#"{"clock_24h": false, "unknown": 1}"#).unwrap();
        assert!(!s.clock_24h);
        assert_eq!(s.open_mode, OpenMode::Hover);
    }
}

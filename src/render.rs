//! Draws the notch with Direct2D into a 32-bit DIB and pushes it to the
//! layered window with per-pixel alpha (fully transparent pixels let clicks through).
//!
//! The render target is a software one: the canvas is small, and a hardware
//! target would load the GPU driver into the process (tens of MB) only to read
//! every frame back for `UpdateLayeredWindow`. All brushes are kept between
//! frames; only the part of the canvas the notch covers is sent to the window.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1BitmapBrush, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory,
    ID2D1LinearGradientBrush, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_BRUSH_PROPERTIES,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_GAMMA_2_2,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    UpdateLayeredWindow, UpdateLayeredWindowIndirect, ULW_ALPHA, UPDATELAYEREDWINDOWINFO,
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::clipboard::{Clip, ClipData};
use crate::image::Image;
use crate::mascot;
use crate::media::MediaInfo;
use crate::settings::BgStyle;
use crate::shelf::ShelfFile;

/// Logical (96-DPI) canvas size. The notch is drawn centred at the top of it.
/// Wide enough that the fillets beside the expanded notch fit on both sides.
pub const CANVAS_W: f32 = 520.0;
pub const CANVAS_H: f32 = 250.0;

const COLLAPSED_H: f32 = 32.0;
/// Collapsed clock row: margin at each end, the inset when the fox or the
/// album art sits on the left, and the room the music bars add on the right.
const CLOCK_PAD: f32 = 15.0;
const CLOCK_ICON_IN: f32 = 48.0;
const CLOCK_BARS: f32 = 26.0;
const EXPANDED_W: f32 = 470.0;
const EXPANDED_H: f32 = 232.0;

pub const PAGE_MUSIC: usize = 0;
pub const PAGE_SHELF: usize = 1;
pub const PAGE_FRIENDS: usize = 2;
pub const PAGE_COUNT: usize = 3;
const PAGE_ICONS: [&str; PAGE_COUNT] = ["\u{E8D6}", "\u{E8B7}", "\u{E716}"];

/// Room around the notch body for shapes that reach past it (mascot accessories).
const DIRTY_MARGIN: f32 = 12.0;
/// Free space above a notch glued to the bottom edge, for the fillet that runs
/// up the side of the screen in the bottom corners.
const BOTTOM_PAD: f32 = 26.0;
/// Fillets are this many times taller than wide, like a drop running off the edge.
const FILLET_TALL: f32 = 1.35;

/// Clipboard rows visible at once.
pub const CLIP_ROWS: usize = 4;

const WHITE: [f32; 3] = [1.0; 3];
const GREY: [f32; 3] = [0.6; 3];

/// A short-lived indicator shown in the collapsed notch.
#[derive(Clone, Debug, PartialEq)]
pub enum Hud {
    Volume { level: f32, muted: bool },
    CapsLock(bool),
    Device(String),
    /// A Windows notification.
    Notice(crate::notify::Notice),
}

impl Hud {
    fn width(&self) -> f32 {
        match self {
            Hud::Volume { .. } => 270.0,
            Hud::CapsLock(_) => 230.0,
            Hud::Device(_) => 300.0,
            Hud::Notice(_) => 390.0,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Hud::Notice(n) if n.body.is_empty() => 50.0,
            Hud::Notice(_) => 64.0,
            _ => COLLAPSED_H,
        }
    }
}

/// Clickable areas returned by [`Renderer::draw`], in logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Prev,
    PlayPause,
    Next,
    /// The progress bar; the x position picks the seek target.
    Seek,
    Shuffle,
    /// The speaker icon: toggles mute.
    Volume,
    /// The volume bar: click, drag or scroll to set the level.
    VolumeBar,
    Lyrics,
    Settings,
    Page(usize),
    /// The tab on the right edge that goes to the next page.
    NextPage,
    ShelfFile(usize),
    /// The small buttons on a hovered shelf file: take it off the shelf, show it in its folder.
    ShelfRemove(usize),
    ShelfReveal(usize),
    ShelfClear,
    Clip(usize),
    /// The × on a hovered clipboard row.
    ClipRemove(usize),
    ClipClear,
    /// The clipboard list area, for the mouse wheel.
    ClipList,
    /// The lyrics column (for the wheel) and one line in it (click to play from there).
    LyricList,
    LyricLine(usize),
    AddFriend,
    Requests,
    /// Friends page sub-views: back to the list, copy our code, the code box,
    /// paste into it, send the request, answer / cancel the n-th request.
    FriendsBack,
    CopyCode,
    CodeBox,
    PasteCode,
    SendCode,
    Accept(usize),
    Decline(usize),
    Cancel(usize),
    /// Requests page tab: 0 received, 1 sent.
    ReqTab(u8),
    /// Open the n-th friend's profile card, poke/kiss them, or listen along.
    FriendCard(usize),
    Poke(usize),
    Kiss(usize),
    Along(usize),
}

#[derive(Clone, Copy, PartialEq)]
pub enum LyricsStatus {
    Loading,
    Missing,
    Found,
}

/// Which screen edge the notch is glued to.
#[derive(Clone, Copy, PartialEq)]
pub enum Edge {
    Top,
    Bottom,
    /// Floating: all four corners rounded.
    None,
}

pub struct Theme {
    pub background: [f32; 3],
    pub opacity: f32,
    pub accent: [f32; 3],
    pub style: BgStyle,
    pub grad_strength: f32,
    pub grad_soft: f32,
}

pub struct Scene<'a> {
    /// 0 = collapsed pill, 1 = fully expanded panel.
    pub open: f32,
    /// The page being shown (for hit areas) and the animated page position
    /// (fractional while sliding from one page to the next).
    pub page: usize,
    pub page_pos: f32,
    pub clock: &'a str,
    pub date: &'a str,
    pub media: &'a MediaInfo,
    pub position_ms: u64,
    /// Synced lyric lines (start ms, text); an empty text is an instrumental pause.
    pub lyric_lines: &'a [(u64, String)],
    /// The line being sung; -1 before the first line.
    pub lyric_index: isize,
    /// 0..1 since the current line started; drives the scroll animation.
    pub lyric_t: f32,
    pub lyrics_status: LyricsStatus,
    pub lyrics_mode: bool,
    /// How many lines the reader scrolled the lyrics away from the sung line.
    pub lyric_scroll: f32,
    /// The notch is on the lower half of the screen (the gradient runs upwards).
    pub low: bool,
    /// Friends, sorted as the friends page shows them.
    pub social: &'a crate::social::State,
    pub social_on: bool,
    /// Friends page: 0 the list, 1 add a friend, 2 requests, 3 friend/bot profile.
    pub friend_view: u8,
    pub friend_tab: u8,
    /// Selected friend code when `friend_view == 3`.
    pub friend_selected: &'a str,
    /// Active Poke/Kiss 1-second combo: (friend_code, is_kiss, count).
    pub combo: Option<(&'a str, bool, u32)>,
    /// Floating "+1" feedback particles above Poke/Kiss buttons: (is_kiss, age 0..1, dx).
    pub pops: &'a [(bool, f32, f32)],
    /// True briefly after poking/kissing so the friend's profile avatar smiles happily.
    pub profile_happy: bool,
    /// Incoming Poke/Kiss banner shown at the top of the expanded notch even while poking/kissing.
    pub top_banner: Option<(&'a crate::notify::Notice, f32)>,
    /// The friend code typed so far, whether the box has the keyboard, and a short note.
    pub friend_input: &'a str,
    pub friend_typing: bool,
    pub friend_note: Option<&'a str>,
    /// 0 = two columns (lyrics on the right), 1 = wide music layout (lyrics off).
    pub wide: f32,
    /// Volume 0..1 and mute of the app playing the media; None when it has no
    /// audio session.
    pub volume: Option<(f32, bool)>,
    /// The volume slider beside the speaker: 0 folded, 1 open.
    pub vol_open: f32,
    pub shelf: &'a [ShelfFile],
    pub clips: &'a [Clip],
    pub clip_scroll: usize,
    pub drag_over: bool,
    /// HUD to show and how far it has animated in (0..1).
    pub hud: Option<(&'a Hud, f32)>,
    /// Seconds since start; drives small animations.
    pub time: f32,
    pub theme: Theme,
    pub edge: Edge,
    /// Also glued to the left (-1) or right (1) side of the screen (corner placements), or 0.
    pub side: i8,
    /// "Minimal" idle style: a thin bar when nothing plays.
    pub minimal: bool,
    /// Bouncing bars in the collapsed notch while music plays.
    pub visualizer: bool,
    /// Mascot look and whether it is napping; None hides the mascot.
    pub mascot: Option<(mascot::Look, bool)>,
    /// Smooth music-dance & floating notes blend (0 silent .. 1 vibing).
    pub mascot_music: f32,
    /// Hover highlight per hit area, 0..1, eased by the caller.
    pub hover: &'a [(Hit, f32)],
    /// The area just clicked and how much of its feedback is left (1 → 0).
    pub flash: Option<(Hit, f32)>,
}

impl Scene<'_> {
    /// The lyric line being sung, if it has words and lyrics are on.
    fn current_line(&self) -> Option<&str> {
        if !self.lyrics_mode || self.lyrics_status != LyricsStatus::Found || self.lyric_index < 0 {
            return None;
        }
        let line = self.lyric_lines.get(self.lyric_index as usize)?.1.as_str();
        (!line.is_empty()).then_some(line)
    }

    /// How much the pointer hovers `hit` (0..1).
    fn hv(&self, hit: Hit) -> f32 {
        self.hover.iter().find(|(h, _)| *h == hit).map_or(0.0, |e| e.1)
    }

    /// Click feedback left on `hit` (1 just clicked → 0 gone).
    fn fl(&self, hit: Hit) -> f32 {
        self.flash.filter(|f| f.0 == hit).map_or(0.0, |f| f.1)
    }
}

/// Fast start, gentle stop.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

struct Fonts {
    clock_small: IDWriteTextFormat,
    clock: IDWriteTextFormat,
    date: IDWriteTextFormat,
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_center: IDWriteTextFormat,
    small_right: IDWriteTextFormat,
    section: IDWriteTextFormat,
    big: IDWriteTextFormat,
    pill: IDWriteTextFormat,
    lyric: IDWriteTextFormat,
    lyric_small: IDWriteTextFormat,
    icons: IDWriteTextFormat,
    icons_small: IDWriteTextFormat,
    icons_tiny: IDWriteTextFormat,
    icons_big: IDWriteTextFormat,
}

type Hits = Vec<(D2D_RECT_F, Hit)>;

pub struct Renderer {
    factory: ID2D1Factory,
    rt: ID2D1DCRenderTarget,
    dw: IDWriteFactory,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub width: i32,
    pub height: i32,
    scale: f32,
    f: Fonts,
    /// The one solid brush; its colour is set before each use.
    solid: ID2D1SolidColorBrush,
    /// Background gradient for the current colours (end point set per frame).
    bg_grad: RefCell<Option<([u32; 9], ID2D1LinearGradientBrush)>>,
    /// GPU copies of images and their brushes, keyed by `Image::id`, plus whether they were used this frame.
    bitmaps: RefCell<HashMap<u64, (ID2D1Bitmap, ID2D1BitmapBrush, bool)>>,
    /// Text widths already measured, keyed by format and text.
    measured: RefCell<Vec<(usize, String, f32)>>,
    /// UTF-16 scratch buffer for drawing text.
    wide: RefCell<Vec<u16>>,
    /// Average album art colour for the "Album" background, keyed by art id.
    art_color: RefCell<Option<(u64, [f32; 3])>>,
    /// The notch body drawn in the last frame, in logical coordinates.
    pub body: D2D_RECT_F,
    /// The body drawn and the body sent to the window last time. Only the union of
    /// the old and new body (plus a margin) is cleared, drawn and sent.
    drawn_body: Cell<Option<D2D_RECT_F>>,
    /// A frame was sent to the window already.
    shown: Cell<bool>,
    /// The pixel rectangle of `dc` the render target is bound to.
    bound: Cell<Option<RECT>>,
    /// Between BeginDraw and EndDraw.
    drawing: Cell<bool>,
    /// Canvas to render target transform (the bound rectangle's offset).
    origin: Cell<Matrix3x2>,
    /// Text colours for the current background (dark text on light themes).
    fg: Cell<[f32; 3]>,
    dim: Cell<[f32; 3]>,
    /// Collapsed width, eased toward its target so lyric lines do not make it jump.
    collapsed_w: Cell<f32>,
    /// True while the collapsed width is still easing (the caller keeps redrawing).
    pub settling: Cell<bool>,
    /// Opacity of the whole window, 0..255 (lower while see-through).
    ghost: Cell<u8>,
    /// True when the last frame drew something that moves on its own (mascot,
    /// equaliser, pulsing dots), so the caller keeps a modest frame rate.
    pub live: Cell<bool>,
    /// Everything is drawn this far down the canvas (bottom placements).
    y_pad: Cell<f32>,
}

thread_local! {
    /// One software Direct2D target for every layered window (the notch and the
    /// desktop fox). Each software target brings its own rasteriser with a large
    /// working set, so sharing one saves tens of MB.
    static SHARED: RefCell<Option<(ID2D1Factory, ID2D1DCRenderTarget)>> = const { RefCell::new(None) };
    /// Someone else bound the shared target since the notch last did.
    static LENT: Cell<bool> = const { Cell::new(false) };
}

/// The shared factory and DC render target (made on first use).
pub fn shared_target() -> Result<(ID2D1Factory, ID2D1DCRenderTarget)> {
    if let Some(t) = SHARED.with(|s| s.borrow().clone()) {
        return Ok(t);
    }
    let t = unsafe {
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = factory.CreateDCRenderTarget(&props)?;
        (factory, rt)
    };
    SHARED.with(|s| *s.borrow_mut() = Some(t.clone()));
    Ok(t)
}

/// Call before binding the shared target to another DC or changing its DPI:
/// the notch then binds it again before its next frame.
pub fn lend_target() {
    LENT.with(|l| l.set(true));
}

impl Renderer {
    /// `scale` = monitor DPI / 96 × user scale.
    pub fn new(scale: f32) -> Result<Self> {
        let width = (CANVAS_W * scale).round() as i32;
        let height = (CANVAS_H * scale).round() as i32;
        unsafe {
            let (factory, rt) = shared_target()?;
            lend_target();
            // Draw in logical units; Direct2D scales to pixels.
            rt.SetDpi(96.0 * scale, 96.0 * scale);
            let solid = rt.CreateSolidColorBrush(&rgba(WHITE, 1.0), None)?;

            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height, // top-down
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

            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let text = w!("Segoe UI Variable Text");
            let icon_font = if font_exists(&dw, w!("Segoe Fluent Icons")) {
                w!("Segoe Fluent Icons")
            } else {
                w!("Segoe MDL2 Assets") // Windows 10, same code points
            };
            let semi = DWRITE_FONT_WEIGHT_SEMI_BOLD;
            let normal = DWRITE_FONT_WEIGHT_NORMAL;
            let (left, center, right) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING);
            let f = Fonts {
                clock_small: format(&dw, text, 13.0, semi, center)?,
                clock: format(&dw, text, 15.0, semi, right)?,
                date: format(&dw, text, 13.0, normal, left)?,
                title: format(&dw, text, 15.0, semi, left)?,
                body: format(&dw, text, 13.0, normal, left)?,
                small: format(&dw, text, 11.5, normal, left)?,
                small_center: format(&dw, text, 11.0, normal, center)?,
                small_right: format(&dw, text, 11.5, normal, right)?,
                section: format(&dw, text, 14.5, semi, left)?,
                big: format(&dw, text, 22.0, windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT_BOLD, left)?,
                pill: format(&dw, text, 12.5, semi, center)?,
                lyric: format(&dw, text, 13.5, semi, left)?,
                lyric_small: format(&dw, text, 13.0, semi, left)?,
                icons: format(&dw, icon_font, 16.0, normal, center)?,
                icons_small: format(&dw, icon_font, 13.0, normal, center)?,
                icons_tiny: format(&dw, icon_font, 9.0, normal, center)?,
                icons_big: format(&dw, icon_font, 26.0, normal, center)?,
            };
            for tf in [&f.clock_small, &f.clock, &f.date, &f.title, &f.body, &f.small, &f.small_center, &f.small_right, &f.section, &f.big, &f.pill, &f.lyric, &f.lyric_small] {
                let sign = dw.CreateEllipsisTrimmingSign(tf)?;
                let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                tf.SetTrimming(&trimming, &sign)?;
            }

            Ok(Self {
                factory,
                rt,
                dw,
                dc,
                bitmap,
                old,
                bits: bits as *mut u8,
                width,
                height,
                scale,
                f,
                solid,
                bg_grad: RefCell::new(None),
                bitmaps: RefCell::new(HashMap::new()),
                measured: RefCell::new(Vec::new()),
                wide: RefCell::new(Vec::with_capacity(128)),
                art_color: RefCell::new(None),
                body: D2D_RECT_F::default(),
                drawn_body: Cell::new(None),
                shown: Cell::new(false),
                bound: Cell::new(None),
                drawing: Cell::new(false),
                origin: Cell::new(Matrix3x2::identity()),
                fg: Cell::new(WHITE),
                dim: Cell::new(GREY),
                collapsed_w: Cell::new(0.0),
                settling: Cell::new(false),
                ghost: Cell::new(255),
                live: Cell::new(false),
                y_pad: Cell::new(0.0),
            })
        }
    }

    /// Draws one frame. Returns the clickable areas in logical coordinates.
    pub fn draw(&mut self, s: &Scene) -> Result<Hits> {
        let mut hits = Vec::new();
        for v in self.bitmaps.borrow_mut().values_mut() {
            v.2 = false;
        }
        self.live.set(false);
        // `frame` lays the notch out, then starts drawing. EndDraw must run even
        // when a part of the frame fails, or every later frame fails too and the
        // notch freezes on its last picture.
        let drawn = self.frame(s, &mut hits);
        let pad = self.y_pad.get();
        if pad != 0.0 {
            for (r, _) in hits.iter_mut() {
                r.top += pad;
                r.bottom += pad;
            }
        }
        let ended = if self.drawing.replace(false) { unsafe { self.rt.EndDraw(None, None) } } else { Ok(()) };
        // Drop bitmaps that were not drawn this frame.
        self.bitmaps.borrow_mut().retain(|_, v| v.2);
        drawn?;
        if ended.is_err() {
            self.bound.set(None);
        }
        ended?;
        Ok(hits)
    }

    /// Binds the render target to the part of the canvas that changes and
    /// starts drawing there. The DC render target copies its whole area in and
    /// out every frame, so a small area keeps a frame well under a millisecond.
    fn begin(&self, area: D2D_RECT_F) -> Result<()> {
        let k = self.scale;
        let px = RECT {
            left: ((area.left * k).floor() as i32).max(0),
            top: ((area.top * k).floor() as i32).max(0),
            right: ((area.right * k).ceil() as i32).min(self.width),
            bottom: ((area.bottom * k).ceil() as i32).min(self.height),
        };
        unsafe {
            if LENT.with(|l| l.replace(false)) {
                self.rt.SetDpi(96.0 * k, 96.0 * k);
                self.bound.set(None);
            }
            if self.bound.get() != Some(px) {
                self.rt.BindDC(self.dc, &px)?;
                self.bound.set(Some(px));
            }
            self.rt.BeginDraw();
            self.drawing.set(true);
            // Draw in canvas coordinates; the target starts at the area's corner.
            self.origin.set(translate(-(px.left as f32) / k, -(px.top as f32) / k + self.y_pad.get()));
            self.rt.SetTransform(&self.origin.get());
            self.rt.Clear(Some(&rgba([0.0; 3], 0.0)));
        }
        Ok(())
    }

    fn frame(&mut self, s: &Scene, hits: &mut Hits) -> Result<()> {
        let rt = &self.rt;
        let e = ease(s.open);
        let light = luminance(s.theme.background) > 0.6;
        self.fg.set(if light { [0.08; 3] } else { WHITE });
        self.dim.set(if light { [0.38; 3] } else { GREY });
        let hud_t = s.hud.map(|(_, t)| ease(t)).unwrap_or(0.0);
        // Minimal stays a thin bar even while music plays.
        let idle_minimal = s.minimal;
        let lyric_now = if s.media.playing { s.current_line() } else { None };
        // The clock row hugs its content with the same margin on both sides:
        // [fox or art] clock [bars], the bars only while music plays.
        let icon_slot = s.mascot.is_some() || s.media.playing;
        let bars_on = s.media.playing && s.visualizer;
        let left_in = if icon_slot { CLOCK_ICON_IN } else { CLOCK_PAD };
        let clock_w = self.measure(s.clock, &self.f.clock_small);
        let row_w = left_in + clock_w + CLOCK_PAD;
        let (target, mut collapsed_h) = if idle_minimal {
            (lerp(120.0, 180.0, hud_t), lerp(10.0, COLLAPSED_H, hud_t))
        } else if let Some(line) = lyric_now {
            ((self.measure(line, &self.f.lyric_small) + 104.0).clamp(210.0, 470.0), COLLAPSED_H)
        } else {
            (row_w + if bars_on { CLOCK_BARS } else { 0.0 }, COLLAPSED_H)
        };
        let prev = self.collapsed_w.get();
        let mut cw = if prev == 0.0 || idle_minimal { target } else { prev + (target - prev) * 0.25 };
        if (cw - target).abs() < 0.5 {
            cw = target;
        }
        self.collapsed_w.set(cw);
        self.settling.set(cw != target);
        let mut collapsed_w = cw;
        if let Some((hud, _)) = s.hud {
            collapsed_w = lerp(collapsed_w, hud.width(), hud_t);
            collapsed_h = lerp(collapsed_h, hud.height(), hud_t);
        }
        let w = lerp(collapsed_w, EXPANDED_W, e);
        let h = lerp(collapsed_h, EXPANDED_H, e);
        let x0 = (CANVAS_W - w) / 2.0;
        let radius = lerp(collapsed_h.min(28.0) / 2.0, 26.0, e);
        // Extend past the glued edge so only the far corners are rounded.
        let glued = !matches!(s.edge, Edge::None);
        let side = if glued { s.side } else { 0 };
        let mut body = match s.edge {
            Edge::Top => D2D_RECT_F { left: x0, top: -radius, right: x0 + w, bottom: h },
            Edge::Bottom => D2D_RECT_F { left: x0, top: 0.0, right: x0 + w, bottom: h + radius },
            Edge::None => D2D_RECT_F { left: x0, top: 0.0, right: x0 + w, bottom: h },
        };
        // In a corner the screen's side edge squares off that side too.
        if side < 0 {
            body.left -= radius;
        } else if side > 0 {
            body.right += radius;
        }
        let pad = if s.edge == Edge::Bottom { BOTTOM_PAD } else { 0.0 };
        self.y_pad.set(pad);
        self.body = D2D_RECT_F { left: x0, top: pad, right: x0 + w, bottom: h + pad };
        // Where the notch meets a screen edge it flares out in a short concave
        // curve, so it grows out of the edge instead of butting into it.
        let fillet = if glued { lerp(13.0, 18.0, e) } else { 0.0 };
        // Everything outside the old and new body is already transparent.
        // (With the bottom edge glued, the body reaches `radius` past it.)
        // The canvas rectangle drawn this frame, before the bottom padding.
        let painted = D2D_RECT_F {
            top: if side != 0 && s.edge == Edge::Bottom { -fillet * FILLET_TALL } else { 0.0 },
            bottom: body.bottom.max(h) + if side != 0 && s.edge == Edge::Top { fillet * FILLET_TALL } else { 0.0 },
            left: body.left - fillet,
            right: body.right + fillet,
        };
        let painted = D2D_RECT_F { top: painted.top + pad, bottom: painted.bottom + pad, ..painted };
        let area = match self.drawn_body.replace(Some(painted)) {
            Some(p) => {
                let m = DIRTY_MARGIN;
                D2D_RECT_F {
                    left: (p.left.min(painted.left) - m).max(0.0),
                    top: 0.0,
                    right: (p.right.max(painted.right) + m).min(CANVAS_W),
                    bottom: (p.bottom.max(painted.bottom) + m).min(CANVAS_H),
                }
            }
            None => D2D_RECT_F { left: 0.0, top: 0.0, right: CANVAS_W, bottom: CANVAS_H },
        };
        self.begin(area)?;
        let bg = self.background(s, h)?;
        // The body reaches past the edges it is glued to (so those corners are
        // square); cut it at the screen edge so nothing shows over the taskbar or
        // on a neighbouring monitor.
        let far = 1.0e4;
        let edge_clip = D2D_RECT_F {
            left: if side < 0 { x0 } else { -far },
            top: if s.edge == Edge::Top { 0.0 } else { -far },
            right: if side > 0 { x0 + w } else { far },
            bottom: if s.edge == Edge::Bottom { h } else { far },
        };
        unsafe {
            rt.PushAxisAlignedClip(&edge_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: body, radiusX: radius, radiusY: radius }, &bg);
        }
        let filled = if fillet > 0.0 { self.fill_fillets(x0, x0 + w, h, radius, s.edge == Edge::Bottom, side, fillet, &bg) } else { Ok(()) };
        unsafe { rt.PopAxisAlignedClip() };
        filled?;
        if idle_minimal && hud_t == 0.0 && e < 0.05 {
            return Ok(());
        }

        // Collapsed content fades out during the first half, expanded fades in after.
        let a_small = (1.0 - e * 2.0).clamp(0.0, 1.0);
        let a_big = (e * 2.0 - 1.0).clamp(0.0, 1.0);

        if a_small > 0.0 {
            // The clock (with art and equaliser) cross-fades with the HUD.
            let a_clock = a_small * (1.0 - hud_t * 2.0).clamp(0.0, 1.0);
            let a_hud = a_small * (hud_t * 2.0 - 1.0).clamp(0.0, 1.0);
            if a_clock > 0.0 {
                if let Some(line) = lyric_now {
                    // The line being sung replaces the clock. Note and line are
                    // centred as one group, so they stay in the middle while the
                    // notch grows or shrinks around them.
                    // Fuzzie (or the album art) on the left, the note on the right,
                    // and the line centred between them.
                    if s.mascot.is_some() {
                        self.mascot(s, x0 + 25.0, COLLAPSED_H / 2.0 + 1.0, 18.0, a_clock)?;
                    } else {
                        let art = D2D_RECT_F { left: x0 + 12.0, top: 6.0, right: x0 + 32.0, bottom: 26.0 };
                        self.art(s.media.art.as_deref(), art, 5.0, a_clock)?;
                    }
                    self.note(x0 + w - 25.0, COLLAPSED_H / 2.0, s.theme.accent, a_clock)?;
                    let (lo, hi) = (x0 + 48.0, x0 + w - 48.0);
                    let mid = (lo + hi) / 2.0;
                    let room = (hi - lo).max(10.0);
                    // The new line rises in from below while the one before it
                    // rises out and fades, both kept inside the notch.
                    let u = ease2(s.lyric_t.clamp(0.0, 1.0));
                    let rise = COLLAPSED_H * 0.6;
                    let clip = D2D_RECT_F { left: lo - 2.0, top: 0.0, right: hi + 2.0, bottom: collapsed_h };
                    unsafe { rt.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                    let prev = (s.lyric_index > 0).then(|| s.lyric_lines.get(s.lyric_index as usize - 1)).flatten().map(|l| l.1.as_str()).filter(|l| !l.is_empty());
                    let mut drawn = Ok(());
                    if let (Some(old), true) = (prev, u < 1.0) {
                        let ow = self.measure(old, &self.f.lyric_small).min(room);
                        let dy = -rise * u;
                        let r = D2D_RECT_F { left: mid - ow / 2.0, top: dy, right: mid + ow / 2.0 + 4.0, bottom: COLLAPSED_H + dy };
                        drawn = self.text(old, &self.f.lyric_small, r, self.fg(), a_clock * (1.0 - u) * (1.0 - u));
                    }
                    if drawn.is_ok() {
                        let tw = self.measure(line, &self.f.lyric_small).min(room);
                        let dy = rise * (1.0 - u);
                        let r = D2D_RECT_F { left: mid - tw / 2.0, top: dy, right: mid + tw / 2.0 + 4.0, bottom: COLLAPSED_H + dy };
                        drawn = self.text(line, &self.f.lyric_small, r, self.fg(), a_clock * u.powf(0.7));
                    }
                    unsafe { rt.PopAxisAlignedClip() };
                    drawn?;
                } else {
                    // How far the bars' room has opened (follows the eased width,
                    // so they fade in as the notch grows and out as it shrinks).
                    let bars_t = ((collapsed_w - row_w) / CLOCK_BARS).clamp(0.0, 1.0);
                    let right_in = CLOCK_PAD + CLOCK_BARS * bars_t;
                    let mid = x0 + (left_in + w - right_in) / 2.0;
                    let r = D2D_RECT_F { left: mid - clock_w / 2.0 - 4.0, top: 0.0, right: mid + clock_w / 2.0 + 4.0, bottom: COLLAPSED_H };
                    self.text(s.clock, &self.f.clock_small, r, self.fg(), a_clock)?;
                    if s.mascot.is_some() {
                        self.mascot(s, x0 + 24.0, COLLAPSED_H / 2.0 + 1.0, 18.0, a_clock)?;
                    } else if s.media.playing {
                        let art = D2D_RECT_F { left: x0 + 12.0, top: 6.0, right: x0 + 32.0, bottom: 26.0 };
                        self.art(s.media.art.as_deref(), art, 5.0, a_clock)?;
                    }
                    if bars_t > 0.0 && s.visualizer {
                        // The bars span 14 px left of their x; keep them CLOCK_PAD from the edge.
                        self.equaliser(x0 + w - CLOCK_PAD - 6.0, COLLAPSED_H / 2.0, s.time, s.theme.accent, a_clock * bars_t * bars_t)?;
                    }
                }
            }
            if let Some((hud, _)) = s.hud {
                if a_hud > 0.0 {
                    self.hud(hud, x0, w, collapsed_h, s.theme.accent, a_hud)?;
                }
            }
        }

        if a_big > 0.0 {
            // While the notch grows or shrinks, its content is laid out at full
            // size: keep it inside the body so nothing shows past its edge.
            // It is drawn at full size and scaled down to the body as one piece
            // (anchored at the top middle), so it grows and shrinks with the notch.
            let inside = D2D_RECT_F { left: x0, top: 0.0, right: x0 + w, bottom: h };
            unsafe { rt.PushAxisAlignedClip(&inside, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
            let k = (h / EXPANDED_H).clamp(0.3, 1.0);
            let cx = x0 + w / 2.0;
            let (cw, ch) = (w / k, h / k);
            let cx0 = cx - cw / 2.0;
            let origin = self.origin.get();
            let zoom = Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: cx * (1.0 - k), M32: 0.0 };
            self.origin.set(zoom * origin);
            unsafe { rt.SetTransform(&self.origin.get()) };
            let first = hits.len();
            let drawn: Result<()> = (|| {
                self.header(s, cx0, cw, a_big, hits)?;
                self.pages(s, cx0, cw, ch, a_big, hits)?;
                self.next_page_tab(s, cx0, cw, a_big, hits)?;
                self.page_dots(s, ch, a_big, hits)?;
                if let Some((n, bt)) = s.top_banner {
                    self.top_banner(s, n, cx0, cw, bt, a_big)?;
                }
                Ok(())
            })();
            self.origin.set(origin);
            unsafe {
                rt.SetTransform(&origin);
                rt.PopAxisAlignedClip();
            }
            // Click areas follow the same scaling.
            if k < 1.0 {
                for (r, _) in hits.iter_mut().skip(first) {
                    *r = D2D_RECT_F { left: cx + (r.left - cx) * k, top: r.top * k, right: cx + (r.right - cx) * k, bottom: r.bottom * k };
                }
            }
            drawn?;
        }
        Ok(())
    }

    /// The page area. While switching, the old and new page slide past each other.
    fn pages(&self, s: &Scene, x0: f32, w: f32, h: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let sliding = (s.page_pos - s.page as f32).abs() > 0.001;
        if sliding {
            let clip = D2D_RECT_F { left: x0, top: 0.0, right: x0 + w, bottom: h };
            unsafe { self.rt.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
        }
        let mut scratch = Vec::new();
        let mut result = Ok(());
        for p in 0..PAGE_COUNT {
            let d = p as f32 - s.page_pos;
            if d.abs() >= 1.0 {
                continue;
            }
            let dx = d * w * 0.55;
            let pa = a * (1.0 - d.abs()).clamp(0.0, 1.0);
            scratch.clear();
            unsafe { self.rt.SetTransform(&(translate(dx, 0.0) * self.origin.get())) };
            let r = match p {
                PAGE_SHELF => self.shelf_page(s, x0, w, pa, &mut scratch),
                PAGE_FRIENDS => self.friends_page(s, x0, w, pa, &mut scratch),
                _ => self.music_page(s, x0, w, pa, &mut scratch),
            };
            unsafe { self.rt.SetTransform(&self.origin.get()) };
            if r.is_err() {
                result = r;
                break;
            }
            if p == s.page {
                hits.extend(scratch.drain(..).map(|(r, hit)| (D2D_RECT_F { left: r.left + dx, right: r.right + dx, ..r }, hit)));
            }
        }
        if sliding {
            unsafe { self.rt.PopAxisAlignedClip() };
        }
        result
    }

    /// The concave corners joining the body (`left`..`right`, `h` tall) to the
    /// screen edges it is glued to: one at each free end along the top or bottom
    /// edge, and in a corner placement one running along the side of the screen.
    /// Each is a quarter ellipse `r` wide and `FILLET_TALL` times as tall: running
    /// down the side, it meets the rounded far corner (`radius`) in one S-shaped
    /// sweep instead of a small bump followed by a straight edge.
    #[allow(clippy::too_many_arguments)]
    fn fill_fillets(&self, left: f32, right: f32, h: f32, radius: f32, bottom: bool, side: i8, r: f32, bg: &ID2D1Brush) -> Result<()> {
        use windows::Win32::Graphics::Direct2D::Common::D2D1_BEZIER_SEGMENT;
        let k = 0.5523;
        // The glued edge, the far edge, and the direction from the glued edge into the body.
        let (ey, fy, dv) = if bottom { (h, 0.0, -1.0) } else { (0.0, h, 1.0) };
        // (corner, along the screen edge, along the body side)
        let mut corners: Vec<((f32, f32), (f32, f32), (f32, f32))> = Vec::with_capacity(3);
        if side >= 0 {
            corners.push(((left, ey), (-1.0, 0.0), (0.0, dv)));
        }
        if side <= 0 {
            corners.push(((right, ey), (1.0, 0.0), (0.0, dv)));
        }
        if side > 0 {
            corners.push(((right, fy), (0.0, dv), (-1.0, 0.0)));
        } else if side < 0 {
            corners.push(((left, fy), (0.0, dv), (1.0, 0.0)));
        }
        let v = |x: f32, y: f32| Vector2 { X: x, Y: y };
        unsafe {
            let geo = self.factory.CreatePathGeometry()?;
            let sink = geo.Open()?;
            // Down the notch side it may not reach past the start of the far corner.
            let tall_side = (r * FILLET_TALL).min((h - radius).max(2.0));
            for ((cx, cy), (ax, ay), (bx, by)) in corners {
                // Horizontal legs are `r` long, vertical ones taller.
                let len = |x: f32, along_body: bool| if x != 0.0 { r } else if along_body { tall_side } else { r * FILLET_TALL };
                let (ra, rb) = (len(ax, false), len(bx, true));
                let (pa, pb) = ((cx + ax * ra, cy + ay * ra), (cx + bx * rb, cy + by * rb));
                // Reach a pixel into the body, so no antialiased seam shows between them.
                sink.BeginFigure(v(cx - ax, cy - ay), windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLine(v(pa.0, pa.1));
                sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                    point1: v(pa.0 - ax * ra * k, pa.1 - ay * ra * k),
                    point2: v(pb.0 - bx * rb * k, pb.1 - by * rb * k),
                    point3: v(pb.0, pb.1),
                });
                sink.AddLine(v(pb.0 - ax, pb.1 - ay));
                sink.EndFigure(windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_CLOSED);
            }
            sink.Close()?;
            self.rt.FillGeometry(&geo, bg, None);
        }
        Ok(())
    }

    fn background(&self, s: &Scene, h: f32) -> Result<ID2D1Brush> {
        let t = &s.theme;
        let tint = match t.style {
            BgStyle::Solid => return Ok(self.brush(t.background, t.opacity)?.into()),
            BgStyle::Gradient => t.accent,
            BgStyle::Album => self.album_color(s.media.art.as_deref()).unwrap_or(t.accent),
        };
        let key = [t.background, tint].concat().iter().map(|v| v.to_bits()).chain([t.opacity.to_bits(), t.grad_strength.to_bits(), t.grad_soft.to_bits()]).collect::<Vec<_>>();
        let key: [u32; 9] = key.try_into().unwrap_or([0; 9]);
        // The colour gathers at the free edge: the bottom of a notch at the top
        // of the screen, the top of one sitting on the bottom edge.
        let (start_pt, end) = if s.low {
            (Vector2 { X: 0.0, Y: h.max(1.0) }, Vector2 { X: 0.0, Y: 0.0 })
        } else {
            (Vector2 { X: 0.0, Y: 0.0 }, Vector2 { X: 0.0, Y: h.max(1.0) })
        };
        if let Some((k, b)) = self.bg_grad.borrow().as_ref() {
            if *k == key {
                unsafe {
                    b.SetStartPoint(start_pt);
                    b.SetEndPoint(end);
                }
                return Ok(b.clone().into());
            }
        }
        // Many stops along a smoothstep curve, so the colour eases in with no visible band.
        let start = 0.5 * (1.0 - t.grad_soft);
        let mut stops = vec![D2D1_GRADIENT_STOP { position: 0.0, color: rgba(t.background, t.opacity) }];
        for i in 0..=12 {
            let u = i as f32 / 12.0;
            let e = u * u * (3.0 - 2.0 * u);
            let c = mix(t.background, tint, t.grad_strength * e);
            stops.push(D2D1_GRADIENT_STOP { position: start + (1.0 - start) * u, color: rgba(c, t.opacity) });
        }
        let b = unsafe {
            let coll = self.rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: start_pt, endPoint: end };
            self.rt.CreateLinearGradientBrush(&props, None, &coll)?
        };
        *self.bg_grad.borrow_mut() = Some((key, b.clone()));
        Ok(b.into())
    }

    fn album_color(&self, art: Option<&Image>) -> Option<[f32; 3]> {
        let art = art?;
        if let Some((id, c)) = *self.art_color.borrow() {
            if id == art.id {
                return Some(c);
            }
        }
        let mut acc = [0f32; 3];
        let n = (art.pixels.len() / 4).max(1) as f32;
        for p in art.pixels.chunks_exact(4) {
            acc[0] += p[2] as f32;
            acc[1] += p[1] as f32;
            acc[2] += p[0] as f32;
        }
        // Boost saturation a little so the tint is visible.
        let avg = acc.map(|v| v / n / 255.0);
        let mean = (avg[0] + avg[1] + avg[2]) / 3.0;
        let c = avg.map(|v| (mean + (v - mean) * 1.8).clamp(0.0, 1.0));
        *self.art_color.borrow_mut() = Some((art.id, c));
        Some(c)
    }

    // ---- expanded: header and page dots ----

    fn header(&self, s: &Scene, x0: f32, w: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let date_x = if s.mascot.is_some() { 58.0 } else { 22.0 };
        self.mascot(s, x0 + 30.0, 23.0, 25.0, a)?;
        let date = D2D_RECT_F { left: x0 + date_x, top: 9.0, right: x0 + 260.0, bottom: 35.0 };
        self.text(s.date, &self.f.date, date, self.dim(), a)?;
        let clock = D2D_RECT_F { left: x0 + w - 190.0, top: 9.0, right: x0 + w - 48.0, bottom: 35.0 };
        self.text(s.clock, &self.f.clock, clock, self.fg(), a)?;
        let gear = D2D_RECT_F { left: x0 + w - 42.0, top: 9.0, right: x0 + w - 16.0, bottom: 35.0 };
        self.icon_button("\u{E713}", gear, Hit::Settings, self.fg(), a, s, hits)?;
        Ok(())
    }

    /// Dots for the pages; a pill with the page icon glides to the current one.
    fn page_dots(&self, s: &Scene, h: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let cy = h - 20.0;
        let cx = CANVAS_W / 2.0;
        let pos = s.page_pos.clamp(0.0, (PAGE_COUNT - 1) as f32);
        let px = cx + (pos - 1.0) * 48.0;
        let pill = D2D_RECT_F { left: px - 19.0, top: cy - 11.0, right: px + 19.0, bottom: cy + 11.0 };
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: pill, radiusX: 11.0, radiusY: 11.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(s.theme.accent, 0.28 * a)?);
        }
        for (i, icon) in PAGE_ICONS.iter().enumerate() {
            let x = cx + (i as f32 - 1.0) * 48.0;
            let r = D2D_RECT_F { left: x - 19.0, top: cy - 11.0, right: x + 19.0, bottom: cy + 11.0 };
            // How much the pill covers this slot.
            let near = (1.0 - (i as f32 - pos).abs()).clamp(0.0, 1.0);
            let hot = s.hv(Hit::Page(i));
            if near < 1.0 {
                let d = 3.5 + hot * 0.8;
                let dot = D2D_RECT_F { left: x - d, top: cy - d, right: x + d, bottom: cy + d };
                unsafe { self.rt.FillEllipse(&ellipse(dot), &self.brush(self.fg(), (0.35 + 0.35 * hot) * a * (1.0 - near))?) };
            }
            if near > 0.0 {
                // The icons ride in the pill and cross-fade as it moves.
                self.text(icon, &self.f.icons_small, pill, s.theme.accent, a * near)?;
            }
            hits.push((r, Hit::Page(i)));
        }
        Ok(())
    }

    // ---- music page ----

    /// Two columns (art, title, progress and controls left; synced lyrics right),
    /// easing into one wide layout when lyrics are turned off. The volume is a
    /// speaker at the end of the controls; its slider slides out on hover.
    fn music_page(&self, s: &Scene, x0: f32, w: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let accent = s.theme.accent;
        let fg = self.fg();
        let has_media = s.media.available && !s.media.title.is_empty();
        // 0: two columns (lyrics on the right); 1: lyrics off, big cover on the
        // left with the title, progress and controls beside it.
        let k = ease2(s.wide);
        let l = x0 + 22.0;
        let narrow_r = x0 + 236.0;
        let wide_r = x0 + w - 32.0;
        let col_r = lerp(narrow_r, wide_r, k);

        // Cover: small beside the title, or a big square down the left.
        let side = lerp(52.0, 128.0, k);
        let art = D2D_RECT_F { left: l, top: 46.0, right: l + side, bottom: 46.0 + side };
        self.art(s.media.art.as_deref(), art, lerp(8.0, 12.0, k), a)?;
        let tx = art.right + lerp(14.0, 18.0, k);

        // Lyrics toggle: a round note button that stretches into a "Lyrics" pill.
        let on = if s.lyrics_mode { 1.0 } else { 0.0 } * (1.0 - k);
        let bw = lerp(28.0, 76.0, k);
        let btn = D2D_RECT_F { left: col_r - bw, top: lerp(50.0, 48.0, k), right: col_r, bottom: lerp(78.0, 76.0, k) };
        let hot = s.hv(Hit::Lyrics);
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: btn, radiusX: 14.0, radiusY: 14.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(fg, (0.12 + 0.08 * hot) * a)?);
            if on > 0.0 {
                self.rt.FillRoundedRectangle(&rr, &self.brush(accent, on * (1.0 - 0.18 * hot) * a)?);
            }
        }
        let f = s.fl(Hit::Lyrics);
        if f > 0.005 {
            unsafe {
                let g = 4.0 * (1.0 - f);
                let pr = D2D_RECT_F { left: btn.left - g, top: btn.top - g, right: btn.right + g, bottom: btn.bottom + g };
                self.rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: pr, radiusX: 14.0 + g, radiusY: 14.0 + g }, &self.brush(accent, 0.7 * f * a)?, 1.5, None);
            }
        }
        if k < 1.0 {
            let ic = D2D_RECT_F { left: btn.right - 28.0, right: btn.right, ..btn };
            self.text("\u{E8D6}", &self.f.icons_small, ic, mix(fg, WHITE, on), a * (1.0 - k))?;
        }
        if k > 0.0 {
            self.text("Lyrics", &self.f.pill, btn, fg, a * k)?;
        }
        hits.push((btn, Hit::Lyrics));

        let t1 = D2D_RECT_F { left: tx, top: lerp(52.0, 51.0, k), right: btn.left - 8.0, bottom: lerp(74.0, 73.0, k) };
        let t2 = D2D_RECT_F { left: tx, top: lerp(74.0, 76.0, k), right: col_r, bottom: lerp(94.0, 96.0, k) };
        if has_media {
            self.text(&s.media.title, &self.f.title, t1, fg, a)?;
            self.text(&s.media.artist, &self.f.body, t2, self.dim(), a)?;
        } else {
            self.text("Nothing playing", &self.f.title, t1, fg, a)?;
            self.text("Play something", &self.f.body, t2, self.dim(), a)?;
        }

        // Progress: elapsed, bar, -remaining (under the cover, or beside it).
        let y = lerp(126.0, 120.0, k);
        let pl = lerp(l, tx, k);
        let el = D2D_RECT_F { left: pl, top: y - 9.0, right: pl + 38.0, bottom: y + 9.0 };
        let rem = D2D_RECT_F { left: col_r - 46.0, top: y - 9.0, right: col_r, bottom: y + 9.0 };
        let dur = s.media.duration_ms;
        let pos = s.position_ms.min(dur);
        self.text(&fmt_time(pos), &self.f.small, el, self.dim(), a)?;
        self.text(&format!("-{}", fmt_time(dur - pos)), &self.f.small_right, rem, self.dim(), a)?;
        let bar = D2D_RECT_F { left: pl + 40.0, top: y - 2.0, right: rem.left - 6.0, bottom: y + 2.0 };
        let fr = if dur > 0 { pos as f32 / dur as f32 } else { 0.0 };
        let hot = if dur > 0 { s.hv(Hit::Seek) } else { 0.0 };
        self.bar(bar, fr, fg, 0.2, fg, hot, a)?;
        if dur > 0 {
            hits.push((D2D_RECT_F { top: y - 9.0, bottom: y + 9.0, ..bar }, Hit::Seek));
        }

        // Controls: shuffle, prev, play/pause, next, speaker. Two columns: five
        // even slots under the cover. Lyrics off: shuffle and the speaker at the
        // ends of the text column, the other three grouped in its middle. While
        // the volume slider is out, shuffle fades and the middle three move left.
        let cy = lerp(166.0, 160.0, k);
        let sp = (narrow_r - l - 30.0) / 4.0;
        let narrow = |i: f32| l + 15.0 + i * sp;
        let mid = (tx + wide_r) / 2.0;
        let wide = [tx + 12.0, mid - 50.0, mid, mid + 50.0, wide_r - 14.0];
        let o = if s.volume.is_some() { ease(s.vol_open) } else { 0.0 };
        let shift = lerp(sp, 40.0, k);
        let slot = |i: usize| lerp(narrow(i as f32), wide[i], k) - if i < 4 { shift * o } else { 0.0 };
        let enabled = if s.media.available { 1.0 } else { 0.4 };
        let at = |cx: f32, size: f32| D2D_RECT_F { left: cx - size / 2.0, top: cy - size / 2.0, right: cx + size / 2.0, bottom: cy + size / 2.0 };
        let shuffle_on = s.media.shuffle == Some(true);
        let shuffle_c = if shuffle_on { accent } else { self.dim() };
        let sa = a * enabled * (1.0 - o);
        if sa > 0.005 {
            let x = slot(0);
            let mut scratch = Vec::new();
            let shuffle_hits = if o < 0.5 { &mut *hits } else { &mut scratch };
            self.icon_button("\u{E8B1}", at(x, 30.0), Hit::Shuffle, shuffle_c, sa, s, shuffle_hits)?;
            if shuffle_on {
                let d = D2D_RECT_F { left: x - 2.0, top: cy + 15.0, right: x + 2.0, bottom: cy + 19.0 };
                unsafe { self.rt.FillEllipse(&ellipse(d), &self.brush(accent, sa)?) };
            }
        }
        self.icon_button("\u{E892}", at(slot(1), 32.0), Hit::Prev, fg, a * enabled, s, hits)?;
        let hot = s.hv(Hit::PlayPause);
        let pf = s.fl(Hit::PlayPause);
        let play = at(slot(2), 40.0 + 2.0 * hot - 4.0 * ((pf - 0.7) / 0.3).clamp(0.0, 1.0));
        unsafe {
            self.rt.FillEllipse(&ellipse(play), &self.brush(fg, (1.0 - 0.12 * hot) * a * enabled)?);
            if pf > 0.005 {
                let g = 10.0 * (1.0 - pf);
                let pr = D2D_RECT_F { left: play.left - g, top: play.top - g, right: play.right + g, bottom: play.bottom + g };
                self.rt.DrawEllipse(&ellipse(pr), &self.brush(accent, 0.7 * pf * a)?, 1.5, None);
            }
        }
        let play_icon = if s.media.playing { "\u{E769}" } else { "\u{E768}" };
        self.text(play_icon, &self.f.icons, play, s.theme.background, a)?;
        hits.push((at(slot(2), 40.0), Hit::PlayPause));
        let next_x = slot(3);
        self.icon_button("\u{E893}", at(next_x, 32.0), Hit::Next, fg, a * enabled, s, hits)?;

        // Speaker (click mutes the media app) and its slider.
        let (level, muted) = s.volume.unwrap_or((0.0, false));
        let icon = if muted || level < 0.005 {
            "\u{E74F}"
        } else if level < 0.34 {
            "\u{E993}"
        } else if level < 0.67 {
            "\u{E994}"
        } else {
            "\u{E995}"
        };
        let spk_x = slot(4);
        let spk = at(spk_x, 26.0);
        let va = if s.volume.is_some() { a } else { a * 0.4 };
        let spk_c = if muted { self.dim() } else { fg };
        self.icon_button_with(icon, &self.f.icons_small, spk, Hit::Volume, spk_c, va, s, hits)?;
        if o > 0.01 {
            let vbar = D2D_RECT_F { left: next_x + 24.0, top: cy - 2.0, right: spk.left - 2.0, bottom: cy + 2.0 };
            let fill = if muted { self.dim() } else { accent };
            self.bar(vbar, if muted { 0.0 } else { level }, fg, 0.18, fill, s.hv(Hit::VolumeBar), a * o)?;
            if o > 0.3 {
                hits.push((D2D_RECT_F { top: cy - 12.0, bottom: cy + 12.0, ..vbar }, Hit::VolumeBar));
            }
        }

        // Divider and the lyrics column: they fade and drift out over the first
        // part of widening (in over the last part of narrowing), so nothing overlaps.
        let la = a * (1.0 - ease(k / 0.6));
        if la > 0.005 {
            unsafe {
                let d = D2D_RECT_F { left: x0 + 250.0, top: 50.0, right: x0 + 251.0, bottom: 184.0 };
                self.rt.FillRectangle(&d, &self.brush(fg, 0.1 * la)?);
            }
            let dx = 28.0 * ease(k / 0.6);
            let col = D2D_RECT_F { left: x0 + 266.0 + dx, top: 46.0, right: x0 + w - 32.0 + dx, bottom: 196.0 };
            hits.push((col, Hit::LyricList));
            let mut line_hits = Vec::new();
            self.lyrics_column(s, col, has_media, la, &mut line_hits)?;
            if la > 0.5 {
                hits.extend(line_hits);
            }
        }
        Ok(())
    }

    /// A thin rounded bar filled to `f`; a knob grows in while hovered.
    #[allow(clippy::too_many_arguments)]
    fn bar(&self, bar: D2D_RECT_F, f: f32, track: [f32; 3], track_a: f32, fill: [f32; 3], hot: f32, a: f32) -> Result<()> {
        let y = (bar.top + bar.bottom) / 2.0;
        let grow = hot * 1.0;
        let bar = D2D_RECT_F { top: bar.top - grow / 2.0, bottom: bar.bottom + grow / 2.0, ..bar };
        let done = D2D_RECT_F { right: bar.left + (bar.right - bar.left) * f.clamp(0.0, 1.0), ..bar };
        let r = (bar.bottom - bar.top) / 2.0;
        unsafe {
            let rr = |rect| D2D1_ROUNDED_RECT { rect, radiusX: r, radiusY: r };
            self.rt.FillRoundedRectangle(&rr(bar), &self.brush(track, track_a * a)?);
            if done.right > done.left + 0.5 {
                self.rt.FillRoundedRectangle(&rr(done), &self.brush(fill, a)?);
            }
            if hot > 0.01 {
                let kr = 6.0 * hot;
                let k = D2D_RECT_F { left: done.right - kr, top: y - kr, right: done.right + kr, bottom: y + kr };
                self.rt.FillEllipse(&ellipse(k), &self.brush(WHITE, a * hot)?);
            }
        }
        Ok(())
    }

    /// The synced lyrics: the sung line bright, the rest dim. The wheel scrolls
    /// through the song (`lyric_scroll` lines away from the sung one); clicking
    /// a line plays the song from there.
    fn lyrics_column(&self, s: &Scene, col: D2D_RECT_F, has_media: bool, a: f32, hits: &mut Hits) -> Result<()> {
        let accent = s.theme.accent;
        let note = |msg: &str| self.text(msg, &self.f.body, col, self.dim(), a * 0.8);
        if !has_media {
            return note("Lyrics show up here");
        }
        match s.lyrics_status {
            LyricsStatus::Loading => return note("Looking for lyrics\u{2026}"),
            LyricsStatus::Missing => return note("No synced lyrics for this song"),
            LyricsStatus::Found => {}
        }
        let lines = s.lyric_lines;
        let i = s.lyric_index;
        let slot = 30.0;
        let cy = col.top + 52.0;
        // The list slides up one slot as a new line starts, and follows the wheel.
        let shift = (1.0 - ease(s.lyric_t)) * slot - s.lyric_scroll * slot;
        let fade_px = 26.0;
        unsafe {
            self.rt.PushAxisAlignedClip(&col, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        let off = s.lyric_scroll.floor() as isize;
        let first = (i - 3 + off).max(-1);
        let last = (i + 7 + off).min(lines.len() as isize - 1);
        let mut result = Ok(());
        for k in first..=last {
            let d = (k - i) as f32;
            let y = cy + d * slot + shift;
            let edge = ((y - col.top) / fade_px).min((col.bottom - y) / fade_px).clamp(0.0, 1.0);
            if edge <= 0.0 {
                continue;
            }
            let current = k == i;
            let base = if current {
                1.0
            } else if d < 0.0 {
                0.5
            } else {
                (0.8 - 0.12 * (d - 1.0)).max(0.35)
            };
            let r = D2D_RECT_F { left: col.left, top: y - 14.0, right: col.right, bottom: y + 14.0 };
            // k == -1 is the intro before the first line: shown as a pause.
            let text = if k < 0 { "" } else { lines[k as usize].1.as_str() };
            let hit = (k >= 0).then(|| Hit::LyricLine(k as usize));
            let hot = hit.map_or(0.0, |h| s.hv(h));
            let flash = hit.map_or(0.0, |h| s.fl(h));
            if hot > 0.005 || flash > 0.005 {
                unsafe {
                    let hr = D2D_RECT_F { left: r.left - 6.0, top: r.top + 1.0, right: r.right, bottom: r.bottom - 1.0 };
                    let rr = D2D1_ROUNDED_RECT { rect: hr, radiusX: 7.0, radiusY: 7.0 };
                    self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.07 * hot * a * edge)?);
                    if flash > 0.005 {
                        self.rt.FillRoundedRectangle(&rr, &self.brush(accent, 0.3 * flash * a * edge)?);
                    }
                }
            }
            let al = a * (base + (1.0 - base) * 0.6 * hot) * edge;
            if let Some(h) = hit {
                if edge > 0.5 {
                    hits.push((r, h));
                }
            }
            result = if text.is_empty() {
                let icon = D2D_RECT_F { right: col.left + 18.0, ..r };
                if current && s.media.playing {
                    self.live.set(true);
                    // Three dots that pulse one after another.
                    for j in 0..3 {
                        let p = ((s.time * 3.2 - j as f32 * 0.55).sin() * 0.5 + 0.5) * 0.7 + 0.3;
                        let dx = col.left + 28.0 + j as f32 * 9.0;
                        let dot = D2D_RECT_F { left: dx - 2.5, top: y - 2.5, right: dx + 2.5, bottom: y + 2.5 };
                        unsafe { self.rt.FillEllipse(&ellipse(dot), &self.brush(self.fg(), al * p)?) };
                    }
                    self.text("\u{E8D6}", &self.f.icons_small, icon, accent, al)
                } else {
                    self.text("\u{E8D6}", &self.f.icons_small, icon, if current { accent } else { self.dim() }, al)
                }
            } else {
                let c = if current { self.fg() } else { mix(self.dim(), self.fg(), hot) };
                self.text(text, &self.f.lyric, r, c, al)
            };
            if result.is_err() {
                break;
            }
        }
        unsafe {
            self.rt.PopAxisAlignedClip();
        }
        result
    }

    /// A small tab on the right edge that goes to the next page.
    fn next_page_tab(&self, s: &Scene, x0: f32, w: f32, a: f32, hits: &mut Hits) -> Result<()> {
        // Fades out as the last page slides in.
        let a = a * (PAGE_COUNT as f32 - 1.0 - s.page_pos).clamp(0.0, 1.0);
        if a <= 0.005 {
            return Ok(());
        }
        let hot = s.hv(Hit::NextPage);
        let r = D2D_RECT_F { left: x0 + w - 24.0, top: 96.0, right: x0 + w - 4.0, bottom: 140.0 };
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 9.0, radiusY: 9.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), (0.1 + 0.1 * hot) * a)?);
        }
        let ir = D2D_RECT_F { left: r.left + 1.5 * hot, right: r.right + 1.5 * hot, ..r };
        self.text("\u{E76C}", &self.f.icons_small, ir, self.fg(), a)?;
        if s.page + 1 < PAGE_COUNT {
            hits.push((r, Hit::NextPage));
        }
        Ok(())
    }

    // ---- shelf + clipboard page ----

    fn shelf_page(&self, s: &Scene, x0: f32, w: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let accent = s.theme.accent;
        let mid = x0 + w / 2.0;
        let (lx, lr) = (x0 + 20.0, mid - 8.0);
        let (rx, rr) = (mid + 12.0, x0 + w - 18.0);

        // Shelf title, count and clear.
        let tr = D2D_RECT_F { left: lx + 2.0, top: 42.0, right: lr, bottom: 64.0 };
        self.text("Shelf", &self.f.section, tr, self.fg(), a)?;
        if !s.shelf.is_empty() {
            let cw = self.measure("Shelf", &self.f.section);
            let cr = D2D_RECT_F { left: tr.left + cw + 7.0, ..tr };
            self.text(&s.shelf.len().to_string(), &self.f.body, cr, self.dim(), a)?;
        }
        let trash = D2D_RECT_F { left: lr - 26.0, top: 41.0, right: lr, bottom: 65.0 };
        self.icon_button("\u{E74D}", trash, Hit::ShelfClear, self.dim(), a, s, hits)?;

        let bx = D2D_RECT_F { left: lx, top: 70.0, right: lr, bottom: 190.0 };
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: bx, radiusX: 14.0, radiusY: 14.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.05 * a)?);
            let (c, ba) = if s.drag_over { (accent, 0.9) } else { (self.fg(), 0.1) };
            self.rt.DrawRoundedRectangle(&rr, &self.brush(c, ba * a)?, if s.drag_over { 1.5 } else { 1.0 }, None);
        }
        if s.shelf.is_empty() {
            let ir = D2D_RECT_F { top: bx.top + 26.0, bottom: bx.top + 62.0, ..bx };
            self.text("\u{E896}", &self.f.icons_big, ir, if s.drag_over { accent } else { self.dim() }, a * 0.8)?;
            let tr = D2D_RECT_F { top: ir.bottom + 2.0, bottom: ir.bottom + 22.0, ..bx };
            self.text("Drop files here", &self.f.small_center, tr, self.dim(), a)?;
        } else {
            let cols = 3usize;
            let cw = (bx.right - bx.left - 12.0) / cols as f32;
            let max_cells = 6usize;
            let shown = if s.shelf.len() > max_cells { max_cells - 1 } else { s.shelf.len() };
            for (i, file) in s.shelf.iter().take(shown).enumerate() {
                let cx = bx.left + 6.0 + (i % cols) as f32 * cw;
                let cy = bx.top + 4.0 + (i / cols) as f32 * 58.0;
                let cell = D2D_RECT_F { left: cx, top: cy, right: cx + cw, bottom: cy + 56.0 };
                // The cell stays lit while the pointer is on one of its buttons.
                let hot = (s.hv(Hit::ShelfFile(i)) + s.hv(Hit::ShelfRemove(i)) + s.hv(Hit::ShelfReveal(i))).min(1.0);
                if hot > 0.005 {
                    unsafe {
                        let r = D2D1_ROUNDED_RECT { rect: cell, radiusX: 8.0, radiusY: 8.0 };
                        self.rt.FillRoundedRectangle(&r, &self.brush(self.fg(), 0.08 * hot * a)?);
                    }
                }
                let f = s.fl(Hit::ShelfFile(i));
                if f > 0.005 {
                    unsafe {
                        let r = D2D1_ROUNDED_RECT { rect: cell, radiusX: 8.0, radiusY: 8.0 };
                        self.rt.FillRoundedRectangle(&r, &self.brush(s.theme.accent, 0.3 * f * a)?);
                    }
                }
                let mid_x = (cell.left + cell.right) / 2.0;
                let lift = 2.0 * hot;
                let ir = D2D_RECT_F { left: mid_x - 16.0, top: cy + 4.0 - lift, right: mid_x + 16.0, bottom: cy + 36.0 - lift };
                match &file.icon {
                    Some(img) => self.image(img, ir, 0.0, a)?,
                    None => self.text("\u{E8A5}", &self.f.icons_big, ir, self.fg(), a)?,
                }
                let nr = D2D_RECT_F { left: cell.left + 3.0, top: cy + 37.0, right: cell.right - 3.0, bottom: cy + 53.0 };
                self.text(&file.name, &self.f.small_center, nr, self.fg(), a)?;
                hits.push((cell, Hit::ShelfFile(i)));
                if hot > 0.05 {
                    let reveal = D2D_RECT_F { left: cell.left + 3.0, top: cy + 3.0, right: cell.left + 21.0, bottom: cy + 21.0 };
                    let remove = D2D_RECT_F { left: cell.right - 21.0, top: cy + 3.0, right: cell.right - 3.0, bottom: cy + 21.0 };
                    self.mini_button("\u{E838}", reveal, Hit::ShelfReveal(i), a * hot, s, hits)?;
                    self.mini_button("\u{E711}", remove, Hit::ShelfRemove(i), a * hot, s, hits)?;
                }
            }
            if s.shelf.len() > max_cells {
                let i = max_cells - 1;
                let cx = bx.left + 6.0 + (i % cols) as f32 * cw;
                let cy = bx.top + 4.0 + (i / cols) as f32 * 58.0;
                let cell = D2D_RECT_F { left: cx, top: cy, right: cx + cw, bottom: cy + 56.0 };
                self.text(&format!("+{}", s.shelf.len() - shown), &self.f.section, cell, self.dim(), a)?;
            }
        }

        // Clipboard.
        let ctr = D2D_RECT_F { left: rx, top: 42.0, right: rr, bottom: 64.0 };
        self.text("Clipboard", &self.f.section, ctr, self.fg(), a)?;
        let trash = D2D_RECT_F { left: rr - 26.0, top: 41.0, right: rr, bottom: 65.0 };
        self.icon_button("\u{E74D}", trash, Hit::ClipClear, self.dim(), a, s, hits)?;
        let list = D2D_RECT_F { left: rx - 4.0, top: 68.0, right: rr, bottom: 68.0 + CLIP_ROWS as f32 * 30.0 };
        hits.push((list, Hit::ClipList));
        if s.clips.is_empty() {
            let er = D2D_RECT_F { top: 70.0, bottom: 96.0, ..ctr };
            self.text("Copy something and it shows up here", &self.f.small, er, self.dim(), a)?;
        }
        for (row, (i, clip)) in s.clips.iter().enumerate().skip(s.clip_scroll).take(CLIP_ROWS).enumerate() {
            let y = 70.0 + row as f32 * 30.0;
            let r = D2D_RECT_F { left: rx - 4.0, top: y, right: rr, bottom: y + 28.0 };
            let hot = (s.hv(Hit::Clip(i)) + s.hv(Hit::ClipRemove(i))).min(1.0);
            if hot > 0.005 {
                unsafe {
                    let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 7.0, radiusY: 7.0 };
                    self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.08 * hot * a)?);
                }
            }
            let copied = s.fl(Hit::Clip(i));
            if copied > 0.005 {
                unsafe {
                    let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 7.0, radiusY: 7.0 };
                    self.rt.FillRoundedRectangle(&rr, &self.brush(s.theme.accent, 0.28 * copied.min(1.0) * a)?);
                    self.rt.DrawRoundedRectangle(&rr, &self.brush(s.theme.accent, 0.7 * copied * a)?, 1.2, None);
                }
            }
            // The label: fades from the time to "Copied" and back.
            let tag = (copied * 4.0).min(1.0);
            let thumb = D2D_RECT_F { left: rx, top: y + 4.0, right: rx + 20.0, bottom: y + 24.0 };
            match (&clip.thumb, &clip.data) {
                (Some(img), _) => self.image(img, thumb, 4.0, a)?,
                (None, ClipData::Text(_)) => self.text("\u{E8C8}", &self.f.icons_small, thumb, self.dim(), a)?,
                (None, _) => self.text("\u{EB9F}", &self.f.icons_small, thumb, self.dim(), a)?,
            }
            let lr = D2D_RECT_F { left: rx + 28.0 + 2.0 * hot, top: y, right: rr - 44.0 - 22.0 * tag, bottom: y + 28.0 };
            self.text(&clip.label(), &self.f.body, lr, self.fg(), a)?;
            let tr = D2D_RECT_F { left: rr - 44.0, top: y, right: rr - 4.0, bottom: y + 28.0 };
            // On hover the time gives way to a × that removes the row.
            let x_on = hot * (1.0 - tag);
            if tag < 1.0 {
                self.text(&clip.time, &self.f.small_right, tr, self.dim(), a * (1.0 - tag) * (1.0 - hot))?;
            }
            if tag > 0.0 {
                let cr = D2D_RECT_F { left: rr - 66.0, top: y, right: rr - 4.0, bottom: y + 28.0 };
                self.text("\u{2713} Copied", &self.f.small_right, cr, s.theme.accent, a * tag)?;
            }
            hits.push((r, Hit::Clip(i)));
            if x_on > 0.05 {
                let xr = D2D_RECT_F { left: rr - 26.0, top: y + 4.0, right: rr - 6.0, bottom: y + 24.0 };
                self.icon_button_with("\u{E711}", &self.f.icons_tiny, xr, Hit::ClipRemove(i), self.fg(), a * x_on, s, hits)?;
            }
        }
        // Scroll hint when more rows exist.
        if s.clips.len() > CLIP_ROWS {
            let track = D2D_RECT_F { left: rr + 4.0, top: 72.0, right: rr + 6.0, bottom: 68.0 + CLIP_ROWS as f32 * 30.0 };
            let n = s.clips.len() as f32;
            let th = (track.bottom - track.top) * (CLIP_ROWS as f32 / n);
            let ty = track.top + (track.bottom - track.top - th) * (s.clip_scroll as f32 / (n - CLIP_ROWS as f32));
            unsafe {
                let thumb = D2D_RECT_F { top: ty, bottom: ty + th, ..track };
                self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: thumb, radiusX: 1.0, radiusY: 1.0 }, &self.brush(self.fg(), 0.3 * a)?);
            }
        }
        Ok(())
    }

    // ---- friends page ----

    fn friends_page(&self, s: &Scene, x0: f32, w: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let lx = x0 + 22.0;
        let right = x0 + w - 18.0;
        let tr = D2D_RECT_F { left: lx, top: 42.0, right: lx + 200.0, bottom: 66.0 };
        if s.friend_view == 3 && s.social_on {
            return self.friend_profile_view(s, lx, right, a, hits);
        }
        if s.friend_view != 0 && s.social_on {
            // "‹ Title": the whole thing goes back to the list.
            let title = if s.friend_view == 1 { "Add friend" } else { "Requests" };
            let tw = self.measure(title, &self.f.section);
            let back = D2D_RECT_F { left: lx - 6.0, top: 42.0, right: lx + 26.0 + tw, bottom: 66.0 };
            let hot = s.hv(Hit::FriendsBack);
            if hot > 0.0 {
                unsafe {
                    let rr = D2D1_ROUNDED_RECT { rect: back, radiusX: 10.0, radiusY: 10.0 };
                    self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.08 * hot * a)?);
                }
            }
            self.text("\u{E76B}", &self.f.icons_small, D2D_RECT_F { left: lx - 4.0, right: lx + 14.0, ..tr }, self.fg(), a)?;
            self.text(title, &self.f.section, D2D_RECT_F { left: lx + 20.0, right: lx + 30.0 + tw, ..tr }, self.fg(), a)?;
            hits.push((back, Hit::FriendsBack));
            return if s.friend_view == 1 { self.add_friend_view(s, lx, right, a, hits) } else { self.requests_view(s, lx, right, a, hits) };
        }
        self.text("Friends", &self.f.section, tr, self.fg(), a)?;
        let cw = self.measure("Friends", &self.f.section);
        let so = s.social;
        self.text(&format!("{}/50", so.friends.len()), &self.f.small, D2D_RECT_F { left: lx + cw + 8.0, ..tr }, self.dim(), a)?;

        let req_label = if so.incoming.is_empty() { "Requests".to_string() } else { format!("Requests {}", so.incoming.len()) };
        let add = D2D_RECT_F { left: right - 76.0, top: 42.0, right, bottom: 66.0 };
        let rw = 40.0 + self.measure(&req_label, &self.f.pill);
        let req = D2D_RECT_F { left: add.left - 8.0 - rw, top: 42.0, right: add.left - 8.0, bottom: 66.0 };
        for (r, icon, label, hit) in [(req, "\u{EB52}", req_label.as_str(), Hit::Requests), (add, "\u{E710}", "Add", Hit::AddFriend)] {
            let hot = s.hv(hit);
            unsafe {
                let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 12.0, radiusY: 12.0 };
                self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), (0.1 + 0.07 * hot) * a)?);
            }
            let ir = D2D_RECT_F { left: r.left + 8.0, right: r.left + 26.0, ..r };
            self.text(icon, &self.f.icons_small, ir, self.fg(), a)?;
            let lr = D2D_RECT_F { left: r.left + 22.0, ..r };
            self.text(label, &self.f.pill, lr, self.fg(), a)?;
            hits.push((r, hit));
        }

        if !so.friends.is_empty() && s.social_on {
            return self.friend_rows(s, lx, right, a, hits);
        }
        let card = D2D_RECT_F { left: lx, top: 78.0, right, bottom: 186.0 };
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: card, radiusX: 14.0, radiusY: 14.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.04 * a)?);
        }
        self.mascot(s, (card.left + card.right) / 2.0, card.top + 34.0, 34.0, a)?;
        use crate::social::Conn;
        let code_line;
        let (l1, l2) = if !s.social_on {
            ("Friends are off", "Turn them on in Settings \u{2192} Social")
        } else if so.conn == Conn::Online && !so.code.is_empty() {
            code_line = format!("Your code: {}  \u{00B7}  tap Add for theirs", so.code);
            ("No friends yet", code_line.as_str())
        } else if so.conn == Conn::Retrying {
            ("Can't reach the Fuzzie server", "Trying again in a moment")
        } else {
            ("Connecting\u{2026}", "")
        };
        let t1 = D2D_RECT_F { left: card.left, top: card.top + 56.0, right: card.right, bottom: card.top + 76.0 };
        self.text(l1, &self.f.pill, t1, self.fg(), a)?;
        let t2 = D2D_RECT_F { top: t1.bottom, bottom: t1.bottom + 20.0, ..t1 };
        self.text(l2, &self.f.small_center, t2, self.dim(), a)?;
        Ok(())
    }

    /// A rounded button with a centred label; `on` fills it with the accent.
    #[allow(clippy::too_many_arguments)]
    fn pill(&self, s: &Scene, r: D2D_RECT_F, label: &str, hit: Hit, on: bool, a: f32, hits: &mut Hits) -> Result<()> {
        let hot = s.hv(hit);
        let (c, k) = if on { (s.theme.accent, 0.55 + 0.2 * hot) } else { (self.fg(), 0.1 + 0.07 * hot) };
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 12.0, radiusY: 12.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(c, k * a)?);
        }
        self.text(label, &self.f.pill, r, self.fg(), a)?;
        hits.push((r, hit));
        Ok(())
    }

    /// A button with an icon before its label; `on` fills it with the accent.
    #[allow(clippy::too_many_arguments)]
    fn icon_pill(&self, s: &Scene, r: D2D_RECT_F, icon: &str, label: &str, hit: Hit, on: bool, a: f32, hits: &mut Hits) -> Result<()> {
        let hot = s.hv(hit);
        let (c, k) = if on { (s.theme.accent, 0.55 + 0.2 * hot) } else { (self.fg(), 0.1 + 0.07 * hot) };
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 9.0, radiusY: 9.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(c, k * a)?);
        }
        let lw = self.measure(label, &self.f.pill);
        let start = (r.left + r.right) / 2.0 - (lw + 20.0) / 2.0;
        self.text(icon, &self.f.icons_small, D2D_RECT_F { left: start, right: start + 14.0, ..r }, self.fg(), a)?;
        self.text(label, &self.f.pill, D2D_RECT_F { left: start + 20.0, right: start + 20.0 + lw, ..r }, self.fg(), a)?;
        hits.push((r, hit));
        Ok(())
    }

    /// Our code on the left; a box for a friend's code on the right.
    fn add_friend_view(&self, s: &Scene, lx: f32, right: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let so = s.social;
        let mid = (lx + right) / 2.0;
        // Our code.
        self.text("Your code", &self.f.small, D2D_RECT_F { left: lx, top: 76.0, right: mid - 16.0, bottom: 92.0 }, self.dim(), a)?;
        let code = if so.code.is_empty() { "\u{2026}" } else { so.code.as_str() };
        self.text(code, &self.f.big, D2D_RECT_F { left: lx, top: 92.0, right: mid - 16.0, bottom: 124.0 }, self.fg(), a)?;
        let copy = D2D_RECT_F { left: lx, top: 132.0, right: lx + 86.0, bottom: 158.0 };
        self.icon_pill(s, copy, "\u{E8C8}", "Copy", Hit::CopyCode, false, a, hits)?;
        unsafe {
            let line = D2D_RECT_F { left: mid - 0.5, top: 78.0, right: mid + 0.5, bottom: 178.0 };
            self.rt.FillRectangle(&line, &self.brush(self.fg(), 0.1 * a)?);
        }

        // Their code.
        let cl = mid + 16.0;
        self.text("Friend's code", &self.f.small, D2D_RECT_F { left: cl, top: 76.0, right, bottom: 92.0 }, self.dim(), a)?;
        let field = D2D_RECT_F { left: cl, top: 96.0, right, bottom: 124.0 };
        let hot = s.hv(Hit::CodeBox);
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: field, radiusX: 9.0, radiusY: 9.0 };
            self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), (0.07 + 0.05 * hot) * a)?);
            if s.friend_typing {
                self.rt.DrawRoundedRectangle(&rr, &self.brush(s.theme.accent, 0.9 * a)?, 1.5, None);
            }
        }
        hits.push((field, Hit::CodeBox));
        let tr = D2D_RECT_F { left: field.left + 12.0, top: field.top + 5.0, right: field.right - 10.0, bottom: field.bottom };
        if s.friend_input.is_empty() && !s.friend_typing {
            self.text("FUZ-XXXXXX", &self.f.body, tr, self.dim(), 0.6 * a)?;
        } else {
            let caret = s.friend_typing && (s.time * 2.0) as i64 % 2 == 0;
            let shown = format!("{}{}", s.friend_input, if caret { "|" } else { "" });
            self.text(&shown, &self.f.body, tr, self.fg(), a)?;
        }
        let send = D2D_RECT_F { left: cl, top: 132.0, right: cl + 80.0, bottom: 158.0 };
        self.icon_pill(s, send, "\u{E72A}", "Send", Hit::SendCode, !s.friend_input.is_empty(), a, hits)?;
        let paste = D2D_RECT_F { left: send.right + 6.0, right: send.right + 92.0, ..send };
        self.icon_pill(s, paste, "\u{E77F}", "Paste", Hit::PasteCode, false, a, hits)?;

        let fresh = |t: &std::time::Instant| t.elapsed().as_secs() < 8;
        let note = match (&so.error, s.friend_note) {
            (Some((e, t)), _) if fresh(t) => Some(e.as_str()),
            (_, Some(n)) => Some(n),
            _ => None,
        };
        if let Some(n) = note {
            self.text(n, &self.f.small, D2D_RECT_F { left: lx, top: 164.0, right, bottom: 180.0 }, self.dim(), a)?;
        }
        Ok(())
    }

    /// Requests: the ones waiting for us (Accept / Decline) or the ones we sent (Cancel).
    fn requests_view(&self, s: &Scene, lx: f32, right: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let so = s.social;
        // Received / Sent tabs on the right of the title.
        let sent_label = format!("Sent {}", so.outgoing.len());
        let got_label = format!("Received {}", so.incoming.len());
        let sw = 22.0 + self.measure(&sent_label, &self.f.pill);
        let gw = 22.0 + self.measure(&got_label, &self.f.pill);
        let sent = D2D_RECT_F { left: right - sw, top: 42.0, right, bottom: 66.0 };
        let got = D2D_RECT_F { left: sent.left - 6.0 - gw, right: sent.left - 6.0, ..sent };
        for (r, label, tab) in [(got, got_label.as_str(), 0u8), (sent, sent_label.as_str(), 1u8)] {
            let on = s.friend_tab == tab;
            let hot = s.hv(Hit::ReqTab(tab));
            let k = if on { 0.16 } else { 0.07 * hot };
            unsafe {
                let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 12.0, radiusY: 12.0 };
                self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), k * a)?);
            }
            self.text(label, &self.f.pill, r, if on { self.fg() } else { self.dim() }, a)?;
            hits.push((r, Hit::ReqTab(tab)));
        }

        const ROW: f32 = 36.0;
        let list = if s.friend_tab == 0 { &so.incoming } else { &so.outgoing };
        if list.is_empty() {
            self.mascot(s, lx + 30.0, 122.0, 30.0, a)?;
            let msg = if s.friend_tab == 0 { "No requests right now" } else { "No pending requests" };
            self.text(msg, &self.f.body, D2D_RECT_F { left: lx + 66.0, top: 112.0, right, bottom: 132.0 }, self.dim(), a)?;
            return Ok(());
        }
        for (i, (c, n)) in list.iter().enumerate().take(3) {
            let name = if n.is_empty() { c.as_str() } else { n.as_str() };
            let top = 76.0 + i as f32 * (ROW + 4.0);
            let cy = top + ROW / 2.0;
            let row = D2D_RECT_F { left: lx, top, right, bottom: top + ROW };
            unsafe {
                let rr = D2D1_ROUNDED_RECT { rect: row, radiusX: 12.0, radiusY: 12.0 };
                self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.04 * a)?);
            }
            let mut bx = right - 6.0;
            let mut button = |label: &str, hit: Hit, on: bool| -> Result<()> {
                let bw = 22.0 + self.measure(label, &self.f.pill);
                let r = D2D_RECT_F { left: bx - bw, top: cy - 12.0, right: bx, bottom: cy + 12.0 };
                self.pill(s, r, label, hit, on, a, hits)?;
                bx = r.left - 6.0;
                Ok(())
            };
            let sub = if s.friend_tab == 0 {
                button("Decline", Hit::Decline(i), false)?;
                button("Accept", Hit::Accept(i), true)?;
                format!("{c} \u{00B7} wants to be friends")
            } else {
                button("Cancel", Hit::Cancel(i), false)?;
                format!("{c} \u{00B7} waiting for an answer")
            };
            let tl = lx + 14.0;
            self.text(name, &self.f.body, D2D_RECT_F { left: tl, top: cy - 17.0, right: bx - 4.0, bottom: cy + 1.0 }, self.fg(), a)?;
            self.text(&sub, &self.f.small, D2D_RECT_F { left: tl, top: cy + 1.0, right: bx - 4.0, bottom: cy + 15.0 }, self.dim(), a)?;
        }
        Ok(())
    }

    /// Friend / Fuzzie Bot profile card: large circular avatar (smiles happily on poke/kiss),
    /// status, Poke & Kiss action pills with active combo counter and floating "+1" feedback.
    fn friend_profile_view(&self, s: &Scene, lx: f32, right: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let so = s.social;
        let Some((idx, f)) = so.friends.iter().enumerate().find(|(_, f)| f.code == s.friend_selected) else {
            return self.friend_rows(s, lx, right, a, hits);
        };
        // Back chevron at top-left of profile.
        let back = D2D_RECT_F { left: lx - 8.0, top: 44.0, right: lx + 18.0, bottom: 70.0 };
        let bhot = s.hv(Hit::FriendsBack);
        if bhot > 0.0 {
            unsafe {
                let rr = D2D1_ROUNDED_RECT { rect: back, radiusX: 10.0, radiusY: 10.0 };
                self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.10 * bhot * a)?);
            }
        }
        self.text("\u{E76B}", &self.f.icons_small, back, self.fg(), a)?;
        hits.push((back, Hit::FriendsBack));

        // Large circular avatar badge on the left.
        let (acx, acy, ar) = (lx + 52.0, 96.0, 38.0);
        unsafe {
            let e = D2D1_ELLIPSE { point: Vector2 { X: acx, Y: acy }, radiusX: ar, radiusY: ar };
            self.rt.FillEllipse(&e, &self.brush(self.fg(), 0.06 * a)?);
            if s.profile_happy {
                self.rt.DrawEllipse(&e, &self.brush(s.theme.accent, 0.55 * a)?, 1.5, None);
            }
        }
        let look = if f.is_bot { mascot::Look::bot() } else { mascot::Look::new(0, f.skin, f.acc, 0) };
        let hype = if s.profile_happy { 0.85 } else { 0.0 };
        mascot::draw_still_hype(&self.rt, &self.factory, look, acx, acy + 3.0, 44.0, a, !f.online, 0.0, hype)?;

        // Friend name + online dot + status.
        let tx = acx + ar + 18.0;
        let nw = self.measure(&f.name, &self.f.big);
        self.text(&f.name, &self.f.big, D2D_RECT_F { left: tx, top: 54.0, right: right - 10.0, bottom: 86.0 }, self.fg(), a)?;
        let dot_x = (tx + nw + 12.0).min(right - 70.0);
        let dot_c = if f.online { [0.25, 0.85, 0.45] } else { [0.55, 0.55, 0.62] };
        unsafe {
            let e = D2D1_ELLIPSE { point: Vector2 { X: dot_x, Y: 70.0 }, radiusX: 4.5, radiusY: 4.5 };
            self.rt.FillEllipse(&e, &self.brush(dot_c, a)?);
        }
        let st_txt = if f.is_bot { "Online  \u{00B7}  Bot" } else if f.online { "Online" } else { "Offline" };
        self.text(st_txt, &self.f.small, D2D_RECT_F { left: dot_x + 10.0, top: 61.0, right, bottom: 80.0 }, self.dim(), a)?;

        let sub = if f.is_bot {
            "Always online. Nothing here counts in your stats.".to_string()
        } else if let Some(t) = &f.np {
            if t.artist.is_empty() { format!("\u{266A} {}", t.title) } else { format!("\u{266A} {} \u{2014} {}", t.title, t.artist) }
        } else if f.online {
            "Ready to hang out.".to_string()
        } else {
            "Currently offline.".to_string()
        };
        self.text(&sub, &self.f.small, D2D_RECT_F { left: tx, top: 92.0, right, bottom: 114.0 }, self.dim(), a)?;

        // Action pills along the bottom (Poke & Kiss, plus Listen if a real friend is playing).
        let active_combo = s.combo.filter(|(c, _, _)| *c == f.code);
        let btn_y = 148.0;
        let btn_h = 34.0;
        let mut bx = lx;
        let has_listen = f.online && f.np.is_some();
        let bw = if has_listen { 132.0 } else { 172.0 };
        let gap = 10.0;

        let poke_r = D2D_RECT_F { left: bx, top: btn_y, right: bx + bw, bottom: btn_y + btn_h };
        bx += bw + gap;
        let kiss_r = D2D_RECT_F { left: bx, top: btn_y, right: bx + bw, bottom: btn_y + btn_h };
        bx += bw + gap;

        let draw_action = |r: D2D_RECT_F, icon: &str, label: &str, hit: Hit, accent: [f32; 3], active: bool, hits: &mut Hits| -> Result<()> {
            let hot = s.hv(hit);
            let fl = s.fl(hit);
            let alpha_bg = (0.09 + 0.08 * hot + 0.14 * fl + if active { 0.07 } else { 0.0 }) * a;
            unsafe {
                let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 17.0, radiusY: 17.0 };
                self.rt.FillRoundedRectangle(&rr, &self.brush(if active || fl > 0.05 { accent } else { self.fg() }, alpha_bg)?);
                let border_a = (0.14 + 0.35 * hot + 0.55 * fl + if active { 0.45 } else { 0.0 }).min(0.95) * a;
                self.rt.DrawRoundedRectangle(&rr, &self.brush(if active || fl > 0.05 { accent } else { self.fg() }, border_a)?, 1.3, None);
            }
            let lw = self.measure(label, &self.f.body);
            let start = (r.left + r.right) / 2.0 - (lw + 22.0) / 2.0;
            self.text(icon, &self.f.icons_small, D2D_RECT_F { left: start, right: start + 16.0, ..r }, accent, a)?;
            self.text(label, &self.f.body, D2D_RECT_F { left: start + 20.0, right: start + 24.0 + lw, ..r }, self.fg(), a)?;
            if f.online {
                hits.push((r, hit));
            }
            Ok(())
        };

        let poke_on = active_combo.is_some_and(|(_, is_kiss, _)| !is_kiss);
        let kiss_on = active_combo.is_some_and(|(_, is_kiss, _)| is_kiss);
        let kiss_pink = [1.0, 0.42, 0.62];
        draw_action(poke_r, "\u{E734}", "Poke", Hit::Poke(idx), s.theme.accent, poke_on, hits)?;
        draw_action(kiss_r, "\u{EB52}", "Kiss", Hit::Kiss(idx), kiss_pink, kiss_on, hits)?;

        if has_listen {
            let along = so.along.as_deref() == Some(f.code.as_str());
            let lr = D2D_RECT_F { left: bx, top: btn_y, right: right.min(bx + bw), bottom: btn_y + btn_h };
            self.pill(s, lr, if along { "Listening" } else { "Listen" }, Hit::Along(idx), along, a, hits)?;
        }

        // Active combo counter ("×19") right above the active button corner.
        if let Some((_, is_kiss, count)) = active_combo {
            let r = if is_kiss { kiss_r } else { poke_r };
            let txt = format!("\u{00D7}{count}");
            let cr = D2D_RECT_F { left: r.right - 56.0, top: r.top - 22.0, right: r.right + 12.0, bottom: r.top + 2.0 };
            self.text(&txt, &self.f.section, cr, self.fg(), a)?;
        }

        // Floating "+1" particles rising above the pressed button!
        for &(is_kiss, age, dx) in s.pops {
            let r = if is_kiss { kiss_r } else { poke_r };
            let cx = (r.left + r.right) * 0.5 + dx;
            let py = r.top - 10.0 - age * 38.0;
            let pa = (1.0 - age).powf(0.75) * a;
            let c = if is_kiss { kiss_pink } else { s.theme.accent };
            let pr = D2D_RECT_F { left: cx - 18.0, top: py - 10.0, right: cx + 18.0, bottom: py + 12.0 };
            self.text("+1", &self.f.section, pr, c, pa)?;
        }
        Ok(())
    }

    /// Up to three friends: their Fuzzie, name, what they play, Poke / Kiss / Listen.
    fn friend_rows(&self, s: &Scene, lx: f32, right: f32, a: f32, hits: &mut Hits) -> Result<()> {
        let so = s.social;
        const ROW: f32 = 42.0;
        let shown = so.friends.len().min(3);
        for (i, f) in so.friends.iter().take(shown).enumerate() {
            let top = 74.0 + i as f32 * ROW;
            let cy = top + ROW / 2.0;
            let row = D2D_RECT_F { left: lx, top: top + 2.0, right, bottom: top + ROW - 2.0 };
            let rhot = s.hv(Hit::FriendCard(i));
            unsafe {
                let rr = D2D1_ROUNDED_RECT { rect: row, radiusX: 12.0, radiusY: 12.0 };
                self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), (0.04 + 0.05 * rhot) * a)?);
            }
            hits.push((row, Hit::FriendCard(i)));
            let fa = if f.online { a } else { a * 0.45 };
            let look = if f.is_bot { mascot::Look::bot() } else { mascot::Look::new(0, f.skin, f.acc, 0) };
            mascot::draw_still(&self.rt, &self.factory, look, lx + 22.0, cy + 1.0, 17.0, fa, !f.online, 0.0)?;
            if f.online {
                unsafe {
                    let e = D2D1_ELLIPSE { point: Vector2 { X: lx + 34.0, Y: cy + 10.0 }, radiusX: 3.5, radiusY: 3.5 };
                    self.rt.FillEllipse(&e, &self.brush([0.25, 0.85, 0.45], a)?);
                }
            }
            // Buttons from the right.
            let mut bx = right - 8.0;
            let mut button = |label: &str, hit: Hit, on: bool| -> Result<()> {
                let bw = 22.0 + self.measure(label, &self.f.pill);
                let r = D2D_RECT_F { left: bx - bw, top: cy - 12.0, right: bx, bottom: cy + 12.0 };
                let hot = s.hv(hit);
                let (c, k) = if on { (s.theme.accent, 0.55 + 0.2 * hot) } else { (self.fg(), 0.1 + 0.07 * hot) };
                unsafe {
                    let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: 12.0, radiusY: 12.0 };
                    self.rt.FillRoundedRectangle(&rr, &self.brush(c, k * a)?);
                }
                self.text(label, &self.f.pill, r, self.fg(), a)?;
                hits.push((r, hit));
                bx = r.left - 6.0;
                Ok(())
            };
            if f.online {
                button("Kiss", Hit::Kiss(i), false)?;
                button("Poke", Hit::Poke(i), false)?;
                if f.np.is_some() {
                    let along = so.along.as_deref() == Some(f.code.as_str());
                    button(if along { "Listening" } else { "Listen" }, Hit::Along(i), along)?;
                }
            }
            let text_l = lx + 46.0;
            let sub = match (&f.np, f.online) {
                (Some(t), true) if t.artist.is_empty() => format!("\u{266A} {}", t.title),
                (Some(t), true) => format!("\u{266A} {} \u{2014} {}", t.title, t.artist),
                (None, true) if f.is_bot => "Online \u{00B7} Companion bot".to_string(),
                (None, true) => "Online".to_string(),
                _ => "Offline".to_string(),
            };
            self.text(&f.name, &self.f.body, D2D_RECT_F { left: text_l, top: cy - 18.0, right: bx - 4.0, bottom: cy + 1.0 }, self.fg(), fa)?;
            self.text(&sub, &self.f.small, D2D_RECT_F { left: text_l, top: cy + 1.0, right: bx - 4.0, bottom: cy + 16.0 }, self.dim(), fa)?;
        }
        Ok(())
    }

    // ---- building blocks ----

    #[allow(clippy::too_many_arguments)]
    fn icon_button(&self, icon: &str, r: D2D_RECT_F, hit: Hit, c: [f32; 3], a: f32, s: &Scene, hits: &mut Hits) -> Result<()> {
        self.icon_button_with(icon, &self.f.icons, r, hit, c, a, s, hits)
    }

    /// A small dark round button drawn over a tile (shelf file corners).
    fn mini_button(&self, icon: &str, r: D2D_RECT_F, hit: Hit, a: f32, s: &Scene, hits: &mut Hits) -> Result<()> {
        let hot = s.hv(hit);
        unsafe {
            self.rt.FillEllipse(&ellipse(r), &self.brush([0.0, 0.0, 0.0], (0.55 + 0.25 * hot) * a)?);
            self.rt.DrawEllipse(&ellipse(r), &self.brush([1.0, 1.0, 1.0], (0.18 + 0.3 * hot) * a)?, 1.0, None);
        }
        self.text(icon, &self.f.icons_tiny, r, [1.0, 1.0, 1.0], a)?;
        hits.push((r, hit));
        Ok(())
    }

    /// A round icon button whose hover circle fades in and out.
    #[allow(clippy::too_many_arguments)]
    fn icon_button_with(&self, icon: &str, font: &IDWriteTextFormat, r: D2D_RECT_F, hit: Hit, c: [f32; 3], a: f32, s: &Scene, hits: &mut Hits) -> Result<()> {
        let hot = s.hv(hit);
        if hot > 0.005 {
            let g = 2.0 * (1.0 - hot);
            let hr = D2D_RECT_F { left: r.left + g, top: r.top + g, right: r.right - g, bottom: r.bottom - g };
            unsafe {
                self.rt.FillEllipse(&ellipse(hr), &self.brush(self.fg(), 0.12 * hot * a)?);
            }
        }
        let f = s.fl(hit);
        if f > 0.005 {
            let p = ease_out(1.0 - f);
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            let rad = (r.right - r.left) / 2.0 * (0.7 + 0.6 * p);
            let er = D2D_RECT_F { left: cx - rad, top: cy - rad, right: cx + rad, bottom: cy + rad };
            unsafe {
                self.rt.FillEllipse(&ellipse(er), &self.brush(s.theme.accent, 0.3 * f * a)?);
                self.rt.DrawEllipse(&ellipse(er), &self.brush(s.theme.accent, 0.6 * f * a)?, 1.5, None);
            }
        }
        // The icon dips a little as it is pressed.
        let dip = 1.0 - 0.12 * ((f - 0.7) / 0.3).clamp(0.0, 1.0);
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        let ir = D2D_RECT_F { left: cx - (cx - r.left) * dip, top: cy - (cy - r.top) * dip, right: cx + (r.right - cx) * dip, bottom: cy + (r.bottom - cy) * dip };
        self.text(icon, font, ir, c, a)?;
        hits.push((r, hit));
        Ok(())
    }

    /// Album art, or a music-note tile when there is none.
    fn art(&self, art: Option<&Image>, r: D2D_RECT_F, radius: f32, a: f32) -> Result<()> {
        if let Some(img) = art {
            return self.image(img, r, radius, a);
        }
        unsafe {
            let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
            self.rt.FillRoundedRectangle(&rr, &self.brush(self.fg(), 0.1 * a)?);
        }
        let f = if r.right - r.left > 40.0 { &self.f.icons_big } else { &self.f.icons_small };
        self.text("\u{E8D6}", f, r, [0.7; 3], a)
    }

    /// Draws `img` scaled to cover `r` (centre crop), with rounded corners.
    fn image(&self, img: &Image, r: D2D_RECT_F, radius: f32, a: f32) -> Result<()> {
        let brush = self.brush_for(img)?;
        let (rw, rh) = (r.right - r.left, r.bottom - r.top);
        let k = (rw / img.w as f32).max(rh / img.h as f32);
        let (ox, oy) = (r.left + (rw - img.w as f32 * k) / 2.0, r.top + (rh - img.h as f32 * k) / 2.0);
        unsafe {
            brush.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: ox, M32: oy });
            brush.SetOpacity(a);
            self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, &brush);
        }
        Ok(())
    }

    fn brush_for(&self, img: &Image) -> Result<ID2D1BitmapBrush> {
        let mut cache = self.bitmaps.borrow_mut();
        if let Some(entry) = cache.get_mut(&img.id) {
            entry.2 = true;
            return Ok(entry.1.clone());
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let size = D2D_SIZE_U { width: img.w, height: img.h };
        unsafe {
            let bmp = self.rt.CreateBitmap(size, Some(img.pixels.as_ptr() as _), img.w * 4, &props)?;
            let bprops = D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            };
            let brush = self.rt.CreateBitmapBrush(&bmp, Some(&bprops), None)?;
            cache.insert(img.id, (bmp, brush.clone(), true));
            Ok(brush)
        }
    }

    fn mascot(&self, s: &Scene, cx: f32, cy: f32, size: f32, a: f32) -> Result<()> {
        let Some((look, sleeping)) = s.mascot else { return Ok(()) };
        if a <= 0.0 {
            return Ok(());
        }
        self.live.set(true);
        mascot::draw_music(&self.rt, &self.factory, look, cx, cy, size, a, sleeping, s.time, s.mascot_music)
    }

    fn fg(&self) -> [f32; 3] {
        self.fg.get()
    }

    fn dim(&self) -> [f32; 3] {
        self.dim.get()
    }

    /// A solid beamed pair of notes in colour `c`, about 14 px, centred on (cx, cy).
    fn note(&self, cx: f32, cy: f32, c: [f32; 3], a: f32) -> Result<()> {
        if a <= 0.0 {
            return Ok(());
        }
        let b = self.brush(c, a)?;
        let (l, r) = (cx - 4.5, cx + 5.0);
        let (top_l, top_r, foot) = (cy - 5.5, cy - 7.0, cy + 4.5);
        unsafe {
            // Heads, tilted a little like printed notes.
            let old = {
                let mut m = Matrix3x2::default();
                self.rt.GetTransform(&mut m);
                m
            };
            for (hx, hy) in [(l - 2.3, foot), (r - 2.3, foot - 1.5)] {
                let (sn, cs) = (-20.0f32).to_radians().sin_cos();
                let rot = Matrix3x2 { M11: cs, M12: sn, M21: -sn, M22: cs, M31: hx - cs * hx + sn * hy, M32: hy - sn * hx - cs * hy };
                self.rt.SetTransform(&(rot * old));
                self.rt.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: hx, Y: hy }, radiusX: 3.1, radiusY: 2.3 }, &b);
            }
            self.rt.SetTransform(&old);
            // Stems and the beam joining them.
            self.rt.FillRectangle(&D2D_RECT_F { left: l, top: top_l, right: l + 1.6, bottom: foot }, &b);
            self.rt.FillRectangle(&D2D_RECT_F { left: r, top: top_r, right: r + 1.6, bottom: foot - 1.5 }, &b);
            let beam = self.factory.CreatePathGeometry()?;
            let sink = beam.Open()?;
            sink.BeginFigure(Vector2 { X: l, Y: top_l }, windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(Vector2 { X: r + 1.6, Y: top_r });
            sink.AddLine(Vector2 { X: r + 1.6, Y: top_r + 2.8 });
            sink.AddLine(Vector2 { X: l, Y: top_l + 2.8 });
            sink.EndFigure(windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            self.rt.FillGeometry(&beam, &b, None);
        }
        Ok(())
    }

    /// Three bouncing bars in the accent colour, centred on (cx, cy).
    fn equaliser(&self, cx: f32, cy: f32, t: f32, accent: [f32; 3], a: f32) -> Result<()> {
        self.live.set(true);
        let b = self.brush(accent, a)?;
        for i in 0..3 {
            let phase = i as f32 * 1.7;
            let speed = 7.0 + i as f32 * 2.3;
            let bh = 5.0 + 9.0 * ((t * speed + phase).sin() * 0.5 + 0.5);
            let bx = cx - 7.5 + i as f32 * 5.5;
            let r = D2D_RECT_F { left: bx, top: cy - bh / 2.0, right: bx + 3.0, bottom: cy + bh / 2.0 };
            unsafe {
                self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: 1.5, radiusY: 1.5 }, &b);
            }
        }
        Ok(())
    }

    fn hud(&self, hud: &Hud, x0: f32, w: f32, hh: f32, accent: [f32; 3], a: f32) -> Result<()> {
        let icon_r = D2D_RECT_F { left: x0 + 12.0, top: 0.0, right: x0 + 40.0, bottom: COLLAPSED_H };
        let text_r = D2D_RECT_F { left: x0 + 44.0, top: 0.0, right: x0 + w - 16.0, bottom: COLLAPSED_H };
        match hud {
            Hud::Volume { level, muted } => {
                let icon = if *muted || *level < 0.005 {
                    "\u{E74F}"
                } else if *level < 0.34 {
                    "\u{E993}"
                } else if *level < 0.67 {
                    "\u{E994}"
                } else {
                    "\u{E995}"
                };
                self.text(icon, &self.f.icons_small, icon_r, self.fg(), a)?;
                let pct = D2D_RECT_F { left: x0 + w - 52.0, top: 0.0, right: x0 + w - 14.0, bottom: COLLAPSED_H };
                let shown = if *muted { 0 } else { (level * 100.0).round() as i32 };
                self.text(&shown.to_string(), &self.f.small_right, pct, self.fg(), a)?;
                let y = COLLAPSED_H / 2.0;
                let bar = D2D_RECT_F { left: x0 + 46.0, top: y - 2.5, right: x0 + w - 58.0, bottom: y + 2.5 };
                let f = if *muted { 0.0 } else { level.clamp(0.0, 1.0) };
                let done = D2D_RECT_F { right: bar.left + (bar.right - bar.left) * f, ..bar };
                let c = if *muted { [0.5; 3] } else { accent };
                unsafe {
                    let rr = |r| D2D1_ROUNDED_RECT { rect: r, radiusX: 2.5, radiusY: 2.5 };
                    self.rt.FillRoundedRectangle(&rr(bar), &self.brush(self.fg(), 0.18 * a)?);
                    self.rt.FillRoundedRectangle(&rr(done), &self.brush(c, a)?);
                }
            }
            Hud::CapsLock(on) => {
                self.text("\u{E765}", &self.f.icons_small, icon_r, self.fg(), a)?;
                self.text("Caps Lock", &self.f.body, text_r, self.fg(), a)?;
                let state = D2D_RECT_F { left: x0 + w - 60.0, top: 0.0, right: x0 + w - 14.0, bottom: COLLAPSED_H };
                let (label, c) = if *on { ("On", accent) } else { ("Off", [0.55; 3]) };
                self.text(label, &self.f.small_right, state, c, a)?;
            }
            Hud::Device(name) => {
                self.text("\u{E7F6}", &self.f.icons_small, icon_r, accent, a)?;
                self.text(name, &self.f.body, text_r, self.fg(), a)?;
            }
            Hud::Notice(n) => {
                // A bell in a soft accent circle, then app, title and text.
                let cy = hh / 2.0;
                let (cx, r) = (x0 + 32.0, 17.0);
                unsafe {
                    let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r };
                    self.rt.FillEllipse(&e, &self.brush(accent, 0.22 * a)?);
                }
                if let Some(look) = n.fox {
                    mascot::draw_still_hype(&self.rt, &self.factory, look, cx, cy + 1.0, 22.0, a, false, 0.0, 0.85)?;
                } else {
                    let bell = D2D_RECT_F { left: cx - r, top: cy - r, right: cx + r, bottom: cy + r };
                    self.text("\u{EA8F}", &self.f.icons_small, bell, accent, a)?;
                }
                let badge_w = if n.badge.is_empty() { 0.0 } else { 44.0 };
                let (left, right) = (x0 + 58.0, x0 + w - 20.0 - badge_w);
                let lines: [(&str, &IDWriteTextFormat, [f32; 3], f32); 3] = [(&n.app, &self.f.small, self.dim(), 15.0), (&n.title, &self.f.body, self.fg(), 19.0), (&n.body, &self.f.small, self.dim(), 16.0)];
                let shown: Vec<_> = lines.iter().filter(|l| !l.0.is_empty()).collect();
                let total: f32 = shown.iter().map(|l| l.3).sum();
                let mut y = cy - total / 2.0;
                for (t, f, c, lh) in shown {
                    self.text(t, f, D2D_RECT_F { left, top: y, right, bottom: y + lh }, *c, a)?;
                    y += lh;
                }
                if !n.badge.is_empty() {
                    let br = D2D_RECT_F { left: x0 + w - 62.0, top: cy - 12.0, right: x0 + w - 16.0, bottom: cy + 12.0 };
                    self.text(&n.badge, &self.f.section, br, accent, a)?;
                }
            }
        }
        Ok(())
    }

    /// Compact incoming Poke/Kiss toast pill sliding down at the top of the expanded notch
    /// (visible even while the user is actively poking/kissing below!).
    fn top_banner(&self, s: &Scene, n: &crate::notify::Notice, x0: f32, w: f32, t: f32, a: f32) -> Result<()> {
        let u = ease(t.clamp(0.0, 1.0));
        let ba = u * a;
        if ba <= 0.01 {
            return Ok(());
        }
        let bw = (w - 36.0).min(380.0);
        let bl = x0 + (w - bw) * 0.5;
        let bh = 36.0;
        let bt = lerp(-bh, 5.0, u);
        let rect = D2D_RECT_F { left: bl, top: bt, right: bl + bw, bottom: bt + bh };
        let rr = D2D1_ROUNDED_RECT { rect, radiusX: 18.0, radiusY: 18.0 };
        unsafe {
            self.rt.FillRoundedRectangle(&rr, &self.brush([0.05, 0.04, 0.07], 0.99 * ba)?);
            self.rt.DrawRoundedRectangle(&rr, &self.brush(s.theme.accent, 0.60 * ba)?, 1.2, None);
        }
        let cy = bt + bh * 0.5;
        if let Some(look) = n.fox {
            mascot::draw_still_hype(&self.rt, &self.factory, look, bl + 24.0, cy + 1.0, 20.0, ba, false, 0.0, 0.85)?;
        }
        let badge_w = if n.badge.is_empty() { 0.0 } else { 44.0 };
        self.text(&n.title, &self.f.body, D2D_RECT_F { left: bl + 46.0, top: bt, right: bl + bw - 14.0 - badge_w, bottom: bt + bh }, self.fg(), ba)?;
        if !n.badge.is_empty() {
            let br = D2D_RECT_F { left: bl + bw - 56.0, top: bt, right: bl + bw - 14.0, bottom: bt + bh };
            self.text(&n.badge, &self.f.section, br, s.theme.accent, ba)?;
        }
        Ok(())
    }

    /// Copies the last frame onto the window at screen position `pos`. Only the
    /// part covered by the notch now or in the previous frame is sent.
    /// The next frame clears and sends the whole canvas, so nothing an earlier
    /// frame left behind (outside the last dirty area) can stay on screen.
    pub fn full_refresh(&self) {
        self.drawn_body.set(None);
        self.shown.set(false);
    }

    /// Sets the opacity of the whole notch (the next frame sends all of it).
    pub fn set_ghost(&self, alpha: u8) {
        if self.ghost.replace(alpha) != alpha {
            self.shown.set(false);
        }
    }

    pub fn present(&self, hwnd: HWND, pos: POINT) -> Result<()> {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: self.ghost.get(), AlphaFormat: AC_SRC_ALPHA as u8 };
        let size = SIZE { cx: self.width, cy: self.height };
        let zero = POINT::default();
        // The first frame after start-up sends everything.
        let dirty = if self.shown.replace(true) { self.bound.get() } else { None };
        unsafe {
            if let Some(rc) = dirty {
                let info = UPDATELAYEREDWINDOWINFO {
                    cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
                    hdcDst: HDC::default(),
                    pptDst: &pos,
                    psize: &size,
                    hdcSrc: self.dc,
                    pptSrc: &zero,
                    crKey: COLORREF(0),
                    pblend: &blend,
                    dwFlags: ULW_ALPHA,
                    prcDirty: &rc,
                };
                if UpdateLayeredWindowIndirect(hwnd, &info).as_bool() {
                    return Ok(());
                }
            }
            UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&size), Some(self.dc), Some(&zero), COLORREF(0), Some(&blend), ULW_ALPHA)
        }
    }

    /// The last frame as premultiplied BGRA (for `--snapshot`).
    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize) }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The shared solid brush, set to colour `c` with alpha `a`.
    fn brush(&self, c: [f32; 3], a: f32) -> Result<ID2D1SolidColorBrush> {
        unsafe { self.solid.SetColor(&rgba(c, a)) };
        Ok(self.solid.clone())
    }

    fn text(&self, s: &str, f: &IDWriteTextFormat, r: D2D_RECT_F, c: [f32; 3], a: f32) -> Result<()> {
        if a <= 0.003 || s.is_empty() {
            return Ok(());
        }
        let mut wide = self.wide.borrow_mut();
        wide.clear();
        wide.extend(s.encode_utf16());
        let b = self.brush(c, a)?;
        unsafe {
            self.rt.DrawText(&wide, f, &r, &b, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        }
        Ok(())
    }

    fn measure(&self, s: &str, f: &IDWriteTextFormat) -> f32 {
        let fk = f.as_raw() as usize;
        if let Some(e) = self.measured.borrow().iter().find(|e| e.0 == fk && e.1 == s) {
            return e.2;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let width = unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&wide, f, 1000.0, 100.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_err() {
                return 0.0;
            }
            m.widthIncludingTrailingWhitespace
        };
        let mut cache = self.measured.borrow_mut();
        if cache.len() >= 16 {
            cache.remove(0);
        }
        cache.push((fk, s.to_string(), width));
        width
    }
}

use windows::core::Interface;

impl Drop for Renderer {
    fn drop(&mut self) {
        mascot::release(&self.rt, &self.factory);
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub(crate) fn format(dw: &IDWriteFactory, family: PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT, align: DWRITE_TEXT_ALIGNMENT) -> Result<IDWriteTextFormat> {
    unsafe {
        let f = dw.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us"))?;
        f.SetTextAlignment(align)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(f)
    }
}

pub(crate) fn font_exists(dw: &IDWriteFactory, family: PCWSTR) -> bool {
    unsafe {
        let mut coll = None;
        if dw.GetSystemFontCollection(&mut coll, false).is_err() {
            return false;
        }
        let Some(coll) = coll else { return false };
        let mut index = 0;
        let mut exists = Default::default();
        coll.FindFamilyName(family, &mut index, &mut exists).is_ok() && exists.as_bool()
    }
}

pub(crate) fn rgba(c: [f32; 3], a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a }
}

pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub(crate) fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

pub(crate) fn ellipse(r: D2D_RECT_F) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: Vector2 { X: (r.left + r.right) / 2.0, Y: (r.top + r.bottom) / 2.0 },
        radiusX: (r.right - r.left) / 2.0,
        radiusY: (r.bottom - r.top) / 2.0,
    }
}

fn translate(x: f32, y: f32) -> Matrix3x2 {
    Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: x, M32: y }
}

/// 83000 -> "1:23"
fn fmt_time(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Smooth ease-in-out so the expand animation does not feel linear.
pub(crate) fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smoother ease-in-out (no jolt at either end): for layout changes.
pub(crate) fn ease2(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Moves `v` toward `target` at a steady speed, covering 0..1 in `secs`; the
/// caller eases the result. Returns true while it is still moving.
pub fn glide(v: &mut f32, target: f32, dt: f32, secs: f32) -> bool {
    let step = dt / secs;
    if (target - *v).abs() <= step {
        *v = target;
        return false;
    }
    *v += step * (target - *v).signum();
    true
}

/// Moves `v` toward `target` at `rate` per second (exponential easing, frame-rate
/// independent). Returns true while it is still moving.
pub fn approach(v: &mut f32, target: f32, dt: f32, rate: f32) -> bool {
    *v += (target - *v) * (1.0 - (-rate * dt).exp());
    if (target - *v).abs() < 0.002 {
        *v = target;
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_settles() {
        let mut v = 0.0;
        let mut frames = 0;
        while approach(&mut v, 1.0, 1.0 / 60.0, 14.0) {
            frames += 1;
            assert!(frames < 200);
        }
        assert_eq!(v, 1.0);
        // The same time span gives the same result at any frame rate.
        let (mut a, mut b) = (0.0f32, 0.0f32);
        for _ in 0..6 {
            approach(&mut a, 1.0, 1.0 / 60.0, 10.0);
        }
        for _ in 0..3 {
            approach(&mut b, 1.0, 1.0 / 30.0, 10.0);
        }
        assert!((a - b).abs() < 1e-4);
    }

    #[test]
    fn times() {
        assert_eq!(fmt_time(83_000), "1:23");
        assert_eq!(fmt_time(3_723_000), "1:02:03");
    }
}

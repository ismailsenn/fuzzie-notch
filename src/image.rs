//! Small in-memory images shared between threads and the renderer, plus
//! PNG / DIB / icon conversion helpers.

use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, HICON, ICONINFO};

/// BGRA pixels with premultiplied alpha, ready for Direct2D.
pub struct Image {
    /// Unique per image; the renderer caches GPU bitmaps by it.
    pub id: u64,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

pub fn next_id() -> u64 {
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

impl Image {
    /// From straight-alpha BGRA.
    pub fn from_straight(w: u32, h: u32, mut bgra: Vec<u8>) -> Self {
        premultiply(&mut bgra);
        Self { id: next_id(), w, h, pixels: bgra }
    }

    pub fn from_premultiplied(w: u32, h: u32, bgra: Vec<u8>) -> Self {
        Self { id: next_id(), w, h, pixels: bgra }
    }
}

pub fn premultiply(bgra: &mut [u8]) {
    for p in bgra.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

/// Box-filtered thumbnail that covers a `size`×`size` square (centre crop), straight alpha in and out.
pub fn thumbnail(w: u32, h: u32, bgra: &[u8], size: u32) -> Vec<u8> {
    let side = w.min(h).max(1) as f32;
    let (ox, oy) = ((w as f32 - side) / 2.0, (h as f32 - side) / 2.0);
    let step = side / size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for ty in 0..size {
        for tx in 0..size {
            let (x0, y0) = (ox + tx as f32 * step, oy + ty as f32 * step);
            let (x1, y1) = ((x0 + step).ceil() as u32, (y0 + step).ceil() as u32);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in (y0 as u32)..y1.min(h) {
                for x in (x0 as u32)..x1.min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += bgra[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((ty * size + tx) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

/// Straight BGRA -> PNG bytes.
pub fn encode_png(w: u32, h: u32, bgra: &[u8]) -> Option<Vec<u8>> {
    let mut rgba = bgra.to_vec();
    for p in rgba.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(&rgba).ok()?;
    }
    Some(out)
}

/// PNG bytes -> straight BGRA.
pub fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let channels = info.color_type.samples();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let row = &buf[y * info.line_size..][..w as usize * channels];
        for p in row.chunks_exact(channels) {
            let (r, g, b, a) = match channels {
                4 => (p[0], p[1], p[2], p[3]),
                3 => (p[0], p[1], p[2], 255),
                2 => (p[0], p[0], p[0], p[1]),
                _ => (p[0], p[0], p[0], 255),
            };
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    Some((w, h, out))
}

/// Clipboard CF_DIB bytes (24 or 32 bit) -> straight BGRA.
pub fn decode_dib(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let header = u32_at(0)? as usize;
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bits = u16::from_le_bytes(data.get(14..16)?.try_into().ok()?);
    let compression = u32_at(16)?;
    if width <= 0 || height == 0 || !(bits == 24 || bits == 32) || !(compression == 0 || compression == 3) {
        return None;
    }
    let mut offset = header;
    if compression == 3 && header == 40 {
        offset += 12; // colour masks follow the header
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let bpp = bits as usize / 8;
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let pixels = data.get(offset..offset + stride * h)?;
    let mut out = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        let src_row = if height < 0 { y } else { h - 1 - y };
        let row = &pixels[src_row * stride..];
        for x in 0..w {
            let s = &row[x * bpp..];
            let o = (y * w + x) * 4;
            out[o..o + 3].copy_from_slice(&s[..3]);
            out[o + 3] = if bpp == 4 { s[3] } else { 255 };
            any_alpha |= bpp == 4 && s[3] != 0;
        }
    }
    if bpp == 4 && !any_alpha {
        // Many apps leave the alpha byte at zero; treat the image as opaque.
        for p in out.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    Some((w as u32, h as u32, out))
}

/// Straight BGRA -> CF_DIB bytes (32 bit, bottom-up).
pub fn encode_dib(w: u32, h: u32, bgra: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(40 + bgra.len());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..h as usize).rev() {
        out.extend_from_slice(&bgra[y * w as usize * 4..][..w as usize * 4]);
    }
    out
}

/// Converts an icon handle to an [`Image`]. Does not destroy the icon.
pub fn from_hicon(icon: HICON) -> Option<Image> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(icon, &mut ii).ok()?;
        let read = |bmp: HBITMAP| -> Option<(u32, u32, Vec<u8>)> {
            let mut bm = BITMAP::default();
            if GetObjectW(bmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _)) == 0 {
                return None;
            }
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            let dc = CreateCompatibleDC(None);
            let lines = GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bmi, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            (lines > 0).then_some((w as u32, h as u32, px))
        };
        let color = if ii.hbmColor.is_invalid() { None } else { read(ii.hbmColor) };
        let mask = if ii.hbmMask.is_invalid() { None } else { read(ii.hbmMask) };
        let _ = DeleteObject(ii.hbmColor.into());
        let _ = DeleteObject(ii.hbmMask.into());
        let (w, h, mut px) = color?;
        if !px.chunks_exact(4).any(|p| p[3] != 0) {
            // Old-style icon: take transparency from the mask (white = transparent).
            if let Some((_, _, m)) = mask {
                for (p, mp) in px.chunks_exact_mut(4).zip(m.chunks_exact(4)) {
                    p[3] = if mp[0] > 127 { 0 } else { 255 };
                }
            }
        }
        // Icon bitmaps are already premultiplied.
        Some(Image::from_premultiplied(w, h, px))
    }
}

/// Simple 64-bit FNV-1a hash, used to spot duplicate clipboard images.
pub fn hash(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_dib_round_trip() {
        let px: Vec<u8> = (0..2 * 3 * 4).map(|i| (i * 10) as u8).collect();
        let png = encode_png(2, 3, &px).unwrap();
        assert_eq!(decode_png(&png).unwrap(), (2, 3, px.clone()));
        assert_eq!(decode_dib(&encode_dib(2, 3, &px)).unwrap(), (2, 3, px));
    }

    #[test]
    fn thumb_size() {
        let px = vec![255u8; 10 * 4 * 4];
        assert_eq!(thumbnail(10, 4, &px, 8).len(), 8 * 8 * 4);
    }
}

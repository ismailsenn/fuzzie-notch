//! Shinobi pack: skins and accessories inspired by the ninja-world anime.
//! Everything is our own drawing in the fox's style: hair, eyes, face marks,
//! chakra effects and gear, built from the same head-unit shapes as the rest.

use std::f32::consts::PI;

use windows::core::Result;
use windows_numerics::Matrix3x2;

use super::{build_path, edge, flip, mirror, mix, put, rot, rrect, tr, Eye, Face, Fx, Hair, Layer, Pen, Rgb, Skin, INK, S, TAIL, WHITE};
use S::*;

const RED: Rgb = [0.84, 0.07, 0.12];
const BLACK: Rgb = [0.07, 0.03, 0.05];
const STEEL_HI: Rgb = [0.93, 0.95, 0.98];
const STEEL: Rgb = [0.7, 0.74, 0.8];
const STEEL_LO: Rgb = [0.48, 0.52, 0.6];

/// A unit circle (radius 1 around the origin) as path commands.
const fn circle() -> [S; 6] {
    let k = 0.5523;
    [M(1.0, 0.0), B(1.0, k, k, 1.0, 0.0, 1.0), B(-k, 1.0, -1.0, k, -1.0, 0.0), B(-1.0, -k, -k, -1.0, 0.0, -1.0), B(k, -1.0, 1.0, -k, 1.0, 0.0), Z]
}

/// A tomoe: round head of radius 1 with a tail curling off to the left.
fn tomoe() -> Vec<S> {
    let mut v = circle().to_vec();
    v.extend_from_slice(&[M(0.15, -0.99), Q(-1.6, -1.15, -2.6, 0.55), Q(-1.45, -0.25, -0.95, 0.32), Z]);
    v
}

/// Teardrop flame pointing up, about 1 tall, base at y = 0.5.
const FLAME: &[S] = &[M(0.0, 0.5), Q(-0.55, 0.32, -0.32, -0.15), Q(-0.12, 0.05, 0.0, -1.0), Q(0.12, 0.05, 0.32, -0.15), Q(0.55, 0.32, 0.0, 0.5), Z];

/// A crow in flight, wings spread (1 wide).
const CROW: &[S] = &[M(-0.5, -0.05), Q(-0.3, -0.32, -0.06, -0.06), L(0.0, -0.12), L(0.06, -0.06), Q(0.3, -0.32, 0.5, -0.05), Q(0.26, -0.1, 0.12, 0.04), L(0.06, 0.2), L(0.0, 0.12), L(-0.06, 0.2), L(-0.12, 0.04), Q(-0.26, -0.1, -0.5, -0.05), Z];

/// The plain eye style of one eye (some skins differ between the two).
pub(super) fn eye_for(eye: Eye, side: f32) -> Eye {
    match eye {
        Eye::OneSharingan if side > 0.0 => Eye::Sharingan,
        Eye::OneSharingan => Eye::Normal,
        e => e,
    }
}

/// Draws a patterned eye; `m` maps the unit circle onto the open eye.
pub(super) fn iris(p: &Pen, eye: Eye, m: Matrix3x2, t: f32) -> Result<()> {
    match eye {
        Eye::Sharingan => {
            p.fill_el(m, 0.0, 0.0, 1.0, 1.0, RED, 1.0);
            p.ring(m, 0.0, 0.0, 0.96, 0.96, BLACK, 0.7, 0.1);
            p.ring(m, 0.0, 0.0, 0.58, 0.58, BLACK, 0.85, 0.08);
            p.fill_el(m, 0.0, 0.0, 0.22, 0.22, BLACK, 1.0);
            let tom = p.shape("nr_tomoe", |p| build_path(p.f, &tomoe()))?;
            for k in 0..3 {
                let a = t * 50.0 + k as f32 * 120.0;
                let (s, c) = a.to_radians().sin_cos();
                p.fill(put(0.58 * c, 0.58 * s, 0.16, a - 90.0) * m, &tom, BLACK, 1.0);
            }
        }
        Eye::Mangekyo => {
            p.fill_el(m, 0.0, 0.0, 1.0, 1.0, RED, 1.0);
            p.ring(m, 0.0, 0.0, 0.96, 0.96, BLACK, 0.8, 0.1);
            let blade = p.path("nr_blade", &[M(0.12, -0.26), Q(0.5, -0.66, 0.96, -0.3), Q(0.62, -0.28, 0.32, 0.06), Z])?;
            for k in 0..3 {
                p.fill(rot(t * 25.0 + k as f32 * 120.0, 0.0, 0.0) * m, &blade, BLACK, 1.0);
            }
            p.fill_el(m, 0.0, 0.0, 0.34, 0.34, BLACK, 1.0);
            p.fill_el(m, 0.0, 0.0, 0.11, 0.11, RED, 1.0);
        }
        Eye::Eternal => {
            p.fill_el(m, 0.0, 0.0, 1.0, 1.0, RED, 1.0);
            p.ring(m, 0.0, 0.0, 0.7, 0.7, BLACK, 1.0, 0.1);
            for k in 0..3 {
                let f = rot(t * 20.0 + k as f32 * 120.0, 0.0, 0.0) * m;
                p.fill_el(f, 0.0, -0.42, 0.19, 0.48, BLACK, 1.0);
                p.fill_el(f, 0.0, -0.5, 0.07, 0.22, RED, 0.9);
            }
            p.fill_el(m, 0.0, 0.0, 0.2, 0.2, BLACK, 1.0);
        }
        Eye::Rinnegan => {
            let base = [0.74, 0.68, 0.92];
            p.fill_el(m, 0.0, 0.0, 1.0, 1.0, base, 1.0);
            for r in [0.3f32, 0.55, 0.8] {
                p.ring(m, 0.0, 0.0, r, r, [0.32, 0.24, 0.48], 0.9, 0.07);
            }
            p.fill_el(m, 0.0, 0.0, 0.13, 0.13, [0.2, 0.14, 0.3], 1.0);
        }
        Eye::Beast => {
            p.at(m);
            let g = p.radial(0.0, 0.0, 1.0, 1.0, &[(0.0, [1.0, 0.86, 0.3], 1.0), (0.6, [1.0, 0.5, 0.08], 1.0), (1.0, [0.78, 0.18, 0.05], 1.0)], 1.0)?;
            unsafe { p.rt.FillEllipse(&super::ell(0.0, 0.0, 1.0, 1.0), &g) };
            p.fill_el(m, 0.0, 0.0, 0.17, 0.82, BLACK, 1.0);
        }
        Eye::Normal | Eye::OneSharingan => {}
    }
    Ok(())
}

// ---------------------------------------------------------------- hair

/// Hair seen behind the head and ears (spikes, manes, long hair).
pub(super) fn hair_back(p: &Pen, sk: &Skin, head: Matrix3x2) -> Result<()> {
    let c = sk.hairc;
    let (key, cmds): (&'static str, &[S]) = match sk.hair {
        Hair::None => return Ok(()),
        Hair::Spiky => ("nr_h_spiky", &[M(-0.36, -0.12), L(-0.5, -0.42), L(-0.27, -0.34), L(-0.26, -0.62), L(-0.08, -0.4), L(0.03, -0.68), L(0.14, -0.4), L(0.32, -0.62), L(0.29, -0.34), L(0.52, -0.44), L(0.37, -0.12), Z]),
        Hair::Flash => ("nr_h_flash", &[M(-0.38, -0.1), L(-0.6, -0.36), L(-0.34, -0.36), L(-0.38, -0.62), L(-0.13, -0.44), L(0.0, -0.7), L(0.13, -0.44), L(0.38, -0.62), L(0.34, -0.36), L(0.6, -0.36), L(0.38, -0.1), Z]),
        Hair::Avenger => ("nr_h_avenger", &[M(-0.36, -0.12), L(-0.64, -0.36), L(-0.38, -0.36), L(-0.52, -0.64), L(-0.2, -0.46), L(-0.12, -0.74), L(0.06, -0.48), L(0.26, -0.74), L(0.28, -0.44), L(0.6, -0.58), L(0.4, -0.3), L(0.66, -0.22), L(0.37, -0.1), Z]),
        Hair::Silver => ("nr_h_silver", &[M(-0.36, -0.12), L(-0.76, -0.56), L(-0.42, -0.46), L(-0.58, -0.84), L(-0.22, -0.52), L(-0.14, -0.82), L(0.04, -0.48), L(0.28, -0.64), L(0.26, -0.38), L(0.44, -0.32), L(0.36, -0.1), Z]),
        Hair::Mane => (
            "nr_h_mane",
            &[M(-0.42, 0.36), L(-0.72, 0.3), L(-0.58, 0.12), L(-0.86, 0.0), L(-0.62, -0.14), L(-0.8, -0.38), L(-0.48, -0.38), L(-0.56, -0.66), L(-0.24, -0.5), L(0.0, -0.74), L(0.24, -0.5), L(0.56, -0.66), L(0.48, -0.38), L(0.8, -0.38), L(0.62, -0.14), L(0.86, 0.0), L(0.58, 0.12), L(0.72, 0.3), L(0.42, 0.36), Z],
        ),
        Hair::Curtain => ("nr_h_curtain", &[M(-0.46, -0.24), Q(-0.66, 0.1, -0.62, 0.56), L(-0.44, 0.5), L(0.44, 0.5), L(0.62, 0.56), Q(0.66, 0.1, 0.46, -0.24), Q(0.0, -0.56, -0.46, -0.24), Z]),
        Hair::Short => ("nr_h_short", &[M(-0.32, -0.18), L(-0.4, -0.42), L(-0.22, -0.37), L(-0.16, -0.52), L(-0.03, -0.39), L(0.08, -0.54), L(0.17, -0.38), L(0.32, -0.47), L(0.33, -0.18), Z]),
        Hair::Long => ("nr_h_long", &[M(-0.44, -0.2), Q(-0.6, 0.06, -0.52, 0.34), L(0.52, 0.34), Q(0.6, 0.06, 0.44, -0.2), Q(0.0, -0.52, -0.44, -0.2), Z]),
    };
    let g = p.path(key, cmds)?;
    let shine = mix(c, WHITE, if super::luminance(c) < 0.2 { 0.25 } else { 0.35 });
    p.at(head);
    let b = p.linear(0.0, -0.75, 0.0, 0.4, &[(0.0, shine, 1.0), (0.55, c, 1.0), (1.0, mix(c, INK, 0.25), 1.0)])?;
    p.fill_with(head, &g, &b);
    p.stroke(head, &g, hair_line(c), 1.0, p.lw);
    // Hashirama's and Itachi's long hair falls behind the shoulders too.
    Ok(())
}

fn hair_line(c: Rgb) -> Rgb {
    if super::luminance(c) < 0.2 {
        mix(c, WHITE, 0.45)
    } else {
        mix(c, INK, 0.6)
    }
}

/// Bangs over the forehead, side locks over the cheeks, and the face marks
/// that sit under the eyes. Drawn after the head's outline.
pub(super) fn over_head(p: &Pen, sk: &Skin, head: Matrix3x2) -> Result<()> {
    let c = sk.hairc;
    let line = hair_line(c);
    // Forehead bangs, clipped to the head.
    let bangs: Option<(&'static str, &[S])> = match sk.hair {
        Hair::None => None,
        Hair::Spiky => Some(("nr_b_spiky", &[M(-0.6, -0.45), L(0.6, -0.45), L(0.6, -0.16), L(0.41, -0.07), L(0.33, -0.2), L(0.22, -0.05), L(0.13, -0.2), L(0.03, -0.06), L(-0.06, -0.2), L(-0.16, -0.05), L(-0.25, -0.2), L(-0.36, -0.07), L(-0.43, -0.18), L(-0.6, -0.12), Z])),
        Hair::Flash => Some(("nr_b_flash", &[M(-0.6, -0.45), L(0.6, -0.45), L(0.6, -0.2), L(0.3, -0.1), L(0.18, -0.23), L(0.06, -0.08), L(-0.04, -0.23), L(-0.16, -0.1), L(-0.28, -0.23), L(-0.6, -0.2), Z])),
        Hair::Avenger => Some(("nr_b_avenger", &[M(-0.6, -0.45), L(0.6, -0.45), L(0.6, -0.2), L(0.2, -0.15), L(0.09, -0.04), L(0.0, -0.19), L(-0.1, -0.03), L(-0.2, -0.16), L(-0.6, -0.2), Z])),
        Hair::Silver => Some(("nr_b_silver", &[M(-0.6, -0.45), L(0.6, -0.45), L(0.6, -0.22), L(0.2, -0.18), L(0.07, -0.09), L(-0.04, -0.21), L(-0.18, -0.08), L(-0.3, -0.21), L(-0.6, -0.2), Z])),
        Hair::Mane => Some(("nr_b_mane", &[M(-0.6, -0.45), L(0.6, -0.45), L(0.6, -0.24), L(0.26, -0.16), L(0.12, -0.25), L(-0.02, -0.12), L(-0.14, -0.24), L(-0.6, -0.08), Z])),
        Hair::Curtain => Some(("nr_b_curtain", &[M(0.0, -0.42), Q(-0.04, -0.2, -0.3, -0.13), Q(-0.42, -0.09, -0.5, 0.0), L(-0.6, -0.5), Z, M(0.0, -0.42), Q(0.04, -0.2, 0.3, -0.13), Q(0.42, -0.09, 0.5, 0.0), L(0.6, -0.5), Z])),
        Hair::Short => Some(("nr_b_short", &[M(-0.6, -0.45), L(0.6, -0.45), L(0.6, -0.24), L(0.24, -0.2), L(0.14, -0.28), L(0.04, -0.18), L(-0.06, -0.28), L(-0.18, -0.18), L(-0.28, -0.27), L(-0.6, -0.22), Z])),
        Hair::Long => Some(("nr_b_long", &[M(0.0, -0.4), Q(-0.1, -0.22, -0.3, -0.17), L(-0.6, -0.2), L(-0.6, -0.5), Z, M(0.0, -0.4), Q(0.1, -0.22, 0.3, -0.17), L(0.6, -0.2), L(0.6, -0.5), Z])),
    };
    if let Some((key, cmds)) = bangs {
        let g = p.on_head(key, cmds)?;
        p.at(head);
        let b = p.linear(0.0, -0.35, 0.0, 0.0, &[(0.0, mix(c, WHITE, 0.25), 1.0), (1.0, c, 1.0)])?;
        p.fill_with(head, &g, &b);
        p.stroke(head, &g, line, 1.0, p.lw * 0.85);
        // A couple of light strands.
        if !p.small {
            for x in [-0.22f32, 0.1] {
                p.seg(head, x, -0.27, x + 0.04, -0.16, WHITE, 0.3, 0.014);
            }
        }
    }
    // Long locks framing the face.
    let lock: Option<(&'static str, &[S], bool)> = match sk.hair {
        Hair::Flash => Some(("nr_l_flash", &[M(0.26, -0.27), Q(0.5, -0.16, 0.49, 0.22), Q(0.42, 0.06, 0.36, 0.0), Q(0.33, -0.12, 0.26, -0.27), Z], true)),
        Hair::Avenger => Some(("nr_l_avenger", &[M(0.24, -0.27), Q(0.52, -0.14, 0.5, 0.3), Q(0.43, 0.1, 0.35, 0.02), Q(0.31, -0.13, 0.24, -0.27), Z], true)),
        Hair::Long => Some(("nr_l_long", &[M(0.22, -0.26), Q(0.54, -0.12, 0.52, 0.32), Q(0.44, 0.12, 0.36, 0.04), Q(0.3, -0.14, 0.22, -0.26), Z], true)),
        Hair::Curtain => Some(("nr_l_curtain", &[M(0.36, -0.2), Q(0.58, 0.0, 0.55, 0.38), Q(0.48, 0.16, 0.42, 0.06), Z], true)),
        Hair::Mane => Some(("nr_l_mane", &[M(-0.12, -0.3), Q(-0.44, -0.2, -0.52, 0.32), Q(-0.38, 0.08, -0.3, -0.02), Q(-0.22, -0.15, -0.12, -0.3), Z], false)),
        _ => None,
    };
    if let Some((key, cmds, both)) = lock {
        let g = p.path(key, cmds)?;
        let sides: &[Matrix3x2] = if both { &[Matrix3x2::identity(), flip()] } else { &[Matrix3x2::identity()] };
        for &s in sides {
            p.fill(s * head, &g, c, 1.0);
            p.stroke(s * head, &g, line, 1.0, p.lw * 0.85);
        }
    }

    // Face marks.
    let mc = sk.facec;
    match sk.face {
        Face::Whiskers => {
            for side in [-1.0f32, 1.0] {
                for (y0, y1) in [(0.11, 0.08), (0.165, 0.165), (0.22, 0.25)] {
                    p.seg(head, 0.31 * side, y0, 0.45 * side, y1, mc, 0.9, 0.022);
                }
            }
        }
        Face::Mask => {
            let g = p.on_head("nr_mask", &[M(-0.6, 0.125), Q(0.0, 0.085, 0.6, 0.125), L(0.6, 0.6), L(-0.6, 0.6), Z])?;
            p.at(head);
            let b = p.linear(0.0, 0.1, 0.0, 0.4, &[(0.0, mix(mc, WHITE, 0.12), 1.0), (1.0, mix(mc, INK, 0.2), 1.0)])?;
            p.fill_with(head, &g, &b);
            p.stroke(head, &g, mix(mc, INK, 0.5), 1.0, p.lw * 0.8);
            // The nose under the cloth, and a fold or two.
            p.fill_el(head, 0.0, 0.2, 0.05, 0.03, WHITE, 0.14);
            let fold = p.path("nr_mask_fold", &[M(-0.2, 0.3), Q(0.0, 0.34, 0.2, 0.3), O])?;
            p.stroke(head, &fold, mix(mc, INK, 0.45), 0.7, 0.014);
        }
        Face::RedMarks => {
            let mark = p.path("nr_redmark", &[M(-0.022, 0.0), Q(0.0, -0.01, 0.022, 0.0), L(0.01, 0.075), Q(0.0, 0.085, -0.01, 0.075), Z])?;
            for side in [-1.0f32, 1.0] {
                p.fill(tr(0.2 * side, 0.175) * head, &mark, mc, 0.9);
            }
        }
        Face::TearLines => {
            let l = p.path("nr_tear", &[M(0.11, 0.135), Q(0.125, 0.19, 0.18, 0.215), O])?;
            for s in [Matrix3x2::identity(), flip()] {
                p.stroke(s * head, &l, mc, 0.85, 0.016);
            }
        }
        Face::Beast => {
            // Dark marks sweeping from the eyes up towards the ears.
            let g = p.path("nr_beast", &[M(0.255, 0.07), Q(0.33, 0.0, 0.41, -0.15), Q(0.36, -0.02, 0.25, 0.015), Z])?;
            for s in [Matrix3x2::identity(), flip()] {
                p.fill(s * head, &g, mc, 0.95);
            }
            let brow = p.path("nr_beast_brow", &[M(0.1, -0.05), Q(0.17, -0.09, 0.27, -0.05), O])?;
            for s in [Matrix3x2::identity(), flip()] {
                p.stroke(s * head, &brow, mc, 0.9, 0.024);
            }
        }
        Face::None => {}
    }
    Ok(())
}

/// Marks drawn over the eyes (Kakashi's scar).
pub(super) fn over_eyes(p: &Pen, sk: &Skin, head: Matrix3x2) -> Result<()> {
    if sk.face == Face::Mask {
        let scar = p.path("nr_scar", &[M(0.165, -0.08), Q(0.19, 0.06, 0.2, 0.2), O])?;
        p.stroke(head, &scar, [0.5, 0.26, 0.28], 0.9, 0.016);
    }
    Ok(())
}

/// Nine tails fanned out behind (Kurama). Returns false for other skins.
pub(super) fn tails(p: &Pen, sk: &Skin, fur: &windows::Win32::Graphics::Direct2D::ID2D1Brush, t: f32, sleeping: bool) -> Result<bool> {
    if sk.face != Face::Beast {
        return Ok(false);
    }
    let tail = p.path("tail", TAIL)?;
    let tip = p.clip_el("tail_tip", &tail, 0.60, -0.07, 0.12, 0.13)?;
    let amp = if sleeping { 2.0 } else { 7.0 };
    // Back ones first: the outer tails sit behind the inner ones.
    let fan: [(f32, bool); 9] = [(-62.0, false), (-62.0, true), (-40.0, false), (-40.0, true), (-18.0, false), (-18.0, true), (4.0, false), (4.0, true), (24.0, false)];
    for (i, &(deg, left)) in fan.iter().enumerate() {
        let swing = (t * 1.6 + i as f32 * 0.7).sin() * amp;
        let mut m = rot(deg + swing, 0.22, 0.36);
        if left {
            m = m * flip();
        }
        p.fill_with(m, &tail, fur);
        p.fill(m, &tip, sk.cream, 1.0);
        p.stroke(m, &tail, p.line, 1.0, p.lw);
    }
    Ok(true)
}

// ---------------------------------------------------------------- effects

pub(super) fn fx_back(p: &Pen, sk: &Skin, t: f32) -> Result<()> {
    if sk.fx != Fx::Chakra {
        return Ok(());
    }
    let c = sk.fxc;
    let body = Matrix3x2::identity();
    let pulse = 0.8 + 0.2 * (t * 2.4).sin();
    p.glow(body, 0.0, -0.05, 0.9 * pulse, 0.84 * pulse, c, 0.45)?;
    // Flames licking up around the outline.
    let flame = p.path("nr_flame", FLAME)?;
    let n = if p.small { 6 } else { 11 };
    for i in 0..n {
        let u = i as f32 / (n - 1) as f32;
        let ang = PI * (1.05 + 0.9 * u); // from the lower left, over the top, to the lower right
        let (s, co) = ang.sin_cos();
        let (x, y) = (co * 0.64, -0.05 + s * 0.58);
        let flick = 0.75 + 0.25 * (t * 9.0 + i as f32 * 2.1).sin() + 0.12 * (t * 23.0 + i as f32).sin();
        let lean = co * 25.0 + (t * 3.0 + i as f32).sin() * 8.0;
        let k = 0.2 * flick;
        p.fill(Matrix3x2::scale(k * 0.7, k) * rot(lean, 0.0, 0.0) * tr(x, y), &flame, c, 0.55);
        p.fill(Matrix3x2::scale(k * 0.4, k * 0.6) * rot(lean, 0.0, 0.0) * tr(x, y + 0.03), &flame, mix(c, WHITE, 0.55), 0.6);
    }
    Ok(())
}

/// A cheap repeatable pseudo-random number in 0..1.
fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

pub(super) fn fx_front(p: &Pen, sk: &Skin, t: f32) -> Result<()> {
    let body = Matrix3x2::identity();
    let c = sk.fxc;
    match sk.fx {
        Fx::Lightning => {
            // Crackling bolts that jump to a new spot several times a second.
            let step = (t * 9.0) as u32;
            for b in 0..2u32 {
                let seed = step.wrapping_mul(7) + b * 101;
                if hash(seed + 3) < 0.25 {
                    continue;
                }
                let ang = PI * (0.6 + 1.8 * hash(seed)) ;
                let (s, co) = ang.sin_cos();
                let mut pts = Vec::with_capacity(6);
                for j in 0..6 {
                    let r = 0.5 + j as f32 * 0.07;
                    let jit = (hash(seed + 11 + j) - 0.5) * 0.14;
                    pts.push((co * r - s * jit, 0.05 + s * r * 0.9 + co * jit));
                }
                for w in pts.windows(2) {
                    let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                    p.seg(body, x0, y0, x1, y1, c, 0.55, 0.05);
                    p.seg(body, x0, y0, x1, y1, mix(c, WHITE, 0.7), 1.0, 0.018);
                }
                let (ex, ey) = pts[5];
                p.spark(body, ex, ey, 0.05, WHITE, 0.9)?;
            }
            // A soft charge around the paws.
            p.glow(body, 0.0, 0.42, 0.5, 0.14, c, 0.3 + 0.15 * (t * 17.0).sin().abs())?;
        }
        Fx::Flash => {
            // Yellow flashes: speed streaks and a sudden spark here and there.
            for i in 0..4 {
                let u = (t * 0.9 + i as f32 * 0.27).fract();
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let y = -0.3 + i as f32 * 0.2;
                let x = side * (0.55 + u * 0.35);
                let a = (u * PI).sin();
                p.seg(body, x, y, x - side * 0.22 * a, y, c, a * 0.85, 0.022);
                p.seg(body, x, y, x - side * 0.12 * a, y, WHITE, a, 0.01);
            }
            let step = (t * 1.4) as u32;
            let u = (t * 1.4).fract();
            if u < 0.3 {
                let (x, y) = (if hash(step) < 0.5 { -0.62 } else { 0.64 }, -0.4 + hash(step + 9) * 0.7);
                let r = 0.13 * (u / 0.3 * PI).sin();
                p.glow(body, x, y, r * 1.8, r * 1.8, c, 0.6)?;
                p.spark(body, x, y, r, WHITE, 1.0)?;
            }
        }
        Fx::Crows => {
            let crow = p.path("nr_crow", CROW)?;
            let n = if p.small { 2 } else { 4 };
            for i in 0..n {
                let u = (t * 0.16 + i as f32 * 0.31).fract();
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let x = side * (0.5 + u * 0.32) + (t * 1.3 + i as f32).sin() * 0.03;
                let y = 0.3 - u * 1.1;
                let a = (u * PI).sin();
                let flap = 0.35 + 0.65 * (t * 9.0 + i as f32 * 1.7).sin().abs();
                let k = 0.2 + 0.05 * (i % 2) as f32;
                let m = Matrix3x2::scale(k, k * flap) * rot(side * -12.0, 0.0, 0.0) * tr(x, y);
                p.fill(m, &crow, c, a);
                p.stroke(m, &crow, mix(c, WHITE, 0.45), a * 0.8, 0.04);
                p.fill_el(body, x + side * 0.012, y - 0.02 * flap, 0.008, 0.008, RED, a);
            }
            // A few loose feathers.
            for i in 0..3 {
                let u = (t * 0.2 + i as f32 * 0.37).fract();
                let x = [-0.66, 0.6, -0.42][i] + (t * 2.0 + i as f32).sin() * 0.05;
                let y = -0.6 + u * 1.1;
                let a = (u * PI).sin() * 0.85;
                let m = rot(t * 80.0 + i as f32 * 70.0, 0.0, 0.0) * tr(x, y);
                p.fill_el(m, 0.0, 0.0, 0.05, 0.016, c, a);
                p.seg(m, -0.05, 0.0, 0.05, 0.0, mix(c, WHITE, 0.4), a * 0.7, 0.006);
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------- accessories

/// One cloud of the cloak: three puffs on a flat base, 1 wide.
const CLOUD: &[S] = &[
    M(-0.5, 0.15),
    Q(-0.62, -0.05, -0.38, -0.1),
    Q(-0.32, -0.34, -0.06, -0.24),
    Q(0.08, -0.42, 0.26, -0.22),
    Q(0.5, -0.24, 0.46, -0.02),
    Q(0.62, 0.12, 0.42, 0.18),
    L(-0.36, 0.2),
    Q(-0.48, 0.2, -0.5, 0.15),
    Z,
];

/// A three-pronged kunai pointing up, about 1 long, grip at the bottom.
const KUNAI: &[S] = &[
    M(0.0, -0.62),
    L(0.06, -0.3),
    L(0.2, -0.48),
    L(0.12, -0.2),
    L(0.05, -0.12),
    L(0.04, 0.3),
    L(-0.04, 0.3),
    L(-0.05, -0.12),
    L(-0.12, -0.2),
    L(-0.2, -0.48),
    L(-0.06, -0.3),
    Z,
];

/// A four-point throwing star of radius 1.
fn shuriken() -> Vec<S> {
    let mut v = Vec::new();
    for i in 0..4 {
        let a = i as f32 * PI / 2.0;
        let (s, c) = a.sin_cos();
        let pt = |r: f32, da: f32| {
            let (s2, c2) = (a + da).sin_cos();
            (c2 * r, s2 * r)
        };
        let tip = (c, s);
        let side = pt(0.36, 0.5);
        let back = pt(0.3, -0.35);
        if i == 0 {
            v.push(M(back.0, back.1));
        } else {
            v.push(L(back.0, back.1));
        }
        v.push(Q(tip.0 * 0.5 + back.0 * 0.2, tip.1 * 0.5 + back.1 * 0.2, tip.0, tip.1));
        v.push(L(side.0, side.1));
    }
    v.push(Z);
    // The hole in the middle (wound the other way, so it stays empty).
    let k = 0.5523 * 0.14;
    let r = 0.14;
    v.extend_from_slice(&[M(r, 0.0), B(r, -k, k, -r, 0.0, -r), B(-k, -r, -r, -k, -r, 0.0), B(-r, k, -k, r, 0.0, r), B(k, r, r, k, r, 0.0), Z]);
    v
}

/// Shinobi accessories; `i` counts from the first one in `ACCESSORIES`.
pub(super) fn accessory(p: &Pen, i: usize, layer: Layer, h: Matrix3x2, t: f32) -> Result<()> {
    use Layer::*;
    let lw = p.lw * 0.85;
    match (i, layer) {
        // Leaf headband: blue cloth, a polished plate with an engraved leaf swirl,
        // and the knot's tails fluttering behind.
        (0, Front) => {
            let blue = [0.18, 0.3, 0.62];
            let tail = p.path("nr_band_tail", &[M(0.0, -0.02), B(-0.08, -0.03, -0.16, 0.02, -0.3, 0.0), L(-0.28, 0.065), B(-0.16, 0.075, -0.08, 0.045, 0.0, 0.035), Z])?;
            for (j, base) in [-14.0f32, -36.0].into_iter().enumerate() {
                let deg = base + (t * 6.5 + j as f32 * 1.3).sin() * 10.0;
                p.obj(rot(deg, 0.0, 0.0) * tr(-0.41, -0.19) * h, 1.0, &tail, mix(blue, INK, 0.08 * j as f32));
            }
            let band = p.on_head("headband", super::BAND)?;
            p.obj(h, 1.0, &band, blue);
            p.fill_el(h, -0.41, -0.19, 0.036, 0.032, blue, 1.0);
            let plate = p.path("nr_plate", &rrect(-0.15, -0.305, 0.15, -0.19, 0.022))?;
            p.at(h);
            let steel = p.linear(0.0, -0.305, 0.0, -0.19, &[(0.0, STEEL_HI, 1.0), (0.5, STEEL, 1.0), (1.0, STEEL_LO, 1.0)])?;
            p.fill_with(h, &plate, &steel);
            p.stroke(h, &plate, [0.3, 0.33, 0.4], 1.0, lw);
            // Rivets.
            for (x, y) in [(-0.125, -0.282), (0.125, -0.282), (-0.125, -0.213), (0.125, -0.213)] {
                p.fill_el(h, x, y, 0.008, 0.008, [0.4, 0.43, 0.5], 1.0);
            }
            if !p.small {
                // Engraved leaf: a spiral with a pointed tail.
                let mut v = Vec::new();
                for j in 0..=24 {
                    let u = j as f32 / 24.0;
                    let a = -PI / 2.0 + u * PI * 3.2;
                    let r = 0.042 * (1.0 - 0.8 * u);
                    let (s, c) = a.sin_cos();
                    let (x, y) = (0.008 + c * r, -0.252 + s * r);
                    v.push(if j == 0 { M(x, y) } else { L(x, y) });
                }
                v.push(O);
                let spiral = p.shape("nr_leaf_spiral", |p| build_path(p.f, &v))?;
                p.stroke(h, &spiral, [0.26, 0.28, 0.34], 1.0, 0.011);
                let tip = p.path("nr_leaf_tip", &[M(-0.03, -0.27), L(-0.075, -0.215), L(-0.012, -0.214), O])?;
                p.stroke(h, &tip, [0.26, 0.28, 0.34], 1.0, 0.011);
                p.seg(h, -0.135, -0.296, 0.09, -0.296, WHITE, 0.6, 0.008);
            }
        }
        // Akatsuki cloak: a high black collar with red clouds and a red lining.
        (1, Mid) => {
            let black = [0.1, 0.09, 0.12];
            let collar = p.path("nr_cloak", &[M(-0.56, 0.56), L(-0.5, 0.1), Q(-0.3, 0.3, 0.0, 0.32), Q(0.3, 0.3, 0.5, 0.1), L(0.56, 0.56), Z])?;
            p.at(h);
            let b = p.linear(0.0, 0.1, 0.0, 0.56, &[(0.0, [0.22, 0.2, 0.25], 1.0), (1.0, black, 1.0)])?;
            p.fill_with(h, &collar, &b);
            let lining = p.path("nr_cloak_lining", &[M(-0.5, 0.1), Q(-0.3, 0.3, 0.0, 0.32), Q(0.3, 0.3, 0.5, 0.1), O])?;
            p.stroke(h, &lining, [0.78, 0.1, 0.14], 1.0, 0.03);
            p.stroke(h, &collar, mix(black, WHITE, 0.35), 1.0, lw);
            let cloud = p.path("nr_cloud", CLOUD)?;
            for (x, y, k, deg) in [(-0.32, 0.43, 0.2, -8.0f32), (0.33, 0.41, 0.18, 10.0)] {
                let m = put(x, y, k, deg) * h;
                p.fill(m, &cloud, WHITE, 1.0);
                p.fill(Matrix3x2::scale(0.82, 0.78) * put(x, y + 0.004, k, deg) * h, &cloud, [0.86, 0.1, 0.15], 1.0);
            }
        }
        // Hokage hat: a wide conical hat with a red crown and a flame emblem;
        // white veils hang down behind.
        (2, Back) => {
            let veil = p.path("nr_veil", &[M(0.3, -0.3), Q(0.62, -0.2, 0.66, 0.34), L(0.46, 0.3), Q(0.44, -0.05, 0.3, -0.3), Z])?;
            let sway = (t * 1.5).sin() * 3.0;
            for s in [Matrix3x2::identity(), flip()] {
                let m = rot(sway, 0.3, -0.3) * s * h;
                p.fill(m, &veil, [0.97, 0.97, 0.95], 1.0);
                p.stroke(m, &veil, [0.62, 0.62, 0.66], 1.0, lw);
            }
        }
        (2, Front) => {
            let f = rot(-3.0, 0.0, -0.3) * h;
            let white = [0.98, 0.97, 0.94];
            let red = [0.86, 0.14, 0.18];
            p.obj_el(f, 0.0, -0.3, 0.64, 0.13, white);
            p.ring(f, 0.0, -0.3, 0.6, 0.11, red, 0.9, 0.018);
            let cone = p.path("nr_kage_cone", &[M(-0.5, -0.31), Q(-0.24, -0.5, 0.0, -0.78), Q(0.24, -0.5, 0.5, -0.31), Q(0.0, -0.24, -0.5, -0.31), Z])?;
            p.at(f);
            let shade = p.linear(-0.5, 0.0, 0.5, 0.0, &[(0.0, [0.86, 0.85, 0.84], 1.0), (0.4, white, 1.0), (1.0, [0.82, 0.81, 0.8], 1.0)])?;
            p.fill_with(f, &cone, &shade);
            let crown = p.clip("nr_kage_crown", &cone, |p| build_path(p.f, &[M(-0.6, -0.56), L(0.6, -0.56), L(0.6, -0.9), L(-0.6, -0.9), Z]))?;
            p.fill(f, &crown, red, 1.0);
            p.stroke(f, &cone, [0.55, 0.55, 0.58], 1.0, lw);
            // Straw weave lines.
            for x in [-0.3f32, -0.15, 0.15, 0.3] {
                p.seg(f, x * 0.9, -0.33, x * 0.4, -0.55, [0.7, 0.68, 0.66], 0.5, 0.01);
            }
            // The emblem: a red diamond plate with a white flame.
            let plate = p.path("nr_kage_plate", &[M(0.0, -0.52), L(0.1, -0.42), L(0.0, -0.32), L(-0.1, -0.42), Z])?;
            p.obj(f, 1.0, &plate, red);
            let flame = p.path("nr_flame", FLAME)?;
            let k = 0.075 + 0.005 * (t * 6.0).sin();
            p.fill(Matrix3x2::scale(k * 0.8, k) * tr(0.0, -0.41) * f, &flame, white, 1.0);
        }
        // Rasengan: a swirling ball of chakra held up beside the face.
        (3, Front) => {
            let (x, y, r) = (0.5, 0.18, 0.17);
            let blue = [0.35, 0.72, 1.0];
            p.glow(h, x, y, r * 2.0, r * 2.0, blue, 0.55)?;
            p.at(h);
            let ball = p.radial(x - 0.03, y - 0.03, r, r, &[(0.0, WHITE, 1.0), (0.35, [0.75, 0.92, 1.0], 1.0), (1.0, [0.2, 0.55, 1.0], 0.85)], 1.0)?;
            unsafe { p.rt.FillEllipse(&super::ell(x, y, r, r), &ball) };
            // Swirls spinning fast.
            let arc = p.path("nr_swirl", &[M(0.0, -0.95), Q(0.9, -0.85, 0.85, 0.1), O])?;
            for j in 0..4 {
                let deg = t * 520.0 + j as f32 * 90.0;
                let k = r * (0.55 + 0.4 * ((j % 2) as f32));
                p.stroke(Matrix3x2::scale(k, k) * rot(deg, 0.0, 0.0) * tr(x, y), &arc, WHITE, 0.75, 0.09);
            }
            p.ring(h, x, y, r, r, [0.75, 0.92, 1.0], 0.9, 0.012);
            p.ring(h, x, y, r * 1.18 + 0.01 * (t * 12.0).sin(), r * 1.18, blue, 0.45, 0.01);
            if !p.small {
                for j in 0..3 {
                    let a = t * 6.0 + j as f32 * 2.1;
                    p.spark(h, x + a.cos() * r * 1.3, y + a.sin() * r * 1.3, 0.03, WHITE, 0.8)?;
                }
            }
        }
        // A three-pronged kunai floating beside the head, with a seal tag on its grip.
        (4, Front) => {
            let bob = (t * 2.0).sin() * 0.03;
            let m = put(-0.66, -0.12 + bob, 0.46, -18.0 + (t * 1.3).sin() * 6.0) * h;
            let kunai = p.path("nr_kunai", KUNAI)?;
            p.at(m);
            let steel = p.linear(-0.2, 0.0, 0.2, 0.0, &[(0.0, STEEL_LO, 1.0), (0.5, STEEL_HI, 1.0), (1.0, STEEL, 1.0)])?;
            p.fill_with(m, &kunai, &steel);
            p.stroke(m, &kunai, [0.25, 0.27, 0.33], 1.0, lw / 0.46);
            // Wrapped grip and the ring at the end.
            p.fill(m, &p.path("nr_kunai_grip", &rrect(-0.05, -0.08, 0.05, 0.28, 0.02))?, [0.2, 0.18, 0.24], 1.0);
            for j in 0..5 {
                let y = -0.05 + j as f32 * 0.07;
                p.seg(m, -0.05, y, 0.05, y + 0.04, [0.42, 0.4, 0.46], 1.0, 0.016);
            }
            p.ring(m, 0.0, 0.37, 0.08, 0.08, [0.3, 0.32, 0.38], 1.0, 0.035);
            // The seal tag, fluttering.
            let tag = p.path("nr_tag", &[M(0.05, 0.02), L(0.32, 0.06), L(0.3, 0.2), L(0.05, 0.16), Z])?;
            let tm = rot((t * 5.0).sin() * 8.0, 0.05, 0.09) * m;
            p.obj(tm, 0.46, &tag, [0.96, 0.92, 0.8]);
            p.seg(tm, 0.11, 0.1, 0.27, 0.12, [0.7, 0.16, 0.18], 1.0, 0.02);
            let u = (t * 0.7).fract();
            if u < 0.18 {
                let k = (u / 0.18 * PI).sin();
                p.glow(m, 0.0, -0.62, 0.3 * k, 0.3 * k, [1.0, 0.9, 0.3], 0.7)?;
                p.spark(m, 0.0, -0.62, 0.16 * k, WHITE, 1.0)?;
            }
        }
        // Anbu mask worn on the side of the head: a white cat face with red marks.
        (5, Front) => {
            let k = 0.44;
            let f = put(0.31, -0.25, k, 22.0) * h;
            let white = [0.98, 0.98, 0.97];
            let red = [0.84, 0.12, 0.16];
            let face = p.shape("nr_anbu", |p| build_path(p.f, &super::sym(&[M(0.0, -0.26), B(0.2, -0.27, 0.34, -0.16, 0.36, 0.04), B(0.36, 0.24, 0.16, 0.42, 0.0, 0.44)])))?;
            let ear_r = [M(0.12, -0.2), L(0.32, -0.46), L(0.35, -0.05), Z];
            let ears = p.shape("nr_anbu_ears", |p| build_path(p.f, &[ear_r.to_vec(), mirror(&ear_r)].concat()))?;
            p.obj(f, k, &ears, white);
            p.obj(f, k, &face, white);
            // Eye holes and the red stripes.
            for side in [-1.0f32, 1.0] {
                let eye = rot(-14.0 * side, 0.14 * side, 0.02) * f;
                p.fill_el(eye, 0.14 * side, 0.02, 0.07, 0.035, INK, 1.0);
                p.seg(f, 0.08 * side, -0.14, 0.22 * side, -0.08, red, 1.0, 0.04);
                p.seg(f, 0.2 * side, 0.14, 0.3 * side, 0.2, red, 1.0, 0.035);
                p.fill_el(f, 0.25 * side, -0.33, 0.035, 0.06, red, 0.9);
            }
            let fore = p.path("nr_anbu_fore", &[M(0.0, -0.24), L(0.05, -0.12), L(0.0, -0.06), L(-0.05, -0.12), Z])?;
            p.fill(f, &fore, red, 1.0);
            p.fill_el(f, 0.0, 0.26, 0.04, 0.025, INK, 1.0);
        }
        // Uchiha fan carried on the back: red top, white bottom, wooden handle.
        (6, Back) => {
            let f = put(-0.5, -0.1, 0.6, -28.0) * h;
            p.seg(f, 0.0, 0.05, 0.0, 0.62, [0.45, 0.3, 0.2], 1.0, 0.06);
            p.seg(f, 0.0, 0.05, 0.0, 0.62, [0.7, 0.5, 0.34], 1.0, 0.035);
            p.fill_el(f, 0.0, -0.18, 0.27, 0.3, WHITE, 1.0);
            let disc = p.shape("nr_fan_disc", |p| p.ellipse_geo(0.0, -0.18, 0.27, 0.3))?;
            let top = p.clip("nr_fan_top", &disc, |p| build_path(p.f, &[M(-0.4, -0.6), L(0.4, -0.6), L(0.4, -0.14), Q(0.0, -0.22, -0.4, -0.14), Z]))?;
            p.fill(f, &top, [0.86, 0.1, 0.14], 1.0);
            p.ring(f, 0.0, -0.18, 0.27, 0.3, [0.4, 0.06, 0.1], 1.0, lw);
            p.seg(f, 0.0, 0.12, 0.0, 0.2, [0.4, 0.06, 0.1], 1.0, lw);
        }
        // Susanoo: a ghostly ribcage of chakra around the fox, flickering.
        (7, Back) | (7, Front) => {
            let c = [0.55, 0.45, 1.0];
            let flick = 0.8 + 0.2 * (t * 7.0).sin() * (t * 3.1).cos();
            let body = Matrix3x2::identity();
            if layer == Back {
                p.glow(body, 0.0, 0.0, 1.0, 0.95, c, 0.45 * flick)?;
                // Spine and the backs of the ribs.
                p.seg(body, 0.0, -0.5, 0.0, 0.62, c, 0.5 * flick, 0.06);
                let rib = p.path("nr_rib", &[M(0.0, 0.0), Q(0.6, -0.08, 0.66, 0.22), O])?;
                for j in 0..4 {
                    let y = -0.18 + j as f32 * 0.2;
                    let k = 1.0 - j as f32 * 0.08;
                    for s in [Matrix3x2::identity(), flip()] {
                        p.stroke(Matrix3x2::scale(k, k) * tr(0.0, y) * s, &rib, c, 0.55 * flick, 0.05 / k);
                    }
                }
                // A skull-like helm looming above.
                let helm = p.path("nr_sus_helm", &[M(-0.36, -0.42), Q(-0.4, -0.86, 0.0, -0.9), Q(0.4, -0.86, 0.36, -0.42), Q(0.0, -0.5, -0.36, -0.42), Z])?;
                p.fill(body, &helm, c, 0.22 * flick);
                p.stroke(body, &helm, mix(c, WHITE, 0.4), 0.7 * flick, 0.03);
                for side in [-1.0f32, 1.0] {
                    p.seg(body, 0.24 * side, -0.82, 0.42 * side, -1.0, mix(c, WHITE, 0.3), 0.6 * flick, 0.035);
                }
            } else {
                // The fronts of the lowest ribs pass in front of the paws.
                let front = p.path("nr_rib_front", &[M(-0.62, 0.4), Q(0.0, 0.66, 0.62, 0.4), O])?;
                p.stroke(body, &front, c, 0.45 * flick, 0.05);
                p.stroke(body, &front, mix(c, WHITE, 0.6), 0.5 * flick, 0.016);
            }
        }
        // Sage marks: orange pigment above and below the eyes.
        (8, Front) => {
            let orange = [0.98, 0.5, 0.14];
            let upper = p.path("nr_sage_up", &[M(0.085, -0.02), Q(0.185, -0.13, 0.295, -0.02), Q(0.185, -0.075, 0.085, -0.02), Z])?;
            let lower = p.path("nr_sage_low", &[M(0.1, 0.16), Q(0.185, 0.22, 0.28, 0.16), Q(0.185, 0.19, 0.1, 0.16), Z])?;
            let pulse = 0.85 + 0.15 * (t * 2.0).sin();
            for s in [Matrix3x2::identity(), flip()] {
                p.fill(s * h, &upper, orange, pulse);
                p.fill(s * h, &lower, mix(orange, [0.85, 0.2, 0.1], 0.4), pulse);
            }
            p.glow(h, 0.0, 0.05, 0.42, 0.16, orange, 0.2 * pulse)?;
        }
        // A big ninja scroll strapped across the back.
        (9, Back) => {
            let f = rot(-30.0, 0.0, 0.0) * tr(0.0, -0.1) * h;
            let paper = [0.96, 0.92, 0.82];
            let body = p.path("nr_scroll", &rrect(-0.78, -0.09, 0.78, 0.09, 0.08))?;
            p.at(f);
            let b = p.linear(0.0, -0.09, 0.0, 0.09, &[(0.0, WHITE, 1.0), (0.5, paper, 1.0), (1.0, mix(paper, INK, 0.2), 1.0)])?;
            p.fill_with(f, &body, &b);
            p.stroke(f, &body, edge(paper), 1.0, lw);
            for x in [-0.4f32, 0.4] {
                p.fill(f, &p.path(if x < 0.0 { "nr_scroll_band_l" } else { "nr_scroll_band_r" }, &rrect(x - 0.05, -0.095, x + 0.05, 0.095, 0.01))?, [0.8, 0.16, 0.2], 1.0);
            }
            for x in [-0.78f32, 0.78] {
                p.obj_el(f, x, 0.0, 0.045, 0.1, [0.5, 0.34, 0.22]);
            }
        }
        // Two shuriken orbiting the head (behind it on the far side).
        (10, Back) | (10, Front) => {
            let star = p.shape("nr_shuriken", |p| build_path(p.f, &shuriken()))?;
            for j in 0..2 {
                let ang = t * 1.5 + j as f32 * PI;
                if (ang.sin() > 0.0) != (layer == Front) {
                    continue;
                }
                let (x, y) = (ang.cos() * 0.6, -0.12 + ang.sin() * 0.16);
                let k = 0.1 * (1.0 + 0.15 * ang.sin());
                let m = put(x, y, k, t * 540.0) * h;
                p.at(m);
                let steel = p.linear(-1.0, -1.0, 1.0, 1.0, &[(0.0, STEEL_HI, 1.0), (1.0, STEEL_LO, 1.0)])?;
                p.fill_with(m, &star, &steel);
                p.stroke(m, &star, [0.25, 0.27, 0.32], 1.0, lw / k);
            }
        }
        // A sand gourd on the back with a strap across the chest.
        (11, Back) => {
            let tan = [0.82, 0.62, 0.42];
            let f = rot(18.0, 0.0, 0.0) * tr(0.38, -0.16) * h;
            p.at(f);
            let b = p.radial(-0.08, -0.12, 0.42, 0.42, &[(0.0, mix(tan, WHITE, 0.3), 1.0), (1.0, mix(tan, INK, 0.2), 1.0)], 1.0)?;
            unsafe {
                p.rt.FillEllipse(&super::ell(0.0, 0.05, 0.3, 0.32), &b);
                p.rt.FillEllipse(&super::ell(0.0, -0.36, 0.17, 0.17), &b);
            }
            p.ring(f, 0.0, 0.05, 0.3, 0.32, edge(tan), 1.0, lw);
            p.ring(f, 0.0, -0.36, 0.17, 0.17, edge(tan), 1.0, lw);
            p.fill_el(f, 0.0, -0.22, 0.1, 0.05, tan, 1.0);
            p.obj(f, 1.0, &p.path("nr_gourd_cork", &rrect(-0.06, -0.6, 0.06, -0.5, 0.02))?, [0.9, 0.86, 0.78]);
            // A sash tied round the waist.
            p.obj(f, 1.0, &p.path("nr_gourd_sash", &rrect(-0.31, -0.04, 0.31, 0.04, 0.02))?, [0.72, 0.18, 0.2]);
        }
        _ => {}
    }
    Ok(())
}

//! Five fluffy Bongo-style mascot species drawn in Direct2D head units:
//! Fluffy Fox, Mochi Cat, Puff Bunny, Chibi Shiba and Red Panda.
//!
//! Each species keeps its own head silhouette, ear geometry, torso proportion,
//! tail shape and species face markings while inheriting any chosen [`Skin`]'s
//! fur palette, pattern, eyes, hair and animated effects.

use windows::core::Result;
use windows::Win32::Graphics::Direct2D::{ID2D1Brush, ID2D1Geometry};
use windows_numerics::Matrix3x2;

use super::{build_path, flip, mix, rot, sym, tap_burst, tr, Hair, Pattern, Pen, Skin, INK, S, WHITE};
use crate::render::luminance;
use S::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Species {
    Fox,
    Cat,
    Bunny,
    Shiba,
    RedPanda,
}

pub struct SpeciesInfo {
    pub id: Species,
    pub name: &'static str,
    pub tag: &'static str,
    pub body_w: f32,
    pub arm_x: f32,
    pub eye_x: f32,
    pub eye_y: f32,
    pub eye_rx: f32,
    pub eye_ry: f32,
    pub desk_y: f32,
}

pub const SPECIES: &[SpeciesInfo] = &[
    SpeciesInfo { id: Species::Fox, name: "Fluffy Fox", tag: "Sleek · Bushy tail", body_w: 0.34, arm_x: 0.25, eye_x: 0.165, eye_y: 0.04, eye_rx: 0.068, eye_ry: 0.078, desk_y: 0.45 },
    SpeciesInfo { id: Species::Cat, name: "Mochi Cat", tag: "Squishy · Toe beans", body_w: 0.40, arm_x: 0.29, eye_x: 0.175, eye_y: 0.05, eye_rx: 0.072, eye_ry: 0.080, desk_y: 0.45 },
    SpeciesInfo { id: Species::Bunny, name: "Puff Bunny", tag: "Long ears · Cotton tail", body_w: 0.36, arm_x: 0.24, eye_x: 0.165, eye_y: 0.055, eye_rx: 0.070, eye_ry: 0.082, desk_y: 0.45 },
    SpeciesInfo { id: Species::Shiba, name: "Chibi Shiba", tag: "Plump cheeks · Curly tail", body_w: 0.41, arm_x: 0.29, eye_x: 0.180, eye_y: 0.04, eye_rx: 0.066, eye_ry: 0.074, desk_y: 0.45 },
    SpeciesInfo { id: Species::RedPanda, name: "Red Panda", tag: "Ringed tail · Teddy ears", body_w: 0.39, arm_x: 0.28, eye_x: 0.175, eye_y: 0.04, eye_rx: 0.068, eye_ry: 0.076, desk_y: 0.45 },
];

pub fn get(i: usize) -> &'static SpeciesInfo {
    &SPECIES[i % SPECIES.len()]
}

// ---------------------------------------------------------------- head paths

const HEAD_FOX: &[S] = &[
    M(0.0, -0.33),
    Q(0.05, -0.38, 0.09, -0.32),
    B(0.24, -0.31, 0.37, -0.19, 0.40, -0.03),
    Q(0.51, 0.03, 0.46, 0.11),
    Q(0.53, 0.16, 0.41, 0.22),
    B(0.29, 0.31, 0.13, 0.34, 0.0, 0.34),
];

const HEAD_CAT: &[S] = &[
    M(0.0, -0.32),
    B(0.27, -0.32, 0.43, -0.20, 0.46, -0.02),
    Q(0.52, 0.06, 0.48, 0.15),
    B(0.45, 0.28, 0.28, 0.35, 0.0, 0.35),
];

const HEAD_BUNNY: &[S] = &[
    M(0.0, -0.32),
    Q(0.04, -0.36, 0.08, -0.31),
    B(0.24, -0.30, 0.37, -0.17, 0.41, 0.01),
    B(0.47, 0.14, 0.44, 0.28, 0.31, 0.33),
    B(0.18, 0.36, 0.08, 0.36, 0.0, 0.36),
];

const HEAD_SHIBA: &[S] = &[
    M(0.0, -0.32),
    B(0.26, -0.32, 0.42, -0.19, 0.46, -0.01),
    B(0.54, 0.10, 0.51, 0.27, 0.36, 0.33),
    B(0.21, 0.37, 0.09, 0.37, 0.0, 0.37),
];

const HEAD_REDPANDA: &[S] = &[
    M(0.0, -0.32),
    Q(0.04, -0.36, 0.08, -0.31),
    B(0.27, -0.31, 0.42, -0.18, 0.45, 0.0),
    Q(0.52, 0.07, 0.48, 0.15),
    Q(0.51, 0.22, 0.39, 0.28),
    B(0.26, 0.35, 0.11, 0.35, 0.0, 0.35),
];

pub(super) fn head_geo(p: &Pen, sp: Species) -> Result<ID2D1Geometry> {
    let (key, half): (&'static str, &[S]) = match sp {
        Species::Fox => ("sp_head_fox", HEAD_FOX),
        Species::Cat => ("sp_head_cat", HEAD_CAT),
        Species::Bunny => ("sp_head_bunny", HEAD_BUNNY),
        Species::Shiba => ("sp_head_shiba", HEAD_SHIBA),
        Species::RedPanda => ("sp_head_rp", HEAD_REDPANDA),
    };
    p.shape(key, |p| build_path(p.f, &sym(half)))
}

// ---------------------------------------------------------------- tails

const TAIL_FOX: &[S] = &[
    M(-0.02, 0.04),
    B(0.26, 0.12, 0.56, 0.02, 0.62, -0.22),
    B(0.65, -0.34, 0.57, -0.46, 0.44, -0.45),
    B(0.36, -0.44, 0.33, -0.32, 0.28, -0.22),
    B(0.21, -0.10, 0.12, -0.05, -0.04, -0.04),
    Z,
];

const TAIL_FOX_TIP: &[S] = &[
    M(0.31, -0.24),
    L(0.41, -0.27),
    L(0.39, -0.18),
    L(0.51, -0.22),
    L(0.50, -0.12),
    L(0.68, -0.16),
    L(0.72, -0.52),
    L(0.28, -0.52),
    Z,
];

const TAIL_CAT: &[S] = &[
    M(-0.02, 0.05),
    B(0.22, 0.08, 0.38, -0.06, 0.34, -0.27),
    B(0.31, -0.37, 0.19, -0.37, 0.19, -0.26),
    B(0.20, -0.12, 0.12, -0.02, -0.02, -0.03),
    Z,
];

const TAIL_BUNNY: &[S] = &[
    M(0.0, -0.10),
    B(0.10, -0.16, 0.22, -0.10, 0.21, -0.01),
    B(0.26, 0.05, 0.21, 0.14, 0.12, 0.13),
    B(0.05, 0.16, -0.04, 0.12, -0.04, 0.02),
    Z,
];

const TAIL_REDPANDA: &[S] = &[
    M(-0.02, 0.05),
    B(0.26, 0.11, 0.56, 0.01, 0.58, -0.22),
    B(0.59, -0.35, 0.48, -0.42, 0.38, -0.38),
    B(0.31, -0.35, 0.28, -0.22, 0.23, -0.14),
    B(0.16, -0.06, 0.08, -0.04, -0.04, -0.04),
    Z,
];

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_tail(p: &Pen, sp: Species, sk: &Skin, fur: &ID2D1Brush, body: Matrix3x2, wag_deg: f32, sleeping: bool) -> Result<()> {
    let sleep_off = if sleeping { 0.03 } else { 0.0 };
    match sp {
        Species::Fox => {
            let m = rot(wag_deg + if sleeping { 12.0 } else { 0.0 }, 0.0, 0.0) * tr(0.24, 0.36 + sleep_off) * body;
            let tail = p.path("sp_tail_fox", TAIL_FOX)?;
            let tip = p.clip("sp_tail_fox_tip", &tail, |p| build_path(p.f, TAIL_FOX_TIP))?;
            p.fill_with(m, &tail, fur);
            let tip_c = if sk.pattern == Pattern::Patch { sk.mark } else { sk.cream };
            p.fill(m, &tip, tip_c, 1.0);
            p.stroke(m, &tail, p.line, 1.0, p.lw);
        }
        Species::Cat => {
            let m = rot(wag_deg * 1.15 + if sleeping { 14.0 } else { 0.0 }, 0.0, 0.0) * tr(0.30, 0.36 + sleep_off) * body;
            let tail = p.path("sp_tail_cat", TAIL_CAT)?;
            p.fill_with(m, &tail, fur);
            let tip = p.clip_el("sp_tail_cat_tip", &tail, 0.26, -0.31, 0.10, 0.10)?;
            p.fill(m, &tip, sk.cream, 0.9);
            p.stroke(m, &tail, p.line, 1.0, p.lw);
        }
        Species::Bunny => {
            let m = rot(wag_deg * 0.7, 0.0, 0.0) * tr(0.32, 0.36 + sleep_off) * body;
            let tail = p.path("sp_tail_bunny", TAIL_BUNNY)?;
            p.fill(m, &tail, sk.cream, 1.0);
            p.stroke(m, &tail, p.line, 1.0, p.lw);
        }
        Species::Shiba => {
            let m = rot(wag_deg * 0.75 + if sleeping { 9.0 } else { 0.0 }, 0.0, 0.0) * tr(0.33, 0.32 + sleep_off) * body;
            let outer = p.shape("sp_tail_shiba", |p| p.ellipse_geo(0.12, -0.04, 0.17, 0.17))?;
            p.fill_with(m, &outer, fur);
            p.stroke(m, &outer, p.line, 1.0, p.lw);
            p.fill_el(m, 0.13, -0.03, 0.072, 0.072, sk.cream, 1.0);
            let curl = p.path("sp_tail_shiba_curl", &[M(0.19, -0.01), Q(0.14, 0.06, 0.06, 0.02), Q(0.02, -0.04, 0.07, -0.10), O])?;
            p.stroke(m, &curl, p.line, 0.85, p.lw * 0.75);
        }
        Species::RedPanda => {
            let m = rot(wag_deg * 0.95 + if sleeping { 12.0 } else { 0.0 }, 0.0, 0.0) * tr(0.28, 0.36 + sleep_off) * body;
            let tail = p.path("sp_tail_rp", TAIL_REDPANDA)?;
            p.fill_with(m, &tail, fur);
            if !p.small {
                let dark = mix(sk.fur, INK, 0.68);
                let light = mix(sk.cream, sk.fur, 0.25);
                // Clean parallel rings stepping along the tail curve without overlapping.
                let r1 = p.clip("sp_tail_rp_r1", &tail, |p| build_path(p.f, &[M(0.08, 0.12), L(0.22, -0.06), L(0.27, -0.02), L(0.13, 0.16), Z]))?;
                let b1 = p.clip("sp_tail_rp_b1", &tail, |p| build_path(p.f, &[M(0.20, 0.14), L(0.34, -0.06), L(0.38, -0.02), L(0.24, 0.18), Z]))?;
                let r2 = p.clip("sp_tail_rp_r2", &tail, |p| build_path(p.f, &[M(0.28, 0.10), L(0.45, -0.13), L(0.50, -0.08), L(0.33, 0.15), Z]))?;
                let r3 = p.clip("sp_tail_rp_r3", &tail, |p| build_path(p.f, &[M(0.34, -0.15), L(0.56, -0.31), L(0.60, -0.24), L(0.38, -0.08), Z]))?;
                p.fill(m, &r1, dark, 0.88);
                p.fill(m, &b1, light, 0.78);
                p.fill(m, &r2, dark, 0.88);
                p.fill(m, &r3, dark, 0.88);
            }
            p.stroke(m, &tail, p.line, 1.0, p.lw);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- bodies

pub(super) fn draw_body(p: &Pen, sp: Species, sk: &Skin, fur: &ID2D1Brush, body: Matrix3x2, sleeping: bool) -> Result<()> {
    let info = SPECIES.iter().find(|s| s.id == sp).unwrap();
    let w = info.body_w * if sleeping { 1.07 } else { 1.0 };
    let y_bot = info.desk_y;
    let y_top = if sleeping { 0.21 } else { 0.16 };
    let key: &'static str = match (sp, sleeping) {
        (Species::Fox, false) => "sp_body_fox",
        (Species::Fox, true) => "sp_body_fox_s",
        (Species::Cat, false) => "sp_body_cat",
        (Species::Cat, true) => "sp_body_cat_s",
        (Species::Bunny, false) => "sp_body_bunny",
        (Species::Bunny, true) => "sp_body_bunny_s",
        (Species::Shiba, false) => "sp_body_shiba",
        (Species::Shiba, true) => "sp_body_shiba_s",
        (Species::RedPanda, false) => "sp_body_rp",
        (Species::RedPanda, true) => "sp_body_rp_s",
    };
    let g = p.shape(key, |p| {
        let k = if sp == Species::Fox { 1.03 } else { 1.10 };
        build_path(
            p.f,
            &[
                M(-w * 0.86, y_top),
                B(-w * k, 0.28, -w * k, y_bot, -w * 0.62, y_bot),
                L(w * 0.62, y_bot),
                B(w * k, y_bot, w * k, 0.28, w * 0.86, y_top),
                Z,
            ],
        )
    })?;
    p.fill_with(body, &g, fur);

    // Chest ruff / belly patch (warm cream for contrast under chin and between arms).
    if sp == Species::Fox {
        let ruff = p.path("sp_ruff_fox", &[M(-0.17, 0.20), Q(-0.15, 0.34, -0.07, 0.37), Q(0.0, 0.43, 0.07, 0.37), Q(0.15, 0.34, 0.17, 0.20), Z])?;
        p.fill(body, &ruff, sk.cream, 0.96);
    } else {
        let rx = if sp == Species::RedPanda { 0.18 } else { w * 0.54 };
        p.fill_el(body, 0.0, 0.37, rx, 0.10, sk.cream, 0.92);
    }

    // Soft under-chin ambient occlusion separating head from torso.
    if !p.small {
        p.at(body);
        let sh = p.radial(0.0, 0.25, 0.30, 0.13, &[(0.0, INK, 0.26), (1.0, INK, 0.0)], 1.0)?;
        p.fill_with(body, &g, &sh);
    }

    p.stroke(body, &g, p.line, 1.0, p.lw);
    Ok(())
}

// ---------------------------------------------------------------- ears

const EAR_FOX: &[S] = &[M(-0.14, 0.04), B(-0.10, -0.22, 0.05, -0.44, 0.18, -0.43), B(0.26, -0.41, 0.26, -0.10, 0.15, 0.15), Z];
const EAR_FOX_IN: &[S] = &[M(-0.06, 0.03), B(-0.03, -0.15, 0.06, -0.30, 0.14, -0.29), B(0.18, -0.27, 0.18, -0.07, 0.09, 0.08), Z];
const EAR_FOX_TUFT: &[S] = &[M(-0.07, 0.05), Q(0.0, -0.05, 0.05, -0.01), Q(0.07, -0.06, 0.11, -0.01), Q(0.06, 0.08, -0.04, 0.08), Z];

const EAR_CAT: &[S] = &[M(-0.14, 0.04), B(-0.11, -0.16, 0.02, -0.31, 0.12, -0.29), B(0.20, -0.27, 0.20, -0.06, 0.13, 0.12), Z];
const EAR_CAT_IN: &[S] = &[M(-0.07, 0.02), B(-0.05, -0.10, 0.03, -0.20, 0.09, -0.19), B(0.14, -0.18, 0.14, -0.06, 0.08, 0.06), Z];

const EAR_BUNNY: &[S] = &[M(-0.08, 0.04), B(-0.15, -0.26, -0.06, -0.56, 0.09, -0.54), B(0.24, -0.51, 0.22, -0.18, 0.09, 0.06), Z];
const EAR_BUNNY_IN: &[S] = &[M(-0.02, -0.01), B(-0.07, -0.22, -0.01, -0.43, 0.07, -0.42), B(0.15, -0.40, 0.14, -0.18, 0.05, 0.01), Z];

const EAR_SHIBA: &[S] = &[M(-0.14, 0.04), B(-0.10, -0.15, 0.02, -0.30, 0.11, -0.28), B(0.19, -0.26, 0.20, -0.06, 0.13, 0.12), Z];
const EAR_SHIBA_IN: &[S] = &[M(-0.07, 0.02), B(-0.04, -0.10, 0.03, -0.20, 0.09, -0.18), B(0.13, -0.16, 0.14, -0.05, 0.08, 0.06), Z];

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_ears(p: &Pen, sp: Species, sk: &Skin, fur: &ID2D1Brush, head: Matrix3x2, ear_deg: [f32; 2], sleeping: bool) -> Result<()> {
    let droop = if sleeping { if sp == Species::Bunny { 24.0 } else { 15.0 } } else { 0.0 };
    for (idx, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let deg = ear_deg[idx] + droop;
        let mirror_x = if side < 0.0 { flip() } else { Matrix3x2::identity() };
        match sp {
            Species::Fox => {
                let m = rot(deg, 0.0, 0.0) * tr(0.24, -0.22) * mirror_x * head;
                let outer = p.path("sp_ear_fox", EAR_FOX)?;
                let inner = p.path("sp_ear_fox_in", EAR_FOX_IN)?;
                if sk.pattern == Pattern::Patch {
                    p.fill(m, &outer, sk.mark, 1.0);
                } else {
                    p.fill_with(m, &outer, fur);
                    let tip = p.clip_el("sp_ear_fox_tip", &outer, 0.18, -0.41, 0.16, 0.14)?;
                    let tip_c = if sk.hair != Hair::None { mix(sk.hairc, p.line, 0.15) } else { mix(sk.fur, INK, 0.55) };
                    p.fill(m, &tip, tip_c, 1.0);
                }
                p.fill(m, &inner, sk.ear, 1.0);
                if !p.small {
                    let tuft = p.path("sp_ear_fox_tuft", EAR_FOX_TUFT)?;
                    p.fill(m, &tuft, sk.cream, 0.95);
                }
                p.stroke(m, &outer, p.line, 1.0, p.lw);
            }
            Species::Cat => {
                let m = rot(deg, 0.0, 0.0) * tr(0.26, -0.23) * mirror_x * head;
                let outer = p.path("sp_ear_cat", EAR_CAT)?;
                let inner = p.path("sp_ear_cat_in", EAR_CAT_IN)?;
                if sk.pattern == Pattern::Patch {
                    p.fill(m, &outer, sk.mark, 1.0);
                } else {
                    p.fill_with(m, &outer, fur);
                }
                p.fill(m, &inner, sk.ear, 1.0);
                p.stroke(m, &outer, p.line, 1.0, p.lw);
            }
            Species::Bunny => {
                let fold = if side > 0.0 { 7.0 } else { 0.0 };
                let m = rot(8.0 + fold + deg * 1.25, 0.0, 0.0) * tr(0.15, -0.27) * mirror_x * head;
                let outer = p.path("sp_ear_bunny", EAR_BUNNY)?;
                let inner = p.path("sp_ear_bunny_in", EAR_BUNNY_IN)?;
                if sk.pattern == Pattern::Patch {
                    p.fill(m, &outer, sk.mark, 1.0);
                } else {
                    p.fill_with(m, &outer, fur);
                }
                p.fill(m, &inner, sk.ear, 1.0);
                p.stroke(m, &outer, p.line, 1.0, p.lw);
            }
            Species::Shiba => {
                let m = rot(8.0 + deg, 0.0, 0.0) * tr(0.29, -0.21) * mirror_x * head;
                let outer = p.path("sp_ear_shiba", EAR_SHIBA)?;
                let inner = p.path("sp_ear_shiba_in", EAR_SHIBA_IN)?;
                if sk.pattern == Pattern::Patch {
                    p.fill(m, &outer, sk.mark, 1.0);
                } else {
                    p.fill_with(m, &outer, fur);
                }
                p.fill(m, &inner, sk.ear, 1.0);
                p.stroke(m, &outer, p.line, 1.0, p.lw);
            }
            Species::RedPanda => {
                let m = rot(deg * 0.65, 0.0, 0.0) * tr(0.32, -0.23) * mirror_x * head;
                // White-rimmed teddy ears with dark interior.
                p.fill_el(m, 0.02, -0.05, 0.17, 0.16, sk.cream, 1.0);
                p.ring(m, 0.02, -0.05, 0.17, 0.16, p.line, 1.0, p.lw);
                let dark = mix(sk.fur, INK, 0.68);
                p.fill_el(m, 0.0, -0.03, 0.10, 0.095, dark, 1.0);
                if !p.small {
                    p.fill_el(m, -0.01, 0.02, 0.055, 0.028, sk.cream, 0.9);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- head markings

const CREAM_FOX: &[S] = &[
    M(-0.56, 0.07),
    B(-0.34, 0.05, -0.15, -0.03, 0.0, 0.06),
    B(0.15, -0.03, 0.34, 0.05, 0.56, 0.07),
    L(0.56, 0.42),
    L(-0.56, 0.42),
    Z,
];

const CREAM_SHIBA: &[S] = &[
    M(-0.56, 0.09),
    B(-0.36, 0.08, -0.18, -0.02, 0.0, 0.07),
    B(0.18, -0.02, 0.36, 0.08, 0.56, 0.09),
    L(0.56, 0.42),
    L(-0.56, 0.42),
    Z,
];

pub(super) fn draw_head_markings(p: &Pen, sp: Species, sk: &Skin, head: Matrix3x2, hg: &ID2D1Geometry) -> Result<()> {
    match sp {
        Species::Fox => {
            let mask = p.clip("sp_mask_fox", hg, |p| build_path(p.f, CREAM_FOX))?;
            p.fill(head, &mask, sk.cream, 1.0);
        }
        Species::Shiba => {
            let mask = p.clip("sp_mask_shiba", hg, |p| build_path(p.f, CREAM_SHIBA))?;
            p.fill(head, &mask, sk.cream, 1.0);
            for side in [-1.0f32, 1.0] {
                p.fill_el(rot(12.0 * side, 0.17 * side, -0.095) * head, 0.17 * side, -0.095, 0.054, 0.034, sk.cream, 1.0);
            }
        }
        Species::Cat => {
            // Soft muzzle pillow on darker furs; subtle cream chin.
            if luminance(sk.fur) < 0.85 {
                let mask = p.clip("sp_mask_cat", hg, |p| p.ellipse_geo(0.0, 0.24, 0.30, 0.16))?;
                p.fill(head, &mask, sk.cream, 0.85);
            }
        }
        Species::Bunny => {
            if luminance(sk.fur) < 0.85 {
                let mask = p.clip("sp_mask_bunny", hg, |p| p.ellipse_geo(0.0, 0.23, 0.24, 0.14))?;
                p.fill(head, &mask, sk.cream, 0.82);
            }
        }
        Species::RedPanda => {
            p.fill_el(head, 0.0, 0.18, 0.21, 0.14, sk.cream, 1.0);
            for side in [-1.0f32, 1.0] {
                let cm = p.clip(if side < 0.0 { "sp_rp_cl" } else { "sp_rp_cr" }, hg, |p| p.ellipse_geo(0.33 * side, 0.13, 0.17, 0.115))?;
                p.fill(head, &cm, sk.cream, 1.0);
                p.fill_el(rot(11.0 * side, 0.155 * side, -0.095) * head, 0.155 * side, -0.095, 0.052, 0.034, sk.cream, 1.0);
                if !p.small {
                    let tear = mix(sk.fur, INK, 0.38);
                    p.fill_el(rot(-9.0 * side, 0.18 * side, 0.17) * head, 0.18 * side, 0.17, 0.034, 0.068, tear, 0.9);
                }
            }
        }
    }

    // Soft pillowy crown sheen clipped inside the head.
    if !p.small {
        p.at(head);
        let sheen = p.radial(-0.08, -0.24, 0.32, 0.22, &[(0.0, WHITE, 0.28), (1.0, WHITE, 0.0)], 1.0)?;
        p.fill_with(head, hg, &sheen);
    }
    Ok(())
}

/// Delicate cat whiskers drawn after the head outline.
pub(super) fn draw_whiskers(p: &Pen, sp: Species, head: Matrix3x2) {
    if sp != Species::Cat || p.small {
        return;
    }
    for side in [-1.0f32, 1.0] {
        p.seg(head, 0.34 * side, 0.10, 0.49 * side, 0.075, p.line, 0.55, p.lw * 0.45);
        p.seg(head, 0.35 * side, 0.16, 0.49 * side, 0.18, p.line, 0.55, p.lw * 0.45);
    }
}

// ---------------------------------------------------------------- Bongo arms & paws

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_arms(p: &Pen, sp: Species, sk: &Skin, body: Matrix3x2, paws: Option<[f32; 2]>, sleeping: bool, size: f32) -> Result<()> {
    let info = SPECIES.iter().find(|s| s.id == sp).unwrap();
    let dark_socks = matches!(sp, Species::RedPanda | Species::Fox);
    let arm_c = match sp {
        Species::RedPanda => mix(sk.fur, INK, 0.68),
        Species::Fox => mix(sk.fur, INK, 0.18),
        _ => mix(sk.fur, sk.cream, 0.35),
    };
    let paw_c = match sp {
        Species::RedPanda => mix(sk.fur, INK, 0.68),
        Species::Fox => mix(sk.fur, INK, 0.64),
        _ => sk.cream,
    };
    let pad_c = match sp {
        Species::RedPanda => [0.88, 0.70, 0.64],
        Species::Shiba => [0.55, 0.37, 0.27],
        _ => [1.0, 0.62, 0.72],
    };

    for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let down = if sleeping {
            1.0
        } else if let Some(arr) = paws {
            arr[i]
        } else {
            1.0
        };
        let (sx, sy) = ((info.arm_x + 0.02) * side, 0.345);
        let (px, py) = if sleeping {
            // Tucked snugly together under the chin as a sleeping pillow!
            (0.15 * side, info.desk_y - 0.015)
        } else if paws.is_some() {
            let x_up = (info.arm_x + 0.06) * side;
            let y_up = 0.24;
            let x_dn = (info.arm_x + 0.02) * side;
            let y_dn = info.desk_y - 0.01;
            (x_up + (x_dn - x_up) * down, y_up + (y_dn - y_up) * down)
        } else {
            // Resting neatly on the bottom edge in Notch / Settings.
            (0.20 * side, info.desk_y - 0.015)
        };

        let arm_w = match sp {
            Species::Bunny => 0.110,
            Species::Fox => 0.115,
            _ => 0.125,
        };
        // Articulated foreleg sleeve only in active Bongo mode (clean resting paws in Notch/Settings).
        if paws.is_some() && !sleeping {
            p.seg(body, sx, sy, px, py, p.line, 1.0, arm_w + p.lw * 1.6);
            p.seg(body, sx, sy, px, py, arm_c, 1.0, arm_w);
        }

        let base_rx = if sp == Species::Bunny { 0.102 } else { 0.114 };
        let base_ry = if sp == Species::Bunny { 0.086 } else { 0.092 };
        let (rx, ry) = if paws.is_none() && !sleeping {
            (base_rx, 0.072)
        } else {
            (base_rx * (1.0 + down * 0.14), base_ry * (1.0 - down * 0.22))
        };
        let paw_deg = if paws.is_none() && !sleeping {
            0.0
        } else {
            side * (-18.0 * (1.0 - down) + 3.5 * down)
        };
        let pm = rot(paw_deg, 0.0, 0.0) * tr(px, py) * body;

        p.fill_el(pm, 0.0, 0.0, rx, ry, paw_c, 1.0);
        if dark_socks && !p.small && (paws.is_none() || sleeping || down > 0.55) {
            // Subtle warm rim arc on the upper edge of dark resting/slapped paws.
            p.seg(pm, -rx * 0.42, -ry * 0.52, rx * 0.42, -ry * 0.52, WHITE, 0.28, 0.018);
        }
        p.ring(pm, 0.0, 0.0, rx, ry, p.line, 1.0, p.lw);

        if !p.small {
            if paws.is_some() && !sleeping && down < 0.55 {
                let beans = 1.0 - down / 0.55;
                p.fill_el(pm, 0.0, 0.016, 0.038, 0.027, pad_c, beans * 0.95);
                for (bx, by) in [(-0.042, -0.024), (0.0, -0.036), (0.042, -0.024)] {
                    p.fill_el(pm, bx, by, 0.016, 0.016, pad_c, beans * 0.95);
                }
            } else {
                let crease = if dark_socks { WHITE } else { p.line };
                let ca = if dark_socks { 0.28 } else { 0.55 };
                for tx in [-0.032f32, 0.032] {
                    p.seg(pm, tx, ry * 0.30, tx, ry * 0.80, crease, ca, p.lw * 0.55);
                }
            }
        }

        if paws.is_some() && !sleeping && down > 0.5 {
            let a = (down - 0.5) * 2.0;
            tap_burst(p, px, py, side, size, a)?;
        }
    }
    Ok(())
}

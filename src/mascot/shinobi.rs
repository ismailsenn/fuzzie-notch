//! Shinobi pack: a ninja-themed set of looks and gear for the fox. Everything
//! is our own drawing in the fox's style: hair, patterned eyes, face marks,
//! aura effects and gear, built from the same head-unit shapes as the rest.

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

/// An ellipse as path commands.
fn oval(x: f32, y: f32, rx: f32, ry: f32) -> Vec<S> {
    circle()
        .iter()
        .map(|s| match *s {
            M(a, b) => M(x + a * rx, y + b * ry),
            B(a, b, c, d, e, f) => B(x + a * rx, y + b * ry, x + c * rx, y + d * ry, x + e * rx, y + f * ry),
            o => o,
        })
        .collect()
}

/// Path commands moved by (dx, dy).
fn shift(cmds: &[S], dx: f32, dy: f32) -> Vec<S> {
    cmds.iter()
        .map(|s| match *s {
            M(x, y) => M(x + dx, y + dy),
            L(x, y) => L(x + dx, y + dy),
            Q(a, b, x, y) => Q(a + dx, b + dy, x + dx, y + dy),
            B(a, b, c, d, x, y) => B(a + dx, b + dy, c + dx, d + dy, x + dx, y + dy),
            o => o,
        })
        .collect()
}

/// A tomoe: round head of radius 1 at the origin, its tail sweeping round
/// clockwise (towards +x along the top) and thinning to a point.
fn tomoe() -> Vec<S> {
    let mut v = circle().to_vec();
    v.extend_from_slice(&[M(-0.98, -0.2), Q(-0.7, -1.5, 0.9, -1.75), Q(-0.05, -1.25, 0.7, -0.7), Z]);
    v
}

/// Teardrop flame pointing up, about 1 tall, base at y = 0.5.
const FLAME: &[S] = &[M(0.0, 0.5), Q(-0.55, 0.32, -0.32, -0.15), Q(-0.12, 0.05, 0.0, -1.0), Q(0.12, 0.05, 0.32, -0.15), Q(0.55, 0.32, 0.0, 0.5), Z];

/// A crow in flight, wings spread (1 wide).
const CROW: &[S] = &[M(-0.5, -0.05), Q(-0.3, -0.32, -0.06, -0.06), L(0.0, -0.12), L(0.06, -0.06), Q(0.3, -0.32, 0.5, -0.05), Q(0.26, -0.1, 0.12, 0.04), L(0.06, 0.2), L(0.0, 0.12), L(-0.06, 0.2), L(-0.12, 0.04), Q(-0.26, -0.1, -0.5, -0.05), Z];

/// The pattern of one eye (some skins differ between the two).
pub(super) fn eye_for(eye: Eye, side: f32) -> Eye {
    match eye {
        Eye::OneTomoe if side > 0.0 => Eye::Tomoe,
        Eye::OneTomoe => Eye::Normal,
        e => e,
    }
}

/// A red iris: bright in the middle, deep at the rim, with a dark rim line.
fn red_iris(p: &Pen, m: Matrix3x2) -> Result<()> {
    p.at(m);
    let g = p.radial(0.0, 0.0, 1.0, 1.0, &[(0.0, [1.0, 0.36, 0.32], 1.0), (0.55, [0.9, 0.08, 0.12], 1.0), (1.0, [0.56, 0.02, 0.07], 1.0)], 1.0)?;
    unsafe { p.rt.FillEllipse(&super::ell(0.0, 0.0, 1.0, 1.0), &g) };
    p.ring(m, 0.0, 0.0, 0.94, 0.94, BLACK, 0.95, 0.12);
    Ok(())
}

/// Draws a patterned eye; `m` maps the unit circle onto the open eye.
pub(super) fn iris(p: &Pen, eye: Eye, m: Matrix3x2, t: f32) -> Result<()> {
    match eye {
        Eye::Tomoe => {
            red_iris(p, m)?;
            p.ring(m, 0.0, 0.0, 0.54, 0.54, BLACK, 0.5, 0.05);
            p.fill_el(m, 0.0, 0.0, 0.2, 0.2, BLACK, 1.0);
            let tom = p.shape("sh_tomoe", |p| build_path(p.f, &tomoe()))?;
            for k in 0..3 {
                let a = t * 40.0 + k as f32 * 120.0;
                let (s, c) = a.to_radians().sin_cos();
                // Each comma sits on the ring, its tail trailing round it.
                p.fill(put(0.54 * s, -0.54 * c, 0.19, a + 90.0) * m, &tom, BLACK, 1.0);
            }
        }
        Eye::Pinwheel => {
            red_iris(p, m)?;
            // Three curved blades sweeping out from a dark hub.
            let blade = p.path("sh_blade", &[M(-0.2, -0.2), Q(0.1, -0.95, 0.86, -0.42), Q(0.34, -0.42, 0.24, -0.08), Z])?;
            for k in 0..3 {
                p.fill(rot(t * 22.0 + k as f32 * 120.0, 0.0, 0.0) * m, &blade, BLACK, 1.0);
            }
            p.fill_el(m, 0.0, 0.0, 0.34, 0.34, BLACK, 1.0);
            p.fill_el(m, 0.0, 0.0, 0.12, 0.12, [0.9, 0.08, 0.12], 1.0);
        }
        Eye::Petals => {
            red_iris(p, m)?;
            // A dark ring crossed by three long petals, and three thin ones between.
            p.ring(m, 0.0, 0.0, 0.6, 0.6, BLACK, 1.0, 0.13);
            let petal = p.path("sh_petal", &[M(0.0, -0.94), Q(0.2, -0.5, 0.0, -0.06), Q(-0.2, -0.5, 0.0, -0.94), Z])?;
            let thin = p.path("sh_petal_thin", &[M(0.0, -0.86), Q(0.08, -0.6, 0.0, -0.34), Q(-0.08, -0.6, 0.0, -0.86), Z])?;
            for k in 0..3 {
                let a = t * 16.0 + k as f32 * 120.0;
                p.fill(rot(a, 0.0, 0.0) * m, &petal, BLACK, 1.0);
                p.fill(rot(a + 60.0, 0.0, 0.0) * m, &thin, BLACK, 0.9);
            }
            p.fill_el(m, 0.0, 0.0, 0.2, 0.2, BLACK, 1.0);
        }
        Eye::Ripple => {
            p.at(m);
            let g = p.radial(0.0, 0.0, 1.0, 1.0, &[(0.0, [0.93, 0.9, 1.0], 1.0), (0.7, [0.76, 0.7, 0.96], 1.0), (1.0, [0.6, 0.52, 0.86], 1.0)], 1.0)?;
            unsafe { p.rt.FillEllipse(&super::ell(0.0, 0.0, 1.0, 1.0), &g) };
            let ink = [0.3, 0.22, 0.48];
            for r in [0.36f32, 0.64, 0.92] {
                p.ring(m, 0.0, 0.0, r, r, ink, 1.0, 0.085);
            }
            p.fill_el(m, 0.0, 0.0, 0.14, 0.14, ink, 1.0);
        }
        Eye::Beast => {
            p.at(m);
            let g = p.radial(0.0, 0.0, 1.0, 1.0, &[(0.0, [1.0, 0.86, 0.3], 1.0), (0.6, [1.0, 0.5, 0.08], 1.0), (1.0, [0.78, 0.18, 0.05], 1.0)], 1.0)?;
            unsafe { p.rt.FillEllipse(&super::ell(0.0, 0.0, 1.0, 1.0), &g) };
            p.ring(m, 0.0, 0.0, 0.94, 0.94, [0.45, 0.08, 0.02], 0.9, 0.1);
            p.fill_el(m, 0.0, 0.0, 0.17, 0.8, BLACK, 1.0);
        }
        Eye::Normal | Eye::OneTomoe => {}
    }
    Ok(())
}

// ---------------------------------------------------------------- hair

/// One lock of hair: base (x, y), direction in degrees (0 up, 90 right,
/// 180 down), length, half-width at the base, and how far the tip bends
/// to the lock's right.
#[derive(Clone, Copy)]
struct Lock(f32, f32, f32, f32, f32, f32);

impl Lock {
    /// A lock pointing out from (cx, cy) at `deg`, its tip `r` away.
    fn ray(cx: f32, cy: f32, deg: f32, r: f32, w: f32, bend: f32) -> Lock {
        let (s, c) = deg.to_radians().sin_cos();
        Lock(cx + 0.2 * s, cy - 0.2 * c, deg, r - 0.2, w, bend)
    }

    fn mirrored(self) -> Lock {
        let Lock(x, y, deg, len, w, bend) = self;
        Lock(-x, y, -deg, len, w, -bend)
    }

    /// The point `a` along the lock and `b` to its right.
    fn at(self, a: f32, b: f32) -> (f32, f32) {
        let Lock(x, y, deg, ..) = self;
        let (s, c) = deg.to_radians().sin_cos();
        (x + s * a + c * b, y - c * a + s * b)
    }

    /// A curved, pointed lock whose root is buried a little behind its base.
    fn path(self) -> Vec<S> {
        let Lock(_, _, _, len, w, bend) = self;
        let l0 = self.at(0.0, -w);
        let r0 = self.at(0.0, w);
        let tip = self.at(len, bend);
        let cl = self.at(len * 0.55, -w * 0.8 + bend * 0.5);
        let cr = self.at(len * 0.55, w * 0.8 + bend * 0.5);
        let root = self.at(-0.07, 0.0);
        vec![M(l0.0, l0.1), Q(cl.0, cl.1, tip.0, tip.1), Q(cr.0, cr.1, r0.0, r0.1), L(root.0, root.1), Z]
    }
}

/// `locks` and their mirror images.
fn both(locks: &[Lock]) -> Vec<Lock> {
    locks.iter().copied().chain(locks.iter().map(|l| l.mirrored())).collect()
}

/// Rays from (cx, cy): (degrees, tip distance, half-width, bend); with
/// `mirrored`, each one off the middle line gets its mirror image too.
fn rays(cx: f32, cy: f32, v: &[(f32, f32, f32, f32)], mirrored: bool) -> Vec<Lock> {
    let mut out = Vec::new();
    for &(d, r, w, b) in v {
        let l = Lock::ray(cx, cy, d, r, w, b);
        out.push(l);
        if mirrored && d != 0.0 {
            out.push(l.mirrored());
        }
    }
    out
}

/// The cap of hair over the forehead, hugging the top of the head (it covers
/// the roots of the ears); the bangs hang from it.
const CAP: &[S] = &[M(-0.47, -0.02), Q(-0.5, -0.28, -0.24, -0.335), Q(0.0, -0.375, 0.24, -0.335), Q(0.5, -0.28, 0.47, -0.02), Q(0.36, -0.2, 0.0, -0.21), Q(-0.36, -0.2, -0.47, -0.02), Z];
/// The cap parted in the middle.
const CAP_PART: &[S] = &[
    M(-0.47, -0.02),
    Q(-0.5, -0.28, -0.24, -0.335),
    Q(0.0, -0.375, 0.24, -0.335),
    Q(0.5, -0.28, 0.47, -0.02),
    Q(0.38, -0.2, 0.06, -0.24),
    L(0.0, -0.29),
    L(-0.06, -0.24),
    Q(-0.38, -0.2, -0.47, -0.02),
    Z,
];

/// Hair behind the head and ears: a mass and short, chunky locks.
fn back_hair(h: Hair) -> Vec<Vec<S>> {
    let top = oval(0.0, -0.14, 0.42, 0.27);
    let (mass, locks) = match h {
        Hair::Spiky => (top, rays(0.0, -0.12, &[(0.0, 0.57, 0.12, 0.03), (26.0, 0.57, 0.12, 0.06), (56.0, 0.55, 0.12, 0.07), (88.0, 0.5, 0.11, 0.06), (116.0, 0.42, 0.1, 0.05)], true)),
        Hair::Flash => (top, rays(0.0, -0.12, &[(0.0, 0.56, 0.12, 0.0), (28.0, 0.57, 0.12, 0.07), (60.0, 0.55, 0.12, 0.08), (92.0, 0.5, 0.11, 0.07)], true)),
        Hair::Swept => (top, rays(0.02, -0.12, &[(-80.0, 0.46, 0.11, -0.05), (-45.0, 0.5, 0.11, -0.04), (-10.0, 0.54, 0.12, 0.03), (22.0, 0.57, 0.12, 0.07), (52.0, 0.6, 0.12, 0.09), (82.0, 0.57, 0.12, 0.09), (110.0, 0.48, 0.11, 0.07)], false)),
        Hair::Leaning => (top, rays(0.0, -0.12, &[(-90.0, 0.46, 0.11, -0.05), (-62.0, 0.58, 0.12, -0.08), (-36.0, 0.64, 0.12, -0.09), (-10.0, 0.6, 0.12, -0.08), (18.0, 0.52, 0.11, -0.06), (50.0, 0.45, 0.1, -0.04), (85.0, 0.42, 0.1, -0.03)], false)),
        Hair::Mane => (oval(0.0, -0.04, 0.5, 0.4), rays(0.0, -0.04, &[(0.0, 0.6, 0.13, 0.03), (28.0, 0.62, 0.13, 0.06), (58.0, 0.63, 0.13, 0.08), (90.0, 0.64, 0.13, 0.08), (122.0, 0.62, 0.13, 0.07), (152.0, 0.56, 0.12, 0.05)], true)),
        Hair::Short => (oval(0.0, -0.14, 0.42, 0.26), rays(0.0, -0.12, &[(0.0, 0.48, 0.11, 0.02), (28.0, 0.48, 0.11, 0.04), (58.0, 0.47, 0.11, 0.05), (90.0, 0.44, 0.1, 0.05)], true)),
        Hair::Straight | Hair::Parted => (top, rays(0.0, -0.12, &[(40.0, 0.5, 0.12, 0.03), (80.0, 0.48, 0.12, 0.04)], true)),
        Hair::Sharp => (top, rays(0.0, -0.12, &[(0.0, 0.6, 0.085, 0.0), (18.0, 0.58, 0.085, 0.02), (38.0, 0.57, 0.085, 0.03), (60.0, 0.56, 0.085, 0.03), (84.0, 0.52, 0.085, 0.03), (108.0, 0.44, 0.08, 0.02)], true)),
        Hair::None => return Vec::new(),
    };
    std::iter::once(mass).chain(locks.iter().map(|l| l.path())).collect()
}

/// The cap and the big, soft bangs hanging from it.
fn front_hair(h: Hair) -> (&'static [S], Vec<Lock>) {
    let mut v = Vec::new();
    let cap = match h {
        Hair::Spiky => {
            v.push(Lock(0.0, -0.27, 180.0, 0.2, 0.1, 0.03));
            v.extend(both(&[Lock(0.18, -0.26, 166.0, 0.19, 0.095, 0.04), Lock(0.34, -0.2, 150.0, 0.16, 0.085, 0.04)]));
            CAP
        }
        Hair::Flash => {
            v.extend([Lock(-0.07, -0.27, 186.0, 0.19, 0.09, -0.03), Lock(0.11, -0.27, 174.0, 0.18, 0.09, 0.03)]);
            v.extend(both(&[Lock(0.36, -0.21, 176.0, 0.32, 0.075, 0.04)]));
            CAP
        }
        Hair::Swept => {
            v.extend([Lock(0.03, -0.27, 190.0, 0.2, 0.1, -0.04), Lock(-0.18, -0.25, 198.0, 0.17, 0.09, -0.03), Lock(0.22, -0.25, 168.0, 0.16, 0.085, 0.03)]);
            v.extend(both(&[Lock(0.38, -0.19, 176.0, 0.28, 0.07, 0.035)]));
            CAP
        }
        Hair::Leaning => {
            v.extend([
                Lock(-0.3, -0.21, 206.0, 0.15, 0.085, -0.03),
                Lock(-0.12, -0.26, 200.0, 0.19, 0.095, -0.04),
                Lock(0.07, -0.27, 194.0, 0.19, 0.095, -0.04),
                Lock(0.26, -0.24, 186.0, 0.16, 0.09, -0.03),
                Lock(0.41, -0.15, 176.0, 0.11, 0.07, -0.01),
            ]);
            CAP
        }
        Hair::Mane => {
            v.push(Lock(0.0, -0.27, 184.0, 0.2, 0.1, -0.03));
            v.extend(both(&[Lock(0.18, -0.26, 170.0, 0.19, 0.095, 0.04), Lock(0.38, -0.19, 174.0, 0.34, 0.08, 0.04)]));
            CAP
        }
        Hair::Short => {
            v.push(Lock(0.0, -0.27, 180.0, 0.13, 0.09, 0.0));
            v.extend(both(&[Lock(0.16, -0.26, 168.0, 0.13, 0.085, 0.02), Lock(0.31, -0.22, 154.0, 0.12, 0.08, 0.02)]));
            CAP
        }
        // Curtain bangs: from a part in the middle, full locks sweep down
        // and out over the forehead, then long locks frame the face.
        Hair::Straight => {
            v.extend(both(&[Lock(0.05, -0.27, 156.0, 0.2, 0.1, 0.05), Lock(0.22, -0.25, 162.0, 0.17, 0.09, 0.04), Lock(0.4, -0.18, 178.0, 0.42, 0.075, 0.03)]));
            CAP_PART
        }
        Hair::Parted => {
            v.extend(both(&[Lock(0.05, -0.27, 160.0, 0.19, 0.1, 0.05), Lock(0.22, -0.25, 166.0, 0.16, 0.09, 0.04), Lock(0.39, -0.19, 177.0, 0.33, 0.07, 0.03)]));
            CAP_PART
        }
        Hair::Sharp => {
            v.push(Lock(0.0, -0.27, 180.0, 0.18, 0.065, 0.0));
            v.extend(both(&[Lock(0.13, -0.27, 170.0, 0.18, 0.065, 0.02), Lock(0.26, -0.24, 158.0, 0.16, 0.065, 0.025), Lock(0.38, -0.18, 146.0, 0.13, 0.06, 0.02)]));
            CAP
        }
        Hair::None => CAP,
    };
    (cap, v)
}

/// Cache keys per hair style: back, front, the front's shadow on the forehead.
fn keys(h: Hair) -> (&'static str, &'static str, &'static str) {
    match h {
        Hair::Spiky => ("sh_hb_spiky", "sh_hf_spiky", "sh_hs_spiky"),
        Hair::Flash => ("sh_hb_flash", "sh_hf_flash", "sh_hs_flash"),
        Hair::Swept => ("sh_hb_swept", "sh_hf_swept", "sh_hs_swept"),
        Hair::Leaning => ("sh_hb_lean", "sh_hf_lean", "sh_hs_lean"),
        Hair::Mane => ("sh_hb_mane", "sh_hf_mane", "sh_hs_mane"),
        Hair::Short => ("sh_hb_short", "sh_hf_short", "sh_hs_short"),
        Hair::Straight => ("sh_hb_straight", "sh_hf_straight", "sh_hs_straight"),
        Hair::Parted => ("sh_hb_parted", "sh_hf_parted", "sh_hs_parted"),
        Hair::Sharp => ("sh_hb_sharp", "sh_hf_sharp", "sh_hs_sharp"),
        Hair::None => ("", "", ""),
    }
}

/// Outline of the hair: darker than the hair, but on very dark hair a
/// sheen-tinted edge so it still reads on a black background.
fn hair_line(sk: &Skin) -> Rgb {
    if super::luminance(sk.hairc) < 0.2 {
        mix(sk.sheen, INK, 0.35)
    } else {
        mix(sk.hairc, INK, 0.58)
    }
}

/// The shinobi looks are drawn a little more anime: lashes over the eyes.
pub(super) fn anime(sk: &Skin) -> bool {
    sk.hair != Hair::None || sk.eye != Eye::Normal
}

/// An upper lash over a unit-circle eye (right eye; mirrored for the left),
/// thick in the middle and flicked out at the outer corner.
pub(super) const LASH: &[S] = &[M(-1.1, -0.3), Q(-0.66, -1.32, 0.34, -1.24), Q(1.0, -1.16, 1.42, -0.62), L(1.5, -0.86), Q(1.1, -0.92, 0.94, -0.92), Q(0.6, -1.02, 0.3, -1.04), Q(-0.56, -1.08, -1.1, -0.3), Z];

pub(super) fn lash_colour(sk: &Skin) -> Rgb {
    let _ = sk;
    [0.17, 0.11, 0.15]
}

/// Hair seen behind the head and ears.
pub(super) fn hair_back(p: &Pen, sk: &Skin, head: Matrix3x2) -> Result<()> {
    if sk.hair == Hair::None {
        return Ok(());
    }
    let h = sk.hair;
    let g = p.union(keys(h).0, || back_hair(h))?;
    let c = sk.hairc;
    p.at(head);
    let b = p.linear(0.0, -0.75, 0.0, 0.5, &[(0.0, mix(c, sk.sheen, 0.3), 1.0), (0.5, mix(c, INK, 0.08), 1.0), (1.0, mix(c, INK, 0.3), 1.0)])?;
    p.fill_with(head, &g, &b);
    p.stroke(head, &g, hair_line(sk), 1.0, p.lw * 0.75);
    Ok(())
}

/// The cap and bangs (with their soft shadow on the forehead and a glossy
/// ring of shine), then the face marks. Drawn after the head's outline.
pub(super) fn over_head(p: &Pen, sk: &Skin, head: Matrix3x2) -> Result<()> {
    if sk.hair != Hair::None {
        let h = sk.hair;
        let (_, key, skey) = keys(h);
        let parts = move |dy: f32| {
            let (cap, locks) = front_hair(h);
            std::iter::once(shift(cap, 0.0, dy)).chain(locks.iter().map(|l| shift(&l.path(), 0.0, dy))).collect::<Vec<_>>()
        };
        let g = p.union(key, || parts(0.0))?;
        let hg = p.head()?;
        let sh = p.clip(skey, &hg, |p| super::union_paths(p.f, &parts(0.025)))?;
        p.fill(head, &sh, mix(sk.fur, INK, 0.5), 0.16);
        let c = sk.hairc;
        p.at(head);
        let b = p.linear(0.0, -0.38, 0.0, 0.05, &[(0.0, mix(c, sk.sheen, 0.45), 1.0), (0.4, c, 1.0), (1.0, mix(c, INK, 0.16), 1.0)])?;
        p.fill_with(head, &g, &b);
        if !p.small {
            // The glossy "ring" of shine across the crown.
            let lens = p.path("sh_shine", &[M(-1.0, 0.0), Q(0.0, -0.9, 1.0, 0.0), Q(0.0, 0.5, -1.0, 0.0), Z])?;
            for x in [-0.27f32, -0.1, 0.1, 0.27] {
                let y = -0.298 + 0.36 * x * x;
                let m = Matrix3x2::scale(0.06, 0.026) * rot(x * 75.0, 0.0, 0.0) * tr(x, y) * head;
                p.fill(m, &lens, sk.sheen, 0.8);
            }
        }
        if std::ptr::eq(front_hair(h).0, CAP_PART) && !p.small {
            let part = p.path("sh_part", &[M(0.0, -0.37), Q(0.006, -0.33, 0.0, -0.28), O])?;
            p.stroke(head, &part, hair_line(sk), 0.9, p.lw * 0.6);
        }
        p.stroke(head, &g, hair_line(sk), 1.0, p.lw * 0.75);
    }

    // Face marks.
    let mc = sk.facec;
    match sk.face {
        Face::Whiskers => {
            let w = p.path("sh_whisker", &[M(0.0, 0.0), Q(0.07, -0.012, 0.13, 0.0), O])?;
            for s in [Matrix3x2::identity(), flip()] {
                for (y, deg) in [(0.115f32, -12.0f32), (0.165, 0.0), (0.215, 12.0)] {
                    p.stroke(rot(deg, 0.0, 0.0) * tr(0.32, y) * s * head, &w, mc, 0.85, 0.02);
                }
            }
        }
        Face::Mask => {
            let g = p.on_head("sh_mask", &[M(-0.6, 0.125), Q(0.0, 0.085, 0.6, 0.125), L(0.6, 0.6), L(-0.6, 0.6), Z])?;
            p.at(head);
            let b = p.linear(0.0, 0.1, 0.0, 0.4, &[(0.0, mix(mc, WHITE, 0.14), 1.0), (1.0, mix(mc, INK, 0.2), 1.0)])?;
            p.fill_with(head, &g, &b);
            p.stroke(head, &g, mix(mc, INK, 0.5), 1.0, p.lw * 0.8);
            // The nose under the cloth, and a fold or two.
            p.fill_el(head, 0.0, 0.2, 0.05, 0.03, WHITE, 0.14);
            let fold = p.path("sh_mask_fold", &[M(-0.2, 0.3), Q(0.0, 0.34, 0.2, 0.3), O])?;
            p.stroke(head, &fold, mix(mc, INK, 0.45), 0.7, 0.014);
        }
        Face::RedMarks => {
            // A thin tapering stripe under each eye.
            let mark = p.path("sh_redmark", &[M(-0.013, 0.0), Q(0.0, -0.008, 0.013, 0.0), L(0.0, 0.06), Z])?;
            for side in [-1.0f32, 1.0] {
                p.fill(tr(0.185 * side, 0.19) * head, &mark, mc, 0.85);
            }
        }
        Face::TearLines => {
            let l = p.path("sh_tear", &[M(0.105, 0.14), Q(0.115, 0.19, 0.155, 0.215), O])?;
            for s in [Matrix3x2::identity(), flip()] {
                p.stroke(s * head, &l, mc, 0.8, 0.015);
            }
        }
        Face::Beast => {
            // Dark marks sweeping from the eyes up towards the ears.
            let g = p.path("sh_beast", &[M(0.265, 0.07), Q(0.34, 0.0, 0.42, -0.15), Q(0.37, -0.02, 0.26, 0.02), Z])?;
            for s in [Matrix3x2::identity(), flip()] {
                p.fill(s * head, &g, mc, 0.95);
            }
            let brow = p.path("sh_beast_brow", &[M(0.1, -0.06), Q(0.17, -0.1, 0.27, -0.06), O])?;
            for s in [Matrix3x2::identity(), flip()] {
                p.stroke(s * head, &brow, mc, 0.9, 0.024);
            }
            for side in [-1.0f32, 1.0] {
                for (y0, y1) in [(0.13, 0.11), (0.18, 0.18), (0.23, 0.25)] {
                    p.seg(head, 0.34 * side, y0, 0.46 * side, y1, mc, 0.8, 0.024);
                }
            }
        }
        Face::Studs => {
            // Studs down the bridge of the nose and under the lip.
            for (x, y) in [(0.0f32, 0.035f32), (0.0, 0.08), (0.0, 0.125), (-0.035, 0.31), (0.035, 0.31)] {
                p.fill_el(head, x, y, 0.017, 0.017, mix(mc, INK, 0.5), 1.0);
                p.fill_el(head, x, y, 0.012, 0.012, mc, 1.0);
                p.fill_el(head, x - 0.004, y - 0.004, 0.005, 0.005, WHITE, 0.95);
            }
        }
        Face::None => {}
    }
    Ok(())
}

/// Marks drawn over the eyes (a scar across the patterned eye).
pub(super) fn over_eyes(p: &Pen, sk: &Skin, head: Matrix3x2) -> Result<()> {
    if sk.eye == Eye::OneTomoe {
        // Above and below the eye, never across it.
        let scar = p.path("sh_scar", &[M(0.172, -0.14), L(0.178, -0.065), M(0.192, 0.2), L(0.199, 0.26), O])?;
        p.stroke(head, &scar, [0.55, 0.28, 0.3], 0.9, 0.016);
    }
    Ok(())
}

/// Nine tails fanned out behind (the beast). Returns false for other skins.
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
    if sk.fx != Fx::Aura {
        return Ok(());
    }
    let c = sk.fxc;
    let body = Matrix3x2::identity();
    let pulse = 0.8 + 0.2 * (t * 2.4).sin();
    p.glow(body, 0.0, -0.05, 0.9 * pulse, 0.84 * pulse, c, 0.45)?;
    // Flames licking up around the outline.
    let flame = p.path("sh_flame", FLAME)?;
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
            let crow = p.path("sh_crow", CROW)?;
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
        // Leaf plate: a blue cloth band with a polished plate engraved with a
        // leaf swirl; the knot's tails flutter behind.
        (0, Front) => {
            let blue = [0.17, 0.29, 0.62];
            let tail = p.path("sh_band_tail", &[M(0.0, -0.02), B(-0.08, -0.03, -0.16, 0.02, -0.3, 0.0), L(-0.28, 0.065), B(-0.16, 0.075, -0.08, 0.045, 0.0, 0.035), Z])?;
            for (j, base) in [-14.0f32, -36.0].into_iter().enumerate() {
                let deg = base + (t * 6.5 + j as f32 * 1.3).sin() * 10.0;
                p.obj(rot(deg, 0.0, 0.0) * tr(-0.41, -0.19) * h, 1.0, &tail, mix(blue, INK, 0.1 * j as f32));
            }
            let band = p.on_head("headband", super::BAND)?;
            p.at(h);
            let cloth = p.linear(0.0, -0.31, 0.0, -0.155, &[(0.0, mix(blue, WHITE, 0.18), 1.0), (1.0, mix(blue, INK, 0.15), 1.0)])?;
            p.fill_with(h, &band, &cloth);
            p.stroke(h, &band, edge(blue), 1.0, lw);
            p.fill_el(h, -0.41, -0.19, 0.036, 0.032, blue, 1.0);
            let plate = p.path("sh_plate", &rrect(-0.17, -0.318, 0.17, -0.182, 0.026))?;
            p.at(h);
            let steel = p.linear(0.0, -0.318, 0.0, -0.182, &[(0.0, STEEL_HI, 1.0), (0.45, STEEL, 1.0), (1.0, STEEL_LO, 1.0)])?;
            p.fill_with(h, &plate, &steel);
            p.stroke(h, &plate, [0.3, 0.33, 0.4], 1.0, lw);
            for (x, y) in [(-0.145, -0.296), (0.145, -0.296), (-0.145, -0.204), (0.145, -0.204)] {
                p.fill_el(h, x, y, 0.009, 0.009, [0.42, 0.45, 0.52], 1.0);
            }
            if !p.small {
                // Engraved leaf: a spiral with a pointed tip, cut in (dark) with a lit edge.
                let mut v = Vec::new();
                for j in 0..=28 {
                    let u = j as f32 / 28.0;
                    let a = -PI / 2.0 + u * PI * 3.3;
                    let r = 0.05 * (1.0 - 0.82 * u);
                    let (s, c) = a.sin_cos();
                    let (x, y) = (0.012 + c * r, -0.25 + s * r);
                    v.push(if j == 0 { M(x, y) } else { L(x, y) });
                }
                v.extend_from_slice(&[M(-0.032, -0.268), L(-0.085, -0.207), L(-0.008, -0.2), O]);
                let mark = p.shape("sh_leaf_mark", |p| build_path(p.f, &v))?;
                p.stroke(tr(0.0, 0.004) * h, &mark, WHITE, 0.55, 0.012);
                p.stroke(h, &mark, [0.24, 0.26, 0.32], 1.0, 0.012);
                p.seg(h, -0.15, -0.306, 0.11, -0.306, WHITE, 0.7, 0.008);
            }
        }
        // Red cloud cloak: a rounded high collar wrapping under the cheeks,
        // lined in red, with two little red clouds.
        (1, Mid) => {
            let red = [0.86, 0.13, 0.17];
            let edge_top = [M(-0.55, 0.12), Q(-0.4, 0.36, 0.0, 0.41), Q(0.4, 0.36, 0.55, 0.12)];
            let mut v = edge_top.to_vec();
            v.extend_from_slice(&[Q(0.64, 0.46, 0.38, 0.59), Q(0.0, 0.67, -0.38, 0.59), Q(-0.64, 0.46, -0.55, 0.12), Z]);
            let collar = p.path("sh_cloak", &v)?;
            p.at(h);
            let b = p.linear(0.0, 0.12, 0.0, 0.66, &[(0.0, [0.32, 0.3, 0.36], 1.0), (1.0, [0.1, 0.09, 0.12], 1.0)])?;
            p.fill_with(h, &collar, &b);
            let mut l = edge_top.to_vec();
            l.push(O);
            let lining = p.path("sh_cloak_lining", &l)?;
            p.stroke(h, &lining, red, 1.0, 0.04);
            p.stroke(h, &collar, [0.42, 0.4, 0.46], 1.0, lw);
            let cloud = p.path("sh_cloud", CLOUD)?;
            for (x, y, k, deg) in [(-0.34, 0.56, 0.15, -6.0f32), (0.35, 0.55, 0.14, 8.0)] {
                let m = put(x, y, k, deg) * h;
                p.fill(m, &cloud, red, 1.0);
                p.stroke(m, &cloud, WHITE, 1.0, 0.02 / k);
            }
        }
        // Leader's hat: a wide conical hat with a red crown and a flame emblem;
        // white veils hang down behind.
        (2, Back) => {
            let veil = p.path("sh_veil", &[M(0.3, -0.3), Q(0.62, -0.2, 0.66, 0.34), L(0.46, 0.3), Q(0.44, -0.05, 0.3, -0.3), Z])?;
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
            let cone = p.path("sh_kage_cone", &[M(-0.5, -0.31), Q(-0.24, -0.5, 0.0, -0.78), Q(0.24, -0.5, 0.5, -0.31), Q(0.0, -0.24, -0.5, -0.31), Z])?;
            p.at(f);
            let shade = p.linear(-0.5, 0.0, 0.5, 0.0, &[(0.0, [0.86, 0.85, 0.84], 1.0), (0.4, white, 1.0), (1.0, [0.82, 0.81, 0.8], 1.0)])?;
            p.fill_with(f, &cone, &shade);
            let crown = p.clip("sh_kage_crown", &cone, |p| build_path(p.f, &[M(-0.6, -0.56), L(0.6, -0.56), L(0.6, -0.9), L(-0.6, -0.9), Z]))?;
            p.fill(f, &crown, red, 1.0);
            p.stroke(f, &cone, [0.55, 0.55, 0.58], 1.0, lw);
            // Straw weave lines.
            for x in [-0.3f32, -0.15, 0.15, 0.3] {
                p.seg(f, x * 0.9, -0.33, x * 0.4, -0.55, [0.7, 0.68, 0.66], 0.5, 0.01);
            }
            // The emblem: a red diamond plate with a white flame.
            let plate = p.path("sh_kage_plate", &[M(0.0, -0.52), L(0.1, -0.42), L(0.0, -0.32), L(-0.1, -0.42), Z])?;
            p.obj(f, 1.0, &plate, red);
            let flame = p.path("sh_flame", FLAME)?;
            let k = 0.075 + 0.005 * (t * 6.0).sin();
            p.fill(Matrix3x2::scale(k * 0.8, k) * tr(0.0, -0.41) * f, &flame, white, 1.0);
        }
        // Spiral sphere: a swirling ball of energy held up beside the face.
        (3, Front) => {
            let (x, y, r) = (0.5, 0.18, 0.17);
            let blue = [0.35, 0.72, 1.0];
            p.glow(h, x, y, r * 2.0, r * 2.0, blue, 0.55)?;
            p.at(h);
            let ball = p.radial(x - 0.03, y - 0.03, r, r, &[(0.0, WHITE, 1.0), (0.35, [0.75, 0.92, 1.0], 1.0), (1.0, [0.2, 0.55, 1.0], 0.85)], 1.0)?;
            unsafe { p.rt.FillEllipse(&super::ell(x, y, r, r), &ball) };
            let arc = p.path("sh_swirl", &[M(0.0, -0.95), Q(0.9, -0.85, 0.85, 0.1), O])?;
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
        // A three-pronged kunai floating beside the head, a seal tag on its grip.
        (4, Front) => {
            let k = 0.56;
            let bob = (t * 2.0).sin() * 0.03;
            let m = put(-0.68, -0.08 + bob, k, -18.0 + (t * 1.3).sin() * 6.0) * h;
            let kunai = p.path("sh_kunai", KUNAI)?;
            p.at(m);
            let steel = p.linear(-0.2, 0.0, 0.2, 0.0, &[(0.0, STEEL_LO, 1.0), (0.5, STEEL_HI, 1.0), (1.0, STEEL, 1.0)])?;
            p.fill_with(m, &kunai, &steel);
            p.stroke(m, &kunai, [0.25, 0.27, 0.33], 1.0, lw / k);
            p.seg(m, 0.0, -0.58, 0.0, -0.16, WHITE, 0.6, 0.016);
            p.fill(m, &p.path("sh_kunai_grip", &rrect(-0.05, -0.08, 0.05, 0.28, 0.02))?, [0.2, 0.18, 0.24], 1.0);
            for j in 0..5 {
                let y = -0.05 + j as f32 * 0.07;
                p.seg(m, -0.05, y, 0.05, y + 0.04, [0.42, 0.4, 0.46], 1.0, 0.016);
            }
            p.ring(m, 0.0, 0.37, 0.08, 0.08, [0.3, 0.32, 0.38], 1.0, 0.035);
            let tag = p.path("sh_tag", &[M(0.05, 0.02), L(0.32, 0.06), L(0.3, 0.2), L(0.05, 0.16), Z])?;
            let tm = rot((t * 5.0).sin() * 8.0, 0.05, 0.09) * m;
            p.obj(tm, k, &tag, [0.96, 0.92, 0.8]);
            p.seg(tm, 0.11, 0.1, 0.27, 0.12, [0.7, 0.16, 0.18], 1.0, 0.02);
            let u = (t * 0.7).fract();
            if u < 0.18 {
                let a = (u / 0.18 * PI).sin();
                p.glow(m, 0.0, -0.62, 0.3 * a, 0.3 * a, [1.0, 0.9, 0.3], 0.7)?;
                p.spark(m, 0.0, -0.62, 0.16 * a, WHITE, 1.0)?;
            }
        }
        // Cat mask worn on the side of the head: a white cat face with red marks.
        (5, Front) => {
            let k = 0.44;
            let f = put(0.31, -0.25, k, 22.0) * h;
            let white = [0.98, 0.98, 0.97];
            let red = [0.84, 0.12, 0.16];
            let face = p.shape("sh_catmask", |p| build_path(p.f, &super::sym(&[M(0.0, -0.26), B(0.2, -0.27, 0.34, -0.16, 0.36, 0.04), B(0.36, 0.24, 0.16, 0.42, 0.0, 0.44)])))?;
            let ear_r = [M(0.12, -0.2), L(0.32, -0.46), L(0.35, -0.05), Z];
            let ears = p.shape("sh_catmask_ears", |p| build_path(p.f, &[ear_r.to_vec(), mirror(&ear_r)].concat()))?;
            p.obj(f, k, &ears, white);
            p.obj(f, k, &face, white);
            for side in [-1.0f32, 1.0] {
                let eye = rot(-14.0 * side, 0.14 * side, 0.02) * f;
                p.fill_el(eye, 0.14 * side, 0.02, 0.07, 0.035, INK, 1.0);
                p.seg(f, 0.08 * side, -0.14, 0.22 * side, -0.08, red, 1.0, 0.04);
                p.seg(f, 0.2 * side, 0.14, 0.3 * side, 0.2, red, 1.0, 0.035);
                p.fill_el(f, 0.25 * side, -0.33, 0.035, 0.06, red, 0.9);
            }
            let fore = p.path("sh_catmask_fore", &[M(0.0, -0.24), L(0.05, -0.12), L(0.0, -0.06), L(-0.05, -0.12), Z])?;
            p.fill(f, &fore, red, 1.0);
            p.fill_el(f, 0.0, 0.26, 0.04, 0.025, INK, 1.0);
        }
        // A round clan fan carried on the back: red top, white bottom, wooden handle.
        (6, Back) => {
            let k = 0.78;
            let f = put(-0.56, -0.14, k, -24.0) * h;
            p.seg(f, 0.0, 0.05, 0.0, 0.6, [0.42, 0.28, 0.18], 1.0, 0.075);
            p.seg(f, 0.0, 0.05, 0.0, 0.6, [0.72, 0.52, 0.34], 1.0, 0.04);
            p.fill_el(f, 0.0, 0.62, 0.05, 0.03, [0.42, 0.28, 0.18], 1.0);
            p.fill_el(f, 0.0, -0.18, 0.27, 0.3, WHITE, 1.0);
            let disc = p.shape("sh_fan_disc", |p| p.ellipse_geo(0.0, -0.18, 0.27, 0.3))?;
            let top = p.clip("sh_fan_top", &disc, |p| build_path(p.f, &[M(-0.4, -0.6), L(0.4, -0.6), L(0.4, -0.15), Q(0.0, -0.23, -0.4, -0.15), Z]))?;
            p.at(f);
            let red = p.linear(0.0, -0.48, 0.0, -0.15, &[(0.0, [0.95, 0.24, 0.26], 1.0), (1.0, [0.74, 0.06, 0.1], 1.0)])?;
            p.fill_with(f, &top, &red);
            p.ring(f, 0.0, -0.18, 0.27, 0.3, [0.4, 0.06, 0.1], 1.0, lw / k);
            p.ring(f, 0.0, -0.18, 0.24, 0.27, WHITE, 0.35, 0.012);
            p.seg(f, 0.0, 0.12, 0.0, 0.2, [0.4, 0.06, 0.1], 1.0, lw / k);
        }
        // Spirit armor: a glowing guardian's shoulder guards behind the fox,
        // flickering softly.
        (7, Back) => {
            let c = [0.6, 0.5, 1.0];
            let hi = mix(c, WHITE, 0.6);
            let flick = 0.86 + 0.14 * (t * 6.0).sin() * (t * 2.7).cos();
            let body = Matrix3x2::identity();
            p.glow(body, 0.0, 0.0, 1.0, 0.92, hi, 0.45 * flick)?;
            let pad = p.path("sh_arm_pad", &[M(0.4, 0.02), Q(0.56, -0.22, 0.82, -0.12), Q(0.98, -0.02, 0.9, 0.2), Q(0.74, 0.3, 0.46, 0.22), Z])?;
            for s in [Matrix3x2::identity(), flip()] {
                p.at(s);
                let g = p.linear(0.6, -0.2, 0.7, 0.2, &[(0.0, mix(hi, WHITE, 0.3), 0.95), (1.0, c, 0.8)])?;
                p.fill_with(s, &pad, &g);
                p.stroke(s, &pad, hi, 0.95 * flick, 0.024);
            }
        }
        // Sage marks: orange-red pigment above and below the eyes.
        (8, Front) => {
            let orange = [0.86, 0.24, 0.1];
            let upper = p.path("sh_sage_up", &[M(0.075, -0.04), Q(0.185, -0.17, 0.315, -0.035), Q(0.185, -0.095, 0.075, -0.04), Z])?;
            let lower = p.path("sh_sage_low", &[M(0.1, 0.178), Q(0.19, 0.25, 0.29, 0.178), Q(0.19, 0.207, 0.1, 0.178), Z])?;
            let pulse = 0.88 + 0.12 * (t * 2.0).sin();
            for s in [Matrix3x2::identity(), flip()] {
                p.fill(s * h, &upper, orange, pulse);
                p.stroke(s * h, &upper, mix(orange, INK, 0.4), 0.6 * pulse, 0.008);
                p.fill(s * h, &lower, mix(orange, [0.8, 0.16, 0.1], 0.4), pulse);
            }
            p.glow(h, 0.0, 0.05, 0.44, 0.16, orange, 0.18 * pulse)?;
        }
        // A big summoning scroll strapped across the back.
        (9, Back) => {
            let f = rot(-30.0, 0.0, 0.0) * tr(0.0, -0.1) * h;
            let paper = [0.96, 0.92, 0.82];
            let body = p.path("sh_scroll", &rrect(-0.78, -0.09, 0.78, 0.09, 0.08))?;
            p.at(f);
            let b = p.linear(0.0, -0.09, 0.0, 0.09, &[(0.0, WHITE, 1.0), (0.5, paper, 1.0), (1.0, mix(paper, INK, 0.2), 1.0)])?;
            p.fill_with(f, &body, &b);
            p.stroke(f, &body, edge(paper), 1.0, lw);
            for x in [-0.4f32, 0.4] {
                p.fill(f, &p.path(if x < 0.0 { "sh_scroll_band_l" } else { "sh_scroll_band_r" }, &rrect(x - 0.05, -0.095, x + 0.05, 0.095, 0.01))?, [0.8, 0.16, 0.2], 1.0);
            }
            for x in [-0.78f32, 0.78] {
                p.obj_el(f, x, 0.0, 0.045, 0.1, [0.5, 0.34, 0.22]);
            }
        }
        // Two shuriken orbiting the head (behind it on the far side).
        (10, Back) | (10, Front) => {
            let star = p.shape("sh_shuriken", |p| build_path(p.f, &shuriken()))?;
            for j in 0..2 {
                let ang = t * 1.5 + j as f32 * PI;
                if (ang.sin() > 0.0) != (layer == Front) {
                    continue;
                }
                let (x, y) = (ang.cos() * 0.6, -0.12 + ang.sin() * 0.16);
                let k = 0.13 * (1.0 + 0.15 * ang.sin());
                let m = put(x, y, k, t * 540.0) * h;
                p.at(m);
                let steel = p.linear(-1.0, -1.0, 1.0, 1.0, &[(0.0, STEEL_HI, 1.0), (1.0, STEEL_LO, 1.0)])?;
                p.fill_with(m, &star, &steel);
                p.stroke(m, &star, [0.25, 0.27, 0.32], 1.0, lw / k);
            }
        }
        // A sand gourd on the back, a red sash round its waist.
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
            p.obj(f, 1.0, &p.path("sh_gourd_cork", &rrect(-0.06, -0.6, 0.06, -0.5, 0.02))?, [0.9, 0.86, 0.78]);
            p.obj(f, 1.0, &p.path("sh_gourd_sash", &rrect(-0.31, -0.04, 0.31, 0.04, 0.02))?, [0.72, 0.18, 0.2]);
        }
        _ => {}
    }
    Ok(())
}

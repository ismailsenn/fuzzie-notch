//! Fuzzie's fox mascot: our own chubby, Bongo-style fox, drawn entirely in code
//! with Direct2D. It blinks, breathes, twitches its ears and swishes its tail.
//! Skins change colours, patterns and animated effects; accessories are worn on
//! top of any skin. Everything is free to use.
//!
//! Shapes are designed in "head units": the head is about 1 unit wide, centred
//! on (0, 0), y pointing down. [`draw`] scales them to the requested size.
//! Static shapes are built once per Direct2D factory and cached; animation is
//! done with transforms only.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::rc::Rc;

use windows::core::{Interface, Result};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::render::{luminance, rgba};

mod shinobi;
pub mod species;

pub use species::{Species, SpeciesInfo, SPECIES};

/// Complete visual identity of a mascot: species, skin, head accessory, body/gear accessory, and bot flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub species: usize,
    pub skin: usize,
    pub acc: usize,
    pub acc2: usize,
    /// Exclusive robotic visor & antenna look for the built-in Fuzzie Bot friend.
    pub bot: bool,
}

impl Look {
    pub const fn new(species: usize, skin: usize, acc: usize, acc2: usize) -> Self {
        Self { species, skin, acc, acc2, bot: false }
    }

    pub const fn bot() -> Self {
        // Graphite Fluffy Fox with exclusive robotic visor & antenna gear.
        Self { species: 0, skin: 2, acc: 0, acc2: 0, bot: true }
    }
}

pub fn species_get(i: usize) -> &'static SpeciesInfo {
    species::get(i)
}

/// Which accessory slot an item belongs to (Bongo Cat-style dual slots).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccSlot {
    /// Hats, headbands, glasses, horns, crowns, masks and face marks.
    Head,
    /// Wings, cloaks, scarves, rings, armor, scrolls and floating gear.
    Gear,
}

/// Classifies each accessory index into Head or Gear (index 0 "None" fits either).
pub fn acc_slot(i: usize) -> AccSlot {
    match i % ACCESSORIES.len() {
        8 | 9 | 13 | 16 | 28 | 30 | 31 | 33 | 34 | 36 | 37 | 38 => AccSlot::Gear,
        _ => AccSlot::Head,
    }
}

pub fn acc_count(slot: AccSlot) -> usize {
    (1..ACCESSORIES.len()).filter(|&i| acc_slot(i) == slot).count()
}

type Rgb = [f32; 3];

// ---------------------------------------------------------------- skins

/// Marks painted on the fur.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pattern {
    Plain,
    Gradient,
    Stripes,
    Spots,
    Patch,
    Mask,
    Galaxy,
    Star,
    Hearts,
    Calico,
    Kitsune,
    Rainbow,
}

/// Animated effect around the fox.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fx {
    None,
    Twinkle,
    Glow,
    Snow,
    Embers,
    Hearts,
    Bubbles,
    Petals,
    Leaves,
    Stars,
    /// Flame-like aura licking up around the fox.
    Aura,
    /// Crackling electric bolts.
    Lightning,
    /// Yellow speed streaks and sudden flashes.
    Flash,
    /// Crows flying up past the fox, and loose feathers.
    Crows,
}

/// Eye style (patterned eyes are drawn over the eye colour).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Eye {
    Normal,
    /// Red with three turning commas.
    Tomoe,
    /// Red with a three-blade pinwheel.
    Pinwheel,
    /// Red with three petals crossing a ring.
    Petals,
    /// Pale violet rings.
    Ripple,
    /// Fiery orange with a slit pupil.
    Beast,
    /// Only the right eye (as seen) has the commas.
    OneTomoe,
}

/// Hair worn on the head (see `shinobi.rs` for the shapes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hair {
    None,
    /// Wild spikes all round.
    Spiky,
    /// Spikes with long locks framing the face.
    Flash,
    /// Swept back, long side bangs.
    Swept,
    /// Everything leaning to one side.
    Leaning,
    /// A huge spiky mane down to the shoulders.
    Mane,
    /// Short and tousled.
    Short,
    /// Long, straight, parted in the middle.
    Straight,
    /// Parted bangs, long side locks.
    Parted,
    /// Sharp upright spikes.
    Sharp,
}

/// Marks on the face.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    None,
    Whiskers,
    /// Cloth mask over the nose and mouth.
    Mask,
    RedMarks,
    TearLines,
    /// Beast markings, plus nine tails.
    Beast,
    /// Metal studs on the nose and ears.
    Studs,
}

pub struct Skin {
    pub name: &'static str,
    pub fur: Rgb,
    /// Second fur colour (gradient end, galaxy core, calico patch).
    pub fur2: Rgb,
    /// Inside of the ears.
    pub ear: Rgb,
    /// Face mask, paws and tail tip.
    pub cream: Rgb,
    pub eyes: Rgb,
    /// Pattern colour.
    pub mark: Rgb,
    pub pattern: Pattern,
    pub fx: Fx,
    pub fxc: Rgb,
    pub eye: Eye,
    pub hair: Hair,
    pub hairc: Rgb,
    /// Shine on the hair.
    pub sheen: Rgb,
    pub face: Face,
    pub facec: Rgb,
}

impl Skin {
    pub fn animated(&self) -> bool {
        self.fx != Fx::None || self.pattern == Pattern::Galaxy || !matches!(self.eye, Eye::Normal | Eye::Beast)
    }

    const fn grad(mut self, c: u32) -> Self {
        self.fur2 = rgb(c);
        self.pattern = Pattern::Gradient;
        self
    }
    const fn two(mut self, c: u32) -> Self {
        self.fur2 = rgb(c);
        self
    }
    const fn pat(mut self, p: Pattern, c: u32) -> Self {
        self.pattern = p;
        self.mark = rgb(c);
        self
    }
    const fn effect(mut self, f: Fx, c: u32) -> Self {
        self.fx = f;
        self.fxc = rgb(c);
        self
    }
    const fn eye(mut self, e: Eye) -> Self {
        self.eye = e;
        self
    }
    const fn hair(mut self, h: Hair, c: u32, sheen: u32) -> Self {
        self.hair = h;
        self.hairc = rgb(c);
        self.sheen = rgb(sheen);
        self
    }
    const fn face(mut self, f: Face, c: u32) -> Self {
        self.face = f;
        self.facec = rgb(c);
        self
    }
}

const fn rgb(hex: u32) -> Rgb {
    [((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0]
}

const fn sk(name: &'static str, fur: u32, ear: u32, cream: u32, eyes: u32) -> Skin {
    Skin { name, fur: rgb(fur), fur2: rgb(fur), ear: rgb(ear), cream: rgb(cream), eyes: rgb(eyes), mark: rgb(fur), pattern: Pattern::Plain, fx: Fx::None, fxc: rgb(0xFFFFFF), eye: Eye::Normal, hair: Hair::None, hairc: rgb(fur), sheen: rgb(fur), face: Face::None, facec: rgb(eyes) }
}

use Fx as F;
use Pattern as P;

/// All looks; the first 21 keep their old order so saved indices still match.
pub const SKINS: &[Skin] = &[
    sk("Ember", 0xF2782A, 0xFFC4B4, 0xFFF4E8, 0x2A1810),
    sk("Snowdrift", 0xF4F7FB, 0xFFD0DC, 0xFFFFFF, 0x2A3440).effect(F::Snow, 0xFFFFFF),
    sk("Graphite", 0x9AA1AA, 0xF2C4CE, 0xEEF0F2, 0x1E2126),
    sk("Midnight", 0x2C3E70, 0x8FA2E0, 0xD8E0F5, 0x0E1428).effect(F::Twinkle, 0xFFF3B0),
    sk("Cherry", 0xF6A9BC, 0xFFE0E8, 0xFFF3F6, 0x46202C).effect(F::Petals, 0xFFC7D6),
    sk("Moss", 0x7BAF5B, 0xE8F5C8, 0xF1F8E8, 0x1D2A16).effect(F::Leaves, 0x9BD26A),
    sk("Honey", 0xF6C64E, 0xFFE7A8, 0xFFF8E6, 0x3A2A0C).grad(0xE5962A).effect(F::Glow, 0xFFD36B),
    sk("Shadow", 0x2E2E36, 0x7A6A8A, 0x55555F, 0xF2D046).effect(F::Glow, 0x8A6BFF),
    sk("Lilac", 0xC2A6F0, 0xFFD9F2, 0xF8F2FF, 0x2E2148).grad(0xF2A9D8).effect(F::Twinkle, 0xFFFFFF),
    sk("Lagoon", 0x3FA7D6, 0xBDE9FF, 0xE8F7FF, 0x0F2C3D).effect(F::Bubbles, 0xC9F1FF),
    sk("Cocoa", 0x8B5A3C, 0xE9B9A0, 0xF3E2D3, 0x1E120B).pat(P::Spots, 0x5E3A24),
    sk("Peach", 0xFFB38A, 0xFFE0D0, 0xFFF3EC, 0x4A2617).effect(F::Hearts, 0xFF8FA3),
    sk("Lime", 0xB5E34B, 0xF0FFC0, 0xF7FFE6, 0x26330A).pat(P::Stripes, 0x86B52C),
    sk("Coral", 0xFF7A6B, 0xFFD0C8, 0xFFF0EE, 0x3D1410).grad(0xFF5C8D),
    sk("Nebula", 0x3A1E6E, 0xB58CFF, 0xE9DCFA, 0x140A26).pat(P::Galaxy, 0xFF6BD5).two(0x7B4FC0).effect(F::Twinkle, 0xFFE27A),
    sk("Storm", 0x5B6B82, 0xC3CEDE, 0xDDE3EC, 0x101419).pat(P::Stripes, 0x3E4A5C),
    sk("Dusk", 0xF2885C, 0xFFD3C0, 0xFFE9DC, 0x3A1424).grad(0x9A4A8A),
    sk("Glacier", 0xCDEFF8, 0xFFFFFF, 0xFFFFFF, 0x1D3E48).grad(0x8CCFE6).effect(F::Snow, 0xFFFFFF),
    sk("Rosewood", 0xB0485E, 0xF7B6C4, 0xF8DDE3, 0x2A0C13).pat(P::Hearts, 0xFF8FB0),
    sk("Copper", 0xC8733A, 0xF2C4A0, 0xF8E4D2, 0x2B160A).effect(F::Embers, 0xFFB050),
    sk("Pumpkin", 0xFF8C1A, 0xFFD0A0, 0xFFF1DF, 0x2E1600).pat(P::Stripes, 0xE06A00),
    sk("Tiger", 0xFF9A2E, 0xFFD6B0, 0xFFF6EA, 0x24140A).pat(P::Stripes, 0x2A1A14),
    sk("Snow Leopard", 0xE6E9EE, 0xF6C9D2, 0xFFFFFF, 0x3C6E8A).pat(P::Spots, 0x7A808A),
    sk("Dalmatian", 0xFAFAFA, 0xFFD2DC, 0xFFFFFF, 0x1E1E22).pat(P::Spots, 0x26262B),
    sk("Calico", 0xFFFDF8, 0xFFD2DC, 0xFFFFFF, 0x2C2016).pat(P::Calico, 0xF29A4A).two(0x3A3232),
    sk("Panda", 0xFAFAFA, 0xF4C6D0, 0xFFFFFF, 0x141418).pat(P::Patch, 0x26262C),
    sk("Bandit", 0x9EA3AD, 0xE8C6CE, 0xF2F2F4, 0x16161A).pat(P::Mask, 0x2E3038),
    sk("Aurora", 0x37C8B4, 0xD2FFF4, 0xEFFFFB, 0x0E2A30).grad(0x8A5CF0).effect(F::Glow, 0x6BFFE0),
    sk("Galaxy", 0x1A1446, 0x9C7CFF, 0xD9D2FF, 0x0A0820).pat(P::Galaxy, 0x4FA8FF).two(0x5A2CA0).effect(F::Stars, 0xFFF0A0),
    sk("Supernova", 0x2A0E3A, 0xFF9AD8, 0xFFE0F4, 0x16061E).pat(P::Galaxy, 0xFF7A3D).two(0xC2187A).effect(F::Glow, 0xFF6BB0),
    sk("Starlight", 0x22305E, 0xA8B8FF, 0xE6EBFF, 0x0C1230).pat(P::Star, 0xFFE27A).effect(F::Twinkle, 0xFFE27A),
    sk("Cotton Candy", 0xFFB6DA, 0xFFFFFF, 0xFFF6FB, 0x3A2440).grad(0x9AD8FF).effect(F::Hearts, 0xFF7AB8),
    sk("Mint Choco", 0x9FE8D0, 0xFFFFFF, 0xF2FFFA, 0x2A1A12).pat(P::Spots, 0x5A3A2A),
    sk("Lemonade", 0xFFE66B, 0xFFF7C8, 0xFFFDF0, 0x3A3010).grad(0xFFB84A).effect(F::Bubbles, 0xFFFFFF),
    sk("Phoenix", 0xFF4A2A, 0xFFD27A, 0xFFF0C8, 0x2A0A04).grad(0xFFC23A).effect(F::Embers, 0xFFB03A),
    sk("Sakura", 0xFFD2E0, 0xFFFFFF, 0xFFFFFF, 0x4A2A3A).pat(P::Hearts, 0xFF8FB4).effect(F::Petals, 0xFFB7CF),
    sk("Kitsune", 0xFBF8F2, 0xFFB0B8, 0xFFFFFF, 0xB01828).pat(P::Kitsune, 0xE0303A).effect(F::Glow, 0x9FE0FF),
    sk("Ghost", 0xE6E2FF, 0xFFFFFF, 0xFFFFFF, 0x3A3466).effect(F::Glow, 0xC8BFFF),
    sk("Rainbow", 0xFF6B6B, 0xFFFFFF, 0xFFFFFF, 0x2A1A30).pat(P::Rainbow, 0xFFFFFF).effect(F::Twinkle, 0xFFFFFF),
    sk("Golden", 0xFFD36B, 0xFFF1C0, 0xFFF9E8, 0x3A2808).grad(0xE0A020).effect(F::Twinkle, 0xFFF6C0),
    sk("Ocean", 0x2A6FDB, 0xA8D4FF, 0xE0F0FF, 0x0A1A3A).grad(0x1AC0D0).effect(F::Bubbles, 0xBFEFFF),
    sk("Forest", 0x4E8A3E, 0xD8F0B8, 0xEEF6E0, 0x14200E).pat(P::Stripes, 0x386A2C).effect(F::Leaves, 0x8CCB5A),
    sk("Lava", 0x2A1A1A, 0xFF9A5A, 0x4A3030, 0xFFB030).pat(P::Stripes, 0xFF5A1A).effect(F::Embers, 0xFF7A2A),
    sk("Frost", 0xE8F6FF, 0xFFFFFF, 0xFFFFFF, 0x2A5A8A).grad(0xB8DCFF).effect(F::Snow, 0xFFFFFF),
    sk("Bubblegum", 0xFF8FC8, 0xFFE0F0, 0xFFF4FA, 0x3A1030).effect(F::Bubbles, 0xFFD0EA),
    sk("Neon", 0x1A1A2A, 0x00F0FF, 0x2A2A40, 0x00F0FF).pat(P::Stripes, 0x00E0F0).effect(F::Glow, 0x00E0FF),
    sk("Melon", 0x9AE27A, 0xFFC0C8, 0xFFE0E4, 0x1A2A10).pat(P::Stripes, 0x5EB040),
    sk("Blueberry", 0x5A6CF0, 0xC8D0FF, 0xEEF0FF, 0x10142A).pat(P::Spots, 0x3A48C0),
    sk("Strawberry", 0xFF4A6A, 0xFFD0D8, 0xFFF4F6, 0x2A0A10).pat(P::Spots, 0xFFE87A),
    // Shinobi pack: light fur, the character carried by hair, eyes and marks.
    sk("Maelstrom", 0xF7962A, 0xFFCBA0, 0xFFF5E6, 0x2A86E8).hair(Hair::Spiky, 0xFFCB2E, 0xFFF3A6).face(Face::Whiskers, 0x7A4420).effect(F::Aura, 0xFFA23A),
    sk("Golden Flash", 0xF2C98E, 0xFFD9BC, 0xFFF8EC, 0x2F6FD0).hair(Hair::Flash, 0xFFD23F, 0xFFF6C0).effect(F::Flash, 0xFFD84A),
    sk("Avenger", 0xAFBBE6, 0xF2C2D6, 0xF7F8FF, 0xC8102A).hair(Hair::Swept, 0x22273D, 0x6A7CB0).eye(Eye::Tomoe).effect(F::Lightning, 0xA9C4FF),
    sk("Copycat", 0xAAB4C4, 0xF0C4D0, 0xF2F4F8, 0x262A33).hair(Hair::Leaning, 0xE4E8EF, 0xFFFFFF).eye(Eye::OneTomoe).face(Face::Mask, 0x2C3550).effect(F::Lightning, 0xBFE6FF),
    sk("Warlord", 0xDC8A90, 0xFFC6CC, 0xFFF4F4, 0xC0101E).hair(Hair::Mane, 0x1B1A24, 0x56608E).eye(Eye::Petals).effect(F::Aura, 0x6A86FF),
    sk("Tide Lord", 0x95C8EA, 0xD8F0FF, 0xF6FBFF, 0xB3172C).hair(Hair::Short, 0xE6EAF2, 0xFFFFFF).face(Face::RedMarks, 0xD42A40).effect(F::Bubbles, 0x9FD8FF),
    sk("Forest Lord", 0xA8C98A, 0xF0E6B8, 0xFAFDF2, 0x3A2414).hair(Hair::Straight, 0x34231A, 0x8A6248).effect(F::Leaves, 0x6CBF4A),
    sk("Crow", 0xA898BE, 0xEEC4DA, 0xFAF6FF, 0xC8102A).hair(Hair::Parted, 0x1C1A24, 0x5E5478).eye(Eye::Pinwheel).face(Face::TearLines, 0x8E7686).effect(F::Crows, 0x1E1C26),
    sk("Nine Tails", 0xF26A1B, 0xFFB070, 0xFFE2C0, 0xE8401A).grad(0xC8341A).eye(Eye::Beast).face(Face::Beast, 0x2A0E08).effect(F::Aura, 0xFF6A1A),
    sk("Six Paths", 0xC4B6E2, 0xFFD2C4, 0xFBF8FF, 0xA898D8).hair(Hair::Sharp, 0xF07A2E, 0xFFC890).eye(Eye::Ripple).face(Face::Studs, 0xB8BEC8).effect(F::Glow, 0xB8A8FF),
];

/// A themed set of skins and accessories, shown together in the pickers.
pub struct Pack {
    pub name: &'static str,
    /// Range in [`SKINS`].
    pub skins: std::ops::Range<usize>,
    /// Range in [`ACCESSORIES`] (index 0, "None", belongs to no pack).
    pub accs: std::ops::Range<usize>,
}

/// The packs, in picker order; together they cover every skin and accessory.
pub const PACKS: &[Pack] = &[
    Pack { name: "Classic", skins: 0..SHINOBI_SKIN, accs: 1..SHINOBI },
    Pack { name: "Shinobi Pack", skins: SHINOBI_SKIN..SKINS.len(), accs: SHINOBI..ACCESSORIES.len() },
];

/// The first shinobi skin.
const SHINOBI_SKIN: usize = 49;

pub fn get(i: usize) -> &'static Skin {
    &SKINS[i % SKINS.len()]
}

// ---------------------------------------------------------------- accessories

/// Accessory names and whether they animate; index 0 is "none".
pub const ACCESSORIES: &[(&str, bool)] = &[
    ("None", false),
    ("Headband", false),
    ("Ninja Band", true),
    ("Star Clip", true),
    ("Flower", true),
    ("Heart Boppers", true),
    ("Top Hat", false),
    ("Royal Crown", true),
    ("Astronaut", true),
    ("Planet Ring", true),
    ("Forest Antlers", true),
    ("Fox Mask", false),
    ("Peacock Crest", true),
    ("Angel Wings", true),
    ("Glasses", false),
    ("Bow", true),
    ("Scarf", true),
    ("Headphones", true),
    ("Halo", true),
    ("Sunglasses", true),
    ("Party Hat", true),
    ("Witch Hat", true),
    ("Flower Crown", true),
    ("Devil Horns", false),
    ("Sprout", true),
    ("Beanie", true),
    ("Knight Helmet", true),
    // Shinobi pack (drawn in mascot/shinobi.rs).
    ("Leaf Plate", true),
    ("Red Cloud Cloak", false),
    ("Leader's Hat", true),
    ("Spiral Sphere", true),
    ("Warp Kunai", true),
    ("Cat Mask", false),
    ("Clan Fan", false),
    ("Spirit Armor", true),
    ("Sage Marks", true),
    ("Summon Scroll", false),
    ("Shuriken Orbit", true),
    ("Sand Gourd", false),
];

/// The first shinobi accessory.
const SHINOBI: usize = 27;

/// The helmet covers the ears, so they are not drawn under it.
const HELMET: usize = 26;

pub fn acc_name(i: usize) -> &'static str {
    ACCESSORIES[i % ACCESSORIES.len()].0
}

// ---------------------------------------------------------------- paths

/// Path commands in head units.
#[derive(Clone, Copy, Debug, PartialEq)]
enum S {
    /// Move (starts a figure).
    M(f32, f32),
    L(f32, f32),
    /// Quadratic: control, end.
    Q(f32, f32, f32, f32),
    /// Cubic: control 1, control 2, end.
    B(f32, f32, f32, f32, f32, f32),
    /// Close the figure.
    Z,
    /// End the figure open (for strokes).
    O,
}
use S::*;

fn end_of(s: &S) -> (f32, f32) {
    match *s {
        M(x, y) | L(x, y) | Q(_, _, x, y) | B(_, _, _, _, x, y) => (x, y),
        Z | O => (0.0, 0.0),
    }
}

/// The right half of a symmetric outline (from a point on x = 0 down to
/// another point on x = 0) becomes the whole closed outline.
fn sym(half: &[S]) -> Vec<S> {
    let mut out = half.to_vec();
    let mut cur = end_of(&half[0]);
    let mut starts = Vec::new();
    for s in &half[1..] {
        starts.push(cur);
        cur = end_of(s);
    }
    for (s, st) in half[1..].iter().zip(starts).rev() {
        out.push(match *s {
            L(..) => L(-st.0, st.1),
            Q(a, b, _, _) => Q(-a, b, -st.0, st.1),
            B(a, b, c, d, _, _) => B(-c, d, -a, b, -st.0, st.1),
            other => other,
        });
    }
    out.push(Z);
    out
}

/// Mirror image across x = 0.
fn mirror(cmds: &[S]) -> Vec<S> {
    cmds.iter()
        .map(|s| match *s {
            M(x, y) => M(-x, y),
            L(x, y) => L(-x, y),
            Q(a, b, x, y) => Q(-a, b, -x, y),
            B(a, b, c, d, x, y) => B(-a, b, -c, d, -x, y),
            other => other,
        })
        .collect()
}

fn rrect(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Vec<S> {
    vec![M(x0 + r, y0), L(x1 - r, y0), Q(x1, y0, x1, y0 + r), L(x1, y1 - r), Q(x1, y1, x1 - r, y1), L(x0 + r, y1), Q(x0, y1, x0, y1 - r), L(x0, y0 + r), Q(x0, y0, x0 + r, y0), Z]
}

/// Five-point star of radius 1.
fn star5() -> Vec<S> {
    let mut v = Vec::new();
    for i in 0..10 {
        let r = if i % 2 == 0 { 1.0 } else { 0.45 };
        let a = -PI / 2.0 + i as f32 * PI / 5.0;
        let p = (a.cos() * r, a.sin() * r);
        v.push(if i == 0 { M(p.0, p.1) } else { L(p.0, p.1) });
    }
    v.push(Z);
    v
}

/// Zigzag spring from (0, 0) up to (0, -1).
fn spring() -> Vec<S> {
    let mut v = vec![M(0.0, 0.0)];
    for i in 1..8 {
        let x = if i % 2 == 0 { -0.12 } else { 0.12 };
        v.push(L(x, -(i as f32) / 8.0));
    }
    v.push(L(0.0, -1.0));
    v.push(O);
    v
}

const TAIL: &[S] = &[M(0.16, 0.40), B(0.42, 0.49, 0.67, 0.37, 0.68, 0.10), B(0.69, -0.02, 0.63, -0.10, 0.56, -0.13), B(0.58, 0.0, 0.52, 0.17, 0.28, 0.24), Z];
const NOSE: &[S] = &[M(-0.044, -0.014), Q(0.0, -0.024, 0.044, -0.014), Q(0.026, 0.022, 0.0, 0.026), Q(-0.026, 0.022, -0.044, -0.014), Z];
const MOUTH: &[S] = &[M(-0.064, 0.0), Q(-0.032, 0.042, 0.0, -0.002), Q(0.032, 0.042, 0.064, 0.0), O];
const MOUTH_HAPPY: &[S] = &[M(-0.068, -0.006), Q(-0.034, 0.046, 0.0, -0.002), Q(0.034, 0.046, 0.068, -0.006), O];
const MOUTH_OPEN: &[S] = &[M(-0.072, -0.006), Q(-0.036, 0.015, 0.0, -0.004), Q(0.036, 0.015, 0.072, -0.006), Q(0.0, 0.078, -0.072, -0.006), Z];
const LID: &[S] = &[M(-0.066, 0.004), Q(0.0, 0.066, 0.066, 0.004), O];
const HAPPY_EYE: &[S] = &[M(-0.066, 0.022), Q(0.0, -0.058, 0.066, 0.022), O];
const HAPPY_EYE_BIG: &[S] = &[M(-0.072, 0.028), Q(0.0, -0.076, 0.072, 0.028), O];
const ZED: &[S] = &[M(-0.5, -0.5), L(0.5, -0.5), L(-0.5, 0.5), L(0.5, 0.5), O];
const SPARK: &[S] = &[M(0.0, -1.0), Q(0.12, -0.12, 1.0, 0.0), Q(0.12, 0.12, 0.0, 1.0), Q(-0.12, 0.12, -1.0, 0.0), Q(-0.12, -0.12, 0.0, -1.0), Z];
const HEART: &[S] = &[M(0.0, 0.42), B(-0.62, 0.0, -0.42, -0.62, 0.0, -0.25), B(0.42, -0.62, 0.62, 0.0, 0.0, 0.42), Z];
const LEAF: &[S] = &[M(-0.5, 0.0), Q(0.0, -0.42, 0.5, 0.0), Q(0.0, 0.42, -0.5, 0.0), Z];
const BAND: &[S] = &[M(-0.6, -0.25), B(-0.2, -0.31, 0.2, -0.31, 0.6, -0.25), L(0.6, -0.155), B(0.2, -0.215, -0.2, -0.215, -0.6, -0.155), Z];

/// One geometry covering all the outlines in `parts` (at least one).
fn union_paths(f: &ID2D1Factory, parts: &[Vec<S>]) -> Result<ID2D1Geometry> {
    let mut acc = build_path(f, &parts[0])?;
    for part in &parts[1..] {
        let b = build_path(f, part)?;
        unsafe {
            let geo = f.CreatePathGeometry()?;
            let sink = geo.Open()?;
            acc.CombineWithGeometry(&b, D2D1_COMBINE_MODE_UNION, None, 0.0004, &sink)?;
            sink.Close()?;
            acc = geo.cast()?;
        }
    }
    Ok(acc)
}

fn build_path(f: &ID2D1Factory, cmds: &[S]) -> Result<ID2D1Geometry> {
    let v = |x: f32, y: f32| Vector2 { X: x, Y: y };
    unsafe {
        let geo = f.CreatePathGeometry()?;
        let sink = geo.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let mut open = false;
        for c in cmds {
            match *c {
                M(x, y) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    }
                    sink.BeginFigure(v(x, y), D2D1_FIGURE_BEGIN_FILLED);
                    open = true;
                }
                L(x, y) => sink.AddLine(v(x, y)),
                Q(a, b, x, y) => sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: v(a, b), point2: v(x, y) }),
                B(a, b, c, d, x, y) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v(a, b), point2: v(c, d), point3: v(x, y) }),
                Z => {
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    open = false;
                }
                O => {
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    open = false;
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        sink.Close()?;
        geo.cast()
    }
}

// ---------------------------------------------------------------- maths

fn tr(x: f32, y: f32) -> Matrix3x2 {
    Matrix3x2::translation(x, y)
}

/// Clockwise rotation by `deg` around (x, y).
fn rot(deg: f32, x: f32, y: f32) -> Matrix3x2 {
    let (s, c) = deg.to_radians().sin_cos();
    Matrix3x2 { M11: c, M12: s, M21: -s, M22: c, M31: x - c * x + s * y, M32: y - s * x - c * y }
}

/// Scale `s`, rotate `deg`, then move to (x, y): places a unit shape.
fn put(x: f32, y: f32, s: f32, deg: f32) -> Matrix3x2 {
    Matrix3x2::scale(s, s) * rot(deg, 0.0, 0.0) * tr(x, y)
}

fn flip() -> Matrix3x2 {
    Matrix3x2::scale(-1.0, 1.0)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

const WHITE: Rgb = [1.0, 1.0, 1.0];
const INK: Rgb = [0.10, 0.07, 0.10];

/// Outline colour for an accessory part.
fn edge(c: Rgb) -> Rgb {
    mix(c, INK, 0.58)
}

/// 0 open .. 1 closed; a blink every few seconds, sometimes a double one.
fn blink(t: f32) -> f32 {
    let period = 4.3;
    let u = t.rem_euclid(period);
    let d = if u < 0.16 {
        u
    } else if (0.3..0.46).contains(&u) && (t / period).floor() as i64 % 3 == 0 {
        u - 0.3
    } else {
        return 0.0;
    };
    1.0 - (d / 0.08 - 1.0).abs()
}

/// Which ear twitches (1 right, -1 left) and by how many degrees.
fn twitch(t: f32) -> (f32, f32) {
    let period = 5.7;
    let u = t.rem_euclid(period);
    let side = if (t / period).floor() as i64 % 2 == 0 { 1.0 } else { -1.0 };
    if u < 0.4 {
        let k = u / 0.4;
        (side, (k * PI * 3.0).sin() * (1.0 - k) * 16.0)
    } else {
        (side, 0.0)
    }
}

// ---------------------------------------------------------------- the pen

struct Store {
    _factory: ID2D1Factory,
    stroke: ID2D1StrokeStyle,
    shapes: RefCell<HashMap<(u8, &'static str), ID2D1Geometry>>,
}

thread_local! {
    static STORES: RefCell<Vec<(usize, Rc<Store>)>> = const { RefCell::new(Vec::new()) };
    /// Gradient brushes keyed by (render target, colour stops).
    static BRUSHES: RefCell<HashMap<(usize, u64), ID2D1Brush>> = RefCell::new(HashMap::new());
    /// One solid brush per render target, recoloured for every shape.
    static SOLIDS: RefCell<Vec<(usize, ID2D1SolidColorBrush)>> = const { RefCell::new(Vec::new()) };
}

fn solid_for(rt: &ID2D1RenderTarget) -> Result<ID2D1SolidColorBrush> {
    let key = rt.as_raw() as usize;
    if let Some(b) = SOLIDS.with(|v| v.borrow().iter().find(|e| e.0 == key).map(|e| e.1.clone())) {
        return Ok(b);
    }
    let b = unsafe { rt.CreateSolidColorBrush(&rgba(WHITE, 1.0), None)? };
    SOLIDS.with(|v| v.borrow_mut().push((key, b.clone())));
    Ok(b)
}

#[derive(Clone, Copy)]
pub struct BongoState {
    /// How far each paw (left, right) is tapped down, 0..1.
    pub paws: [f32; 2],
    /// Eyelids, 0 open .. 1 closed.
    pub blink: f32,
    /// Keeps the idle motions (breathing, tail, effects) running.
    pub live: bool,
    /// Time for the floating "zZZ", which drifts even when the fox holds still.
    pub zt: f32,
    /// Draws the glows of skins and accessories.
    pub glow: bool,
    /// Low-pass smoothed body squash (0..1) on paw tap.
    pub squash: f32,
    /// Low-pass smoothed subtle head tilt (-1..1).
    pub head_bob: f32,
    /// Gentle tap ear bounce in degrees (left, right).
    pub ear_bounce: [f32; 2],
    /// Typing speed happiness (0..1; >0.28 warm ^ ^, >0.58 joyful open smile).
    pub hype: f32,
    /// Smooth sleep/wake blend (0 awake .. 1 deeply asleep).
    pub sleep_mix: f32,
    /// Smooth yarn-ball play blend (0 upright .. 1 lying on back batting yarn).
    pub yarn_mix: f32,
    /// Yarn ball state in mascot units: [x, y, rotation_deg, squash].
    pub yarn_ball: [f32; 4],
}

#[derive(Clone, Copy)]
struct Bongo {
    paws: Option<[f32; 2]>,
    blink: f32,
    live: bool,
    zt: f32,
    glow: bool,
    head_only: bool,
    squash: f32,
    head_bob: f32,
    ear_bounce: [f32; 2],
    hype: f32,
    sleep_mix: f32,
    yarn_mix: f32,
    yarn_ball: [f32; 4],
}

/// The fox being drawn has its glows turned off.
fn glow_off() -> bool {
    BONGO.with(|c| c.get()).is_some_and(|b| !b.glow)
}

thread_local! {
    // Set while the desktop mascot draws: pose, paws, squash, hype and eyes.
    static BONGO: std::cell::Cell<Option<Bongo>> = const { std::cell::Cell::new(None) };
    // What [`fox`] draws: 1 the mascot itself, 2 accessory parts behind it, 4 accessory parts on it.
    static LAYERS: std::cell::Cell<u8> = const { std::cell::Cell::new(7) };
}

/// The desktop mascot: articulated Bongo Cat paws, smooth squash/ear bounce, sleep/wake & yarn play.
#[allow(clippy::too_many_arguments)]
pub fn draw_bongo(rt: &ID2D1RenderTarget, factory: &ID2D1Factory, look: Look, cx: f32, cy: f32, size: f32, sleeping: bool, st: BongoState) -> Result<()> {
    BONGO.with(|c| {
        c.set(Some(Bongo {
            paws: Some(st.paws),
            blink: st.blink,
            live: st.live,
            zt: st.zt,
            glow: st.glow,
            head_only: false,
            squash: st.squash,
            head_bob: st.head_bob,
            ear_bounce: st.ear_bounce,
            hype: st.hype,
            sleep_mix: st.sleep_mix,
            yarn_mix: st.yarn_mix,
            yarn_ball: st.yarn_ball,
        }))
    });
    let r = draw(rt, factory, look, cx, cy, size, 1.0, sleeping, st.zt);
    BONGO.with(|c| c.set(None));
    r
}

/// Just the head, still, no tail or paws (kept for small head-only icons if needed).
#[allow(dead_code, clippy::too_many_arguments)]
pub fn draw_head(rt: &ID2D1RenderTarget, factory: &ID2D1Factory, species: usize, skin: usize, cx: f32, cy: f32, size: f32) -> Result<()> {
    BONGO.with(|c| {
        c.set(Some(Bongo {
            paws: None,
            blink: 0.0,
            live: false,
            zt: 0.0,
            glow: true,
            head_only: true,
            squash: 0.0,
            head_bob: 0.0,
            ear_bounce: [0.0; 2],
            hype: 0.0,
            sleep_mix: 0.0,
            yarn_mix: 0.0,
            yarn_ball: [0.0; 4],
        }))
    });
    let r = draw(rt, factory, Look::new(species, skin, 0, 0), cx, cy, size, 1.0, false, 0.0);
    BONGO.with(|c| c.set(None));
    r
}

/// One frame of the idle animation at `t` seconds, without the per-mascot
/// rhythm, drawing only `layers` (1 the mascot, 2 accessory parts behind it,
/// 4 accessory parts on it). Used to export frames for the web page.
#[allow(clippy::too_many_arguments)]
pub fn draw_layers(rt: &ID2D1RenderTarget, factory: &ID2D1Factory, look: Look, cx: f32, cy: f32, size: f32, t: f32, layers: u8) -> Result<()> {
    let phase = ((look.skin % SKINS.len()) as f32 * 0.618 + (look.acc % ACCESSORIES.len()) as f32 * 0.377).fract() * 11.0;
    LAYERS.with(|c| c.set(layers));
    let r = draw(rt, factory, look, cx, cy, size, 1.0, false, t - phase);
    LAYERS.with(|c| c.set(7));
    r
}

#[allow(clippy::too_many_arguments)]
pub fn draw_still(rt: &ID2D1RenderTarget, factory: &ID2D1Factory, look: Look, cx: f32, cy: f32, size: f32, a: f32, sleeping: bool, blink: f32) -> Result<()> {
    BONGO.with(|c| {
        c.set(Some(Bongo {
            paws: None,
            blink,
            live: false,
            zt: 0.0,
            glow: true,
            head_only: false,
            squash: 0.0,
            head_bob: 0.0,
            ear_bounce: [0.0; 2],
            hype: 0.0,
            sleep_mix: if sleeping { 1.0 } else { 0.0 },
            yarn_mix: 0.0,
            yarn_ball: [0.0; 4],
        }))
    });
    let r = draw(rt, factory, look, cx, cy, size, a, sleeping, 0.0);
    BONGO.with(|c| c.set(None));
    r
}

/// Forgets the brushes made for a render target and the shapes made with a
/// factory that are about to go away, so they do not keep them alive.
pub fn release(rt: &impl Interface, factory: &ID2D1Factory) {
    let (rt, f) = (rt.as_raw() as usize, factory.as_raw() as usize);
    BRUSHES.with(|m| m.borrow_mut().retain(|k, _| k.0 != rt));
    SOLIDS.with(|v| v.borrow_mut().retain(|e| e.0 != rt));
    STORES.with(|v| v.borrow_mut().retain(|e| e.0 != f));
}

fn store(f: &ID2D1Factory) -> Result<Rc<Store>> {
    let key = f.as_raw() as usize;
    if let Some(s) = STORES.with(|v| v.borrow().iter().find(|e| e.0 == key).map(|e| e.1.clone())) {
        return Ok(s);
    }
    let props = D2D1_STROKE_STYLE_PROPERTIES {
        startCap: D2D1_CAP_STYLE_ROUND,
        endCap: D2D1_CAP_STYLE_ROUND,
        dashCap: D2D1_CAP_STYLE_ROUND,
        lineJoin: D2D1_LINE_JOIN_ROUND,
        miterLimit: 1.0,
        dashStyle: D2D1_DASH_STYLE_SOLID,
        dashOffset: 0.0,
    };
    let stroke = unsafe { f.CreateStrokeStyle(&props, None)? };
    let s = Rc::new(Store { _factory: f.clone(), stroke, shapes: RefCell::new(HashMap::new()) });
    STORES.with(|v| {
        let mut v = v.borrow_mut();
        if v.len() >= 4 {
            v.remove(0);
        }
        v.push((key, s.clone()));
    });
    Ok(s)
}

struct Pen<'a> {
    rt: &'a ID2D1RenderTarget,
    f: &'a ID2D1Factory,
    st: Rc<Store>,
    brush: ID2D1SolidColorBrush,
    /// Head units to device, including the caller's transform.
    base: Matrix3x2,
    a: f32,
    /// Outline width in head units.
    lw: f32,
    /// The mascot's outline colour.
    line: Rgb,
    /// Drawn at a small size: skip fine details.
    small: bool,
    /// Active species (for species-aware cached head/pattern clipping).
    sp: Species,
}

fn ell(x: f32, y: f32, rx: f32, ry: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: rx, radiusY: ry }
}

impl Pen<'_> {
    fn at(&self, m: Matrix3x2) {
        unsafe { self.rt.SetTransform(&(m * self.base)) }
    }

    fn solid(&self, c: Rgb, a: f32) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&rgba(c, a * self.a)) };
        &self.brush
    }

    fn fill(&self, m: Matrix3x2, g: &ID2D1Geometry, c: Rgb, a: f32) {
        self.at(m);
        unsafe { self.rt.FillGeometry(g, self.solid(c, a), None) }
    }

    fn fill_with(&self, m: Matrix3x2, g: &ID2D1Geometry, b: &ID2D1Brush) {
        self.at(m);
        unsafe { self.rt.FillGeometry(g, b, None) }
    }

    fn stroke(&self, m: Matrix3x2, g: &ID2D1Geometry, c: Rgb, a: f32, w: f32) {
        self.at(m);
        unsafe { self.rt.DrawGeometry(g, self.solid(c, a), w, &self.st.stroke) }
    }

    /// Filled, with a darker outline; `k` is the scale inside `m`.
    fn obj(&self, m: Matrix3x2, k: f32, g: &ID2D1Geometry, c: Rgb) {
        self.fill(m, g, c, 1.0);
        self.stroke(m, g, edge(c), 1.0, self.lw * 0.85 / k);
    }

    fn fill_el(&self, m: Matrix3x2, x: f32, y: f32, rx: f32, ry: f32, c: Rgb, a: f32) {
        self.at(m);
        unsafe { self.rt.FillEllipse(&ell(x, y, rx, ry), self.solid(c, a)) }
    }

    #[allow(clippy::too_many_arguments)]
    fn ring(&self, m: Matrix3x2, x: f32, y: f32, rx: f32, ry: f32, c: Rgb, a: f32, w: f32) {
        self.at(m);
        unsafe { self.rt.DrawEllipse(&ell(x, y, rx, ry), self.solid(c, a), w, &self.st.stroke) }
    }

    fn obj_el(&self, m: Matrix3x2, x: f32, y: f32, rx: f32, ry: f32, c: Rgb) {
        self.fill_el(m, x, y, rx, ry, c, 1.0);
        self.ring(m, x, y, rx, ry, edge(c), 1.0, self.lw * 0.85);
    }

    #[allow(clippy::too_many_arguments)]
    fn seg(&self, m: Matrix3x2, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgb, a: f32, w: f32) {
        self.at(m);
        unsafe { self.rt.DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.solid(c, a), w, &self.st.stroke) }
    }

    fn stops(&self, s: &[(f32, Rgb, f32)]) -> Result<ID2D1GradientStopCollection> {
        let v: Vec<D2D1_GRADIENT_STOP> = s.iter().map(|&(p, c, a)| D2D1_GRADIENT_STOP { position: p, color: rgba(c, a) }).collect();
        unsafe { self.rt.CreateGradientStopCollection(&v, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) }
    }

    /// Gradient brushes are costly to create (each is a texture), so they are
    /// kept per render target and colour stops; position and opacity are set per use.
    fn cached_brush(&self, kind: u8, s: &[(f32, Rgb, f32)], make: impl FnOnce(ID2D1GradientStopCollection) -> Result<ID2D1Brush>) -> Result<ID2D1Brush> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        kind.hash(&mut h);
        for (p, c, a) in s {
            (p.to_bits(), c.map(f32::to_bits), a.to_bits()).hash(&mut h);
        }
        let key = (self.rt.as_raw() as usize, h.finish());
        if let Some(b) = BRUSHES.with(|m| m.borrow().get(&key).cloned()) {
            return Ok(b);
        }
        let b = make(self.stops(s)?)?;
        BRUSHES.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() > 600 {
                let rt = key.0;
                m.retain(|k, _| k.0 != rt);
            }
            m.insert(key, b.clone());
        });
        Ok(b)
    }

    #[allow(clippy::too_many_arguments)]
    fn radial(&self, x: f32, y: f32, rx: f32, ry: f32, s: &[(f32, Rgb, f32)], alpha: f32) -> Result<ID2D1Brush> {
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: Vector2 { X: x, Y: y }, gradientOriginOffset: Vector2 { X: 0.0, Y: 0.0 }, radiusX: rx, radiusY: ry };
        let b = self.cached_brush(0, s, |stops| unsafe { Ok(self.rt.CreateRadialGradientBrush(&props, None, &stops)?.into()) })?;
        unsafe {
            let r: ID2D1RadialGradientBrush = b.cast()?;
            r.SetCenter(props.center);
            r.SetRadiusX(rx);
            r.SetRadiusY(ry);
            r.SetOpacity(self.a * alpha);
        }
        Ok(b)
    }

    fn linear(&self, x0: f32, y0: f32, x1: f32, y1: f32, s: &[(f32, Rgb, f32)]) -> Result<ID2D1Brush> {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: x0, Y: y0 }, endPoint: Vector2 { X: x1, Y: y1 } };
        let b = self.cached_brush(1, s, |stops| unsafe { Ok(self.rt.CreateLinearGradientBrush(&props, None, &stops)?.into()) })?;
        unsafe {
            let l: ID2D1LinearGradientBrush = b.cast()?;
            l.SetStartPoint(props.startPoint);
            l.SetEndPoint(props.endPoint);
            l.SetOpacity(self.a);
        }
        Ok(b)
    }

    /// A soft round glow that fades to nothing at its edge.
    #[allow(clippy::too_many_arguments)]
    fn glow(&self, m: Matrix3x2, x: f32, y: f32, rx: f32, ry: f32, c: Rgb, a: f32) -> Result<()> {
        if glow_off() {
            return Ok(());
        }
        self.at(m);
        let b = self.radial(x, y, rx, ry, &[(0.0, c, 1.0), (0.5, c, 0.45), (1.0, c, 0.0)], a)?;
        unsafe { self.rt.FillEllipse(&ell(x, y, rx, ry), &b) };
        Ok(())
    }

    fn shape(&self, key: &'static str, build: impl FnOnce(&Self) -> Result<ID2D1Geometry>) -> Result<ID2D1Geometry> {
        let k = (0u8, key);
        if let Some(g) = self.st.shapes.borrow().get(&k) {
            return Ok(g.clone());
        }
        let g = build(self)?;
        self.st.shapes.borrow_mut().insert(k, g.clone());
        Ok(g)
    }

    fn shape_sp(&self, key: &'static str, build: impl FnOnce(&Self) -> Result<ID2D1Geometry>) -> Result<ID2D1Geometry> {
        let k = (self.sp as u8 + 1, key);
        if let Some(g) = self.st.shapes.borrow().get(&k) {
            return Ok(g.clone());
        }
        let g = build(self)?;
        self.st.shapes.borrow_mut().insert(k, g.clone());
        Ok(g)
    }

    fn path(&self, key: &'static str, cmds: &[S]) -> Result<ID2D1Geometry> {
        self.shape(key, |p| build_path(p.f, cmds))
    }

    fn ellipse_geo(&self, x: f32, y: f32, rx: f32, ry: f32) -> Result<ID2D1Geometry> {
        unsafe { self.f.CreateEllipseGeometry(&ell(x, y, rx, ry))?.cast() }
    }

    /// Cached union of the outlines in `parts` (one clean outline to stroke).
    fn union(&self, key: &'static str, parts: impl FnOnce() -> Vec<Vec<S>>) -> Result<ID2D1Geometry> {
        self.shape(key, |p| union_paths(p.f, &parts()))
    }

    /// Cached intersection of `a` with the shape `b` builds.
    fn clip(&self, key: &'static str, a: &ID2D1Geometry, b: impl FnOnce(&Self) -> Result<ID2D1Geometry>) -> Result<ID2D1Geometry> {
        self.shape(key, |p| unsafe {
            let b = b(p)?;
            let geo = p.f.CreatePathGeometry()?;
            let sink = geo.Open()?;
            a.CombineWithGeometry(&b, D2D1_COMBINE_MODE_INTERSECT, None, 0.0004, &sink)?;
            sink.Close()?;
            geo.cast()
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn clip_el(&self, key: &'static str, a: &ID2D1Geometry, x: f32, y: f32, rx: f32, ry: f32) -> Result<ID2D1Geometry> {
        self.clip(key, a, |p| p.ellipse_geo(x, y, rx, ry))
    }

    fn head(&self) -> Result<ID2D1Geometry> {
        species::head_geo(self, self.sp)
    }

    /// `cmds` clipped to the active species' head outline.
    fn on_head(&self, key: &'static str, cmds: &[S]) -> Result<ID2D1Geometry> {
        let head = self.head()?;
        self.shape_sp(key, |p| unsafe {
            let b = build_path(p.f, cmds)?;
            let geo = p.f.CreatePathGeometry()?;
            let sink = geo.Open()?;
            head.CombineWithGeometry(&b, D2D1_COMBINE_MODE_INTERSECT, None, 0.0004, &sink)?;
            sink.Close()?;
            geo.cast()
        })
    }

    fn star(&self) -> Result<ID2D1Geometry> {
        self.shape("star5", |p| build_path(p.f, &star5()))
    }

    /// A four-point twinkle of radius `r`.
    fn spark(&self, m: Matrix3x2, x: f32, y: f32, r: f32, c: Rgb, a: f32) -> Result<()> {
        if r <= 0.001 {
            return Ok(());
        }
        let g = self.path("spark", SPARK)?;
        self.fill(put(x, y, r, 0.0) * m, &g, c, a);
        Ok(())
    }

    /// A five-petal flower of radius `r`.
    #[allow(clippy::too_many_arguments)]
    fn flower(&self, m: Matrix3x2, x: f32, y: f32, r: f32, deg: f32, petal: Rgb, centre: Rgb) {
        let f = rot(deg, 0.0, 0.0) * tr(x, y) * m;
        for k in 0..5 {
            let pm = tr(0.0, -r * 0.55) * rot(k as f32 * 72.0, 0.0, 0.0) * f;
            self.fill_el(pm, 0.0, 0.0, r * 0.42, r * 0.55, petal, 1.0);
            self.ring(pm, 0.0, 0.0, r * 0.42, r * 0.55, edge(petal), 0.8, self.lw * 0.6);
        }
        self.obj_el(f, 0.0, 0.0, r * 0.38, r * 0.38, centre);
    }
}

// ---------------------------------------------------------------- the fox

#[derive(Clone, Copy, PartialEq)]
enum Layer {
    /// Behind the fox.
    Back,
    /// On the fox, under its paws.
    Mid,
    /// On top of everything.
    Front,
}

/// Draws the mascot centred on (cx, cy); `size` is the head width in pixels.
/// `look` selects species, skin, head accessory and body/gear accessory.
#[allow(clippy::too_many_arguments)]
pub fn draw(rt: &ID2D1RenderTarget, factory: &ID2D1Factory, look: Look, cx: f32, cy: f32, size: f32, a: f32, sleeping: bool, t: f32) -> Result<()> {
    if size <= 1.0 || a <= 0.0 {
        return Ok(());
    }
    let info = species::get(look.species);
    let sk = get(look.skin);
    let acc = look.acc % ACCESSORIES.len();
    let acc2 = look.acc2 % ACCESSORIES.len();
    let mut old = Matrix3x2::default();
    unsafe { rt.GetTransform(&mut old) };
    let line = if luminance(sk.fur) < 0.2 { mix(sk.fur, WHITE, 0.5) } else { mix(sk.fur, INK, 0.66) };
    let pen = Pen {
        rt,
        f: factory,
        st: store(factory)?,
        brush: solid_for(rt)?,
        base: Matrix3x2::scale(size, size) * tr(cx, cy) * old,
        a,
        lw: 0.038f32.max(1.0 / size),
        line,
        small: size < 34.0,
        sp: info.id,
    };
    // Each mascot gets its own rhythm so a grid of them does not blink in step.
    let phase = ((look.skin % SKINS.len()) as f32 * 0.618 + acc as f32 * 0.377 + look.species as f32 * 0.213).fract() * 11.0;
    let r = fox(&pen, info, sk, acc, acc2, look.bot, sleeping, t + phase, size);
    unsafe { rt.SetTransform(&old) };
    r
}

fn fur_brush(p: &Pen, sk: &Skin) -> Result<ID2D1Brush> {
    let top = mix(sk.fur, WHITE, 0.22);
    match sk.pattern {
        Pattern::Gradient => p.linear(0.0, -0.45, 0.0, 0.42, &[(0.0, top, 1.0), (0.35, sk.fur, 1.0), (1.0, sk.fur2, 1.0)]),
        Pattern::Galaxy => p.radial(0.05, -0.05, 0.6, 0.55, &[(0.0, mix(sk.fur2, WHITE, 0.15), 1.0), (0.55, sk.fur2, 1.0), (1.0, sk.fur, 1.0)], 1.0),
        Pattern::Rainbow => {
            let bands = [0xFF6B6B, 0xFFB050, 0xFFE66B, 0x7CE08A, 0x5AB0FF, 0xB58CFF];
            let s: Vec<(f32, Rgb, f32)> = bands.iter().enumerate().map(|(i, &c)| (i as f32 / 5.0, mix(rgb(c), WHITE, 0.15), 1.0)).collect();
            p.linear(0.0, -0.55, 0.0, 0.45, &s)
        }
        _ => p.radial(-0.12, -0.2, 0.78, 0.74, &[(0.0, top, 1.0), (0.5, sk.fur, 1.0), (1.0, mix(sk.fur, INK, 0.1), 1.0)], 1.0),
    }
}

fn draw_accs(p: &Pen, acc: usize, acc2: usize, layer: Layer, head: Matrix3x2, t: f32) -> Result<()> {
    if acc != 0 {
        accessory(p, acc, layer, head, t)?;
    }
    if acc2 != 0 && acc2 != acc {
        accessory(p, acc2, layer, head, t)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn fox(p: &Pen, info: &SpeciesInfo, sk: &Skin, acc: usize, acc2: usize, is_bot: bool, sleeping: bool, t: f32, size: f32) -> Result<()> {
    let bongo = BONGO.with(|c| c.get());
    let zt = bongo.map_or(t, |b| b.zt);
    let t = match bongo {
        Some(b) if !b.live => 0.0,
        _ => t,
    };
    let head_only = bongo.is_some_and(|b| b.head_only);
    let squash = bongo.map_or(0.0, |b| b.squash);
    let head_bob = bongo.map_or(0.0, |b| b.head_bob);
    let ear_bounce = bongo.map_or([0.0; 2], |b| b.ear_bounce);
    let hype = bongo.map_or(0.0, |b| b.hype);
    let sm = bongo.map_or(if sleeping { 1.0 } else { 0.0 }, |b| b.sleep_mix);
    let ym_raw = bongo.map_or(0.0, |b| b.yarn_mix);
    let ym = ym_raw * ym_raw * (3.0 - 2.0 * ym_raw); // smoothstep organic roll
    let yarn_ball = bongo.map_or([0.18, -0.30, 0.0, 0.0], |b| b.yarn_ball);

    // Calm breathing + low-pass smoothed squash + organic sleep/yarn transitions.
    let breathe = (t * (2.0 - sm * 0.65)).sin();
    let sx = 1.0 + squash * 0.045 + sm * 0.04 - breathe * (0.007 + sm * 0.011);
    let sy = 1.0 - squash * 0.045 - sm * 0.05 + breathe * (0.012 + sm * 0.016);
    let body = tr(0.0, -info.desk_y) * Matrix3x2::scale(sx, sy) * tr(0.0, info.desk_y);

    // Head smoothly slumps onto tucked paws when sleeping, or reclines back onto the
    // left side of the taskbar looking up-right at the bouncing yarn ball!
    let hx = -0.19 * ym;
    let bob = sm * 0.082 + ym * 0.13 + breathe * (0.010 + sm * 0.004);
    let awake_sway = (t * 0.85).sin() * 1.4 + head_bob * 1.4;
    let sleep_sway = 6.2 + (t * 0.7).sin() * 0.7;
    let yarn_sway = -28.0 + (t * 1.1).sin() * 1.6 + (yarn_ball[0] - 0.18) * 14.0;
    let sway = (awake_sway * (1.0 - sm) + sleep_sway * sm) * (1.0 - ym) + yarn_sway * ym;
    let head = rot(sway, 0.0, 0.34) * tr(hx, bob) * body;
    let fur = fur_brush(p, sk)?;
    let show = LAYERS.with(|c| c.get());

    // Behind the mascot.
    if !head_only && show & 1 != 0 {
        back_fx(p, sk, t)?;
    }
    if show & 2 != 0 {
        draw_accs(p, acc, acc2, Layer::Back, head, t)?;
    }

    if show & 1 != 0 {
        // Calm, weighted tail sway (always drawn unless head_only app icon).
        if !head_only && !shinobi::tails(p, sk, &fur, t, sm > 0.5)? {
            let phase = t * (1.05 - sm * 0.60);
            let amp = (3.6 + hype * 1.2) * (1.0 - sm) + 1.4 * sm;
            let wag = (phase.sin() + 0.22 * (phase * 2.0 - 0.6).sin()) * amp;
            species::draw_tail(p, info.id, sk, &fur, body, wag, sm, ym)?;
        }

        // Cozy species torso sitting on the desk/bottom edge (or reclining on its back for yarn play!).
        if !head_only {
            species::draw_body(p, info.id, sk, &fur, body, sm, ym, t)?;
        }

        // When deeply asleep (sm > 0.5), paws tuck behind the resting chin!
        if !head_only && sm > 0.5 {
            species::draw_arms(p, info.id, sk, &fur, body, bongo.and_then(|b| b.paws), sm, ym, size)?;
        }

        shinobi::hair_back(p, sk, head)?;

        // Ears (gentle idle sway + occasional soft single-ear flick + tap bounce).
        if acc != HELMET && acc2 != HELMET {
            let (tw_side, tw) = twitch(t);
            let awake_k = 1.0 - sm;
            let idle_l = awake_k * ((t * 1.05 - 0.9).sin() * 1.6 + if tw_side < 0.0 { tw * 0.65 } else { 0.0 });
            let idle_r = awake_k * ((t * 1.05 + 0.9).sin() * 1.6 + if tw_side > 0.0 { tw * 0.65 } else { 0.0 });
            species::draw_ears(p, info.id, sk, &fur, head, [idle_l + ear_bounce[0], idle_r + ear_bounce[1]], sm)?;
        }

        // Head.
        let hg = p.head()?;
        p.fill_with(head, &hg, &fur);
        pattern_under(p, sk, head, t)?;
        species::draw_head_markings(p, info.id, sk, head, &hg)?;
        pattern_over(p, sk, head)?;
        p.stroke(head, &hg, p.line, 1.0, p.lw);
        species::draw_whiskers(p, info.id, head);
        shinobi::over_head(p, sk, head)?;
        face(p, info, sk, head, sm, ym, yarn_ball, hype, t, breathe)?;
        shinobi::over_eyes(p, sk, head)?;
        if is_bot {
            draw_bot_gear(p, head, body, t)?;
        }
    }

    if show & 4 != 0 {
        draw_accs(p, acc, acc2, Layer::Mid, head, t)?;
    }

    if show & 1 != 0 && !head_only && sm <= 0.5 {
        species::draw_arms(p, info.id, sk, &fur, body, bongo.and_then(|b| b.paws), sm, ym, size)?;
    }

    // Playful bouncing yarn ball batted by the paws during yarn mode!
    if show & 1 != 0 && !head_only && ym > 0.02 {
        draw_yarn_ball(p, body, yarn_ball, ym, t)?;
    }

    if show & 4 != 0 {
        draw_accs(p, acc, acc2, Layer::Front, head, t)?;
    }
    if !head_only && show & 1 != 0 {
        front_fx(p, sk, t)?;
    }

    let z_vis = ((sm - 0.55) / 0.45).clamp(0.0, 1.0);
    if z_vis > 0.01 && bongo.is_some() {
        let z = p.path("zed", ZED)?;
        for i in 0..3 {
            let u = (zt * 0.32 + i as f32 / 3.0).fract();
            let s = 0.075 + u * 0.07;
            let m = put(0.42 + u * 0.3 + (u * 7.0).sin() * 0.03, -0.22 - u * 0.48, s, -12.0);
            let w = (0.055f32).max(1.4 / size) / s;
            let a = (u * PI).sin() * z_vis;
            p.stroke(m, &z, mix(sk.fur, INK, 0.7), 0.85 * a, w + 2.0 * p.lw / s);
            p.stroke(m, &z, [0.94, 0.96, 1.0], a, w);
        }
    } else if z_vis > 0.01 {
        let z = p.path("zed", ZED)?;
        let c = [0.9, 0.93, 1.0];
        for i in 0..3 {
            let u = (t * 0.32 + i as f32 / 3.0).fract();
            let s = 0.07 + u * 0.07;
            let m = put(0.44 + u * 0.26 + (u * 7.0).sin() * 0.03, -0.24 - u * 0.5, s, -12.0);
            let w = (0.05f32).max(1.2 / size) / s;
            p.stroke(m, &z, c, (u * PI).sin() * 0.95 * z_vis, w);
        }
    }
    Ok(())
}

/// Cute wound yarn ball with squash/stretch, wrapped wool strands & loose curly thread.
fn draw_yarn_ball(p: &Pen, body: Matrix3x2, ball: [f32; 4], ym: f32, t: f32) -> Result<()> {
    let vis = ((ym - 0.45) / 0.55).clamp(0.0, 1.0);
    let vis = vis * vis * (3.0 - 2.0 * vis);
    let [bx, by, deg, sq] = ball;
    let r = 0.145 * vis;
    if r < 0.01 {
        return Ok(());
    }
    let sx = (1.0 + sq * 0.16).clamp(0.84, 1.22);
    let sy = (1.0 - sq * 0.16).clamp(0.82, 1.20);
    let m = rot(deg, 0.0, 0.0) * Matrix3x2::scale(sx, sy) * tr(bx, by) * body;
    let wool = [0.98, 0.44, 0.56];
    let hi = [1.0, 0.74, 0.80];
    let deep = [0.72, 0.18, 0.32];
    let outline = [0.36, 0.11, 0.18];

    // Playful loose curly yarn strand trailing softly from the ball.
    let sway = (t * 4.5).sin() * 0.045;
    let thread = build_path(
        p.f,
        &[
            M(bx + r * 0.55, by + r * 0.65),
            B(bx + 0.22 + sway, by + 0.10, bx + 0.04 - sway, by + 0.26, bx + 0.20 + sway, by + 0.34),
            Q(bx + 0.28 + sway, by + 0.38, bx + 0.24, by + 0.42),
            O,
        ],
    )?;
    p.stroke(body, &thread, outline, 0.50 * vis, 0.032);
    p.stroke(body, &thread, wool, 0.92 * vis, 0.018);

    // Main spherical wool gradient.
    p.at(m);
    let g = p.radial(-r * 0.28, -r * 0.28, r * 1.15, r * 1.15, &[(0.0, hi, 1.0), (0.55, wool, 1.0), (1.0, deep, 1.0)], 1.0)?;
    unsafe { p.rt.FillEllipse(&ell(0.0, 0.0, r, r), &g) };

    // Criss-cross wound yarn arcs inside the ball.
    let a1 = p.path("yarn_a1", &[M(-0.84, -0.35), Q(0.0, -0.02, 0.86, -0.28), O])?;
    let a2 = p.path("yarn_a2", &[M(-0.88, 0.14), Q(0.05, 0.42, 0.84, 0.18), O])?;
    let a3 = p.path("yarn_a3", &[M(-0.34, -0.84), Q(-0.02, 0.0, -0.24, 0.86), O])?;
    let a4 = p.path("yarn_a4", &[M(0.28, -0.82), Q(0.48, 0.04, 0.18, 0.85), O])?;
    let sm = Matrix3x2::scale(r, r) * m;
    for arc in [&a1, &a2, &a3, &a4] {
        p.stroke(sm, arc, deep, 0.68 * vis, 0.11);
        p.stroke(tr(-0.008, -0.008) * sm, arc, hi, 0.50 * vis, 0.055);
    }
    p.ring(m, 0.0, 0.0, r, r, outline, 0.96 * vis, p.lw * 0.85);
    Ok(())
}

/// Exclusive cute robotic visor, antenna and glowing core for the built-in Fuzzie Bot friend.
fn draw_bot_gear(p: &Pen, head: Matrix3x2, body: Matrix3x2, t: f32) -> Result<()> {
    let cyan = [0.22, 0.88, 1.0];
    let metal = [0.30, 0.35, 0.44];
    let pulse = 0.72 + 0.28 * (t * 3.0).sin();
    // Cute crown antenna with glowing cyan beacon.
    p.seg(head, 0.0, -0.33, 0.0, -0.58, metal, 1.0, 0.045);
    p.glow(head, 0.0, -0.61, 0.14, 0.14, cyan, 0.75 * pulse)?;
    p.obj_el(head, 0.0, -0.61, 0.055, 0.055, cyan);
    // Sleek translucent cyber-visor across the eyes.
    let visor = p.path("bot_visor", &rrect(-0.34, -0.045, 0.34, 0.125, 0.075))?;
    p.fill(head, &visor, cyan, 0.24);
    p.stroke(head, &visor, metal, 1.0, p.lw * 0.9);
    p.seg(head, -0.24, -0.01, -0.14, -0.01, WHITE, 0.7, 0.018);
    // Tiny glowing chest core on the torso.
    if !p.small {
        p.glow(body, 0.0, 0.33, 0.09, 0.09, cyan, 0.65 * pulse)?;
        p.fill_el(body, 0.0, 0.33, 0.036, 0.036, cyan, 0.95);
    }
    Ok(())
}

/// Small, quiet impact marks fanning down from a paw that hits the desk.
fn tap_burst(p: &Pen, x: f32, y: f32, _side: f32, size: f32, a: f32) -> Result<()> {
    let body = Matrix3x2::identity();
    let c = mix(p.line, INK, 0.35);
    let w = 0.022f32.max(1.1 / size);
    // Angles from straight down; the middle mark is a touch longer.
    for (deg, len) in [(-38.0f32, 0.04), (0.0, 0.05), (38.0, 0.04)] {
        let (s, co) = deg.to_radians().sin_cos();
        let (dx, dy) = (s, co);
        let (x0, y0) = (x + dx * 0.09, y + 0.025 + dy * 0.06);
        p.seg(body, x0, y0, x0 + dx * len, y0 + dy * len, c, 0.7 * a, w);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn face(p: &Pen, info: &SpeciesInfo, sk: &Skin, head: Matrix3x2, sleep_mix: f32, yarn_mix: f32, yarn_ball: [f32; 4], hype: f32, t: f32, breathe: f32) -> Result<()> {
    let asleep = sleep_mix >= 0.82;
    // Blush (glows rosier as typing speed rises or while playing with yarn).
    if sk.pattern != Pattern::Hearts {
        let pink = [1.0, 0.48, 0.62];
        let boost = ((hype * 1.25).max(yarn_mix * 0.65)).min(1.0) * (1.0 - sleep_mix * 0.5);
        for side in [-1.0f32, 1.0] {
            let (bx, by) = ((info.eye_x + 0.115) * side, info.eye_y + 0.095);
            if p.small || glow_off() {
                p.fill_el(head, bx, by, 0.074 + boost * 0.018, 0.045, pink, 0.42 + boost * 0.35);
            } else {
                p.glow(head, bx, by, 0.10 + boost * 0.025, 0.065, pink, 0.58 + boost * 0.34)?;
            }
            // Cute anime blush hatch marks in joyful flow state.
            if !p.small && !asleep && hype > 0.55 {
                for j in -1..=1 {
                    let x = bx + j as f32 * 0.026;
                    p.seg(head, x - 0.007, by + 0.016, x + 0.009, by - 0.016, [1.0, 0.34, 0.48], 0.68, 0.016);
                }
            }
        }
        // Subtle sparkle beside the cheek when typing fast.
        if !p.small && !asleep && hype > 0.58 {
            p.spark(head, 0.44, -0.06, 0.062, [1.0, 0.92, 0.48], 0.9)?;
        }
    }
    // Eyes: smooth sleep droop -> fast-typing happy ^ ^ -> normal (blink ONLY in normal mode, never during happy eyes!).
    let base_b = if asleep || hype > 0.28 {
        0.0
    } else if let Some(bg) = BONGO.with(|c| c.get()) {
        if bg.live { blink(t).max(bg.blink) } else { bg.blink }
    } else {
        blink(t)
    };
    let b = if asleep { 1.0 } else { base_b.max(sleep_mix * 0.96) };
    let look_x = if asleep || hype > 0.28 {
        0.0
    } else if yarn_mix > 0.1 {
        (0.022 + (yarn_ball[0] - 0.18) * 0.05).clamp(-0.02, 0.032) * yarn_mix
    } else {
        (t * 0.31).sin() * 0.012
    };
    let look_y = if asleep || hype > 0.28 { 0.0 } else { -0.024 * yarn_mix };
    let lid = p.path("lid", LID)?;
    let happy_eye = p.path("happy_eye", HAPPY_EYE)?;
    let happy_eye_big = p.path("happy_eye_big", HAPPY_EYE_BIG)?;
    for side in [-1.0f32, 1.0] {
        let (ex, ey) = (info.eye_x * side + look_x, info.eye_y + look_y);
        if asleep {
            p.stroke(tr(ex, ey) * head, &lid, sk.eyes, 1.0, p.lw * 1.1);
            continue;
        }
        if hype > 0.58 && sleep_mix < 0.25 {
            p.stroke(tr(ex, ey) * head, &happy_eye_big, sk.eyes, 1.0, p.lw * 1.25);
            continue;
        }
        if hype > 0.28 && sleep_mix < 0.25 {
            p.stroke(tr(ex, ey) * head, &happy_eye, sk.eyes, 1.0, p.lw * 1.15);
            continue;
        }
        if b > 0.8 {
            p.stroke(tr(ex, ey) * head, &lid, sk.eyes, 1.0, p.lw * 1.1);
            continue;
        }
        let style = shinobi::eye_for(sk.eye, side);
        let patterned = style != Eye::Normal && !p.small;
        let (rx, ry0) = if patterned { (0.095, 0.106) } else { (info.eye_rx, info.eye_ry) };
        let ry = ry0 * (1.0 - b);
        p.fill_el(head, ex, ey, rx, ry, sk.eyes, 1.0);
        if patterned {
            shinobi::iris(p, style, Matrix3x2::scale(rx, ry) * tr(ex, ey) * head, t)?;
        } else if !p.small {
            p.at(head);
            let shine = p.radial(ex, ey + ry * 0.55, 0.065, ry * 0.58, &[(0.0, mix(sk.eyes, WHITE, 0.45), 0.9), (1.0, sk.eyes, 0.0)], 1.0)?;
            unsafe { p.rt.FillEllipse(&ell(ex, ey + ry * 0.4, 0.065, ry * 0.58), &shine) };
        }
        let k = 1.0 - b;
        if patterned {
            p.fill_el(head, ex - rx * 0.6, ey - ry * 0.62, 0.02, 0.02 * k, WHITE, 0.95);
        } else {
            p.fill_el(head, ex - 0.023, ey - 0.028 * k, 0.026, 0.024 * k, WHITE, 0.95);
            if !p.small {
                p.fill_el(head, ex + 0.025, ey + 0.028 * k, 0.012, 0.012 * k, WHITE, 0.85);
            }
        }
        if shinobi::anime(sk) && !p.small {
            let lash = p.path("sh_lash", shinobi::LASH)?;
            p.fill(Matrix3x2::scale(rx * side, ry) * tr(ex, ey) * head, &lash, shinobi::lash_colour(sk), 1.0);
        }
    }
    if sk.face == Face::Mask {
        return Ok(());
    }
    // Nose (with subtle idle wiggle for Bunny).
    let wiggle = if info.id == Species::Bunny && !asleep && (t * 1.2).sin() > 0.5 { (t * 7.5).sin() * 0.004 } else { 0.0 };
    let ny = info.eye_y + 0.088 + wiggle;
    let nose_c = if info.id == Species::Bunny {
        [1.0, 0.58, 0.68]
    } else if luminance(sk.cream) > 0.35 {
        mix(sk.fur, INK, 0.82)
    } else {
        [1.0, 0.72, 0.82]
    };
    let nm = tr(0.0, ny) * head;
    if info.id == Species::Bunny {
        p.fill_el(nm, 0.0, 0.0, 0.036, 0.024, nose_c, 1.0);
    } else {
        let nose = p.path("nose", NOSE)?;
        p.fill(nm, &nose, nose_c, 1.0);
    }
    if !p.small {
        p.fill_el(nm, -0.012, -0.007, 0.012, 0.007, WHITE, 0.65);
    }

    // Mouth: relaxed sleeping 'o' + pastel bubble (fades smoothly with sleep_mix), or 2-tier clean happy smile (no tongue).
    let mm = tr(0.0, ny + 0.044) * head;
    let line_c = if luminance(sk.cream) > 0.35 { mix(sk.fur, INK, 0.82) } else { [1.0, 0.72, 0.82] };
    if sleep_mix > 0.65 {
        let sa = ((sleep_mix - 0.65) / 0.35).clamp(0.0, 1.0);
        let mr = 0.020 + (breathe * 0.5 + 0.5) * 0.009;
        p.fill_el(mm, 0.0, 0.012, mr * 1.15, mr, [0.92, 0.54, 0.60], sa);
        p.ring(mm, 0.0, 0.012, mr * 1.15, mr, line_c, sa, p.lw * 0.65);
        // Translucent pastel sleep bubble expanding/shrinking with breathing.
        if !p.small {
            let bub = (0.050 + (breathe * 0.5 + 0.5) * 0.042) * sa;
            let (bx, by) = (0.04 + bub * 0.65, ny + 0.01 - bub * 0.15);
            p.at(head);
            let bg = p.radial(bx - bub * 0.3, by - bub * 0.3, bub, bub, &[(0.0, WHITE, 0.78 * sa), (0.6, [0.78, 0.92, 1.0], 0.32 * sa), (1.0, [0.68, 0.86, 1.0], 0.55 * sa)], 1.0)?;
            unsafe { p.rt.FillEllipse(&ell(bx, by, bub, bub), &bg) };
            p.ring(head, bx, by, bub, bub, WHITE, 0.72 * sa, 0.014);
        }
    } else if hype > 0.58 {
        let open = p.path("mouth_open", MOUTH_OPEN)?;
        p.fill(mm, &open, [0.95, 0.52, 0.61], 1.0);
        p.stroke(mm, &open, line_c, 1.0, p.lw * 0.78);
    } else if hype > 0.28 {
        let happy = p.path("mouth_happy", MOUTH_HAPPY)?;
        p.stroke(mm, &happy, line_c, 1.0, p.lw * 0.85);
    } else {
        let mouth = p.path("mouth", MOUTH)?;
        p.stroke(mm, &mouth, line_c, 1.0, p.lw * 0.82);
    }
    Ok(())
}

/// Galaxy stars: (x, y, radius).
const GALAXY_STARS: [(f32, f32, f32); 8] = [(-0.25, -0.18, 0.016), (0.18, -0.22, 0.02), (0.02, -0.12, 0.012), (0.33, -0.03, 0.014), (-0.36, -0.04, 0.012), (-0.1, -0.25, 0.014), (0.26, -0.13, 0.01), (-0.18, -0.06, 0.01)];

fn pattern_under(p: &Pen, sk: &Skin, head: Matrix3x2, t: f32) -> Result<()> {
    let hg = p.head()?;
    match sk.pattern {
        Pattern::Stripes => {
            let mut cmds = vec![M(-0.05, -0.36), Q(0.0, -0.2, 0.0, -0.16), Q(0.0, -0.2, 0.05, -0.36), Z];
            let side = [M(0.12, -0.36), Q(0.13, -0.24, 0.14, -0.2), Q(0.16, -0.25, 0.23, -0.34), Z, M(0.62, -0.10), Q(0.48, -0.07, 0.35, -0.04), Q(0.48, -0.03, 0.62, 0.0), Z, M(0.62, 0.04), Q(0.52, 0.06, 0.42, 0.085), Q(0.52, 0.095, 0.62, 0.11), Z];
            cmds.extend_from_slice(&side);
            cmds.extend(mirror(&side));
            let g = p.on_head("stripes", &cmds)?;
            p.fill(head, &g, sk.mark, 1.0);
        }
        Pattern::Spots => {
            for (x, y, rx, ry) in [(-0.22, -0.14, 0.05, 0.04), (0.2, -0.18, 0.042, 0.036), (0.05, -0.25, 0.03, 0.025), (-0.07, -0.2, 0.024, 0.02), (0.33, -0.04, 0.035, 0.03), (-0.35, -0.02, 0.03, 0.028), (0.1, -0.1, 0.02, 0.018)] {
                p.fill_el(head, x, y, rx, ry, sk.mark, 1.0);
            }
        }
        Pattern::Calico => {
            let a = p.shape_sp("calico_a", |p| unsafe {
                let b = p.ellipse_geo(-0.3, -0.22, 0.24, 0.2)?;
                let geo = p.f.CreatePathGeometry()?;
                let sink = geo.Open()?;
                hg.CombineWithGeometry(&b, D2D1_COMBINE_MODE_INTERSECT, None, 0.0004, &sink)?;
                sink.Close()?;
                geo.cast()
            })?;
            let b = p.shape_sp("calico_b", |p| unsafe {
                let b = p.ellipse_geo(0.34, -0.06, 0.16, 0.22)?;
                let geo = p.f.CreatePathGeometry()?;
                let sink = geo.Open()?;
                hg.CombineWithGeometry(&b, D2D1_COMBINE_MODE_INTERSECT, None, 0.0004, &sink)?;
                sink.Close()?;
                geo.cast()
            })?;
            p.fill(head, &a, sk.mark, 1.0);
            p.fill(head, &b, sk.fur2, 1.0);
        }
        Pattern::Galaxy => {
            // Drifting nebula clouds, clipped to the head by filling its outline.
            let (dx, dy) = ((t * 0.3).sin() * 0.06, (t * 0.23).cos() * 0.04);
            let n1 = p.radial(-0.18 + dx, -0.12 + dy, 0.32, 0.24, &[(0.0, sk.mark, 0.75), (1.0, sk.mark, 0.0)], 1.0)?;
            p.fill_with(head, &hg, &n1);
            let n2 = p.radial(0.22 - dx, -0.05 - dy, 0.26, 0.2, &[(0.0, mix(sk.mark, WHITE, 0.3), 0.45), (1.0, sk.mark, 0.0)], 1.0)?;
            p.fill_with(head, &hg, &n2);
            for (i, &(x, y, r)) in GALAXY_STARS.iter().enumerate() {
                let tw = 0.55 + 0.45 * (t * 2.3 + i as f32 * 1.7).sin();
                if i % 3 == 0 && !p.small {
                    p.spark(head, x, y, r * 2.4 * tw, WHITE, 0.95)?;
                } else {
                    p.fill_el(head, x, y, r, r, WHITE, tw);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn pattern_over(p: &Pen, sk: &Skin, head: Matrix3x2) -> Result<()> {
    match sk.pattern {
        Pattern::Patch => {
            for side in [-1.0f32, 1.0] {
                let m = rot(-24.0 * side, 0.19 * side, 0.08) * head;
                p.fill_el(m, 0.19 * side, 0.08, 0.125, 0.105, sk.mark, 1.0);
            }
        }
        Pattern::Mask => {
            let g = p.on_head("mask", &[M(-0.6, -0.03), B(-0.3, -0.09, -0.1, 0.0, 0.0, 0.02), B(0.1, 0.0, 0.3, -0.09, 0.6, -0.03), L(0.6, 0.12), B(0.35, 0.17, 0.12, 0.15, 0.0, 0.11), B(-0.12, 0.15, -0.35, 0.17, -0.6, 0.12), Z])?;
            p.fill(head, &g, sk.mark, 1.0);
        }
        Pattern::Star => {
            let g = p.star()?;
            p.obj(put(0.0, -0.17, 0.075, 0.0) * head, 0.075, &g, sk.mark);
        }
        Pattern::Hearts => {
            let g = p.path("heart", HEART)?;
            for side in [-1.0f32, 1.0] {
                p.fill(put(0.31 * side, 0.18, 0.11, 12.0 * side) * head, &g, sk.mark, 0.9);
            }
        }
        Pattern::Kitsune => {
            let flame = p.path("kitsune_flame", &[M(0.0, -0.27), Q(0.05, -0.18, 0.0, -0.1), Q(-0.05, -0.18, 0.0, -0.27), Z])?;
            p.fill(head, &flame, sk.mark, 1.0);
            for side in [-1.0f32, 1.0] {
                p.seg(head, 0.38 * side, 0.08, 0.47 * side, 0.06, sk.mark, 1.0, 0.024);
                p.seg(head, 0.39 * side, 0.13, 0.48 * side, 0.13, sk.mark, 1.0, 0.024);
            }
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------- effects

fn back_fx(p: &Pen, sk: &Skin, t: f32) -> Result<()> {
    shinobi::fx_back(p, sk, t)?;
    if sk.fx == Fx::Glow && !glow_off() {
        let pulse = 0.82 + 0.18 * (t * 1.8).sin();
        let (rx, ry) = (0.94 + 0.05 * pulse, 0.88 + 0.05 * pulse);
        p.at(Matrix3x2::identity());
        let b = p.radial(0.0, -0.02, rx, ry, &[(0.0, sk.fxc, 1.0), (0.48, sk.fxc, 0.76), (0.78, sk.fxc, 0.28), (1.0, sk.fxc, 0.0)], 0.82 * pulse)?;
        unsafe { p.rt.FillEllipse(&ell(0.0, -0.02, rx, ry), &b) };
    }
    Ok(())
}

/// Particle starting x positions, kept to the sides of the face.
const SIDES: [f32; 6] = [-0.66, 0.62, -0.5, 0.74, -0.78, 0.5];

fn front_fx(p: &Pen, sk: &Skin, t: f32) -> Result<()> {
    let body = Matrix3x2::identity();
    let c = sk.fxc;
    let n = if p.small { 3 } else { 6 };
    let k = if p.small { 1.6 } else { 1.0 };
    match sk.fx {
        Fx::Twinkle => {
            for (i, (x, y)) in [(-0.58, -0.38), (0.6, -0.48), (-0.66, 0.16), (0.68, 0.22), (0.02, -0.74)].into_iter().enumerate().take(n) {
                let s = (t * 2.4 + i as f32 * 1.7).sin().max(0.0);
                p.spark(body, x, y, 0.075 * s * k, c, 0.95)?;
            }
        }
        Fx::Stars => {
            let g = p.star()?;
            for i in 0..3 {
                let ang = t * 0.9 + i as f32 * PI * 2.0 / 3.0;
                let (x, y) = (ang.cos() * 0.72, -0.1 + ang.sin() * 0.5);
                p.fill(put(x, y, 0.06 * k, t * 40.0), &g, c, 0.95);
            }
        }
        Fx::Snow | Fx::Petals | Fx::Leaves => {
            let leaf = p.path("leaf", LEAF)?;
            for i in 0..n {
                let u = (t * 0.17 + i as f32 * 0.618).fract();
                let x = SIDES[i] + (t * 1.1 + i as f32).sin() * 0.05;
                let y = -0.75 + u * 1.25;
                let a = (u * PI).sin();
                match sk.fx {
                    Fx::Snow => {
                        p.fill_el(body, x, y, 0.024 * k, 0.024 * k, c, a);
                        if !p.small && i % 2 == 0 {
                            p.spark(body, x, y, 0.045, c, a * 0.8)?;
                        }
                    }
                    Fx::Petals => {
                        let m = rot(t * 90.0 + i as f32 * 50.0, 0.0, 0.0) * tr(x, y);
                        p.fill_el(m, 0.0, 0.0, 0.035 * k, 0.022 * k, c, a);
                    }
                    _ => p.fill(put(x, y, 0.09 * k, t * 70.0 + i as f32 * 60.0), &leaf, c, a),
                }
            }
        }
        Fx::Embers | Fx::Hearts | Fx::Bubbles => {
            let heart = p.path("heart", HEART)?;
            for i in 0..n {
                let u = (t * 0.22 + i as f32 * 0.618).fract();
                let x = SIDES[i] * 0.9 + (t * 2.0 + i as f32 * 2.0).sin() * 0.04;
                let y = 0.45 - u * 1.2;
                let a = (u * PI).sin();
                match sk.fx {
                    Fx::Embers => {
                        p.glow(body, x, y, 0.05 * k, 0.05 * k, c, a * 0.6)?;
                        p.fill_el(body, x, y, 0.016 * k, 0.016 * k, mix(c, WHITE, 0.5), a);
                    }
                    Fx::Hearts => p.fill(put(x, y, 0.11 * k * (0.7 + 0.3 * a), (t * 2.0 + i as f32).sin() * 15.0), &heart, c, a),
                    _ => {
                        let r = (0.03 + 0.015 * (i % 3) as f32) * k;
                        p.ring(body, x, y, r, r, c, a * 0.9, p.lw * 0.6);
                        p.fill_el(body, x - r * 0.35, y - r * 0.35, r * 0.25, r * 0.25, WHITE, a * 0.8);
                    }
                }
            }
        }
        _ => shinobi::fx_front(p, sk, t)?,
    }
    Ok(())
}

// ---------------------------------------------------------------- accessories

const GOLD: Rgb = [1.0, 0.82, 0.32];

fn accessory(p: &Pen, i: usize, layer: Layer, h: Matrix3x2, t: f32) -> Result<()> {
    if i >= SHINOBI {
        return shinobi::accessory(p, i - SHINOBI, layer, h, t);
    }
    use Layer::*;
    let lw = p.lw * 0.85;
    match (i, layer) {
        // Headband
        (1, Front) => {
            let band = p.on_head("headband", BAND)?;
            p.obj(h, 1.0, &band, [0.95, 0.28, 0.34]);
            let stripe = p.on_head("headband_stripe", &[M(-0.6, -0.215), B(-0.2, -0.272, 0.2, -0.272, 0.6, -0.215), L(0.6, -0.195), B(0.2, -0.252, -0.2, -0.252, -0.6, -0.195), Z])?;
            p.fill(h, &stripe, WHITE, 0.9);
        }
        // Ninja band with tails fluttering in the wind.
        (2, Front) => {
            let navy = [0.18, 0.2, 0.32];
            let tail = p.path("ninja_tail", &[M(0.0, -0.02), B(-0.08, -0.03, -0.16, 0.02, -0.28, 0.0), L(-0.26, 0.06), B(-0.15, 0.07, -0.08, 0.04, 0.0, 0.03), Z])?;
            for (j, base) in [-18.0f32, -38.0].into_iter().enumerate() {
                let deg = base + (t * 6.0 + j as f32 * 1.3).sin() * 9.0;
                p.obj(rot(deg, 0.0, 0.0) * tr(-0.40, -0.19) * h, 1.0, &tail, navy);
            }
            let band = p.on_head("headband", BAND)?;
            p.obj(h, 1.0, &band, navy);
            p.fill_el(h, -0.40, -0.19, 0.035, 0.03, navy, 1.0);
            let plate = p.path("ninja_plate", &rrect(-0.1, -0.29, 0.1, -0.205, 0.02))?;
            p.obj(h, 1.0, &plate, [0.84, 0.87, 0.92]);
            p.ring(h, 0.0, -0.247, 0.022, 0.018, [0.4, 0.44, 0.5], 1.0, lw * 0.7);
        }
        // Star clip by the ear, twinkling.
        (3, Front) => {
            let g = p.star()?;
            p.obj(put(0.28, -0.27, 0.085, 15.0) * h, 0.085, &g, [1.0, 0.84, 0.3]);
            let s = (t * 3.0).sin().max(0.0);
            p.spark(h, 0.39, -0.37, 0.05 * s, WHITE, 0.95)?;
        }
        // Flower behind the ear, swaying.
        (4, Front) => p.flower(h, -0.30, -0.28, 0.1, (t * 1.2).sin() * 8.0, [1.0, 0.62, 0.78], [1.0, 0.86, 0.35]),
        // Heart boppers on springs.
        (5, Front) => {
            let pink = [0.96, 0.36, 0.56];
            let band = p.path("bopper_band", &[M(-0.40, -0.17), B(-0.34, -0.40, 0.34, -0.40, 0.40, -0.17), O])?;
            p.stroke(h, &band, edge(pink), 1.0, 0.05);
            p.stroke(h, &band, pink, 1.0, 0.03);
            let sp = p.shape("spring", |p| build_path(p.f, &spring()))?;
            let heart = p.path("heart", HEART)?;
            for side in [-1.0f32, 1.0] {
                let deg = side * 16.0 + (t * 4.0 + side).sin() * 7.0;
                let len = 0.3 + 0.025 * (t * 6.0 + side).sin();
                let (bx, by) = (0.14 * side, -0.33);
                p.stroke(Matrix3x2::scale(len, len) * rot(deg, 0.0, 0.0) * tr(bx, by) * h, &sp, [0.35, 0.3, 0.38], 1.0, 0.022 / len);
                let r = deg.to_radians();
                let (hx, hy) = (bx + len * r.sin(), by - len * r.cos());
                p.obj(put(hx, hy - 0.03, 0.15, deg * 0.5) * h, 0.15, &heart, [1.0, 0.3, 0.45]);
                p.fill_el(h, hx - 0.025, hy - 0.05, 0.012, 0.009, WHITE, 0.8);
            }
        }
        // Top hat.
        (6, Front) => {
            let f = rot(8.0, 0.04, -0.3) * h;
            let dark = [0.16, 0.14, 0.2];
            p.obj_el(f, 0.04, -0.31, 0.29, 0.055, dark);
            let crown = p.path("hat_crown", &[M(-0.12, -0.32), L(-0.14, -0.68), Q(0.04, -0.72, 0.22, -0.68), L(0.2, -0.32), Q(0.04, -0.29, -0.12, -0.32), Z])?;
            p.obj(f, 1.0, &crown, dark);
            let band = p.path("hat_band", &[M(-0.123, -0.36), L(-0.127, -0.43), Q(0.04, -0.455, 0.207, -0.43), L(0.203, -0.36), Q(0.04, -0.335, -0.123, -0.36), Z])?;
            p.fill(f, &band, [0.88, 0.26, 0.36], 1.0);
            p.seg(f, -0.08, -0.47, -0.095, -0.64, WHITE, 0.22, 0.025);
        }
        // Royal crown with hopping glints.
        (7, Front) => {
            let f = rot(-4.0, 0.0, -0.3) * h;
            let crown = p.path("crown", &[M(-0.24, -0.28), L(-0.27, -0.52), L(-0.14, -0.40), L(0.0, -0.58), L(0.14, -0.40), L(0.27, -0.52), L(0.24, -0.28), Q(0.0, -0.25, -0.24, -0.28), Z])?;
            let gold = p.linear(0.0, -0.58, 0.0, -0.26, &[(0.0, [1.0, 0.92, 0.55], 1.0), (1.0, [0.93, 0.62, 0.12], 1.0)])?;
            p.fill_with(f, &crown, &gold);
            p.stroke(f, &crown, edge(GOLD), 1.0, lw);
            p.obj_el(f, 0.0, -0.34, 0.035, 0.04, [0.95, 0.2, 0.35]);
            for side in [-1.0f32, 1.0] {
                p.obj_el(f, 0.14 * side, -0.32, 0.024, 0.026, [0.3, 0.6, 1.0]);
            }
            let tips = [(-0.27, -0.52), (0.0, -0.58), (0.27, -0.52)];
            for (x, y) in tips {
                p.obj_el(f, x, y, 0.028, 0.028, GOLD);
            }
            let u = t * 1.4;
            let (x, y) = tips[(u as usize) % 3];
            p.spark(f, x + 0.03, y - 0.03, 0.07 * (u.fract() * PI).sin(), WHITE, 1.0)?;
        }
        // Astronaut: collar under the paws, glass bubble on top.
        (8, Mid) => p.obj_el(h, 0.0, 0.42, 0.46, 0.085, [0.9, 0.92, 0.96]),
        (8, Front) => {
            p.at(h);
            let glass = p.radial(0.0, -0.05, 0.68, 0.68, &[(0.0, [0.75, 0.9, 1.0], 0.06), (0.8, [0.75, 0.9, 1.0], 0.14), (1.0, [0.8, 0.92, 1.0], 0.35)], 1.0)?;
            unsafe { p.rt.FillEllipse(&ell(0.0, -0.05, 0.68, 0.68), &glass) };
            p.ring(h, 0.0, -0.05, 0.68, 0.68, [0.85, 0.92, 1.0], 0.75, 0.028);
            let hi = p.path("astro_hi", &[M(-0.5, -0.2), Q(-0.44, -0.5, -0.18, -0.6), O])?;
            p.stroke(h, &hi, WHITE, 0.8, 0.045);
            p.fill_el(h, -0.55, -0.06, 0.02, 0.02, WHITE, 0.8);
            p.seg(h, 0.4, -0.58, 0.5, -0.8, [0.75, 0.78, 0.85], 1.0, 0.025);
            let on = if (t * 1.5).fract() < 0.5 { 1.0 } else { 0.35 };
            p.glow(h, 0.5, -0.8, 0.08, 0.08, [1.0, 0.3, 0.3], on * 0.7)?;
            p.fill_el(h, 0.5, -0.8, 0.032, 0.032, [1.0, 0.35, 0.35], 0.5 + 0.5 * on);
        }
        // Planet ring: the ring circles the head, a little planet orbits on it.
        (9, Back) | (9, Front) => {
            // Around the neck, so the front of the ring passes under the smile.
            let f = rot(-12.0, 0.0, 0.0) * tr(0.0, 0.17) * h;
            let ring_c = [1.0, 0.82, 0.55];
            let (rx, ry) = (0.78, 0.16);
            if layer == Back {
                p.ring(f, 0.0, 0.0, rx, ry, ring_c, 0.85, 0.04);
                p.ring(f, 0.0, 0.0, rx - 0.06, ry - 0.025, ring_c, 0.45, 0.014);
            } else {
                let k = 0.5523;
                let half = |rx: f32, ry: f32| [M(-rx, 0.0), B(-rx, k * ry, -k * rx, ry, 0.0, ry), B(k * rx, ry, rx, k * ry, rx, 0.0), O];
                let front = p.path("ring_front", &half(rx, ry))?;
                let front2 = p.path("ring_front2", &half(rx - 0.06, ry - 0.025))?;
                p.stroke(f, &front, ring_c, 0.95, 0.04);
                p.stroke(f, &front2, ring_c, 0.5, 0.014);
            }
            let ang = t * 1.3;
            if (ang.sin() > 0.0) == (layer == Front) {
                let (x, y) = (rx * ang.cos(), ry * ang.sin());
                p.at(f);
                let ball = p.radial(x - 0.025, y - 0.025, 0.09, 0.09, &[(0.0, [0.7, 0.8, 1.0], 1.0), (1.0, [0.35, 0.3, 0.85], 1.0)], 1.0)?;
                unsafe { p.rt.FillEllipse(&ell(x, y, 0.07, 0.07), &ball) };
                p.ring(f, x, y, 0.07, 0.07, edge([0.45, 0.4, 0.9]), 1.0, lw);
                p.ring(f, x, y, 0.12, 0.03, [1.0, 0.85, 0.6], 0.9, 0.014);
            }
        }
        // Forest antlers with leaves swaying.
        (10, Front) => {
            let antler = p.path("antler", &[M(0.15, -0.27), Q(0.22, -0.48, 0.33, -0.62), O, M(0.235, -0.44), Q(0.33, -0.46, 0.42, -0.4), O, M(0.29, -0.55), Q(0.26, -0.65, 0.25, -0.74), O])?;
            let leaf = p.path("leaf", LEAF)?;
            let brown = [0.6, 0.4, 0.25];
            for side in [-1.0f32, 1.0] {
                let m = if side < 0.0 { flip() * h } else { h };
                p.stroke(m, &antler, edge(brown), 1.0, 0.075);
                p.stroke(m, &antler, brown, 1.0, 0.048);
                let sw = (t * 1.5 + side).sin() * 10.0;
                p.obj(put(0.45, -0.42, 0.13, 20.0 + sw) * m, 0.13, &leaf, [0.45, 0.8, 0.38]);
                p.obj(put(0.27, -0.79, 0.11, -70.0 + sw) * m, 0.11, &leaf, [0.55, 0.86, 0.42]);
            }
        }
        // Fox mask worn on the side of the head.
        (11, Front) => {
            let k = 0.46;
            let f = put(-0.3, -0.24, k, -24.0) * h;
            let face = p.shape("mask_face", |p| build_path(p.f, &sym(&[M(0.0, -0.22), B(0.18, -0.24, 0.3, -0.14, 0.34, 0.04), B(0.3, 0.22, 0.12, 0.4, 0.0, 0.46)])))?;
            let ear_r = [M(0.08, -0.16), L(0.28, -0.52), L(0.34, -0.02), Z];
            let in_r = [M(0.15, -0.16), L(0.27, -0.4), L(0.3, -0.06), Z];
            let ears = p.shape("mask_ears", |p| build_path(p.f, &[ear_r.to_vec(), mirror(&ear_r)].concat()))?;
            let inner = p.shape("mask_in", |p| build_path(p.f, &[in_r.to_vec(), mirror(&in_r)].concat()))?;
            let white = [0.98, 0.97, 0.95];
            let red = [0.88, 0.2, 0.25];
            p.obj(f, k, &ears, white);
            p.fill(f, &inner, red, 1.0);
            p.obj(f, k, &face, white);
            p.fill_el(f, 0.0, -0.08, 0.04, 0.07, red, 1.0);
            let eye = p.path("mask_eye", &[M(0.07, 0.07), Q(0.15, 0.0, 0.24, 0.06), O, M(-0.07, 0.07), Q(-0.15, 0.0, -0.24, 0.06), O])?;
            p.stroke(f, &eye, INK, 1.0, 0.045);
            for side in [-1.0f32, 1.0] {
                p.seg(f, 0.18 * side, 0.18, 0.28 * side, 0.14, red, 1.0, 0.04);
                p.seg(f, 0.17 * side, 0.25, 0.26 * side, 0.23, red, 1.0, 0.04);
            }
            p.fill_el(f, 0.0, 0.38, 0.035, 0.028, INK, 1.0);
        }
        // Peacock crest: stalks with eye-spot tips, swaying.
        (12, Front) => {
            for (j, base) in [-38.0f32, -19.0, 0.0, 19.0, 38.0].into_iter().enumerate() {
                let deg = base + (t * 1.3 + j as f32 * 0.6).sin() * 4.0;
                let len = if j == 2 { 0.3 } else if j % 2 == 1 { 0.27 } else { 0.22 };
                let m = rot(deg, 0.0, 0.0) * tr(0.0, -0.29) * h;
                p.seg(m, 0.0, 0.0, 0.0, -len, [0.1, 0.4, 0.42], 1.0, 0.018);
                p.obj_el(m, 0.0, -len - 0.04, 0.045, 0.06, [0.1, 0.72, 0.66]);
                p.fill_el(m, 0.0, -len - 0.035, 0.028, 0.038, [0.18, 0.4, 0.9], 1.0);
                p.fill_el(m, 0.0, -len - 0.03, 0.013, 0.016, [0.06, 0.08, 0.25], 1.0);
            }
        }
        // Angel wings, flapping.
        (13, Back) => {
            let wing = p.path("wing", &[M(0.0, 0.0), B(0.15, -0.25, 0.4, -0.38, 0.62, -0.36), B(0.58, -0.28, 0.56, -0.24, 0.5, -0.2), B(0.56, -0.18, 0.55, -0.12, 0.46, -0.08), B(0.5, -0.04, 0.46, 0.02, 0.36, 0.04), B(0.25, 0.1, 0.1, 0.1, 0.0, 0.06), Z])?;
            let lines = p.path("wing_lines", &[M(0.18, -0.07), Q(0.32, -0.11, 0.46, -0.15), O, M(0.12, 0.0), Q(0.25, -0.02, 0.38, -0.04), O])?;
            let flap = (t * 3.2).sin() * 12.0;
            let white = [0.99, 0.98, 1.0];
            for side in [-1.0f32, 1.0] {
                let mut m = rot(-flap, 0.0, 0.0) * tr(0.27, 0.1);
                if side < 0.0 {
                    m = m * flip();
                }
                p.fill(m, &wing, white, 1.0);
                p.stroke(m, &wing, [0.62, 0.68, 0.86], 1.0, lw);
                p.stroke(m, &lines, [0.62, 0.68, 0.86], 0.6, lw * 0.7);
            }
        }
        // Round glasses.
        (14, Front) => {
            let frame = [0.2, 0.17, 0.22];
            for side in [-1.0f32, 1.0] {
                let x = 0.185 * side;
                p.fill_el(h, x, 0.065, 0.112, 0.112, WHITE, 0.12);
                p.ring(h, x, 0.065, 0.112, 0.112, frame, 1.0, 0.032);
                p.seg(h, x - 0.06, 0.03, x - 0.025, -0.005, WHITE, 0.6, 0.016);
                p.seg(h, 0.297 * side, 0.04, 0.43 * side, 0.0, frame, 1.0, 0.028);
            }
            let bridge = p.path("glasses_bridge", &[M(-0.075, 0.04), Q(0.0, 0.0, 0.075, 0.04), O])?;
            p.stroke(h, &bridge, frame, 1.0, 0.03);
        }
        // Bow by the ear.
        (15, Front) => {
            let k = 0.16;
            let f = put(0.3, -0.29, k, 20.0 + (t * 2.0).sin() * 5.0) * h;
            let pink = [1.0, 0.45, 0.66];
            let loop_r = [M(0.0, 0.0), B(0.3, -0.45, 0.75, -0.35, 0.7, 0.0), B(0.75, 0.35, 0.3, 0.45, 0.0, 0.0), Z];
            let tail_r = [M(0.04, 0.08), L(0.3, 0.62), L(0.12, 0.56), L(0.0, 0.16), Z];
            let tails = p.shape("bow_tails", |p| build_path(p.f, &[tail_r.to_vec(), mirror(&tail_r)].concat()))?;
            let loops = p.shape("bow_loops", |p| build_path(p.f, &[loop_r.to_vec(), mirror(&loop_r)].concat()))?;
            p.obj(f, k, &tails, mix(pink, INK, 0.12));
            p.obj(f, k, &loops, pink);
            p.fill_el(f, 0.0, 0.0, 0.17, 0.17, mix(pink, INK, 0.08), 1.0);
            p.ring(f, 0.0, 0.0, 0.17, 0.17, edge(pink), 1.0, lw / k);
        }
        // Scarf: wrap under the paws, a tail fluttering over them.
        (16, Mid) => {
            let red = [0.9, 0.26, 0.3];
            let scarf = p.path("scarf", &[M(-0.43, 0.25), B(-0.22, 0.41, 0.22, 0.41, 0.43, 0.25), L(0.42, 0.35), B(0.22, 0.5, -0.22, 0.5, -0.42, 0.35), Z])?;
            p.obj(h, 1.0, &scarf, red);
            let knit = p.path("scarf_line", &[M(-0.42, 0.3), B(-0.22, 0.455, 0.22, 0.455, 0.42, 0.3), O])?;
            p.stroke(h, &knit, WHITE, 0.55, 0.02);
        }
        (16, Front) => {
            let red = [0.86, 0.22, 0.27];
            let m = rot((t * 2.5).sin() * 8.0, 0.26, 0.4) * h;
            let tail = p.path("scarf_tail", &[M(0.2, 0.38), L(0.32, 0.4), L(0.37, 0.66), L(0.23, 0.67), Z])?;
            p.obj(m, 1.0, &tail, red);
            p.seg(m, 0.23, 0.56, 0.36, 0.555, WHITE, 0.55, 0.02);
            for j in 0..4 {
                let x = 0.245 + j as f32 * 0.04;
                p.seg(m, x, 0.672, x, 0.71, red, 1.0, 0.016);
            }
        }
        // Headphones; the cups pulse and little notes float out.
        (17, Front) => {
            let dark = [0.26, 0.26, 0.34];
            let band = p.path("hp_band", &[M(-0.47, -0.04), B(-0.47, -0.52, 0.47, -0.52, 0.47, -0.04), O])?;
            p.stroke(h, &band, edge(dark), 1.0, 0.085);
            p.stroke(h, &band, dark, 1.0, 0.058);
            let cup = p.path("hp_cup", &rrect(-0.075, -0.13, 0.075, 0.13, 0.06))?;
            let pad = p.path("hp_pad", &rrect(-0.035, -0.09, 0.035, 0.09, 0.03))?;
            let beat = 1.0 + 0.06 * (t * 5.0).sin().max(0.0);
            let accent = [1.0, 0.45, 0.66];
            for side in [-1.0f32, 1.0] {
                let m = put(0.49 * side, 0.05, beat, 0.0) * h;
                p.obj(m, beat, &cup, dark);
                p.fill(m, &pad, accent, 1.0);
            }
            if !p.small {
                for j in 0..2 {
                    let u = (t * 0.5 + j as f32 * 0.5).fract();
                    let side = if j == 0 { 1.0 } else { -1.0 };
                    let (x, y) = (side * (0.62 + u * 0.14), -0.02 - u * 0.5);
                    let a = (u * PI).sin();
                    p.fill_el(rot(-20.0, x, y) * h, x, y, 0.034, 0.026, accent, a);
                    p.seg(h, x + 0.03, y - 0.005, x + 0.03, y - 0.11, accent, a, 0.016);
                    p.seg(h, x + 0.03, y - 0.11, x + 0.07, y - 0.08, accent, a, 0.016);
                }
            }
        }
        // Halo bobbing above the head.
        (18, Front) => {
            let y = -0.76 + (t * 2.0).sin() * 0.03;
            p.glow(h, 0.0, y, 0.38, 0.17, [1.0, 0.9, 0.5], 0.5)?;
            p.ring(h, 0.0, y, 0.24, 0.065, edge(GOLD), 1.0, 0.065);
            p.ring(h, 0.0, y, 0.24, 0.065, GOLD, 1.0, 0.042);
            p.ring(h, 0.0, y - 0.008, 0.22, 0.05, WHITE, 0.45, 0.012);
        }
        // Sunglasses with a glint now and then.
        (19, Front) => {
            let lens_r = [M(0.04, 0.0), L(0.34, -0.015), Q(0.34, 0.14, 0.2, 0.16), Q(0.06, 0.155, 0.04, 0.0), Z];
            let lenses = p.shape("shades", |p| build_path(p.f, &[lens_r.to_vec(), mirror(&lens_r)].concat()))?;
            let tint = p.linear(0.0, -0.02, 0.0, 0.16, &[(0.0, [0.08, 0.08, 0.12], 1.0), (1.0, [0.28, 0.2, 0.45], 1.0)])?;
            p.fill_with(h, &lenses, &tint);
            p.stroke(h, &lenses, INK, 1.0, lw);
            p.seg(h, -0.36, -0.01, 0.36, -0.01, INK, 1.0, 0.035);
            for side in [-1.0f32, 1.0] {
                p.seg(h, 0.1 * side, 0.11, 0.16 * side, 0.03, WHITE, 0.3, 0.022);
            }
            let u = (t * 0.4).fract();
            if u < 0.25 {
                p.spark(h, 0.28, 0.02, 0.075 * (u / 0.25 * PI).sin(), WHITE, 1.0)?;
            }
        }
        // Party hat with a bouncing pompom.
        (20, Front) => {
            let f = rot(14.0, 0.04, -0.27) * h;
            let cone = p.path("party", &[M(-0.14, -0.27), L(0.05, -0.74), L(0.22, -0.26), Q(0.04, -0.22, -0.14, -0.27), Z])?;
            p.obj(f, 1.0, &cone, [0.4, 0.72, 1.0]);
            p.fill_el(f, 0.04, -0.36, 0.03, 0.03, [1.0, 0.88, 0.35], 1.0);
            p.fill_el(f, 0.08, -0.5, 0.022, 0.022, [1.0, 0.45, 0.66], 1.0);
            p.fill_el(f, 0.045, -0.6, 0.017, 0.017, WHITE, 1.0);
            p.fill_el(f, -0.04, -0.44, 0.016, 0.016, WHITE, 1.0);
            let r = 0.055 * (1.0 + 0.1 * (t * 6.0).sin());
            p.obj_el(f, 0.05, -0.76, r, r, [1.0, 0.45, 0.66]);
        }
        // Witch hat with a twinkle at the tip.
        (21, Front) => {
            let f = rot(-6.0, 0.0, -0.3) * h;
            let purple = [0.36, 0.22, 0.5];
            p.obj_el(f, 0.0, -0.3, 0.42, 0.085, purple);
            let cone = p.path("witch", &[M(-0.22, -0.32), Q(-0.08, -0.55, 0.05, -0.8), Q(0.16, -0.86, 0.32, -0.76), Q(0.14, -0.72, 0.11, -0.62), Q(0.15, -0.45, 0.22, -0.32), Q(0.0, -0.29, -0.22, -0.32), Z])?;
            p.obj(f, 1.0, &cone, purple);
            let band = p.clip("witch_band", &cone, |p| build_path(p.f, &[M(-0.3, -0.33), L(-0.3, -0.41), L(0.3, -0.41), L(0.3, -0.33), Z]))?;
            p.fill(f, &band, [1.0, 0.62, 0.22], 1.0);
            let buckle = p.path("witch_buckle", &rrect(-0.04, -0.405, 0.04, -0.335, 0.012))?;
            p.stroke(f, &buckle, GOLD, 1.0, 0.018);
            p.spark(f, 0.36, -0.82, 0.07 * (t * 2.6).sin().max(0.0), [1.0, 0.95, 0.6], 1.0)?;
        }
        // Flower crown.
        (22, Front) => {
            let leaf = p.path("leaf", LEAF)?;
            let colours = [[1.0, 0.62, 0.78], [1.0, 0.88, 0.4], [0.96, 0.94, 1.0], [1.0, 0.7, 0.55], [0.8, 0.66, 1.0]];
            let pos = |u: f32| {
                let x = -0.38 + 0.76 * u;
                (x, -0.315 + 0.19 * (x / 0.38).powi(2))
            };
            for j in 0..4 {
                let (x, y) = pos((j as f32 + 0.5) / 4.0);
                p.obj(put(x, y - 0.01, 0.1, if j % 2 == 0 { -30.0 } else { 30.0 }) * h, 0.1, &leaf, [0.45, 0.78, 0.4]);
            }
            for (j, c) in colours.iter().enumerate() {
                let (x, y) = pos(j as f32 / 4.0);
                let bob = (t * 2.0 + j as f32).sin() * 0.008;
                p.flower(h, x, y + bob, 0.075, j as f32 * 20.0, *c, [1.0, 0.86, 0.35]);
            }
        }
        // Devil horns.
        (23, Front) => {
            // Between the ears, curling outwards.
            let horn_r = [M(0.02, -0.29), Q(0.02, -0.46, 0.15, -0.54), Q(0.11, -0.43, 0.15, -0.28), Z];
            let horns = p.shape("horns", |p| build_path(p.f, &[horn_r.to_vec(), mirror(&horn_r)].concat()))?;
            let red = p.linear(0.0, -0.54, 0.0, -0.25, &[(0.0, [1.0, 0.4, 0.35], 1.0), (1.0, [0.75, 0.1, 0.16], 1.0)])?;
            p.fill_with(h, &horns, &red);
            p.stroke(h, &horns, edge([0.8, 0.15, 0.2]), 1.0, lw);
        }
        // A little sprout, swaying.
        (24, Front) => {
            let f = rot((t * 1.8).sin() * 12.0, 0.0, 0.0) * tr(0.0, -0.3) * h;
            let green = [0.45, 0.82, 0.35];
            let stem = p.path("sprout_stem", &[M(0.0, 0.0), Q(0.03, -0.1, 0.0, -0.2), O])?;
            p.stroke(f, &stem, edge(green), 1.0, 0.045);
            p.stroke(f, &stem, green, 1.0, 0.028);
            let leaf = p.path("leaf", LEAF)?;
            p.obj(put(-0.061, -0.235, 0.15, -150.0) * f, 0.15, &leaf, green);
            p.obj(put(0.061, -0.235, 0.15, -30.0) * f, 0.15, &leaf, mix(green, WHITE, 0.15));
        }
        // Knit beanie with a bouncing pompom.
        (25, Front) => {
            let orange = [0.96, 0.56, 0.26];
            let dome = p.path("beanie", &[M(-0.42, -0.12), B(-0.42, -0.5, 0.42, -0.5, 0.42, -0.12), Z])?;
            p.obj(h, 1.0, &dome, orange);
            for x in [-0.2f32, 0.0, 0.2] {
                p.seg(h, x, -0.38 + x.abs() * 0.3, x * 1.1, -0.2, edge(orange), 0.35, 0.014);
            }
            let cuff = p.path("beanie_cuff", &rrect(-0.43, -0.21, 0.43, -0.09, 0.045))?;
            p.obj(h, 1.0, &cuff, mix(orange, INK, 0.12));
            for j in 0..9 {
                let x = -0.34 + j as f32 * 0.085;
                p.seg(h, x, -0.19, x, -0.11, edge(orange), 0.35, 0.014);
            }
            let y = -0.44 - (t * 3.0).sin().abs() * 0.025;
            p.obj_el(h, 0.0, y, 0.07, 0.07, [1.0, 0.95, 0.88]);
        }
        // Knight helmet: a polished great helm with a centre ridge, a riveted
        // rim, a barred visor and pierced cheek guards, crowned by a big red
        // plume of three feathers sweeping back from a gold holder.
        (HELMET, Front) => {
            let ink = [0.2, 0.21, 0.27];
            let sway = (t * 2.2).sin() * 4.0;
            // Back to front: the deepest red feather sits behind.
            let feathers: [(&'static str, (f32, f32), (f32, f32), f32, Rgb, f32); 3] = [
                ("helm_f3", (-0.18, -0.98), (-0.74, -0.52), 0.17, [0.68, 0.05, 0.12], 1.6),
                ("helm_f2", (-0.04, -1.08), (-0.6, -0.84), 0.18, [0.84, 0.1, 0.17], 0.8),
                ("helm_f1", (0.12, -1.08), (-0.36, -1.02), 0.16, [0.98, 0.26, 0.3], 0.0),
            ];
            let base = (0.0f32, -0.6f32);
            let q = |a: f32, b: f32, c: f32, u: f32| (1.0 - u) * (1.0 - u) * a + 2.0 * (1.0 - u) * u * b + u * u * c;
            for (k, &(key, c, tip, w, col, lag)) in feathers.iter().enumerate() {
                let f = rot(sway * (1.0 + 0.3 * k as f32) + (t * 2.2 - lag).sin() * 2.0, base.0, base.1) * h;
                // Spine and width at u in 0..1; the edge is cut into soft notches.
                let spine = |u: f32| (q(base.0, c.0, tip.0, u), q(base.1, c.1, tip.1, u));
                let g = p.shape(key, |p| {
                    const N: usize = 22;
                    let mut left = Vec::with_capacity(N + 1);
                    let mut right = Vec::with_capacity(N + 1);
                    for j in 0..=N {
                        let u = j as f32 / N as f32;
                        let (x, y) = spine(u);
                        let (x2, y2) = spine((u + 0.01).min(1.0));
                        let (x1, y1) = spine((u - 0.01).max(0.0));
                        let (dx, dy) = (x2 - x1, y2 - y1);
                        let len = (dx * dx + dy * dy).sqrt().max(1e-5);
                        let (nx, ny) = (-dy / len, dx / len);
                        let mut r = w * u.powf(0.35) * (1.0 - u.powi(4)).max(0.0).sqrt() + 0.035 * (1.0 - u);
                        if j % 4 == 2 && j > 4 && j < N - 2 {
                            r *= 0.86;
                        }
                        left.push((x + nx * r, y + ny * r));
                        right.push((x - nx * r * 0.8, y - ny * r * 0.8));
                    }
                    let mut v = vec![M(left[0].0, left[0].1)];
                    v.extend(left.iter().skip(1).map(|&(x, y)| L(x, y)));
                    v.extend(right.iter().rev().map(|&(x, y)| L(x, y)));
                    v.push(Z);
                    build_path(p.f, &v)
                })?;
                p.at(f);
                let (tx, ty) = spine(1.0);
                let b = p.linear(base.0, base.1, tx, ty, &[(0.0, mix(col, INK, 0.35), 1.0), (0.45, col, 1.0), (1.0, mix(col, WHITE, 0.25), 1.0)])?;
                p.fill_with(f, &g, &b);
                // Barbs: fine strokes from the spine sweeping out and back.
                if !p.small {
                    for j in 2..11 {
                        let u = j as f32 / 12.0;
                        let (x, y) = spine(u);
                        let (x2, y2) = spine(u + 0.06);
                        let r = w * u.powf(0.35) * (1.0 - u.powi(4)).sqrt() * 0.8;
                        let (dx, dy) = (x2 - x, y2 - y);
                        let len = (dx * dx + dy * dy).sqrt().max(1e-5);
                        let (nx, ny) = (-dy / len, dx / len);
                        p.seg(f, x, y, x + nx * r + dx * 0.6, y + ny * r + dy * 0.6, mix(col, INK, 0.45), 0.55, p.lw * 0.55);
                        p.seg(f, x, y, x - nx * r * 0.7 + dx * 0.6, y - ny * r * 0.7 + dy * 0.6, mix(col, INK, 0.45), 0.45, p.lw * 0.5);
                    }
                    // The quill, with a sheen beside it.
                    for j in 0..10 {
                        let (x0, y0) = spine(j as f32 / 10.0);
                        let (x1, y1) = spine((j + 1) as f32 / 10.0);
                        p.seg(f, x0, y0, x1, y1, mix(col, WHITE, 0.55), 0.8, p.lw * 0.6);
                    }
                }
                p.stroke(f, &g, [0.38, 0.03, 0.07], 1.0, p.lw * 0.9);
            }

            // The shell: a dome down over the ears, with cheek guards and an open face.
            let steel = p.radial(-0.2, -0.36, 0.8, 0.75, &[(0.0, WHITE, 1.0), (0.5, [0.87, 0.89, 0.94], 1.0), (1.0, [0.56, 0.6, 0.69], 1.0)], 1.0)?;
            let shell = p.path(
                "helm_shell",
                &[
                    M(-0.53, 0.1),
                    B(-0.62, -0.42, -0.3, -0.58, 0.0, -0.58),
                    B(0.3, -0.58, 0.62, -0.42, 0.53, 0.1),
                    Q(0.46, 0.15, 0.41, 0.08),
                    L(0.41, -0.02),
                    B(0.26, -0.1, -0.26, -0.1, -0.41, -0.02),
                    L(-0.41, 0.08),
                    Q(-0.46, 0.15, -0.53, 0.1),
                    Z,
                ],
            )?;
            p.fill_with(h, &shell, &steel);
            // Darker steel down the sides, so the dome looks round.
            p.at(h);
            let side_shade = p.linear(-0.55, 0.0, 0.55, 0.0, &[(0.0, [0.4, 0.43, 0.52], 0.45), (0.25, [0.4, 0.43, 0.52], 0.0), (0.75, [0.4, 0.43, 0.52], 0.0), (1.0, [0.4, 0.43, 0.52], 0.55)])?;
            p.fill_with(h, &shell, &side_shade);
            // Rim band along the lower edge.
            let rim = p.path("helm_rim", &[M(-0.5, -0.02), B(-0.3, -0.12, 0.3, -0.12, 0.5, -0.02), O])?;
            p.stroke(h, &rim, [0.52, 0.55, 0.64], 0.7, 0.06);
            p.stroke(h, &rim, WHITE, 0.5, 0.012);
            p.stroke(h, &shell, ink, 1.0, p.lw * 1.25);

            // Centre ridge (comb) from the brow over the crown.
            let comb = p.path("helm_comb", &[M(-0.035, -0.29), Q(-0.04, -0.5, -0.03, -0.59), L(0.03, -0.59), Q(0.04, -0.5, 0.035, -0.29), Z])?;
            p.at(h);
            let cb = p.linear(-0.04, 0.0, 0.04, 0.0, &[(0.0, [0.7, 0.73, 0.8], 1.0), (0.4, WHITE, 1.0), (1.0, [0.62, 0.65, 0.74], 1.0)])?;
            p.fill_with(h, &comb, &cb);
            p.stroke(h, &comb, ink, 0.8, p.lw * 0.7);

            // The visor with its breathing bars.
            let visor = p.path("helm_visor", &[M(-0.4, -0.05), B(-0.25, -0.13, 0.25, -0.13, 0.4, -0.05), L(0.4, -0.2), B(0.25, -0.3, -0.25, -0.3, -0.4, -0.2), Z])?;
            let vis = p.linear(0.0, -0.3, 0.0, -0.05, &[(0.0, [0.96, 0.97, 1.0], 1.0), (0.6, [0.8, 0.83, 0.89], 1.0), (1.0, [0.66, 0.7, 0.78], 1.0)])?;
            p.fill_with(h, &visor, &vis);
            for k in -4..=4 {
                let x = k as f32 * 0.075;
                let bow = 1.0 - (x / 0.4) * (x / 0.4);
                let (top, bottom) = (-0.2 - 0.075 * bow + 0.035, -0.05 - 0.06 * bow - 0.03);
                p.seg(h, x, top, x, bottom, ink, 0.92, 0.03);
                if !p.small {
                    p.seg(h, x + 0.017, top + 0.005, x + 0.017, bottom - 0.005, WHITE, 0.6, 0.008);
                }
            }
            let lip = p.path("helm_visor_lip", &[M(-0.4, -0.2), B(-0.25, -0.3, 0.25, -0.3, 0.4, -0.2), O])?;
            p.stroke(h, &lip, WHITE, 0.75, 0.016);
            p.stroke(h, &visor, ink, 1.0, p.lw);

            // Rivets round the dome and along the brow.
            if !p.small {
                for j in 0..9 {
                    let a = PI * (1.12 + 0.76 * j as f32 / 8.0);
                    let (x, y) = (a.cos() * 0.47, -0.07 + a.sin() * 0.44);
                    if x.abs() < 0.06 {
                        continue;
                    }
                    p.fill_el(h, x, y, 0.018, 0.018, [0.74, 0.77, 0.84], 1.0);
                    p.ring(h, x, y, 0.018, 0.018, ink, 0.8, p.lw * 0.5);
                    p.fill_el(h, x - 0.005, y - 0.006, 0.006, 0.006, WHITE, 0.95);
                }
                // Pierced cheek guards.
                for side in [-1.0f32, 1.0] {
                    for (k, y) in [0.0f32, 0.045].into_iter().enumerate() {
                        for i in 0..2 {
                            let x = (0.455 + i as f32 * 0.03 - k as f32 * 0.012) * side;
                            p.fill_el(h, x, y, 0.009, 0.009, ink, 0.85);
                        }
                    }
                }
            }

            // Shine on the dome.
            let hi = p.path("helm_hi", &[M(-0.42, -0.26), Q(-0.38, -0.46, -0.17, -0.53), O])?;
            p.stroke(h, &hi, WHITE, 0.9, 0.04);
            p.fill_el(h, -0.25, -0.47, 0.025, 0.018, WHITE, 0.9);
            let hi2 = p.path("helm_hi2", &[M(0.3, -0.48), Q(0.42, -0.4, 0.46, -0.28), O])?;
            p.stroke(h, &hi2, WHITE, 0.45, 0.02);
            // Hinges where the visor meets the shell.
            for side in [-1.0f32, 1.0] {
                p.fill_el(h, 0.45 * side, -0.12, 0.045, 0.045, [0.8, 0.82, 0.88], 1.0);
                p.ring(h, 0.45 * side, -0.12, 0.045, 0.045, ink, 1.0, p.lw * 0.9);
                p.seg(h, 0.45 * side - 0.025, -0.12, 0.45 * side + 0.025, -0.12, ink, 0.7, p.lw * 0.6);
                p.fill_el(h, 0.45 * side - 0.012, -0.135, 0.012, 0.012, WHITE, 0.9);
            }
            // The gold plume holder on the crest.
            let holder = p.path("helm_holder", &rrect(-0.06, -0.655, 0.06, -0.56, 0.025))?;
            p.at(h);
            let gold = p.linear(-0.06, 0.0, 0.06, 0.0, &[(0.0, mix(GOLD, INK, 0.3), 1.0), (0.4, mix(GOLD, WHITE, 0.4), 1.0), (1.0, mix(GOLD, INK, 0.2), 1.0)])?;
            p.fill_with(h, &holder, &gold);
            p.stroke(h, &holder, edge(GOLD), 1.0, p.lw * 0.8);
            p.seg(h, -0.06, -0.6, 0.06, -0.6, edge(GOLD), 0.7, p.lw * 0.6);
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symmetric_outline() {
        let full = sym(&[M(0.0, -1.0), L(1.0, 0.0), B(1.0, 0.5, 0.5, 1.0, 0.0, 1.0)]);
        assert_eq!(full, vec![M(0.0, -1.0), L(1.0, 0.0), B(1.0, 0.5, 0.5, 1.0, 0.0, 1.0), B(-0.5, 1.0, -1.0, 0.5, -1.0, 0.0), L(0.0, -1.0), Z]);
        assert_eq!(mirror(&[M(1.0, 2.0), Q(3.0, 4.0, 5.0, 6.0)]), vec![M(-1.0, 2.0), Q(-3.0, 4.0, -5.0, 6.0)]);
    }

    #[test]
    fn indices_wrap_and_names_are_unique() {
        assert_eq!(get(SKINS.len() + 7).name, SKINS[7].name);
        assert_eq!(SKINS[7].name, "Shadow");
        assert_eq!(acc_name(0), "None");
        let mut names: Vec<_> = SKINS.iter().map(|s| s.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), SKINS.len());
        assert!(SKINS.len() >= 40 && ACCESSORIES.len() >= 20);
    }

    #[test]
    fn blink_and_twitch_stay_in_range() {
        for i in 0..2000 {
            let t = i as f32 * 0.013;
            assert!((0.0..=1.0).contains(&blink(t)));
            assert!(twitch(t).1.abs() <= 16.0);
        }
    }
}

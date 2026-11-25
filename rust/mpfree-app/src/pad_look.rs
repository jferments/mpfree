//! Prefs › Appearance › Screen and lights: how the 16 pads light (layout
//! spec section 5). Pure colour rules on the pad colour message's 0–127
//! values; the main loop applies them where it lights the kit's pads.
//!
//! **State-changing on the MPC**, as every pad colour is: the colours go
//! out on the pad colour message Akai's app uses (`akai::pad_colour`), in
//! the controller's RAM only. Most of what these rules make (a pad at 40 %,
//! white, the track's or accent's colour, the heat colours, the dim grey)
//! are colours Akai's app was not seen sending, as the step pads' white and
//! purple are, so the next device round looks at them.

use mpfree_ui::Color;

/// What a held pad shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeldPad {
    /// Its resting colour at full brightness, as Akai's app lights a hit
    /// (the default; `akai::hit_colour`).
    #[default]
    Brighter,
    /// White.
    White,
    /// Its resting colour: a held pad doesn't change.
    Same,
}

/// What colour a kit pad with a sound rests in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadsShow {
    /// The kit's colour (the default, as before).
    #[default]
    Kit,
    /// The armed track's colour in the theme in use.
    Track,
    /// The theme's accent.
    Accent,
    /// How hard the pad was hit last: blue soft, through green and amber,
    /// to red hard (the kit's colour until it is hit).
    Heat,
}

/// What a kit pad with no sound on it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmptyPads {
    /// Dark (the default, as before).
    #[default]
    Dark,
    /// A faint grey, so the grid shows.
    Dim,
}

impl HeldPad {
    pub const ALL: [HeldPad; 3] = [HeldPad::Brighter, HeldPad::White, HeldPad::Same];
    pub fn name(self) -> &'static str {
        ["Brighter", "White", "Same"][self as usize]
    }
    pub fn key(self) -> &'static str {
        ["brighter", "white", "same"][self as usize]
    }
    pub fn parse(v: &str) -> Option<HeldPad> {
        HeldPad::ALL.into_iter().find(|h| h.key() == v.trim())
    }
}

impl PadsShow {
    pub const ALL: [PadsShow; 4] = [
        PadsShow::Kit,
        PadsShow::Track,
        PadsShow::Accent,
        PadsShow::Heat,
    ];
    pub fn name(self) -> &'static str {
        ["Kit colour", "Track", "Accent", "Hit heat"][self as usize]
    }
    /// The line under its button.
    pub fn line(self) -> &'static str {
        ["each kit's", "the track's", "the theme's", "how hard"][self as usize]
    }
    pub fn key(self) -> &'static str {
        ["kit", "track", "accent", "heat"][self as usize]
    }
    pub fn parse(v: &str) -> Option<PadsShow> {
        PadsShow::ALL.into_iter().find(|s| s.key() == v.trim())
    }
}

impl EmptyPads {
    pub const ALL: [EmptyPads; 2] = [EmptyPads::Dark, EmptyPads::Dim];
    pub fn name(self) -> &'static str {
        ["Dark", "Dim"][self as usize]
    }
    pub fn key(self) -> &'static str {
        ["dark", "dim"][self as usize]
    }
    pub fn parse(v: &str) -> Option<EmptyPads> {
        EmptyPads::ALL.into_iter().find(|e| e.key() == v.trim())
    }
}

/// The lightest and darkest Pad light brightness, in percent. Never 0:
/// lit pads always show.
pub const BRIGHTNESS_MIN: u8 = 10;
pub const BRIGHTNESS_MAX: u8 = 100;

/// A resting colour's brightest value (Akai's app rests pads in 0–63 and
/// lights a hit in 0–127).
const REST_MAX: u8 = 0x3F;
/// A pad with no sound, with Empty pads Dim.
pub const DIM: [u8; 3] = [0x06, 0x06, 0x06];
/// A held pad, with A held pad White.
pub const WHITE: [u8; 3] = [0x7F, 0x7F, 0x7F];

/// How the pads light, as Prefs sets it, with the colour Track or Accent
/// rests in (already at resting level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadLook {
    pub brightness: u8,
    pub held: HeldPad,
    pub show: PadsShow,
    pub empty: EmptyPads,
    pub tint: [u8; 3],
}

impl Default for PadLook {
    fn default() -> PadLook {
        PadLook {
            brightness: BRIGHTNESS_MAX,
            held: HeldPad::Brighter,
            show: PadsShow::Kit,
            empty: EmptyPads::Dark,
            tint: [0; 3],
        }
    }
}

/// A screen colour (0–255) at a pad's resting level (0–63).
pub fn rest_level(c: Color) -> [u8; 3] {
    [c.r, c.g, c.b].map(|v| v >> 2)
}

/// The pad's colour on the screen for a pad colour (0–127), for Prefs'
/// picture of the pads: a resting pad (up to 63) a little short of full,
/// a lit one (up to 127) full.
pub fn on_screen(rgb: [u8; 3]) -> Color {
    let up = |v: u8| (u16::from(v) * 3).min(255) as u8;
    Color {
        r: up(rgb[0]),
        g: up(rgb[1]),
        b: up(rgb[2]),
        a: 255,
    }
}

/// The heat colour of a hit at `velocity` (1–127), at resting level: blue
/// for the softest, through green and amber, to red for the hardest.
pub fn heat_colour(velocity: u8) -> [u8; 3] {
    let t = f32::from(velocity.clamp(1, 127) - 1) / 126.0;
    let hue = 230.0 * (1.0 - t);
    let x = 1.0 - ((hue / 60.0) % 2.0 - 1.0).abs();
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        _ => (x, 0.0, 1.0),
    };
    let v = |c: f32| (c * f32::from(REST_MAX)).round() as u8;
    [v(r), v(g), v(b)]
}

/// The kit's 16 pads at rest: `has` says which have a sound, `kit` is the
/// kit's colour and `heat` each pad's last hit (0: not hit yet).
pub fn rest(has: [bool; 16], kit: [u8; 3], look: &PadLook, heat: &[u8; 16]) -> [[u8; 3]; 16] {
    std::array::from_fn(|p| {
        if !has[p] {
            return match look.empty {
                EmptyPads::Dark => [0; 3],
                EmptyPads::Dim => DIM,
            };
        }
        match look.show {
            PadsShow::Kit => kit,
            PadsShow::Track | PadsShow::Accent => look.tint,
            PadsShow::Heat if heat[p] > 0 => heat_colour(heat[p]),
            PadsShow::Heat => kit,
        }
    })
}

/// A held pad's colour, from its resting colour.
pub fn held(rest: [u8; 3], look: &PadLook) -> [u8; 3] {
    match look.held {
        HeldPad::Brighter => crate::akai::hit_colour(rest),
        HeldPad::White => WHITE,
        HeldPad::Same => rest,
    }
}

/// `rgb` at `brightness` percent: a value that was lit stays at least 1,
/// so a lit pad never goes dark.
pub fn scale(rgb: [u8; 3], brightness: u8) -> [u8; 3] {
    let b = u16::from(brightness.clamp(BRIGHTNESS_MIN, BRIGHTNESS_MAX));
    rgb.map(|v| {
        if v == 0 || b == 100 {
            v
        } else {
            ((u16::from(v) * b + 50) / 100).max(1) as u8
        }
    })
}

/// Prefs' picture of the pads "as they'd light": an example kit (pads 14
/// to 16 empty) with pad 6 held, and, for Hit heat, pads hit from soft to
/// hard.
pub fn example(kit: [u8; 3], look: &PadLook) -> [Color; 16] {
    let has = std::array::from_fn(|p| p < 13);
    let heat: [u8; 16] = std::array::from_fn(|p| if p < 13 { 10 + p as u8 * 9 } else { 0 });
    let rest = rest(has, kit, look, &heat);
    std::array::from_fn(|p| {
        let c = if p == 5 { held(rest[p], look) } else { rest[p] };
        on_screen(scale(c, look.brightness))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIT: [u8; 3] = [0x3F, 0x22, 0x00];

    #[test]
    fn the_defaults_light_the_pads_as_before() {
        let look = PadLook::default();
        let has = std::array::from_fn(|p| p % 3 != 0);
        let rest = rest(has, KIT, &look, &[0; 16]);
        for p in 0..16 {
            assert_eq!(rest[p], if has[p] { KIT } else { [0; 3] });
            assert_eq!(held(rest[p], &look), crate::akai::hit_colour(rest[p]));
            assert_eq!(scale(rest[p], look.brightness), rest[p]);
        }
    }

    #[test]
    fn each_choice_changes_what_the_pads_show() {
        let has = std::array::from_fn(|p| p < 8);
        let mut heat = [0; 16];
        heat[1] = 1;
        heat[2] = 127;
        let look = PadLook {
            show: PadsShow::Heat,
            empty: EmptyPads::Dim,
            held: HeldPad::White,
            ..PadLook::default()
        };
        let r = rest(has, KIT, &look, &heat);
        assert_eq!(r[0], KIT, "not hit yet: the kit's colour");
        assert_eq!(r[1], heat_colour(1));
        assert!(r[1][2] > r[1][0], "soft is blue: {:?}", r[1]);
        assert!(r[2][0] > r[2][2] && r[2][1] < 8, "hard is red: {:?}", r[2]);
        assert_eq!(r[9], DIM, "no sound: dim");
        assert_eq!(held(r[0], &look), WHITE);
        let tint = [0x10, 0x20, 0x30];
        for show in [PadsShow::Track, PadsShow::Accent] {
            let look = PadLook {
                show,
                tint,
                held: HeldPad::Same,
                ..PadLook::default()
            };
            let r = rest(has, KIT, &look, &heat);
            assert_eq!((r[0], r[9]), (tint, [0; 3]));
            assert_eq!(held(r[0], &look), tint);
        }
    }

    #[test]
    fn brightness_scales_every_colour_but_keeps_lit_pads_lit() {
        assert_eq!(scale([0x7F, 0x40, 0x00], 50), [0x40, 0x20, 0x00]);
        assert_eq!(scale([0x01, 0x02, 0x00], 10), [0x01, 0x01, 0x00]);
        assert_eq!(
            scale([0x3F, 0x3F, 0x3F], 0),
            scale([0x3F; 3], BRIGHTNESS_MIN)
        );
        // Every colour stays in the message's 0-127.
        for v in [1u8, 63, 127] {
            assert!(heat_colour(v).iter().all(|&c| c <= REST_MAX));
            assert!(crate::akai::hit_colour(heat_colour(v))
                .iter()
                .all(|&c| c <= 0x7F));
        }
    }

    #[test]
    fn names_and_keys_read_back() {
        for h in HeldPad::ALL {
            assert_eq!(HeldPad::parse(h.key()), Some(h));
        }
        for s in PadsShow::ALL {
            assert_eq!(PadsShow::parse(s.key()), Some(s));
        }
        for e in EmptyPads::ALL {
            assert_eq!(EmptyPads::parse(e.key()), Some(e));
        }
        assert_eq!(PadsShow::parse("purple"), None);
    }
}

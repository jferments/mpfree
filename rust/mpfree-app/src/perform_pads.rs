//! The Perform pads: pages of pad actions, kept apart from the kits'
//! sounds. A kit's pad is a sound (a sample, a slice, an instrument's
//! voice), sequenced on its track and saved with the kit; a Perform pad is
//! an action on the looper or the mix, which belongs to the project and
//! not to any track (Jay, 17 Nov 23:52Z: "separating throws/FX/loop
//! keys/etc from 'sound' pads").
//!
//! The project has one **Perform set**: eight pages (PAD BANK A to H) of
//! 16 pads each (the Play, Pads and Sequencer redesign's step 6, board 7).
//! Page A is the looper's **loop keys** (the Looper tab as an instrument,
//! step 2); page B starts with Play's eight throws; the others start
//! empty. SHIFT + SAMPLER on the Looper tab puts the set on the pads at
//! the page shown last, the Looper's Pads switch at page A, and they stay
//! Perform pads on the Looper tab and Play. The set is saved with the
//! project ([`PerformSet::text`], the `perform` part).

use crate::looper_deck::RingState;
use mpfree_looper::MAX_LOOPS;

/// Pads on a page.
pub const PADS: usize = 16;
/// The `perform` part's first line: its format and version.
pub const FORMAT: &str = "mpfree-perform 1";
/// Pages in the set: PAD BANK A to H.
pub const PAGES: usize = 8;

/// Play's throws by their place in [`crate::play_page::THROW_LIST`], as
/// the project saves them: a slug that a renamed throw keeps.
pub const THROW_SLUGS: [&str; 8] = [
    "delay-throw",
    "reverb-wash",
    "stutter",
    "tape-stop",
    "filter-dip",
    "beat-repeat",
    "drop-drums",
    "vinyl-brake",
];

/// How a throw pad holds its effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hold {
    /// While the pad is held, as Play's throw cards.
    #[default]
    WhileHeld,
    /// A hit starts it and the next hit lets it go.
    Latch,
}

/// A throw on a Perform pad: Play's throw at `index` in
/// [`crate::play_page::THROW_LIST`]. (Where it acts, when it starts and
/// how it lets go, board 7's On, Starts and Let go, need the mixer's
/// throws to take them first: they play on the master, at once, and ring
/// out, as Play's do.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Throw {
    pub index: u8,
    pub hold: Hold,
}

impl Throw {
    /// Its name on Play's card ("Reverb wash").
    pub fn name(self) -> &'static str {
        crate::play_page::THROW_LIST
            .get(usize::from(self.index))
            .map_or("Throw", |t| t.name)
    }
}

/// What a Perform pad does. New kinds of action are new variants; a pad
/// with nothing on it is [`PadAction::None`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadAction {
    /// Nothing: the pad is dark and a hit does nothing.
    #[default]
    None,
    /// Loop `n`'s key (0 to 7, Loop 1 to 8): a hit is the loop's next
    /// step, as a tap on its ring on the Looper tab (record from the next
    /// bar line, cancel, end the first pass, overdub, play, unmute); held
    /// for a second it clears the loop.
    LoopKey(u8),
    /// Loop `n`'s mute (0 to 7): a hit mutes or unmutes it; with SHIFT
    /// it solos it or lets go.
    LoopMute(u8),
    /// One of Play's throws, held or latched.
    Throw(Throw),
    /// Track `t`'s mute (0 is Track 1): a hit mutes or unmutes it, at the
    /// launch line in Perform, as TRACK MUTE's pads do.
    Mute(u8),
    /// Scene row `n` (0 is Scene 1): a hit launches it, as the grid's
    /// scene button.
    Scene(u8),
}

/// A page of Perform pads: its name, and what each pad does, pad 1 (bottom
/// left) first, as the kits number their pads.
#[derive(Clone, Debug, PartialEq)]
pub struct PerformPage {
    pub name: String,
    pub pads: [PadAction; PADS],
}

impl PerformPage {
    /// The loop keys. Pads 13 to 16 (the top row, where the eye goes
    /// first) are Loops 1 to 4's keys and pads 9 to 12 Loops 5 to 8's;
    /// pads 5 to 8 and 1 to 4 are their mutes, each two rows under its
    /// key, so a column of pads belongs to one loop.
    pub fn loop_keys() -> PerformPage {
        let mut pads = [PadAction::None; PADS];
        for k in 0..4u8 {
            let c = usize::from(k);
            pads[12 + c] = PadAction::LoopKey(k);
            pads[8 + c] = PadAction::LoopKey(4 + k);
            pads[4 + c] = PadAction::LoopMute(k);
            pads[c] = PadAction::LoopMute(4 + k);
        }
        PerformPage {
            name: "Loop keys".to_string(),
            pads,
        }
    }

    /// Play's eight throws on pads 1 to 8 (board 7's page B), each held
    /// while the pad is.
    pub fn throws() -> PerformPage {
        let mut pads = [PadAction::None; PADS];
        for (i, p) in pads.iter_mut().take(THROW_SLUGS.len()).enumerate() {
            *p = PadAction::Throw(Throw {
                index: i as u8,
                hold: Hold::WhileHeld,
            });
        }
        PerformPage {
            name: "Throws".to_string(),
            pads,
        }
    }

    /// A page with nothing on it.
    pub fn empty() -> PerformPage {
        PerformPage {
            name: String::new(),
            pads: [PadAction::None; PADS],
        }
    }

    /// Pad `pad`'s action (0 is pad 1); None past the page.
    pub fn action(&self, pad: usize) -> PadAction {
        self.pads.get(pad).copied().unwrap_or_default()
    }

    /// The pad that is loop `n`'s key, if the page has one.
    #[cfg(test)]
    pub fn key_of(&self, n: u8) -> Option<usize> {
        self.pads.iter().position(|&a| a == PadAction::LoopKey(n))
    }
}

/// The project's Perform set: its eight pages, A first.
#[derive(Clone, Debug, PartialEq)]
pub struct PerformSet {
    pub pages: [PerformPage; PAGES],
}

impl Default for PerformSet {
    /// A new project's: the loop keys on page A, Play's throws on page B,
    /// the rest empty.
    fn default() -> PerformSet {
        let mut pages: [PerformPage; PAGES] = std::array::from_fn(|_| PerformPage::empty());
        pages[0] = PerformPage::loop_keys();
        pages[1] = PerformPage::throws();
        PerformSet { pages }
    }
}

/// A pad's name in the text: its page's letter and its number from 1
/// ("B2").
fn pad_label(page: usize, pad: usize) -> String {
    format!("{}{}", (b'A' + page as u8) as char, pad + 1)
}

impl PerformSet {
    /// Page `page` (0 is A), or A past the end.
    pub fn page(&self, page: usize) -> &PerformPage {
        self.pages.get(page).unwrap_or(&self.pages[0])
    }

    /// Page `page`'s name for the log and the screen: its own, else
    /// "Page C".
    pub fn page_name(&self, page: usize) -> String {
        let p = page.min(PAGES - 1);
        let name = &self.pages[p].name;
        if name.is_empty() {
            format!("Page {}", (b'A' + p as u8) as char)
        } else {
            format!("Page {} · {name}", (b'A' + p as u8) as char)
        }
    }

    /// The `perform` part: its format line, then each page's name and
    /// each pad with an action, a line each. Numbers are as the screen
    /// shows them, from 1 (loop 1, track 1, scene 1); a throw goes by its
    /// slug, then its settings as `key=value` (`hold=held` or
    /// `hold=latch`; where it plays, when it starts and how it lets go
    /// join them as the mixer takes them).
    ///
    /// ```text
    /// mpfree-perform 1
    /// page A = Loop keys
    /// pad A13 = loop-key 1
    /// pad A5 = loop-mute 1
    /// page B = Throws
    /// pad B2 = throw reverb-wash hold=held
    /// pad B8 = throw vinyl-brake hold=latch
    /// pad C1 = mute 3
    /// pad C5 = scene 2
    /// ```
    pub fn text(&self) -> String {
        let mut out = format!("{FORMAT}\n");
        for (k, page) in self.pages.iter().enumerate() {
            if !page.name.is_empty() {
                out.push_str(&format!(
                    "page {} = {}\n",
                    (b'A' + k as u8) as char,
                    page.name
                ));
            }
            for (i, a) in page.pads.iter().enumerate() {
                let what = match *a {
                    PadAction::None => continue,
                    PadAction::LoopKey(n) => format!("loop-key {}", n + 1),
                    PadAction::LoopMute(n) => format!("loop-mute {}", n + 1),
                    PadAction::Throw(t) => {
                        let Some(slug) = THROW_SLUGS.get(usize::from(t.index)) else {
                            continue;
                        };
                        match t.hold {
                            Hold::WhileHeld => format!("throw {slug} hold=held"),
                            Hold::Latch => format!("throw {slug} hold=latch"),
                        }
                    }
                    PadAction::Mute(t) => format!("mute {}", u16::from(t) + 1),
                    PadAction::Scene(n) => format!("scene {}", u16::from(n) + 1),
                };
                out.push_str(&format!("pad {} = {what}\n", pad_label(k, i)));
            }
        }
        out
    }

    /// A `perform` part read back: the set, and a line for each line it
    /// skipped (which the log shows). Pages and pads the text doesn't
    /// name are empty: the part holds the whole set.
    pub fn parse(text: &str) -> (PerformSet, Vec<String>) {
        let mut set = PerformSet {
            pages: std::array::from_fn(|_| PerformPage::empty()),
        };
        let mut skipped = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if line == FORMAT {
                continue;
            }
            if parse_line(&mut set, line).is_none() {
                skipped.push(format!("perform: skipped \"{line}\""));
            }
        }
        (set, skipped)
    }
}

/// The page (0 is A) and pad (0 is pad 1) a label such as "B12" names.
fn parse_label(label: &str) -> Option<(usize, usize)> {
    let mut chars = label.chars();
    let page = chars.next()?.to_ascii_uppercase();
    let page = usize::from(u8::try_from(page).ok()?.checked_sub(b'A')?);
    let pad: usize = chars.as_str().parse().ok()?;
    (page < PAGES && (1..=PADS).contains(&pad)).then_some((page, pad - 1))
}

/// One line of the `perform` part put on `set`; None when it doesn't read.
fn parse_line(set: &mut PerformSet, line: &str) -> Option<()> {
    let (key, value) = line.split_once('=')?;
    let mut key = key.split_whitespace();
    let value = value.trim();
    match (key.next()?, key.next()?) {
        ("page", letter) => {
            let (page, _) = parse_label(&format!("{letter}1"))?;
            set.pages[page].name = value.to_string();
        }
        ("pad", label) => {
            let (page, pad) = parse_label(label)?;
            let mut words = value.split_whitespace();
            let kind = words.next()?;
            let number = |w: Option<&str>, most: u16| -> Option<u8> {
                let n: u16 = w?.parse().ok()?;
                (1..=most).contains(&n).then(|| (n - 1) as u8)
            };
            let action = match kind {
                "loop-key" => PadAction::LoopKey(number(words.next(), MAX_LOOPS as u16)?),
                "loop-mute" => PadAction::LoopMute(number(words.next(), MAX_LOOPS as u16)?),
                "throw" => {
                    let slug = words.next()?;
                    let index = THROW_SLUGS.iter().position(|&s| s == slug)? as u8;
                    let mut hold = Hold::WhileHeld;
                    for w in words.by_ref() {
                        match w.split_once('=')? {
                            ("hold", "held") => hold = Hold::WhileHeld,
                            ("hold", "latch") => hold = Hold::Latch,
                            // The only place a throw plays yet.
                            ("on", "master") => {}
                            _ => return None,
                        }
                    }
                    PadAction::Throw(Throw { index, hold })
                }
                "mute" => PadAction::Mute(number(words.next(), 256)?),
                "scene" => PadAction::Scene(number(words.next(), 256)?),
                _ => return None,
            };
            set.pages[page].pads[pad] = action;
        }
        _ => return None,
    }
    Some(())
}

/// Play's throw at `index` held (`down`) or let go, for the mixer: its
/// performance effect, or for Drop drums the drum pads muted (`drums`,
/// [`drum_pads`]; none once let go).
pub fn throw_fx(index: usize, down: bool, drums: u16) -> Option<crate::mixing::PlayFx> {
    use crate::mixing::PlayFx;
    let def = crate::play_page::THROW_LIST.get(index)?;
    Some(match def.fx {
        Some(fx) => PlayFx::Throw { fx, down },
        None => PlayFx::Drop(if down { drums } else { 0 }),
    })
}

/// The drum pads Drop drums mutes among a kit's first 16 (`names`: each
/// pad's place and sound): the kicks, snares and hats, as Play's Drop
/// drums card has them ([`crate::play_page::PlayPage::drum_pads`]).
pub fn drum_pads<'a>(names: impl Iterator<Item = (usize, &'a str)>) -> u16 {
    use mpfree_ui::draw;
    let mut mask = 0u16;
    for (p, name) in names.filter(|(p, _)| *p < PADS) {
        if matches!(draw::sound_color(name), Some(c) if c == draw::KICK || c == draw::SNARE || c == draw::HAT)
        {
            mask |= 1 << p;
        }
    }
    mask
}

/// What a loop key's light needs to know about its loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopLight {
    pub state: RingState,
    /// The loop's colour, 0 to 255 a channel (the deck's).
    pub colour: [u8; 3],
    /// The loop is in its first beat: a playing loop's key flashes, so
    /// the hands can see where it comes round.
    pub downbeat: bool,
    /// Soloed: its mute pad lights blue.
    pub solo: bool,
}

impl Default for LoopLight {
    fn default() -> LoopLight {
        LoopLight {
            state: RingState::Empty,
            colour: [0; 3],
            downbeat: false,
            solo: false,
        }
    }
}

/// How long a loop key is held to clear its loop, as a ring on the
/// Looper tab.
pub const CLEAR_HOLD_S: f64 = crate::looper_deck::CLEAR_HOLD_S as f64;

/// Each loop's light, from the looper: its state, its colour on the deck,
/// and whether it plays its first beat.
pub fn lights(looper: &crate::looping::LoopHost) -> [LoopLight; MAX_LOOPS] {
    std::array::from_fn(|n| {
        let Some(l) = looper.loop_status(n) else {
            return LoopLight::default();
        };
        let c = crate::looper_tab::LOOP_COLOURS[n % crate::looper_tab::LOOP_COLOURS.len()];
        let beats = l.ticks as f64 / mpfree_transport::time::PPQ as f64;
        LoopLight {
            state: RingState::of(&l, None),
            colour: [c.r, c.g, c.b],
            downbeat: beats > 0.0 && f64::from(l.position) * beats < 1.0,
            solo: l.solo && l.phase != mpfree_looper::Phase::Empty,
        }
    })
}

/// The Perform pads held, as the pad player and the smoke tests keep them:
/// when each went down, whether its hold has cleared its loop, and the
/// action it pressed (a page turned while it is held changes nothing: it
/// lets go of what it pressed).
#[derive(Clone, Debug, Default)]
pub struct Held {
    down: [Option<(f64, bool, PadAction)>; PADS],
}

impl Held {
    /// Pad `pad` went down at `t` seconds (the smoke session's).
    #[cfg(test)]
    pub fn press(&mut self, pad: usize, t: f64) {
        self.press_action(pad, t, PadAction::None);
    }

    /// Pad `pad` went down at `t` seconds, pressing `action`.
    pub fn press_action(&mut self, pad: usize, t: f64, action: PadAction) {
        if let Some(d) = self.down.get_mut(pad) {
            *d = Some((t, false, action));
        }
    }

    /// Pad `pad` let go: true when it was down as a Perform pad (its
    /// release is the Perform pads', not the kit's; the smoke session's).
    #[cfg(test)]
    pub fn release(&mut self, pad: usize) -> bool {
        self.release_action(pad).is_some()
    }

    /// Pad `pad` let go: the action it pressed, if it was down as a
    /// Perform pad.
    pub fn release_action(&mut self, pad: usize) -> Option<PadAction> {
        self.down
            .get_mut(pad)
            .and_then(Option::take)
            .map(|(_, _, a)| a)
    }

    /// The action pad `pad` pressed, while it is held.
    pub fn action(&self, pad: usize) -> Option<PadAction> {
        self.down.get(pad).copied().flatten().map(|(_, _, a)| a)
    }

    /// The pads whose hold reached [`CLEAR_HOLD_S`] by `t`, once each.
    pub fn holds(&mut self, t: f64) -> Vec<usize> {
        let mut out = Vec::new();
        for (pad, d) in self.down.iter_mut().enumerate() {
            if let Some((at, false, a)) = *d {
                if t - at >= CLEAR_HOLD_S {
                    *d = Some((at, true, a));
                    out.push(pad);
                }
            }
        }
        out
    }
}

/// Red: recording, waiting for the bar (blinking), or muted on a mute pad.
pub const RED: [u8; 3] = [0x7F, 0x00, 0x00];
/// Amber: overdubbing.
pub const AMBER: [u8; 3] = [0x7F, 0x44, 0x00];
/// Blue: a soloed loop's mute pad (SHIFT + the pad).
pub const SOLO: [u8; 3] = [0x00, 0x38, 0x7F];
const DARK: [u8; 3] = [0; 3];

/// A colour at `f` of its brightness, in the pads' 0 to 127.
fn pad_rgb(c: [u8; 3], f: f32) -> [u8; 3] {
    c.map(|v| ((f32::from(v >> 1) * f.clamp(0.0, 1.0)).round() as u8).min(0x7F))
}

/// What the pads that aren't loop keys light by: the throws playing (a
/// bit for each place in [`crate::play_page::THROW_LIST`]: held or
/// latched) and the tracks muted (a bit each, Track 1 the lowest).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lit {
    pub throws: u8,
    pub muted: u128,
}

/// A scene pad's green, dim: a hit launches the row.
const SCENE: [u8; 3] = [0x30, 0xD0, 0x60];
/// A track mute pad's colour while its track plays.
const TRACK: [u8; 3] = [0xE0, 0xE0, 0xE0];

/// The 16 pads' colours for `page`, from each loop's light: lit by state
/// so the pads can be played without looking at the screen. A key: dark
/// when its loop is empty, red blinking while it waits for its bar, red
/// recording, its colour playing (full on the loop's first beat, softer
/// after), amber overdubbing, its colour dim when muted, off or quiet
/// for another loop's solo. A mute pad: dark for an empty loop, red while
/// muted, blue while soloed, its colour dim while it plays. `blink`: the
/// half of the blink that is lit. A throw: its card's colour, dim, full
/// while it plays. A track mute: red while the track is muted, dim white
/// while it plays. A scene: dim green.
pub fn colours(page: &PerformPage, loops: &[LoopLight], lit: &Lit, blink: bool) -> [[u8; 3]; PADS] {
    std::array::from_fn(|pad| {
        let light = |n: u8| loops.get(usize::from(n)).copied().unwrap_or_default();
        match page.action(pad) {
            PadAction::None => DARK,
            PadAction::LoopKey(n) => {
                let l = light(n);
                match l.state {
                    RingState::Empty => DARK,
                    RingState::Waiting(_) if blink => RED,
                    RingState::Waiting(_) => DARK,
                    RingState::Recording => RED,
                    RingState::Overdubbing => AMBER,
                    RingState::Playing if l.downbeat => pad_rgb(l.colour, 1.0),
                    RingState::Playing => pad_rgb(l.colour, 0.55),
                    RingState::Muted | RingState::Off => pad_rgb(l.colour, 0.12),
                }
            }
            PadAction::LoopMute(n) => {
                let l = light(n);
                match l.state {
                    RingState::Empty | RingState::Waiting(_) => DARK,
                    RingState::Muted => RED,
                    _ if l.solo => SOLO,
                    _ => pad_rgb(l.colour, 0.2),
                }
            }
            PadAction::Throw(t) => {
                let Some(def) = crate::play_page::THROW_LIST.get(usize::from(t.index)) else {
                    return DARK;
                };
                let c = [def.color.r, def.color.g, def.color.b];
                let on = lit.throws & (1 << t.index) != 0;
                pad_rgb(c, if on { 1.0 } else { 0.15 })
            }
            PadAction::Mute(t) if lit.muted & (1u128 << (t & 127)) != 0 => RED,
            PadAction::Mute(_) => pad_rgb(TRACK, 0.12),
            PadAction::Scene(_) => pad_rgb(SCENE, 0.2),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_keys_put_each_loop_in_a_column_key_over_mute() {
        let p = PerformPage::loop_keys();
        assert_eq!(p.name, "Loop keys");
        // Pad 13 (top left) is Loop 1's key, pad 5 its mute; pad 9 is
        // Loop 5's key and pad 1 (bottom left) its mute.
        assert_eq!(p.action(12), PadAction::LoopKey(0));
        assert_eq!(p.action(4), PadAction::LoopMute(0));
        assert_eq!(p.action(15), PadAction::LoopKey(3));
        assert_eq!(p.action(8), PadAction::LoopKey(4));
        assert_eq!(p.action(0), PadAction::LoopMute(4));
        assert_eq!(p.action(11), PadAction::LoopKey(7));
        assert_eq!(p.action(3), PadAction::LoopMute(7));
        assert_eq!(p.action(16), PadAction::None);
        for n in 0..8u8 {
            let key = p.key_of(n).unwrap();
            let mute = p.pads.iter().position(|&a| a == PadAction::LoopMute(n));
            assert_eq!(
                mute,
                Some(key - 8),
                "loop {n}'s mute is two rows under its key"
            );
        }
    }

    #[test]
    fn a_key_held_a_second_clears_once() {
        let mut h = Held::default();
        h.press(12, 1.0);
        assert!(h.holds(1.5).is_empty());
        assert_eq!(h.holds(2.0), vec![12]);
        assert!(h.holds(3.0).is_empty(), "once");
        assert!(h.release(12));
        assert!(!h.release(12), "let go once");
        // A hold the pad let go of first never comes.
        h.press(13, 4.0);
        assert!(h.release(13));
        assert!(h.holds(6.0).is_empty());
    }

    #[test]
    fn the_keys_light_by_state() {
        let p = PerformPage::loop_keys();
        let blue = [0x6E, 0xA8, 0xFE];
        let mut loops = [LoopLight::default(); 8];
        loops[0] = LoopLight {
            state: RingState::Playing,
            colour: blue,
            downbeat: false,
            solo: false,
        };
        loops[1].state = RingState::Recording;
        loops[2].state = RingState::Waiting(Some(5));
        loops[3].state = RingState::Overdubbing;
        loops[4] = LoopLight {
            state: RingState::Muted,
            colour: blue,
            downbeat: false,
            solo: false,
        };
        let on = colours(&p, &loops, &Lit::default(), true);
        let off = colours(&p, &loops, &Lit::default(), false);
        // Loop 1 plays in its colour, its mute dim; on its first beat the
        // key is full.
        assert_eq!(on[12], [0x1E, 0x2E, 0x46]);
        assert_eq!(on[4], [0x0B, 0x11, 0x19]);
        loops[0].downbeat = true;
        assert_eq!(
            colours(&p, &loops, &Lit::default(), true)[12],
            [0x37, 0x54, 0x7F]
        );
        // Loop 2 records: red. Loop 3 waits: red, blinking.
        assert_eq!(on[13], RED);
        assert_eq!((on[14], off[14]), (RED, [0; 3]));
        // Loop 4 overdubs: amber.
        assert_eq!(on[15], AMBER);
        // Loop 5 is muted: its key dim, its mute red.
        assert_eq!(on[8], [0x07, 0x0A, 0x0F]);
        assert_eq!(on[0], RED);
        // Empty loops: dark keys and mutes.
        assert_eq!(on[11], [0; 3]);
        assert_eq!(on[3], [0; 3]);
        // Loop 1 soloed: its mute blue; a loop quiet for it is off, dim.
        loops[0].solo = true;
        loops[5] = LoopLight {
            state: RingState::Off,
            colour: blue,
            downbeat: false,
            solo: false,
        };
        let solo = colours(&p, &loops, &Lit::default(), true);
        assert_eq!(solo[4], SOLO);
        assert_eq!(solo[9], [0x07, 0x0A, 0x0F]);
        // Every value is in the pads' 0 to 127.
        assert!(on.iter().flatten().all(|&v| v <= 0x7F));
    }

    #[test]
    fn a_new_projects_set_is_the_loop_keys_then_the_throws() {
        let set = PerformSet::default();
        assert_eq!(set.page(0), &PerformPage::loop_keys());
        assert_eq!(set.page(1), &PerformPage::throws());
        assert!(set.pages[2..].iter().all(|p| *p == PerformPage::empty()));
        assert_eq!(set.page_name(0), "Page A · Loop keys");
        assert_eq!(set.page_name(1), "Page B · Throws");
        assert_eq!(set.page_name(2), "Page C");
        assert_eq!(set.page_name(99), "Page H", "past H is H");
        assert_eq!(set.page(99), set.page(0), "past the set is A");
        // Page B: the eight throws on pads 1 to 8, held while held.
        let b = set.page(1);
        for (i, def) in crate::play_page::THROW_LIST.iter().enumerate() {
            let PadAction::Throw(t) = b.action(i) else {
                panic!("pad {} isn't a throw", i + 1);
            };
            assert_eq!((t.name(), t.hold), (def.name, Hold::WhileHeld));
        }
        assert!(b.pads[8..].iter().all(|&a| a == PadAction::None));
    }

    #[test]
    fn each_throws_slug_is_its_name() {
        for (slug, def) in THROW_SLUGS.iter().zip(crate::play_page::THROW_LIST) {
            assert_eq!(*slug, def.name.to_ascii_lowercase().replace(' ', "-"));
        }
        assert_eq!(THROW_SLUGS.len(), crate::play_page::THROW_LIST.len());
    }

    #[test]
    fn the_set_saves_as_text_and_reads_back() {
        let mut set = PerformSet::default();
        set.pages[1].pads[7] = PadAction::Throw(Throw {
            index: 7,
            hold: Hold::Latch,
        });
        set.pages[2].name = "Live".to_string();
        set.pages[2].pads[0] = PadAction::Mute(2);
        set.pages[2].pads[4] = PadAction::Scene(1);
        set.pages[7].pads[15] = PadAction::LoopKey(7);
        let text = set.text();
        assert_eq!(text.lines().next(), Some(FORMAT));
        for line in [
            "page A = Loop keys",
            "pad A13 = loop-key 1",
            "pad A5 = loop-mute 1",
            "page B = Throws",
            "pad B2 = throw reverb-wash hold=held",
            "pad B8 = throw vinyl-brake hold=latch",
            "page C = Live",
            "pad C1 = mute 3",
            "pad C5 = scene 2",
            "pad H16 = loop-key 8",
        ] {
            assert!(text.lines().any(|l| l == line), "{line:?} in\n{text}");
        }
        assert!(!text.contains("page D"), "an unnamed page has no line");
        let (back, skipped) = PerformSet::parse(&text);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(back, set);
        // The default set too, and an empty part is an empty set.
        let (back, _) = PerformSet::parse(&PerformSet::default().text());
        assert_eq!(back, PerformSet::default());
        let (empty, _) = PerformSet::parse("");
        assert!(empty.pages.iter().all(|p| *p == PerformPage::empty()));
    }

    #[test]
    fn a_line_that_doesnt_read_is_skipped_and_said() {
        let text = "pad A1 = loop-key 1\n\
                    pad A2 = loop-key 9\n\
                    pad I1 = mute 1\n\
                    pad A17 = mute 1\n\
                    pad A3 = throw moon-bounce\n\
                    pad A4 = throw stutter hold=forever\n\
                    pad A8 = throw stutter on=bus-2\n\
                    pad A9 = throw stutter on=master hold=latch\n\
                    pad A10 = throw tape-stop\n\
                    pad A5 = scene 0\n\
                    pad A6 = wobble 2\n\
                    nonsense\n\
                    pad a7 = mute 4\n";
        let (set, skipped) = PerformSet::parse(text);
        assert_eq!(set.page(0).action(0), PadAction::LoopKey(0));
        assert_eq!(
            set.page(0).action(6),
            PadAction::Mute(3),
            "a small letter reads"
        );
        assert_eq!(
            skipped,
            [
                "perform: skipped \"pad A2 = loop-key 9\"",
                "perform: skipped \"pad I1 = mute 1\"",
                "perform: skipped \"pad A17 = mute 1\"",
                "perform: skipped \"pad A3 = throw moon-bounce\"",
                "perform: skipped \"pad A4 = throw stutter hold=forever\"",
                "perform: skipped \"pad A8 = throw stutter on=bus-2\"",
                "perform: skipped \"pad A5 = scene 0\"",
                "perform: skipped \"pad A6 = wobble 2\"",
                "perform: skipped \"nonsense\"",
            ]
        );
        for pad in [1, 2, 3, 4, 5, 7] {
            assert_eq!(set.page(0).action(pad), PadAction::None);
        }
        // A throw's settings in any order; with none it is held.
        let stutter = |hold| PadAction::Throw(Throw { index: 2, hold });
        assert_eq!(set.page(0).action(8), stutter(Hold::Latch));
        assert_eq!(
            set.page(0).action(9),
            PadAction::Throw(Throw {
                index: 3,
                hold: Hold::WhileHeld
            })
        );
    }

    #[test]
    fn throws_mutes_and_scenes_light_by_what_they_do() {
        let mut page = PerformPage::throws();
        page.pads[8] = PadAction::Mute(0);
        page.pads[9] = PadAction::Mute(1);
        page.pads[10] = PadAction::Scene(0);
        let lit = Lit {
            throws: 0b10,
            muted: 0b10,
        };
        let c = colours(&page, &[], &lit, true);
        // Reverb wash plays: its card's colour full; Delay throw dim.
        let card = |i: usize| {
            let d = crate::play_page::THROW_LIST[i].color;
            [d.r, d.g, d.b]
        };
        assert_eq!(c[1], pad_rgb(card(1), 1.0));
        assert_eq!(c[0], pad_rgb(card(0), 0.15));
        assert_ne!(c[0], DARK);
        // Track 2 is muted: red; Track 1 plays: dim white.
        assert_eq!(c[9], RED);
        assert_eq!(c[8], pad_rgb(TRACK, 0.12));
        assert_ne!(c[8], DARK);
        // A scene: dim green. An empty pad: dark.
        assert_eq!(c[10], pad_rgb(SCENE, 0.2));
        assert_eq!(c[11], DARK);
        assert!(c.iter().flatten().all(|&v| v <= 0x7F));
    }

    #[test]
    fn drop_drums_mutes_the_kicks_snares_and_hats() {
        let names = [
            (0, "Kick 1"),
            (1, "Snare"),
            (2, "Hat cl"),
            (3, "Bass 1"),
            (4, "Chop 2"),
            (20, "Kick 9"),
        ];
        assert_eq!(drum_pads(names.into_iter()), 0b111);
        // Drop drums (place 6) mutes them held and none let go; the
        // others are the mixer's throws.
        use crate::mixing::PlayFx;
        assert_eq!(throw_fx(6, true, 0b111), Some(PlayFx::Drop(0b111)));
        assert_eq!(throw_fx(6, false, 0b111), Some(PlayFx::Drop(0)));
        assert!(matches!(
            throw_fx(1, true, 0),
            Some(PlayFx::Throw { down: true, .. })
        ));
        assert_eq!(throw_fx(8, true, 0), None);
    }

    #[test]
    fn a_held_pad_keeps_what_it_pressed() {
        let mut h = Held::default();
        let reverb = PadAction::Throw(Throw {
            index: 1,
            hold: Hold::WhileHeld,
        });
        h.press_action(1, 0.0, reverb);
        assert_eq!(h.action(1), Some(reverb));
        assert_eq!(h.action(2), None);
        assert_eq!(h.holds(2.0), vec![1]);
        assert_eq!(h.action(1), Some(reverb), "after its hold too");
        assert_eq!(h.release_action(1), Some(reverb));
        assert_eq!(h.release_action(1), None);
        assert_eq!(h.action(1), None);
    }
}

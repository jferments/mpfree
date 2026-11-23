//! What the pads play: the selected track's instrument, or one chosen for
//! them (the instrument redesign's step 3, pad focus; the design doc
//! "Instruments, pads and tracks: the redesign", rules 1 and 2, Jay's yes
//! of 20 Nov 2026). Step 1 (Play on the pads, #722) is where it started.
//!
//! **The pads follow the selected track until you choose for them**
//! ([`PadFocus::Track`]): selecting a track puts its instrument on the
//! pads, as on the MPC. On the Synth tab, while the selected track's
//! instrument is a kit, the button under the instrument card chooses the
//! synth the tab shows (the shared synth, instrument 1, on its own Synth
//! channel of the mixer) for the pads ([`PadFocus::Instrument`]), and it
//! stays there whichever track is selected, until **Follow track** (the
//! button again, or the "Pads: Poly" chip in the header). The tracks keep
//! what they play and their clips play it. The pads' focus is saved with
//! the project (the `pads` part, [`App::pads_part`]); a new project, or one
//! saved before, follows the track.
//!
//! **Nothing implicit moves the pads** (rule 1, and the one rule of the
//! sampler refactor's step 15b): arming or selecting a track, a row tapped
//! in REC's list, REC, a kit picked for the selected track (the pads that
//! follow the track play its new kit; a choice of their own, a bank's own
//! and the layers stay), or an instrument gone from the list (its entry
//! stays, missing, below), or the arp (below). What does: a choice you
//! make for the pads, Follow track, and a project opened (its own map).
//!
//! **One pad map** (sampler step 15b, [`PadMap`]; the plan's "Step 15,
//! revised after the 23 Nov review", Jay's go 23 Nov 09:02 UTC): what the
//! pads play is one small value, the base for every bank, each bank's own
//! entry and the layers (app/pad_layers.rs), kept, saved and restored
//! whole (the list of synths' Cancel). A hit resolves bank entry, then
//! base; each layer that reaches the bank adds its sound. The base can be
//! a whole kit of the list ([`App::pads_kit`]): the pads play its pads on
//! their own, the page they are on, while the selected track plays
//! something else, and REC records it on a track nobody set up, which then
//! plays that kit ([`App::rec_kit_on`]). An entry whose instrument the
//! list hasn't got (gone, or a project's that names one it doesn't have)
//! keeps its place, missing, said once: the bank plays the base
//! meanwhile, a missing base the track's instrument, a missing layer
//! nothing. A kit's source bank (`from C`) and a layer's reach (`bank B`)
//! are in the map and its saved form; the screens that set them come with
//! steps 15e and 15f.
//!
//! **How the audio loop plays them** (`main.rs`, `preview_on`): while the
//! selected track plays the shared synth itself, the pads play it the
//! usual way, through the arp and the recorder, as the track's; while it
//! plays anything else, the loop plays the pad hits on the synth before the
//! track's arp and the recorder see them (through the pads' arp while the
//! arp is on, below), so nothing records them there.
//!
//! **REC** (rule 3, step 1's way with today's per-track instruments): while
//! the pads have the synth and the armed track plays something else, REC
//! or OVERDUB on the notes record the synth where it plays: a track nobody
//! set up plays the same synth (the Synth kit, no copy) and the take goes
//! there (`desk.rs`, [`synth_track`], [`App::rec_synth_on`]). The pads keep
//! the synth (rule 1); the track now plays it too, so they play it through
//! the track, and a second REC records there again. "Nobody set up" is
//! more than empty (the design review of 20 Nov, point B): no clips, a note
//! track's settings and default name as a new one has them ("Track 2" for
//! the second), no instrument of its own, kit 1 or the synth, no MIDI
//! output and no mixer settings ([`unfit`]); never Track 1, the Drums
//! track. The selected track when it is one, else the first one, else a
//! new track at the end of the project; nothing at all when the project is
//! full. A pinned arm (or a row tapped in REC's list) is the musician's
//! choice (point A): REC records there or nowhere, and says why. What is
//! recorded always plays what was heard, and no clip already recorded
//! changes. With the loop armed the pads keep the synth: the loop records
//! what is heard. Where REC will record is the recording plan
//! ([`RecPlan`], `desk::rec_plan`, step 5b): the REC chip names it before
//! you press ("REC → Track 2 · Poly"), and REC acts on the same plan.
//!
//! **Banks** (step 6c, the design's rule 10: "a bank points at an
//! instrument; it holds nothing"): a bank of the pads, A to H, can play an
//! instrument of its own, picked in the list of synths with "Bank B only"
//! at its top ([`App::pick_pads_synth`]); the other banks play what every
//! bank plays. The kit keeps all 128 of its pads: bank B given back plays
//! them as before. [`App::pads_focus`] is what the pads play now, the
//! bank's own while the pads are on it; everything above (REC, the arp,
//! the header's chip, "Pads B: FM") follows it. A bank's own applies on a
//! kit's pages: while the selected track plays the shared synth, PAD BANK
//! picks its layout's banks instead (Jay, 20 Nov 09:55Z), and the banks'
//! own wait. Follow track gives every bank back to the track. The banks'
//! own are saved in the pads part with the map (`bank B instrument 3`).
//!
//! **The arp** (sampler step 15c): NOTE REPEAT, Latch and the arp play
//! what the pads play, and never give the pads back. While the pads play
//! through the selected track, the track's arp plays them as before. While
//! they play an instrument on their own (a synth, a whole kit, a bank's
//! own), the pads' arp in the audio loop (`main.rs`, `PadsArp`) takes the
//! hits instead: the selected track's arp as the Arp panel sets it, on an
//! engine of its own, playing that instrument: a kit's pads, a synth's
//! notes (its pad layout's), a host instrument's pads. Its notes end
//! where they started when the pads move to another instrument. Layers
//! play each hit once, as pressed (through the arp with step 15f).

use super::{App, Effect};
use crate::instrument_list::InstrumentId;
use mpfree_seq::{Project, Track};

/// What the pads play: the selected track's instrument, or an instrument
/// chosen for them that stays whichever track is selected: the shared
/// synth (the Synth tab's button), or, from step 6, an instrument of the
/// list the instrument host holds (a free place's, picked in the list of
/// synths, or a track's own).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadFocus {
    /// The selected track's instrument (a new project's): "Follow track".
    #[default]
    Track,
    /// This instrument of the project's list ("Pads: Poly").
    Instrument(InstrumentId),
}

/// The most kits REC can record besides the armed track's: the pad map's
/// base, a bank's own each (sampler step 15c) and its layers, and the
/// kits the MIDI routes name (step 15f).
pub const MAP_KITS: usize =
    1 + super::KIT_PAGES + super::pad_layers::MORE + super::route_synths::ROUTE_SYNTHS;

/// The most sounds of the pad map REC records besides the first: its base
/// and each bank's own ([`App::rec_map_sounds`]).
pub const MAP_MORE: usize = 1 + super::KIT_PAGES;

/// The pads part's first line ([`crate::project::PADS`]): version 2,
/// the pad map (sampler step 15b), with the layers in it.
pub const PADS_FORMAT: &str = "mpfree-pads 2";

impl PadFocus {
    /// The shared synth, chosen for the pads.
    pub const SYNTH: PadFocus = PadFocus::Instrument(InstrumentId::SHARED_SYNTH);

    /// The words after `play` or a bank's letter: `track`, `instrument 3`.
    fn words(self) -> String {
        match self {
            PadFocus::Track => "track".to_string(),
            PadFocus::Instrument(id) => format!("instrument {id}"),
        }
    }
}

/// A bank's own entry in the pad map (step 6c's bank's own, sampler step
/// 15b): what bank A to H plays ahead of the base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BankEntry {
    /// The track's instrument, or one of the project's list.
    pub play: PadFocus,
    /// For a kit, the bank of it this bank plays (bank E playing kit 3's
    /// bank A, step 15e); None: the same bank, as a whole kit plays.
    pub from: Option<u8>,
}

impl BankEntry {
    /// What a bank's own pick plays, from the same bank.
    pub const fn of(play: PadFocus) -> BankEntry {
        BankEntry { play, from: None }
    }
}

/// A layer in the pad map (step 11a's layers, sampler step 15b): one more
/// instrument each hit plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerEntry {
    /// The instrument of the project's list it plays.
    pub id: InstrumentId,
    /// The bank it plays on alone; None: every bank (today's layers).
    pub reach: Option<u8>,
    /// For a kit (step 15f), the bank of it the layer plays; None: the
    /// bank the pads are on.
    pub from: Option<u8>,
}

impl LayerEntry {
    /// A layer on every bank.
    pub const fn every(id: InstrumentId) -> LayerEntry {
        LayerEntry {
            id,
            reach: None,
            from: None,
        }
    }

    /// Whether the layer plays on bank `b`.
    pub fn reaches(self, b: usize) -> bool {
        self.reach.is_none_or(|r| usize::from(r) == b)
    }
}

/// The pad map (sampler step 15b, "one pad map"; the plan's "Step 15,
/// revised after the 23 Nov review", Jay's go of 23 Nov 09:02 UTC): what
/// the pads play, in one small value the app keeps, saves and changes
/// only when you do. A hit resolves bank entry, then base; each layer
/// that reaches the bank adds its own sound. An entry whose instrument
/// the project's list hasn't got is kept, missing: that bank plays the
/// base meanwhile (a missing base: the track's instrument), and a missing
/// layer is silent. Fixed size, no heap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadMap {
    /// What every bank without an entry plays: Follow track, or an
    /// instrument of the list (a synth or a whole kit).
    pub base: PadFocus,
    /// Bank A to H's own entry.
    pub banks: [Option<BankEntry>; super::KIT_PAGES],
    /// The layers, packed in order.
    pub layers: [Option<LayerEntry>; super::pad_layers::MORE],
}

impl PadMap {
    /// The pads part: its first line, `play track` or `play instrument 1`,
    /// a line for each bank with an entry (`bank B instrument 7`, `bank B
    /// instrument 7 from C`, `bank C track`), then a line per layer
    /// (`layer instrument 9`, `layer instrument 9 bank B from A`).
    pub fn text(&self) -> String {
        let mut text = format!("{PADS_FORMAT}\nplay {}\n", self.base.words());
        for (b, e) in self.banks.iter().enumerate() {
            let Some(e) = e else { continue };
            text.push_str(&format!(
                "bank {} {}{}\n",
                super::bank_letter(b),
                e.play.words(),
                from_words(e.from)
            ));
        }
        for l in self.layers.iter().flatten() {
            let reach = l
                .reach
                .map(|r| format!(" bank {}", super::bank_letter(usize::from(r))))
                .unwrap_or_default();
            text.push_str(&format!(
                "layer instrument {}{reach}{}\n",
                l.id,
                from_words(l.from)
            ));
        }
        text
    }

    /// The part read back ([`PadMap::text`]); Err says why it doesn't
    /// read: another first line (an older version), a line it doesn't
    /// know, an instrument number that can't be one, what the pads play
    /// said twice or not at all, a bank past H or said twice, a layer said
    /// twice or more than [`super::pad_layers::MORE`]. Whether the
    /// project's list has the instruments is [`App::open_pads`]'s to say:
    /// an entry it hasn't got is kept, missing.
    pub fn parse(text: &str) -> Result<PadMap, String> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        match lines.next() {
            Some(PADS_FORMAT) => {}
            Some(other) => return Err(format!("its first line is \"{other}\"")),
            None => return Err("it is empty".to_string()),
        }
        let bad = |line: &str| format!("\"{line}\" doesn't read");
        let instrument = |line: &str, n: &str| {
            n.parse()
                .ok()
                .and_then(InstrumentId::from_number)
                .ok_or_else(|| bad(line))
        };
        let bank = |line: &str, letter: &str| {
            bank_of_letter(letter)
                .map(|b| b as u8)
                .ok_or_else(|| bad(line))
        };
        let mut map = PadMap::default();
        let mut base = None;
        let mut layers = 0;
        for line in lines {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["play", "track"] | ["play", "instrument", _] => {
                    let f = match words.as_slice() {
                        ["play", "instrument", n] => PadFocus::Instrument(instrument(line, n)?),
                        _ => PadFocus::Track,
                    };
                    if base.replace(f).is_some() {
                        return Err("it says twice what the pads play".to_string());
                    }
                }
                ["bank", letter, rest @ ..] => {
                    let b = usize::from(bank(line, letter)?);
                    let (play, from) = match rest {
                        ["track"] => (PadFocus::Track, None),
                        ["instrument", n] => (PadFocus::Instrument(instrument(line, n)?), None),
                        ["instrument", n, "from", f] => (
                            PadFocus::Instrument(instrument(line, n)?),
                            Some(bank(line, f)?),
                        ),
                        _ => return Err(bad(line)),
                    };
                    if map.banks[b].replace(BankEntry { play, from }).is_some() {
                        return Err(format!("it says twice what bank {letter} plays"));
                    }
                }
                ["layer", "instrument", n, rest @ ..] => {
                    let id = instrument(line, n)?;
                    let (reach, from) = match rest {
                        [] => (None, None),
                        ["bank", r] => (Some(bank(line, r)?), None),
                        ["from", f] => (None, Some(bank(line, f)?)),
                        ["bank", r, "from", f] => (Some(bank(line, r)?), Some(bank(line, f)?)),
                        _ => return Err(bad(line)),
                    };
                    if map.layers.iter().flatten().any(|l| l.id == id) {
                        return Err(format!("it names instrument {id} twice as a layer"));
                    }
                    let Some(slot) = map.layers.get_mut(layers) else {
                        return Err(format!(
                            "it has more than {} layers, the most the pads play",
                            super::pad_layers::MORE
                        ));
                    };
                    *slot = Some(LayerEntry { id, reach, from });
                    layers += 1;
                }
                _ => return Err(bad(line)),
            }
        }
        map.base = base.ok_or_else(|| "it doesn't say what the pads play".to_string())?;
        Ok(map)
    }
}

/// ` from C` for a kit's source bank, else nothing.
fn from_words(from: Option<u8>) -> String {
    from.map(|f| format!(" from {}", super::bank_letter(usize::from(f))))
        .unwrap_or_default()
}

/// Bank `A` to `H` (0 to 7), as [`super::bank_letter`] writes it.
fn bank_of_letter(letter: &str) -> Option<usize> {
    (0..super::KIT_PAGES).find(|&b| letter.len() == 1 && letter.starts_with(super::bank_letter(b)))
}

/// Why track `t` can't take the pads' synth for REC without changing
/// something on it, in a few words after its name ("has clips"); None for
/// a track nobody set up. The project says what it holds (`project`), the
/// app the rest (`app_side`: an instrument of its own, a kit or MIDI output
/// picked, mixer settings).
pub fn unfit(
    project: &Project,
    t: usize,
    app_side: impl Fn(usize) -> Option<&'static str>,
) -> Option<&'static str> {
    let Some(k) = project.tracks.get(t) else {
        return Some("isn't in the project");
    };
    if k.is_loops() {
        return Some("is a Loop track");
    }
    if k.is_audio() {
        return Some("is an audio track");
    }
    if !k.slots.is_empty() {
        return Some("has clips");
    }
    // Renamed, or a setting moved (swing, MIDI effects, transpose...). The
    // name must be this track's own default, "Track 2" for the second: a
    // track renamed "Track 99" is set up (the design review of 20 Nov's
    // second pass). A track after a deleted one keeps its old name and so
    // counts as set up too, the safe way: tracks can't be deleted from
    // the middle yet, and when they can, the track's lasting id (step 2c)
    // is the way to tell. Never allocates (an empty name allocates
    // nothing).
    let named = k
        .name
        .strip_prefix("Track ")
        .is_some_and(|n| !n.starts_with(['0', '+']) && n.parse::<usize>() == Ok(t + 1));
    if !named || !k.same_settings(&Track::new("", k.dest)) {
        return Some("has settings of its own");
    }
    app_side(t)
}

/// Where REC records the pads' synth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthTake {
    /// On this track (0 based), which nobody set up: only its kit changes,
    /// to the Synth kit.
    On(usize),
    /// On a new track at the end of the project: every track is set up.
    New,
    /// Nowhere, and nothing changes: the arm is pinned to `track`, which
    /// `why` says can't take it.
    Pinned { track: usize, why: &'static str },
    /// Nowhere, and nothing changes: every track is set up and the
    /// project holds no more.
    Full,
}

/// The recording plan (the redesign's step 5b): where REC will record the
/// pads' synth, worked out each time something changes so the REC chip
/// names it before you press, and by REC itself, so the chip and REC
/// can't disagree (`desk.rs`, `rec_plan`). None of it changes anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecPlan {
    pub take: SynthTake,
    /// The pads' synth's name ("Poly").
    pub synth: &'static str,
    /// The holder of the instrument the pads play in the instrument host
    /// (step 6: a synth picked in the list of synths, in a free place),
    /// which the track REC records on plays by a send; None for the
    /// shared synth, which it plays as its Synth kit.
    pub holder: Option<usize>,
    /// The armed track plays the pads' synth already (`take` is it): only
    /// the layers want a place.
    pub ready: bool,
    /// The kit place the pads play (sampler step 15): the track REC
    /// records on plays that kit; `synth` is then a stand-in name ("the
    /// pads' kit") the screen replaces with the kit's.
    pub kit: Option<usize>,
    /// Where REC records each sound besides the first: the pads' layers
    /// beyond the first (step 11c), in their order, then each synth a MIDI
    /// route names (step 11e), then the pads' base and each bank's own
    /// (sampler step 15c) ([`App::rec_more_sounds`]); None where there is
    /// none.
    pub layers: [Option<LayerTake>; super::REC_MORE],
}

/// One sound of the pad map that REC records (sampler step 15c, "REC
/// records what is heard"): a synth, by its holder in the instrument host
/// (None for the shared synth), or a kit, by its kit place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecSound {
    Synth(Option<usize>),
    Kit(usize),
}

/// Where a sound REC records besides the first comes from in the pad map
/// or the routes, for what REC says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecFrom {
    /// A layer of the pads (its slot in the map, step 11c).
    Layer(usize),
    /// A synth a MIDI route names (its row, step 11e).
    Route(usize),
    /// What every bank without its own plays (sampler step 15c).
    Base,
    /// Bank A to H's own (sampler step 15c).
    Bank(usize),
}

/// One sound REC records besides the first ([`App::rec_more_sounds`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecMore {
    /// Its name ("Pluck"; "a kit" for a kit, which the app has no name
    /// for).
    pub name: &'static str,
    pub sound: RecSound,
    pub from: RecFrom,
}

impl RecMore {
    /// The sound as REC names it: "Pluck" for a layer, "FM" for a route's
    /// synth, "Bank B's FM", "Bank C's kit", "the pads' Poly".
    pub fn said(self) -> String {
        match self.from {
            RecFrom::Layer(_) | RecFrom::Route(_) => self.name.to_string(),
            RecFrom::Base => format!("the pads' {}", self.short()),
            RecFrom::Bank(b) => format!("Bank {}'s {}", super::bank_letter(b), self.short()),
        }
    }

    /// "FM", or "kit" for a kit.
    fn short(self) -> &'static str {
        match self.sound {
            RecSound::Kit(_) => "kit",
            RecSound::Synth(_) => self.name,
        }
    }
}

/// Where REC records one sound besides the first: one of the pads' layers
/// (the instrument redesign's step 11c), a synth a MIDI route names (step
/// 11e), or, from sampler step 15c, the pads' base or a bank's own. As the
/// first, where it lives: an armed track that plays it already (one REC
/// records too, or the armed track the first moves the arm from), else a
/// free track pointed at it, else a new one; `Full` when the project
/// holds no more, and REC records nothing. Never `Pinned`: the pin is the
/// first's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerTake {
    pub take: SynthTake,
    /// Its name ("Pluck"), as [`RecMore::name`].
    pub synth: &'static str,
    /// What it plays.
    pub sound: RecSound,
    /// Where it comes from.
    pub from: RecFrom,
    /// The track plays it already: nothing on it changes.
    pub plays: bool,
}

impl LayerTake {
    /// The sound as REC names it ([`RecMore::said`]).
    pub fn said(self) -> String {
        RecMore {
            name: self.synth,
            sound: self.sound,
            from: self.from,
        }
        .said()
    }
}

impl RecPlan {
    /// The REC chip's words after the arrow: "Track 2 · Poly", "New track
    /// · Poly", "Track 1 · not free" (pinned to a track that can't take
    /// it), "No free track · Poly"; with layers that record too, "Track 2
    /// · FM + 1 more" (step 11c); "Track 1 + 1 more" for the armed track
    /// as it is, with a synth a MIDI route names (step 11e). `name` gives
    /// a track's name ("Bass" for one you named).
    pub fn chip(self, name: impl Fn(usize) -> String) -> String {
        self.chip_as(self.synth, name)
    }

    /// [`RecPlan::chip`] with `what` for the pads' synth: a kit's name
    /// (sampler step 15), which the screen has. A sound with no track
    /// (the project holds no more) makes it "No free track · Bank B's FM":
    /// REC records nothing then (sampler step 15c).
    pub fn chip_as(self, what: &str, name: impl Fn(usize) -> String) -> String {
        let more = self.more();
        let first = match self.take {
            // The armed track as it is (a kit), with a route's synth.
            SynthTake::On(t) if what.is_empty() => name(t),
            SynthTake::On(t) => format!("{} · {what}", name(t)),
            SynthTake::New => format!("New track · {what}"),
            SynthTake::Pinned { track, .. } => return format!("{} · not free", name(track)),
            SynthTake::Full => return format!("No free track · {what}"),
        };
        if let Some(l) = self
            .layers
            .iter()
            .flatten()
            .find(|l| l.take == SynthTake::Full)
        {
            return format!("No free track · {}", l.said());
        }
        if more == 0 {
            first
        } else {
            format!("{first} + {more} more")
        }
    }

    /// How many layers record besides the first: those with a place.
    pub fn more(self) -> usize {
        self.layers
            .iter()
            .flatten()
            .filter(|l| l.take != SynthTake::Full)
            .count()
    }
}

/// Where REC records each sound besides the first ([`App::rec_more_sounds`]:
/// the pads' layers, step 11c, the routes' synths, step 11e, the pads'
/// base and banks' own, sampler step 15c), in order: an armed track that
/// plays it already, from `candidates` (a bit each: the other armed
/// tracks, and the armed one when the first moves the arm from it), else
/// the first free track, else a new one while the project holds more.
/// `first` is the track the first records on (None: a new one). `plays`
/// says whether a track plays a sound; `fits` whether a track is free
/// ([`unfit`] says None). Allocates nothing.
pub fn layer_takes(
    more: [Option<RecMore>; super::REC_MORE],
    tracks: usize,
    first: Option<usize>,
    candidates: u128,
    plays: impl Fn(usize, RecSound) -> bool,
    fits: impl Fn(usize) -> bool,
) -> [Option<LayerTake>; super::REC_MORE] {
    let bit = |t: usize| 1u128.checked_shl(t as u32).unwrap_or(0);
    let mut used = first.map_or(0, bit);
    // New tracks the plan adds: the first's, then the others'.
    let mut news = usize::from(first.is_none());
    more.map(|m| {
        let m = m?;
        let armed = (0..tracks.min(128))
            .find(|&t| candidates & bit(t) != 0 && used & bit(t) == 0 && plays(t, m.sound));
        let (take, plays) = if let Some(t) = armed {
            (SynthTake::On(t), true)
        } else if let Some(t) = (0..tracks).find(|&t| used & bit(t) == 0 && fits(t)) {
            (SynthTake::On(t), false)
        } else if tracks + news < mpfree_seq::MAX_TRACKS {
            news += 1;
            (SynthTake::New, false)
        } else {
            (SynthTake::Full, false)
        };
        if let SynthTake::On(t) = take {
            used |= bit(t);
        }
        Some(LayerTake {
            take,
            synth: m.name,
            sound: m.sound,
            from: m.from,
            plays,
        })
    })
}

/// Where REC records the pads' synth: the armed track when nobody set it
/// up, else (unless `pinned`) the first track nobody set up, else a new
/// one ([`unfit`] says what counts).
pub fn synth_track(
    project: &Project,
    armed: usize,
    pinned: bool,
    app_side: impl Fn(usize) -> Option<&'static str>,
) -> SynthTake {
    let why = |t: usize| unfit(project, t, &app_side);
    match why(armed) {
        None => return SynthTake::On(armed),
        Some(why) if pinned => return SynthTake::Pinned { track: armed, why },
        Some(_) => {}
    }
    if let Some(t) = (0..project.tracks.len()).find(|&t| why(t).is_none()) {
        SynthTake::On(t)
    } else if project.tracks.len() < mpfree_seq::MAX_TRACKS {
        SynthTake::New
    } else {
        SynthTake::Full
    }
}

impl App {
    /// What the pads play now: the selected track's instrument, or one
    /// chosen for them; on a bank with one of its own (step 6c), that. A
    /// missing entry (its instrument gone from the list, sampler step 15b)
    /// plays the base meanwhile, and a missing base the track's.
    pub fn pads_focus(&self) -> PadFocus {
        self.pads_bank()
            .and_then(|b| self.bank_play(b))
            .unwrap_or(self.base_now())
    }

    /// The pad map as it is, missing entries and all (sampler step 15b).
    #[cfg(test)]
    pub fn pad_map(&self) -> PadMap {
        self.pad_map
    }

    /// Whether `f` can play: the track's instrument, or one the project's
    /// list has (else its entry is missing).
    fn listed(&self, f: PadFocus) -> bool {
        match f {
            PadFocus::Instrument(id) => self.instrument_list.kind(id).is_some(),
            PadFocus::Track => true,
        }
    }

    /// The base as it plays: the track's instrument while it is missing.
    pub(super) fn base_now(&self) -> PadFocus {
        Some(self.pad_map.base)
            .filter(|&f| self.listed(f))
            .unwrap_or(PadFocus::Track)
    }

    /// What bank `b`'s own entry plays, if it has one that isn't missing.
    pub(super) fn bank_play(&self, b: usize) -> Option<PadFocus> {
        self.pad_map
            .banks
            .get(b)
            .copied()
            .flatten()
            .map(|e| e.play)
            .filter(|&f| self.listed(f))
    }

    /// The map's entries the project's list hasn't got, a bit each: the
    /// base 0, bank A to H 1 to 8, the layers after. Allocates nothing.
    fn missing(&self) -> u16 {
        let base = u16::from(!self.listed(self.pad_map.base));
        let banks = (0..super::KIT_PAGES)
            .filter(|&b| self.pad_map.banks[b].is_some_and(|e| !self.listed(e.play)))
            .fold(0, |bits, b| bits | 1 << (1 + b));
        let layers = (0..super::pad_layers::MORE)
            .filter(|&l| {
                self.pad_map.layers[l].is_some_and(|e| !self.listed(PadFocus::Instrument(e.id)))
            })
            .fold(0, |bits, l| bits | 1 << (1 + super::KIT_PAGES + l));
        base | banks | layers
    }

    /// What newly missing entries (`bits`, as [`App::missing`]) mean, said
    /// once: each keeps its place in the map until you replace it or
    /// Follow track clears the map, and plays the base (a bank), the
    /// track's instrument (the base) or nothing (a layer) meanwhile.
    fn say_missing(&self, bits: u16, how: &str) -> Vec<String> {
        let mut notes = Vec::new();
        let id_of = |f: PadFocus| match f {
            PadFocus::Instrument(id) => id.to_string(),
            PadFocus::Track => String::new(),
        };
        if bits & 1 != 0 {
            notes.push(format!(
                "The pads' instrument {} {how}: they play Track {}'s instrument until you pick another (Follow track clears it)",
                id_of(self.pad_map.base),
                self.track + 1
            ));
        }
        for b in 0..super::KIT_PAGES {
            if bits >> (1 + b) & 1 == 0 {
                continue;
            }
            let id = self.pad_map.banks[b].map_or(String::new(), |e| id_of(e.play));
            notes.push(format!(
                "Bank {}'s instrument {id} {how}: the bank plays what the other banks play until you pick another",
                super::bank_letter(b)
            ));
        }
        let layers = (bits >> (1 + super::KIT_PAGES)).count_ones();
        if layers == 1 {
            notes.push(format!(
                "A layer's instrument {how}: the pads play the others"
            ));
        } else if layers > 1 {
            notes.push(format!(
                "{layers} layers' instruments {how}: the pads play the others"
            ));
        }
        notes
    }

    /// The bank the pads are on when it plays something of its own (step
    /// 6c), for "Pads B: FM": on a kit's pages, not while the selected
    /// track plays the shared synth (PAD BANK picks its layout's banks
    /// then).
    pub fn pads_bank(&self) -> Option<usize> {
        let b = self.pad_page;
        (self.pad_layout.is_none() && self.bank_play(b).is_some()).then_some(b)
    }

    /// The bank of kit pads the 16 pads play now: the source bank of the
    /// bank's own kit while the pads are on a bank that has one (sampler
    /// step 15e: bank D playing kit 3's bank A, "bank D instrument 9 from
    /// A"), else the bank they are on. A pad hit, its colours, its flash,
    /// REC and the Sample page all take the kit pad of this bank.
    pub fn kit_page(&self) -> usize {
        self.pads_bank()
            .and_then(|b| self.pad_map.banks.get(b).copied().flatten())
            .filter(|e| match e.play {
                PadFocus::Instrument(id) => matches!(
                    self.instrument_list.kind(id),
                    Some(crate::instrument_list::Kind::Kit { .. })
                ),
                PadFocus::Track => false,
            })
            .and_then(|e| e.from)
            .map_or(self.pad_page, |f| usize::from(f).min(super::KIT_PAGES - 1))
    }

    /// Whether the banks' own apply now ([`App::pads_bank`]): on a kit's
    /// pages, so the list of synths offers "Bank B only".
    pub fn banks_apply(&self) -> bool {
        self.pad_layout.is_none()
    }

    /// Which of the pads play what `plays` picks out, for who plays (the
    /// instrument redesign's step 8, "FM: pads bank B, Track 2"): `.0`
    /// the pads' own choice does (every bank without one of its own), `.1`
    /// a bit for each bank whose own does. The banks' own count only on a
    /// kit's pages ([`App::banks_apply`]). A layer (step 11b) counts as
    /// the pads' own choice. Allocates nothing.
    pub fn pads_who(&self, plays: impl Fn(PadFocus) -> bool) -> (bool, u8) {
        // What every bank plays, its layers too (step 11b).
        let all =
            plays(self.base_now()) || self.pad_layers().any(|id| plays(PadFocus::Instrument(id)));
        if !self.banks_apply() {
            return (all, 0);
        }
        let every = self.pad_map.banks.iter().any(Option::is_none) && all;
        let banks = (0..super::KIT_PAGES)
            .filter(|&b| self.bank_play(b).is_some_and(&plays))
            .fold(0u8, |bits, b| bits | 1 << b);
        (every, banks)
    }

    /// The holder in the instrument host of instrument `id` of the
    /// project's list, as the app last had it: a track's own or a free
    /// place's; None for the shared synth, or one the list hasn't got.
    pub fn instrument_holder(&self, id: InstrumentId) -> Option<usize> {
        self.instrument_list.kind(id)?.holder()
    }

    /// What bank `b` plays of its own (None: what every bank plays).
    #[cfg(test)]
    pub fn bank_focus(&self, b: usize) -> Option<PadFocus> {
        self.bank_play(b)
    }

    /// The pads went to another bank (`was`: the page before): the list of
    /// synths, open, names the new one. What the pads play follows by
    /// [`App::pads_focus`]. Nothing allocated otherwise.
    pub(super) fn bank_moved(&mut self, was: (PadFocus, Option<usize>)) -> Vec<Effect> {
        let now = (self.pads_focus(), self.pads_bank());
        let mut fx = Vec::new();
        if now.0 != was.0 {
            fx.push(Effect::PadsFocus(now.0));
        }
        if self.synth_pick.is_some() {
            fx.push(Effect::SynthPick(self.synth_pick_view()));
        }
        fx
    }

    /// Whether any bank points at holder `holder`, or waits for free
    /// place `n`'s id (`n`: the holder's place), so nothing puts it away.
    pub(super) fn banks_hold(&self, holder: usize) -> bool {
        let n = holder.checked_sub(crate::instruments::TRACKS);
        self.bank_to.iter().any(|&t| t.is_some() && t == n)
            || self.pad_map.banks.iter().flatten().any(|e| match &e.play {
                PadFocus::Instrument(id) => {
                    self.instrument_list.kind(*id).and_then(|k| k.holder()) == Some(holder)
                }
                PadFocus::Track => false,
            })
    }

    /// Whether the shared synth is chosen for the pads, whichever track is
    /// selected (the Synth tab's button, "Pads: Poly").
    pub fn pads_chose_synth(&self) -> bool {
        self.pads_focus() == PadFocus::SYNTH
    }

    /// The holder in the instrument host of the instrument chosen for the
    /// pads (step 6): a free place's, or a track's own. None while they
    /// follow the track or have the shared synth, or once the list hasn't
    /// got it.
    pub fn pads_holder(&self) -> Option<usize> {
        match self.pads_focus() {
            PadFocus::Instrument(id) => self.instrument_list.kind(id)?.holder(),
            PadFocus::Track => None,
        }
    }

    /// The kit place chosen for the pads (sampler step 15): a kit of the
    /// project's list. None while they follow the track or play a synth,
    /// or once the list hasn't got it.
    pub fn pads_kit(&self) -> Option<usize> {
        match self.pads_focus() {
            PadFocus::Instrument(id) => match self.instrument_list.kind(id)? {
                crate::instrument_list::Kind::Kit { kit } => Some(kit),
                _ => None,
            },
            PadFocus::Track => None,
        }
    }

    /// The instrument chosen for the pads that the host holds, as the
    /// audio loop last said ("FM"), for the screen.
    pub fn pads_held(&self) -> Option<crate::instruments::Held> {
        self.track_sound(self.pads_holder()?)
    }

    /// The project's list of instruments as the audio loop has it now
    /// (copied: it holds no heap). A free place given a synth for the
    /// pads ([`App::pick_pads_synth`]) is what they play once it has an
    /// id. An entry of the pad map whose instrument has gone from the list
    /// stays, missing (sampler step 15b: nothing implicit changes the
    /// pads), said once: a bank plays the base meanwhile, the base the
    /// track's instrument, a layer nothing. Nothing to do, nothing
    /// allocated.
    pub fn set_instrument_list(
        &mut self,
        list: &crate::instrument_list::InstrumentList,
    ) -> Vec<Effect> {
        let was = self.pads_focus();
        let missing = self.missing();
        self.instrument_list.clone_from(list);
        // A synth given a free place for a bank (step 6c), the pads or a
        // layer (step 11a) that has its id now.
        for b in 0..super::KIT_PAGES {
            if let Some(id) = self.bank_to[b].and_then(|n| list.free_of(n)) {
                self.bank_to[b] = None;
                self.pad_map.banks[b] = Some(BankEntry::of(PadFocus::Instrument(id)));
            }
        }
        if let Some(id) = self.pads_to.and_then(|n| list.free_of(n)) {
            self.pads_to = None;
            self.pad_map.base = PadFocus::Instrument(id);
        }
        self.layers_follow_list();
        let gone = self.missing() & !missing;
        let mut fx: Vec<Effect> = self
            .say_missing(gone, "is gone")
            .into_iter()
            .map(Effect::Note)
            .collect();
        if self.pads_focus() != was {
            fx.insert(0, Effect::PadsFocus(self.pads_focus()));
        }
        fx
    }

    /// The list of a project being opened, before its pads part
    /// ([`App::open_pads`]): copied, with nothing said of the map it
    /// replaces.
    pub fn open_list(&mut self, list: &crate::instrument_list::InstrumentList) {
        self.instrument_list.clone_from(list);
    }

    /// The free places whose instrument nothing plays any more (not the
    /// pads, nor a track by a send, nor one waiting to be the pads', nor
    /// a MIDI route that names it, step 11e), each
    /// taken out (step 6): a synth tried on the pads and left goes away,
    /// as nothing records it. Once a period; nothing allocated when there
    /// is none.
    pub fn put_away_free(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        let pads = self.pads_holder();
        for n in 0..crate::instruments::FREE {
            let holder = crate::instruments::free(n);
            let Some(held) = self.track_sound(holder) else {
                continue;
            };
            let sent = (0..mpfree_seq::MAX_TRACKS).any(|t| self.track_send(t) == Some(holder));
            let all = match self.base_now() {
                PadFocus::Instrument(id) => self.instrument_list.kind(id).and_then(|k| k.holder()),
                PadFocus::Track => None,
            };
            if pads == Some(holder)
                || all == Some(holder)
                || self.routes_hold(holder)
                || self.pads_to == Some(n)
                || sent
                || self.keeps_free(n)
                || self.banks_hold(holder)
                || self.layers_hold(holder)
            {
                continue;
            }
            self.set_track_sound(holder, None);
            fx.push(Effect::TrackSound {
                track: holder,
                id: None,
            });
            fx.push(Effect::Note(format!(
                "{}: nothing plays it, so it is put away",
                held.name
            )));
        }
        fx
    }

    /// The Synth tab's button under the instrument card: the pads play the
    /// synth the tab shows whichever track is selected, or, while they do,
    /// follow the selected track again. Nothing while the pads follow a
    /// track that plays a synth already: its own, or the shared one.
    pub fn toggle_pads_synth(&mut self) -> Vec<Effect> {
        // The shared synth, or a synth picked in the list (step 6): Follow
        // track.
        if self.pads_focus() != PadFocus::Track || self.pads_to.is_some() {
            return self.pads_follow_track();
        }
        if self.pad_layout.is_some() || self.track_plays_instrument(self.track) {
            return Vec::new();
        }
        // A pad held on the kit now still lets go there: the audio loop
        // plays on the synth only the hits that start from here.
        self.pad_map.base = PadFocus::SYNTH;
        self.project_edit = true;
        vec![
            Effect::PadsFocus(self.pads_focus()),
            Effect::Note(format!(
                "The pads play {} on every track · Track {} keeps its kit · REC records it on an empty track",
                self.synth_pads.engine.name(),
                self.track + 1
            )),
        ]
    }

    /// Follow track (the Synth tab's button again, or the "Pads: Poly"
    /// chip): the pads play the selected track's instrument again.
    pub fn pads_follow_track(&mut self) -> Vec<Effect> {
        self.pads_back(|t| format!("The pads follow the track: Track {t}'s instrument"))
    }

    /// REC while the pads play a synth picked in the list of synths (step
    /// 6), from holder `holder` of the instrument host: track `track` (0
    /// based) plays that same instrument from now on by a send
    /// ([`Effect::TrackSend`]), so the take plays the sound that was heard,
    /// with no copy. The pads keep it (rule 1). The caller arms `track`,
    /// which it added for this take when `added`.
    pub fn rec_send_on(&mut self, track: usize, holder: usize, added: bool) -> Vec<Effect> {
        let name = self
            .track_sound(holder)
            .map_or("the pads' synth", |h| h.name);
        let new = if added { "a new track, " } else { "" };
        self.project_edit = true;
        // Before the audio loop says so: the press that follows records
        // there.
        if let Some(s) = self.track_sends.get_mut(track) {
            *s = u8::try_from(holder).ok();
        }
        vec![
            Effect::TrackSend { track, to: holder },
            Effect::Note(format!(
                "REC records {name} on {new}Track {}: the track plays the same {name} as the pads",
                track + 1
            )),
        ]
    }

    /// The name of the synth the pads can play ("Poly").
    pub fn pads_synth_name(&self) -> &'static str {
        self.synth_pads.engine.name()
    }

    /// REC while the pads have the synth: track `track` (0 based) plays
    /// that same synth from now on, as picking Synth in the list of kits
    /// does ([`Effect::SynthOnTrack`]): one synth for the pads and the
    /// track, no copy, so the take plays the sound that was shaped and
    /// heard. The pads keep the synth (rule 1). The caller arms `track`
    /// ([`synth_track`]), which it added for this take when `added`.
    pub fn rec_synth_on(&mut self, track: usize, added: bool) -> Vec<Effect> {
        let name = self.synth_pads.engine.name();
        let new = if added { "a new track, " } else { "" };
        // The project keeps each track's kit: unsaved work.
        self.project_edit = true;
        if track < 128 {
            // Before the audio loop says so next period: the press that
            // follows records there.
            self.synth_tracks |= 1 << track;
        }
        vec![
            Effect::SynthOnTrack(track),
            Effect::Note(format!(
                "REC records {name} on {new}Track {}: the track plays the same {name} as the pads",
                track + 1
            )),
        ]
    }

    /// The holder whose instrument track `track` plays: its own, else the
    /// one it sends to, as the audio loop last said.
    pub fn track_owner(&self, track: usize) -> Option<usize> {
        if self.track_sound(track).is_some() {
            Some(track)
        } else {
            self.track_send(track)
        }
    }

    /// The tracks with a kit (not the synth) or a MIDI output picked, a bit
    /// each, as the audio loop has them (`TrackKits::picked`,
    /// `MidiOut::routed`): set up, so REC never gives them the pads' synth.
    pub fn set_configured(&mut self, tracks: u128) {
        self.configured = tracks;
    }

    /// Whether track `track` has a kit or a MIDI output picked
    /// ([`App::set_configured`]).
    pub fn configured(&self, track: usize) -> bool {
        track < 128 && self.configured >> track & 1 == 1
    }

    /// The tracks that play the shared synth (the Synth kit), a bit each,
    /// as the audio loop has them (`TrackKits::on_synth`).
    pub fn set_synth_tracks(&mut self, tracks: u128) {
        self.synth_tracks = tracks;
    }

    /// The tracks that play each kit the pad map names (its base, the
    /// banks' own and its layers, sampler steps 15, 15c and 15f) and each
    /// kit a MIDI route names (15f), as the audio loop has them
    /// (`on_kit`: `TrackKits::on_kit`, a bit each). Allocates nothing.
    pub fn set_kit_tracks(&mut self, on_kit: impl Fn(usize) -> u128) {
        let mut out = [(usize::MAX, 0); MAP_KITS];
        let mut n = 0;
        for k in self.map_kits() {
            if n < MAP_KITS && !out[..n].iter().any(|e| e.0 == k) {
                out[n] = (k, on_kit(k));
                n += 1;
            }
        }
        self.kit_tracks = out;
    }

    /// The kit places the pad map names that the list has: its base, then
    /// the banks' own, then its layers, then the kits the MIDI routes name
    /// (sampler step 15f).
    fn map_kits(&self) -> impl Iterator<Item = usize> + '_ {
        let base = std::iter::once(self.pad_map.base);
        let banks = self.pad_map.banks.iter().flatten().map(|e| e.play);
        let layers = self.pad_map.layers.iter().flatten();
        let layers = layers.map(|l| PadFocus::Instrument(l.id));
        let routes = self.route_synths.iter().flatten();
        let routes = routes.map(|&id| PadFocus::Instrument(id));
        base.chain(banks)
            .chain(layers)
            .chain(routes)
            .filter_map(|f| match self.sound_of(f)? {
                RecSound::Kit(k) => Some(k),
                RecSound::Synth(_) => None,
            })
    }

    /// Whether track `track` plays kit place `kit`, one the pad map names,
    /// with no instrument of its own or a send over it: REC records that
    /// kit there as it is (sampler steps 15 and 15c).
    pub fn plays_kit(&self, track: usize, kit: usize) -> bool {
        track < 128
            && self
                .kit_tracks
                .iter()
                .any(|&(k, tracks)| k == kit && tracks >> track & 1 == 1)
            && !self.track_plays_instrument(track)
    }

    /// Whether track `track` plays the kit chosen for the pads, with no
    /// instrument of its own or a send over it: REC records the pads there
    /// as they are (sampler step 15).
    pub fn plays_pads_kit(&self, track: usize) -> bool {
        self.pads_kit().is_some_and(|k| self.plays_kit(track, k))
    }

    /// What `f` plays as a sound REC records (sampler step 15c): the shared
    /// synth, a synth by its holder, a kit by its place; None for the
    /// track's instrument, or one the list hasn't got.
    pub fn sound_of(&self, f: PadFocus) -> Option<RecSound> {
        let PadFocus::Instrument(id) = f else {
            return None;
        };
        Some(match self.instrument_list.kind(id)? {
            crate::instrument_list::Kind::SharedSynth => RecSound::Synth(None),
            crate::instrument_list::Kind::Kit { kit } => RecSound::Kit(kit),
            k => RecSound::Synth(Some(k.holder()?)),
        })
    }

    /// The name REC gives what `f` plays: the synth's ("FM"), "a kit".
    pub(super) fn sound_name(&self, f: PadFocus) -> &'static str {
        match (f, self.sound_of(f)) {
            (_, Some(RecSound::Kit(_))) => "a kit",
            (PadFocus::Instrument(id), _) => self.instrument_name(id),
            (PadFocus::Track, _) => "the track's instrument",
        }
    }

    /// What the pads play now as a sound REC records ([`App::sound_of`]
    /// of [`App::pads_focus`]); None while they follow the track.
    pub fn pads_sound(&self) -> Option<RecSound> {
        self.sound_of(self.pads_focus())
    }

    /// Whether track `track` plays `sound`, as the audio loop last said:
    /// a synth ([`App::track_plays_synth`]) or a kit the map names
    /// ([`App::plays_kit`]).
    pub fn track_plays(&self, track: usize, sound: RecSound) -> bool {
        match sound {
            RecSound::Synth(holder) => self.track_plays_synth(track, holder),
            RecSound::Kit(kit) => self.plays_kit(track, kit),
        }
    }

    /// Whether a bank has its own now (sampler step 15c): while it does,
    /// REC keeps the arm where it is, so the banks that follow the track
    /// go on playing what they played, and records the pads' base on a
    /// track of its own like every bank's own ([`App::rec_map_sounds`]).
    pub fn map_has_banks(&self) -> bool {
        self.banks_apply() && (0..super::KIT_PAGES).any(|b| self.bank_play(b).is_some())
    }

    /// The sounds of the pad map REC records besides the first (sampler
    /// step 15c): while a bank has its own ([`App::map_has_banks`]), the
    /// base, then each bank's own, A to H, each sound once (a bank's own
    /// that follows the track records on the armed track). None of them
    /// otherwise: the base is the first. Allocates nothing.
    pub fn rec_map_sounds(&self) -> [Option<RecMore>; MAP_MORE] {
        let mut out = [None; MAP_MORE];
        if !self.map_has_banks() {
            return out;
        }
        let entries = std::iter::once((RecFrom::Base, self.base_now())).chain(
            (0..super::KIT_PAGES).filter_map(|b| self.bank_play(b).map(|f| (RecFrom::Bank(b), f))),
        );
        let mut n = 0;
        for (from, f) in entries {
            let Some(sound) = self.sound_of(f) else {
                continue;
            };
            if n < MAP_MORE
                && !out[..n]
                    .iter()
                    .flatten()
                    .any(|m: &RecMore| m.sound == sound)
            {
                out[n] = Some(RecMore {
                    name: self.sound_name(f),
                    sound,
                    from,
                });
                n += 1;
            }
        }
        out
    }

    /// REC while the pads have a kit the armed track doesn't play
    /// (sampler step 15): track `track` (0 based) plays that kit from now
    /// on, as picking it in the list of kits does ([`Effect::KitOnTrack`]),
    /// so the take plays the kit that was heard. The pads keep it (rule
    /// 1). The caller arms `track`, which it added for this take when
    /// `added`.
    pub fn rec_kit_on(&mut self, track: usize, kit: usize, added: bool) -> Vec<Effect> {
        let new = if added { "a new track, " } else { "" };
        self.project_edit = true;
        if track < 128 {
            // Before the audio loop says so: the press that follows
            // records there.
            if let Some(e) = self.kit_tracks.iter_mut().find(|e| e.0 == kit) {
                e.1 |= 1 << track;
            } else if let Some(e) = self.kit_tracks.iter_mut().find(|e| e.0 == usize::MAX) {
                *e = (kit, 1 << track);
            }
        }
        vec![
            Effect::KitOnTrack { track, kit },
            Effect::Note(format!(
                "REC records the pads' kit on {new}Track {}: the track plays the same kit as the pads",
                track + 1
            )),
        ]
    }

    /// Whether track `track` plays the synth the pads have: the shared
    /// synth, with no instrument of its own or MIDI output over it. REC
    /// records the pads there as they are.
    pub fn plays_pads_synth(&self, track: usize) -> bool {
        track < 128
            && self.synth_tracks >> track & 1 == 1
            && !self.configured(track)
            && !self.track_plays_instrument(track)
    }

    /// The pads part of the project: the pad map ([`PadMap::text`]), its
    /// layers and missing entries too.
    pub fn pads_part(&self) -> String {
        self.pad_map.text()
    }

    /// A project's pads part (None: New: the pads follow the track), after
    /// its list ([`App::open_list`]). An entry naming an instrument the
    /// list hasn't got is kept, missing, and said: that bank plays the
    /// base, the base the track's instrument, a layer nothing, until you
    /// pick another. A part that doesn't read is said, and the pads follow
    /// the track.
    pub fn open_pads(&mut self, text: Option<&str>) -> (Vec<String>, Vec<Effect>) {
        let mut notes = Vec::new();
        self.pads_to = None;
        self.bank_to = [None; super::KIT_PAGES];
        self.layer_to = [None; super::pad_layers::MORE];
        self.pad_map = match text.map(PadMap::parse) {
            None => PadMap::default(),
            Some(Ok(map)) => map,
            Some(Err(why)) => {
                notes.push(format!(
                    "project: the pads part wasn't opened ({why}); the pads follow the track"
                ));
                PadMap::default()
            }
        };
        notes.extend(
            self.say_missing(self.missing(), "isn't in the project's list")
                .into_iter()
                .map(|n| format!("project: {n}")),
        );
        (notes, vec![Effect::PadsFocus(self.pads_focus())])
    }

    /// The pads follow the selected track again, saying what `why` writes.
    /// Nothing (and no allocation) when they do already.
    pub(super) fn pads_back(&mut self, why: impl FnOnce(usize) -> String) -> Vec<Effect> {
        let banks = self.pad_map.banks.iter().any(Option::is_some)
            || self.bank_to.iter().any(Option::is_some);
        if self.pad_map.base == PadFocus::Track
            && self.pads_to.is_none()
            && !banks
            && !self.has_layers()
        {
            return Vec::new();
        }
        // Every bank too (step 6c), and the layers (step 11a): Follow
        // track is the pads following the track.
        self.pad_map = PadMap::default();
        self.pads_to = None;
        self.bank_to = [None; super::KIT_PAGES];
        self.clear_layers();
        self.project_edit = true;
        vec![
            Effect::PadsFocus(PadFocus::Track),
            Effect::Note(why(self.track + 1)),
        ]
    }
}

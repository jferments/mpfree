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
//! **What never moves the pads** (rule 1): arming or selecting a track, a
//! row tapped in REC's list, or REC. What does: Follow track; a kit picked
//! for the selected track in the list of kits (a choice of what the pads
//! play, as on the MPC: the pads follow the track and play it); a project
//! opened (its own focus); and the arp (below).
//!
//! **How the audio loop plays them** (`main.rs`, `preview_on`): while the
//! selected track plays the shared synth itself, the pads play it the
//! usual way, through the arp and the recorder, as the track's; while it
//! plays anything else, the loop plays the pad hits on the synth before the
//! arp and the recorder see them, so nothing records them there.
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
//! own are saved in the pads part (`bank B instrument 3`), written only
//! while a bank has one, so a project without stays readable by builds
//! from before.
//!
//! **The arp** (point C): the arp plays through the selected track, so
//! while the pads have the synth over a track that plays something else,
//! it can't play them: the button won't choose the synth while the arp
//! plays the pads (NOTE REPEAT held, Latch, or the Arp panel), and the arp
//! starting there gives the pads back to the track
//! ([`App::arp_ends_pads_synth`]). So a take never goes through an arp the
//! try-out didn't. Over a track that plays the synth, the arp plays it as
//! the track's. The arp by instrument comes with a later step.

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

/// The pads part's first line ([`crate::project::PADS`]).
pub const PADS_FORMAT: &str = "mpfree-pads 1";

impl PadFocus {
    /// The shared synth, chosen for the pads.
    pub const SYNTH: PadFocus = PadFocus::Instrument(InstrumentId::SHARED_SYNTH);

    /// The pads part: its first line, then `play track` or `play
    /// instrument 1`.
    pub fn text(self) -> String {
        match self {
            PadFocus::Track => format!("{PADS_FORMAT}\nplay track\n"),
            PadFocus::Instrument(id) => format!("{PADS_FORMAT}\nplay instrument {id}\n"),
        }
    }
}

/// The pads part as saved: what every bank plays, then what a bank plays
/// of its own (step 6c).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadsPart {
    /// What every bank plays ([`PadFocus::text`]'s line).
    pub all: PadFocus,
    /// What bank A to H plays of its own (None: what every bank plays).
    pub banks: [Option<PadFocus>; super::KIT_PAGES],
}

impl PadsPart {
    /// The part: [`PadFocus::text`], then a line for each bank with one of
    /// its own: `bank B track`, `bank B instrument 3`.
    pub fn text(&self) -> String {
        let mut text = self.all.text();
        for (b, f) in self.banks.iter().enumerate() {
            let Some(f) = f else { continue };
            let letter = super::bank_letter(b);
            match f {
                PadFocus::Track => text.push_str(&format!("bank {letter} track\n")),
                PadFocus::Instrument(id) => {
                    text.push_str(&format!("bank {letter} instrument {id}\n"));
                }
            }
        }
        text
    }

    /// The part read back ([`PadsPart::text`]); Err says why it doesn't
    /// read: another first line, a line it doesn't know, an instrument
    /// number that can't be one, what the pads play said twice or not at
    /// all, a bank past H or said twice. Whether the project's list has
    /// the instruments is [`App::open_pads`]'s to check.
    pub fn parse(text: &str) -> Result<PadsPart, String> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        match lines.next() {
            Some(PADS_FORMAT) => {}
            Some(other) => return Err(format!("its first line is \"{other}\"")),
            None => return Err("it is empty".to_string()),
        }
        let instrument = |line: &str, n: &str| {
            n.parse()
                .ok()
                .and_then(InstrumentId::from_number)
                .ok_or_else(|| format!("\"{line}\" doesn't read"))
        };
        let mut part = PadsPart::default();
        let mut all = None;
        for line in lines {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["play", "track"] | ["play", "instrument", _] => {
                    let f = match words.as_slice() {
                        ["play", "instrument", n] => PadFocus::Instrument(instrument(line, n)?),
                        _ => PadFocus::Track,
                    };
                    if all.replace(f).is_some() {
                        return Err("it says twice what the pads play".to_string());
                    }
                }
                ["bank", letter, rest @ ..] => {
                    let b =
                        bank_of_letter(letter).ok_or_else(|| format!("\"{line}\" doesn't read"))?;
                    let f = match rest {
                        ["track"] => PadFocus::Track,
                        ["instrument", n] => PadFocus::Instrument(instrument(line, n)?),
                        _ => return Err(format!("\"{line}\" doesn't read")),
                    };
                    if part.banks[b].replace(f).is_some() {
                        return Err(format!("it says twice what bank {letter} plays"));
                    }
                }
                _ => return Err(format!("\"{line}\" doesn't read")),
            }
        }
        part.all = all.ok_or_else(|| "it doesn't say what the pads play".to_string())?;
        Ok(part)
    }
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
    /// Where REC records each of the pads' layers beyond the first (step
    /// 11c), in their order, then each synth a MIDI route names (step
    /// 11e, [`App::rec_more_synths`]); None where there is none.
    pub layers: [Option<LayerTake>; super::REC_MORE],
}

/// Where REC records one of the pads' layers (the instrument redesign's
/// step 11c): as the first, where its synth lives. An armed track that
/// plays it already (one REC records too, or the armed track the first
/// moves the arm from), else a free track pointed at it, else a new one;
/// `Full` when the project holds no more, and that layer isn't recorded.
/// Never `Pinned`: the pin is the first's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerTake {
    pub take: SynthTake,
    /// The layer's synth's name ("Pluck").
    pub synth: &'static str,
    /// Its holder in the instrument host, as [`RecPlan::holder`]; None for
    /// the shared synth.
    pub holder: Option<usize>,
    /// The track plays the synth already: nothing on it changes.
    pub plays: bool,
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
    /// (sampler step 15), which the screen has.
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

/// Where REC records the pads' layers beyond the first (step 11c), in
/// order: an armed track that plays a layer's synth already, from
/// `candidates` (a bit each: the other armed tracks, and the armed one
/// when the first moves the arm from it), else the first free track, else
/// a new one while the project holds more. `first` is the track the
/// first records on (None: a new one). `plays` says whether a track plays
/// a layer (its holder, None for the shared synth); `fits` whether a
/// track is free ([`unfit`] says None). Allocates nothing.
pub fn layer_takes(
    layers: [Option<(&'static str, Option<usize>)>; super::REC_MORE],
    tracks: usize,
    first: Option<usize>,
    candidates: u128,
    plays: impl Fn(usize, Option<usize>) -> bool,
    fits: impl Fn(usize) -> bool,
) -> [Option<LayerTake>; super::REC_MORE] {
    let bit = |t: usize| 1u128.checked_shl(t as u32).unwrap_or(0);
    let mut used = first.map_or(0, bit);
    // New tracks the plan adds: the first's, then the layers'.
    let mut news = usize::from(first.is_none());
    layers.map(|l| {
        let (synth, holder) = l?;
        let armed = (0..tracks.min(128))
            .find(|&t| candidates & bit(t) != 0 && used & bit(t) == 0 && plays(t, holder));
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
            synth,
            holder,
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
    /// chosen for them; on a bank with one of its own (step 6c), that.
    pub fn pads_focus(&self) -> PadFocus {
        self.pads_bank()
            .and_then(|b| self.bank_focus[b])
            .unwrap_or(self.pads_focus)
    }

    /// The bank the pads are on when it plays something of its own (step
    /// 6c), for "Pads B: FM": on a kit's pages, not while the selected
    /// track plays the shared synth (PAD BANK picks its layout's banks
    /// then).
    pub fn pads_bank(&self) -> Option<usize> {
        let b = self.pad_page;
        (self.pad_layout.is_none() && self.bank_focus.get(b).copied().flatten().is_some())
            .then_some(b)
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
            plays(self.pads_focus) || self.pad_layers().any(|id| plays(PadFocus::Instrument(id)));
        if !self.banks_apply() {
            return (all, 0);
        }
        let every = self.bank_focus.iter().any(Option::is_none) && all;
        let banks = (0..super::KIT_PAGES)
            .filter(|&b| self.bank_focus[b].is_some_and(&plays))
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
        self.bank_focus.get(b).copied().flatten()
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
            || self.bank_focus.iter().flatten().any(|f| match f {
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
    /// id; an instrument chosen for them that has gone from the list gives
    /// them back to the track, said. Nothing to do, nothing allocated.
    pub fn set_instrument_list(
        &mut self,
        list: &crate::instrument_list::InstrumentList,
    ) -> Vec<Effect> {
        self.instrument_list.clone_from(list);
        let was = self.pads_focus();
        let mut fx = Vec::new();
        // A bank's synth that has its id now, or one gone (step 6c).
        for b in 0..super::KIT_PAGES {
            if let Some(id) = self.bank_to[b].and_then(|n| list.free_of(n)) {
                self.bank_to[b] = None;
                self.bank_focus[b] = Some(PadFocus::Instrument(id));
            }
            if let Some(PadFocus::Instrument(id)) = self.bank_focus[b] {
                if list.kind(id).is_none() {
                    self.bank_focus[b] = None;
                    self.project_edit = true;
                    fx.push(Effect::Note(format!(
                        "Bank {}'s synth is gone: the bank plays what the other banks play",
                        super::bank_letter(b)
                    )));
                }
            }
        }
        if let Some(id) = self.pads_to.and_then(|n| list.free_of(n)) {
            self.pads_to = None;
            self.pads_focus = PadFocus::Instrument(id);
        }
        if let PadFocus::Instrument(id) = self.pads_focus {
            if list.kind(id).is_none() {
                fx.extend(self.pads_back(|t| {
                    format!("The pads' synth is gone: they follow the track and play Track {t}'s instrument")
                }));
            }
        }
        // A layer's synth that has its id now, or one gone (step 11a); none
        // left to say once the pads' own synth went (Follow track gave
        // them up).
        fx.extend(self.layers_follow_list());
        if self.pads_focus() != was && !fx.iter().any(|e| matches!(e, Effect::PadsFocus(_))) {
            fx.insert(0, Effect::PadsFocus(self.pads_focus()));
        }
        fx
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
            let all = match self.pads_focus {
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
        if self.generating() {
            return vec![Effect::Tell(
                "The pads play the synth as pressed: let go of NOTE REPEAT or Latch, or close the Arp panel, first"
                    .into(),
            )];
        }
        // A pad held on the kit now still lets go there: the audio loop
        // plays on the synth only the hits that start from here.
        self.pads_focus = PadFocus::SYNTH;
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

    /// A kit picked for the selected track in the list of kits (or turned
    /// with SHIFT + the wheel): a choice of what the pads play, so they
    /// follow the track again and play it, as on the MPC. Nothing (and no
    /// allocation) while they follow it already.
    pub fn kit_pick_follows_track(&mut self) -> Vec<Effect> {
        self.pads_back(|t| {
            format!("Kit picked: the pads follow the track and play Track {t}'s kit")
        })
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

    /// Once a period: the arp plays the pads now (NOTE REPEAT, Latch or
    /// the Arp panel) while they have the synth over a track that plays
    /// something else, so the pads follow the track again and play its
    /// instrument through the arp, as a take would. Nothing (and no
    /// allocation) otherwise: over a track that plays the synth the arp
    /// plays it.
    pub fn arp_ends_pads_synth(&mut self) -> Vec<Effect> {
        // The shared synth over a track that plays something else (no
        // synth layout on the pads), or an instrument of the host's that
        // the track doesn't play.
        let over = match self.pads_focus() {
            PadFocus::Track => false,
            _ if self.pads_chose_synth() => self.pad_layout.is_none(),
            PadFocus::Instrument(_) => self
                .pads_holder()
                .is_some_and(|h| self.track_owner(self.track) != Some(h)),
        };
        // Layers (step 11a) play the pads as pressed, which the arp
        // doesn't.
        let over = over || (self.has_layers() && self.layers_apply());
        if !over || !self.generating() {
            return Vec::new();
        }
        self.pads_back(|t| {
            format!("The arp plays Track {t}'s instrument: the pads follow the track")
        })
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

    /// The tracks that play the kit chosen for the pads (sampler step 15),
    /// a bit each, as the audio loop has them (`TrackKits::on_kit`); 0
    /// while the pads have none.
    pub fn set_pads_kit_tracks(&mut self, tracks: u128) {
        self.pads_kit_tracks = tracks;
    }

    /// Whether track `track` plays the kit chosen for the pads, with no
    /// instrument of its own or a send over it: REC records the pads there
    /// as they are (sampler step 15).
    pub fn plays_pads_kit(&self, track: usize) -> bool {
        track < 128 && self.pads_kit_tracks >> track & 1 == 1 && !self.track_plays_instrument(track)
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
            self.pads_kit_tracks |= 1 << track;
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

    /// The pads part of the project ([`PadFocus::text`]).
    pub fn pads_part(&self) -> String {
        PadsPart {
            all: self.pads_focus,
            banks: self.bank_focus,
        }
        .text()
    }

    /// A project's pads part (None: New, or a project saved before it had
    /// one: the pads follow the track). A part that doesn't read, or names
    /// an instrument the project's list hasn't got
    /// ([`App::set_instrument_list`] first), is said, and the pads follow
    /// the track.
    pub fn open_pads(&mut self, text: Option<&str>) -> (Vec<String>, Vec<Effect>) {
        let mut notes = Vec::new();
        self.pads_to = None;
        self.bank_to = [None; super::KIT_PAGES];
        // The layers come after, from their own part (`open_pad_layers`).
        self.clear_layers();
        let listed = |f: &PadFocus| match f {
            PadFocus::Instrument(id) => self.instrument_list.kind(*id).is_some(),
            PadFocus::Track => true,
        };
        let part = match text.map(PadsPart::parse) {
            None => PadsPart::default(),
            Some(Ok(PadsPart {
                all: PadFocus::Instrument(id),
                ..
            })) if self.instrument_list.kind(id).is_none() => {
                notes.push(format!(
                    "project: the pads part wasn't opened (instrument {id} isn't in the project's list); the pads follow the track"
                ));
                PadsPart::default()
            }
            Some(Ok(mut p)) => {
                // A bank's instrument the list hasn't got: that bank plays
                // what the others play, said; the rest opens.
                for b in 0..super::KIT_PAGES {
                    if let Some(f) = p.banks[b].filter(|f| !listed(f)) {
                        let PadFocus::Instrument(id) = f else {
                            continue;
                        };
                        notes.push(format!(
                            "project: bank {}'s instrument {id} isn't in the project's list; the bank plays what the other banks play",
                            super::bank_letter(b)
                        ));
                        p.banks[b] = None;
                    }
                }
                p
            }
            Some(Err(why)) => {
                notes.push(format!(
                    "project: the pads part wasn't opened ({why}); the pads follow the track"
                ));
                PadsPart::default()
            }
        };
        self.pads_focus = part.all;
        self.bank_focus = part.banks;
        (notes, vec![Effect::PadsFocus(self.pads_focus())])
    }

    /// The pads follow the selected track again, saying what `why` writes.
    /// Nothing (and no allocation) when they do already.
    pub(super) fn pads_back(&mut self, why: impl FnOnce(usize) -> String) -> Vec<Effect> {
        let banks =
            self.bank_focus.iter().any(Option::is_some) || self.bank_to.iter().any(Option::is_some);
        if self.pads_focus == PadFocus::Track
            && self.pads_to.is_none()
            && !banks
            && !self.has_layers()
        {
            return Vec::new();
        }
        // Every bank too (step 6c), and the layers (step 11a): Follow
        // track is the pads following the track.
        self.pads_focus = PadFocus::Track;
        self.pads_to = None;
        self.bank_focus = [None; super::KIT_PAGES];
        self.bank_to = [None; super::KIT_PAGES];
        self.clear_layers();
        self.project_edit = true;
        vec![
            Effect::PadsFocus(PadFocus::Track),
            Effect::Note(why(self.track + 1)),
        ]
    }
}

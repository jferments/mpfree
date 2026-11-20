//! The Synth tab's options (MENU on the Synth tab): which engine the pads
//! play, and how the pads are laid out from a scale (owner, 2026-11-11;
//! `docs/dev/design/scales.md`, "The pads").
//!
//! - **Engine**: Poly, FM, Drums or Pluck. The other engine's notes finish
//!   their release. Drums plays a sound on each pad from the kit's Pad 1
//!   note, so the scale, layout and octave don't apply to it.
//! - **Scale** and **Scale root**: the pads' own scale, Off until one is
//!   picked (the arp has its own). With no scale every layout is plain
//!   Chromatic, as in round 6.
//! - **Pad layout**: the bank playing's layout, Chromatic (the scale lit),
//!   In key or Chord pads. Banks A to H start as `DEFAULT_BANKS`.
//! - **In key rows**: in sequence, or a fourth apart.
//! - **Tuning**, **Tuning root** and **A4**: how notes become pitches for
//!   every engine but Drums: 12-TET, just intonation, Pythagorean,
//!   quarter-comma meantone or 24-TET quarter tones, then the musician's
//!   Scala files ([`App::tuning_files`]); the root the table tunings are
//!   pure on, and A4 from 380 to 480 Hz. A file's keyboard map, when it
//!   has one, sets the root and pitch itself.
//!
//! The app keeps these and hands them to the synth as
//! [`Effect::SynthPads`]; the audio loop applies them. The pads panel's
//! Chromatic | In key switch ([`App::synth_pick_layout`]) and the PAD BANK
//! buttons picking a bank's layout ([`App::synth_bank`]) change them too.
//!
//! They are the song's (Jay, 13 Nov): kept in the project's "synth" part
//! ([`App::synth_settings`], put back by [`App::open_synth`]), not in
//! Prefs, so a project's key and tuning don't follow you into the next
//! one. New project starts chromatic in C at A4 = 440. Every row above is
//! kept, and each bank's layout; the bank playing and the octave are not:
//! the pads stay where they are.

use super::{App, Effect, Item, View};
use crate::synth::{Builtin, Engine, A4_MAX, A4_MIN, BANKS};
use mpfree_arp::scale::pads::{InKeyRows, PadLayout};
use mpfree_arp::scale::{Scale, NOTE_NAMES, SCALES};
use mpfree_store::Settings;

/// The Synth tab's rows in its options menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthItem {
    Engine,
    Scale,
    Root,
    Layout,
    Rows,
    Tuning,
    TuningRoot,
    A4,
}

/// The options menu opened from the Synth tab: the synth's own rows, then
/// the Play rows as before.
pub(super) const SYNTH_OPTIONS: [Item; 12] = [
    Item::Synth(SynthItem::Engine),
    Item::Synth(SynthItem::Scale),
    Item::Synth(SynthItem::Root),
    Item::Synth(SynthItem::Layout),
    Item::Synth(SynthItem::Rows),
    Item::Synth(SynthItem::Tuning),
    Item::Synth(SynthItem::TuningRoot),
    Item::Synth(SynthItem::A4),
    Item::Rate,
    Item::Tempo,
    Item::PadLevel,
    Item::Latch,
];

/// Each bank's key in the "synth" settings.
const BANK_KEYS: [&str; BANKS] = [
    "bank-a", "bank-b", "bank-c", "bank-d", "bank-e", "bank-f", "bank-g", "bank-h",
];

/// One of the Instrument row's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SoundChoice {
    /// The track's kit (or the synth, as its pick in the list of kits).
    Kit,
    /// An instrument of its own: an engine or a plugin synth (node id,
    /// name).
    Sound(&'static str, &'static str),
    /// Another track's own instrument, played from this track (step 4).
    Track(usize),
}

/// Track `owner`'s instrument `name` as a track that plays it names it,
/// on the Synth tab and under the track on Arrange: "Track 3's FM".
pub fn sent_name(owner: usize, name: &str) -> String {
    format!("Track {}'s {name}", owner + 1)
}

fn layout_key(l: PadLayout) -> &'static str {
    match l {
        PadLayout::Chromatic => "chromatic",
        PadLayout::InKey => "in-key",
        PadLayout::Chords => "chords",
    }
}

fn rows_key(r: InKeyRows) -> &'static str {
    match r {
        InKeyRows::Sequence => "sequence",
        InKeyRows::Fourths => "fourths",
    }
}

fn builtin_key(b: Builtin) -> &'static str {
    match b {
        Builtin::Equal12 => "12-tet",
        Builtin::Just => "just",
        Builtin::Pythagorean => "pythagorean",
        Builtin::Meantone => "meantone",
        Builtin::Equal24 => "24-tet",
    }
}

/// A note name ("C#", any case) as 0 to 11.
fn note_of(v: &str) -> Option<u8> {
    NOTE_NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(v))
        .map(|i| i as u8)
}

/// The tuning file prefix in the "synth" settings: `tuning = file NAME`.
const FILE_PREFIX: &str = "file ";

/// The longest value a row shows, as long as the longest built-in one.
const VALUE_CHARS: usize = 20;

/// `name`, cut to [`VALUE_CHARS`] with an ellipsis when it is longer.
fn fit(name: &str) -> String {
    if name.chars().count() <= VALUE_CHARS {
        return name.to_string();
    }
    let mut s: String = name.chars().take(VALUE_CHARS - 1).collect();
    s.push('…');
    s
}

fn step_in<T: Copy + PartialEq>(all: &[T], now: T, d: i32) -> T {
    let at = all.iter().position(|&x| x == now).unwrap_or(0) as i32;
    all[(at + d).clamp(0, all.len() as i32 - 1) as usize]
}

/// The Scale row's choice for Major (0 is Off; Chromatic is skipped).
const MAJOR_CHOICE: usize = 1;

impl App {
    /// The Scale row's choices: 0 is Off, then the built-in scales but
    /// Chromatic (which lays the pads out as Off does), then the owner's.
    fn pad_scale_choices(&self) -> usize {
        SCALES.len() + self.custom_scales.len()
    }

    /// Choice `i`'s name and notes (up, down); None for Off.
    fn pad_scale_entry(&self, i: usize) -> Option<(&str, u16, u16)> {
        match i {
            0 => None,
            i if i < SCALES.len() => Some((SCALES[i].name, SCALES[i].mask, SCALES[i].down)),
            i => self
                .custom_scales
                .get(i - SCALES.len())
                .map(|(n, u, d)| (n.as_str(), *u, *d)),
        }
    }

    fn pad_scale_choice(&self) -> usize {
        if self.synth_pads.scale.is_some() {
            self.pad_scale_pick
        } else {
            0
        }
    }

    /// The Tuning row's choice: the built-in tunings, then the files.
    fn tuning_choice(&self) -> usize {
        let t = &self.synth_pads.tuning;
        match t.file {
            Some(f) if usize::from(f) < self.tuning_files.len() => {
                Builtin::ALL.len() + usize::from(f)
            }
            _ => Builtin::ALL
                .iter()
                .position(|&b| b == t.builtin)
                .unwrap_or(0),
        }
    }

    /// The tuning file picked, as (name, whether its map sets root and A4).
    fn tuning_file(&self) -> Option<&(String, bool)> {
        self.tuning_files
            .get(usize::from(self.synth_pads.tuning.file?))
    }

    pub(super) fn synth_item_text(&self, item: SynthItem) -> (&'static str, String) {
        let p = &self.synth_pads;
        let file = self.tuning_file();
        let mapped = file.is_some_and(|f| f.1);
        match item {
            SynthItem::Engine => (instrument_label(self.track), self.engine_text()),
            SynthItem::Scale => (
                "Scale",
                match self.pad_scale_entry(self.pad_scale_choice()) {
                    None => "Off".to_string(),
                    Some((name, ..)) => name.to_string(),
                },
            ),
            SynthItem::Root => (
                "Scale root",
                NOTE_NAMES[usize::from(self.pad_scale_root % 12)].to_string(),
            ),
            SynthItem::Layout => (
                "Pad layout",
                if p.scale.is_some() {
                    format!(
                        "{} (bank {})",
                        p.layout().label(),
                        super::bank_letter(p.bank)
                    )
                } else {
                    "Chromatic (no scale)".to_string()
                },
            ),
            SynthItem::Rows => ("In key rows", p.rows.label().to_string()),
            SynthItem::Tuning => (
                "Tuning",
                match file {
                    Some((name, _)) => fit(name),
                    None => p.tuning.builtin.name().to_string(),
                },
            ),
            SynthItem::TuningRoot => {
                let root = NOTE_NAMES[usize::from(p.tuning.root % 12)];
                (
                    "Tuning root",
                    if mapped {
                        "set by its .kbm".to_string()
                    } else if file.is_some() || p.tuning.builtin.has_root() {
                        root.to_string()
                    } else {
                        format!("{root} (not used)")
                    },
                )
            }
            SynthItem::A4 => (
                "A4",
                if mapped {
                    "set by its .kbm".to_string()
                } else {
                    format!("{} Hz", p.tuning.a4_hz)
                },
            ),
        }
    }

    pub(super) fn synth_item_help(item: SynthItem) -> Vec<&'static str> {
        match item {
            SynthItem::Engine => vec![
                "What the selected track plays: Kit (the pads'",
                "kit, as every track starts), or its own",
                "instrument: Poly (subtractive), FM (six",
                "operators), Drums (a sound on each pad), Pluck",
                "(strings and mallets) and the others, then",
                "the plugin synths in files/plugins/. It",
                "starts as a copy of the Synth tab's sound,",
                "and the tab then edits the track's own. The",
                "pads play the selected track; the old sound",
                "rings out.",
            ],
            SynthItem::Scale => vec![
                "The pads' own scale (the arp has its own).",
                "Off: pad 1 is C3 and each pad a semitone up.",
                "With a scale, Pad layout picks how the pads",
                "use it. Scales from your patterns file come",
                "after the built-in ones.",
            ],
            SynthItem::Root => vec![
                "The scale's first note. In key and Chord pads",
                "start on it, in octave 3.",
            ],
            SynthItem::Layout => vec![
                "Chromatic: a semitone a pad, the scale lit",
                "(root, in the scale, outside it). In key:",
                "only scale notes. Chord pads: triads on the",
                "bottom rows, sevenths on the top rows. Each",
                "pad bank keeps its own layout: A Chromatic,",
                "B In key, C Chord pads, D In key an octave up.",
            ],
            SynthItem::Rows => vec![
                "In key: the pads climb the scale in sequence,",
                "or each row starts a fourth (three scale",
                "notes) above the row below, so a chord keeps",
                "one shape.",
            ],
            SynthItem::Tuning => vec![
                "How notes become pitches: 12-TET (the usual),",
                "just intonation (pure thirds and fifths on",
                "the root), Pythagorean (pure fifths),",
                "quarter-comma meantone (pure thirds) or",
                "24-TET (each pad a quarter tone). Scala .scl",
                "files in files/tunings/ come after them.",
                "Drums keep their own pitch. Held notes follow",
                "at once.",
            ],
            SynthItem::TuningRoot => vec![
                "The note just intonation, Pythagorean,",
                "meantone and a tuning file start on. It keeps",
                "its usual pitch and the others move around",
                "it. The equal tunings don't use it, and a",
                "file's .kbm keyboard map sets its own.",
            ],
            SynthItem::A4 => vec![
                "The pitch of the A above middle C, 380 to",
                "480 Hz: 440 is the usual, 432 and 442 are",
                "common, 415 is baroque pitch. A file's .kbm",
                "keyboard map sets its own.",
            ],
        }
    }

    /// One of the synth's option rows stepped by `d` (-1 or +1).
    pub(super) fn step_synth_item(&mut self, item: SynthItem, d: i32) -> Vec<Effect> {
        let before = (self.synth_pads, self.pad_scale_pick, self.pad_scale_root);
        match item {
            SynthItem::Engine => return self.step_track_sound(d),
            SynthItem::Scale => {
                self.pad_scale_pick = (self.pad_scale_choice() as i32 + d)
                    .clamp(0, self.pad_scale_choices() as i32 - 1)
                    as usize;
                self.synth_pads.scale = self
                    .pad_scale_entry(self.pad_scale_pick)
                    .map(|(_, up, down)| Scale::with_down(self.pad_scale_root, up, down));
            }
            SynthItem::Root => {
                self.pad_scale_root = (i32::from(self.pad_scale_root) + d).clamp(0, 11) as u8;
                let root = self.pad_scale_root;
                self.synth_pads.scale = self
                    .synth_pads
                    .scale
                    .map(|s| Scale::with_down(root, s.mask, s.down));
            }
            SynthItem::Layout => {
                let l = step_in(&PadLayout::ALL, self.synth_pads.layout(), d);
                self.synth_pads.set_layout(l);
            }
            SynthItem::Rows => {
                self.synth_pads.rows = step_in(&InKeyRows::ALL, self.synth_pads.rows, d)
            }
            SynthItem::Tuning => {
                let last = Builtin::ALL.len() + self.tuning_files.len() - 1;
                let at = (self.tuning_choice() as i32 + d).clamp(0, last as i32) as usize;
                let t = &mut self.synth_pads.tuning;
                // With a file picked, the built-in tuning is 12-TET: what
                // plays if the file were missing, and what a kept file
                // pick comes back with.
                (t.builtin, t.file) = match Builtin::ALL.get(at) {
                    Some(&b) => (b, None),
                    None => (Builtin::Equal12, Some((at - Builtin::ALL.len()) as u8)),
                };
            }
            SynthItem::TuningRoot => {
                let t = &mut self.synth_pads.tuning;
                t.root = (i32::from(t.root % 12) + d).clamp(0, 11) as u8;
            }
            SynthItem::A4 => {
                let t = &mut self.synth_pads.tuning;
                t.a4_hz =
                    (i32::from(t.a4_hz) + d).clamp(i32::from(A4_MIN), i32::from(A4_MAX)) as u16;
            }
        }
        if (self.synth_pads, self.pad_scale_pick, self.pad_scale_root) == before {
            return Vec::new();
        }
        self.project_edit = true;
        vec![Effect::SynthPads(self.synth_pads)]
    }

    /// The Instrument row's choices for a track: Kit, the other tracks'
    /// own instruments (step 4: a track can play another track's, so the
    /// project's instruments come before new ones), then a new one of its
    /// own: the built-in engines and the plugin synths.
    fn sound_choices(&self) -> Vec<SoundChoice> {
        std::iter::once(SoundChoice::Kit)
            .chain(
                (0..mpfree_seq::MAX_TRACKS)
                    .filter(|&t| t != self.track && self.track_sound(t).is_some())
                    .map(SoundChoice::Track),
            )
            .chain(
                Engine::ALL
                    .iter()
                    .map(|e| SoundChoice::Sound(e.node_id(), e.name())),
            )
            .chain(
                self.plugin_synths
                    .iter()
                    .map(|&(id, name)| SoundChoice::Sound(id, name)),
            )
            .collect()
    }

    /// Where the selected track is on the Instrument row's choices.
    fn sound_choice_now(&self, choices: &[SoundChoice]) -> usize {
        let now = match (self.track_send(self.track), self.track_sound(self.track)) {
            (Some(o), _) => SoundChoice::Track(o),
            (None, Some(h)) => SoundChoice::Sound(h.id, h.name),
            (None, None) => SoundChoice::Kit,
        };
        choices
            .iter()
            .position(|c| match (*c, now) {
                (SoundChoice::Sound(a, _), SoundChoice::Sound(b, _)) => a == b,
                (c, now) => c == now,
            })
            .unwrap_or(0)
    }

    /// The Instrument row's value: what the selected track plays.
    fn engine_text(&self) -> String {
        match (self.track_send(self.track), self.track_sound(self.track)) {
            (Some(o), _) => self.sent_name(o),
            (None, Some(h)) => h.name.to_string(),
            (None, None) => "Kit".to_string(),
        }
    }

    /// Track `owner`'s instrument as a track that plays it names it:
    /// "Track 3's FM".
    pub fn sent_name(&self, owner: usize) -> String {
        sent_name(
            owner,
            self.track_sound(owner).map_or("instrument", |h| h.name),
        )
    }

    /// The Instrument row stepped by `d`: the selected track's sound, Kit first,
    /// then the engines and the plugin synths.
    fn step_track_sound(&mut self, d: i32) -> Vec<Effect> {
        let choices = self.sound_choices();
        let at = self.sound_choice_now(&choices);
        let to = (at as i32 + d).clamp(0, choices.len() as i32 - 1) as usize;
        if to == at {
            return Vec::new();
        }
        let track = self.track;
        let pick = match choices[to] {
            SoundChoice::Kit => None,
            SoundChoice::Sound(id, name) => Some((id, name)),
            SoundChoice::Track(owner) => return self.send_track_to(owner),
        };
        // A track with no instrument of its own takes a place only while
        // one is free.
        let others = self.track_sounds.iter().flatten().count();
        if pick.is_some()
            && self.track_sound(track).is_none()
            && others >= crate::instruments::SLOTS
        {
            return vec![Effect::Tell(format!(
                "Track {} plays the kit: {} tracks already have their own instrument, the most at once. Set one of them to Kit first",
                self.track + 1,
                crate::instruments::SLOTS
            ))];
        }
        // The project keeps each track's instrument: unsaved work.
        self.project_edit = true;
        match pick {
            // Its own goes, and the tracks that played it play their kits.
            None => self.end_sends(track),
            // A new sound in its own keeps the tracks that play it (the
            // instrument keeps its id).
            Some(_) => self.track_sends[track] = None,
        }
        self.set_track_sound(
            track,
            pick.map(|(id, name)| crate::instruments::Held {
                id,
                name,
                instance: 0,
            }),
        );
        vec![Effect::TrackSound {
            track,
            id: pick.map(|c| c.0),
        }]
    }

    /// The Instrument row's pick of track `owner`'s instrument for the
    /// selected track (step 4): its own instrument goes, and the tracks
    /// that played that play their kits again (the loop says so).
    fn send_track_to(&mut self, owner: usize) -> Vec<Effect> {
        let track = self.track;
        self.project_edit = true;
        if self.track_sound(track).is_some() {
            self.set_track_sound(track, None);
            self.end_sends(track);
        }
        self.track_sends[track] = Some(owner as u8);
        vec![Effect::TrackSend { track, to: owner }]
    }

    /// The selected track's send ends, as it gets its own instrument or the
    /// kit again; so do the sends to its own instrument, if it goes.
    fn end_sends(&mut self, track: usize) {
        self.track_sends[track] = None;
        for s in self.track_sends.iter_mut() {
            if s.is_some_and(|o| usize::from(o) == track) {
                *s = None;
            }
        }
    }

    /// A tap on track `track`'s sound line on Arrange: the Synth tab's
    /// MENU opens on the Instrument row, for that track (the screen arms
    /// it, so the selected track follows).
    pub fn open_track_instrument(&mut self, track: usize) -> Vec<Effect> {
        if self.view == View::Guide {
            return Vec::new();
        }
        let mut fx = self.set_track(track);
        if self.view != View::Synth {
            fx.extend(self.show(View::Synth));
        }
        // MENU there: the Options rows, the Instrument row first.
        fx.extend(self.show(View::Options));
        fx
    }

    /// What track `track` plays as its own instrument (None: the kit, or
    /// another track's instrument, [`App::track_send`]).
    pub fn track_sound(&self, track: usize) -> Option<crate::instruments::Held> {
        self.track_sounds.get(track).copied().flatten()
    }

    /// The track whose instrument track `track` plays though it has none of
    /// its own (step 4).
    pub fn track_send(&self, track: usize) -> Option<usize> {
        self.track_sends
            .get(track)
            .copied()
            .flatten()
            .map(usize::from)
    }

    /// Whether track `track` plays an instrument rather than its kit: its
    /// own, or another track's.
    pub fn track_plays_instrument(&self, track: usize) -> bool {
        self.track_sound(track).is_some() || self.track_send(track).is_some()
    }

    /// Each track's send as the audio loop has it now
    /// (`Instruments::send`).
    pub fn set_track_sends(&mut self, sends: &[Option<u8>; mpfree_seq::MAX_TRACKS]) {
        self.track_sends = *sends;
    }

    /// Track `track`'s own instrument as the audio loop has it now.
    pub fn set_track_sound(&mut self, track: usize, held: Option<crate::instruments::Held>) {
        if let Some(t) = self.track_sounds.get_mut(track) {
            *t = held;
        }
    }

    /// The pads panel's Chromatic | In key switch (Jay, 12:22Z): the bank
    /// playing takes In key (`in_key`) or Chromatic. In key needs a scale:
    /// with none yet, the tap picks Major on the Scale root (the tap is the
    /// opt-in; no key is forced before it), and the screen says which, so
    /// the switch visibly works (live test 3). Every tap says what the pads
    /// play now.
    pub fn synth_pick_layout(&mut self, in_key: bool) -> Vec<Effect> {
        let mut picked = false;
        if in_key && self.synth_pads.scale.is_none() {
            self.pad_scale_pick = MAJOR_CHOICE;
            self.synth_pads.scale = self
                .pad_scale_entry(MAJOR_CHOICE)
                .map(|(_, up, down)| Scale::with_down(self.pad_scale_root, up, down));
            picked = true;
        }
        let want = if in_key {
            PadLayout::InKey
        } else {
            PadLayout::Chromatic
        };
        let scale = match self.pad_scale_entry(self.pad_scale_choice()) {
            Some((name, ..)) => format!(
                "{} {name}",
                NOTE_NAMES[usize::from(self.pad_scale_root % 12)]
            ),
            None => "no scale".to_string(),
        };
        if self.synth_pads.layout() == want && !picked {
            return vec![Effect::Tell(format!(
                "pads: already {} ({scale})",
                if in_key { "In key" } else { "Chromatic" }
            ))];
        }
        self.synth_pads.set_layout(want);
        self.project_edit = true;
        let line = if in_key && picked {
            format!("pads: In key, {scale} (MENU on the Synth tab, Scale, changes it)")
        } else if in_key {
            format!("pads: In key, {scale}")
        } else {
            format!("pads: Chromatic ({scale} lit)")
        };
        vec![Effect::SynthPads(self.synth_pads), Effect::Tell(line)]
    }

    /// The pads take bank `bank`'s layout (0 = A to 7 = H): the PAD BANK
    /// buttons while the synth has the pads (Jay, 09:55Z).
    pub fn synth_bank(&mut self, bank: usize) -> Vec<Effect> {
        let bank = bank % BANKS;
        if bank == self.synth_pads.bank {
            return Vec::new();
        }
        self.synth_pads.bank = bank;
        vec![Effect::SynthPads(self.synth_pads)]
    }

    /// The "synth" settings: the Synth tab's options and each bank's
    /// layout, as words (`engine = fm`, `scale = Minor`, `bank-b = in-key`,
    /// `tuning = just` or `tuning = file werckmeister3`).
    pub fn synth_settings(&self) -> Settings {
        let p = &self.synth_pads;
        let mut out = Settings::default();
        out.set_str("engine", &p.engine.name().to_lowercase());
        let scale = match self.pad_scale_entry(self.pad_scale_choice()) {
            None => "off",
            Some((name, ..)) => name,
        };
        // A name the settings can't hold is left out (and so not put back).
        out.set_str("scale", scale);
        out.set_str(
            "scale-root",
            NOTE_NAMES[usize::from(self.pad_scale_root % 12)],
        );
        out.set_str("in-key-rows", rows_key(p.rows));
        for (key, (layout, _)) in BANK_KEYS.iter().zip(p.banks) {
            out.set_str(key, layout_key(layout));
        }
        let t = &p.tuning;
        match self.tuning_file() {
            Some((name, _)) => out.set_str("tuning", &format!("{FILE_PREFIX}{name}")),
            None => out.set_str("tuning", builtin_key(t.builtin)),
        };
        out.set_str("tuning-root", NOTE_NAMES[usize::from(t.root % 12)]);
        out.set_i64("a4", i64::from(t.a4_hz));
        out
    }

    /// A project's Synth tab options (its "synth" part), over the
    /// defaults: an option the project doesn't have (or can't be used) is
    /// the default, never the last project's. Returns the lines for the log
    /// and the effect that hands the synth its settings.
    pub fn open_synth(&mut self, s: &Settings) -> (Vec<String>, Vec<Effect>) {
        // The bank playing and each bank's octave stay where they are.
        let old = self.synth_pads;
        self.synth_pads = crate::synth::PadSettings {
            bank: old.bank,
            ..Default::default()
        };
        for (b, (_, octave)) in self.synth_pads.banks.iter_mut().zip(old.banks) {
            b.1 = octave;
        }
        (self.pad_scale_pick, self.pad_scale_root) = (0, 0);
        let (notes, _) = self.restore_synth(s);
        (notes, vec![Effect::SynthPads(self.synth_pads)])
    }

    /// Puts back the Synth tab's options from `s` (words as
    /// [`App::synth_settings`] writes them) over the ones in use. The
    /// owner's scales and the tuning files must be known. Returns the lines
    /// for the log (what was put back, and anything left out: a word not
    /// understood, a scale or tuning file that is gone) and the effect that
    /// hands the synth its settings (none when `s` is empty).
    pub fn restore_synth(&mut self, s: &Settings) -> (Vec<String>, Vec<Effect>) {
        let mut skipped = Vec::new();
        let mut scale = None;
        for (k, v) in s.iter() {
            let p = &mut self.synth_pads;
            let ok = match k {
                // ("Wavetable", gone into Poly, is Poly.)
                "engine" => Engine::from_name(v).map(|e| p.engine = e).is_some(),
                // Looked up once the root is known.
                "scale" => {
                    scale = Some(v);
                    true
                }
                "scale-root" => note_of(v).map(|n| self.pad_scale_root = n).is_some(),
                "in-key-rows" => InKeyRows::ALL
                    .iter()
                    .find(|&&r| rows_key(r) == v)
                    .map(|&r| p.rows = r)
                    .is_some(),
                "tuning" => match v.strip_prefix(FILE_PREFIX) {
                    Some(name) => self
                        .tuning_files
                        .iter()
                        .position(|(n, _)| n == name)
                        .and_then(|i| u8::try_from(i).ok())
                        .map(|i| (p.tuning.builtin, p.tuning.file) = (Builtin::Equal12, Some(i)))
                        .is_some(),
                    None => Builtin::ALL
                        .iter()
                        .find(|&&b| builtin_key(b) == v)
                        .map(|&b| (p.tuning.builtin, p.tuning.file) = (b, None))
                        .is_some(),
                },
                "tuning-root" => note_of(v).map(|n| p.tuning.root = n).is_some(),
                "a4" => s
                    .get_i64(k)
                    .filter(|n| (i64::from(A4_MIN)..=i64::from(A4_MAX)).contains(n))
                    .map(|n| p.tuning.a4_hz = n as u16)
                    .is_some(),
                k => match BANK_KEYS.iter().position(|&b| b == k) {
                    Some(b) => PadLayout::ALL
                        .iter()
                        .find(|&&l| layout_key(l) == v)
                        .map(|&l| p.banks[b].0 = l)
                        .is_some(),
                    None => false,
                },
            };
            if !ok {
                skipped.push(format!("{k} = {v}"));
            }
        }
        if let Some(name) = scale {
            let found = if name == "off" {
                Some(0)
            } else {
                (1..self.pad_scale_choices())
                    .find(|&i| self.pad_scale_entry(i).is_some_and(|(n, ..)| n == name))
            };
            match found {
                Some(i) => {
                    self.pad_scale_pick = i;
                    let root = self.pad_scale_root;
                    self.synth_pads.scale = self
                        .pad_scale_entry(i)
                        .map(|(_, up, down)| Scale::with_down(root, up, down));
                }
                None => skipped.push(format!("scale = {name}")),
            }
        } else if let Some(sc) = self.synth_pads.scale {
            // No scale kept: the root still moves the one picked.
            self.synth_pads.scale = Some(Scale::with_down(self.pad_scale_root, sc.mask, sc.down));
        }
        let mut notes = Vec::new();
        if !s.is_empty() {
            let text = |item| self.synth_item_text(item).1;
            notes.push(format!(
                "synth options: {}, scale {} on {}, pad layout {}, tuning {} (root {}, A4 {})",
                self.synth_pads.engine.name(),
                text(SynthItem::Scale),
                text(SynthItem::Root),
                text(SynthItem::Layout),
                text(SynthItem::Tuning),
                text(SynthItem::TuningRoot),
                text(SynthItem::A4),
            ));
        }
        if !skipped.is_empty() {
            notes.push(format!(
                "synth options left out (not understood, or the scale or tuning file is gone): {}",
                skipped.join(", ")
            ));
        }
        let fx = if s.is_empty() {
            Vec::new()
        } else {
            vec![Effect::SynthPads(self.synth_pads)]
        };
        (notes, fx)
    }
}

/// The Instrument row's label, naming the selected track so the row says
/// whose sound it sets ("Track 2 instrument").
fn instrument_label(track: usize) -> &'static str {
    const LABELS: [&str; 16] = [
        "Track 1 instrument",
        "Track 2 instrument",
        "Track 3 instrument",
        "Track 4 instrument",
        "Track 5 instrument",
        "Track 6 instrument",
        "Track 7 instrument",
        "Track 8 instrument",
        "Track 9 instrument",
        "Track 10 instrument",
        "Track 11 instrument",
        "Track 12 instrument",
        "Track 13 instrument",
        "Track 14 instrument",
        "Track 15 instrument",
        "Track 16 instrument",
    ];
    LABELS.get(track).copied().unwrap_or("Instrument")
}

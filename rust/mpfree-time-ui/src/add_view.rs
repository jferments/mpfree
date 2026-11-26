//! Add a part and the row's chords on Arrange's Clips grid (composing
//! tools S2.6, layout spec 4r): the panel's choices and how it looks
//! ([`AddPick`], [`AddPanel`]), the chord picker's ([`ChordPick`]), the
//! Record panel's Chords tab ([`ChordsCard`]), and what the grid shows of
//! them: the part being tried in its cell, the clips off their row's
//! chords, the row's chords under it ([`strip_of`]).
//!
//! The time engine does the work (`TimeAction::Add`, `TimeAction::Chords`,
//! [`crate::adding`]); the page keeps only what the screen picked. The
//! design is docs/dev/design/compose.md (S2.6).

use std::collections::BTreeSet;
use std::sync::Arc;

use mpfree_arp::{Phrase, PhraseKind};
use mpfree_compose::{style_at, style_count, style_name, ClipStyle};
use mpfree_compose::{InstrumentKind, Lanes, Relation, Settings, STYLES};
use mpfree_context::chord::{Chord, ChordAnswer, ChordSpan, ChordTimeline, Heard, Quality};
use mpfree_context::PartRole;
use mpfree_seq::Holds;
use mpfree_transport::time::Tick;
use mpfree_ui::widgets::OptionList;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

use crate::adding::{AddAction, AddState, ChordsAction, NewTrack, PartAsk};
use crate::chord_strip::{ChipLook, ChordStrip, PickerView, StripChip};
use crate::editor::compose::{cards, opening_q, RANGES};
use crate::view::{track_color, TimeAction, TimeState};
use crate::widgets::{CellMark, CellState, GridCell};

/// The parts Add a part makes, as its four tiles show them.
pub const PARTS: [PartRole; 4] = [
    PartRole::Bass,
    PartRole::Chords,
    PartRole::Melody,
    PartRole::Drums,
];

/// The part's lengths the Bars row steps: the row's, then whole bars.
pub const PART_BARS: [Option<u32>; 6] = [None, Some(1), Some(2), Some(4), Some(8), Some(16)];

/// The picker's kinds of chord, in [`crate::chord_strip::KINDS`]' order.
pub const KIND_QUALITIES: [Quality; 12] = [
    Quality::Major,
    Quality::Minor,
    Quality::Dominant7,
    Quality::Major7,
    Quality::Minor7,
    Quality::Sus4,
    Quality::Major6,
    Quality::Minor6,
    Quality::Diminished,
    Quality::Diminished7,
    Quality::HalfDiminished7,
    Quality::Power,
];

/// The kinds of phrase a part can play (the Style row run on into
/// phrases, P2): a bass line for Bass, chords for Chords, a riff or an
/// arp line for a second line. Drums take none: a phrase plays notes.
pub fn phrase_kinds(part: PartRole) -> &'static [PhraseKind] {
    match part {
        PartRole::Bass => &[PhraseKind::Bass],
        PartRole::Chords => &[PhraseKind::Chords],
        PartRole::Melody => &[PhraseKind::Riff, PhraseKind::Line],
        _ => &[],
    }
}

/// The panel's rows of ‹ › steppers, top to bottom. Sound is a stepper
/// only for New track; on a track it reads what the track plays.
pub const PANEL_ROWS: [&str; 5] = ["Role", "Style", "Bars", "Track", "Sound"];
pub const ROW_ROLE: usize = 0;
pub const ROW_STYLE: usize = 1;
pub const ROW_BARS: usize = 2;
pub const ROW_TRACK: usize = 3;
pub const ROW_SOUND: usize = 4;

/// The Track row's choice past the last track with room: a track Keep
/// adds, with a synth made for the part (S2.6b).
pub const NEW_TRACK: &str = "New track";

/// The synths New track offers a bass part, the first the one it starts
/// on: (node id, name), as mpfree-app's `synth::Engine` names them.
pub const BASS_SOUNDS: [(&str, &str); 6] = [
    ("mpfree.bass", "Bass"),
    ("mpfree.squelch", "Squelch"),
    ("mpfree.analog", "Analog"),
    ("mpfree.poly", "Poly"),
    ("mpfree.fm", "FM"),
    ("mpfree.pluck", "Pluck"),
];
/// ... a chords part.
pub const CHORDS_SOUNDS: [(&str, &str); 7] = [
    ("mpfree.poly", "Poly"),
    ("mpfree.keys", "Keys"),
    ("mpfree.strings", "Strings"),
    ("mpfree.drawbar", "Drawbar"),
    ("mpfree.choir", "Choir"),
    ("mpfree.analog", "Analog"),
    ("mpfree.fm", "FM"),
];
/// ... a tune.
pub const MELODY_SOUNDS: [(&str, &str); 8] = [
    ("mpfree.fm", "FM"),
    ("mpfree.pluck", "Pluck"),
    ("mpfree.analog", "Analog"),
    ("mpfree.wind", "Wind"),
    ("mpfree.poly", "Poly"),
    ("mpfree.keys", "Keys"),
    ("mpfree.strings", "Strings"),
    ("mpfree.squelch", "Squelch"),
];
/// ... drums: the Drums synth's kit.
pub const DRUMS_SOUNDS: [(&str, &str); 1] = [("mpfree.drums", "Drums")];

/// The synths New track offers `part`, the first the one it starts on.
pub fn sounds_for(part: PartRole) -> &'static [(&'static str, &'static str)] {
    match part {
        PartRole::Bass => &BASS_SOUNDS,
        PartRole::Chords => &CHORDS_SOUNDS,
        PartRole::Melody => &MELODY_SOUNDS,
        PartRole::Drums => &DRUMS_SOUNDS,
    }
}

/// What plays a part on New track's synth `id`: a bass synth plays bass,
/// the Drums synth drums, else the part's own kind.
pub fn kind_on(part: PartRole, id: &str) -> InstrumentKind {
    match id {
        "mpfree.bass" | "mpfree.squelch" => InstrumentKind::Bass,
        "mpfree.drums" => InstrumentKind::Percussive,
        _ => kind_of_part(part),
    }
}

/// The name New track gives the part's track: the part's word ("Bass"),
/// or with the first number free when a track has it ("Bass 2").
pub fn new_track_name(st: &TimeState, part: PartRole) -> String {
    let word = part_word(part);
    let taken = |n: &str| st.tracks.iter().any(|t| t.name.eq_ignore_ascii_case(n));
    if !taken(word) {
        return word.to_string();
    }
    (2..)
        .map(|k| format!("{word} {k}"))
        .find(|n| !taken(n))
        .expect("a free number")
}

/// Whether the project has room for New track.
fn room_for_a_track(st: &TimeState) -> bool {
    st.tracks.len() < mpfree_seq::MAX_TRACKS
}

/// The panel's buttons: `Toggle(k)`.
pub const PANEL_CLOSE: usize = 0;
pub const PANEL_TRY_AGAIN: usize = 1;
pub const PANEL_STEP: usize = 2;
pub const PANEL_BACK: usize = 3;
pub const PANEL_KEEP: usize = 4;

/// The Chords tab's Fit to the new chords: `Toggle(CARD_FIT)`.
pub const CARD_FIT: usize = 0;

/// The tab the Chords tab is of the Record panel's tabs.
pub const CHORDS_TAB: usize = 2;

/// A part's name, as its tile says it.
pub fn part_word(part: PartRole) -> &'static str {
    match part {
        PartRole::Bass => "Bass",
        PartRole::Chords => "Chords",
        PartRole::Melody => "Melody",
        PartRole::Drums => "Drums",
    }
}

/// The words under Role: how the part plays with the others.
pub fn relation_words(part: PartRole, relation: Relation) -> &'static str {
    match (part, relation) {
        (PartRole::Drums, Relation::Support) => "Keeps time under the others",
        (PartRole::Drums, Relation::Answer) => "Fills where the others rest",
        (PartRole::Drums, Relation::Interlock) => "Hits between the others' notes",
        (PartRole::Drums, Relation::StayOut) => "Little, out of the way",
        (_, Relation::Support) => "With the beat, under the tune",
        (_, Relation::Answer) => "Plays where the tune rests",
        (_, Relation::Interlock) => "Locks into the beat's gaps",
        (_, Relation::StayOut) => "Sparse, out of the tune's way",
    }
}

/// What plays a part when the screen doesn't know the track's
/// instrument.
pub fn kind_of_part(part: PartRole) -> InstrumentKind {
    match part {
        PartRole::Bass => InstrumentKind::Bass,
        PartRole::Chords => InstrumentKind::Chords,
        PartRole::Melody => InstrumentKind::OneNote,
        PartRole::Drums => InstrumentKind::Percussive,
    }
}

/// The tracks whose cell in row `row` can take a part: note tracks (not
/// Loop or audio tracks) with an empty cell there, or a clip with nothing
/// in it ([`has_room`]).
pub fn part_tracks(st: &TimeState, row: usize) -> Vec<usize> {
    st.tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.audio)
        .filter(|(t, _)| has_room(st, *t, row))
        .map(|(t, _)| t)
        .collect()
}

/// Cell `row` of `track` can take a part: it is empty, or its clip has
/// no notes (the Sequencer's empty clip makes way at Keep, S2.6; a clip
/// of only locks or moves the time engine refuses, saying so).
pub fn has_room(st: &TimeState, track: usize, row: usize) -> bool {
    st.clip(track, row).is_none_or(|c| c.notes == 0)
}

/// The panel's choices: the screen's own, kept while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct AddPick {
    pub row: usize,
    pub track: usize,
    pub part: PartRole,
    pub relation: Relation,
    /// An index into [`STYLES`], then the styles made from clips
    /// ([`mpfree_compose::style_at`]).
    pub style: usize,
    /// The styles made from clips (composing step 6), the song's as the
    /// panel opened: the Style row runs on into them after the built-in
    /// ones, before the phrases.
    pub made: Arc<[ClipStyle]>,
    /// One of [`PART_BARS`].
    pub bars: Option<u32>,
    /// The Q-Links: Similarity, Variation and Activity in %, Range 0 to 2
    /// (as Continue's).
    pub q: [i32; 4],
    /// New track (S2.6b): the part goes on a track Keep adds; `track` is
    /// the index it will get (the project's track count), `new_name` its
    /// name, and `sound` an index into [`sounds_for`] the part.
    pub new_track: bool,
    pub new_name: String,
    pub sound: usize,
    /// While the app makes New track's synth before a try is asked, or
    /// when it didn't come: what the status says ("Making the Bass
    /// synth…"), and whether it warns. The app sets it
    /// ([`crate::view::TimePage::set_add_waiting`]).
    pub waiting: Option<(String, bool)>,
    /// The phrases the Style row runs on into after its styles (P2): the
    /// app's list, MPFree's then the owner's
    /// ([`crate::view::TimePage::phrases`]).
    pub phrases: Arc<[(String, Phrase)]>,
    /// The phrase picked on the Style row, an index into `phrases`: the
    /// part plays it over the row's chords in place of one the composer
    /// writes. None: the style.
    pub phrase: Option<usize>,
}

impl AddPick {
    /// The panel opening on row `row`: on `track` if its cell there is
    /// empty, else the first track whose name says the part (a "Bass"
    /// track for Bass), else the first empty one. Why not, when no track
    /// has an empty cell in the row.
    pub fn open(st: &TimeState, row: usize, track: Option<usize>) -> Result<AddPick, String> {
        let tracks = part_tracks(st, row);
        if tracks.is_empty() && !room_for_a_track(st) {
            return Err(format!(
                "Every track has a clip in row {}, and the project holds no more tracks: add a part to another row",
                row + 1
            ));
        }
        let part = PartRole::Bass;
        let mut pick = AddPick {
            row,
            track: tracks.first().copied().unwrap_or(st.tracks.len()),
            part,
            relation: Relation::Support,
            style: STYLES
                .iter()
                .position(|s| s.name == Settings::default().style.name)
                .unwrap_or(0),
            made: Arc::clone(&st.styles),
            bars: None,
            q: opening_q(),
            new_track: false,
            new_name: String::new(),
            sound: 0,
            waiting: None,
            phrases: Arc::default(),
            phrase: None,
        };
        // No track has room in the row: New track.
        if tracks.is_empty() {
            pick.pick_new_track(st);
            return Ok(pick);
        }
        pick.track = match track.filter(|t| tracks.contains(t)) {
            Some(t) => t,
            None => pick.track_for(st, &tracks),
        };
        // An empty clip there: the part as long as it is, when that is
        // one of the lengths Bars steps.
        let bar = st.transport.meter.bar_ticks().max(1);
        if let Some(c) = st.clip(pick.track, row) {
            let len = c.speed.to_song(c.length as f64) as Tick;
            let bars = (len % bar == 0).then(|| (len / bar) as u32);
            if PART_BARS.contains(&bars) && bars.is_some() {
                pick.bars = bars;
            }
        }
        Ok(pick)
    }

    /// The track a part goes on by default: one whose name says it.
    fn track_for(&self, st: &TimeState, tracks: &[usize]) -> usize {
        let word = part_word(self.part).to_lowercase();
        tracks
            .iter()
            .copied()
            .find(|&t| st.tracks[t].name.to_lowercase().contains(&word))
            .unwrap_or(tracks[0])
    }

    /// The part `k` of [`PARTS`] picked: the track follows it while it was
    /// the default. On New track the sound starts on the part's first
    /// synth, and the track takes the part's name.
    pub fn set_part(&mut self, st: &TimeState, k: usize) -> bool {
        let Some(&part) = PARTS.get(k) else {
            return false;
        };
        if part == self.part {
            return false;
        }
        // A phrase of another kind than the new part takes: the style
        // again.
        if self
            .phrase_of()
            .is_some_and(|(_, p)| !phrase_kinds(part).contains(&p.kind))
        {
            self.phrase = None;
        }
        if self.new_track {
            self.part = part;
            self.pick_new_track(st);
            return true;
        }
        let tracks = part_tracks(st, self.row);
        let was_default = !tracks.is_empty() && self.track == self.track_for(st, &tracks);
        self.part = part;
        if was_default {
            self.track = self.track_for(st, &tracks);
        }
        true
    }

    /// The Track row on New track: the index the track will get, its
    /// name, the part's first synth.
    fn pick_new_track(&mut self, st: &TimeState) {
        self.new_track = true;
        self.track = st.tracks.len();
        self.new_name = new_track_name(st, self.part);
        self.sound = 0;
    }

    /// The tracks changed while the panel is open (a + Track, UNDO): New
    /// track's index and name follow them, so the next ask is for the
    /// track Keep will add. True if they moved.
    pub fn follow_tracks(&mut self, st: &TimeState) -> bool {
        if !self.new_track || self.track == st.tracks.len() {
            return false;
        }
        self.track = st.tracks.len();
        self.new_name = new_track_name(st, self.part);
        true
    }

    /// New track's synth: (node id, name).
    pub fn sound_of(&self) -> (&'static str, &'static str) {
        let list = sounds_for(self.part);
        list[self.sound.min(list.len() - 1)]
    }

    /// Row `row` of [`PANEL_ROWS`] stepped `delta`: true when it changed,
    /// or why not at an end.
    pub fn step(&mut self, st: &TimeState, row: usize, delta: i32) -> Result<bool, String> {
        let by = |i: usize, n: usize| (i as i64 + i64::from(delta)).clamp(0, n as i64 - 1) as usize;
        match row {
            ROW_ROLE => {
                let at = Relation::ALL
                    .iter()
                    .position(|&r| r == self.relation)
                    .unwrap_or(0);
                let to = Relation::ALL[by(at, Relation::ALL.len())];
                let changed = to != self.relation;
                self.relation = to;
                Ok(changed)
            }
            ROW_STYLE => {
                // The styles (the built-in ones, then those made from
                // clips), then the phrases of the part's kind.
                let phrases = self.part_phrases();
                let styles = style_count(&self.made);
                let at = match self.phrase {
                    Some(i) => styles + phrases.iter().position(|&p| p == i).unwrap_or(0),
                    None => self.style.min(styles - 1),
                };
                let to = by(at, styles + phrases.len());
                if to == at && delta > 0 && phrases.is_empty() {
                    return Err(match self.part {
                        PartRole::Drums => "Drums take no phrase: a phrase plays notes".to_string(),
                        _ => format!(
                            "{} is the last style, and there's no phrase of this kind yet",
                            style_name(styles - 1, &self.made)
                        ),
                    });
                }
                if to < styles {
                    self.style = to;
                    self.phrase = None;
                } else {
                    self.phrase = Some(phrases[to - styles]);
                }
                Ok(to != at)
            }
            ROW_BARS => {
                let at = PART_BARS.iter().position(|&b| b == self.bars).unwrap_or(0);
                let to = PART_BARS[by(at, PART_BARS.len())];
                let changed = to != self.bars;
                self.bars = to;
                Ok(changed)
            }
            ROW_TRACK => {
                // The tracks with room, then New track while the project
                // holds more.
                let tracks = part_tracks(st, self.row);
                let new = room_for_a_track(st);
                let n = tracks.len() + usize::from(new);
                let at = if self.new_track {
                    new.then_some(tracks.len())
                } else {
                    tracks.iter().position(|&t| t == self.track)
                };
                let Some(at) = at else {
                    return Err(if self.new_track {
                        "The project holds no more tracks: pick a track with room".to_string()
                    } else {
                        format!(
                            "{}'s cell in row {} filled: pick another track",
                            self.track_name(st),
                            self.row + 1
                        )
                    });
                };
                let to = by(at, n);
                if to == at {
                    return Err(match n {
                        1 if self.new_track => format!(
                            "Every track has a clip in row {}: the part goes on a new track",
                            self.row + 1
                        ),
                        1 => format!(
                            "{} is the only track with an empty cell in row {}, and the project holds no more tracks",
                            self.track_name(st),
                            self.row + 1
                        ),
                        _ if delta < 0 => "That's the first track with room".to_string(),
                        _ if self.new_track => "New track is the last choice".to_string(),
                        _ => "That's the last track with room".to_string(),
                    });
                }
                if to == tracks.len() {
                    self.pick_new_track(st);
                } else {
                    self.new_track = false;
                    self.track = tracks[to];
                }
                Ok(true)
            }
            ROW_SOUND => {
                if !self.new_track {
                    return Err(format!(
                        "The part plays on {}'s own sound: pick New track on the Track row to choose a synth",
                        self.track_name(st)
                    ));
                }
                let list = sounds_for(self.part);
                if list.len() == 1 {
                    return Err(format!(
                        "{} play on the {} synth",
                        part_word(self.part),
                        list[0].1
                    ));
                }
                let to = by(self.sound, list.len());
                let changed = to != self.sound;
                self.sound = to;
                Ok(changed)
            }
            _ => Ok(false),
        }
    }

    fn track_name(&self, st: &TimeState) -> String {
        if self.new_track {
            return NEW_TRACK.to_string();
        }
        st.tracks
            .get(self.track)
            .map_or_else(|| format!("Track {}", self.track + 1), |t| t.name.clone())
    }

    /// What the part plays on, as the Sound row reads it: New track's
    /// synth, else the track's instrument.
    fn sound_words(&self, st: &TimeState) -> String {
        if self.new_track {
            return self.sound_of().1.to_string();
        }
        match st.tracks.get(self.track).map(|t| t.sound.as_str()) {
            Some("") | None => "nothing on the track yet".to_string(),
            Some(s) => format!("the track's {s}"),
        }
    }

    /// Row `row`'s value as the panel shows it.
    pub fn value(&self, st: &TimeState, row: usize) -> String {
        match row {
            ROW_ROLE => self.relation.words().to_string(),
            ROW_STYLE => match self.phrase_of() {
                Some((name, _)) => name,
                None => style_name(self.style, &self.made).to_string(),
            },
            ROW_BARS => match self.bars {
                None => format!("{} · the row", row_bars(st, self.row)),
                Some(b) => format!("{b}"),
            },
            ROW_TRACK => self.track_name(st),
            ROW_SOUND => self.sound_words(st),
            _ => String::new(),
        }
    }

    /// The settings an ask carries.
    pub fn settings(&self) -> Settings {
        let pct = |v: i32| v.clamp(0, 100) as f32 / 100.0;
        Settings {
            style: style_at(self.style, &self.made),
            similarity: pct(self.q[0]),
            variation: pct(self.q[1]),
            activity: pct(self.q[2]),
            range: RANGES[self.q[3].clamp(0, 2) as usize],
            ..Settings::default()
        }
    }

    /// The ask for a part with these choices. What plays it is the part's
    /// own kind until the app, which knows the track's instrument and its
    /// kit's drums, fills it in; on New track, what its synth plays.
    pub fn ask(&self) -> PartAsk {
        let new_track = self.new_track.then(|| NewTrack {
            name: self.new_name.clone(),
            sound: self.sound_of().0,
        });
        PartAsk {
            row: self.row,
            track: self.track,
            part: self.part,
            relation: self.relation,
            settings: self.settings(),
            kind: match &new_track {
                Some(n) => kind_on(self.part, n.sound),
                None => kind_of_part(self.part),
            },
            lanes: Lanes::default(),
            pads: [None; 4],
            bars: self.bars,
            new_track,
            phrase: self.phrase_of(),
        }
    }

    /// The phrases of the part's kind, as places in `phrases`, in the
    /// list's order: MPFree's, then the owner's.
    pub fn part_phrases(&self) -> Vec<usize> {
        let kinds = phrase_kinds(self.part);
        self.phrases
            .iter()
            .enumerate()
            .filter(|(_, (_, p))| kinds.contains(&p.kind))
            .map(|(i, _)| i)
            .collect()
    }

    /// The phrase picked on the Style row, and its name.
    pub fn phrase_of(&self) -> Option<(String, Phrase)> {
        let (name, p) = self.phrases.get(self.phrase?)?;
        Some((name.clone(), *p))
    }

    /// The app's list of phrases changed (one kept, a project opened):
    /// the pick follows its phrase by name, or goes back to the style.
    pub fn set_phrases(&mut self, phrases: Arc<[(String, Phrase)]>) -> bool {
        if Arc::ptr_eq(&self.phrases, &phrases) || self.phrases == phrases {
            return false;
        }
        let was = self.phrase_of();
        self.phrases = phrases;
        self.phrase =
            was.and_then(|(name, p)| self.phrases.iter().position(|(n, q)| *n == name && *q == p));
        true
    }

    /// Q-Link `k` turned `ticks` detents: true when it changed.
    pub fn turn(&mut self, k: usize, ticks: i32) -> bool {
        let Some(v) = self.q.get_mut(k) else {
            return false;
        };
        let was = *v;
        *v = if k == 3 {
            (*v + ticks.signum()).clamp(0, 2)
        } else {
            (*v + ticks).clamp(0, 100)
        };
        *v != was
    }

    /// The Q-Links' cards: names and values; "–" while a phrase is
    /// picked (it plays as written, so they shape nothing).
    pub fn cards(&self) -> [(&'static str, String); 4] {
        let mut c = cards(self.q);
        if self.phrase.is_some() {
            for (_, v) in &mut c {
                *v = "–".to_string();
            }
        }
        c
    }

    /// Where each Q-Link stands, 0 to 1.
    pub fn knob_pos(&self) -> [f32; 4] {
        let pct = |v: i32| v.clamp(0, 100) as f32 / 100.0;
        [
            pct(self.q[0]),
            pct(self.q[1]),
            pct(self.q[2]),
            self.q[3].clamp(0, 2) as f32 / 2.0,
        ]
    }
}

/// Row `row`'s length in whole bars, as the screens show it (1 at least).
pub fn row_bars(st: &TimeState, row: usize) -> u32 {
    let bar = st.transport.meter.bar_ticks().max(1);
    let len = st
        .tracks
        .iter()
        .filter_map(|t| t.slots.get(row)?.as_ref())
        .map(|c| c.speed.to_song(c.length as f64) as Tick)
        .max()
        .unwrap_or(bar);
    ((len + bar - 1) / bar).max(1) as u32
}

/// The row's chords the screens show: the ones picked on the panel and
/// not kept yet, else the row's as heard (`TimeState::row_chords`, else
/// the Song view's `TimeState::song_chords`).
pub fn shown_line(st: &TimeState, row: usize) -> Option<ChordTimeline> {
    if let Some(a) = st.adding.as_ref().filter(|a| a.row == row) {
        if let Some(p) = &a.picked {
            return Some(p.clone());
        }
    }
    st.row_chords
        .as_ref()
        .filter(|(r, _)| *r == row)
        .or_else(|| st.song_chords.iter().find(|(r, _)| *r == row))
        .map(|(_, c)| c.line.clone())
}

/// The row's chords under it, for the grid: a chip per chord, the play
/// line where the row plays, the chip the picker is open on. None until
/// the row has been heard.
pub fn strip_of(st: &TimeState, row: usize, open: Option<usize>) -> Option<ChordStrip> {
    let line = shown_line(st, row)?;
    let bar = st.transport.meter.bar_ticks().max(1);
    let bars = if line.is_empty() {
        row_bars(st, row)
    } else {
        ((line.len() + bar - 1) / bar).max(1) as u32
    };
    let len = if line.is_empty() {
        Tick::from(bars) * bar
    } else {
        line.len()
    } as f32;
    let mut said = [false; 3];
    let chips = line
        .spans()
        .iter()
        .map(|s| {
            let look = match s.heard {
                Heard::Set => ChipLook::Yours,
                Heard::Suggested => ChipLook::Suggested,
                Heard::Heard if s.answer.chord().is_none() => ChipLook::Suggested,
                Heard::Heard => ChipLook::Heard,
            };
            // The first chip of each look says what it is.
            let k = look as usize;
            let word = if said[k] {
                ""
            } else {
                said[k] = true;
                match look {
                    ChipLook::Heard => "heard",
                    ChipLook::Suggested if s.answer.chord().is_some() => "suggested",
                    ChipLook::Suggested => "",
                    ChipLook::Yours => "yours",
                }
            };
            StripChip {
                from: s.start as f32 / len,
                to: s.end() as f32 / len,
                name: match s.answer {
                    ChordAnswer::Chord(c) => c.name(),
                    ChordAnswer::NoClearChord => "–".to_string(),
                },
                look,
                word: word.to_string(),
            }
        })
        .collect();
    Some(ChordStrip {
        row,
        bars,
        chips,
        play: row_play(st, row, len as Tick),
        open,
    })
}

/// Where row `row` plays, 0 to 1 of `len`, while a clip of it plays.
fn row_play(st: &TimeState, row: usize, len: Tick) -> Option<f32> {
    let t = &st.transport;
    let l = st
        .launches
        .iter()
        .find(|l| l.slot == row && l.origin <= t.position)?;
    let len = len.max(1);
    Some(((t.position - l.origin).rem_euclid(len)) as f32 / len as f32)
}

/// The part being tried in cell `slot` of `track`, as the grid draws it:
/// dashed in the track's colour, "try N", the ring while it plays.
pub fn try_cell(st: &TimeState, track: usize, slot: usize) -> Option<GridCell> {
    let a = st.adding.as_ref()?;
    if (a.track, a.row) != (track, slot) || !has_room(st, track, slot) {
        return None;
    }
    let name = a
        .made
        .as_ref()
        .map_or_else(|| part_word(a.part).to_string(), |m| m.clip.name.clone());
    let n = a.position.map_or(1, |p| p.0);
    let mut cell = GridCell {
        state: CellState::Stopped,
        name,
        detail: format!("try {n}"),
        mark: CellMark::Try,
        ..GridCell::default()
    };
    match a.state {
        AddState::Making if a.made.is_none() => cell.detail = "making it".to_string(),
        AddState::Nothing => return None,
        AddState::Playing => {
            cell.state = CellState::Playing;
            let len = a.made.as_ref().map_or(1, |m| m.clip.length.max(1));
            cell.progress = row_play(st, slot, len).map_or(0.0, |f| (f * 64.0).floor() / 64.0);
        }
        AddState::Ready | AddState::Making => {
            if let Some(at) = a.line {
                cell.state = CellState::Queued;
                cell.foot = format!("from {}", crate::view::bar_beat(at, st.transport.meter));
            }
        }
    }
    Some(cell)
}

/// Clip `slot` of `track` is kept with notes off its row's chords.
pub fn unfit(st: &TimeState, track: usize, slot: usize) -> bool {
    st.unfit
        .iter()
        .any(|u| u.row == slot && u.clips.iter().any(|c| c.0 == track))
}

/// What the panel shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddPanelView {
    pub row: usize,
    /// What it hears in the row: a dot in the track's colour, its name,
    /// what it is.
    pub heard: Vec<(Color, String, String)>,
    /// The part's tile lit: an index into [`PARTS`].
    pub part: usize,
    /// [`PANEL_ROWS`]' values.
    pub values: [String; 5],
    pub role_words: String,
    pub sound: String,
    /// New track: Sound is a stepper of the part's synths.
    pub new_track: bool,
    pub status: String,
    pub status_warn: bool,
    /// The try stepper: "‹ Try 1" or "Try 3 ›", and whether there is one.
    pub step: (String, bool),
    pub can_keep: bool,
    /// The Style row is on a phrase: it reads Phrase, and Try again is
    /// faint (a phrase plays one way).
    pub phrase: bool,
}

/// The panel for `pick` and the time engine's state.
pub fn panel_view(st: &TimeState, pick: &AddPick) -> AddPanelView {
    let bar = st.transport.meter.bar_ticks().max(1);
    let heard = st
        .tracks
        .iter()
        .enumerate()
        .filter(|(t, tr)| *t != pick.track && !tr.silent && !tr.audio)
        .filter_map(|(t, tr)| {
            let c = st.clip(t, pick.row)?;
            if c.notes == 0 {
                return None;
            }
            let bars = (c.speed.to_song(c.length as f64) as Tick + bar - 1) / bar;
            let what = match c.holds {
                Holds::Hits => "drums",
                Holds::Notes => "notes",
            };
            let len = if bars == 1 {
                "1 bar".to_string()
            } else {
                format!("{bars} bars")
            };
            Some((track_color(t), tr.name.clone(), format!("{what}, {len}")))
        })
        .collect();
    let a = st
        .adding
        .as_ref()
        .filter(|a| (a.track, a.row) == (pick.track, pick.row));
    let (status, status_warn) = match a {
        _ if pick.waiting.is_some() => pick.waiting.clone().unwrap_or_default(),
        None => ("Try again makes a part".to_string(), false),
        Some(a) => {
            let n = a.position.map_or(1, |p| p.0);
            match a.state {
                AddState::Nothing => (a.words.clone().unwrap_or_default(), true),
                AddState::Making if a.made.is_none() => ("Making a part…".to_string(), false),
                AddState::Making => (format!("Try {n} plays; making the next…"), false),
                AddState::Ready => match a.line {
                    Some(at) => (
                        format!(
                            "Try {n} · from {}",
                            crate::view::bar_beat(at, st.transport.meter)
                        ),
                        false,
                    ),
                    None => (format!("Try {n} · plays once the row plays"), false),
                },
                AddState::Playing if pick.phrase.is_some() => (
                    format!(
                        "Try {n} · {}, with the row",
                        pick.phrase_of().map(|(name, _)| name).unwrap_or_default()
                    ),
                    false,
                ),
                AddState::Playing if pick.new_track => {
                    (format!("Try {n} · plays on the new track"), false)
                }
                AddState::Playing => (format!("Try {n} · plays in its cell with the row"), false),
            }
        }
    };
    let step = match a.and_then(|a| a.position) {
        Some((i, _)) if i > 1 => (format!("‹ Try {}", i - 1), true),
        Some((i, n)) if i < n => (format!("Try {} ›", i + 1), true),
        _ => ("‹ Try".to_string(), false),
    };
    let sound = pick.sound_words(st);
    AddPanelView {
        row: pick.row,
        heard,
        part: PARTS.iter().position(|&p| p == pick.part).unwrap_or(0),
        values: std::array::from_fn(|r| pick.value(st, r)),
        role_words: relation_words(pick.part, pick.relation).to_string(),
        sound,
        new_track: pick.new_track,
        status,
        status_warn,
        step,
        can_keep: a.is_some_and(|a| a.made.is_some() && a.state != AddState::Nothing),
        phrase: pick.phrase.is_some(),
    }
}

/// The chord picker's state: the chip it is open on and what is lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordPick {
    pub row: usize,
    pub chip: usize,
    /// Indexes into [`crate::chord_strip::ROOTS`] and [`KIND_QUALITIES`].
    pub root: Option<usize>,
    pub kind: Option<usize>,
    /// Opened on a bar of a row with no chords yet: the line to pick on
    /// is a bar of no chord for each bar of the row.
    pub fresh: bool,
}

impl ChordPick {
    /// Open on chip `chip` of `line`, its chord lit.
    pub fn open(row: usize, line: &ChordTimeline, chip: usize) -> Option<ChordPick> {
        let s = line.spans().get(chip)?;
        let (root, kind) = match s.answer.chord() {
            Some(c) => (
                Some(usize::from(c.root)),
                KIND_QUALITIES.iter().position(|&q| q == c.quality),
            ),
            None => (None, None),
        };
        Some(ChordPick {
            row,
            chip,
            root,
            kind,
            fresh: false,
        })
    }

    /// The chord lit, once a root is: a major chord until a kind is.
    pub fn chord(&self) -> Option<Chord> {
        let root = self.root?;
        let q = KIND_QUALITIES[self.kind.unwrap_or(0).min(11)];
        Some(Chord::new(root as u8, q))
    }

    /// The picker as it shows on `line`, with the row's chords as heard.
    pub fn view(
        &self,
        line: &ChordTimeline,
        heard: Option<&ChordTimeline>,
        bar: Tick,
    ) -> PickerView {
        let span = line.spans().get(self.chip);
        let heard_here = span.and_then(|s| heard?.span_at(s.start)).map(|h| h.answer);
        let picked = self.chord();
        let now = span.map(|s| s.answer);
        PickerView {
            chip: self.chip,
            bar: span.map_or(1, |s| (s.start / bar.max(1)) as u32 + 1),
            heard: heard_here
                .and_then(ChordAnswer::chord)
                .map_or_else(String::new, Chord::name),
            picked: picked.map_or_else(String::new, Chord::name),
            root: self.root,
            kind: picked.map(|_| self.kind.unwrap_or(0)),
            can_split: span.is_some_and(|s| split_at(s, bar).is_some()),
            can_join: self.chip + 1 < line.spans().len(),
            can_as_heard: span.is_some_and(|s| s.heard == Heard::Set) && heard_here.is_some(),
            can_use: picked.is_some_and(|c| now != Some(ChordAnswer::Chord(c))),
        }
    }
}

/// Where Split cuts a span: on the bar line nearest its middle, or on a
/// beat when it is a bar or less; None for a span of one beat.
fn split_at(s: &ChordSpan, bar: Tick) -> Option<Tick> {
    let beat = (bar / 4).max(1);
    let unit = if s.len > bar { bar } else { beat };
    let k = ((s.len / 2 + unit / 2) / unit).max(1);
    let at = s.start + k * unit;
    (at > s.start && at < s.end()).then_some(at)
}

/// The line with chip `chip` set to `chord`, as yours.
pub fn use_chord(line: &ChordTimeline, chip: usize, chord: Chord) -> Option<ChordTimeline> {
    let s = *line.spans().get(chip)?;
    let mut out = line.clone();
    out.set_at(s.start, ChordAnswer::Chord(chord));
    Some(out)
}

/// The line with chip `chip` cut in two, both halves its chord, yours.
pub fn split(line: &ChordTimeline, chip: usize, bar: Tick) -> Option<ChordTimeline> {
    let s = *line.spans().get(chip)?;
    let at = split_at(&s, bar)?;
    let mut spans = line.spans().to_vec();
    let first = ChordSpan::new(s.start, at - s.start, s.answer, Heard::Set);
    let second = ChordSpan::new(at, s.end() - at, s.answer, Heard::Set);
    spans.splice(chip..=chip, [first, second]);
    ChordTimeline::new(spans).ok()
}

/// The line with chip `chip` and the next as one, its chord, yours.
pub fn join_next(line: &ChordTimeline, chip: usize) -> Option<ChordTimeline> {
    let s = *line.spans().get(chip)?;
    let n = *line.spans().get(chip + 1)?;
    let mut spans = line.spans().to_vec();
    let joined = ChordSpan::new(s.start, s.len + n.len, s.answer, Heard::Set);
    spans.splice(chip..=chip + 1, [joined]);
    ChordTimeline::new(spans).ok()
}

/// The line with chip `chip` given back the chord the row's notes give
/// there.
pub fn as_heard(line: &ChordTimeline, chip: usize, heard: &ChordTimeline) -> Option<ChordTimeline> {
    let s = *line.spans().get(chip)?;
    let h = heard.span_at(s.start)?;
    let mut spans = line.spans().to_vec();
    spans[chip] = ChordSpan::new(s.start, s.len, h.answer, Heard::Heard);
    ChordTimeline::new(spans).ok()
}

/// A line of `bars` bars with no chord in any: what a tap on a bar of a
/// row with no chords yet picks on.
pub fn bare_line(bars: u32, bar: Tick) -> ChordTimeline {
    let spans = (0..i64::from(bars.max(1)))
        .map(|b| ChordSpan::new(b * bar, bar, ChordAnswer::NoClearChord, Heard::Heard))
        .collect();
    ChordTimeline::new(spans).unwrap_or_default()
}

/// What a picked line asks of the time engine: with Add a part open on
/// the row, the panel's chords (kept with the part); else a chord set on
/// the row, one take.
pub fn line_action(st: &TimeState, row: usize, line: ChordTimeline) -> TimeAction {
    if st.adding.as_ref().is_some_and(|a| a.row == row) {
        return TimeAction::Add(AddAction::Line(line));
    }
    // The line as shown: the strip's row, or the Song view's Chords lane.
    let shown = st
        .row_chords
        .as_ref()
        .filter(|(r, _)| *r == row)
        .or_else(|| st.song_chords.iter().find(|(r, _)| *r == row))
        .map(|(_, c)| c.line.clone())
        .unwrap_or_default();
    TimeAction::Chords(ChordsAction::Set { row, line, shown })
}

/// What the Chords tab shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChordsCardView {
    /// "5 · Break".
    pub row: String,
    /// What changed: "Bar 4: Ab7, yours (heard G7)".
    pub changed: Vec<String>,
    /// "2 clips don't fit the new chords", or that every clip fits.
    pub verdict: String,
    pub warn: bool,
    /// Each clip off the chords: its track's colour, its name, how many
    /// notes Fit moves, ticked.
    pub clips: Vec<(Color, String, String, bool)>,
    pub can_fit: bool,
}

/// The Chords tab for the scene pick: what changed, and the clips off the
/// new chords (`untick`: the (row, track) left as they are).
pub fn chords_view(st: &TimeState, untick: &BTreeSet<(usize, usize)>) -> ChordsCardView {
    let row = st.scene_focus;
    let name = st.scenes.get(row).filter(|n| !n.is_empty());
    let row_words = match name {
        Some(n) => format!("{} · {n}", row + 1),
        None => format!("{}", row + 1),
    };
    let bar = st.transport.meter.bar_ticks().max(1);
    let u = st.unfit.iter().find(|u| u.row == row);
    let line = shown_line(st, row);
    let heard = st
        .row_chords
        .as_ref()
        .filter(|(r, _)| *r == row)
        .map(|(_, c)| &c.heard);
    let mut changed = Vec::new();
    if let (Some(u), Some(line)) = (u, &line) {
        for s in line.spans() {
            let before = u.before.span_at(s.start).map(|b| b.answer);
            if before == Some(s.answer) {
                continue;
            }
            let mut w = format!("Bar {}: {}", s.start / bar + 1, s.answer.name());
            if s.heard == Heard::Set {
                w.push_str(", yours");
            }
            if let Some(h) = heard
                .and_then(|h| h.span_at(s.start))
                .and_then(|h| h.answer.chord())
                .filter(|h| ChordAnswer::Chord(*h) != s.answer)
            {
                w.push_str(&format!(" (heard {})", h.name()));
            }
            changed.push(w);
        }
    }
    let clips: Vec<_> = u
        .map(|u| {
            u.clips
                .iter()
                .map(|&(t, n)| {
                    let name = st
                        .clip(t, row)
                        .map_or_else(|| format!("Track {}", t + 1), |c| c.name.clone());
                    let notes = match n {
                        1 => "1 note to move".to_string(),
                        n => format!("{n} notes to move"),
                    };
                    (track_color(t), name, notes, !untick.contains(&(row, t)))
                })
                .collect()
        })
        .unwrap_or_default();
    let (verdict, warn) = match clips.len() {
        0 if line.as_ref().is_none_or(ChordTimeline::is_empty) => (
            format!(
                "Row {} has no chords yet: tap a bar on the row to set one",
                row + 1
            ),
            false,
        ),
        0 => (
            format!(
                "Every clip fits row {}'s chords: tap one on the row to change it",
                row + 1
            ),
            false,
        ),
        1 => ("A clip doesn't fit the new chords".to_string(), true),
        n => (format!("{n} clips don't fit the new chords"), true),
    };
    let can_fit = clips.iter().any(|c| c.3);
    ChordsCardView {
        row: row_words,
        changed,
        verdict,
        warn,
        clips,
        can_fit,
    }
}

// ---------------------------------------------------------------- widgets

const PAD: i32 = 14;
const STEP_X: i32 = 88;
const STEP_W: i32 = 148;
const STEP_H: i32 = 38;
const TILE_H: i32 = 40;
const BUTTON_H: i32 = 42;
/// Heard lines shown; more are summed up on the last. New track's
/// Sound stepper takes the room of one.
const HEARD_LINES: usize = 4;
const HEARD_LINES_NEW: usize = 3;

/// The Add a part panel: the side area, from under the transport bar to
/// the foot, while it is open. A tap gives `Toggle(PANEL_*)`, a part's
/// tile `Select(k)`, a stepper's ‹ or › `Step { index: ROW_*, delta }`.
#[derive(Default)]
pub struct AddPanel {
    pub view: AddPanelView,
}

/// Where the panel's parts are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelGeo {
    pub close: Rect,
    pub heard_y: i32,
    pub tiles: [Rect; 4],
    /// The steppers' value boxes, [`PANEL_ROWS`]' order. Sound's is a
    /// stepper only on New track ([`AddPanelView::new_track`]); else the
    /// Sound line reads at `sound`.
    pub steppers: [Rect; 5],
    pub role_words: Rect,
    pub sound: Rect,
    pub status: Rect,
    /// Try again, the try stepper, Back, Keep.
    pub buttons: [Rect; 4],
}

impl AddPanel {
    pub fn new() -> AddPanel {
        AddPanel::default()
    }

    /// How many heard lines show.
    fn heard_lines(&self) -> usize {
        if self.view.new_track {
            HEARD_LINES_NEW
        } else {
            HEARD_LINES
        }
    }

    /// Row `k`'s name: the Style row reads Phrase while it is on one.
    fn row_name(&self, k: usize) -> &'static str {
        if k == ROW_STYLE && self.view.phrase {
            "Phrase"
        } else {
            PANEL_ROWS[k]
        }
    }

    /// The steppers that answer: Sound's only on New track.
    fn live_steppers(&self) -> usize {
        if self.view.new_track {
            PANEL_ROWS.len()
        } else {
            ROW_SOUND
        }
    }

    pub fn geo(&self, r: Rect) -> PanelGeo {
        let x = r.x + PAD;
        let w = r.w - 2 * PAD;
        let half = (w - 6) / 2;
        let lines = self.view.heard.len().clamp(1, self.heard_lines()) as i32;
        let heard_y = r.y + 68;
        let tiles_y = heard_y + lines * 21 + 10;
        let tiles = std::array::from_fn(|k| {
            let (c, row) = ((k % 2) as i32, (k / 2) as i32);
            Rect::new(
                x + c * (half + 6),
                tiles_y + row * (TILE_H + 6),
                half,
                TILE_H,
            )
        });
        let role_y = tiles_y + 2 * (TILE_H + 6) + 2;
        let sound_y = role_y + 204;
        let ys = [role_y, role_y + 66, role_y + 112, role_y + 158, sound_y];
        let step_x = (x + STEP_X).min(r.right() - PAD - STEP_W);
        let steppers = ys.map(|y| Rect::new(step_x, y, r.right() - PAD - step_x, STEP_H));
        let status_y = if self.view.new_track {
            sound_y + STEP_H + 4
        } else {
            sound_y + 28
        };
        let b1 = status_y + 24;
        let b2 = b1 + BUTTON_H + 8;
        PanelGeo {
            close: Rect::new(r.right() - PAD - 40, r.y + 6, 40, 32),
            heard_y,
            tiles,
            steppers,
            role_words: Rect::new(x, role_y + STEP_H + 2, w, 22),
            sound: Rect::new(x, sound_y, w, 24),
            status: Rect::new(x, status_y, w, 20),
            buttons: [
                Rect::new(x, b1, half, BUTTON_H),
                Rect::new(x + half + 6, b1, half, BUTTON_H),
                Rect::new(x, b2, half, BUTTON_H),
                Rect::new(x + half + 6, b2, half, BUTTON_H),
            ],
        }
    }
}

fn button(cx: &mut PaintCx, b: Rect, label: &str, lit: bool, live: bool) {
    let t = cx.theme.clone();
    let fill = if lit && live { t.accent } else { t.raised };
    cx.canvas.fill_round_rect(b, t.round(10), fill);
    let ink = match (lit, live) {
        (_, false) => t.text_faint,
        (true, true) => t.ink_on(t.accent),
        (false, true) => t.text,
    };
    cx.text(FontId::Bold, t.size_small, b, Align::Center, ink, label);
}

impl Widget for AddPanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let v = &self.view;
        let g = self.geo(r);
        cx.panel(r);
        let x = r.x + PAD;
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(x, r.y + 12, r.w - 2 * PAD - 44, 24),
            Align::Left,
            t.text,
            &format!("ADD A PART · ROW {}", v.row + 1),
        );
        cx.canvas.fill_round_rect(g.close, t.round(10), t.raised);
        cx.text(
            FontId::Sans,
            t.size_body + 2.0,
            g.close,
            Align::Center,
            t.text,
            "×",
        );
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            Rect::new(x, r.y + 46, r.w - 2 * PAD, 18),
            Align::Left,
            t.text_dim,
            "HEARD",
        );
        let line = |k: usize| Rect::new(x, g.heard_y + k as i32 * 21, r.w - 2 * PAD, 21);
        if v.heard.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                line(0),
                Align::Left,
                t.text_dim,
                "nothing in the row yet",
            );
        }
        let most = self.heard_lines();
        let shown = if v.heard.len() > most {
            most - 1
        } else {
            v.heard.len()
        };
        for (k, (color, name, what)) in v.heard.iter().take(shown).enumerate() {
            let l = line(k);
            cx.canvas
                .fill_round_rect(Rect::new(l.x + 2, l.y + 7, 8, 8), 4, *color);
            let name_r = Rect::new(l.x + 18, l.y, l.w - 18, l.h);
            let w = cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                name_r,
                Align::Left,
                t.text,
                name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(name_r.x + w, l.y, (name_r.w - w).max(0), l.h),
                Align::Left,
                t.text_dim,
                &format!(" · {what}"),
            );
        }
        if v.heard.len() > most {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                line(most - 1),
                Align::Left,
                t.text_dim,
                &format!("and {} more", v.heard.len() - shown),
            );
        }
        for (k, b) in g.tiles.iter().enumerate() {
            let lit = v.part == k;
            let fill = if lit { t.accent } else { t.raised };
            cx.canvas.fill_round_rect(*b, t.round(10), fill);
            let ink = if lit { t.ink_on(t.accent) } else { t.text };
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                *b,
                Align::Center,
                ink,
                part_word(PARTS[k]),
            );
        }
        for (k, s) in g.steppers.iter().enumerate().take(self.live_steppers()) {
            cx.text(
                FontId::Sans,
                t.size_body - 1.0,
                Rect::new(x, s.y, s.x - x, s.h),
                Align::Left,
                t.text,
                self.row_name(k),
            );
            cx.canvas.fill_round_rect(*s, t.round(10), t.raised);
            let ends = s.inset_xy(10, 0);
            cx.text(
                FontId::Bold,
                t.size_body,
                ends,
                Align::Left,
                t.text_dim,
                "‹",
            );
            cx.text(
                FontId::Bold,
                t.size_body,
                ends,
                Align::Right,
                t.text_dim,
                "›",
            );
            let ink = if k == ROW_ROLE || (k == ROW_STYLE && v.phrase) {
                t.readable(t.accent, t.raised)
            } else {
                t.text
            };
            cx.text(
                FontId::Bold,
                t.size_small,
                s.inset_xy(22, 0),
                Align::Center,
                ink,
                &v.values[k],
            );
        }
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            g.role_words,
            Align::Left,
            t.text_dim,
            &v.role_words,
        );
        if !v.new_track {
            cx.text(
                FontId::Sans,
                t.size_body - 1.0,
                g.sound,
                Align::Left,
                t.text,
                "Sound",
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                g.sound,
                Align::Right,
                t.text_dim,
                &v.sound,
            );
        }
        let ink = if v.status_warn {
            t.readable(t.warn, t.surface)
        } else {
            t.readable(t.good, t.surface)
        };
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            g.status,
            Align::Left,
            ink,
            &v.status,
        );
        button(cx, g.buttons[0], "Try again", false, !v.phrase);
        button(cx, g.buttons[1], &v.step.0, false, v.step.1);
        button(cx, g.buttons[2], "Back", false, true);
        button(cx, g.buttons[3], "Keep", true, v.can_keep);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geo(r);
        let mut out = vec![Target::tap("add-close", PANEL_CLOSE, "Close", g.close)];
        for (k, b) in g.tiles.iter().enumerate() {
            out.push(
                Target::tap("add-part-kind", k, part_word(PARTS[k]), *b)
                    .disabled_if(self.view.part == k),
            );
        }
        for (k, s) in g.steppers.iter().enumerate().take(self.live_steppers()) {
            let (less, more) = s.take_left(s.w / 2);
            out.push(Target::tap(
                "add-row-less",
                k,
                format!("{} ‹", self.row_name(k)),
                less,
            ));
            out.push(Target::tap(
                "add-row-more",
                k,
                format!("{} ›", self.row_name(k)),
                more,
            ));
        }
        let v = &self.view;
        out.push(
            Target::tap("add-try-again", PANEL_TRY_AGAIN, "Try again", g.buttons[0])
                .disabled_if(v.phrase),
        );
        out.push(
            Target::tap("add-step", PANEL_STEP, v.step.0.clone(), g.buttons[1])
                .disabled_if(!v.step.1),
        );
        out.push(Target::tap("add-back", PANEL_BACK, "Back", g.buttons[2]));
        out.push(
            Target::tap("add-keep", PANEL_KEEP, "Keep", g.buttons[3]).disabled_if(!v.can_keep),
        );
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let g = self.geo(r);
        if g.close.contains(x, y) {
            return Some(EventKind::Toggle(PANEL_CLOSE));
        }
        if let Some(k) = g.tiles.iter().position(|b| b.contains(x, y)) {
            return Some(EventKind::Select(k));
        }
        if let Some(k) = g
            .steppers
            .iter()
            .take(self.live_steppers())
            .position(|s| s.contains(x, y))
        {
            let s = g.steppers[k];
            let delta = if x < s.x + s.w / 2 { -1 } else { 1 };
            return Some(EventKind::Step { index: k, delta });
        }
        let v = &self.view;
        // Try again faint on a phrase still answers: the page says why.
        let live = [true, v.step.1, true, v.can_keep];
        let index = [PANEL_TRY_AGAIN, PANEL_STEP, PANEL_BACK, PANEL_KEEP];
        g.buttons
            .iter()
            .position(|b| b.contains(x, y))
            .filter(|&k| live[k])
            .map(|k| EventKind::Toggle(index[k]))
    }
}

/// The Record panel's Chords tab: the scene pick's row, what changed in
/// its chords, the kept clips off them (ticked: Fit moves them) and Fit
/// to the new chords. Its tabs are the Record panel's (`Tab(k)`); the
/// Row's ‹ › gives `Step { index: 0, delta }`, a clip's tick
/// `Select(k)`, Fit `Toggle(CARD_FIT)`.
pub struct ChordsCard {
    /// The title line and its tabs, drawn as the other tabs draw them.
    pub head: OptionList,
    pub view: ChordsCardView,
}

/// Where the Chords tab's parts are.
#[derive(Clone, Debug, PartialEq)]
pub struct CardGeo {
    pub row: Rect,
    pub changed_y: i32,
    pub verdict: Rect,
    pub clips: Vec<Rect>,
    pub fit: Option<Rect>,
    pub foot_y: i32,
}

impl ChordsCard {
    pub fn new() -> ChordsCard {
        let mut head = OptionList::new("Chords");
        head.empty = String::new();
        head.tab = CHORDS_TAB;
        ChordsCard {
            head,
            view: ChordsCardView::default(),
        }
    }

    pub fn geo(&self, r: Rect) -> CardGeo {
        let x = r.x + PAD;
        let w = r.w - 2 * PAD;
        let top = r.y + PAD + 34 + 6;
        let row = Rect::new(x + STEP_X - 14, top, w - STEP_X + 14, 40);
        let changed_y = top + 52;
        let lines = self.view.changed.len().min(3) as i32;
        let verdict = Rect::new(x, changed_y + lines * 22 + 4, w, 22);
        let mut y = verdict.bottom() + 6;
        let mut clips = Vec::new();
        for _ in &self.view.clips {
            if y + 28 > r.bottom() - PAD - BUTTON_H - 8 {
                break;
            }
            clips.push(Rect::new(x, y, w, 28));
            y += 32;
        }
        let fit = (!self.view.clips.is_empty()).then(|| Rect::new(x, y + 4, w, BUTTON_H));
        let foot_y = fit.map_or(y + 4, |f| f.bottom() + 12);
        CardGeo {
            row,
            changed_y,
            verdict,
            clips,
            fit,
            foot_y,
        }
    }
}

impl Default for ChordsCard {
    fn default() -> Self {
        ChordsCard::new()
    }
}

impl Widget for ChordsCard {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        self.head.paint(cx, r);
        let t = cx.theme.clone();
        let v = &self.view;
        let g = self.geo(r);
        let x = r.x + PAD;
        cx.canvas
            .fill_round_rect(Rect::new(x, g.row.y, 3, g.row.h), 2, t.accent);
        cx.text(
            FontId::Bold,
            t.size_body,
            Rect::new(x + 14, g.row.y, g.row.x - x - 14, g.row.h),
            Align::Left,
            t.text,
            "Row",
        );
        cx.canvas.fill_round_rect(g.row, t.round(10), t.raised);
        let ends = g.row.inset_xy(10, 0);
        cx.text(
            FontId::Bold,
            t.size_body,
            ends,
            Align::Left,
            t.text_dim,
            "‹",
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            ends,
            Align::Right,
            t.text_dim,
            "›",
        );
        cx.text(
            FontId::Bold,
            t.size_small,
            g.row.inset_xy(22, 0),
            Align::Center,
            t.readable(t.accent, t.raised),
            &v.row,
        );
        for (k, line) in v.changed.iter().take(3).enumerate() {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(x, g.changed_y + k as i32 * 22, r.w - 2 * PAD, 22),
                Align::Left,
                t.text_dim,
                line,
            );
        }
        let ink = if v.warn {
            t.readable(t.warn, t.surface)
        } else {
            t.text_dim
        };
        let font = if v.warn { FontId::Bold } else { FontId::Sans };
        cx.text(
            font,
            t.size_small - 1.0,
            g.verdict,
            Align::Left,
            ink,
            &v.verdict,
        );
        for ((color, name, notes, ticked), b) in v.clips.iter().zip(&g.clips) {
            cx.canvas.fill_round_rect(*b, t.round(8), t.raised);
            let tick = Rect::new(b.x + 8, b.y + 6, 16, 16);
            if *ticked {
                cx.canvas.fill_round_rect(tick, 4, t.accent);
                cx.text(
                    FontId::Bold,
                    t.size_small - 3.0,
                    tick,
                    Align::Center,
                    t.ink_on(t.accent),
                    "✓",
                );
            } else {
                cx.canvas.stroke_round_rect(tick, 4, 1, t.outline);
            }
            let words = Rect::new(b.x + 34, b.y, b.w - 42, b.h);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                words,
                Align::Left,
                t.readable(*color, t.raised),
                name,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                words,
                Align::Right,
                t.text_dim,
                notes,
            );
        }
        if let Some(f) = g.fit {
            button(cx, f, "Fit to the new chords", true, v.can_fit);
            for (k, line) in [
                "Notes under a changed chord move;",
                "rhythm stays. Drums and audio",
                "never move. UNDO takes it back.",
            ]
            .iter()
            .enumerate()
            {
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    Rect::new(x, g.foot_y + k as i32 * 18, r.w - 2 * PAD, 18),
                    Align::Left,
                    t.text_dim,
                    line,
                );
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = self.head.targets(r);
        let g = self.geo(r);
        let (less, more) = g.row.take_left(g.row.w / 2);
        out.push(Target::tap("chords-row-less", 0, "Row ‹", less));
        out.push(Target::tap("chords-row-more", 1, "Row ›", more));
        for (k, b) in g.clips.iter().enumerate() {
            let name = self.view.clips.get(k).map_or("", |c| c.1.as_str());
            out.push(Target::tap("chords-tick", k, format!("Fit {name}"), *b));
        }
        if let Some(f) = g.fit {
            out.push(
                Target::tap("chords-fit", CARD_FIT, "Fit to the new chords", f)
                    .disabled_if(!self.view.can_fit),
            );
        }
        out
    }

    fn touch(&mut self, f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let head_bottom = r.y + PAD + 34;
        if y < head_bottom {
            return self.head.touch(f, phase, x, y, r);
        }
        if phase != Touch::Down {
            return None;
        }
        let g = self.geo(r);
        if g.row.contains(x, y) {
            let delta = if x < g.row.x + g.row.w / 2 { -1 } else { 1 };
            return Some(EventKind::Step { index: 0, delta });
        }
        if let Some(k) = g.clips.iter().position(|b| b.contains(x, y)) {
            return Some(EventKind::Select(k));
        }
        g.fit
            .filter(|f| f.contains(x, y) && self.view.can_fit)
            .map(|_| EventKind::Toggle(CARD_FIT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Tick = 3840;

    fn line(chords: &[(u8, Quality, Tick, Heard)]) -> ChordTimeline {
        let mut at = 0;
        let spans = chords
            .iter()
            .map(|&(root, q, len, heard)| {
                let s = ChordSpan::new(at, len, ChordAnswer::Chord(Chord::new(root, q)), heard);
                at += len;
                s
            })
            .collect();
        ChordTimeline::new(spans).unwrap()
    }

    #[test]
    fn the_picker_lights_the_chord_and_use_makes_it_yours() {
        let l = line(&[
            (0, Quality::Minor7, 2 * BAR, Heard::Heard),
            (7, Quality::Dominant7, BAR, Heard::Heard),
        ]);
        let p = ChordPick::open(4, &l, 1).unwrap();
        assert_eq!((p.root, p.kind), (Some(7), Some(2)));
        let v = p.view(&l, Some(&l), BAR);
        assert_eq!(
            (v.bar, v.heard.as_str(), v.picked.as_str()),
            (3, "G7", "G7")
        );
        assert!(!v.can_use, "nothing changed yet");
        assert!(!v.can_as_heard && !v.can_join && v.can_split);
        let p = ChordPick { root: Some(8), ..p };
        let v = p.view(&l, Some(&l), BAR);
        assert_eq!(v.picked, "Ab7");
        assert!(v.can_use);
        let used = use_chord(&l, 1, p.chord().unwrap()).unwrap();
        let s = used.spans()[1];
        assert_eq!(s.heard, Heard::Set);
        assert_eq!(s.answer.name(), "Ab7");
        // As heard gives it back.
        let back = as_heard(&used, 1, &l).unwrap();
        assert_eq!(back, l);
        let v = ChordPick::open(4, &used, 1)
            .unwrap()
            .view(&used, Some(&l), BAR);
        assert!(v.can_as_heard);
    }

    #[test]
    fn split_cuts_on_a_bar_line_and_join_makes_one_chord() {
        let l = line(&[
            (0, Quality::Minor7, 2 * BAR, Heard::Heard),
            (5, Quality::Minor7, BAR, Heard::Heard),
        ]);
        let s = split(&l, 0, BAR).unwrap();
        assert_eq!(s.spans().len(), 3);
        assert_eq!((s.spans()[0].len, s.spans()[1].start), (BAR, BAR));
        assert!(s.spans()[..2].iter().all(|x| x.heard == Heard::Set));
        // A bar splits on a beat; a beat doesn't split.
        let one = split(&l, 1, BAR).unwrap();
        assert_eq!(one.spans()[1].len, 2 * BAR / 4);
        let beat = line(&[(0, Quality::Major, BAR / 4, Heard::Heard)]);
        assert!(split(&beat, 0, BAR).is_none());
        let j = join_next(&l, 0).unwrap();
        assert_eq!(j.spans().len(), 1);
        assert_eq!(j.spans()[0].len, 3 * BAR);
        assert_eq!(j.spans()[0].answer.name(), "Cm7");
        assert!(join_next(&l, 1).is_none());
        let bare = bare_line(4, BAR);
        assert_eq!(bare.len(), 4 * BAR);
        assert!(bare.spans().iter().all(|s| s.answer.chord().is_none()));
    }

    #[test]
    fn the_panel_buttons_and_steppers_answer_where_they_are() {
        let mut p = AddPanel::new();
        p.view.heard = vec![(Color::rgb(0x3DDC97), "Solo".into(), "notes, 8 bars".into())];
        p.view.step = ("‹ Try 1".into(), true);
        let r = Rect::new(878, 136, 250, 614);
        let g = p.geo(r);
        assert!(g.buttons[3].bottom() <= r.bottom(), "{:?}", g.buttons[3]);
        let mut all: Vec<Rect> = vec![g.close];
        all.extend(g.tiles);
        all.extend(g.steppers);
        all.extend(g.buttons);
        for (i, a) in all.iter().enumerate() {
            assert!(r.intersect(*a) == *a, "{a:?}");
            for b in &all[i + 1..] {
                let o = a.intersect(*b);
                assert!(o.w <= 0 || o.h <= 0, "{a:?} {b:?}");
            }
        }
        let mid = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = mid(g.tiles[2]);
        assert_eq!(p.touch(1, Touch::Down, x, y, r), Some(EventKind::Select(2)));
        let s = g.steppers[ROW_TRACK];
        assert_eq!(
            p.touch(1, Touch::Down, s.x + 4, s.y + 4, r),
            Some(EventKind::Step {
                index: ROW_TRACK,
                delta: -1
            })
        );
        let (x, y) = mid(g.buttons[3]);
        assert_eq!(p.touch(1, Touch::Down, x, y, r), None, "nothing to keep");
        p.view.can_keep = true;
        assert_eq!(
            p.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(PANEL_KEEP))
        );
        let (x, y) = mid(g.close);
        assert_eq!(
            p.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(PANEL_CLOSE))
        );
    }

    /// New track's panel (S2.6b): the Sound stepper takes the reading
    /// line's place and answers there, at most three heard lines show,
    /// and everything still fits the side area without overlapping; off
    /// New track the Sound row doesn't answer.
    #[test]
    fn new_tracks_panel_fits_its_sound_stepper() {
        let mut p = AddPanel::new();
        p.view.heard = (0..6)
            .map(|k| {
                (
                    Color::rgb(0x3DDC97),
                    format!("Track {k}"),
                    "notes, 8 bars".into(),
                )
            })
            .collect();
        p.view.step = ("‹ Try 1".into(), true);
        let r = Rect::new(878, 136, 250, 614);
        let mid = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let sound = p.geo(r).steppers[ROW_SOUND];
        let (x, y) = (sound.x + 4, sound.y + sound.h / 2);
        assert_eq!(p.touch(1, Touch::Down, x, y, r), None, "a reading");
        p.view.new_track = true;
        let g = p.geo(r);
        assert!(g.buttons[3].bottom() <= r.bottom(), "{:?}", g.buttons[3]);
        let mut all: Vec<Rect> = vec![g.close];
        all.extend(g.tiles);
        all.extend(g.steppers);
        all.extend(g.buttons);
        for (i, a) in all.iter().enumerate() {
            assert!(r.intersect(*a) == *a, "{a:?}");
            for b in &all[i + 1..] {
                let o = a.intersect(*b);
                assert!(o.w <= 0 || o.h <= 0, "{a:?} {b:?}");
            }
        }
        let s = g.steppers[ROW_SOUND];
        assert_eq!(
            p.touch(2, Touch::Down, s.x + 4, s.y + 4, r),
            Some(EventKind::Step {
                index: ROW_SOUND,
                delta: -1
            })
        );
        let (x, y) = mid(g.buttons[3]);
        assert_eq!(p.touch(3, Touch::Down, x, y, r), None, "nothing to keep");
    }

    #[test]
    fn the_chords_card_ticks_clips_and_fits_the_ticked() {
        let mut c = ChordsCard::new();
        c.head.tabs = vec!["Record".into(), "Clip".into(), "Chords".into()];
        c.view.clips = vec![
            (
                Color::rgb(0x6EA8FE),
                "Bass".into(),
                "3 notes to move".into(),
                true,
            ),
            (
                Color::rgb(0xC69CF5),
                "Solo".into(),
                "2 notes to move".into(),
                false,
            ),
        ];
        c.view.can_fit = true;
        let r = Rect::new(878, 136, 250, 374);
        let g = c.geo(r);
        assert_eq!(g.clips.len(), 2);
        let f = g.fit.unwrap();
        assert!(f.bottom() <= r.bottom());
        let mid = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = mid(g.clips[1]);
        assert_eq!(c.touch(1, Touch::Down, x, y, r), Some(EventKind::Select(1)));
        let (x, y) = mid(f);
        assert_eq!(
            c.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(CARD_FIT))
        );
        assert_eq!(
            c.touch(1, Touch::Down, g.row.x + 4, g.row.y + 4, r),
            Some(EventKind::Step {
                index: 0,
                delta: -1
            })
        );
    }
}

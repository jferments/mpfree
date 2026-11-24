//! The tuning screens (tunings step T6, approved as drawn by Jay on 23
//! Nov 2026; [tunings.md](../../../../docs/dev/design/tunings.md), layout
//! spec 3): the Synth tab's Tuning row is its instrument's, and the
//! Tunings page holds the song's tuning and every instrument's.
//!
//! - **The Synth tab's Tuning row** (`synth_pads.rs`) picks what the
//!   instrument the tab shows plays in: "Song's" first, then the built-in
//!   tunings and the tuning files as its own. With its own, the OWN tag
//!   shows and its own Tuning root and A4 rows come under the row. A
//!   plugin that takes only MIDI notes can't follow: the row says "12-TET
//!   (can't follow)" in amber and can't be stepped. Its help says what the
//!   instrument and the song play in, with a Tunings button.
//! - **The Tunings page** opens from that button, as an options page (Back
//!   or MENU goes back to the row): the song's Tuning, Tuning root and A4
//!   under one heading, then every instrument of the project's list under
//!   another, each with what it plays in. A kit's, and a free place's,
//!   own root and A4 come under its row there (the Synth tab can't show
//!   them); a synth's help has a button to its options.
//!
//! The rows are worked out each time they are drawn
//! ([`App::tunings_rows`]): the menu's items are the first of
//! [`TUNINGS_ROWS`], one for each.

use mpfree_arp::scale::NOTE_NAMES;
use mpfree_tuning::{Builtin, TuningPick, A4_MAX, A4_MIN};

use super::{synth_pads, wrap_line, App, Effect, Item, MenuView, RowMark, View, HELP_CHARS};
use crate::instrument_list::{InstrumentId, Kind, KITS};
use mpfree_sampler_ui::play_tab::KitTuning;

/// A Tunings page row's help card: its heading, its facts (amber when
/// the bool is set), its link (title, line) and its help.
type RowHelp = (
    String,
    Vec<String>,
    bool,
    Option<(String, String)>,
    Vec<String>,
);

/// One row of the Tunings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuningsRow {
    /// "THE SONG'S TUNING"
    SongHeading,
    SongTuning,
    SongRoot,
    SongA4,
    /// "INSTRUMENTS · WHAT EACH PLAYS IN"
    InstrumentsHeading,
    /// An instrument of the project's list: Song's or its own.
    Instrument(InstrumentId),
    /// A kit's or a free place's own root and A4, under its row while it
    /// has its own.
    OwnRoot(InstrumentId),
    OwnA4(InstrumentId),
}

/// The most rows the page can have: the song's four, the instruments'
/// heading, every instrument, and a root and an A4 for each kit and free
/// place.
pub const TUNINGS_MOST: usize =
    5 + crate::tunings::MOST + 2 * (crate::instrument_list::KITS + crate::instrument_list::FREE);

/// The page's menu items: row `k` is `Item::Tunings(k)`.
pub static TUNINGS_ROWS: [Item; TUNINGS_MOST] = {
    let mut a = [Item::Tunings(0); TUNINGS_MOST];
    let mut k = 0;
    while k < TUNINGS_MOST {
        a[k] = Item::Tunings(k as u16);
        k += 1;
    }
    a
};

/// The page's title, as Back names it.
const TITLE: &str = "Tunings";

/// The Tuning rows' help button to the page.
pub fn link() -> (String, String) {
    (
        TITLE.to_string(),
        "The song's tuning and every instrument's".to_string(),
    )
}

/// What the page says under its rows.
const GEAR_LINE: &str = "Gear on MIDI out plays 12-TET for now.";

/// The longest file name a Tuning row shows, as long as the longest
/// built-in tuning's name.
const VALUE_CHARS: usize = 20;

/// `s`, cut to [`VALUE_CHARS`] with an ellipsis when it is longer.
fn fit(s: &str) -> String {
    if s.chars().count() <= VALUE_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(VALUE_CHARS - 1).collect();
    out.push('…');
    out
}

/// Tracks by number, as a line says them: "Track 3", "Tracks 3 and 4",
/// "Tracks 1, 3 and 4", "Tracks 1, 2, 3 and 5 more".
fn tracks_words(tracks: &[usize]) -> Option<String> {
    let n: Vec<String> = tracks.iter().map(|t| (t + 1).to_string()).collect();
    Some(match n.len() {
        0 => return None,
        1 => format!("Track {}", n[0]),
        2..=4 => format!(
            "Tracks {} and {}",
            n[..n.len() - 1].join(", "),
            n[n.len() - 1]
        ),
        k => format!("Tracks {} and {} more", n[..3].join(", "), k - 3),
    })
}

impl App {
    /// Whether the options show the Tunings page.
    pub(super) fn on_tunings_page(&self) -> bool {
        self.tunings_page && self.view == View::Options
    }

    /// The instrument the Synth tab shows: the selected track's own (or
    /// the one it plays by a send), else the shared synth. None while the
    /// list hasn't given that instrument its id yet.
    pub(super) fn synth_tab_instrument(&self) -> Option<InstrumentId> {
        match self.track_owner(self.track) {
            Some(h) => self.instrument_list.of_holder(h),
            None => Some(InstrumentId::SHARED_SYNTH),
        }
    }

    /// Whether instrument `id` can't follow a tuning (a plugin synth that
    /// takes only MIDI notes, as the audio loop says).
    pub(super) fn tuning_deaf(&self, id: InstrumentId) -> bool {
        let Some(h) = self.instrument_list.kind(id).and_then(|k| k.holder()) else {
            return false;
        };
        let (tracks, free) = self.tuning_deaf;
        if h < crate::instruments::TRACKS {
            tracks >> h & 1 == 1
        } else {
            let place = h - crate::instruments::TRACKS;
            place < 16 && free >> place & 1 == 1
        }
    }

    /// The Synth tab's instrument can't follow a tuning.
    pub(super) fn synth_tab_deaf(&self) -> bool {
        self.synth_tab_instrument()
            .is_some_and(|id| self.tuning_deaf(id))
    }

    /// The Synth tab's instrument plays its own tuning (and can).
    pub(super) fn synth_tab_owns_tuning(&self) -> bool {
        self.synth_tab_instrument()
            .is_some_and(|id| self.tunings.own(id).is_some() && !self.tuning_deaf(id))
    }

    /// What the audio loop says, a few times a second while the options
    /// show: the tracks' and free places' instruments that can't follow a
    /// tuning (a bit each), and the kit each track plays. The options
    /// follow when they change.
    pub fn set_tuning_facts(
        &mut self,
        deaf: (u128, u16),
        kit_of: [Option<u8>; mpfree_seq::MAX_TRACKS],
    ) -> Vec<Effect> {
        if self.tuning_deaf == deaf && *self.track_kits == kit_of {
            return Vec::new();
        }
        self.tuning_deaf = deaf;
        *self.track_kits = kit_of;
        if self.view == View::Options {
            vec![self.menu_view()]
        } else {
            Vec::new()
        }
    }

    /// The kits' names, in kit list order, as the start loaded them: the
    /// Tunings page names a kit by its name.
    pub fn set_kit_names(&mut self, names: Vec<String>) {
        self.kit_names = names;
    }

    // ----------------------------------- the Pads tab's kit Tuning (4e)

    /// The song's tuning and each kit's own, cheaply, for the audio loop
    /// to see a change without making any text.
    pub fn kit_tunings_key(&self) -> (TuningPick, [Option<TuningPick>; KITS]) {
        let own = std::array::from_fn(|k| {
            self.instrument_list
                .of_kit(k)
                .and_then(|id| self.tunings.own(id))
        });
        (self.tunings.song(), own)
    }

    /// Each kit's tuning as the Pads tab's Edit pad names it, in kit
    /// order: its Tuning's value ("Song's: Just (5-limit) on D", or its
    /// own) and the pad line's words.
    pub fn kit_tunings(&self) -> Vec<KitTuning> {
        let (song, own) = self.kit_tunings_key();
        own.iter()
            .map(|p| match p {
                Some(p) => KitTuning {
                    value: self.pick_short(*p),
                    line: "tuning: own".into(),
                },
                None => KitTuning {
                    value: format!("Song's: {}", self.pick_on(song)),
                    line: "tuning: song's".into(),
                },
            })
            .collect()
    }

    /// Edit pad's Tuning for kit `kit` stepped by `d`: Song's, then the
    /// built-in tunings and the files as its own, as its row on the
    /// Tunings page.
    pub fn step_kit_tuning(&mut self, kit: usize, d: i32) -> Vec<Effect> {
        match self.instrument_list.of_kit(kit) {
            Some(id) => self.step_own_tuning(id, d),
            None => Vec::new(),
        }
    }

    // -------------------------------------------------- tunings as text

    /// The tuning file `p` picks, if the app has it: (name, whether its
    /// .kbm sets its root and A4).
    pub(super) fn pick_file(&self, p: TuningPick) -> Option<&(String, bool)> {
        self.tuning_files.get(usize::from(p.file?))
    }

    /// Where `p` is on a Tuning row's choices: the built-in tunings, then
    /// the files.
    pub(super) fn tuning_choice_of(&self, p: TuningPick) -> usize {
        match p.file {
            Some(f) if usize::from(f) < self.tuning_files.len() => {
                Builtin::ALL.len() + usize::from(f)
            }
            _ => Builtin::ALL
                .iter()
                .position(|&b| b == p.builtin)
                .unwrap_or(0),
        }
    }

    /// How many choices a Tuning row has for a tuning (Song's aside).
    pub(super) fn tuning_choices(&self) -> usize {
        Builtin::ALL.len() + self.tuning_files.len()
    }

    /// `p` with choice `c` (a built-in tuning, then the files), its root
    /// and A4 kept. With a file picked, the built-in tuning is 12-TET:
    /// what plays if the file were missing, and what a kept file pick
    /// comes back with.
    pub(super) fn with_choice(&self, mut p: TuningPick, c: usize) -> TuningPick {
        (p.builtin, p.file) = match Builtin::ALL.get(c) {
            Some(&b) => (b, None),
            None => (Builtin::Equal12, Some((c - Builtin::ALL.len()) as u8)),
        };
        p
    }

    /// A Tuning row's value for `p`: the built-in tuning's name, or the
    /// file's, cut to fit.
    pub(super) fn pick_name(&self, p: TuningPick) -> String {
        match self.pick_file(p) {
            Some((name, _)) => fit(name),
            None => p.builtin.name().to_string(),
        }
    }

    /// `p`'s name and root: "Just (5-limit) on D", "12-TET", a file with a
    /// keyboard map by its name.
    pub(super) fn pick_on(&self, p: TuningPick) -> String {
        let (name, root) = match self.pick_file(p) {
            Some((name, true)) => return name.clone(),
            Some((name, false)) => (name.as_str(), true),
            None => (p.builtin.name(), p.builtin.has_root()),
        };
        if root {
            format!("{name} on {}", NOTE_NAMES[usize::from(p.root % 12)])
        } else {
            name.to_string()
        }
    }

    /// `p` in full, as the help says it: "Just (5-limit) on D, A4 = 440
    /// Hz"; a file whose map sets its root and A4 by its name alone.
    pub(super) fn pick_full(&self, p: TuningPick) -> String {
        if self.pick_file(p).is_some_and(|f| f.1) {
            return self.pick_on(p);
        }
        format!("{}, A4 = {} Hz", self.pick_on(p), p.a4_hz)
    }

    /// The Tunings page's value for an own tuning: "Meantone 1/4 comma on
    /// C · 415 Hz" (A4 only when not 440); the row cuts it to fit.
    fn pick_short(&self, p: TuningPick) -> String {
        let mut s = self.pick_on(p);
        if p.a4_hz != 440 && !self.pick_file(p).is_some_and(|f| f.1) {
            s.push_str(&format!(" · {} Hz", p.a4_hz));
        }
        s
    }

    /// A Tuning root row's value for `p`.
    pub(super) fn root_text(&self, p: TuningPick) -> String {
        let root = NOTE_NAMES[usize::from(p.root % 12)];
        match self.pick_file(p) {
            Some((_, true)) => "set by its .kbm".to_string(),
            Some(_) => root.to_string(),
            None if p.builtin.has_root() => root.to_string(),
            None => format!("{root} (not used)"),
        }
    }

    /// An A4 row's value for `p`.
    pub(super) fn a4_text(&self, p: TuningPick) -> String {
        if self.pick_file(p).is_some_and(|f| f.1) {
            "set by its .kbm".to_string()
        } else {
            format!("{} Hz", p.a4_hz)
        }
    }

    /// `p` with its root stepped by `d` (C to B, no further).
    pub(super) fn step_root(p: TuningPick, d: i32) -> TuningPick {
        TuningPick {
            root: (i32::from(p.root % 12) + d).clamp(0, 11) as u8,
            ..p
        }
    }

    /// `p` with A4 stepped by `d` Hz (380 to 480 Hz, no further).
    pub(super) fn step_a4(p: TuningPick, d: i32) -> TuningPick {
        TuningPick {
            a4_hz: (i32::from(p.a4_hz) + d).clamp(i32::from(A4_MIN), i32::from(A4_MAX)) as u16,
            ..p
        }
    }

    // ------------------------------------------------ the instruments

    /// The tracks that play instrument `id`: a synth's tracks (its own, by
    /// a send, or the shared synth's), a kit's.
    fn tracks_of(&self, id: InstrumentId) -> Vec<usize> {
        let Some(kind) = self.instrument_list.kind(id) else {
            return Vec::new();
        };
        (0..mpfree_seq::MAX_TRACKS)
            .filter(|&t| match kind {
                Kind::SharedSynth => self.plays_pads_synth(t),
                Kind::Kit { kit } => {
                    self.track_kits[t] == Some(kit as u8) && !self.track_plays_instrument(t)
                }
                k => k.holder().is_some() && self.track_owner(t) == k.holder(),
            })
            .collect()
    }

    /// An instrument's name and who plays it: "Poly · Track 2", "FM ·
    /// Tracks 3 and 4", "Classic House · kit".
    pub(super) fn instrument_title(&self, id: InstrumentId) -> String {
        let tracks = tracks_words(&self.tracks_of(id));
        match self.instrument_list.kind(id) {
            Some(Kind::SharedSynth) => format!(
                "{} · {}",
                self.synth_pads.engine.name(),
                tracks.unwrap_or_else(|| "the pads".to_string())
            ),
            Some(Kind::Own { track }) => format!(
                "{} · {}",
                self.track_sound(track).map_or("Synth", |h| h.name),
                tracks.unwrap_or_else(|| format!("Track {}", track + 1))
            ),
            Some(Kind::Free { place }) => format!(
                "Synth · {}",
                tracks.unwrap_or_else(|| format!("free place {}", place + 1))
            ),
            Some(Kind::Kit { kit }) => match self.kit_names.get(kit).filter(|n| !n.is_empty()) {
                Some(name) => format!("{name} · kit"),
                None => format!("Kit {}", kit + 1),
            },
            None => "An instrument".to_string(),
        }
    }

    /// An instrument's name alone: "Poly", "Bassline", "Classic House".
    fn instrument_name_of(&self, id: InstrumentId) -> String {
        match self.instrument_list.kind(id) {
            Some(Kind::SharedSynth) => self.synth_pads.engine.name().to_string(),
            Some(Kind::Own { track }) => self
                .track_sound(track)
                .map_or("This synth", |h| h.name)
                .to_string(),
            Some(Kind::Kit { kit }) => self
                .kit_names
                .get(kit)
                .filter(|n| !n.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Kit {}", kit + 1)),
            _ => "This synth".to_string(),
        }
    }

    /// The instruments the page lists, in order: the kits the tracks play
    /// and those with their own tuning, the shared synth, the tracks' own,
    /// the free places'.
    fn listed_instruments(&self) -> Vec<InstrumentId> {
        let list = &self.instrument_list;
        let kits = (0..crate::instrument_list::KITS).filter_map(|k| {
            let id = list.of_kit(k)?;
            let used = k == 0
                || self.track_kits.contains(&Some(k as u8))
                || self.tunings.own(id).is_some();
            used.then_some(id)
        });
        let own = (0..crate::instrument_list::TRACKS).filter_map(|t| list.own_of(t));
        let free = (0..crate::instrument_list::FREE).filter_map(|p| list.free_of(p));
        kits.chain(std::iter::once(InstrumentId::SHARED_SYNTH))
            .chain(own)
            .chain(free)
            .collect()
    }

    /// Whether `id`'s own root and A4 are on the Tunings page (a kit's
    /// and a free place's; a track's synth has them on the Synth tab).
    fn root_on_page(&self, id: InstrumentId) -> bool {
        matches!(
            self.instrument_list.kind(id),
            Some(Kind::Kit { .. } | Kind::Free { .. })
        )
    }

    /// The Tunings page's rows as they are now.
    pub(super) fn tunings_rows(&self) -> Vec<TuningsRow> {
        let mut rows = vec![
            TuningsRow::SongHeading,
            TuningsRow::SongTuning,
            TuningsRow::SongRoot,
            TuningsRow::SongA4,
            TuningsRow::InstrumentsHeading,
        ];
        for id in self.listed_instruments() {
            rows.push(TuningsRow::Instrument(id));
            if self.root_on_page(id) && self.tunings.own(id).is_some() && !self.tuning_deaf(id) {
                rows.push(TuningsRow::OwnRoot(id));
                rows.push(TuningsRow::OwnA4(id));
            }
        }
        rows.truncate(TUNINGS_MOST);
        rows
    }

    /// Row `k` of the Tunings page, if there is one.
    fn tunings_row(&self, k: usize) -> Option<TuningsRow> {
        self.tunings_rows().get(k).copied()
    }

    /// Row `k`'s marks as the wheel and a tap go by: a heading, or a value
    /// that can't be changed (the row as drawn has more,
    /// [`App::tunings_page_view`]).
    pub(super) fn tunings_item_mark(&self, k: usize) -> RowMark {
        match self.tunings_row(k) {
            Some(TuningsRow::SongHeading | TuningsRow::InstrumentsHeading) => RowMark {
                heading: true,
                ..RowMark::default()
            },
            Some(TuningsRow::Instrument(id)) if self.tuning_deaf(id) => RowMark {
                fixed: true,
                ..RowMark::default()
            },
            _ => RowMark::default(),
        }
    }

    /// A row's name, value and marks.
    fn tunings_row_text(&self, row: TuningsRow) -> (String, String, RowMark) {
        let song = self.tunings.song();
        match row {
            TuningsRow::SongHeading => (
                "The song's tuning".to_string(),
                String::new(),
                RowMark {
                    heading: true,
                    ..RowMark::default()
                },
            ),
            TuningsRow::InstrumentsHeading => (
                "Instruments · what each plays in".to_string(),
                String::new(),
                RowMark {
                    heading: true,
                    ..RowMark::default()
                },
            ),
            TuningsRow::SongTuning => (
                "Tuning".to_string(),
                self.pick_name(song),
                RowMark::default(),
            ),
            TuningsRow::SongRoot => (
                "Tuning root".to_string(),
                self.root_text(song),
                RowMark::default(),
            ),
            TuningsRow::SongA4 => ("A4".to_string(), self.a4_text(song), RowMark::default()),
            TuningsRow::Instrument(id) => {
                let own = self.tunings.own(id);
                let deaf = self.tuning_deaf(id);
                let sub = match (self.instrument_list.kind(id), own) {
                    _ if deaf => "plugin, takes MIDI only".to_string(),
                    (Some(Kind::Kit { .. }), _) => {
                        let tracks = tracks_words(&self.tracks_of(id));
                        match tracks {
                            Some(t) => format!("{t} · keygroups, Notes and 16 Levels"),
                            None => "keygroups, Notes and 16 Levels".to_string(),
                        }
                    }
                    (_, Some(_)) => "its own".to_string(),
                    (Some(Kind::SharedSynth), None) => "on the pads too".to_string(),
                    _ => String::new(),
                };
                let value = match own {
                    _ if deaf => "12-TET (can't follow)".to_string(),
                    Some(p) => self.pick_short(p),
                    None => "Song's".to_string(),
                };
                (
                    self.instrument_title(id),
                    value,
                    RowMark {
                        tag: "",
                        sub,
                        fixed: deaf,
                        heading: false,
                    },
                )
            }
            TuningsRow::OwnRoot(id) => {
                let p = self.tunings.pick(Some(id));
                (
                    "Tuning root".to_string(),
                    self.root_text(p),
                    RowMark {
                        sub: format!("{}'s own", self.instrument_name_of(id)),
                        ..RowMark::default()
                    },
                )
            }
            TuningsRow::OwnA4(id) => {
                let p = self.tunings.pick(Some(id));
                (
                    "A4".to_string(),
                    self.a4_text(p),
                    RowMark {
                        sub: format!("{}'s own", self.instrument_name_of(id)),
                        ..RowMark::default()
                    },
                )
            }
        }
    }

    /// The selected row's heading, facts, link and help.
    fn tunings_row_help(&self, row: TuningsRow) -> RowHelp {
        let song = self.tunings.song();
        let lines = |words: &[&str]| -> Vec<String> {
            let mut out = Vec::new();
            for (i, w) in words.iter().enumerate() {
                if i > 0 {
                    out.push(String::new());
                }
                out.extend(wrap_line(w, HELP_CHARS));
            }
            out
        };
        match row {
            TuningsRow::SongHeading | TuningsRow::InstrumentsHeading | TuningsRow::SongTuning => (
                "The song's tuning".to_string(),
                lines(&[&format!("The song: {}", self.pick_full(song))]),
                false,
                None,
                lines(&[
                    "Every instrument on Song's plays it and moves when it changes: 12-TET (the usual), just intonation (pure thirds and fifths on the root), Pythagorean (pure fifths), quarter-comma meantone (pure thirds) or 24-TET (quarter tones). Scala .scl files in files/tunings/ come after them.",
                    "Kits' keygroups, Notes and 16 Levels play it too; pad hits and slices keep their own pitch. Held notes follow at once. It is saved with the project; a new project is 12-TET at A4 = 440 Hz.",
                ]),
            ),
            TuningsRow::SongRoot | TuningsRow::OwnRoot(_) => (
                "Tuning root".to_string(),
                Vec::new(),
                false,
                None,
                lines(&[
                    "The note just intonation, Pythagorean, meantone and a tuning file start on. It keeps its usual pitch and the others move around it. The equal tunings don't use it, and a file's .kbm keyboard map sets its own.",
                ]),
            ),
            TuningsRow::SongA4 | TuningsRow::OwnA4(_) => (
                "A4".to_string(),
                Vec::new(),
                false,
                None,
                lines(&[
                    "The pitch of the A above middle C, 380 to 480 Hz: 440 is the usual, 432 and 442 are common, 415 is baroque pitch. A file's .kbm keyboard map sets its own.",
                ]),
            ),
            TuningsRow::Instrument(id) => {
                let own = self.tunings.own(id);
                let deaf = self.tuning_deaf(id);
                let name = self.instrument_name_of(id);
                let facts = if deaf {
                    lines(&[&format!(
                        "{name} takes MIDI notes only, so it plays 12-TET at A4 = 440 Hz"
                    )])
                } else {
                    match own {
                        Some(p) => lines(&[&format!("Its own: {}", self.pick_full(p))]),
                        None => lines(&[&format!("Song's: {}", self.pick_full(song))]),
                    }
                };
                let kit = matches!(self.instrument_list.kind(id), Some(Kind::Kit { .. }));
                let link = (!kit && !self.root_on_page(id)).then(|| {
                    (
                        "Open its options".to_string(),
                        "Tuning root and A4 are on the Synth tab".to_string(),
                    )
                });
                let mut help = vec![
                    "Each instrument plays the song's tuning unless it has its own. Step a row to give it one, or back to Song's.",
                ];
                if kit {
                    help.push("A kit's keygroups, Notes and 16 Levels play its tuning; its pad hits and slices play as tuned on the pad. Its own root and A4 come under its row.");
                } else if deaf {
                    help.push("A plugin synth that takes only MIDI notes, and an MPC plugin, plays 12-TET whatever it is given.");
                }
                (self.instrument_title(id), facts, deaf, link, lines(&help))
            }
        }
    }

    /// The Tunings page as the options show it.
    pub(super) fn tunings_page_view(&self) -> MenuView {
        let rows = self.tunings_rows();
        let selected = self.selected.min(rows.len() - 1);
        let (mut names, mut marks) = (Vec::new(), Vec::new());
        for &row in &rows {
            let (name, value, mark) = self.tunings_row_text(row);
            names.push((name, value));
            marks.push(mark);
        }
        let (help_heading, facts, facts_warn, help_link, mut help) =
            self.tunings_row_help(rows[selected]);
        help.push(String::new());
        help.push(GEAR_LINE.to_string());
        MenuView {
            title: TITLE.to_string(),
            rows: names,
            marks,
            selected,
            editing: self.menu_editing,
            help_heading,
            facts,
            facts_warn,
            help_link,
            help,
            help_buttons: Vec::new(),
            prefs: None,
        }
    }

    /// The Tunings page opens (a Tuning row's help button), on the song's
    /// Tuning row.
    pub fn open_tunings_page(&mut self) -> Vec<Effect> {
        if self.view != View::Options {
            return Vec::new();
        }
        self.tunings_page = true;
        self.menu_editing = false;
        self.selected = 1;
        vec![self.menu_view()]
    }

    /// Back or MENU on the Tunings page: the options it opened from, on
    /// their Tuning row.
    pub(super) fn leave_tunings_page(&mut self) -> Vec<Effect> {
        self.tunings_page = false;
        self.menu_editing = false;
        self.selected = self
            .row_of(Item::Synth(synth_pads::SynthItem::Tuning))
            .unwrap_or(0);
        vec![self.menu_view()]
    }

    /// A tap on the help's button: on a Tuning row, the Tunings page; on
    /// the Tunings page, a synth's options (its track picked, on its
    /// Tuning row).
    pub(super) fn follow_help_link(&mut self) -> Vec<Effect> {
        if !self.on_tunings_page() {
            let on_tuning = self.items().get(self.selected).is_some_and(|&i| {
                matches!(
                    i,
                    Item::Synth(
                        synth_pads::SynthItem::Tuning
                            | synth_pads::SynthItem::TuningRoot
                            | synth_pads::SynthItem::A4
                    )
                )
            });
            return if on_tuning {
                self.open_tunings_page()
            } else {
                Vec::new()
            };
        }
        let Some(TuningsRow::Instrument(id)) = self.tunings_row(self.selected) else {
            return Vec::new();
        };
        let track = match self.instrument_list.kind(id) {
            Some(Kind::Own { track }) => Some(track),
            Some(Kind::SharedSynth) => {
                // The selected track if it shows the shared synth, else
                // the first that plays it, else the first without an
                // instrument of its own.
                if self.track_owner(self.track).is_none() {
                    Some(self.track)
                } else {
                    (0..mpfree_seq::MAX_TRACKS)
                        .find(|&t| self.plays_pads_synth(t))
                        .or_else(|| {
                            (0..self.track_count.max(1)).find(|&t| self.track_owner(t).is_none())
                        })
                }
            }
            _ => None,
        };
        let Some(track) = track else {
            return Vec::new();
        };
        let mut fx = Vec::new();
        if track != self.track {
            fx.extend(self.set_track(track));
        }
        self.tunings_page = false;
        self.menu_editing = false;
        if self.back != View::Synth {
            // The Synth tab's options, as MENU there opens them.
            fx.extend(self.show(View::Synth));
            fx.extend(self.show(View::Options));
        }
        self.selected = self
            .row_of(Item::Synth(synth_pads::SynthItem::Tuning))
            .unwrap_or(0);
        fx.push(self.menu_view());
        fx
    }

    /// Row `k` of the Tunings page stepped by `d`.
    pub(super) fn step_tunings_row(&mut self, k: usize, d: i32) -> Vec<Effect> {
        let Some(row) = self.tunings_row(k) else {
            return Vec::new();
        };
        let song = self.tunings.song();
        let song_to = |p: TuningPick| Some(p).filter(|&p| p != song);
        match row {
            TuningsRow::SongHeading | TuningsRow::InstrumentsHeading => Vec::new(),
            TuningsRow::SongTuning => {
                let at = self.tuning_choice_of(song) as i32;
                let to = (at + d).clamp(0, self.tuning_choices() as i32 - 1) as usize;
                self.song_tuning_to(song_to(self.with_choice(song, to)))
            }
            TuningsRow::SongRoot => self.song_tuning_to(song_to(App::step_root(song, d))),
            TuningsRow::SongA4 => self.song_tuning_to(song_to(App::step_a4(song, d))),
            TuningsRow::Instrument(id) => self.step_own_tuning(id, d),
            TuningsRow::OwnRoot(id) => match self.tunings.own(id) {
                Some(p) => self.set_own_if_changed(id, Some(App::step_root(p, d))),
                None => Vec::new(),
            },
            TuningsRow::OwnA4(id) => match self.tunings.own(id) {
                Some(p) => self.set_own_if_changed(id, Some(App::step_a4(p, d))),
                None => Vec::new(),
            },
        }
    }

    /// The song's tuning set to `p`, as its rows on the Tunings page set
    /// it; nothing when it is already.
    pub fn set_song_tuning(&mut self, p: TuningPick) -> Vec<Effect> {
        let song = self.tunings.song();
        self.song_tuning_to(Some(p).filter(|&p| p != song))
    }

    /// The song's tuning set to `p` (None: no change): the shared synth
    /// follows if it plays Song's, and the project changed.
    fn song_tuning_to(&mut self, p: Option<TuningPick>) -> Vec<Effect> {
        let Some(p) = p else {
            return Vec::new();
        };
        self.tunings.set_song(p);
        self.project_edit = true;
        self.shared_tuning_follows();
        vec![Effect::SynthPads(self.synth_pads)]
    }

    /// An instrument's Tuning row stepped by `d`: Song's, then the built-in
    /// tunings and files as its own (from the song's root and A4). One that
    /// can't follow says why.
    pub(super) fn step_own_tuning(&mut self, id: InstrumentId, d: i32) -> Vec<Effect> {
        if self.tuning_deaf(id) {
            return vec![Effect::Tell(format!(
                "{} takes MIDI notes only, so it plays 12-TET: it can't follow a tuning",
                self.instrument_name_of(id)
            ))];
        }
        let own = self.tunings.own(id);
        // 0 is Song's, then the choices.
        let at = own.map_or(0, |p| 1 + self.tuning_choice_of(p)) as i32;
        let to = (at + d).clamp(0, self.tuning_choices() as i32) as usize;
        let next = match to {
            0 => None,
            c => Some(self.with_choice(own.unwrap_or(self.tunings.song()), c - 1)),
        };
        self.set_own_if_changed(id, next)
    }

    /// Instrument `id`'s own tuning set to `pick`, if that changes it.
    pub(super) fn set_own_if_changed(
        &mut self,
        id: InstrumentId,
        pick: Option<TuningPick>,
    ) -> Vec<Effect> {
        if self.tunings.own(id) == pick {
            return Vec::new();
        }
        self.set_own_tuning(id, pick)
    }

    // --------------------------------------- the Synth tab's Tuning row

    /// The Synth tab's Tuning row's value: "Song's: Just (5-limit) on D",
    /// its own tuning's name, or "12-TET (can't follow)".
    pub(super) fn synth_tuning_value(&self) -> String {
        let Some(id) = self.synth_tab_instrument() else {
            return format!("Song's: {}", self.pick_on(self.tunings.song()));
        };
        if self.tuning_deaf(id) {
            return "12-TET (can't follow)".to_string();
        }
        match self.tunings.own(id) {
            Some(p) => self.pick_name(p),
            None => format!("Song's: {}", self.pick_on(self.tunings.song())),
        }
    }

    /// The Synth tab's instrument's own tuning, for its own root and A4
    /// rows (the song's while it has none).
    pub(super) fn synth_tab_pick(&self) -> TuningPick {
        self.tunings.pick(self.synth_tab_instrument())
    }

    /// The Tuning row's facts, over its help: what the instrument and the
    /// song play in.
    pub(super) fn synth_tuning_facts(&self) -> Vec<String> {
        let song = format!("The song: {}", self.pick_full(self.tunings.song()));
        let Some(id) = self.synth_tab_instrument() else {
            return wrap_line(&song, HELP_CHARS);
        };
        let first = if self.tuning_deaf(id) {
            format!(
                "{} takes MIDI notes only, so it plays 12-TET at A4 = 440 Hz",
                self.instrument_name_of(id)
            )
        } else {
            match self.tunings.own(id) {
                Some(p) => format!(
                    "{} plays its own tuning: {}",
                    self.instrument_title(id),
                    self.pick_full(p)
                ),
                None => format!("{} plays the song's tuning", self.instrument_title(id)),
            }
        };
        let mut out = wrap_line(&first, HELP_CHARS);
        out.extend(wrap_line(&song, HELP_CHARS));
        out
    }

    /// The Synth tab's Tuning row (or its own root or A4) stepped by `d`.
    pub(super) fn step_synth_tuning(&mut self, item: synth_pads::SynthItem, d: i32) -> Vec<Effect> {
        use synth_pads::SynthItem;
        let Some(id) = self.synth_tab_instrument() else {
            return vec![Effect::Tell(
                "tunings: this instrument isn't in the project's list yet; try again in a moment"
                    .to_string(),
            )];
        };
        match item {
            SynthItem::Tuning => self.step_own_tuning(id, d),
            SynthItem::TuningRoot => match self.tunings.own(id) {
                Some(p) => self.set_own_if_changed(id, Some(App::step_root(p, d))),
                None => Vec::new(),
            },
            SynthItem::A4 => match self.tunings.own(id) {
                Some(p) => self.set_own_if_changed(id, Some(App::step_a4(p, d))),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

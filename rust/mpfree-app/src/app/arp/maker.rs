//! New idea and More like this on the Phrase tab (composing tools P2): a
//! phrase made to order, or more like the one the track plays, tried on the
//! armed track before it joins Yours (the design is in
//! `docs/dev/design/arpeggiator.md`, "New idea and More like this").
//!
//! - **New idea** (the list's heading) makes a phrase of a kind, in a
//!   style, of one, two or four bars, as busy as asked
//!   ([`mpfree_compose::new_idea`]), written for the key the phrase follows
//!   (its minor or major; the In key switch moves it onto the key itself).
//! - **More like this** (the detail panel's heading) makes another from the
//!   track's phrase: its rhythm with new notes, its notes on a new rhythm
//!   ([`mpfree_compose::next_rework`]), or developed: a step up, upside
//!   down, its first half twice ([`mpfree_compose::develop`]). A phrase the
//!   composer cannot read (not 4/4 on a 1/16 or 1/8 grid, or longer than
//!   four bars) is offered the developed changes alone.
//!
//! While either is open its panel takes the list's place, and:
//!
//! - The try shown is the armed track's engine's phrase
//!   ([`App::send_phrase`]), so a held pad plays it, with the tab's Speed,
//!   Gate, Swing and Octaves. The project keeps the track's pick, never a
//!   try.
//! - The wheel picks a row and - and + change it. A change makes a try at
//!   once; Try again makes another with the same choices (the composer's
//!   next attempt that is none of the tries kept). The last [`KEPT`] are
//!   kept, and ‹ Try N steps back through them. A developed change comes
//!   out one way, so Try again has nothing to add there.
//! - **Keep** puts the try on Yours under a free name ("Groove bass 1",
//!   "Night drive 2"), the track picks it, and with a store it is saved
//!   with the owner's other phrases ([`Effect::SavePhrases`]).
//! - **Back** (or ×) closes it and the track's pick plays again. So does
//!   anything that takes the tab away: another tab, another page, another
//!   track, a project opened.
//!
//! A try is made where it is asked, on the control thread: the composer
//! makes one in well under a millisecond on a desktop build (ten in 4 ms,
//! 24 Nov 2026), and the panel says nothing is new rather than retrying
//! for ever ([`mpfree_compose::idea::TRIES`]).

use crate::app::{App, Effect, View};
use mpfree_arp::{Mode, Phrase, PhraseKind, Scale, Written};
use mpfree_arp_ui::maker::{MakerRow, MakerView};
use mpfree_arp_ui::phrase_tab::key_name;
use mpfree_arp_ui::phrases::Shelf;
use mpfree_arp_ui::view::MakerAction;
use mpfree_compose::style::STYLES;
use mpfree_compose::{develop, next_idea, next_rework, reworkable, Develop, NewIdea, Rework};

/// Tries kept while the panel is open, as Add a part keeps.
pub const KEPT: usize = 8;

/// How loud Keep as a phrase writes a pattern's plain steps: a phrase's
/// plain note (its accents are louder, a `vel` lane says its own).
const PATTERN_VELOCITY: u8 = mpfree_arp::phrase::PLAIN_VELOCITY;

/// The Bars row's values.
const BARS: [u8; 3] = [1, 2, 4];

/// The Activity row's values: the composer's Sparse (0) to Busy (1).
const ACTIVITY: [(&str, f32); 5] = [
    ("Sparse", 0.0),
    ("Light", 0.25),
    ("Middle", 0.5),
    ("Busy", 0.75),
    ("Very busy", 1.0),
];

/// What More like this changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Change {
    NewNotes,
    NewRhythm,
    Up,
    Upside,
    Halves,
}

impl Change {
    const ALL: [Change; 5] = [
        Change::NewNotes,
        Change::NewRhythm,
        Change::Up,
        Change::Upside,
        Change::Halves,
    ];

    fn label(self) -> &'static str {
        match self {
            Change::NewNotes => "New notes",
            Change::NewRhythm => "New rhythm",
            Change::Up => "Up a step",
            Change::Upside => "Upside down",
            Change::Halves => "First half twice",
        }
    }

    /// The line under the row.
    fn note(self) -> &'static str {
        match self {
            Change::NewNotes => "Its rhythm, with new notes",
            Change::NewRhythm => "Its notes, on a new rhythm",
            Change::Up => "Every note a step up its scale",
            Change::Upside => "Turned upside down about its home note",
            Change::Halves => "Its first half, played twice",
        }
    }

    /// The rework this change asks the composer for, or None for a
    /// developed change, which comes out one way.
    fn rework(self) -> Option<Rework> {
        match self {
            Change::NewNotes => Some(Rework::NewNotes),
            Change::NewRhythm => Some(Rework::NewRhythm),
            _ => None,
        }
    }

    fn develop(self) -> Option<Develop> {
        match self {
            Change::Up => Some(Develop::Up),
            Change::Upside => Some(Develop::Upside),
            Change::Halves => Some(Develop::Halves),
            _ => None,
        }
    }
}

/// The choices kept between openings, as Continue keeps its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct MakerSet {
    /// A place in [`STYLES`].
    style: usize,
    /// A place in [`BARS`].
    bars: usize,
    /// A place in [`ACTIVITY`].
    activity: usize,
}

impl Default for MakerSet {
    /// Groove, two bars, Middle: what the owner's boards show.
    fn default() -> Self {
        MakerSet {
            style: 1,
            bars: 1,
            activity: 2,
        }
    }
}

/// A row of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowName {
    Kind,
    Style,
    Bars,
    Activity,
    Change,
}

/// One try, and the name Keep gives it before its number.
#[derive(Clone, Debug, PartialEq)]
struct Try {
    phrase: Phrase,
    base: String,
}

/// New idea or More like this, open.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct Maker {
    /// More like this: the phrase it makes more from. None: New idea.
    from: Option<(String, Phrase)>,
    /// The composer can read `from` (a rework is offered).
    reworkable: bool,
    kind: PhraseKind,
    set: MakerSet,
    change: Change,
    /// Written in minor: the key the phrase follows is minor (or of no
    /// kind, or none).
    minor: bool,
    seed: u64,
    /// The composer's next attempt for Try again.
    attempt: u16,
    tries: Vec<Try>,
    shown: usize,
    /// The row the wheel is on.
    row: usize,
    /// Why the last ask made nothing, for the status.
    refused: Option<String>,
}

/// Whether a phrase following `key` is written in minor: unless the key
/// has a major third and no minor one. A key of no kind, or none, takes
/// minor, as most of MPFree's phrases are.
fn minor_for(key: Option<Scale>) -> bool {
    key.is_none_or(|k| !(k.mask & (1 << 4) != 0 && k.mask & (1 << 3) == 0))
}

/// `name` without a number at its end: "Night drive 2" is "Night drive".
fn base_of(name: &str) -> &str {
    match name.rsplit_once(' ') {
        Some((head, n))
            if !head.trim().is_empty()
                && !n.is_empty()
                && n.chars().all(|c| c.is_ascii_digit()) =>
        {
            head.trim_end()
        }
        _ => name,
    }
}

/// The first name "`base` N", N from `first`, that no phrase in `names`
/// has (ignoring case), within the library's 32 characters.
fn free_name<'a>(base: &str, first: u32, names: impl Iterator<Item = &'a str> + Clone) -> String {
    let mut n = first;
    loop {
        let tail = format!(" {n}");
        let room = mpfree_arp::library::MAX_NAME.saturating_sub(tail.len());
        let head: String = base.chars().take(room).collect();
        let name = format!("{}{tail}", head.trim_end());
        if !names.clone().any(|m| m.eq_ignore_ascii_case(&name)) {
            return name;
        }
        n += 1;
    }
}

/// `base` as a phrase's name when no phrase in `names` has it (within the
/// library's 32 characters), else the first "`base` N" free from 2: what
/// Keep as a phrase and Make a phrase name theirs.
pub(super) fn keep_name<'a>(base: &str, names: impl Iterator<Item = &'a str> + Clone) -> String {
    let bare: String = base.chars().take(mpfree_arp::library::MAX_NAME).collect();
    if names.clone().any(|n| n.eq_ignore_ascii_case(&bare)) {
        free_name(base, 2, names)
    } else {
        bare
    }
}

impl Maker {
    fn rows(&self) -> &'static [RowName] {
        match (&self.from, self.change.rework()) {
            (None, _) => &[
                RowName::Kind,
                RowName::Style,
                RowName::Bars,
                RowName::Activity,
            ],
            (Some(_), Some(_)) => &[RowName::Change, RowName::Style],
            // A developed change has no say from a style.
            (Some(_), None) => &[RowName::Change],
        }
    }

    /// The changes on offer: all five, or the developed ones alone for a
    /// phrase the composer cannot read.
    fn changes(&self) -> &'static [Change] {
        if self.reworkable {
            &Change::ALL
        } else {
            &Change::ALL[2..]
        }
    }

    fn request(&self) -> NewIdea {
        NewIdea {
            kind: self.kind,
            style: STYLES[self.set.style],
            minor: self.minor,
            bars: BARS[self.set.bars],
            activity: ACTIVITY[self.set.activity].1,
            seed: self.seed,
            attempt: self.attempt,
        }
    }

    /// The name a try of the choices as they are takes before its number.
    fn base(&self) -> String {
        match &self.from {
            None => format!("{} {}", STYLES[self.set.style].name, self.kind.word()),
            Some((name, _)) => base_of(name).to_string(),
        }
    }

    /// Row `row` stepped by `delta`. Whether its value changed (it stops
    /// at its ends).
    fn step(&mut self, row: usize, delta: i32) -> bool {
        fn moved(at: &mut usize, len: usize, delta: i32) -> bool {
            let to = (*at as i32 + delta).clamp(0, len as i32 - 1) as usize;
            std::mem::replace(at, to) != to
        }
        let Some(&name) = self.rows().get(row) else {
            return false;
        };
        let changed = match name {
            RowName::Kind => {
                let mut at = PhraseKind::ALL
                    .iter()
                    .position(|k| *k == self.kind)
                    .unwrap_or(0);
                let c = moved(&mut at, PhraseKind::ALL.len(), delta);
                self.kind = PhraseKind::ALL[at];
                c
            }
            RowName::Style => moved(&mut self.set.style, STYLES.len(), delta),
            RowName::Bars => moved(&mut self.set.bars, BARS.len(), delta),
            RowName::Activity => moved(&mut self.set.activity, ACTIVITY.len(), delta),
            RowName::Change => {
                let changes = self.changes();
                let mut at = changes.iter().position(|c| *c == self.change).unwrap_or(0);
                let c = moved(&mut at, changes.len(), delta);
                self.change = changes[at];
                c
            }
        };
        if changed {
            // New choices: the composer's attempts count from the first.
            self.attempt = 0;
            self.row = self.row.min(self.rows().len() - 1);
        }
        changed
    }

    /// A try of the choices as they are: the composer's next attempt that
    /// is none of the tries kept, or a developed change. Whether one was
    /// made; when not, `refused` says why.
    fn make(&mut self) -> bool {
        let offered: Vec<Phrase> = self.tries.iter().map(|t| t.phrase).collect();
        let req = self.request();
        let made = match (&self.from, self.change.rework(), self.change.develop()) {
            (None, _, _) => next_idea(&req, &offered).map(|(idea, at)| (idea.phrase(), at)),
            (Some((_, of)), Some(how), _) => {
                next_rework(of, how, &req, &offered).map(|(idea, at)| (idea.phrase(), at))
            }
            (Some((name, of)), None, Some(how)) => {
                let Some(p) = develop(of, how) else {
                    self.refused = Some(format!(
                        "{} leaves {name} as it is: try another change",
                        self.change.label()
                    ));
                    return false;
                };
                // One way: the same try again is the one kept.
                if let Some(i) = self.tries.iter().position(|t| t.phrase == p) {
                    self.shown = i;
                    self.refused = None;
                    return true;
                }
                Some((p, 0))
            }
            (Some(_), None, None) => None,
        };
        let Some((phrase, attempt)) = made else {
            self.refused = Some(format!(
                "Nothing new after {} tries: change a row",
                mpfree_compose::idea::TRIES
            ));
            return false;
        };
        self.attempt = attempt.wrapping_add(1);
        self.tries.push(Try {
            phrase,
            base: self.base(),
        });
        if self.tries.len() > KEPT {
            self.tries.remove(0);
        }
        self.shown = self.tries.len() - 1;
        self.refused = None;
        true
    }

    /// The try shown.
    fn try_shown(&self) -> Option<&Try> {
        self.tries.get(self.shown)
    }

    /// The stepper: the try before, or from the first the one after.
    fn step_try(&mut self) -> bool {
        if self.shown > 0 {
            self.shown -= 1;
        } else if self.shown + 1 < self.tries.len() {
            self.shown += 1;
        } else {
            return false;
        }
        self.refused = None;
        true
    }

    /// What the panel shows. `keep_as` is the name Keep would give the try
    /// shown, `key` the line on the key it is written for, `pick` the
    /// track's phrase's name.
    fn view(&self, keep_as: &str, key: &str, pick: &str) -> MakerView {
        let row = |name: RowName| {
            let (n, value, note) = match name {
                RowName::Kind => ("Kind", self.kind.label().to_string(), ""),
                RowName::Style => ("Style", STYLES[self.set.style].name.to_string(), ""),
                RowName::Bars => match BARS[self.set.bars] {
                    1 => ("Bars", "1 bar".to_string(), ""),
                    b => ("Bars", format!("{b} bars"), ""),
                },
                RowName::Activity => ("Activity", ACTIVITY[self.set.activity].0.to_string(), ""),
                RowName::Change => (
                    "Change",
                    self.change.label().to_string(),
                    self.change.note(),
                ),
            };
            MakerRow {
                name: n.to_string(),
                value,
                accent: name == RowName::Change,
                note: note.to_string(),
            }
        };
        let shown = self.try_shown();
        let n = self.shown + 1;
        let mut notes = Vec::new();
        match &self.from {
            None => {
                notes.push(key.to_string());
                notes.push(format!(
                    "Try again keeps the last {KEPT} while this is open."
                ));
            }
            Some(_) if self.reworkable => {
                notes.push("New notes, New rhythm, Up a step, Upside down and".to_string());
                notes.push("First half twice (the last three come out one way).".to_string());
            }
            Some(_) => {
                notes.push("Not 4/4 on a plain grid, or past four bars: it can".to_string());
                notes.push("be developed, not given new notes or a rhythm.".to_string());
            }
        }
        if shown.is_some() {
            notes.push(format!("Keep puts it on Yours as {keep_as}."));
        }
        let (status, status_warn) = match (&self.refused, shown) {
            (Some(why), _) => (why.clone(), true),
            (None, Some(_)) => (
                format!("Try {n} · hold a pad: it plays from that note"),
                false,
            ),
            (None, None) => (String::new(), false),
        };
        let len = self.tries.len();
        let step = if self.shown > 0 {
            (format!("‹ Try {}", self.shown), true)
        } else if self.shown + 1 < len {
            (format!("Try {} ›", self.shown + 2), true)
        } else {
            ("‹ Try".to_string(), false)
        };
        MakerView {
            title: match &self.from {
                None => format!("New idea · {}", self.kind.label()),
                Some(_) => "More like this".to_string(),
            },
            from: self.from.clone(),
            rows: self.rows().iter().map(|r| row(*r)).collect(),
            row: self.row,
            notes,
            status,
            status_warn,
            can_try: self.change.rework().is_some() || self.from.is_none(),
            step,
            can_keep: shown.is_some(),
            shown: shown.map(|t| (keep_as.to_string(), t.phrase)),
            head: if pick.is_empty() {
                format!("Try {n}")
            } else {
                format!("Try {n} · Back keeps {pick}")
            },
        }
    }
}

impl App {
    /// The try a held pad plays while New idea or More like this is open:
    /// what [`App::send_phrase`] sends in place of the track's pick.
    pub(super) fn maker_try(&self) -> Option<Phrase> {
        self.maker
            .as_ref()
            .and_then(|m| m.try_shown())
            .map(|t| t.phrase)
    }

    /// New idea or More like this may open: the panel shows the Phrase
    /// tab's list (not a kit track's).
    fn maker_may_open(&self) -> bool {
        self.view == View::Arp
            && self.maker.is_none()
            && self.arp_tab() == Mode::Phrase
            && !self.phrase_on_kit()
    }

    /// The key a try is written for, as a line: the arp's own scale, the
    /// pads' key, or none.
    fn maker_key_line(&self, minor: bool) -> String {
        let e = self.arps.engine_at(self.track);
        let mode = if minor { "minor" } else { "major" };
        match (e.settings().scale, e.key()) {
            (Some(s), _) => format!("Written in {mode} for the arp's scale, {}.", key_name(s)),
            (None, Some(s)) => format!("Written for the pads' key, {}.", key_name(s)),
            (None, None) => format!("Written in {mode}: the pads have no key."),
        }
    }

    fn open_maker(&mut self, from: Option<(String, Phrase)>, kind: PhraseKind) -> Vec<Effect> {
        self.maker_seed = self.maker_seed.wrapping_add(1);
        let reworkable = from.as_ref().is_some_and(|(_, p)| reworkable(p));
        let minor = match &from {
            Some((_, p)) if p.written == Written::Minor => true,
            Some((_, p)) if p.written == Written::Major => false,
            _ => minor_for(self.phrase_key()),
        };
        let mut m = Maker {
            change: if from.is_none() || reworkable {
                Change::NewNotes
            } else {
                Change::Up
            },
            from,
            reworkable,
            kind,
            set: self.maker_set,
            minor,
            seed: self.maker_seed,
            attempt: 0,
            tries: Vec::new(),
            shown: 0,
            row: 0,
            refused: None,
        };
        // A first try at once, as Add a part makes one.
        m.make();
        let title = match &m.from {
            None => format!("new idea ({})", m.kind.label()),
            Some((name, _)) => format!("more like {name}"),
        };
        self.maker = Some(m);
        let mut fx = vec![Effect::Note(format!("arp phrase: {title}"))];
        fx.extend(self.maker_changed());
        fx
    }

    /// New idea: the panel opens on the kind the list shows (Yours opens
    /// on Bass), with the choices last used.
    pub(super) fn open_new_idea(&mut self) -> Vec<Effect> {
        if !self.maker_may_open() {
            return Vec::new();
        }
        let kind = match self.shown_shelf() {
            Shelf::Kind(k) => k,
            Shelf::Own => PhraseKind::Bass,
        };
        self.open_maker(None, kind)
    }

    /// More like this: from the track's phrase.
    pub(super) fn open_more_like(&mut self) -> Vec<Effect> {
        if !self.maker_may_open() {
            return Vec::new();
        }
        let Some((name, phrase)) = self.phrases.get(self.phrase_index).cloned() else {
            return Vec::new();
        };
        let kind = phrase.kind;
        self.open_maker(Some((name, phrase)), kind)
    }

    /// After a try was made or shown: the engine plays it, and a line says
    /// what it is.
    fn maker_changed(&mut self) -> Vec<Effect> {
        self.send_phrase();
        let Some(m) = &self.maker else {
            return Vec::new();
        };
        match (&m.refused, m.try_shown()) {
            (Some(why), _) => vec![Effect::Note(format!("arp phrase: {why}"))],
            (None, Some(t)) => vec![Effect::Note(format!(
                "arp phrase try {}: {}",
                m.shown + 1,
                t.phrase.to_text()
            ))],
            (None, None) => Vec::new(),
        }
    }

    /// A touch on the panel.
    pub(super) fn maker_action(&mut self, a: MakerAction) -> Vec<Effect> {
        let Some(m) = self.maker.as_mut() else {
            return Vec::new();
        };
        match a {
            MakerAction::Step { row, delta } => {
                m.row = row.min(m.rows().len() - 1);
                self.maker_row_step(delta)
            }
            MakerAction::TryAgain => {
                if m.from.is_some() && m.change.rework().is_none() {
                    return Vec::new();
                }
                m.make();
                self.maker_changed()
            }
            MakerAction::Previous => {
                if m.step_try() {
                    self.maker_changed()
                } else {
                    Vec::new()
                }
            }
            MakerAction::Back => self.close_maker(),
            MakerAction::Keep => self.keep_try(),
        }
    }

    /// The row the wheel is on stepped by `delta` (- and +, or a
    /// stepper): a change makes a try.
    fn maker_row_step(&mut self, delta: i32) -> Vec<Effect> {
        let Some(m) = self.maker.as_mut() else {
            return Vec::new();
        };
        let row = m.row;
        if !m.step(row, delta) {
            return Vec::new();
        }
        self.maker_set = m.set;
        m.make();
        self.maker_changed()
    }

    /// The wheel while the panel is open: the row it is on.
    pub(super) fn maker_wheel(&mut self, delta: i32) -> Vec<Effect> {
        let Some(m) = self.maker.as_mut() else {
            return Vec::new();
        };
        let to = (m.row as i32 + delta).clamp(0, m.rows().len() as i32 - 1) as usize;
        if to != m.row {
            m.row = to;
            self.arp_dirty = true;
        }
        Vec::new()
    }

    /// - and + while the panel is open: the wheel's row.
    pub(super) fn maker_step(&mut self, d: i32) -> Vec<Effect> {
        self.maker_row_step(d)
    }

    /// Back, or anything that takes the panel away: it closes and the
    /// track's pick plays again. Nothing else changes.
    pub(in crate::app) fn close_maker(&mut self) -> Vec<Effect> {
        if self.maker.take().is_none() {
            return Vec::new();
        }
        self.send_phrase();
        vec![Effect::Note("arp phrase: back, nothing kept".to_string())]
    }

    /// Keep: the try joins Yours under a free name and the track picks it;
    /// with a store, the owner's phrases are saved.
    fn keep_try(&mut self) -> Vec<Effect> {
        let Some(t) = self.maker.as_ref().and_then(|m| m.try_shown()).cloned() else {
            return Vec::new();
        };
        let name = self.maker_keep_name(&t);
        self.maker = None;
        self.own_phrase(&name, t.phrase);
        let Some(i) = self.phrases.iter().position(|(n, _)| *n == name) else {
            return Vec::new();
        };
        let mut fx = vec![
            Effect::Phrases(self.phrase_list()),
            // A library line, so the report carries it home.
            Effect::Note(format!("arp-phrase-kept: {name} = {}", t.phrase.to_text())),
        ];
        fx.extend(self.pick_phrase(i));
        if self.pattern_file {
            fx.extend(self.phrases_text().map(Effect::SavePhrases));
        } else {
            fx.push(Effect::Note(
                "the phrase stays in memory until the power goes off (no --data folder to keep it in)"
                    .to_string(),
            ));
        }
        fx
    }

    /// The name Keep gives `t`: its base and the first number free in the
    /// list, from 1 for a new idea, from 2 for more like a phrase.
    fn maker_keep_name(&self, t: &Try) -> String {
        let first = match self.maker.as_ref().map(|m| &m.from) {
            Some(Some(_)) => 2,
            _ => 1,
        };
        free_name(&t.base, first, self.phrases.iter().map(|(n, _)| n.as_str()))
    }

    /// Keep as a phrase, on the Pattern tab (board G): the pattern as it
    /// plays on one held note ([`Phrase::from_pattern`], at the arp's rate
    /// and gate) joins Yours under the pattern's name (or the first free
    /// "Name 2"), and with a store the owner's phrases are saved. The
    /// track's pick and the tab stay as they are. Only for a pattern on one
    /// note ([`mpfree_arp::Pattern::on_one_note`]): one that picks the
    /// second held note or walks degrees would become one note over and
    /// over.
    pub(super) fn keep_pattern_as_phrase(&mut self) -> Vec<Effect> {
        if !self.pattern.on_one_note() {
            return Vec::new();
        }
        let name = keep_name(
            &self.pattern_name(),
            self.phrases.iter().map(|(n, _)| n.as_str()),
        );
        let phrase = Phrase::from_pattern(
            &self.pattern,
            self.settings.rate,
            4,
            self.arp.gate,
            PATTERN_VELOCITY,
            60,
        );
        self.own_phrase(&name, phrase);
        let mut fx = vec![
            Effect::Phrases(self.phrase_list()),
            Effect::Note(format!("arp-phrase-kept: {name} = {}", phrase.to_text())),
            Effect::Note(format!(
                "Keep as a phrase: {name} on one note, on the Phrase tab's Yours"
            )),
        ];
        if self.pattern_file {
            fx.extend(self.phrases_text().map(Effect::SavePhrases));
        }
        self.arp_dirty = true;
        fx
    }

    /// What the panel shows, while it is open ([`ArpView::maker`]).
    ///
    /// [`ArpView::maker`]: crate::app::ArpView::maker
    pub(super) fn maker_view(&self) -> Option<MakerView> {
        let m = self.maker.as_ref()?;
        let keep_as = m
            .try_shown()
            .map(|t| self.maker_keep_name(t))
            .unwrap_or_default();
        let pick = self
            .phrases
            .get(self.phrase_index)
            .map_or("", |(n, _)| n.as_str());
        Some(m.view(&keep_as, &self.maker_key_line(m.minor), pick))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_arp::scale::pads::{layout, PadLayout, PadSetup};
    use mpfree_arp_ui::view::ArpAction;

    /// The Arp panel open on its Phrase tab, the pads on a synth, the
    /// track on `pick`.
    fn on_the_phrase_tab(pick: &str) -> App {
        let mut app = App::new(44_100, false);
        app.start();
        app.set_pad_layout(Some(layout(&PadSetup {
            layout: PadLayout::Chromatic,
            ..PadSetup::default()
        })));
        app.toggle_panel();
        assert_eq!(app.view(), View::Arp);
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        let i = app
            .phrases()
            .iter()
            .position(|(n, _)| n == pick)
            .unwrap_or_else(|| panic!("no phrase {pick}"));
        app.pick_phrase(i);
        app
    }

    fn view(app: &App) -> MakerView {
        app.maker_view().expect("the panel is open")
    }

    /// The try shown, and the name Keep gives it.
    fn shown(app: &App) -> (String, Phrase) {
        view(app).shown.expect("a try")
    }

    fn act(app: &mut App, a: MakerAction) -> Vec<Effect> {
        app.arp_action(ArpAction::Maker(a))
    }

    /// New idea: the first try plays at once on the armed track, Try again
    /// makes another, the stepper goes back, a row's change makes a try
    /// of its own, and Keep puts the try on Yours under a free name, the
    /// track picks it, and the owner's phrases are saved.
    #[test]
    fn new_idea_tries_play_and_keep_joins_yours() {
        let mut app = on_the_phrase_tab("Night drive");
        let night = app.phrases()[app.phrase_index()].1;
        app.pattern_file = true;
        let fx = app.arp_action(ArpAction::NewIdea);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Note(n) if n.starts_with("arp phrase try 1: bass"))),
            "{fx:?}"
        );
        let v = view(&app);
        assert_eq!(v.title, "New idea · Bass", "the kind the list shows");
        let rows: Vec<(&str, &str)> = v
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Kind", "Bass"),
                ("Style", "Groove"),
                ("Bars", "2 bars"),
                ("Activity", "Middle")
            ]
        );
        assert!(v.status.starts_with("Try 1 · hold a pad"), "{}", v.status);
        assert_eq!(v.head, "Try 1 · Back keeps Night drive");
        assert_eq!(v.step, ("‹ Try".to_string(), false));
        assert!(v.can_try && v.can_keep);
        assert!(v.notes[0].starts_with("Written in minor"), "{:?}", v.notes);
        let (name, first) = shown(&app);
        assert_eq!(name, "Groove bass 1");
        assert_eq!(first.kind, PhraseKind::Bass);
        assert_eq!(first.bars, 2);
        assert_eq!(app.eng().phrase(), &first, "a held pad plays the try");
        assert_ne!(first, night);

        act(&mut app, MakerAction::TryAgain);
        let (_, second) = shown(&app);
        assert_ne!(second, first);
        assert_eq!(app.eng().phrase(), &second);
        assert_eq!(view(&app).step, ("‹ Try 1".to_string(), true));
        act(&mut app, MakerAction::Previous);
        assert_eq!(shown(&app).1, first);
        assert_eq!(app.eng().phrase(), &first);
        assert_eq!(view(&app).step, ("Try 2 ›".to_string(), true));

        // The wheel picks the Style row; + changes it and makes a try.
        app.wheel(1);
        assert_eq!(view(&app).row, 1);
        app.arp_mode_step(1);
        let v = view(&app);
        assert_eq!(v.rows[1].value, "Jazz-ish");
        assert!(v.status.starts_with("Try 3"), "{}", v.status);
        let (name, third) = shown(&app);
        assert_eq!(name, "Jazz-ish bass 1");
        // A row at its end changes nothing.
        assert!(app.arp_mode_step(1).is_empty());

        let fx = act(&mut app, MakerAction::Keep);
        assert!(app.maker_view().is_none(), "Keep closes it");
        assert_eq!(app.phrases().len(), 65);
        assert_eq!(app.phrases()[64], ("Jazz-ish bass 1".to_string(), third));
        assert_eq!(app.phrase_index(), 64, "the track picks it");
        assert_eq!(app.eng().phrase(), &third);
        assert!(fx.contains(&Effect::Phrases(app.phrase_list())), "{fx:?}");
        assert!(fx.contains(&Effect::Note(format!(
            "arp-phrase-kept: Jazz-ish bass 1 = {}",
            third.to_text()
        ))));
        let saved = fx.iter().find_map(|e| match e {
            Effect::SavePhrases(t) => Some(t.clone()),
            _ => None,
        });
        let saved = mpfree_arp::Library::parse(&saved.expect("saved")).unwrap();
        assert_eq!(saved.phrase("Jazz-ish bass 1"), Some(&third));

        // From Yours, New idea opens on Bass with the choices last used,
        // and the next name is free.
        app.arp_action(ArpAction::NewIdea);
        let v = view(&app);
        assert_eq!(v.title, "New idea · Bass");
        assert_eq!(v.rows[1].value, "Jazz-ish");
        assert_eq!(shown(&app).0, "Jazz-ish bass 2");
        assert_ne!(shown(&app).1, third, "each opening asks anew");
    }

    /// Back, ×, and anything that takes the tab away close the panel and
    /// change nothing: the track's pick plays again.
    #[test]
    fn back_and_leaving_change_nothing() {
        let mut app = on_the_phrase_tab("Night drive");
        let night = app.phrases()[app.phrase_index()].1;
        let at = app.phrase_index();
        let open = |app: &mut App| {
            app.arp_action(ArpAction::MoreLike);
            assert_ne!(app.eng().phrase(), &night, "the try plays");
        };
        let unchanged = |app: &App| {
            assert!(app.maker_view().is_none());
            assert_eq!(app.phrase_index(), at);
            assert_eq!(app.eng().phrase(), &night);
            assert_eq!(app.phrases().len(), 64);
        };
        open(&mut app);
        assert_eq!(
            act(&mut app, MakerAction::Back),
            vec![Effect::Note("arp phrase: back, nothing kept".into())]
        );
        unchanged(&app);
        // Another track, and back.
        open(&mut app);
        app.set_track(1);
        app.set_track(0);
        unchanged(&app);
        // Another tab.
        open(&mut app);
        app.arp_action(ArpAction::Mode(Mode::Pattern));
        app.arp_action(ArpAction::Mode(Mode::Phrase));
        unchanged(&app);
        // Another page.
        open(&mut app);
        app.toggle_panel();
        assert_ne!(app.view(), View::Arp);
        app.toggle_panel();
        unchanged(&app);
        // A kind tapped: it closes and the kind shows.
        open(&mut app);
        app.arp_action(ArpAction::Shelf(Shelf::Own));
        assert!(app.maker_view().is_none());
        assert_eq!(app.shown_shelf(), Shelf::Own);
        // Only from the Phrase tab's list.
        app.arp_action(ArpAction::Mode(Mode::Arp));
        app.arp_action(ArpAction::NewIdea);
        assert!(app.maker_view().is_none());
    }

    /// Keep as a phrase on the Pattern tab: Gallop on one note joins Yours
    /// under its name (then "Gallop 2"), the pick stays, and a pattern
    /// that needs a second held note is refused.
    #[test]
    fn keep_as_a_phrase_puts_a_pattern_on_one_note_on_yours() {
        use mpfree_arp_ui::view::PatternButton;
        let mut app = on_the_phrase_tab("Night drive");
        let pick = app.phrase_index();
        app.pattern_file = true;
        app.arp_action(ArpAction::Mode(Mode::Pattern));
        let at = |name: &str| {
            crate::app::built_in_patterns()
                .iter()
                .position(|(n, _)| n == name)
                .unwrap()
        };
        app.arp_action(ArpAction::Pattern(at("Gallop")));
        let fx = app.arp_action(ArpAction::Button(PatternButton::KeepPhrase));
        assert_eq!(app.phrases().len(), 65);
        let (name, p) = &app.phrases()[64];
        assert_eq!(name, "Gallop");
        // x x . x at 1/16, repeated to a bar: three of every four 1/16s.
        assert_eq!(p.notes().len(), 12);
        assert!(p.notes().iter().all(|n| n.semis == 0));
        assert_eq!(app.phrase_index(), pick, "the pick stays");
        assert!(fx.contains(&Effect::Phrases(app.phrase_list())));
        assert!(fx
            .iter()
            .any(|e| matches!(e, Effect::SavePhrases(t) if t.contains("Gallop ="))));
        app.arp_action(ArpAction::Button(PatternButton::KeepPhrase));
        assert_eq!(app.phrases()[65].0, "Gallop 2");
        // Bounce picks the third held note: nothing.
        app.arp_action(ArpAction::Pattern(at("Bounce")));
        assert!(app
            .arp_action(ArpAction::Button(PatternButton::KeepPhrase))
            .is_empty());
        assert_eq!(app.phrases().len(), 66);
    }

    /// More like this: the changes the phrase can take, Try again only
    /// where a change has more than one way, and Keep's name after the
    /// phrase's.
    #[test]
    fn more_like_this_offers_what_the_phrase_can_take() {
        let mut app = on_the_phrase_tab("Night drive");
        app.arp_action(ArpAction::MoreLike);
        let v = view(&app);
        assert_eq!(v.title, "More like this");
        assert_eq!(
            v.from.as_ref().map(|(n, _)| n.as_str()),
            Some("Night drive")
        );
        let names: Vec<&str> = v.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Change", "Style"]);
        assert_eq!(v.rows[0].value, "New notes");
        assert_eq!(v.rows[0].note, "Its rhythm, with new notes");
        assert_eq!(shown(&app).0, "Night drive 2");
        // New rhythm, then Up a step: one way, no style.
        app.arp_mode_step(1);
        assert_eq!(view(&app).rows[0].value, "New rhythm");
        app.arp_mode_step(1);
        let v = view(&app);
        assert_eq!(v.rows[0].value, "Up a step");
        assert_eq!(v.rows.len(), 1);
        assert!(!v.can_try);
        let tries = v.status.clone();
        assert!(act(&mut app, MakerAction::TryAgain).is_empty());
        assert_eq!(view(&app).status, tries);

        // The owner's file read meanwhile: the try still plays.
        let up = shown(&app).1;
        app.load_phrases("Waltz = riff minor 3/4 : 1 . 3 5 . 3\n");
        assert_eq!(app.eng().phrase(), &up);

        // A phrase in three can only be developed.
        act(&mut app, MakerAction::Back);
        let i = app
            .phrases()
            .iter()
            .position(|(n, _)| n == "Waltz")
            .unwrap();
        app.pick_phrase(i);
        app.arp_action(ArpAction::MoreLike);
        let v = view(&app);
        assert_eq!(v.rows[0].value, "Up a step");
        assert!(v.notes[0].starts_with("Not 4/4"), "{:?}", v.notes);
        assert!(app.arp_mode_step(-1).is_empty(), "no rework to step to");
        assert_eq!(shown(&app).0, "Waltz 2");
    }

    #[test]
    fn names_are_free_and_fit() {
        let names = ["Groove bass 1", "night drive 2"];
        assert_eq!(
            free_name("Groove bass", 1, names.iter().copied()),
            "Groove bass 2"
        );
        assert_eq!(
            free_name("Night drive", 2, names.iter().copied()),
            "Night drive 3"
        );
        assert_eq!(base_of("Night drive 2"), "Night drive");
        assert_eq!(base_of("808"), "808");
        assert_eq!(base_of("Dub steps"), "Dub steps");
        let long = "A very long name for a phrase of mine";
        let n = free_name(long, 2, std::iter::empty());
        assert!(n.chars().count() <= mpfree_arp::library::MAX_NAME, "{n}");
        assert!(n.ends_with(" 2"), "{n}");
    }

    #[test]
    fn the_key_says_minor_or_major() {
        let minor = mpfree_arp::scale::by_name("Minor").map(|d| Scale::of(9, d));
        let major = mpfree_arp::scale::by_name("Major").map(|d| Scale::of(0, d));
        assert!(minor_for(None));
        assert!(minor_for(minor));
        assert!(!minor_for(major));
    }
}

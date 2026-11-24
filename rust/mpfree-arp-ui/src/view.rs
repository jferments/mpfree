//! The arp's two screens: what they show, worked out from the engine, and
//! what their touches mean.
//!
//! - The **Arp panel** shows every setting of one track's arp beside the
//!   pads: mode tabs, the list the wheel moves through, the Q-Link sliders or
//!   the pattern buttons, and the step strip while the arp is generating.
//!   Its fifth tab, Phrase, is three panels of its own in the pads' place
//!   too ([`crate::phrase_tab`]).
//! - The **quick band** is a slim strip at the top of the right-hand column
//!   while NOTE REPEAT is held, or while notes are latched: what the arp
//!   plays, the rate with − and +, and Latch. The rest of the column stays
//!   the app's.
//!
//! The arp is a way of playing the pads, not a place, so neither is a
//! top-level tab: the app shows the panel and the band in its Play page's
//! right-hand column, so the pads never move, and puts [`chip`] in its
//! header. The
//! design is in the arpeggiator design doc (`docs/dev/design/arpeggiator.md`).
//!
//! The screens do not own the engine. The app copies what they need into an
//! [`ArpState`] (from [`ArpState::from_engine`]), calls [`ArpPanel::update`]
//! when it changes, and turns their [`ArpAction`]s into engine calls. The
//! pads, the header and the hint line belong to the app; the panel only adds
//! its own widgets and the layouts that place them.

use crate::maker::{
    MakerView, PhraseMaker, MAKER_BACK, MAKER_CLOSE, MAKER_KEEP, MAKER_STEP, MAKER_TRY,
};
use crate::phrase_tab::{self, PhraseKinds, PhraseKit, PhrasePlays, PhraseRows};
use crate::phrases::{PhraseList, Shelf};
use crate::widgets::{
    Button, ButtonRow, CellKind, Mark, MiniBar, ModBox, PatternList, PatternRow, Segments, Slider,
    SliderStack, StepCell, StepStrip, MARKS, MINI_FASTER, MINI_LATCH, MINI_OPEN, MINI_SLOWER,
    STRIP_EMPTY,
};
use mpfree_arp::{
    Engine, Feel, Follow, Held, Join, ModSource, Mode, Order, Pattern, Pick, Rate, Scale, Settings,
    Speed, Step, MAX_NOTES,
};
use mpfree_ui::widgets::{Chip, ListItem, ListView};
use mpfree_ui::{EventKind, Tone, Ui, UiEvent, Widget};

/// Widget names the page adds. The app's own are `header`, `pads` and `hint`.
pub const MODES: &str = "arp-modes";
pub const LIST: &str = "arp-list";
/// The Pattern mode's list: the library's groups as tabs, a picture of each
/// pattern's rhythm.
pub const PATTERNS: &str = "arp-patterns";
pub const QLINKS: &str = "arp-qlinks";
pub const ACTIONS: &str = "arp-actions";
pub const STEPS: &str = "arp-steps";
pub const MINI: &str = "arp-mini";
/// The Modifier box beside the mode tabs ([`with_modifier`]).
pub const MODBOX: &str = "arp-mod";
/// Its width.
pub const MODBOX_W: i32 = 340;
/// The Phrase tab's panels ([`crate::phrase_tab`]): the kinds, the shown
/// kind's phrases, the phrase that plays and, on a kit track, the panel
/// that says why none does.
pub const KINDS: &str = "arp-phrase-kinds";
pub const PHRASES: &str = "arp-phrase-list";
pub const PLAYS: &str = "arp-phrase-plays";
pub const KIT: &str = "arp-phrase-kit";
pub use crate::maker::MAKER;

/// Every layout but the Phrase tab's keeps the pads where the Play page has
/// them: the arp's widgets only ever take the right-hand column, [`SIDE`]
/// pixels wide, so nothing under the player's fingers moves or resizes when
/// the band or the panel comes up.
///
/// Direct: nothing is generated, so there is no step strip.
pub const DIRECT_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-list:*, arp-qlinks:236):460):*, hint:26)";
/// Repeat and Arp: the list (rates or orders) over the Q-Link sliders, and
/// the step strip.
pub const ARP_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-list:*, arp-qlinks:236, arp-steps:112):460):*, hint:26)";
/// Pattern: the pattern list ([`PATTERNS`], its groups over it) over the
/// pattern buttons, and the step strip.
pub const PATTERN_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-patterns:*, arp-actions:128, arp-steps:112):460):*, hint:26)";
/// Phrase: the kinds, the shown kind's phrases and the phrase that plays,
/// side by side under the tabs. A list of phrases with a picture of each
/// and the one that plays, large, do not fit a 460-pixel column, so this
/// tab takes the pads' room as well: the one layout here without them. (The
/// app's Play frame gives every tab the whole width, and there the pads
/// are not on the panel at all.)
pub const PHRASE_LAYOUT: &str =
    "col(header:60, col(arp-modes:52, row(arp-phrase-kinds:166, arp-phrase-list:*, arp-phrase-plays:397):*):*, hint:26)";
/// Phrase with New idea or More like this open: their panel
/// ([`crate::maker`]) in the list's place.
pub const PHRASE_MAKER_LAYOUT: &str =
    "col(header:60, col(arp-modes:52, row(arp-phrase-kinds:166, arp-phrase-maker:*, arp-phrase-plays:397):*):*, hint:26)";
/// Phrase on a kit track, whose pads play as pressed: the kinds, and one
/// panel that says so in the place of the other two.
pub const PHRASE_KIT_LAYOUT: &str =
    "col(header:60, col(arp-modes:52, row(arp-phrase-kinds:166, arp-phrase-kit:*):*):*, hint:26)";
/// The quick band: a slim strip ([`MINI`], [`MINI_H`] pixels) at the top of
/// the right-hand column. An app puts it over its own column with
/// [`ArpPanel::set_quick_layout`] (the pad player keeps its kits, level and
/// timing under it); this default puts the next steps under it.
pub const QUICK_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-mini:56, arp-steps:112):460):*, hint:26)";
/// The strip's height in the layouts.
pub const MINI_H: i32 = 56;
/// The width of the right-hand column every layout here shares with the
/// app's Play page.
pub const SIDE: i32 = 460;

/// `layout` with the Modifier box ([`MODBOX`], [`MODBOX_W`] pixels) at the
/// right of the mode tabs' row, for a panel as wide as the app's Play page
/// gives it (the tabs keep the rest). The layouts above leave it out: the
/// 460-pixel column has no room beside the tabs.
pub fn with_modifier(layout: &str) -> String {
    layout.replacen(
        &format!("{MODES}:52"),
        &format!("row({MODES}:*, {MODBOX}:{MODBOX_W}):52"),
        1,
    )
}

/// The mode tabs, in [`Mode`] order.
pub const MODE_TABS: [&str; 5] = ["Direct", "Repeat", "Arp", "Pattern", "Phrase"];
/// The Q-Links, top to bottom, as Jay's plan maps them.
pub const QLINK_LABELS: [&str; 4] = ["Rate · Q1", "Gate · Q2", "Swing · Q3", "Octaves · Q4"];
/// The Q-Links on the Phrase tab: a phrase has its own rhythm, so the first
/// sets how fast it plays against it, not a rate.
pub const PHRASE_QLINK_LABELS: [&str; 4] =
    ["Speed · Q1", "Gate · Q2", "Swing · Q3", "Octaves · Q4"];
/// The Swing slider's label while the phrase is written in triplets, which
/// swing leaves as they are: its value reads [`STRAIGHT`].
pub const SWING_TRIPLETS: &str = "Swing · Q3 · triplets";
pub const STRAIGHT: &str = "straight";
/// The pattern buttons, in [`PatternButton`] order.
pub const PATTERN_BUTTONS: [(&str, Tone); 4] = [
    ("Edit pattern", Tone::Accent),
    ("Record on pads", Tone::Bad),
    ("Mutate", Tone::Normal),
    ("Original", Tone::Normal),
];
/// The most cells the step strip shows at once.
pub const STRIP_CELLS: usize = 16;

const MODES_ALL: [Mode; 5] = [
    Mode::Direct,
    Mode::Repeat,
    Mode::Arp,
    Mode::Pattern,
    Mode::Phrase,
];

/// What the Phrase tab shows beyond the list and the track's pick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhraseNow {
    /// The kind the list shows when it is not the pick's: an empty one the
    /// owner stepped to (their own, before they have made a phrase).
    pub shelf: Option<Shelf>,
    /// Where the phrase is as it plays, 0.0 to 1.0 of the way through.
    pub at: Option<f32>,
    /// The pick waits for the bar line while another phrase plays on: the
    /// transport is playing ([`Engine::phrase_waiting`]).
    pub waits: bool,
    /// The phrase that plays until then, as a place in the list.
    pub playing: Option<usize>,
    /// The track's name while the tab shows on a kit track, whose pads
    /// play as pressed: the tab says so in the place of the list.
    pub kit_track: Option<String>,
}

/// Everything the page shows.
#[derive(Clone, Debug)]
pub struct ArpState {
    pub settings: Settings,
    /// What the arp plays now: the settings as the Modifier pushes them
    /// ([`Engine::live`]); the same as [`ArpState::settings`] unpushed.
    pub live: Settings,
    /// How far the Modifier's controller is now, 0.0 to 1.0.
    pub push: f32,
    pub pattern: Pattern,
    /// The patterns to choose from, by name, and which one is loaded.
    pub patterns: Vec<(String, Pattern)>,
    pub pattern_index: usize,
    /// The loaded pattern was changed (mutated, edited or recorded) from the
    /// one in the list: Original puts it back.
    pub pattern_changed: bool,
    /// The phrases to choose from, and the track's pick.
    pub phrases: PhraseList,
    pub phrase_index: usize,
    /// New idea or More like this, while one is open: its panel takes the
    /// list's place and the detail panel shows its try.
    pub maker: Option<MakerView>,
    /// The scale the pads play: the key a phrase follows In key while the
    /// arp's own scale is off ([`ArpState::phrase_key`]).
    pub pads_key: Option<Scale>,
    /// What the Phrase tab shows of the phrase as it plays.
    pub phrase: PhraseNow,
    /// The notes the generator plays, in the order they were pressed.
    pub notes: Vec<Held>,
    /// Steps played since the notes started ([`Engine::step`]).
    pub step: u64,
    pub latched: bool,
    pub bpm: f64,
}

impl ArpState {
    /// What the engine shows now. The pattern list and its state are the
    /// app's, so they are passed in; the phrases are MPFree's own, on the
    /// first, until the app sends its list and the track's pick.
    pub fn from_engine(
        e: &Engine,
        patterns: Vec<(String, Pattern)>,
        pattern_index: usize,
        pattern_changed: bool,
    ) -> ArpState {
        ArpState {
            settings: e.settings(),
            live: e.live(),
            push: e.modifier(),
            pattern: *e.pattern(),
            patterns,
            pattern_index,
            pattern_changed,
            phrases: PhraseList::built_in(),
            phrase_index: 0,
            maker: None,
            pads_key: e.key(),
            phrase: PhraseNow {
                at: e
                    .phrase_at()
                    .map(|t| (t / f64::from(e.phrase().ticks().max(1))) as f32),
                waits: e.phrase_waiting() == Some(Join::Bar),
                ..PhraseNow::default()
            },
            notes: e.active().as_slice().to_vec(),
            step: e.step(),
            latched: e.latch() != mpfree_arp::LatchMode::Off,
            bpm: e.bpm(),
        }
    }

    /// The name of the track's phrase.
    pub fn phrase_name(&self) -> &str {
        self.phrases.name(self.phrase_index)
    }

    /// The key a phrase follows In key now: the arp's own scale, or with
    /// that off the scale the pads play. None: it moves by semitones.
    pub fn phrase_key(&self) -> Option<Scale> {
        self.settings.scale.or(self.pads_key)
    }

    /// The tab the panel shows: the mode the pads play through, or Phrase
    /// on a kit track, where that tab is picked and the pads play as
    /// pressed all the same. The layout, the Q-Links and the hint follow
    /// the tab.
    pub fn tab(&self) -> Mode {
        if self.phrase.kit_track.is_some() {
            Mode::Phrase
        } else {
            self.settings.mode
        }
    }

    /// The kind the Phrase tab's list shows: the pick's, unless an empty
    /// one was stepped to ([`PhraseNow::shelf`]).
    pub fn shelf(&self) -> Shelf {
        self.phrase
            .shelf
            .unwrap_or_else(|| self.phrases.shelf_of(self.phrase_index))
    }

    /// The name the chip and the quick band give the phrase: the one that
    /// plays, which while a pick waits for the bar line is still the one
    /// before it.
    pub fn phrase_heard(&self) -> &str {
        let playing = self.phrase.playing.filter(|_| self.phrase.waits);
        self.phrases.name(playing.unwrap_or(self.phrase_index))
    }

    /// The notes the steps work from: the held notes (or, while they take
    /// turns, the one whose turn it is), sorted, grown into chords and scale
    /// runs as the engine grows them.
    fn sorted(&self) -> Vec<Held> {
        let mut v =
            match mpfree_arp::turn_of(&self.live, self.notes.len(), self.step.saturating_sub(1)) {
                Some(k) => vec![self.notes[k]],
                None => self.notes.clone(),
            };
        v.sort_by_key(|h| h.note);
        v.truncate(MAX_NOTES);
        let mut list = [Held::default(); MAX_NOTES];
        list[..v.len()].copy_from_slice(&v);
        let n = mpfree_arp::step_notes(&self.live, &mut list, v.len());
        list[..n].to_vec()
    }
}

/// What a touch on the page asks for. The app applies it to the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum ArpAction {
    Mode(Mode),
    Rate(Rate),
    Order(Order),
    /// Load pattern `i` of [`ArpState::patterns`].
    Pattern(usize),
    /// A Q-Link slider was touched or moved: the new settings.
    Settings(Settings),
    Button(PatternButton),
    /// A cell of the step strip was tapped: step `i` of the pattern.
    Step(usize),
    /// The band's Latch button: latch on, or off (clearing the notes).
    Latch,
    /// The strip's name was tapped: open the Arp panel.
    OpenPanel,
    /// The Modifier box was tapped: open its options.
    Modifier,
    /// A phrase of the list was tapped: the track picks phrase `i` of
    /// [`ArpState::phrases`].
    Phrase(usize),
    /// A kind was tapped on the Phrase tab: show its phrases.
    Shelf(Shelf),
    /// The list's New idea: its panel opens on the kind shown.
    NewIdea,
    /// More like this, on the phrase that plays: its panel opens.
    MoreLike,
    /// A touch on the New idea or More like this panel.
    Maker(MakerAction),
}

/// A touch on the New idea or More like this panel ([`crate::maker`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MakerAction {
    /// Row `row`'s ‹ (-1) or › (+1): its value changes, and a new try is
    /// made where the value changes what is made.
    Step { row: usize, delta: i32 },
    /// Another try with the same choices.
    TryAgain,
    /// ‹ Try N: the try before (from the first, Try N ›: the one after).
    Previous,
    /// Back, or ×: the panel closes and the track's phrase plays again.
    Back,
    /// The try shown joins Yours and the track picks it.
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternButton {
    Edit,
    Record,
    Mutate,
    Original,
}

/// A pad or MIDI note as the screen names it: pads by number (slot 0 is
/// pad 1), other notes by name.
pub fn note_label(note: u8) -> String {
    if note < 16 {
        format!("{}", note + 1)
    } else {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        format!(
            "{}{}",
            NAMES[usize::from(note % 12)],
            i32::from(note / 12) - 1
        )
    }
}

fn octave_sub(oct: i32) -> String {
    match oct {
        0 => String::new(),
        o if o > 0 => format!("oct +{o}"),
        o => format!("oct {o}"),
    }
}

/// The Q-Links' labels on a tab.
pub fn qlink_labels(tab: Mode) -> [&'static str; 4] {
    if tab == Mode::Phrase {
        PHRASE_QLINK_LABELS
    } else {
        QLINK_LABELS
    }
}

/// A phrase's speed on its slider: half at the left, as written in the
/// middle and double at the right, with what a Modifier's push on the rate
/// makes of it between or beyond them (a number of times: "1.5×").
pub fn speed_value(factor: f64) -> (f32, String) {
    let at = ((factor.max(1e-6).log2() + 1.0) / 2.0).clamp(0.0, 1.0) as f32;
    let named = Speed::ALL
        .iter()
        .find(|s| (s.factor() - factor).abs() < 1e-6);
    let text = match named {
        Some(s) => s.label().to_lowercase(),
        None => {
            let n = format!("{factor:.2}");
            format!("{}×", n.trim_end_matches('0').trim_end_matches('.'))
        }
    };
    (at, text)
}

/// The Q-Link sliders' positions, 0.0 to 1.0, and their texts, on a tab:
/// the first is the rate, or on the Phrase tab the phrase's speed.
pub fn qlink_values(s: &Settings, tab: Mode) -> [(f32, String); 4] {
    let first = if tab == Mode::Phrase {
        speed_value(s.speed.factor())
    } else {
        let rate = Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0);
        (
            rate as f32 / (Rate::ALL.len() - 1) as f32,
            s.rate.label().to_string(),
        )
    };
    [
        first,
        (f32::from(s.gate - 1) / 99.0, format!("{} %", s.gate)),
        (
            f32::from(s.swing - 50) / 25.0,
            if s.swing == 50 {
                "off".to_string()
            } else {
                format!("{} %", s.swing)
            },
        ),
        (f32::from(s.octaves - 1) / 3.0, format!("{}", s.octaves)),
    ]
}

/// The settings with Q-Link `i` moved to `v` (0.0 to 1.0) on a tab.
pub fn with_qlink(mut s: Settings, tab: Mode, i: usize, v: f32) -> Settings {
    let v = v.clamp(0.0, 1.0);
    let pick = |n: usize| ((v * (n - 1) as f32).round() as usize).min(n - 1);
    match i {
        0 if tab == Mode::Phrase => s.speed = Speed::ALL[pick(Speed::ALL.len())],
        0 => s.rate = Rate::ALL[pick(Rate::ALL.len())],
        1 => s.gate = 1 + pick(100) as u8,
        2 => s.swing = 50 + pick(26) as u8,
        3 => s.octaves = 1 + pick(4) as u8,
        _ => {}
    }
    s.clamped()
}

/// The Q-Link sliders as the panel shows them for `st`: each one's label,
/// where it is set, and where the Modifier pushes it now. On the Phrase
/// tab the first is the speed, which a push on the rate moves by the same
/// ratio ([`mpfree_arp::phrase_speed`]), and while the phrase is written in
/// triplets, which swing leaves alone, the Swing slider is quiet and says
/// so.
pub fn qlink_rows(st: &ArpState) -> [Slider; 4] {
    let tab = st.tab();
    let labels = qlink_labels(tab);
    let set = qlink_values(&st.settings, tab);
    let mut now = qlink_values(&st.live, tab);
    let phrase = (tab == Mode::Phrase)
        .then(|| st.phrases.phrases.get(st.phrase_index))
        .flatten();
    if tab == Mode::Phrase {
        now[0] = speed_value(mpfree_arp::phrase_speed(&st.settings, &st.live));
    }
    let triplets = phrase.is_some_and(|(_, p)| p.feel() == Feel::Triplet);
    let mut rows: [Slider; 4] = Default::default();
    for (i, ((row, (value, text)), now)) in rows.iter_mut().zip(set).zip(now).enumerate() {
        *row = Slider {
            label: labels[i].to_string(),
            pushed: (now.1 != text).then_some(now),
            value,
            text,
            quiet: false,
        };
    }
    if triplets {
        rows[2] = Slider {
            label: SWING_TRIPLETS.to_string(),
            text: STRAIGHT.to_string(),
            pushed: None,
            quiet: true,
            ..rows[2].clone()
        };
    }
    rows
}

/// A short picture of an order played over three notes, such as "1 2 3 2".
pub fn order_preview(order: Order) -> String {
    match order {
        Order::Chord => "1+2+3".to_string(),
        Order::Played => "as pressed".to_string(),
        _ => match order.cycle(3) {
            Some(n) => (0..n.min(6))
                .map(|c| format!("{}", order.place(3, c).unwrap_or(0) + 1))
                .collect::<Vec<_>>()
                .join(" "),
            None => "?".to_string(),
        },
    }
}

/// The step strip for the Arp and Repeat modes: the order's notes over the
/// held pads, with the octaves the Modifier pushes to. Returns the cells,
/// the first step's number and the playing cell. A phrase plays its own
/// notes, not steps of an order, so it has none.
pub fn arp_cells(st: &ArpState) -> (Vec<StepCell>, usize, Option<usize>) {
    let s = &st.live;
    let sorted = st.sorted();
    let n = sorted.len();
    if n == 0 || matches!(s.mode, Mode::Direct | Mode::Phrase) {
        return (Vec::new(), 0, None);
    }
    let all = |label: &str| StepCell {
        kind: CellKind::Chord,
        label: label.to_string(),
        ..StepCell::default()
    };
    if s.mode == Mode::Repeat || s.order == Order::Chord {
        let label = if n == 1 {
            note_label(sorted[0].note)
        } else {
            format!("all {n}")
        };
        let playing = (st.step > 0).then_some(0);
        return (vec![all(&label)], 0, playing);
    }
    // Each turn of the held chords starts the order again.
    let turns = mpfree_arp::turn_of(s, st.notes.len(), 0).is_some();
    let since = |step: u64| {
        (if turns {
            step % mpfree_arp::turn_steps(s)
        } else {
            step
        }) as usize
    };
    let grown = s.chord != mpfree_arp::Chord::Off || s.run || turns;
    let base: Vec<Held> = if s.order == Order::Played && !grown {
        st.notes.iter().take(MAX_NOTES).copied().collect()
    } else {
        sorted
    };
    let total = n * usize::from(s.octaves);
    let Some(cycle) = s.order.cycle(total) else {
        // The random orders: as many cells as notes, none known ahead.
        let cells = (0..total.min(STRIP_CELLS))
            .map(|_| StepCell {
                kind: CellKind::Random,
                label: "?".to_string(),
                ..StepCell::default()
            })
            .collect();
        let playing = (st.step > 0).then(|| since(st.step - 1) % total.min(STRIP_CELLS));
        return (cells, 0, playing);
    };
    let last = (st.step > 0).then(|| since(st.step - 1) % cycle);
    let first = last.map_or(0, |l| l / STRIP_CELLS * STRIP_CELLS);
    let cells = (first..cycle.min(first + STRIP_CELLS))
        .map(|c| {
            let i = s.order.place(total, c).unwrap_or(0);
            let h = base[i % n];
            StepCell {
                kind: CellKind::Note,
                label: note_label(h.note),
                sub: octave_sub((i / n) as i32),
                accent: false,
            }
        })
        .collect();
    (cells, first, last.map(|l| l - first))
}

/// The groups of the patterns `st` lists, each with its first pattern's
/// index ([`mpfree_arp::groups::of_list`]).
pub fn pattern_groups(st: &ArpState) -> Vec<(&'static str, usize)> {
    mpfree_arp::groups::of_list(st.patterns.iter().map(|(n, _)| n.as_str()))
}

/// The rhythm of a pattern's first steps (up to [`MARKS`]), as the Pattern
/// list draws it.
pub fn pattern_marks(p: &Pattern) -> Vec<Mark> {
    p.steps()
        .iter()
        .take(MARKS)
        .map(|s| match *s {
            Step::Rest => Mark::Rest,
            Step::Tie => Mark::Tie,
            Step::Play {
                pick,
                accent,
                ratchet,
                chance,
                ..
            } => Mark::Hit {
                hits: ratchet,
                accent,
                chord: pick == Pick::All,
                maybe: chance < 100,
            },
        })
        .collect()
}

/// One pattern step as a cell.
pub fn pattern_cell(step: Step) -> StepCell {
    match step {
        Step::Rest => StepCell {
            kind: CellKind::Rest,
            label: "·".into(),
            ..StepCell::default()
        },
        Step::Tie => StepCell {
            kind: CellKind::Tie,
            label: "—".into(),
            ..StepCell::default()
        },
        Step::Play {
            pick,
            octave,
            accent,
            ratchet,
            chance,
            ..
        } => {
            let (kind, label) = match pick {
                Pick::Next => (CellKind::Note, "x".to_string()),
                Pick::All => (CellKind::Chord, "all".to_string()),
                Pick::Index(i) => (CellKind::Note, format!("{i}")),
                Pick::Random => (CellKind::Random, "?".to_string()),
                Pick::Lowest => (CellKind::Note, "low".to_string()),
                Pick::Highest => (CellKind::Note, "high".to_string()),
            };
            let mut sub = Vec::new();
            if octave != 0 {
                sub.push(octave_sub(i32::from(octave)));
            }
            if ratchet > 1 {
                sub.push(format!("×{ratchet}"));
            }
            if chance < 100 {
                sub.push(format!("{chance}%"));
            }
            StepCell {
                kind,
                label,
                sub: sub.join(" "),
                accent,
            }
        }
    }
}

/// The step strip for the Pattern mode: the pattern's steps, a window of
/// [`STRIP_CELLS`] around the one playing.
pub fn pattern_cells(st: &ArpState) -> (Vec<StepCell>, usize, Option<usize>) {
    let steps = st.pattern.steps();
    if steps.is_empty() {
        return (Vec::new(), 0, None);
    }
    let last =
        (st.step > 0 && !st.notes.is_empty()).then(|| ((st.step - 1) as usize) % steps.len());
    let first = last.map_or(0, |l| l / STRIP_CELLS * STRIP_CELLS);
    let cells = steps[first..steps.len().min(first + STRIP_CELLS)]
        .iter()
        .map(|s| pattern_cell(*s))
        .collect();
    (cells, first, last.map(|l| l - first))
}

/// The arp chip: what the arp does, in one line, for the app's header on
/// every page. Tapping it opens the panel (the app's job). Off and latched
/// look different at a glance.
pub fn chip(st: &ArpState) -> Chip {
    let s = &st.settings;
    let what = match s.mode {
        Mode::Direct => {
            return if st.latched {
                Chip::new("Latch on", Tone::Warn)
            } else {
                Chip::new("Arp off", Tone::Dim)
            };
        }
        Mode::Repeat => format!("Repeat {}", s.rate.label()),
        Mode::Arp => format!("Arp: {} {}", s.order.label(), s.rate.label()),
        // The phrase heard: while a pick waits for its bar line, still the
        // one before it.
        Mode::Phrase => match st.phrase_heard() {
            "" => "Phrase".to_string(),
            name => format!("Phrase: {name}"),
        },
        Mode::Pattern => {
            let name = st
                .patterns
                .get(st.pattern_index)
                .map_or("pattern", |(n, _)| n.as_str());
            format!(
                "Pattern: {name}{} {}",
                if st.pattern_changed { "*" } else { "" },
                s.rate.label()
            )
        }
    };
    if st.latched {
        Chip::new(format!("{what}, latched"), Tone::Warn)
    } else {
        Chip::new(what, Tone::Accent)
    }
}

/// The header chips for the sample screens: the arp chip, then the tempo
/// at the right edge. (The header lays chips out from the right edge, so the
/// last one here is the leftmost.)
pub fn chips(st: &ArpState) -> Vec<Chip> {
    vec![
        Chip::new(format!("{:.1} BPM", st.bpm), Tone::Good),
        chip(st),
    ]
}

/// The hint line under the panel, for each tab.
pub fn hint(mode: Mode) -> &'static str {
    match mode {
        Mode::Direct => "Pads play as pressed · hold NOTE REPEAT to repeat · press it twice: latch",
        Mode::Repeat => "Hold or latch pads · Wheel: rate · Q-Links: rate, gate, swing, octaves",
        Mode::Arp => {
            "Hold or latch pads · Wheel: order · Q-Links: rate, gate, swing, octaves · TAP TEMPO"
        }
        Mode::Pattern => {
            "Patterns play the chord you hold: \"2\" is the 2nd lowest held pad · Wheel: pattern · SHIFT + wheel: group"
        }
        Mode::Phrase => PHRASE_HINT,
    }
}

/// What the step strip says while a phrase plays: it has no steps of an
/// order to show.
pub const PHRASE_STRIP: &str = "the phrase's notes are on the Arp panel's Phrase tab";

/// The Phrase tab's hint: what the wheel, - and + and the Q-Links do there.
pub const PHRASE_HINT: &str = "Wheel: phrase · − / +: kind · Q-Links: speed, gate, swing, octaves";

/// The Phrase tab's hint on a kit track, named as the tab's panel names it.
pub fn phrase_kit_hint(track: &str) -> String {
    format!("{track}'s instrument is a kit · phrases play on a synth · − / +: kind")
}

/// The hint line under the panel for `st`: its tab's ([`hint`]), or on a
/// kit track's Phrase tab why no phrase plays.
pub fn hint_for(st: &ArpState) -> String {
    match &st.phrase.kit_track {
        Some(track) => phrase_kit_hint(track),
        None if st.maker.is_some() && st.tab() == Mode::Phrase => MAKER_HINT.to_string(),
        None => hint(st.tab()).to_string(),
    }
}

/// The hint while New idea or More like this is open.
pub const MAKER_HINT: &str =
    "Wheel: a row · − / +: change it · Q-Links: speed, gate, swing, octaves · Keep: on Yours · Back: nothing changes";

/// The hint line under the quick band.
pub fn quick_hint(mode: Mode) -> &'static str {
    match mode {
        Mode::Pattern => "Tap a rate · Q-Links: rate, gate, swing, octaves · Wheel: pattern · SHIFT + wheel: group · let go: back to your page",
        Mode::Arp => "Tap a rate · Q-Links: rate, gate, swing, octaves · Wheel: order · let go: back to your page",
        Mode::Phrase => "Tap a speed · Q-Links: speed, gate, swing, octaves · Wheel: phrase · let go: back to your page",
        _ => "Tap a rate while you play · press harder for louder · let go: back to your page",
    }
}

/// The panel's layout for a tab (the Phrase tab's as a synth track has
/// it; [`layout_of`] knows a kit track's).
pub fn layout_for(mode: Mode) -> &'static str {
    match mode {
        Mode::Direct => DIRECT_LAYOUT,
        Mode::Repeat | Mode::Arp => ARP_LAYOUT,
        Mode::Pattern => PATTERN_LAYOUT,
        Mode::Phrase => PHRASE_LAYOUT,
    }
}

/// The panel's layout for `st`: its tab's, and on a kit track's Phrase
/// tab the one with the panel that says why no phrase plays.
pub fn layout_of(st: &ArpState) -> &'static str {
    if st.phrase.kit_track.is_some() {
        PHRASE_KIT_LAYOUT
    } else if st.maker.is_some() && st.tab() == Mode::Phrase {
        PHRASE_MAKER_LAYOUT
    } else {
        layout_for(st.tab())
    }
}

/// The quick band's speed, short enough for its room between − and +.
fn speed_short(factor: f64) -> String {
    match Speed::ALL
        .iter()
        .find(|s| (s.factor() - factor).abs() < 1e-6)
    {
        Some(Speed::Half) => "½×".to_string(),
        Some(Speed::AsWritten) => "1×".to_string(),
        Some(Speed::Double) => "2×".to_string(),
        None => speed_value(factor).1,
    }
}

/// `new` goes into widget `name` when it shows something else. A widget
/// reached to be changed is drawn again whether it changed or not, so
/// one that shows the same is left alone: the panel hears from the app
/// every time a step plays or a phrase's cursor moves.
fn show<W: Widget + PartialEq>(ui: &mut Ui, name: &str, new: W) {
    if ui.get::<W>(name).is_some_and(|w| *w != new) {
        if let Some(w) = ui.get_mut::<W>(name) {
            *w = new;
        }
    }
}

/// The arp panel's and the quick band's widgets and their meaning. It keeps
/// nothing but the layout it last used, so the app can rebuild it at any
/// time.
pub struct ArpPanel {
    /// The page name the panel's layouts are added under.
    pub name: String,
    /// The page name of the quick band.
    pub quick: String,
    layout: Option<&'static str>,
    /// The quick band's layout ([`QUICK_LAYOUT`] unless the app gave one).
    quick_layout: String,
    /// Changes every layout before it is added, for an app that puts a bar
    /// of its own on its pages (such as the time engine's transport bar).
    wrap: Option<fn(&str) -> String>,
}

impl ArpPanel {
    /// Adds the widgets to `ui` (the app adds `header`, `pads` and `hint`
    /// first), the panel's page `name` laid out for `st`, and the quick
    /// band's page `quick`.
    pub fn add(ui: &mut Ui, name: &str, quick: &str, st: &ArpState) -> Result<ArpPanel, String> {
        ui.add(MODES, Segments::new(&MODE_TABS));
        ui.add(LIST, ListView::new("Order · Wheel"));
        ui.add(PATTERNS, PatternList::default());
        ui.add(QLINKS, SliderStack::new("Q-Links 1–4", &QLINK_LABELS));
        ui.add(ACTIONS, ButtonRow::new(&PATTERN_BUTTONS));
        ui.add(STEPS, StepStrip::new(8));
        ui.add(MINI, MiniBar::default());
        ui.add(MODBOX, ModBox::default());
        ui.add(KINDS, PhraseKinds::default());
        ui.add(PHRASES, PhraseRows::default());
        ui.add(PLAYS, PhrasePlays::new(&PHRASE_QLINK_LABELS));
        ui.add(KIT, PhraseKit::default());
        ui.add(MAKER, PhraseMaker::default());
        ui.add_page(quick, QUICK_LAYOUT)?;
        let mut panel = ArpPanel {
            name: name.to_string(),
            quick: quick.to_string(),
            layout: None,
            quick_layout: QUICK_LAYOUT.to_string(),
            wrap: None,
        };
        panel.update(ui, st)?;
        Ok(panel)
    }

    fn spec(&self, layout: &str) -> String {
        self.wrap.map_or_else(|| layout.to_string(), |f| f(layout))
    }

    /// Lays both pages out again with `wrap` applied to every layout, now
    /// and whenever the panel's layout changes. The widgets `wrap` adds must
    /// be in `ui` already.
    pub fn set_layout_wrap(&mut self, ui: &mut Ui, wrap: fn(&str) -> String) -> Result<(), String> {
        self.wrap = Some(wrap);
        ui.add_page(&self.quick, &self.spec(&self.quick_layout))?;
        if let Some(layout) = self.layout {
            ui.add_page(&self.name, &self.spec(layout))?;
        }
        Ok(())
    }

    /// The quick band's own layout: an app puts [`MINI`] at the top of the
    /// column its Play page already has, so only that column's first widget
    /// gives up [`MINI_H`] pixels and nothing else moves.
    pub fn set_quick_layout(&mut self, ui: &mut Ui, layout: &str) -> Result<(), String> {
        ui.add_page(&self.quick, &self.spec(layout))?;
        self.quick_layout = layout.to_string();
        Ok(())
    }

    /// Shows `st` on both. The panel's layout follows the tab
    /// ([`layout_of`]). A widget that shows the same as before is left
    /// alone, so it is not drawn again ([`show`]).
    pub fn update(&mut self, ui: &mut Ui, st: &ArpState) -> Result<(), String> {
        let s = &st.settings;
        let tab = st.tab();
        let layout = layout_of(st);
        if self.layout != Some(layout) {
            ui.add_page(&self.name, &self.spec(layout))?;
            self.layout = Some(layout);
        }
        show(ui, MINI, mini_bar(st));
        let active = MODES_ALL.iter().position(|x| *x == tab).unwrap_or(0);
        if ui
            .get::<Segments>(MODES)
            .is_some_and(|m| m.active != active)
        {
            if let Some(m) = ui.get_mut::<Segments>(MODES) {
                m.active = active;
            }
        }
        if let Some(list) = ui.get_mut::<ListView>(LIST) {
            match s.mode {
                Mode::Direct | Mode::Repeat | Mode::Phrase => {
                    list.title = "Rate · Wheel".into();
                    list.items = Rate::ALL
                        .iter()
                        .map(|r| ListItem {
                            text: r.label().into(),
                            detail: rate_detail(*r).into(),
                            color: None,
                        })
                        .collect();
                    list.selected = Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0);
                }
                Mode::Arp => {
                    list.title = "Order · Wheel".into();
                    list.items = Order::ALL
                        .iter()
                        .map(|o| ListItem {
                            text: o.label().into(),
                            detail: order_preview(*o),
                            color: None,
                        })
                        .collect();
                    list.selected = Order::ALL.iter().position(|o| *o == s.order).unwrap_or(0);
                }
                // The patterns have their own list ([`PATTERNS`]).
                Mode::Pattern => {}
            }
        }
        if let Some(list) = ui.get_mut::<PatternList>(PATTERNS) {
            if list.rows.len() != st.patterns.len()
                || list
                    .rows
                    .iter()
                    .zip(&st.patterns)
                    .any(|(r, (n, p))| r.name != *n || r.text != p.to_text())
            {
                list.rows = st
                    .patterns
                    .iter()
                    .map(|(name, p)| PatternRow {
                        name: name.clone(),
                        marks: pattern_marks(p),
                        steps: p.len(),
                        text: p.to_text(),
                    })
                    .collect();
                list.groups = pattern_groups(st)
                    .into_iter()
                    .map(|(g, at)| (g.to_string(), at))
                    .collect();
            }
            list.selected = st.pattern_index.min(st.patterns.len().saturating_sub(1));
        }
        // The Q-Link sliders. The Phrase tab draws its own in the panel
        // of the phrase that plays; the stack here gets the same rows on
        // every tab, for an app that reads the Q-Links from it.
        let rows = qlink_rows(st);
        if ui
            .get::<SliderStack>(QLINKS)
            .is_some_and(|q| q.rows != rows)
        {
            if let Some(q) = ui.get_mut::<SliderStack>(QLINKS) {
                q.rows = rows.to_vec();
            }
        }
        let m = s.modifier;
        let (source, amount) = if m.source == ModSource::Off {
            (String::new(), 0.0)
        } else {
            (m.source.label().to_string(), st.push)
        };
        show(ui, MODBOX, ModBox { source, amount });
        if let Some(b) = ui.get_mut::<ButtonRow>(ACTIONS) {
            b.buttons[3] = Button {
                disabled: !st.pattern_changed,
                ..b.buttons[3].clone()
            };
        }
        let (cells, first, playing) = if s.mode == Mode::Pattern {
            pattern_cells(st)
        } else {
            arp_cells(st)
        };
        if let Some(strip) = ui.get_mut::<StepStrip>(STEPS) {
            strip.cells = cells;
            strip.first = first;
            strip.playing = playing;
            strip.min_cells = if s.mode == Mode::Pattern {
                8
            } else {
                STRIP_CELLS
            };
            // A phrase plays its own notes, which its tab draws.
            strip.empty = if s.mode == Mode::Phrase {
                PHRASE_STRIP
            } else {
                STRIP_EMPTY
            };
        }
        if tab == Mode::Phrase {
            show(ui, KINDS, phrase_tab::kinds(st));
            if st.phrase.kit_track.is_some() {
                show(ui, KIT, phrase_tab::kit(st));
            } else {
                match &st.maker {
                    Some(m) => show(ui, MAKER, PhraseMaker { view: m.clone() }),
                    None => show(ui, PHRASES, phrase_tab::rows(st)),
                }
                let plays = phrase_tab::plays(st);
                // Its sliders may be under a finger, so the panel is
                // changed in place, and only when it shows something else.
                if ui
                    .get::<PhrasePlays>(PLAYS)
                    .is_some_and(|w| w.show != plays || w.sliders.rows != rows)
                {
                    if let Some(w) = ui.get_mut::<PhrasePlays>(PLAYS) {
                        w.show = plays;
                        w.sliders.rows = rows.to_vec();
                    }
                }
            }
        }
        Ok(())
    }

    /// What a touch on one of the arp's widgets asks for; None for touches
    /// on other widgets or that change nothing. `st` is what they show.
    pub fn action(
        &self,
        ev: &UiEvent,
        st: &ArpState,
        qlink: &mut Option<usize>,
    ) -> Option<ArpAction> {
        let s = st.settings;
        match (ev.widget.as_str(), &ev.kind) {
            (MODES, EventKind::Tab(i)) => MODES_ALL.get(*i).map(|m| ArpAction::Mode(*m)),
            (LIST, EventKind::Select(i)) => match s.mode {
                Mode::Direct | Mode::Repeat | Mode::Phrase => {
                    Rate::ALL.get(*i).map(|r| ArpAction::Rate(*r))
                }
                Mode::Arp => Order::ALL.get(*i).map(|o| ArpAction::Order(*o)),
                Mode::Pattern => (*i < st.patterns.len()).then_some(ArpAction::Pattern(*i)),
            },
            (PATTERNS, EventKind::Select(i)) => {
                (*i < st.patterns.len()).then_some(ArpAction::Pattern(*i))
            }
            // A group's tab: its first pattern.
            (PATTERNS, EventKind::Tab(g)) => pattern_groups(st)
                .get(*g)
                .map(|(_, at)| ArpAction::Pattern(*at)),
            // The sliders, in their own panel or the Phrase tab's.
            (QLINKS | PLAYS, EventKind::Select(i)) => {
                *qlink = Some(*i);
                None
            }
            (QLINKS | PLAYS, EventKind::Value(v)) => {
                qlink.map(|i| ArpAction::Settings(with_qlink(s, st.tab(), i, *v)))
            }
            (KINDS, EventKind::Select(i)) => Shelf::ALL.get(*i).map(|k| ArpAction::Shelf(*k)),
            (PHRASES, EventKind::Select(i)) => {
                (*i < st.phrases.phrases.len()).then_some(ArpAction::Phrase(*i))
            }
            (PHRASES, EventKind::Toggle(phrase_tab::NEW_IDEA)) => Some(ArpAction::NewIdea),
            (PLAYS, EventKind::Toggle(phrase_tab::MORE_LIKE)) => Some(ArpAction::MoreLike),
            (MAKER, EventKind::Step { index, delta }) => {
                Some(ArpAction::Maker(MakerAction::Step {
                    row: *index,
                    delta: *delta,
                }))
            }
            (MAKER, EventKind::Toggle(k)) => match *k {
                MAKER_TRY => Some(MakerAction::TryAgain),
                MAKER_STEP => Some(MakerAction::Previous),
                MAKER_CLOSE | MAKER_BACK => Some(MakerAction::Back),
                MAKER_KEEP => Some(MakerAction::Keep),
                _ => None,
            }
            .map(ArpAction::Maker),
            // In key or Exact.
            (PLAYS, EventKind::Tab(i)) => Follow::ALL
                .get(*i)
                .map(|f| ArpAction::Settings(Settings { follow: *f, ..s })),
            (ACTIONS, EventKind::Tab(i)) => [
                PatternButton::Edit,
                PatternButton::Record,
                PatternButton::Mutate,
                PatternButton::Original,
            ]
            .get(*i)
            .map(|b| ArpAction::Button(*b)),
            (STEPS, EventKind::Select(i)) if s.mode == Mode::Pattern => Some(ArpAction::Step(*i)),
            (MODBOX, EventKind::Tab(_)) => Some(ArpAction::Modifier),
            // The band's − and + on a phrase: its speed.
            (MINI, EventKind::Tab(i @ (MINI_SLOWER | MINI_FASTER))) if s.mode == Mode::Phrase => {
                let at = Speed::ALL.iter().position(|x| *x == s.speed).unwrap_or(1);
                let to = if *i == MINI_SLOWER {
                    at.checked_sub(1)
                } else {
                    Some(at + 1)
                };
                to.and_then(|j| Speed::ALL.get(j))
                    .map(|speed| ArpAction::Settings(Settings { speed: *speed, ..s }))
            }
            (MINI, EventKind::Tab(i)) => {
                let at = Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0);
                match *i {
                    MINI_SLOWER => at
                        .checked_sub(1)
                        .and_then(|j| Rate::ALL.get(j))
                        .map(|r| ArpAction::Rate(*r)),
                    MINI_FASTER => Rate::ALL.get(at + 1).map(|r| ArpAction::Rate(*r)),
                    MINI_LATCH => Some(ArpAction::Latch),
                    MINI_OPEN => Some(ArpAction::OpenPanel),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Why a touch that [`ArpPanel::action`] answers with nothing changes
    /// nothing, for the hint line (Jay, 13 Nov: say so in place).
    pub fn why_not(&self, ev: &UiEvent, st: &ArpState) -> Option<&'static str> {
        match (ev.widget.as_str(), &ev.kind) {
            (STEPS, EventKind::Select(_)) if st.settings.mode != Mode::Pattern => {
                Some("The steps show the notes as they play: pick Pattern to edit them")
            }
            _ => None,
        }
    }
}

/// What the quick band shows for `st`: what the arp plays, and between its
/// − and + the rate, or for a phrase its speed.
fn mini_bar(st: &ArpState) -> MiniBar {
    let s = &st.settings;
    let mode = match s.mode {
        Mode::Direct => "Direct".to_string(),
        Mode::Repeat => "Repeat".to_string(),
        Mode::Phrase => match st.phrase_heard() {
            "" => "Phrase".to_string(),
            name => format!("Phrase · {}", short(name, 14)),
        },
        Mode::Arp => format!("Arp · {}", s.order.label()),
        Mode::Pattern => {
            let name = st
                .patterns
                .get(st.pattern_index)
                .map_or("", |(n, _)| n.as_str());
            format!(
                "Pattern · {}{}",
                short(name, 14),
                if st.pattern_changed { "*" } else { "" }
            )
        }
    };
    // What plays now: pushed, in the second accent.
    let (rate, at, of) = if s.mode == Mode::Phrase {
        (
            speed_short(mpfree_arp::phrase_speed(s, &st.live)),
            Speed::ALL.iter().position(|x| *x == s.speed).unwrap_or(1),
            Speed::ALL.len(),
        )
    } else {
        (
            st.live.rate.label().to_string(),
            Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0),
            Rate::ALL.len(),
        )
    };
    MiniBar {
        mode,
        rate,
        pushed: st.live.rate != s.rate,
        latched: st.latched,
        can_slower: at > 0,
        can_faster: at + 1 < of,
    }
}

fn rate_detail(r: Rate) -> &'static str {
    match r {
        Rate::Quarter => "beat",
        Rate::Eighth => "1/2 beat",
        Rate::EighthTriplet => "1/3 beat",
        Rate::Sixteenth => "1/4 beat",
        Rate::SixteenthTriplet => "1/6 beat",
        Rate::ThirtySecond => "1/8 beat",
        Rate::ThirtySecondTriplet => "1/12 beat",
    }
}

/// `text` cut to `max` characters with an ellipsis.
fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max - 1).collect();
        s.push('…');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{built_in_patterns, samples};
    use mpfree_ui::{Fonts, Rect, SoftCanvas, Touch, Widget};

    fn state(settings: Settings, held: &[u8], step: u64) -> ArpState {
        ArpState {
            settings,
            live: settings,
            push: 0.0,
            pattern: Pattern::default(),
            patterns: built_in_patterns(),
            pattern_index: 0,
            pattern_changed: false,
            phrases: PhraseList::built_in(),
            phrase_index: 0,
            maker: None,
            pads_key: None,
            phrase: PhraseNow::default(),
            notes: held
                .iter()
                .map(|&note| Held {
                    note,
                    velocity: 100,
                })
                .collect(),
            step,
            latched: true,
            bpm: 120.0,
        }
    }

    fn sample(name: &str) -> crate::sample::Sample {
        samples(Fonts::embedded)
            .unwrap()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    #[test]
    fn an_app_can_wrap_the_layouts_with_a_bar_of_its_own() {
        fn with_bar(layout: &str) -> String {
            layout.replacen("col(header:60, ", "col(header:60, bar:44, ", 1)
        }
        let mut s = sample("arp-up-2-oct");
        s.ui.add(
            "bar",
            mpfree_ui::widgets::Label::new("bar", Tone::Dim, mpfree_ui::Align::Left),
        );
        let before = s.ui.rect_of(LIST).unwrap();
        s.page.set_layout_wrap(&mut s.ui, with_bar).unwrap();
        assert!(s.ui.rect_of("bar").is_some());
        assert!(s.ui.rect_of(LIST).unwrap().h < before.h);
        // A new mode's layout is wrapped too, and so is the quick band.
        let mut st = s.state.clone();
        st.settings.mode = Mode::Pattern;
        s.page.update(&mut s.ui, &st).unwrap();
        assert!(s.ui.rect_of("bar").is_some() && s.ui.rect_of(ACTIONS).is_some());
        s.ui.show(&s.page.quick.clone()).unwrap();
        assert!(s.ui.rect_of("bar").is_some() && s.ui.rect_of(MINI).is_some());
    }

    #[test]
    fn every_sample_screen_lays_out_and_draws() {
        for mut s in samples(Fonts::embedded).unwrap() {
            let mut canvas = SoftCanvas::new(1280, 800);
            s.ui.paint(&mut canvas);
            let mode = s.state.settings.mode;
            let mut want = vec!["header", "hint"];
            let mut gone = vec![];
            let phrase = [KINDS, PHRASES, PLAYS, KIT];
            if s.quick {
                want.extend(["pads", MINI, STEPS]);
                gone.extend([MODES, LIST, PATTERNS, QLINKS, ACTIONS]);
                gone.extend(phrase);
            } else if s.state.tab() == Mode::Phrase {
                // The Phrase tab: its own panels, in the pads' place too.
                want.extend([MODES, KINDS]);
                if s.state.phrase.kit_track.is_some() {
                    want.push(KIT);
                    gone.extend([PHRASES, PLAYS]);
                } else if s.state.maker.is_some() {
                    want.extend([MAKER, PLAYS]);
                    gone.extend([KIT, PHRASES]);
                } else {
                    want.extend([PHRASES, PLAYS]);
                    gone.extend([KIT, MAKER]);
                }
                gone.extend(["pads", LIST, PATTERNS, QLINKS, ACTIONS, STEPS, MINI]);
            } else {
                want.extend(["pads", MODES]);
                want.extend(if mode == Mode::Pattern {
                    [PATTERNS, ACTIONS]
                } else {
                    [LIST, QLINKS]
                });
                // The strip shows only while the arp is generating.
                if mode == Mode::Direct {
                    gone.push(STEPS)
                } else {
                    want.push(STEPS)
                }
                gone.push(MINI);
                gone.extend(phrase);
            }
            for name in want {
                let r =
                    s.ui.rect_of(name)
                        .unwrap_or_else(|| panic!("{}: no {name}", s.name));
                assert!(
                    r.w > 0 && r.h > 0 && r.right() <= 1280 && r.bottom() <= 800,
                    "{}: {name} {r:?}",
                    s.name
                );
            }
            for name in gone {
                assert!(
                    s.ui.rect_of(name).is_none(),
                    "{}: {name} should not show",
                    s.name
                );
            }
            // The arp is not a tab: the header holds the app's tabs and the chip.
            let header = s.ui.get::<mpfree_ui::widgets::Header>("header").unwrap();
            assert!(!header.tabs.iter().any(|t| t == "Arp"), "{}", s.name);
            assert_eq!(
                header.chips.last().map(|c| c.text.clone()),
                Some(chip(&s.state).text),
                "{}",
                s.name
            );
        }
    }

    #[test]
    fn the_quick_band_is_a_slim_strip_with_the_rate_and_latch() {
        let mut s = sample("quick-hat-roll");
        let r = s.ui.rect_of(MINI).unwrap();
        assert_eq!(r.h, MINI_H);
        let bar = s.ui.get::<MiniBar>(MINI).unwrap().clone();
        assert_eq!(
            (bar.mode.as_str(), bar.rate.as_str(), bar.latched),
            ("Repeat", "1/16", false)
        );
        assert!(bar.can_slower && bar.can_faster);
        let parts = bar.parts(r);
        for (i, p) in parts.iter().enumerate() {
            assert!(p.w >= 44 && p.h >= 44, "part {i}: {p:?}");
            assert!(
                p.x >= r.x && p.right() <= r.right() && p.y >= r.y && p.bottom() <= r.bottom(),
                "part {i}: {p:?}"
            );
            for (j, o) in parts.iter().enumerate().skip(i + 1) {
                assert!(
                    p.intersect(*o).is_empty(),
                    "parts {i} and {j} overlap: {p:?} {o:?}"
                );
            }
        }
        let mut tap = |i: usize| {
            let p = parts[i];
            let ev = s.ui.touch(1, Touch::Down, p.x + 10, p.y + 10);
            s.ui.touch(1, Touch::Up, 0, 0);
            ev.and_then(|ev| s.page.action(&ev, &s.state, &mut None))
        };
        assert_eq!(tap(MINI_SLOWER), Some(ArpAction::Rate(Rate::EighthTriplet)));
        assert_eq!(
            tap(MINI_FASTER),
            Some(ArpAction::Rate(Rate::SixteenthTriplet))
        );
        assert_eq!(tap(MINI_LATCH), Some(ArpAction::Latch));
        assert_eq!(tap(MINI_OPEN), Some(ArpAction::OpenPanel));
        // At the ends, the arrow that would go past does nothing.
        let mut st = s.state.clone();
        st.settings.rate = Rate::ThirtySecondTriplet;
        st.latched = true;
        s.page.update(&mut s.ui, &st).unwrap();
        let bar = s.ui.get::<MiniBar>(MINI).unwrap();
        assert!(bar.can_slower && !bar.can_faster && bar.latched);
        assert_eq!(
            s.ui.touch(
                2,
                Touch::Down,
                parts[MINI_FASTER].x + 10,
                parts[MINI_FASTER].y + 10
            ),
            None
        );
        s.ui.touch(2, Touch::Up, 0, 0);
        // A held hat repeats; the strip under it shows the pad.
        assert_eq!(s.ui.get::<StepStrip>(STEPS).unwrap().cells[0].label, "3");
    }

    /// With a phrase the band's − and + step its speed, and between them
    /// it says how fast the phrase plays; the strip under it has no steps
    /// of an order to show.
    #[test]
    fn the_quick_band_on_a_phrase_steps_its_speed() {
        let mut s = sample("quick-phrase");
        let r = s.ui.rect_of(MINI).unwrap();
        let bar = s.ui.get::<MiniBar>(MINI).unwrap().clone();
        assert_eq!(
            (bar.mode.as_str(), bar.rate.as_str(), bar.pushed),
            ("Phrase · Night drive", "1×", false)
        );
        assert!(bar.can_slower && bar.can_faster);
        let parts = bar.parts(r);
        let mut tap = |i: usize| {
            let p = parts[i];
            let ev = s.ui.touch(1, Touch::Down, p.x + 10, p.y + 10);
            s.ui.touch(1, Touch::Up, 0, 0);
            ev.and_then(|ev| s.page.action(&ev, &s.state, &mut None))
        };
        let speed = |a: Option<ArpAction>| match a {
            Some(ArpAction::Settings(set)) => (set.speed, set.rate),
            other => panic!("{other:?}"),
        };
        let rate = s.state.settings.rate;
        assert_eq!(speed(tap(MINI_SLOWER)), (Speed::Half, rate));
        assert_eq!(speed(tap(MINI_FASTER)), (Speed::Double, rate));
        assert_eq!(tap(MINI_LATCH), Some(ArpAction::Latch));
        // At half speed there is no slower; a push on the rate shows.
        let mut st = s.state.clone();
        st.settings.speed = Speed::Half;
        st.live.speed = Speed::Half;
        s.page.update(&mut s.ui, &st).unwrap();
        let bar = s.ui.get::<MiniBar>(MINI).unwrap();
        assert_eq!(
            (bar.rate.as_str(), bar.can_slower, bar.can_faster),
            ("½×", false, true)
        );
        st.live.rate = Rate::ThirtySecond;
        s.page.update(&mut s.ui, &st).unwrap();
        let bar = s.ui.get::<MiniBar>(MINI).unwrap();
        assert_eq!((bar.rate.as_str(), bar.pushed), ("1×", true));
        let strip = s.ui.get::<StepStrip>(STEPS).unwrap();
        assert!(strip.cells.is_empty());
        assert_eq!(strip.empty, PHRASE_STRIP);
        assert_eq!(
            quick_hint(Mode::Phrase),
            "Tap a speed · Q-Links: speed, gate, swing, octaves · Wheel: phrase · let go: back to your page"
        );
    }

    #[test]
    fn the_chip_says_what_the_arp_does_and_whether_it_is_latched() {
        let mut st = state(
            Settings {
                mode: Mode::Arp,
                ..Settings::default()
            },
            &[0, 2],
            1,
        );
        assert_eq!(
            (chip(&st).text, chip(&st).tone),
            ("Arp: Up 1/16, latched".to_string(), Tone::Warn)
        );
        st.latched = false;
        assert_eq!(
            (chip(&st).text, chip(&st).tone),
            ("Arp: Up 1/16".to_string(), Tone::Accent)
        );
        st.settings.mode = Mode::Repeat;
        st.settings.rate = Rate::ThirtySecondTriplet;
        assert_eq!(chip(&st).text, "Repeat 1/32T");
        st.settings.mode = Mode::Pattern;
        st.pattern_changed = true;
        assert_eq!(
            chip(&st).text,
            format!("Pattern: {}* 1/32T", st.patterns[0].0)
        );
        // A phrase by its name, as a pattern.
        st.settings.mode = Mode::Phrase;
        st.phrase_index = 4;
        assert_eq!(chip(&st).text, format!("Phrase: {}", st.phrases.name(4)));
        st.latched = true;
        assert_eq!(
            chip(&st).text,
            format!("Phrase: {}, latched", st.phrases.name(4))
        );
        st.latched = false;
        st.settings.mode = Mode::Direct;
        assert_eq!(
            (chip(&st).text, chip(&st).tone),
            ("Arp off".to_string(), Tone::Dim)
        );
        st.latched = true;
        assert_eq!(chip(&st).text, "Latch on");
    }

    /// In key, a phrase follows the arp's own scale, or with that off the
    /// scale the pads play.
    #[test]
    fn a_phrases_key_is_the_arps_scale_or_the_pads() {
        let mut st = state(Settings::default(), &[], 0);
        assert_eq!(st.phrase_key(), None);
        let pads = Scale::new(9, mpfree_arp::scale::MINOR);
        st.pads_key = Some(pads);
        assert_eq!(st.phrase_key(), Some(pads));
        let own = Scale::new(2, mpfree_arp::scale::MAJOR);
        st.settings.scale = Some(own);
        assert_eq!(st.phrase_key(), Some(own));
    }

    #[test]
    fn the_order_previews_match_the_engine_orders() {
        let got: Vec<String> = Order::ALL.iter().map(|o| order_preview(*o)).collect();
        assert_eq!(
            got,
            [
                "1 2 3",
                "3 2 1",
                "1 2 3 2",
                "1 2 3 3 2 1",
                "3 1 2",
                "2 1 3",
                "as pressed",
                "?",
                "?",
                "?",
                "1+2+3"
            ]
        );
    }

    #[test]
    fn arp_cells_follow_the_order_and_mark_the_step_playing() {
        let s = Settings {
            mode: Mode::Arp,
            octaves: 2,
            ..Settings::default()
        };
        let (cells, first, playing) = arp_cells(&state(s, &[4, 0, 2], 5));
        let labels: Vec<(&str, &str)> = cells
            .iter()
            .map(|c| (c.label.as_str(), c.sub.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("1", ""),
                ("3", ""),
                ("5", ""),
                ("1", "oct +1"),
                ("3", "oct +1"),
                ("5", "oct +1")
            ]
        );
        assert_eq!((first, playing), (0, Some(4)));
        // As played keeps the press order.
        let s = Settings {
            mode: Mode::Arp,
            order: Order::Played,
            ..Settings::default()
        };
        let (cells, _, playing) = arp_cells(&state(s, &[4, 0, 2], 0));
        assert_eq!(
            cells.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["5", "1", "3"]
        );
        assert_eq!(playing, None);
        // Nothing held, or Direct: an empty strip.
        assert!(arp_cells(&state(s, &[], 3)).0.is_empty());
        assert!(arp_cells(&state(Settings::default(), &[1], 3)).0.is_empty());
        // MIDI notes above the pads are named.
        assert_eq!(note_label(60), "C4");
        // A chord from one note, and a scale run: the strip shows what plays.
        let c_major = Some(mpfree_arp::Scale::new(0, mpfree_arp::scale::MAJOR));
        let s = Settings {
            mode: Mode::Arp,
            scale: c_major,
            chord: mpfree_arp::Chord::Triad,
            ..Settings::default()
        };
        let labels = |st: ArpState| {
            arp_cells(&st)
                .0
                .iter()
                .map(|c| c.label.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(state(s, &[62], 0)), ["D4", "F4", "A4"]);
        let s = Settings {
            mode: Mode::Arp,
            scale: c_major,
            run: true,
            order: Order::Played,
            ..Settings::default()
        };
        assert_eq!(
            labels(state(s, &[64, 60], 0)),
            ["C4", "D4", "E4"],
            "a run goes up, whatever the press order"
        );
        // Chords taking turns: the strip shows the chord whose turn it is,
        // and the step within that turn (a beat = 4 sixteenths).
        let s = Settings {
            mode: Mode::Arp,
            scale: c_major,
            chord: mpfree_arp::Chord::Triad,
            turns: mpfree_arp::Turns::Beat,
            ..Settings::default()
        };
        assert_eq!(labels(state(s, &[62, 67], 4)), ["D4", "F4", "A4"]);
        let (cells, _, playing) = arp_cells(&state(s, &[62, 67], 6));
        assert_eq!((cells[0].label.as_str(), playing), ("G4", Some(1)));
    }

    #[test]
    fn long_orders_scroll_in_windows_of_sixteen() {
        let s = Settings {
            mode: Mode::Arp,
            order: Order::UpDownInclusive,
            octaves: 4,
            ..Settings::default()
        };
        // 3 notes x 4 octaves up and down = 24 counts; count 19 is playing.
        let (cells, first, playing) = arp_cells(&state(s, &[0, 1, 2], 20));
        assert_eq!((cells.len(), first, playing), (8, 16, Some(3)));
    }

    #[test]
    fn pattern_cells_show_picks_and_marks() {
        let p = Pattern::parse("1 x+! . _ *:3 ?%40 l- h").unwrap();
        let cells: Vec<StepCell> = p.steps().iter().map(|s| pattern_cell(*s)).collect();
        let text: Vec<(&str, &str, bool)> = cells
            .iter()
            .map(|c| (c.label.as_str(), c.sub.as_str(), c.accent))
            .collect();
        assert_eq!(
            text,
            [
                ("1", "", false),
                ("x", "oct +1", true),
                ("·", "", false),
                ("—", "", false),
                ("all", "×3", false),
                ("?", "40%", false),
                ("low", "oct -1", false),
                ("high", "", false)
            ]
        );
        let mut st = state(
            Settings {
                mode: Mode::Pattern,
                ..Settings::default()
            },
            &[0],
            11,
        );
        st.pattern = p;
        assert_eq!(pattern_cells(&st).2, Some(2));
    }

    #[test]
    fn qlinks_cover_every_value_and_round_trip() {
        let base = Settings::default();
        let arp = Mode::Arp;
        for (k, r) in Rate::ALL.iter().enumerate() {
            let s = with_qlink(base, arp, 0, k as f32 / 6.0);
            assert_eq!(s.rate, *r);
            assert_eq!(
                with_qlink(base, arp, 0, qlink_values(&s, arp)[0].0).rate,
                *r
            );
        }
        // The three after it are the same on every tab.
        for tab in [arp, Mode::Phrase] {
            for g in 1..=100u8 {
                let s = Settings { gate: g, ..base };
                assert_eq!(with_qlink(base, tab, 1, qlink_values(&s, tab)[1].0).gate, g);
            }
            for w in 50..=75u8 {
                let s = Settings { swing: w, ..base };
                assert_eq!(
                    with_qlink(base, tab, 2, qlink_values(&s, tab)[2].0).swing,
                    w
                );
            }
            for o in 1..=4u8 {
                let s = Settings { octaves: o, ..base };
                assert_eq!(
                    with_qlink(base, tab, 3, qlink_values(&s, tab)[3].0).octaves,
                    o
                );
            }
        }
        assert_eq!(with_qlink(base, arp, 1, 9.0).gate, 100);
        assert_eq!(qlink_values(&base, arp)[2].1, "off");
    }

    /// On the Phrase tab the first Q-Link is the phrase's speed: half at
    /// the left, as written in the middle, double at the right, and the
    /// rate is left where it was.
    #[test]
    fn on_the_phrase_tab_the_first_qlink_is_the_speed() {
        let base = Settings::default();
        let tab = Mode::Phrase;
        assert_eq!(qlink_labels(tab)[0], "Speed · Q1");
        assert_eq!(qlink_labels(Mode::Arp)[0], "Rate · Q1");
        let texts = ["half", "as written", "double"];
        for (k, speed) in Speed::ALL.iter().enumerate() {
            let at = k as f32 / 2.0;
            let s = with_qlink(base, tab, 0, at);
            assert_eq!((s.speed, s.rate), (*speed, base.rate));
            let (value, text) = qlink_values(&s, tab)[0].clone();
            assert_eq!((value, text.as_str()), (at, texts[k]));
            assert_eq!(with_qlink(base, tab, 0, value).speed, *speed);
        }
        // Between and beyond the three, a push reads as a number of times.
        let (at, text) = speed_value(1.5);
        assert!((at - 0.7925).abs() < 1e-3 && text == "1.5×", "{at} {text}");
        assert_eq!(speed_value(4.0), (1.0, "4×".to_string()));
        assert_eq!(speed_value(0.25).1, "0.25×");
        assert_eq!(speed_value(2.0 / 3.0).1, "0.67×");
    }

    #[test]
    fn the_pattern_list_shows_the_groups_and_a_tab_loads_its_first_pattern() {
        let mut s = sample("arp-up-2-oct");
        let mut st = s.state.clone();
        st.settings.mode = Mode::Pattern;
        st.pattern_index = 3;
        s.page.update(&mut s.ui, &st).unwrap();
        let list = s.ui.get::<PatternList>(PATTERNS).unwrap();
        assert_eq!(list.rows.len(), mpfree_arp::groups::count());
        assert_eq!(list.groups.len(), mpfree_arp::groups::GROUPS.len());
        assert_eq!(list.groups[3], ("Rolls".to_string(), 26));
        assert_eq!((list.selected, list.rows[3].name.as_str()), (3, "Climb"));
        // The picture: Climb's seven notes and a rest.
        assert_eq!(list.rows[3].marks.len(), 8);
        assert_eq!(list.rows[3].marks[7], Mark::Rest);
        let r = s.ui.rect_of(PATTERNS).unwrap();
        assert!(r.h >= 230, "three rows under the tabs: {r:?}");
        let mut q = None;
        // The 460 px column: one group at a time, with arrows.
        let arrows: Vec<_> =
            s.ui.targets()
                .into_iter()
                .filter(|t| t.target.id == "group")
                .collect();
        assert_eq!(arrows.len(), 2);
        let on = arrows[1].target.rect;
        let ev = s.ui.touch(1, Touch::Down, on.x + 5, on.y + 5).unwrap();
        s.ui.touch(1, Touch::Up, on.x + 5, on.y + 5);
        assert_eq!(
            s.page.action(&ev, &st, &mut q),
            Some(ArpAction::Pattern(10))
        );
        // As wide as the app's panel: a tab for each group.
        s.page
            .set_layout_wrap(&mut s.ui, |l| {
                l.replacen("row(pads:*, ", "row(pads:160, ", 1)
                    .replacen("):460):*", "):*):*", 1)
            })
            .unwrap();
        let targets = s.ui.targets();
        let tabs: Vec<_> = targets.iter().filter(|t| t.target.id == "group").collect();
        assert_eq!(tabs.len(), 9);
        let rows: Vec<_> = targets
            .iter()
            .filter(|t| t.widget == PATTERNS && t.target.id == "row")
            .collect();
        assert!(rows.len() >= 3, "at least three rows show: {}", rows.len());
        // A tap on Rolls: its first pattern, Trap hats.
        let rolls = tabs[3].target.rect;
        let ev =
            s.ui.touch(1, Touch::Down, rolls.x + rolls.w / 2, rolls.y + rolls.h / 2)
                .unwrap();
        s.ui.touch(1, Touch::Up, rolls.x + rolls.w / 2, rolls.y + rolls.h / 2);
        assert_eq!(
            s.page.action(&ev, &st, &mut q),
            Some(ArpAction::Pattern(26))
        );
        // Picked, the list shows Rolls from its start.
        st.pattern_index = 27;
        s.page.update(&mut s.ui, &st).unwrap();
        let first =
            s.ui.targets()
                .into_iter()
                .find(|t| t.widget == PATTERNS && t.target.id == "row")
                .unwrap()
                .target;
        assert_eq!((first.index, first.label.as_str()), (26, "Trap hats"));
        let ev =
            s.ui.touch(1, Touch::Down, first.rect.x + 20, first.rect.y + 20)
                .unwrap();
        assert_eq!(
            s.page.action(&ev, &st, &mut q),
            Some(ArpAction::Pattern(26))
        );
    }

    #[test]
    fn a_pattern_is_drawn_as_its_rhythm() {
        let p = Pattern::parse("x! . x:3 *%50 _ ?").unwrap();
        let hit = |hits, accent, chord, maybe| Mark::Hit {
            hits,
            accent,
            chord,
            maybe,
        };
        assert_eq!(
            pattern_marks(&p),
            vec![
                hit(1, true, false, false),
                Mark::Rest,
                hit(3, false, false, false),
                hit(1, false, true, true),
                Mark::Tie,
                hit(1, false, false, false),
            ]
        );
        // Only the first bar: the rest is said in steps.
        let long = mpfree_arp::Library::built_in();
        let fill = long.get("Four-bar fill").unwrap();
        assert_eq!(pattern_marks(fill).len(), MARKS);
    }

    #[test]
    fn a_narrow_pattern_list_pages_through_the_groups() {
        let mut list = PatternList {
            rows: (0..12)
                .map(|i| PatternRow {
                    name: format!("P{i}"),
                    marks: vec![Mark::Rest; 4],
                    steps: 4,
                    text: ". . . .".into(),
                })
                .collect(),
            groups: (0..6).map(|g| (format!("G{g}"), g * 2)).collect(),
            selected: 3,
        };
        let r = Rect::new(0, 0, 440, 240);
        let t = list.targets(r);
        let arrows: Vec<_> = t.iter().filter(|t| t.id == "group").collect();
        assert_eq!(arrows.len(), 2, "six tabs in 440 px don't fit: arrows");
        let on = arrows[1].rect;
        assert_eq!(
            list.touch(1, Touch::Down, on.x + 5, on.y + 5, r),
            Some(EventKind::Tab(2))
        );
        let back = arrows[0].rect;
        list.selected = 0;
        assert_eq!(
            list.touch(1, Touch::Down, back.x + 5, back.y + 5, r),
            Some(EventKind::Tab(5)),
            "back from the first group is the last"
        );
        // Wide enough, one tab each.
        let wide = list.targets(Rect::new(0, 0, 1000, 240));
        assert_eq!(wide.iter().filter(|t| t.id == "group").count(), 6);
    }

    #[test]
    fn taps_become_actions() {
        let mut s = sample("arp-up-2-oct");
        let mut qlink = None;
        let tap = |ui: &mut Ui, name: &str, fx: f32, fy: f32| {
            let r = ui.rect_of(name).unwrap();
            let (x, y) = (
                r.x + (r.w as f32 * fx) as i32,
                r.y + (r.h as f32 * fy) as i32,
            );
            let ev = ui.touch(1, Touch::Down, x, y);
            ui.touch(1, Touch::Up, x, y);
            ev
        };
        // The Pattern tab (the fourth of five), then Phrase (the last).
        let ev = tap(&mut s.ui, MODES, 0.7, 0.5).unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut qlink),
            Some(ArpAction::Mode(Mode::Pattern))
        );
        let ev = tap(&mut s.ui, MODES, 0.9, 0.5).unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut qlink),
            Some(ArpAction::Mode(Mode::Phrase))
        );
        // The first row of the order list is Up.
        let ev = tap(&mut s.ui, LIST, 0.5, 0.25).unwrap();
        assert_eq!(
            s.page.action(&ev, &s.state, &mut qlink),
            Some(ArpAction::Order(Order::Up))
        );
        // A finger on the gate slider, then moved to its right end.
        let r = s.ui.rect_of(QLINKS).unwrap();
        let (_, track) = s.ui.get::<SliderStack>(QLINKS).unwrap().row_rects(r, 1);
        let ev = s.ui.touch(2, Touch::Down, track.x + 5, track.y).unwrap();
        assert_eq!(s.page.action(&ev, &s.state, &mut qlink), None);
        assert_eq!(qlink, Some(1));
        let ev =
            s.ui.touch(2, Touch::Move, track.right() + 50, track.y)
                .unwrap();
        let Some(ArpAction::Settings(set)) = s.page.action(&ev, &s.state, &mut qlink) else {
            panic!("{ev:?}")
        };
        assert_eq!(set.gate, 100);
        s.ui.touch(2, Touch::Up, 0, 0);
    }

    /// The Modifier: a box beside the tabs where the panel is wide, the
    /// pushed values beside the set ones on the sliders, the rate playing
    /// on the quick band and the pushed octaves in the step strip; a tap
    /// on the box opens its options.
    #[test]
    fn the_modifier_shows_where_it_pushes() {
        let set = Settings {
            mode: Mode::Arp,
            modifier: mpfree_arp::Modifier {
                source: ModSource::ModWheel,
                ..mpfree_arp::Modifier::default()
            },
            ..Settings::default()
        };
        let mut st = state(set, &[0, 4, 7], 0);
        let mut ui = Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", mpfree_ui::widgets::Header::new(&["Arp"]));
        ui.add("pads", mpfree_ui::widgets::PadGrid::new(16, 4));
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new("", Tone::Dim, mpfree_ui::Align::Center),
        );
        let mut page = ArpPanel::add(&mut ui, "Arp", "Arp quick", &st).unwrap();
        ui.show("Arp").unwrap();
        // The 460-pixel column has no room for it.
        assert!(ui.rect_of(MODBOX).is_none());
        // A full-width panel (the app's) has it at the tabs' right.
        fn full(layout: &str) -> String {
            with_modifier(layout)
                .replace("row(pads:*, ", "row(pads:0, ")
                .replace("):460):*", "):*):*")
        }
        page.set_layout_wrap(&mut ui, full).unwrap();
        ui.show("Arp").unwrap();
        let (tabs, b) = (ui.rect_of(MODES).unwrap(), ui.rect_of(MODBOX).unwrap());
        assert_eq!((b.w, b.y, b.h), (MODBOX_W, tabs.y, tabs.h));
        assert!(b.x >= tabs.right() && tabs.w > 4 * 160, "{tabs:?}");
        let m = ui.get::<ModBox>(MODBOX).unwrap();
        assert_eq!((m.source.as_str(), m.amount), ("Mod wheel", 0.0));
        let q = ui.get::<SliderStack>(QLINKS).unwrap();
        assert!(q.rows.iter().all(|r| r.pushed.is_none()), "unpushed");
        // The wheel at the top: 1/16 → 1/32, 1 → 2 octaves.
        st.push = 1.0;
        st.live.rate = Rate::ThirtySecond;
        st.live.octaves = 2;
        st.step = 1;
        page.update(&mut ui, &st).unwrap();
        assert_eq!(ui.get::<ModBox>(MODBOX).unwrap().amount, 1.0);
        let q = ui.get::<SliderStack>(QLINKS).unwrap();
        assert_eq!(q.rows[0].text, "1/16");
        assert_eq!(q.rows[0].pushed.as_ref().unwrap().1, "1/32");
        assert_eq!(q.rows[3].pushed.as_ref().unwrap().1, "2");
        assert!(q.rows[1].pushed.is_none() && q.rows[2].pushed.is_none());
        let mini = ui.get::<MiniBar>(MINI).unwrap();
        assert_eq!((mini.rate.as_str(), mini.pushed), ("1/32", true));
        let strip = ui.get::<StepStrip>(STEPS).unwrap();
        assert!(
            strip.cells.iter().any(|c| c.sub == "oct +1"),
            "the pushed octave"
        );
        // Off: the box says so.
        st.settings.modifier.source = ModSource::Off;
        page.update(&mut ui, &st).unwrap();
        assert!(ui.get::<ModBox>(MODBOX).unwrap().source.is_empty());
        // A tap on it.
        let ev = ui.touch(1, Touch::Down, b.x + 20, b.y + b.h / 2).unwrap();
        ui.touch(1, Touch::Up, 0, 0);
        assert_eq!(page.action(&ev, &st, &mut None), Some(ArpAction::Modifier));
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
    }

    #[test]
    fn pattern_mode_lays_out_the_buttons_and_original_waits_for_a_change() {
        let mut st = state(
            Settings {
                mode: Mode::Pattern,
                ..Settings::default()
            },
            &[0, 1],
            2,
        );
        st.pattern = Pattern::parse("1 2").unwrap();
        let mut ui = Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", mpfree_ui::widgets::Header::new(&["Arp"]));
        ui.add("pads", mpfree_ui::widgets::PadGrid::new(16, 4));
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new("", Tone::Dim, mpfree_ui::Align::Center),
        );
        let mut page = ArpPanel::add(&mut ui, "Arp", "Arp quick", &st).unwrap();
        ui.show("Arp").unwrap();
        assert!(ui.rect_of(ACTIONS).is_some() && ui.rect_of(QLINKS).is_none());
        let b = ui.get::<ButtonRow>(ACTIONS).unwrap();
        assert!(b.buttons[3].disabled);
        let r = ui.rect_of(ACTIONS).unwrap();
        let orig = b.button_rect(r, 3);
        assert_eq!(ui.touch(1, Touch::Down, orig.x + 5, orig.y + 5), None);
        ui.touch(1, Touch::Up, 0, 0);
        st.pattern_changed = true;
        page.update(&mut ui, &st).unwrap();
        let ev = ui.touch(1, Touch::Down, orig.x + 5, orig.y + 5).unwrap();
        assert_eq!(
            page.action(&ev, &st, &mut None),
            Some(ArpAction::Button(PatternButton::Original))
        );
        ui.touch(1, Touch::Up, 0, 0);
        // Back to Arp: the Q-Links come back.
        st.settings.mode = Mode::Arp;
        page.update(&mut ui, &st).unwrap();
        assert!(ui.rect_of(QLINKS).is_some() && ui.rect_of(ACTIONS).is_none());
        // Direct: the strip goes, and the list takes its room.
        let pads = ui.rect_of("pads").unwrap();
        st.settings.mode = Mode::Direct;
        page.update(&mut ui, &st).unwrap();
        assert!(ui.rect_of(STEPS).is_none() && ui.rect_of(QLINKS).is_some());
        assert_eq!(ui.rect_of("pads").unwrap(), pads);
    }

    #[test]
    fn the_pads_never_move_between_play_the_band_and_the_panel() {
        let st = state(
            Settings {
                mode: Mode::Arp,
                ..Settings::default()
            },
            &[0, 4, 7],
            3,
        );
        let mut ui = Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", mpfree_ui::widgets::Header::new(&["Play"]));
        ui.add("pads", mpfree_ui::widgets::PadGrid::new(16, 4));
        ui.add(
            "kits",
            mpfree_ui::widgets::Label::new("", Tone::Dim, mpfree_ui::Align::Center),
        );
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new("", Tone::Dim, mpfree_ui::Align::Center),
        );
        // The pad player's Play page: pads, and a column of its own beside them.
        ui.add_page(
            "Play",
            "col(header:60, row(pads:*, col(kits:*):460):*, hint:26)",
        )
        .unwrap();
        let mut page = ArpPanel::add(&mut ui, "Arp", "Arp quick", &st).unwrap();
        page.set_quick_layout(
            &mut ui,
            "col(header:60, row(pads:*, col(arp-mini:56, kits:*):460):*, hint:26)",
        )
        .unwrap();
        ui.show("Play").unwrap();
        let pads = ui.rect_of("pads").unwrap();
        let kits = ui.rect_of("kits").unwrap();
        assert_eq!(kits.w, SIDE);
        ui.show("Arp quick").unwrap();
        assert_eq!(ui.rect_of("pads").unwrap(), pads, "the quick band");
        let (mini, under) = (ui.rect_of(MINI).unwrap(), ui.rect_of("kits").unwrap());
        assert_eq!(
            (mini.x, mini.w, mini.y),
            (kits.x, kits.w, kits.y),
            "the strip sits at the top of the column"
        );
        assert_eq!(
            under.bottom(),
            kits.bottom(),
            "the app's column keeps all but the strip's height"
        );
        ui.show("Arp").unwrap();
        for mode in MODES_ALL {
            page.update(
                &mut ui,
                &state(
                    Settings {
                        mode,
                        ..Settings::default()
                    },
                    &[0, 4, 7],
                    3,
                ),
            )
            .unwrap();
            if mode == Mode::Phrase {
                // The one tab that takes the pads' room: a list of phrases
                // with the one that plays does not fit beside them.
                assert_eq!(ui.rect_of("pads"), None, "the Phrase tab");
                continue;
            }
            assert_eq!(ui.rect_of("pads").unwrap(), pads, "the panel in {mode:?}");
        }
    }
}

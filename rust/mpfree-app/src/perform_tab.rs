//! The Pads tab's Perform view (the Play, Pads and Sequencer redesign's
//! step 6, board 7): the project's **Perform set**, its eight pages of
//! action pads ([`crate::perform_pads`]), shown and edited where the kit's
//! pads are. The Pads of row's **Perform pads** chip opens it (not
//! "Perform": the top bar's MAKE / PERFORM chip is on the same screen); a
//! track's chip goes back to the kit.
//!
//! - The **bar** names the pad picked and what it does, and its **On the
//!   pads** switch puts the set on the pads (Play and the Looper tab play
//!   it, as SHIFT + SAMPLER pressed again on the Looper tab does) or takes
//!   it off.
//! - The **map** shows the page's 16 pads as on the MPC (pad 1 bottom
//!   left), each with what it does; its strip picks the page (A to H, as
//!   PAD BANK does here). A tap picks a pad; a tap on the pad picked plays
//!   its throw while the finger is down. A hit on the hardware picks the
//!   pad too, and plays a throw while held: nothing else a pad does
//!   happens here, so the set can be set up while a song plays.
//! - **What B2 does**: Throw, Macro, Loop key, Loop mute, Track mute or
//!   Scene; under it the eight choices of that kind (Play's throws, the
//!   eight macros, Loops 1 to 8, the tracks, the scenes), and the **Pad**
//!   list with its settings (a throw's effect, Hold, Starts and Let go; a
//!   macro's value) and what a hit does.
//! - **Copy**, **Paste** and **Clear** work on the pad picked; **Name
//!   page** types the page's name on the text keyboard. Each edit is one
//!   UNDO step; SHIFT + UNDO redoes. The history is this view's own, kept
//!   until the project changes.
//!
//! The screen keeps a copy of the set (the app sends it at the start, when
//! a project opens and after each edit); an edit goes to the app as
//! [`FromUi::PerformPad`], which saves it with the project.

use crate::gui::FromUi;
use crate::mixing::{ThrowFx, ThrowKnobs};
use crate::perform_pads::{Hold, LetGo, PadAction, PerformSet, Starts, Throw, PADS, PAGES};
use crate::play_page::THROW_LIST;
use mpfree_fx::StripId;
use mpfree_sampler_ui::widgets::{
    ButtonRow, KitTracks, PadBar, PadCell, PadMap, SectionChip, SectionChips, Setting, SettingList,
};
use mpfree_ui::qlink::QLinkCard;
use mpfree_ui::{Color, EventKind, Ui, UiEvent};

/// The view's page.
pub const PAGE: &str = "Pads · Perform";
/// Every widget of the view starts with this.
pub const PREFIX: &str = "prf-";
pub const BAR: &str = "prf-bar";
pub const MAP: &str = "prf-map";
pub const BUTTONS: &str = "prf-buttons";
/// What the pad does ([`Kind`]); a tap on chip `i` is [`EventKind::Tab`]`(i)`.
pub const TYPE: &str = "prf-type";
/// The eight choices of the pad's kind, four a row.
pub const PICK_A: &str = "prf-pick-a";
pub const PICK_B: &str = "prf-pick-b";
pub const LIST: &str = "prf-list";
/// The buttons under the Pad list: Edit the effect › and Hear it.
pub const ACTS: &str = "prf-acts";
/// The Pads of row, the kit view's ([`mpfree_sampler_ui::pads_tab::KITS`]).
pub const KITS: &str = mpfree_sampler_ui::pads_tab::KITS;

/// The view's page: the pads with their buttons as the kit view has them,
/// and what the pad picked does over the rest of the width.
/// The Q-Link column is Play's and the Mix tab's (`qlinks`, 128 px). The
/// chip rows and the buttons under the list are 52 px, so a throw's four
/// rows and what a hit does fit above them.
pub const LAYOUT: &str = "col(header:60, prf-bar:60, smp-pads-kits:52, \
                          row(col(prf-map:*, prf-buttons:64):440, \
                          col(prf-type:52, prf-pick-a:52, prf-pick-b:52, prf-list:*, prf-acts:52):*, \
                          qlinks:128):*, \
                          hint:26)";

/// The hint line under the view: what it is, then what the Q-Links turn
/// with the pad picked ([`PerformTab::qlink_hint`]), in the place of what
/// the kind chips and the map say.
pub fn hint(qlinks: Option<String>) -> String {
    match qlinks {
        Some(q) => format!(
            "Perform pads: one set for the project   ·   A-H: its pages (A is the looper's loop keys)   ·   \
             Q-Links: {q}"
        ),
        None => "Perform pads: one set for the project   ·   A-H: its pages (A is the looper's loop keys)   ·   \
                 a type: what the pad does   ·   hit a throw to hear it   ·   Q-Links: a throw pad's effect"
            .to_string(),
    }
}

/// The buttons under the map.
const COPY: usize = 0;
const PASTE: usize = 1;
const CLEAR: usize = 2;
const NAME: usize = 3;
/// The buttons under the Pad list.
const EDIT_FX: usize = 0;
const HEAR: usize = 1;
const LEARN: usize = 2;

/// What a Perform FX pad's Length and Amount do for each effect, in
/// `mpfree_fx::PERFORM_EFFECTS` order, as the Q-Links name them (Q1 and
/// Q2): no Length where the effect has no use for it (a wash rings for
/// its Decay; a crush holds no time). From `mpfree_fx`'s perform.rs.
const KNOB_NAMES: [(Option<&str>, &str); 15] = [
    (None, ""),
    (Some("Length"), "Mix"),
    (Some("Length"), "Mix"),
    (Some("Length"), "Mix"),
    (Some("Length"), "Mix"),
    (Some("Length"), "Mix"),
    (Some("Length"), "Mix"),
    (Some("Length"), "Depth"),
    (Some("Rate"), "Depth"),
    (Some("Sweep"), "Depth"),
    (Some("Sweep"), "Depth"),
    (None, "Crush"),
    (Some("Time"), "Feedback"),
    (Some("Rise"), "Level"),
    (None, "Decay"),
];

/// Q-Link `q`'s name for a Perform FX pad playing `effect`: Q1 its
/// Length's, Q2 its Amount's; none where it has nothing to turn.
fn knob_name(effect: u8, q: usize) -> Option<&'static str> {
    let (len, amount) = KNOB_NAMES.get(usize::from(effect)).copied()?;
    match q {
        0 => len,
        1 => Some(amount).filter(|a| !a.is_empty()),
        _ => None,
    }
}

/// The Perform FX parameter Q-Link `q` turns (inside the pad's four).
fn knob_param(q: usize) -> u16 {
    use mpfree_fx::ids::perform as pf;
    if q == 0 {
        pf::LENGTH
    } else {
        pf::AMOUNT
    }
}

/// The last Length choice.
fn last_length() -> f32 {
    (mpfree_fx::PERFORM_LENGTHS.len() - 1) as f32
}

/// Q-Link `q`'s value `v` as its card shows it ("1/8D", "55 %").
fn knob_text(q: usize, v: f32) -> String {
    if q == 0 {
        let i = (v.round().max(0.0) as usize).min(mpfree_fx::PERFORM_LENGTHS.len() - 1);
        mpfree_fx::PERFORM_LENGTHS[i].to_string()
    } else {
        format!("{} %", v.round() as i32)
    }
}

/// Q-Link `q`'s value `v` as far round the ring as it goes, 0 to 1.
fn knob_pos(q: usize, v: f32) -> f32 {
    let full = if q == 0 { last_length() } else { 100.0 };
    (v / full).clamp(0.0, 1.0)
}

/// Q-Link `q`'s value `v` turned `detents`: a Length choice a detent, an
/// Amount 1 % a detent (0.1 % with SHIFT, `fine`).
fn knob_turned(q: usize, v: f32, detents: i32, fine: bool) -> f32 {
    if q == 0 {
        (v.round() + detents as f32).clamp(0.0, last_length())
    } else {
        let per = if fine { 0.1 } else { 1.0 };
        (v + detents as f32 * per).clamp(0.0, 100.0)
    }
}

/// A Q-Link edit as the history and the log say it: "Delay throw on
/// Drums: Feedback 62 %". (Not a Q-Link edit: empty.)
fn knob_row(e: &Edit, names: &Names) -> String {
    match e {
        Edit::Knob {
            on,
            fx,
            effect,
            q,
            after,
            ..
        } => format!(
            "{} on {}: {} {}",
            throw_name(*fx),
            names.strip(*on).0,
            knob_name(*effect, *q).unwrap_or("setting"),
            knob_text(*q, *after)
        ),
        _ => String::new(),
    }
}

/// The most characters a page's name may have.
pub const MAX_PAGE_NAME: usize = 24;

/// Macros a pad can hold: the mod engine's.
const MACROS: usize = mpfree_mod::MACROS;

/// A macro pad's value steps by this much (%) on - / + and the wheel.
const VALUE_STEP: i32 = 5;

/// What a Perform pad can do, in the type chips' order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Throw,
    Macro,
    LoopKey,
    LoopMute,
    TrackMute,
    Scene,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Throw,
        Kind::Macro,
        Kind::LoopKey,
        Kind::LoopMute,
        Kind::TrackMute,
        Kind::Scene,
    ];

    /// Its chip's name.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Throw => "Throw",
            Kind::Macro => "Macro",
            Kind::LoopKey => "Loop key",
            Kind::LoopMute => "Loop mute",
            Kind::TrackMute => "Track mute",
            Kind::Scene => "Scene",
        }
    }

    /// The line under its name.
    fn sub(self) -> &'static str {
        match self {
            Kind::Throw => "an effect",
            Kind::Macro => "knobs held",
            Kind::LoopKey => "a loop's steps",
            Kind::LoopMute => "a loop's mute",
            Kind::TrackMute => "a track's mute",
            Kind::Scene => "a row launched",
        }
    }

    /// What `a` is; None for nothing.
    pub fn of(a: PadAction) -> Option<Kind> {
        Some(match a {
            PadAction::None => return None,
            PadAction::Throw(_) => Kind::Throw,
            PadAction::Macro { .. } => Kind::Macro,
            PadAction::LoopKey(_) => Kind::LoopKey,
            PadAction::LoopMute(_) => Kind::LoopMute,
            PadAction::Mute(_) => Kind::TrackMute,
            PadAction::Scene(_) => Kind::Scene,
        })
    }

    /// The pad's action made this kind: a loop's key and its mute swap
    /// keeping the loop; anything else starts at the first choice.
    pub fn action(self, from: PadAction) -> PadAction {
        match (self, from) {
            (k, a) if Kind::of(a) == Some(k) => a,
            (Kind::LoopKey, PadAction::LoopMute(n)) => PadAction::LoopKey(n),
            (Kind::LoopMute, PadAction::LoopKey(n)) => PadAction::LoopMute(n),
            (Kind::Throw, _) => PadAction::Throw(Throw::new(0, Hold::WhileHeld)),
            (Kind::Macro, _) => PadAction::Macro {
                card: 0,
                value: 100,
            },
            (Kind::LoopKey, _) => PadAction::LoopKey(0),
            (Kind::LoopMute, _) => PadAction::LoopMute(0),
            (Kind::TrackMute, _) => PadAction::Mute(0),
            (Kind::Scene, _) => PadAction::Scene(0),
        }
    }
}

/// The project's tracks, scenes and macros as the view names them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Names {
    /// Each track's name and what it plays ("Kit", "FM · Glass Keys").
    pub tracks: Vec<(String, String)>,
    /// Each scene row's name, as far as the project names them.
    pub scenes: Vec<String>,
    /// Each macro's name ("Macro 3" when not named) and how many knobs it
    /// moves, as far as the screen knows them.
    pub macros: Vec<(String, usize)>,
    /// The strips a throw can play on, as the Mix tab names them: the
    /// master first ([`crate::mixing::throw_strips`]).
    pub strips: Vec<(String, StripId)>,
}

impl Names {
    /// Strip `on` by name ("Drums", "Sub 2"), and whether it is one a
    /// throw can play on now (a track's own channel is only while in use).
    fn strip(&self, on: StripId) -> (String, bool) {
        match self.strips.iter().find(|(_, s)| *s == on) {
            Some((n, _)) => (n.clone(), true),
            None => (crate::mixing::name(on), on == StripId::Master),
        }
    }

    /// Where a throw plays, in a sentence: "the master", "Drums".
    fn on_words(&self, on: StripId) -> String {
        match on {
            StripId::Master => "the master".to_string(),
            on => self.strip(on).0,
        }
    }

    /// The strips a throw steps through: the master, then the rest.
    fn throw_strips(&self) -> Vec<StripId> {
        let mut out = vec![StripId::Master];
        out.extend(
            self.strips
                .iter()
                .map(|(_, s)| *s)
                .filter(|&s| s != StripId::Master),
        );
        out
    }

    fn track(&self, t: usize) -> String {
        self.tracks
            .get(t)
            .map_or_else(|| format!("Track {}", t + 1), |(n, _)| n.clone())
    }

    fn scene(&self, n: usize) -> String {
        match self.scenes.get(n).filter(|s| !s.is_empty()) {
            Some(s) => format!("Scene {} · {s}", n + 1),
            None => format!("Scene {}", n + 1),
        }
    }

    /// Macro `m`'s own name, if it has one other than "Macro n".
    fn macro_own(&self, m: usize) -> Option<&str> {
        let plain = format!("Macro {}", m + 1);
        self.macros
            .get(m)
            .map(|(n, _)| n.as_str())
            .filter(|n| !n.is_empty() && *n != plain)
    }

    /// Macro `m` on its pad: its own name ("Dark"), else "Macro 3".
    fn macro_name(&self, m: usize) -> String {
        self.macro_own(m)
            .map_or_else(|| format!("Macro {}", m + 1), str::to_string)
    }

    /// Macro `m` in a sentence: "Macro 3 · Dark", or "Macro 3".
    fn macro_full(&self, m: usize) -> String {
        match self.macro_own(m) {
            Some(n) => format!("Macro {} · {n}", m + 1),
            None => format!("Macro {}", m + 1),
        }
    }

    /// The knobs macro `m` moves.
    fn knobs(&self, m: usize) -> usize {
        self.macros.get(m).map_or(0, |(_, k)| *k)
    }
}

/// "1 knob", "4 knobs".
fn knobs_text(k: usize) -> String {
    match k {
        1 => "1 knob".to_string(),
        k => format!("{k} knobs"),
    }
}

/// Scene rows a pad can launch: the grid's rows.
const SCENES: usize = mpfree_seq::scenes::SCENES;
/// Loops a key can be.
const LOOPS: usize = mpfree_looper::MAX_LOOPS;

/// One edit, for UNDO: a pad's action, or a page's name.
#[derive(Clone, Debug, PartialEq)]
enum Edit {
    Pad {
        page: usize,
        pad: usize,
        before: PadAction,
        after: PadAction,
    },
    Name {
        page: usize,
        before: String,
        after: String,
    },
    /// A run of turns of Q-Link `q` (0: the Length, 1: the Amount) on
    /// throw `fx`'s pad of strip `on`'s Perform FX, playing `effect`.
    Knob {
        on: StripId,
        fx: ThrowFx,
        effect: u8,
        q: usize,
        before: f32,
        after: f32,
    },
}

/// What the view asks of the screen.
#[derive(Debug, PartialEq)]
pub enum Act {
    /// For the app.
    Send(FromUi),
    /// A throw or a macro pad (`a`) held or let go: a finger on the
    /// map's pad.
    Hear(PadAction, bool),
    /// Name page `n` (0 is A) on the text keyboard: its name now.
    Name(usize, String),
    /// A line for the hint and the log.
    Tell(String),
}

/// The Perform view's state.
#[derive(Debug, Default)]
pub struct PerformTab {
    /// The project's set, as the app last sent it.
    pub set: PerformSet,
    /// The page shown: the app's PAD BANK page for the set.
    pub page: usize,
    /// The pad picked (0 is pad 1).
    pub pad: usize,
    /// The set is on the pads (Play and the Looper tab play it).
    pub on: bool,
    /// The Pads tab shows the set, not the kit.
    pub shown: bool,
    built: bool,
    copied: Option<PadAction>,
    lit: [bool; PADS],
    /// The throw or macro a finger on the map's pad plays.
    hearing: Option<PadAction>,
    done: Vec<Edit>,
    undone: Vec<Edit>,
    /// Each throw of the set as its strip's Perform FX has it, as the app
    /// last sent them, with this view's turns since.
    pub knobs: Vec<ThrowKnobs>,
    /// The Q-Link turned last, while its run of turns goes on (one UNDO
    /// step), and the parameter it moves (let go when the finger lifts).
    run: Option<(
        StripId,
        ThrowFx,
        usize,
        mpfree_play::mpfree_param::ParamAddr,
    )>,
}

/// Pad `pad` of page `page` by name ("B2").
pub fn pad_label(page: usize, pad: usize) -> String {
    format!("{}{}", (b'A' + (page % PAGES) as u8) as char, pad + 1)
}

/// What `a` is called on its pad ("Reverb wash", "Loop 2", "Drums").
pub fn action_name(a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => String::new(),
        PadAction::Throw(t) => t.name().to_string(),
        PadAction::Macro { card, .. } => names.macro_name(usize::from(card)),
        PadAction::LoopKey(n) | PadAction::LoopMute(n) => format!("Loop {}", n + 1),
        PadAction::Mute(t) => names.track(usize::from(t)),
        PadAction::Scene(n) => names.scene(usize::from(n)),
    }
}

/// A throw's Hold in words.
fn hold_text(h: Hold) -> &'static str {
    match h {
        Hold::WhileHeld => "While held",
        Hold::Latch => "Latch",
        Hold::Bar => "1 bar",
    }
}

/// A throw's Starts in words.
fn starts_text(s: Starts) -> &'static str {
    match s {
        Starts::AtOnce => "At once",
        Starts::Sixteenth => "Next 1/16",
        Starts::Beat => "Next beat",
    }
}

/// A throw's Let go in words.
fn let_go_text(l: LetGo) -> &'static str {
    match l {
        LetGo::RingOut => "Ring out",
        LetGo::Cut => "Cut",
    }
}

/// When a throw starts, in a sentence: "from the next 1/16".
fn starts_words(s: Starts) -> &'static str {
    match s {
        Starts::AtOnce => "at once",
        Starts::Sixteenth => "from the next 1/16",
        Starts::Beat => "from the next beat",
    }
}

/// What a throw's tail does when it ends, in a sentence.
fn let_go_words(l: LetGo) -> &'static str {
    match l {
        LetGo::RingOut => "it rings out",
        LetGo::Cut => "it stops at once, tail and all",
    }
}

/// What `a` does, in a few words (the bar's line, the log).
pub fn action_summary(a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => "Nothing on it".to_string(),
        PadAction::Throw(t) => format!(
            "Throw · {}, on {}",
            match t.hold {
                Hold::WhileHeld => "while held",
                Hold::Latch => "latched",
                Hold::Bar => "for a bar",
            },
            names.on_words(t.on)
        ),
        PadAction::Macro { card, value } => format!(
            "Macro · {} at {value} % while held",
            names.macro_full(usize::from(card))
        ),
        PadAction::LoopKey(n) => format!("Loop key · the looper's Loop {}", n + 1),
        PadAction::LoopMute(n) => format!("Loop mute · the looper's Loop {}", n + 1),
        PadAction::Mute(t) => format!("Track mute · {}", names.track(usize::from(t))),
        PadAction::Scene(n) => format!("Scene · launches row {}", u16::from(n) + 1),
    }
}

/// What a hit on a pad doing `a` does, for under the Pad list.
pub fn hit_text(label: &str, a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => format!("Nothing yet: pick what {label} does above."),
        PadAction::Throw(t) => {
            let on = names.on_words(t.on);
            let (_, live) = names.strip(t.on);
            if !live {
                return format!(
                    "{on}'s channel plays nothing now (no instrument or kit of its own), so \
                     {} would be heard on nothing: pick another On.",
                    t.name()
                );
            }
            match t.hold {
                Hold::WhileHeld => format!(
                    "Held, it plays {} on {on} {}; let go, {}.",
                    t.name(),
                    starts_words(t.starts),
                    let_go_words(t.let_go)
                ),
                Hold::Latch => format!(
                    "A hit starts {} on {on} {} and the next lets it go ({}).",
                    t.name(),
                    starts_words(t.starts),
                    let_go_words(t.let_go)
                ),
                Hold::Bar => format!(
                    "A hit plays {} on {on} for a bar {}, however briefly the pad is held, and \
                     a hit while it plays starts it again; then {}.",
                    t.name(),
                    starts_words(t.starts),
                    let_go_words(t.let_go)
                ),
            }
        }
        PadAction::Macro { card, value } => {
            let m = usize::from(card);
            match names.knobs(m) {
                0 => format!(
                    "Held, it turns {} to {value} %, but the macro moves no knobs yet: put \
                     some on it from an effect's knobs in the chain editor.",
                    names.macro_full(m)
                ),
                k => format!(
                    "Held, it turns {} to {value} % and its {} with it; let go, they go back \
                     where they were.",
                    names.macro_full(m),
                    knobs_text(k)
                ),
            }
        }
        PadAction::LoopKey(n) => format!(
            "A hit is Loop {}'s next step, as a tap on its ring: record from the next bar \
             line, end the first pass, overdub, play. Held for a second, it clears the loop.",
            n + 1
        ),
        PadAction::LoopMute(n) => format!(
            "A hit mutes or unmutes Loop {}; with SHIFT held it solos it.",
            n + 1
        ),
        PadAction::Mute(t) => format!(
            "A hit mutes or unmutes {} at the launch line in Perform, at once in Make \
             (SHIFT gives the other timing), as TRACK MUTE's pads do.",
            names.track(usize::from(t))
        ),
        PadAction::Scene(n) => format!(
            "A hit launches {}'s row, as its scene button on the grid does.",
            names.scene(usize::from(n))
        ),
    }
}

/// The colour of a pad doing `a`.
fn action_color(a: PadAction) -> Color {
    match a {
        PadAction::None => Color::rgb(0x8A93A6),
        PadAction::Throw(t) => THROW_LIST
            .get(usize::from(t.index))
            .map_or(mpfree_ui::draw::FX, |d| d.color),
        PadAction::LoopKey(n) | PadAction::LoopMute(n) => {
            let c = crate::looper_tab::LOOP_COLOURS;
            c[usize::from(n) % c.len()]
        }
        PadAction::Mute(_) => mpfree_ui::draw::FX,
        PadAction::Scene(_) => mpfree_ui::draw::GREEN,
        PadAction::Macro { .. } => MACRO_COLOR,
    }
}

/// A macro pad's violet.
const MACRO_COLOR: Color = Color::rgb(0xB080F0);

/// A pad's cell on the map.
fn cell(page: usize, pad: usize, a: PadAction, names: &Names) -> PadCell {
    let (detail, badge) = match a {
        PadAction::None => (String::new(), None),
        PadAction::Throw(t) => {
            let on = match t.on {
                StripId::Master => "master".to_string(),
                on => names.strip(on).0,
            };
            (
                match t.hold {
                    Hold::WhileHeld => on,
                    Hold::Latch => format!("latch · {on}"),
                    Hold::Bar => format!("1 bar · {on}"),
                },
                Some("THROW"),
            )
        }
        // The knob count is on the chips and the bar: a cell is narrow.
        PadAction::Macro { value, .. } => (format!("to {value} %"), Some("MACRO")),
        PadAction::LoopKey(_) => ("its key".to_string(), Some("LOOP")),
        PadAction::LoopMute(_) => ("its mute".to_string(), Some("LOOP MUTE")),
        PadAction::Mute(_) => ("track mute".to_string(), Some("MUTE")),
        PadAction::Scene(_) => ("launches it".to_string(), Some("SCENE")),
    };
    PadCell {
        label: pad_label(page, pad),
        name: action_name(a, names),
        detail,
        clip: None,
        color: action_color(a),
        badge: badge.map(str::to_string),
    }
}

/// The page's pads with an action, pad 1 the lowest bit.
fn used(set: &PerformSet, page: usize) -> u16 {
    set.page(page)
        .pads
        .iter()
        .enumerate()
        .filter(|(_, a)| **a != PadAction::None)
        .fold(0, |m, (i, _)| m | 1 << i)
}

/// The first of the eight choices shown for `n` of `len` (a window of
/// eight holding it).
fn window(n: usize, len: usize) -> usize {
    let first = (n / 8) * 8;
    first.min(len.saturating_sub(1) / 8 * 8)
}

impl PerformTab {
    /// The page's action pads and their settings in `ui`, once.
    pub fn build(&mut self, ui: &mut Ui) -> Result<(), String> {
        if self.built {
            return Ok(());
        }
        let mut bar = PadBar::new(&[]);
        bar.button = Some("On the pads: off".into());
        bar.switch = Some(false);
        ui.add(BAR, bar);
        // The Pads of row is the kit view's; the app fills it.
        if ui.get::<KitTracks>(KITS).is_none() {
            ui.add(KITS, KitTracks::new());
        }
        ui.add(MAP, PadMap::new("Perform set"));
        ui.add(
            BUTTONS,
            ButtonRow::new(&["Copy", "Paste", "Clear", "Name page"]),
        );
        ui.add(TYPE, SectionChips::new(&Kind::ALL.map(Kind::label)));
        ui.add(PICK_A, SectionChips::new(&[]));
        ui.add(PICK_B, SectionChips::new(&[]));
        ui.add(LIST, SettingList::new("Pad"));
        ui.add(
            ACTS,
            ButtonRow::new(&["Edit the effect ›", "Hear it", "Learn a key"]),
        );
        ui.add_page(PAGE, LAYOUT)?;
        self.built = true;
        Ok(())
    }

    /// The set as the app has it now: after an edit, or a project's
    /// (`opened`: a project opened or New, whose set has a history of its
    /// own).
    pub fn set_set(&mut self, set: PerformSet, opened: bool) {
        if opened {
            self.done.clear();
            self.undone.clear();
        }
        self.set = set;
    }

    /// UNDO has an edit to take back.
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// The action of the pad picked.
    pub fn picked(&self) -> PadAction {
        self.set.page(self.page).action(self.pad)
    }

    /// The Pad list's rows for `a`.
    fn rows(a: PadAction, names: &Names) -> Vec<Setting> {
        let row = |name: &str, value: String| Setting {
            name: name.to_string(),
            value,
        };
        // What the pad does, and its choice, are the chips above.
        let mut rows = Vec::new();
        match a {
            PadAction::None => {}
            PadAction::Throw(t) => {
                rows.push(row("On", names.strip(t.on).0));
                rows.push(row("Hold", hold_text(t.hold).to_string()));
                rows.push(row("Starts", starts_text(t.starts).to_string()));
                rows.push(row("Let go", let_go_text(t.let_go).to_string()));
            }
            PadAction::Macro { card, value } => {
                rows.push(row("Macro", names.macro_full(usize::from(card))));
                rows.push(row("Value", format!("{value} %")));
            }
            PadAction::LoopKey(n) | PadAction::LoopMute(n) => {
                rows.push(row("Loop", format!("Loop {}", n + 1)));
            }
            PadAction::Mute(t) => {
                rows.push(row("Track", names.track(usize::from(t))));
            }
            PadAction::Scene(n) => {
                rows.push(row("Scene", names.scene(usize::from(n))));
            }
        }
        rows
    }

    /// The eight choices of `a`'s kind shown (four a row), the first's
    /// place, and the one `a` is.
    fn choices(a: PadAction, names: &Names) -> (Vec<SectionChip>, usize, Option<usize>) {
        let chip = |name: String, sub: String| SectionChip { name, sub };
        match a {
            PadAction::None => (Vec::new(), 0, None),
            PadAction::Throw(t) => (
                THROW_LIST
                    .iter()
                    .map(|d| chip(d.name.to_string(), d.sub.to_string()))
                    .collect(),
                0,
                Some(usize::from(t.index)),
            ),
            PadAction::Macro { card, .. } => (
                (0..MACROS)
                    .map(|m| {
                        let knobs = knobs_text(names.knobs(m));
                        let sub = match names.macro_own(m) {
                            Some(n) => format!("{n} · {knobs}"),
                            None => knobs,
                        };
                        chip(format!("Macro {}", m + 1), sub)
                    })
                    .collect(),
                0,
                Some(usize::from(card)),
            ),
            PadAction::LoopKey(n) | PadAction::LoopMute(n) => (
                (0..LOOPS)
                    .map(|k| chip(format!("Loop {}", k + 1), String::new()))
                    .collect(),
                0,
                Some(usize::from(n)),
            ),
            PadAction::Mute(t) => {
                let t = usize::from(t);
                let len = names.tracks.len().max(t + 1);
                let first = window(t, len);
                let chips = (first..len.min(first + 8))
                    .map(|k| {
                        let sound = names.tracks.get(k).map(|(_, s)| s.clone());
                        chip(names.track(k), sound.unwrap_or_default())
                    })
                    .collect();
                (chips, first, Some(t))
            }
            PadAction::Scene(n) => {
                let n = usize::from(n);
                let first = window(n, SCENES);
                let chips = (first..SCENES.min(first + 8))
                    .map(|k| {
                        let name = names.scenes.get(k).cloned().unwrap_or_default();
                        chip(format!("Scene {}", k + 1), name)
                    })
                    .collect();
                (chips, first, Some(n))
            }
        }
    }

    /// Shows the set's page and the pad picked.
    pub fn show(&self, ui: &mut Ui, names: &Names) {
        let page = self.page.min(PAGES - 1);
        let a = self.picked();
        let label = pad_label(page, self.pad);
        let page_name = self.set.page_name(page);
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = action_color(a);
            bar.title = match a {
                PadAction::None => format!("Perform pads {label} · nothing"),
                a => format!("Perform pads {label} · {}", action_name(a, names)),
            };
            bar.subtitle = format!(
                "{} · page {} of the project's Perform set",
                action_summary(a, names),
                (b'A' + page as u8) as char
            );
            bar.button = Some(if self.on {
                "On the pads: on".to_string()
            } else {
                "On the pads: off".to_string()
            });
            bar.switch = Some(self.on);
        }
        if let Some(m) = ui.get_mut::<PadMap>(MAP) {
            m.title = "Perform set".to_string();
            m.right = page_name.clone();
            m.cells = (0..PADS)
                .map(|p| cell(page, p, self.set.page(page).action(p), names))
                .collect();
            m.selected = self.pad;
            m.lit = self.lit.to_vec();
            m.banks = (0..PAGES).map(|k| used(&self.set, k)).collect();
            m.bank = page;
        }
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels = vec![
                "Copy".to_string(),
                match self.copied {
                    Some(_) => format!("Paste {label}"),
                    None => "Paste".to_string(),
                },
                "Clear".to_string(),
                "Name page".to_string(),
            ];
            b.enabled = vec![
                a != PadAction::None,
                self.copied.is_some(),
                a != PadAction::None,
                true,
            ];
            b.why = vec![
                format!("{label} has nothing on it to copy"),
                "Copy a pad first".to_string(),
                format!("{label} has nothing on it"),
                String::new(),
            ];
        }
        if let Some(b) = ui.get_mut::<ButtonRow>(ACTS) {
            let throw = match a {
                PadAction::Throw(t) => t.fx().map(|fx| (t, fx)),
                _ => None,
            };
            let why = match a {
                PadAction::None => format!("{label} has nothing on it"),
                PadAction::Throw(_) => {
                    format!("{label}'s Drop drums mutes the drums: it has no effect")
                }
                PadAction::Macro { .. } => {
                    format!("{label} holds a macro: hold the pad on the map to hear it")
                }
                _ => format!("{label} plays on Play and the Looper tab with the set on the pads"),
            };
            b.labels = vec![
                "Edit the effect ›".to_string(),
                "Hear it".to_string(),
                "Learn a key".to_string(),
            ];
            // A key learned on an empty pad plays what the pad gets later.
            b.enabled = vec![throw.is_some(), throw.is_some(), true];
            b.why = vec![why.clone(), why, String::new()];
        }
        if let Some(c) = ui.get_mut::<SectionChips>(TYPE) {
            c.chips = Kind::ALL
                .iter()
                .map(|k| SectionChip {
                    name: k.label().to_string(),
                    sub: k.sub().to_string(),
                })
                .collect();
            c.selected = Kind::of(a)
                .and_then(|k| Kind::ALL.iter().position(|&x| x == k))
                .unwrap_or(usize::MAX);
        }
        let (chips, first, now) = Self::choices(a, names);
        let at = now.and_then(|n| n.checked_sub(first));
        let (top, bottom) = chips.split_at(chips.len().min(4));
        for (id, part, offset) in [(PICK_A, top, 0), (PICK_B, bottom, 4)] {
            if let Some(c) = ui.get_mut::<SectionChips>(id) {
                c.chips = part.to_vec();
                c.selected = at
                    .and_then(|i| i.checked_sub(offset))
                    .filter(|&i| i < part.len())
                    .unwrap_or(usize::MAX);
            }
        }
        if let Some(l) = ui.get_mut::<SettingList>(LIST) {
            l.title = format!("Pad {label}");
            l.right = page_name;
            l.rows = Self::rows(a, names);
            l.selected = l.selected.min(l.rows.len().saturating_sub(1));
            let live = format!(
                "Live: On the pads (or SHIFT + SAMPLER on the Looper tab) puts page {} on the \
                 pads on Play and the Looper tab, and PAD BANK turns its pages.",
                (b'A' + page as u8) as char
            );
            // A throw's four rows leave room for what a hit does only; a
            // throw on a channel that plays nothing warns.
            let hit = hit_text(&label, a, names);
            let silent = matches!(a, PadAction::Throw(t) if !names.strip(t.on).1);
            l.footer = Some((
                if l.rows.len() > 3 {
                    hit
                } else {
                    format!("{hit} {live}")
                },
                silent,
            ));
        }
    }

    /// The picked pad's throw, its effect, and that effect's settings on
    /// its strip (`None` while the app hasn't sent them).
    fn picked_knobs(&self) -> Option<(Throw, ThrowFx, Option<&ThrowKnobs>)> {
        let PadAction::Throw(t) = self.picked() else {
            return None;
        };
        let fx = t.fx()?;
        let k = self.knobs.iter().find(|k| k.on == t.on && k.fx == fx);
        Some((t, fx, k))
    }

    /// Throw `fx`'s setting `q` on strip `on` is `v` now (a turn or its
    /// UNDO): its address for the app, while the strip has its Perform
    /// FX.
    fn set_knob(
        &mut self,
        on: StripId,
        fx: ThrowFx,
        q: usize,
        v: f32,
    ) -> Option<mpfree_play::mpfree_param::ParamAddr> {
        let k = self
            .knobs
            .iter_mut()
            .find(|k| k.on == on && k.fx == fx && k.uid.is_some())?;
        if q == 0 {
            k.length = v;
        } else {
            k.amount = v;
        }
        k.addr(knob_param(q))
    }

    /// The Q-Link column on this view: the picked throw's Length and
    /// Amount (Q1 and Q2, named for its effect) on the Perform FX of the
    /// strip its On picks. Q3 and Q4, and both while that strip has no
    /// Perform FX or the pad is no throw, are faint ([`Self::qlink_why`]).
    pub fn qlink_cards(&self, names: &Names) -> [QLinkCard; 4] {
        let mut cards: [QLinkCard; 4] = Default::default();
        let Some((t, _, Some(k))) = self.picked_knobs() else {
            return cards;
        };
        if k.uid.is_none() {
            return cards;
        }
        // Not drawn (Jay, 17 Nov: the cards name the knob alone); the
        // hint says it.
        let scope = format!("{} · every {} there", names.strip(t.on).0, t.name());
        for (q, card) in cards.iter_mut().enumerate().take(2) {
            if let Some(name) = knob_name(k.effect, q) {
                let v = if q == 0 { k.length } else { k.amount };
                *card = QLinkCard::new(name, &scope, &knob_text(q, v), knob_pos(q, v));
            }
        }
        cards
    }

    /// Why Q-Link `q` turns nothing on this view now, for the hint; `None`
    /// when it turns the picked throw's setting.
    pub fn qlink_why(&self, q: usize, names: &Names) -> Option<String> {
        let label = pad_label(self.page, self.pad);
        let a = self.picked();
        let Some((t, _, k)) = self.picked_knobs() else {
            return Some(match a {
                PadAction::None => {
                    format!("{label} has nothing on it: the Q-Links turn a throw pad's effect")
                }
                PadAction::Throw(_) => {
                    format!("{label}'s Drop drums mutes the drums: it has no effect to turn")
                }
                PadAction::Macro { .. } => format!(
                    "{label} holds a macro: its Value is in the Pad list, and the Q-Links \
                     turn a throw pad's effect"
                ),
                _ => format!("The Q-Links turn a throw pad's effect: {label} is no throw"),
            });
        };
        let k = match k {
            Some(k) if k.uid.is_some() => k,
            _ => {
                // The words a hit says (`MixHost::throw_waits`).
                return Some(format!(
                    "{} has no Perform FX for the throw: pick the pad's On again",
                    names.strip(t.on).0
                ));
            }
        };
        if q >= 2 {
            return Some(format!(
                "A throw has two settings, on Q1 and Q2: {} has no others here",
                t.name()
            ));
        }
        if knob_name(k.effect, q).is_some() {
            return None;
        }
        let effect = mpfree_fx::PERFORM_EFFECTS
            .get(usize::from(k.effect))
            .copied()
            .unwrap_or("Off");
        Some(match knob_name(k.effect, 1) {
            Some(other) => format!("{effect} has no length: Q2 turns its {other}"),
            None => format!("{label}'s Perform FX pad is Off: Edit the effect picks one"),
        })
    }

    /// What the hint says the Q-Links do with the pad picked, if it is a
    /// throw on a strip with its Perform FX: "Q1 Time, Q2 Feedback: every
    /// Delay throw on Drums".
    pub fn qlink_hint(&self, names: &Names) -> Option<String> {
        let (t, _, Some(k)) = self.picked_knobs()? else {
            return None;
        };
        k.uid?;
        let knobs: Vec<String> = (0..2)
            .filter_map(|q| knob_name(k.effect, q).map(|n| format!("Q{} {n}", q + 1)))
            .collect();
        (!knobs.is_empty()).then(|| {
            format!(
                "{}: every {} on {}",
                knobs.join(", "),
                t.name(),
                names.on_words(t.on)
            )
        })
    }

    /// Q-Link `q` turned `detents` (`fine`: SHIFT held): the picked
    /// throw's setting on its strip's Perform FX, for the app. A run of
    /// turns of one knob is one UNDO step; it ends when the finger lifts
    /// ([`Self::qlink_lift`]) or another edit comes.
    pub fn qlink(&mut self, q: usize, detents: i32, fine: bool, names: &Names) -> Vec<Act> {
        if detents == 0 {
            return Vec::new();
        }
        if let Some(why) = self.qlink_why(q, names) {
            return vec![Act::Tell(why)];
        }
        let Some((t, fx, Some(k))) = self.picked_knobs() else {
            return Vec::new();
        };
        let (on, effect) = (t.on, k.effect);
        let before = if q == 0 { k.length } else { k.amount };
        let after = knob_turned(q, before, detents, fine);
        if after == before {
            return Vec::new();
        }
        let Some(addr) = self.set_knob(on, fx, q, after) else {
            return Vec::new();
        };
        let same_run = self
            .run
            .is_some_and(|(o, f, k, _)| (o, f, k) == (on, fx, q));
        match self.done.last_mut() {
            Some(Edit::Knob { after: a, .. }) if same_run => *a = after,
            _ => {
                self.done.push(Edit::Knob {
                    on,
                    fx,
                    effect,
                    q,
                    before,
                    after,
                });
                self.undone.clear();
            }
        }
        self.run = Some((on, fx, q, addr));
        vec![Act::Send(FromUi::ThrowKnob { addr, value: after })]
    }

    /// A finger left Q-Link `q`: the run of turns ends (the next turn is
    /// another UNDO step), the app lets go of the parameter, and the log
    /// says where the run left it ("Perform pads: Delay throw on Drums:
    /// Feedback 62 %").
    pub fn qlink_lift(&mut self, q: usize, names: &Names) -> Vec<Act> {
        match self.run {
            Some((_, _, k, addr)) if k == q => {
                self.run = None;
                let mut out = vec![Act::Send(FromUi::ThrowKnobLift(addr))];
                if let Some(e @ Edit::Knob { .. }) = self.done.last() {
                    out.push(Act::Tell(format!("Perform pads: {}", knob_row(e, names))));
                }
                out
            }
            _ => Vec::new(),
        }
    }

    /// Learn a key (board 7): MIDI Learn waits for a key, pad or pedal on
    /// outside gear to play the picked pad, as SHIFT + a touch hands it
    /// any value; again, while it waits, cancels.
    fn learn(&self) -> Act {
        use mpfree_play::mpfree_param::{perform, Owner, ParamAddr};
        let id = perform::pad(self.page.min(PAGES - 1) as u8, self.pad.min(PADS - 1) as u8);
        Act::Send(FromUi::LearnValue(crate::gui::LearnValue::Param(
            ParamAddr::new(Owner::Perform, id),
        )))
    }

    /// SHIFT + a touch on a pad of the map: the pad picked, and Learn a key
    /// for it (it doesn't play). None for any other touch.
    pub fn learn_touch(&mut self, ev: &UiEvent) -> Option<Vec<Act>> {
        let k = match (ev.widget.as_str(), &ev.kind) {
            (MAP, EventKind::Select(k))
            | (
                MAP,
                EventKind::Pad {
                    index: k,
                    down: true,
                },
            ) => *k,
            _ => return None,
        };
        if k >= PADS {
            return None;
        }
        self.pad = k;
        Some(vec![self.learn()])
    }

    /// Pad `pad` (0 is pad 1) hit on the hardware: picked.
    pub fn hit(&mut self, pad: usize) {
        if pad < PADS {
            self.pad = pad;
            self.lit[pad] = true;
        }
    }

    /// Pad `pad` let go on the hardware.
    pub fn up(&mut self, pad: usize) {
        if let Some(l) = self.lit.get_mut(pad) {
            *l = false;
        }
    }

    /// Pad `pad` of the page shown does `after` now: an UNDO step, the
    /// app told, and a line for the log.
    fn edit(&mut self, pad: usize, after: PadAction, names: &Names) -> Vec<Act> {
        let page = self.page.min(PAGES - 1);
        let before = self.set.page(page).action(pad);
        if before == after {
            return Vec::new();
        }
        self.done.push(Edit::Pad {
            page,
            pad,
            before,
            after,
        });
        self.undone.clear();
        self.run = None;
        self.apply(page, pad, after);
        let label = pad_label(page, pad);
        let line = match after {
            PadAction::None => format!("Perform pads {label} cleared"),
            a => format!("Perform pads {label}: {}", edit_text(a, names)),
        };
        vec![
            Act::Send(FromUi::PerformPad {
                page,
                pad,
                action: after,
            }),
            Act::Tell(line),
        ]
    }

    /// Page `page` named `name` (typed on the text keyboard): an UNDO
    /// step, the app told, and a line for the log.
    pub fn rename(&mut self, page: usize, name: &str) -> Vec<Act> {
        let Some(p) = self.set.pages.get_mut(page) else {
            return Vec::new();
        };
        let after: String = name.trim().chars().take(MAX_PAGE_NAME).collect();
        if p.name == after {
            return Vec::new();
        }
        let before = std::mem::replace(&mut p.name, after.clone());
        self.run = None;
        self.done.push(Edit::Name {
            page,
            before,
            after: after.clone(),
        });
        self.undone.clear();
        let letter = (b'A' + page as u8) as char;
        vec![
            Act::Send(FromUi::PerformName {
                page,
                name: after.clone(),
            }),
            Act::Tell(format!("Perform pads: page {letter} named {after}")),
        ]
    }

    /// Puts `a` on the copy kept here, as the app will.
    fn apply(&mut self, page: usize, pad: usize, a: PadAction) {
        if let Some(slot) = self
            .set
            .pages
            .get_mut(page)
            .and_then(|p| p.pads.get_mut(pad))
        {
            *slot = a;
        }
    }

    /// UNDO (or SHIFT + UNDO, `redo`): the last edit taken back, or
    /// made again. The page and pad it was on show.
    pub fn undo(&mut self, redo: bool, names: &Names) -> Vec<Act> {
        let (from, to) = if redo {
            (&mut self.undone, &mut self.done)
        } else {
            (&mut self.done, &mut self.undone)
        };
        let Some(e) = from.pop() else {
            return vec![Act::Tell(if redo {
                "Nothing to redo on the Perform set".to_string()
            } else {
                "Nothing to undo on the Perform set".to_string()
            })];
        };
        to.push(e.clone());
        self.run = None;
        let how = if redo { "Redo" } else { "Undo" };
        let mut out = Vec::new();
        let page = match &e {
            Edit::Pad { page, .. } | Edit::Name { page, .. } => Some(*page),
            Edit::Knob { .. } => None,
        };
        if let Some(page) = page.filter(|&p| p != self.page) {
            self.page = page;
            out.push(Act::Send(FromUi::PerformPage(page)));
        }
        match e {
            Edit::Knob {
                on,
                fx,
                effect,
                q,
                before,
                after,
            } => {
                let v = if redo { after } else { before };
                let what = format!(
                    "{}'s {} on {}",
                    throw_name(fx),
                    knob_name(effect, q).unwrap_or("setting"),
                    names.on_words(on)
                );
                match self.set_knob(on, fx, q, v) {
                    Some(addr) => {
                        out.push(Act::Send(FromUi::ThrowKnob { addr, value: v }));
                        out.push(Act::Tell(format!(
                            "{how}: {what} back to {}",
                            knob_text(q, v)
                        )));
                    }
                    None => out.push(Act::Tell(format!(
                        "{how}: {} has no Perform FX now, so {what} can't go back",
                        names.strip(on).0
                    ))),
                }
            }
            Edit::Pad {
                page,
                pad,
                before,
                after,
            } => {
                let a = if redo { after } else { before };
                self.apply(page, pad, a);
                self.pad = pad;
                out.push(Act::Send(FromUi::PerformPad {
                    page,
                    pad,
                    action: a,
                }));
                let label = pad_label(page, pad);
                let what = match a {
                    PadAction::None => "nothing".to_string(),
                    a => edit_text(a, names),
                };
                out.push(Act::Tell(format!(
                    "{how}: Perform pads {label} back to {what}"
                )));
            }
            Edit::Name {
                page,
                before,
                after,
            } => {
                let name = if redo { after } else { before };
                if let Some(p) = self.set.pages.get_mut(page) {
                    p.name = name.clone();
                }
                out.push(Act::Send(FromUi::PerformName {
                    page,
                    name: name.clone(),
                }));
                let letter = (b'A' + page as u8) as char;
                out.push(Act::Tell(if name.is_empty() {
                    format!("{how}: Perform pads page {letter} back to no name")
                } else {
                    format!("{how}: Perform pads page {letter} back to {name}")
                }));
            }
        }
        out
    }

    /// The undo history as the list shows it: its rows ("Start", the
    /// steps UNDO takes back, oldest first, then REDO's) and the row that
    /// is now.
    pub fn history_rows(&self, names: &Names) -> (Vec<String>, usize) {
        let row = |e: &Edit| match e {
            Edit::Pad {
                page, pad, after, ..
            } => {
                let label = pad_label(*page, *pad);
                match *after {
                    PadAction::None => format!("{label} cleared"),
                    a => format!("{label}: {}", edit_text(a, names)),
                }
            }
            Edit::Name { page, after, .. } => {
                format!("Page {} named {after}", (b'A' + *page as u8) as char)
            }
            e @ Edit::Knob { .. } => knob_row(e, names),
        };
        let mut rows = vec!["Start: the set as it was".to_string()];
        rows.extend(self.done.iter().map(row));
        let now = rows.len() - 1;
        rows.extend(self.undone.iter().rev().map(row));
        (rows, now)
    }

    /// The wheel moved the Pad list's highlight `d` rows.
    pub fn pick_row(&self, ui: &mut Ui, d: i32) {
        if let Some(l) = ui.get_mut::<SettingList>(LIST) {
            let last = l.rows.len().saturating_sub(1) as i64;
            l.selected = (l.selected as i64 + i64::from(d)).clamp(0, last) as usize;
        }
    }

    /// The highlighted row of the Pad list stepped `d` (- / +, the wheel
    /// with the row open).
    pub fn step_row(&mut self, ui: &Ui, d: i32, names: &Names) -> Vec<Act> {
        let row = ui.get::<SettingList>(LIST).map_or(0, |l| l.selected);
        self.step(row, d, names)
    }

    /// Row `row` of the Pad list stepped `d`.
    fn step(&mut self, row: usize, d: i32, names: &Names) -> Vec<Act> {
        let a = self.picked();
        let d = d.signum();
        if d == 0 {
            return Vec::new();
        }
        let after = match (row, a) {
            (0, PadAction::Throw(t)) => {
                let strips = names.throw_strips();
                // A strip not on the list now (a track's channel that
                // plays nothing) steps on from the master.
                let at = strips.iter().position(|&s| s == t.on).unwrap_or(0);
                PadAction::Throw(Throw {
                    on: strips[stepped(at, d, strips.len())],
                    ..t
                })
            }
            (1, PadAction::Throw(t)) => {
                const HOLDS: [Hold; 3] = [Hold::WhileHeld, Hold::Latch, Hold::Bar];
                let at = HOLDS.iter().position(|&h| h == t.hold).unwrap_or(0);
                PadAction::Throw(Throw {
                    hold: HOLDS[stepped(at, d, HOLDS.len())],
                    ..t
                })
            }
            (2, PadAction::Throw(t)) => {
                const STARTS: [Starts; 3] = [Starts::AtOnce, Starts::Sixteenth, Starts::Beat];
                let at = STARTS.iter().position(|&s| s == t.starts).unwrap_or(1);
                PadAction::Throw(Throw {
                    starts: STARTS[stepped(at, d, STARTS.len())],
                    ..t
                })
            }
            (3, PadAction::Throw(t)) => PadAction::Throw(Throw {
                let_go: if d > 0 { LetGo::Cut } else { LetGo::RingOut },
                ..t
            }),
            (0, PadAction::Macro { card, value }) => PadAction::Macro {
                card: stepped(usize::from(card), d, MACROS) as u8,
                value,
            },
            (1, PadAction::Macro { card, value }) => PadAction::Macro {
                card,
                value: (i32::from(value) + d * VALUE_STEP).clamp(0, 100) as u8,
            },
            (0, PadAction::LoopKey(n)) => {
                PadAction::LoopKey(stepped(usize::from(n), d, LOOPS) as u8)
            }
            (0, PadAction::LoopMute(n)) => {
                PadAction::LoopMute(stepped(usize::from(n), d, LOOPS) as u8)
            }
            (0, PadAction::Mute(t)) => {
                let len = names.tracks.len().clamp(1, 256);
                PadAction::Mute(stepped(usize::from(t), d, len) as u8)
            }
            (0, PadAction::Scene(n)) => PadAction::Scene(stepped(usize::from(n), d, SCENES) as u8),
            _ => return Vec::new(),
        };
        self.edit(self.pad, after, names)
    }

    /// One of the eight choices tapped: `i` from the first shown.
    fn choose(&mut self, i: usize, names: &Names) -> Vec<Act> {
        let a = self.picked();
        let (chips, first, _) = Self::choices(a, names);
        if i >= chips.len() {
            return Vec::new();
        }
        let n = first + i;
        let after = match a {
            PadAction::None => return Vec::new(),
            PadAction::Throw(t) => PadAction::Throw(Throw {
                index: n as u8,
                ..t
            }),
            PadAction::Macro { value, .. } => PadAction::Macro {
                card: n.min(MACROS - 1) as u8,
                value,
            },
            PadAction::LoopKey(_) => PadAction::LoopKey(n as u8),
            PadAction::LoopMute(_) => PadAction::LoopMute(n as u8),
            PadAction::Mute(_) => PadAction::Mute(n.min(255) as u8),
            PadAction::Scene(_) => PadAction::Scene(n.min(255) as u8),
        };
        self.edit(self.pad, after, names)
    }

    /// A tap on one of the view's widgets: what it asks.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent, names: &Names) -> Vec<Act> {
        let label = pad_label(self.page, self.pad);
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(0)) => vec![Act::Send(if self.on {
                FromUi::PerformOff
            } else {
                FromUi::PerformOn
            })],
            (MAP, EventKind::Tab(k)) if *k < PAGES => {
                if *k == self.page {
                    return Vec::new();
                }
                self.page = *k;
                vec![Act::Send(FromUi::PerformPage(*k))]
            }
            (MAP, EventKind::Select(p)) if *p < PADS => {
                self.pad = *p;
                Vec::new()
            }
            (MAP, EventKind::Pad { index, down: true }) => {
                match self.set.page(self.page).action(*index) {
                    a @ (PadAction::Throw(_) | PadAction::Macro { .. }) => {
                        // A latched throw plays while held here, so
                        // nothing keeps playing.
                        let a = match a {
                            PadAction::Throw(t) if t.hold == Hold::Latch => {
                                PadAction::Throw(Throw {
                                    hold: Hold::WhileHeld,
                                    ..t
                                })
                            }
                            a => a,
                        };
                        self.hearing = Some(a);
                        vec![Act::Hear(a, true)]
                    }
                    PadAction::None => Vec::new(),
                    a => vec![Act::Tell(format!(
                        "{} is a {}: it plays on Play and the Looper tab with the set on the pads",
                        pad_label(self.page, *index),
                        Kind::of(a).map_or("pad", Kind::label).to_lowercase()
                    ))],
                }
            }
            (MAP, EventKind::Pad { down: false, .. }) => match self.hearing.take() {
                Some(a) => vec![Act::Hear(a, false)],
                None => Vec::new(),
            },
            (BUTTONS, EventKind::Tab(COPY)) => {
                let a = self.picked();
                self.copied = (a != PadAction::None).then_some(a);
                vec![Act::Tell(format!(
                    "Copied {label}: {}",
                    edit_text(a, names)
                ))]
            }
            (BUTTONS, EventKind::Tab(PASTE)) => match self.copied {
                Some(a) => self.edit(self.pad, a, names),
                None => Vec::new(),
            },
            (BUTTONS, EventKind::Tab(CLEAR)) => self.edit(self.pad, PadAction::None, names),
            (BUTTONS, EventKind::Tab(NAME)) => {
                let page = self.page.min(PAGES - 1);
                vec![Act::Name(page, self.set.pages[page].name.clone())]
            }
            (BUTTONS, EventKind::Select(i)) => ui
                .get::<ButtonRow>(BUTTONS)
                .and_then(|b| b.why_faint(*i))
                .map(|w| vec![Act::Tell(w.to_string())])
                .unwrap_or_default(),
            (ACTS, EventKind::Tab(EDIT_FX)) => match self.picked() {
                PadAction::Throw(t) => match t.fx() {
                    Some(fx) => vec![Act::Send(FromUi::ThrowEffect { on: t.on, fx })],
                    None => Vec::new(),
                },
                _ => Vec::new(),
            },
            (ACTS, EventKind::Tab(HEAR)) => match self.picked() {
                // A bar of it, as a pad set to 1 bar plays: a tap has no
                // let go. Its Starts and its strip are the pad's.
                PadAction::Throw(t) if t.fx().is_some() => {
                    let a = PadAction::Throw(Throw {
                        hold: Hold::Bar,
                        ..t
                    });
                    vec![
                        Act::Hear(a, true),
                        Act::Hear(a, false),
                        Act::Tell(format!(
                            "{label}: {} for a bar, on {}",
                            t.name(),
                            names.on_words(t.on)
                        )),
                    ]
                }
                _ => Vec::new(),
            },
            (ACTS, EventKind::Tab(LEARN)) => vec![self.learn()],
            (ACTS, EventKind::Select(i)) => ui
                .get::<ButtonRow>(ACTS)
                .and_then(|b| b.why_faint(*i))
                .map(|w| vec![Act::Tell(w.to_string())])
                .unwrap_or_default(),
            (TYPE, EventKind::Tab(i)) => match Kind::ALL.get(*i) {
                Some(k) => {
                    let after = k.action(self.picked());
                    self.edit(self.pad, after, names)
                }
                None => Vec::new(),
            },
            (PICK_A, EventKind::Tab(i)) => self.choose(*i, names),
            (PICK_B, EventKind::Tab(i)) => self.choose(4 + *i, names),
            (LIST, EventKind::Step { index, delta }) => self.step(*index, *delta, names),
            _ => Vec::new(),
        }
    }
}

/// Throw `fx` as Play's card names it ("Delay throw").
fn throw_name(fx: ThrowFx) -> &'static str {
    THROW_LIST
        .iter()
        .find(|t| t.fx == Some(fx))
        .map_or("Throw", |t| t.name)
}

/// `a` for the log ("Reverb wash, latched", "Loop 2's key").
fn edit_text(a: PadAction, names: &Names) -> String {
    match a {
        PadAction::None => "nothing".to_string(),
        PadAction::Throw(t) => {
            let mut text = match t.hold {
                Hold::WhileHeld => format!("{}, while held", t.name()),
                Hold::Latch => format!("{}, latched", t.name()),
                Hold::Bar => format!("{}, for a bar", t.name()),
            };
            match t.starts {
                Starts::AtOnce => text.push_str(", at once"),
                Starts::Sixteenth => {}
                Starts::Beat => text.push_str(", on the beat"),
            }
            if t.let_go == LetGo::Cut {
                text.push_str(", cut");
            }
            if t.on != StripId::Master {
                text.push_str(&format!(", on {}", names.on_words(t.on)));
            }
            text
        }
        PadAction::Macro { card, value } => {
            format!("{} at {value} %", names.macro_full(usize::from(card)))
        }
        PadAction::LoopKey(n) => format!("Loop {}'s key", n + 1),
        PadAction::LoopMute(n) => format!("Loop {}'s mute", n + 1),
        PadAction::Mute(t) => format!("{}'s mute", names.track(usize::from(t))),
        PadAction::Scene(n) => format!("launches {}", names.scene(usize::from(n))),
    }
}

/// `n` stepped `d` within `0..len`, stopping at the ends.
fn stepped(n: usize, d: i32, len: usize) -> usize {
    let last = len.saturating_sub(1) as i64;
    (n as i64 + i64::from(d)).clamp(0, last) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Names {
        Names {
            tracks: vec![
                ("Drums".into(), "Kit".into()),
                ("Bass".into(), "FM · Sub".into()),
                ("Keys".into(), String::new()),
            ],
            scenes: vec![String::new(), "Drop".into()],
            macros: vec![
                ("Macro 1".into(), 0),
                ("Macro 2".into(), 2),
                ("Dark".into(), 4),
            ],
            strips: vec![
                ("Master".into(), StripId::Master),
                ("Drums".into(), StripId::Track(0)),
                ("Synth".into(), StripId::Track(1)),
                ("Bass".into(), StripId::Track(5)),
                ("Sub 1".into(), StripId::Bus(0)),
            ],
        }
    }

    fn sends(acts: &[Act]) -> Vec<&FromUi> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Send(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_kind_keeps_the_loop_and_starts_the_rest_at_the_first() {
        assert_eq!(
            Kind::LoopMute.action(PadAction::LoopKey(3)),
            PadAction::LoopMute(3)
        );
        assert_eq!(
            Kind::LoopKey.action(PadAction::LoopMute(5)),
            PadAction::LoopKey(5)
        );
        assert_eq!(
            Kind::Scene.action(PadAction::LoopKey(5)),
            PadAction::Scene(0)
        );
        let t = PadAction::Throw(Throw::new(3, Hold::Latch));
        assert_eq!(Kind::Throw.action(t), t);
        assert_eq!(Kind::of(PadAction::None), None);
    }

    #[test]
    fn edits_go_to_the_app_and_undo_takes_them_back() {
        let mut tab = PerformTab {
            page: 1,
            pad: 1,
            ..PerformTab::default()
        };
        let n = names();
        // B2 is Reverb wash, held: made a scene.
        let acts = tab.edit(1, Kind::Scene.action(tab.picked()), &n);
        assert_eq!(
            sends(&acts),
            [&FromUi::PerformPad {
                page: 1,
                pad: 1,
                action: PadAction::Scene(0),
            }]
        );
        assert!(acts.contains(&Act::Tell("Perform pads B2: launches Scene 1".into())));
        assert_eq!(tab.picked(), PadAction::Scene(0));
        // Row 2, Drop: the list's only row.
        tab.step(0, 1, &n);
        assert_eq!(tab.picked(), PadAction::Scene(1));
        let (rows, now) = tab.history_rows(&n);
        assert_eq!(
            rows,
            [
                "Start: the set as it was",
                "B2: launches Scene 1",
                "B2: launches Scene 2 · Drop"
            ]
        );
        assert_eq!(now, 2);
        // UNDO twice: Reverb wash again, on page B even from page C.
        tab.page = 2;
        let acts = tab.undo(false, &n);
        assert_eq!(sends(&acts)[0], &FromUi::PerformPage(1));
        tab.undo(false, &n);
        assert_eq!(
            tab.picked(),
            PadAction::Throw(Throw::new(1, Hold::WhileHeld))
        );
        let acts = tab.undo(false, &n);
        assert_eq!(
            acts,
            [Act::Tell("Nothing to undo on the Perform set".into())]
        );
        // SHIFT + UNDO: the scene again.
        tab.undo(true, &n);
        assert_eq!(tab.picked(), PadAction::Scene(0));
        assert_eq!(tab.history_rows(&n).1, 1);
    }

    #[test]
    fn a_throw_steps_its_effect_and_hold() {
        let mut tab = PerformTab {
            page: 1,
            ..PerformTab::default()
        };
        let n = names();
        // The effect is one of the eight chips; Hold is the list's row 1.
        tab.choose(1, &n);
        tab.step(1, 1, &n);
        assert_eq!(tab.picked(), PadAction::Throw(Throw::new(1, Hold::Latch)));
        // Latch, then a bar; the ends hold.
        tab.step(1, 1, &n);
        assert!(tab.step(1, 1, &n).is_empty());
        tab.step(1, -1, &n);
        let a = tab.picked();
        assert_eq!(
            hit_text("B1", a, &n),
            "A hit starts Reverb wash on the master from the next 1/16 and the next lets it go \
             (it rings out)."
        );
        assert_eq!(action_summary(a, &n), "Throw · latched, on the master");
    }

    #[test]
    fn a_throw_steps_its_start_and_let_go_and_says_what_a_hit_does() {
        let mut tab = PerformTab {
            page: 1,
            pad: 1,
            ..PerformTab::default()
        };
        let n = names();
        let rows = |tab: &PerformTab| {
            PerformTab::rows(tab.picked(), &n)
                .into_iter()
                .map(|r| format!("{}: {}", r.name, r.value))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(&tab),
            [
                "On: Master",
                "Hold: While held",
                "Starts: Next 1/16",
                "Let go: Ring out"
            ]
        );
        // A bar, on the beat, cut.
        tab.step(1, 1, &n);
        tab.step(1, 1, &n);
        tab.step(2, 1, &n);
        let acts = tab.step(3, 1, &n);
        assert!(acts.contains(&Act::Tell(
            "Perform pads B2: Reverb wash, for a bar, on the beat, cut".into()
        )));
        assert_eq!(
            tab.picked(),
            PadAction::Throw(Throw {
                starts: Starts::Beat,
                let_go: LetGo::Cut,
                ..Throw::new(1, Hold::Bar)
            })
        );
        assert_eq!(
            hit_text("B2", tab.picked(), &n),
            "A hit plays Reverb wash on the master for a bar from the next beat, however \
             briefly the pad is held, and a hit while it plays starts it again; then it stops \
             at once, tail and all."
        );
        assert_eq!(
            cell(1, 1, tab.picked(), &n).detail,
            "1 bar · master",
            "the map says it plays a bar"
        );
        // At once, the other way.
        tab.step(2, -1, &n);
        tab.step(2, -1, &n);
        assert!(tab.step(2, -1, &n).is_empty(), "the end holds");
        assert_eq!(rows(&tab)[2], "Starts: At once");
    }

    #[test]
    fn a_throw_steps_where_it_plays_and_says_when_that_plays_nothing() {
        let mut tab = PerformTab {
            page: 1,
            pad: 1,
            ..PerformTab::default()
        };
        let n = names();
        // On: the master, then the Mix tab's strips in order.
        let acts = tab.step(0, 1, &n);
        assert!(acts.contains(&Act::Tell(
            "Perform pads B2: Reverb wash, while held, on Drums".into()
        )));
        let t = Throw {
            on: StripId::Track(0),
            ..Throw::new(1, Hold::WhileHeld)
        };
        assert_eq!(tab.picked(), PadAction::Throw(t));
        assert_eq!(PerformTab::rows(tab.picked(), &n)[0].value, "Drums");
        assert_eq!(
            action_summary(tab.picked(), &n),
            "Throw · while held, on Drums"
        );
        assert_eq!(
            hit_text("B2", tab.picked(), &n),
            "Held, it plays Reverb wash on Drums from the next 1/16; let go, it rings out."
        );
        assert_eq!(cell(1, 1, tab.picked(), &n).detail, "Drums");
        // To the last, a submix; the end holds; back to the master.
        for _ in 0..3 {
            tab.step(0, 1, &n);
        }
        assert!(tab.step(0, 1, &n).is_empty());
        assert_eq!(PerformTab::rows(tab.picked(), &n)[0].value, "Sub 1");
        for _ in 0..4 {
            tab.step(0, -1, &n);
        }
        assert_eq!(
            tab.picked(),
            PadAction::Throw(Throw::new(1, Hold::WhileHeld))
        );
        // A track's channel that plays nothing now: said, and it steps on
        // from the master.
        let gone = Throw {
            on: StripId::Track(9),
            ..Throw::new(1, Hold::Latch)
        };
        tab.edit(1, PadAction::Throw(gone), &n);
        assert_eq!(PerformTab::rows(tab.picked(), &n)[0].value, "Track 6");
        assert!(hit_text("B2", tab.picked(), &n).starts_with("Track 6's channel plays nothing now"));
        tab.step(0, 1, &n);
        assert_eq!(PerformTab::rows(tab.picked(), &n)[0].value, "Drums");
    }

    #[test]
    fn a_macro_pad_picks_its_macro_and_value() {
        let mut tab = PerformTab {
            page: 2,
            ..PerformTab::default()
        };
        let n = names();
        // C1, empty: made a macro pad, Macro 1 at 100 %.
        let i = Kind::ALL.iter().position(|&k| k == Kind::Macro).unwrap();
        let mut ui = Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        let ev = UiEvent {
            widget: TYPE.to_string(),
            kind: EventKind::Tab(i),
        };
        tab.event(&mut ui, &ev, &n);
        assert_eq!(
            tab.picked(),
            PadAction::Macro {
                card: 0,
                value: 100
            }
        );
        // The choices: the eight macros, named and with their knobs.
        let (chips, first, now) = PerformTab::choices(tab.picked(), &n);
        assert_eq!((chips.len(), first, now), (8, 0, Some(0)));
        assert_eq!(chips[1].sub, "2 knobs");
        assert_eq!(
            (chips[2].name.as_str(), chips[2].sub.as_str()),
            ("Macro 3", "Dark · 4 knobs")
        );
        assert_eq!(chips[5].sub, "0 knobs");
        // Macro 3, then down to 90 %.
        tab.choose(2, &n);
        tab.step(1, -1, &n);
        tab.step(1, -1, &n);
        assert_eq!(tab.picked(), PadAction::Macro { card: 2, value: 90 });
        assert_eq!(action_name(tab.picked(), &n), "Dark");
        assert_eq!(
            action_summary(tab.picked(), &n),
            "Macro · Macro 3 · Dark at 90 % while held"
        );
        assert_eq!(
            hit_text("C1", tab.picked(), &n),
            "Held, it turns Macro 3 · Dark to 90 % and its 4 knobs with it; let go, they go \
             back where they were."
        );
        assert_eq!(cell(2, 0, tab.picked(), &n).detail, "to 90 %");
        // A macro with no knobs says so in place.
        tab.step(0, 1, &n);
        assert!(hit_text("C1", tab.picked(), &n).contains("moves no knobs yet"));
        // A finger on its pad holds it while down.
        let down = UiEvent {
            widget: MAP.to_string(),
            kind: EventKind::Pad {
                index: 0,
                down: true,
            },
        };
        let up = UiEvent {
            widget: MAP.to_string(),
            kind: EventKind::Pad {
                index: 0,
                down: false,
            },
        };
        let a = tab.picked();
        assert_eq!(tab.event(&mut ui, &down, &n), [Act::Hear(a, true)]);
        assert_eq!(tab.event(&mut ui, &up, &n), [Act::Hear(a, false)]);
    }

    #[test]
    fn name_page_types_the_name_and_undo_takes_it_back() {
        let mut tab = PerformTab {
            page: 2,
            ..PerformTab::default()
        };
        let n = names();
        let mut ui = Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        let ev = UiEvent {
            widget: BUTTONS.to_string(),
            kind: EventKind::Tab(NAME),
        };
        assert_eq!(tab.event(&mut ui, &ev, &n), [Act::Name(2, String::new())]);
        let acts = tab.rename(2, "  Drops  ");
        assert_eq!(
            sends(&acts),
            [&FromUi::PerformName {
                page: 2,
                name: "Drops".into()
            }]
        );
        assert!(acts.contains(&Act::Tell("Perform pads: page C named Drops".into())));
        assert_eq!(tab.set.page_name(2), "Page C · Drops");
        assert!(
            tab.rename(2, "Drops").is_empty(),
            "the same name is no step"
        );
        assert_eq!(tab.history_rows(&n).0[1], "Page C named Drops");
        // UNDO from page A: back on page C, with no name.
        tab.page = 0;
        let acts = tab.undo(false, &n);
        assert_eq!(
            sends(&acts),
            [
                &FromUi::PerformPage(2),
                &FromUi::PerformName {
                    page: 2,
                    name: String::new()
                }
            ]
        );
        assert!(acts.contains(&Act::Tell(
            "Undo: Perform pads page C back to no name".into()
        )));
        assert_eq!(tab.set.page_name(2), "Page C");
        tab.undo(true, &n);
        assert_eq!(tab.set.page_name(2), "Page C · Drops");
        // A long name is cut to fit.
        tab.rename(3, &"x".repeat(40));
        assert_eq!(tab.set.pages[3].name.len(), MAX_PAGE_NAME);
    }

    #[test]
    fn the_track_and_scene_choices_show_eight_round_the_one_picked() {
        let n = names();
        let (chips, first, now) = PerformTab::choices(PadAction::Mute(1), &n);
        assert_eq!(first, 0);
        assert_eq!(now, Some(1));
        assert_eq!(
            chips.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Drums", "Bass", "Keys"]
        );
        assert_eq!(chips[1].sub, "FM · Sub");
        let (chips, first, _) = PerformTab::choices(PadAction::Scene(10), &n);
        assert_eq!(first, 8);
        assert_eq!(chips.len(), 8);
        assert_eq!(chips[0].name, "Scene 9");
        let (chips, _, now) = PerformTab::choices(PadAction::Scene(1), &n);
        assert_eq!(chips[1].sub, "Drop");
        assert_eq!(now, Some(1));
        // A mute of a track past the song's: its window still shows.
        let (chips, first, _) = PerformTab::choices(PadAction::Mute(9), &n);
        assert_eq!(first, 8);
        assert_eq!(chips[1].name, "Track 10");
    }

    #[test]
    fn a_new_project_starts_the_history_again() {
        let n = names();
        let mut tab = PerformTab {
            page: 1,
            ..PerformTab::default()
        };
        tab.edit(0, PadAction::None, &n);
        assert_eq!(tab.picked(), PadAction::None);
        // The app's echo of the edit keeps it.
        let echo = tab.set.clone();
        tab.set_set(echo, false);
        assert_eq!(tab.history_rows(&n).0.len(), 2);
        // Another project's set: a history of its own.
        let mut other = PerformSet::default();
        other.pages[2].pads[0] = PadAction::Scene(3);
        tab.set_set(other, true);
        assert_eq!(tab.history_rows(&n).0.len(), 1);
        assert_eq!(tab.set.page(2).action(0), PadAction::Scene(3));
    }

    /// The Delay throw (B1 of the default set) moved to Drums, with the
    /// Drums track's Perform FX (slot id 3) as the app sends it.
    fn delay_on_drums() -> (PerformTab, mpfree_play::mpfree_param::ParamAddr) {
        use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};
        let mut tab = PerformTab {
            page: 1,
            pad: 0,
            ..PerformTab::default()
        };
        let t = Throw {
            on: StripId::Track(0),
            ..Throw::new(0, Hold::WhileHeld)
        };
        tab.apply(1, 0, PadAction::Throw(t));
        tab.knobs = vec![ThrowKnobs {
            on: StripId::Track(0),
            fx: ThrowFx::Delay,
            uid: Some(3),
            effect: 12,
            length: 8.0,
            amount: 55.0,
        }];
        let amount = ParamAddr::new(
            Owner::Insert(Strip::Track(0), 3),
            mpfree_fx::ids::perform::slot(ThrowFx::Delay.pad()) + mpfree_fx::ids::perform::AMOUNT,
        );
        (tab, amount)
    }

    #[test]
    fn the_q_links_turn_a_throws_own_settings_on_its_strip_one_undo_a_run() {
        let n = names();
        let (mut tab, amount) = delay_on_drums();
        let cards = tab.qlink_cards(&n);
        assert_eq!(
            cards
                .each_ref()
                .map(|c| (c.name.as_str(), c.value.as_str())),
            [("Time", "1/8D"), ("Feedback", "55 %"), ("", ""), ("", "")]
        );
        assert_eq!(cards[1].scope, "Drums · every Delay throw there");
        assert_eq!(
            tab.qlink_hint(&n).as_deref(),
            Some("Q1 Time, Q2 Feedback: every Delay throw on Drums")
        );
        // A run of turns: the mixer told each time, one UNDO step.
        assert_eq!(
            tab.qlink(1, 5, false, &n),
            vec![Act::Send(FromUi::ThrowKnob {
                addr: amount,
                value: 60.0
            })]
        );
        tab.qlink(1, 2, false, &n);
        assert_eq!(tab.qlink_cards(&n)[1].value, "62 %");
        assert_eq!(tab.history_rows(&n).0.len(), 2);
        // The finger lifts: the mod engine lets go; the next turn is a
        // step of its own.
        assert_eq!(
            tab.qlink_lift(1, &n),
            vec![
                Act::Send(FromUi::ThrowKnobLift(amount)),
                Act::Tell("Perform pads: Delay throw on Drums: Feedback 62 %".into())
            ]
        );
        assert!(tab.qlink_lift(1, &n).is_empty());
        tab.qlink(1, -10, true, &n);
        assert_eq!(tab.qlink_cards(&n)[1].value, "61 %");
        assert_eq!(
            tab.history_rows(&n).0,
            vec![
                "Start: the set as it was",
                "Delay throw on Drums: Feedback 62 %",
                "Delay throw on Drums: Feedback 61 %",
            ]
        );
        // Q1 steps the Time a length at a time, and stops at the ends.
        tab.qlink(0, 1, false, &n);
        assert_eq!(tab.qlink_cards(&n)[0].value, "1/4");
        tab.qlink(0, 100, false, &n);
        assert_eq!(tab.qlink_cards(&n)[0].value, "4 bars");
        assert!(tab.qlink(0, 1, false, &n).is_empty());
        // UNDO takes the runs back, the Feedback to where it was.
        let acts = tab.undo(false, &n);
        assert!(acts.contains(&Act::Tell(
            "Undo: Delay throw's Time on Drums back to 1/8D".into()
        )));
        tab.undo(false, &n);
        let acts = tab.undo(false, &n);
        assert_eq!(
            sends(&acts),
            vec![&FromUi::ThrowKnob {
                addr: amount,
                value: 55.0
            }]
        );
        assert_eq!(tab.qlink_cards(&n)[1].value, "55 %");
        let acts = tab.undo(true, &n);
        assert!(acts.contains(&Act::Tell(
            "Redo: Delay throw's Feedback on Drums back to 62 %".into()
        )));
    }

    #[test]
    fn a_q_link_with_nothing_to_turn_says_why() {
        let n = names();
        let (mut tab, _) = delay_on_drums();
        assert_eq!(
            tab.qlink(2, 1, false, &n),
            vec![Act::Tell(
                "A throw has two settings, on Q1 and Q2: Delay throw has no others here".into()
            )]
        );
        // A wash rings for its Decay: no Length.
        tab.knobs[0].effect = 14;
        let cards = tab.qlink_cards(&n);
        assert_eq!(
            (cards[0].name.as_str(), cards[1].name.as_str()),
            ("", "Decay")
        );
        assert_eq!(
            tab.qlink_why(0, &n).as_deref(),
            Some("Wash throw has no length: Q2 turns its Decay")
        );
        // The strip's Perform FX taken off on the Mix tab: the words a hit
        // says, and nothing turns.
        tab.knobs[0].uid = None;
        assert!(tab.qlink_cards(&n).iter().all(|c| c.name.is_empty()));
        assert_eq!(
            tab.qlink(1, 1, false, &n),
            vec![Act::Tell(
                "Drums has no Perform FX for the throw: pick the pad's On again".into()
            )]
        );
        assert_eq!(tab.qlink_hint(&n), None);
        // A macro pad, and an empty one.
        tab.pad = 6;
        tab.apply(
            1,
            6,
            PadAction::Macro {
                card: 0,
                value: 100,
            },
        );
        assert!(tab
            .qlink_why(0, &n)
            .is_some_and(|w| w.starts_with("B7 holds a macro")));
        tab.pad = 15;
        assert_eq!(
            tab.qlink_why(0, &n).as_deref(),
            Some("B16 has nothing on it: the Q-Links turn a throw pad's effect")
        );
    }

    #[test]
    fn edit_the_effect_opens_the_throws_strip_and_hear_it_plays_a_bar() {
        let n = names();
        let (mut tab, _) = delay_on_drums();
        let mut ui = Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        let tap = |i| UiEvent {
            widget: ACTS.into(),
            kind: EventKind::Tab(i),
        };
        assert_eq!(
            tab.event(&mut ui, &tap(EDIT_FX), &n),
            vec![Act::Send(FromUi::ThrowEffect {
                on: StripId::Track(0),
                fx: ThrowFx::Delay
            })]
        );
        let bar = PadAction::Throw(Throw {
            hold: Hold::Bar,
            on: StripId::Track(0),
            ..Throw::new(0, Hold::WhileHeld)
        });
        assert_eq!(
            tab.event(&mut ui, &tap(HEAR), &n),
            vec![
                Act::Hear(bar, true),
                Act::Hear(bar, false),
                Act::Tell("B1: Delay throw for a bar, on Drums".into()),
            ]
        );
        // Nothing to hear or edit on a macro pad.
        tab.pad = 6;
        tab.apply(
            1,
            6,
            PadAction::Macro {
                card: 0,
                value: 100,
            },
        );
        assert!(tab.event(&mut ui, &tap(HEAR), &n).is_empty());
    }

    /// Learn a key hands the picked pad to MIDI Learn, on any kind of pad
    /// (an empty one too: the key plays what it gets later); SHIFT + a
    /// pad on the map picks that pad and hands it over without playing
    /// it, and any other touch is the view's own.
    #[test]
    fn learn_a_key_hands_the_picked_pad_to_midi_learn() {
        use mpfree_play::mpfree_param::{perform, Owner, ParamAddr};
        let n = names();
        let (mut tab, _) = delay_on_drums();
        let mut ui = Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        // The row alone (the page's frame is the GUI's).
        ui.add(ACTS, ButtonRow::new(&["", "", ""]));
        tab.show(&mut ui, &n);
        let learn = |page, pad| {
            vec![Act::Send(FromUi::LearnValue(
                crate::gui::LearnValue::Param(ParamAddr::new(
                    Owner::Perform,
                    perform::pad(page, pad),
                )),
            ))]
        };
        let tap = UiEvent {
            widget: ACTS.into(),
            kind: EventKind::Tab(LEARN),
        };
        assert_eq!(tab.event(&mut ui, &tap, &n), learn(1, 0));
        assert_eq!(
            ui.get::<ButtonRow>(ACTS).map(|b| b.enabled.clone()),
            Some(vec![true, true, true])
        );
        tab.pad = 14;
        tab.show(&mut ui, &n);
        assert_eq!(
            ui.get::<ButtonRow>(ACTS).map(|b| b.enabled.clone()),
            Some(vec![false, false, true]),
            "an empty pad: learn, but nothing to edit or hear"
        );
        assert_eq!(tab.event(&mut ui, &tap, &n), learn(1, 14));
        // SHIFT + a pad: picked and handed over, never played.
        let touch = |kind| UiEvent {
            widget: MAP.into(),
            kind,
        };
        assert_eq!(
            tab.learn_touch(&touch(EventKind::Select(5))),
            Some(learn(1, 5))
        );
        assert_eq!(tab.pad, 5);
        assert_eq!(
            tab.learn_touch(&touch(EventKind::Pad {
                index: 5,
                down: true
            })),
            Some(learn(1, 5))
        );
        assert_eq!(
            tab.learn_touch(&touch(EventKind::Pad {
                index: 5,
                down: false
            })),
            None
        );
        assert_eq!(tab.learn_touch(&touch(EventKind::Tab(2))), None, "a page");
    }
}

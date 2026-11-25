//! The Looper tab as an instrument (the looper redesign, approved by Jay
//! on 17 Nov): the deck of loops across the top ([`LoopDeck`]), the
//! picked loop under it at full width ([`LoopEditor`]), and the Q-Links on
//! the right. Q1 to Q4 are loops 1 to 4's levels; Q-LINK steps to loops 5
//! to 8, then to the picked loop (Feedback, the picked layer's level and
//! pan, the seam fade). And the Play page's loop row ([`LoopRow`], the
//! redesign's step 5), every loop as the deck has them, with the picked
//! loop's layers.
//!
//! The panels are [`crate::looper_deck`]'s, [`crate::looper_tab`]'s and
//! [`crate::loop_row`]'s; this fills them from the looper's view. REC,
//! OVERDUB and UNDO on the MPC work the picked loop while this tab shows
//! (`App::looper_tap`).

use crate::app::Settings;
use crate::loop_row::{LayerChip, LoopRow, LOOP_ROW};
use crate::looper_deck::{CaptureButton, Channel, DeckHead, LoopDeck, RingState, DECK};
use crate::looper_tab::{
    level_frac, level_text, AskButtons, Chip, ChipAct, ChipKind, Ctl, DropSpot, DropTo, EmptyLook,
    HistoryList, LayerDrag, LayerRow, LoopEditor, Pick, PointLook, PointPane, SmartChip, Strip,
    ASK, BAR_CHOICES, EDITOR, LOOP_COLOURS, QLINKS, SEND_SHOW, SPEED_CHOICES,
};
use crate::looping::{
    bars_words_n, hand_moved, Edge, LoopSource, LoopWave, LooperView, Side, POINT_MS,
};
use mpfree_looper::{
    After, Clock, Command, Edit, History, LayerCommand, LayerInfo, Overdubs, Phase, Shift, Snap,
    Starts, Status, Step, LOOP_MAX_BPM, LOOP_MIN_BPM, MAX_LAYERS, MAX_LOOPS, MAX_SEAM,
};
use mpfree_time_ui::view::{LoopPlace, TransportInfo};
use mpfree_transport::State;
use mpfree_ui::qlink::{QLinkCard, QLinkColumn};
use mpfree_ui::{Color, Ui};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const LOOPER_PAGE: &str = "Looper";
/// The deck across the top, the picked loop under it, the Q-Links on the
/// right (the redesign's Main board).
pub const LOOPER_LAYOUT: &str =
    "col(header:60, row(col(ldeck:240, leditor:*):*, lqlinks:128):*, hint:26)";

/// The looper's presses, as `App::looper_tap` takes them.
pub const B_RECORD: usize = 0;
pub const B_OVERDUB: usize = 1;
pub const B_UNDO: usize = 2;
pub const B_REDO: usize = 3;
pub const B_CLEAR: usize = 4;
pub const B_MUTE: usize = 5;
pub const B_MONITOR: usize = 6;

/// The layers' colours, layer 1 first: the loops' (none red, amber or
/// orange, which mean recording and overdubbing), less the grey.
pub const LAYER_COLOURS: [Color; 7] = [
    LOOP_COLOURS[0],
    LOOP_COLOURS[1],
    LOOP_COLOURS[2],
    LOOP_COLOURS[3],
    LOOP_COLOURS[4],
    LOOP_COLOURS[5],
    LOOP_COLOURS[6],
];

/// The trim grips' range, as a command while the loop can be edited.
pub fn trim_command(from: i64, to: i64, s: &Status) -> Option<Command> {
    (editable(s) && 0 <= from && from < to && to <= s.ticks)
        .then_some(Command::Edit(Edit::Trim { from, to }))
}

/// An edit as the undo history names it.
fn edit_text(e: Edit, rate: u32) -> String {
    let sixteenths = |n: i32| {
        if n.abs() == 1 {
            "1/16".to_string()
        } else {
            format!("{} × 1/16", n.abs())
        }
    };
    // A boundary moved, later or earlier, by so many ms.
    let moved = |what: &str, frames: i64| {
        let ms = seam_text(frames.unsigned_abs() as usize, rate);
        if frames < 0 {
            format!("{what} ← {ms}")
        } else {
            format!("{what} → {ms}")
        }
    };
    match e {
        Edit::Half => "½ length".to_string(),
        Edit::Double => "× 2".to_string(),
        Edit::Reverse => "Reverse".to_string(),
        Edit::Nudge(n) if n < 0 => format!("Nudge ← {}", sixteenths(n)),
        Edit::Nudge(n) => format!("Nudge {} →", sixteenths(n)),
        Edit::Trim { .. } => "Trim".to_string(),
        Edit::SeamFade(f) => format!("Seam fade {}", seam_text(f as usize, rate)),
        Edit::Speed(v) => format!("Speed {}", speed_text(v)),
        Edit::Move(d) => moved("Move", d),
        Edit::Resize(d) => moved("End", d),
        Edit::Turn(d) => moved("Bar 1", d),
    }
}

/// A loop's speed as the strip names it: "½×", "1×", "2×".
pub fn speed_text(speed: i8) -> &'static str {
    SPEED_CHOICES[(speed.clamp(-1, 1) + 1) as usize]
}

/// A step as the undo history names it: layers by their place in the
/// loop now ("Layer 3"), when they are in it.
pub fn step_text(s: Step, layers: &[Option<LayerInfo>], rate: u32) -> String {
    let name = |id: u8| {
        layers
            .iter()
            .position(|l| l.is_some_and(|l| l.id == id))
            .map(|k| format!("Layer {}", k + 1))
    };
    match s {
        Step::Overdub(id) => name(id).map_or("Overdub".to_string(), |n| format!("Overdub · {n}")),
        Step::Delete(id) => {
            name(id).map_or("Delete a layer".to_string(), |n| format!("Delete · {n}"))
        }
        Step::Edit(e) => edit_text(e, rate),
        Step::Merge(_) => "Merge layers".to_string(),
        // Overdub or Record over a print fits the loop to the tempo and
        // its speed.
        Step::Fit(_) => "Fit to tempo and speed".to_string(),
        // Dragged here from another loop, or from here to another.
        Step::Added(id, from) => name(id).map_or_else(
            || format!("From loop {}", from + 1),
            |n| format!("From loop {} · {n}", from + 1),
        ),
        Step::Moved(_, to) => format!("Moved to loop {}", to + 1),
        // Smart loop end's Snap, Use my tap or Use my press, with the
        // tempo.
        Step::Snap(d) => {
            let mut parts = Vec::new();
            if d.start != 0 {
                parts.push(edit_text(Edit::Move(d.start), rate).replacen("Move", "Start", 1));
            }
            if d.end != 0 || d.start == 0 {
                parts.push(edit_text(Edit::Resize(d.end), rate));
            }
            format!("Snap · {}", parts.join(" · "))
        }
        // Read as: the loop's length kept, its bars and the tempo changed.
        Step::Bars(b) => format!("Read as {}", crate::looping::bars_words_n(b)),
    }
}

/// The undo history as the panel lists it: Start (the first pass), the
/// steps Undo takes back, oldest first, then the ones Redo brings back.
pub fn history_list(h: &History, layers: &[Option<LayerInfo>], rate: u32) -> HistoryList {
    let mut rows = vec!["Start: the first pass".to_string()];
    rows.extend(h.done().iter().map(|&s| step_text(s, layers, rate)));
    let now = rows.len() - 1;
    rows.extend(h.redo().iter().map(|&s| step_text(s, layers, rate)));
    HistoryList { rows, now }
}

/// A seam fade as text: "10 ms", "cut".
pub fn seam_text(frames: usize, rate: u32) -> String {
    if frames == 0 {
        "cut".to_string()
    } else {
        format!("{:.0} ms", frames as f64 * 1000.0 / f64::from(rate.max(1)))
    }
}

/// Whether the picked loop takes Sync now: it is empty, with nothing
/// coming in (the looper refuses a clock for a loop with sound).
fn takes_sync(s: &Status) -> bool {
    s.phase == Phase::Empty && s.loading.is_none() && s.copying.is_none() && s.capturing.is_none()
}

/// Whether the picked loop takes edits now: it plays (not recording or
/// overdubbing) and no merge is rendering.
fn editable(s: &Status) -> bool {
    s.phase == Phase::Playing && s.merging.is_none()
}

/// What the panel's control `c` asks of the picked loop, for the ones
/// that are a looper command; None while it can't work.
pub fn strip_command(c: Ctl, s: &Status) -> Option<Command> {
    match c {
        Ctl::Reverse if editable(s) => Some(Command::Edit(Edit::Reverse)),
        Ctl::Half if editable(s) => Some(Command::Edit(Edit::Half)),
        Ctl::Double if editable(s) => Some(Command::Edit(Edit::Double)),
        Ctl::Speed(i) if editable(s) && i as i8 - 1 != s.speed => {
            Some(Command::Edit(Edit::Speed(i as i8 - 1)))
        }
        Ctl::Merge if editable(s) && s.layers > 1 => Some(Command::Merge),
        Ctl::Starts(i) => Some(Command::SetStarts(if i == 0 {
            Starts::Bar
        } else {
            Starts::Play
        })),
        // Sync: taken by the loop while it is empty, and by every empty
        // loop after it (LoopHost::clock). Once it records it keeps its
        // own, and Sync shows it, fixed (Jay's review, 23 Nov, 5.2).
        Ctl::Sync(i) if takes_sync(s) => Some(Command::SetClock(if i == 0 {
            Clock::Song
        } else {
            Clock::Free
        })),
        Ctl::Then(i) => Some(Command::SetAfter(if i == 0 {
            After::Play
        } else {
            After::Overdub
        })),
        // The next overdub's way, even while one records.
        Ctl::Overdubs(i) => Some(Command::SetOverdubs(match i {
            0 => Overdubs::Add,
            1 => Overdubs::Replace,
            _ => Overdubs::Once,
        })),
        _ => None,
    }
}

/// Why the panel's control `c` can't work now, for the screen to say in
/// place when its faint button is tapped (Jay, 12 Nov).
pub fn why_faint(c: Ctl, s: &Status) -> &'static str {
    let phase = match s.phase {
        Phase::Empty => "Record a loop first: this changes the loop that plays",
        Phase::Armed | Phase::Recording => "This works once the first pass is recorded and plays",
        Phase::Overdubbing => "This works while the loop plays, not while it overdubs",
        Phase::Playing if s.merging.is_some() => "Wait for the merge to finish",
        Phase::Playing => "That can't change this loop now",
    };
    match c {
        Ctl::Send => "Record a loop first: Send sends the loop that plays",
        Ctl::Clear => "Nothing to clear: the loop is empty",
        Ctl::Listen if s.phase == Phase::Playing && s.merging.is_none() => {
            "Listen plays a loop on its own while the music is stopped: press STOP first"
        }
        Ctl::Listen if s.phase == Phase::Empty => "Nothing to listen to yet: record the loop first",
        Ctl::Undo => "Nothing to undo on this loop yet",
        Ctl::Redo => "Nothing to redo: Undo first",
        Ctl::History => "No history yet: overdub a layer first",
        Ctl::Merge if s.phase == Phase::Playing && s.merging.is_none() => {
            "Merge needs two layers or more: overdub one first"
        }
        Ctl::Starts(_) => {
            "Recording the other loops starts on the bar: When I play is for the pads and the line input"
        }
        Ctl::Sync(_) if s.clock == Clock::Free => {
            "This loop is free and stays so: pick an empty loop to set Sync"
        }
        Ctl::Sync(_) => "This loop keeps time with the song and stays so: pick an empty loop to set Sync",
        _ => phase,
    }
}

/// Why Send's choice `i` can't be used now.
pub fn why_no_send(i: usize) -> &'static str {
    if i == SEND_SHOW {
        return "This loop is in no cell yet: once it has a first pass it shows on a Loop track";
    }
    "Not built yet: Show in Clips works now (the bounce to a pad, a track or a file comes next)"
}

/// Feedback as the panel and the Q-Links move it: 50 % to 100 % kept a
/// pass, in steps of 1 % (SHIFT: 0.2 %).
const FADE_OLD_MIN: f32 = 0.5;

/// How far round Feedback's ring is lit, 0 to 1.
fn fade_old_pos(k: f32) -> f32 {
    ((k - FADE_OLD_MIN) / (1.0 - FADE_OLD_MIN)).clamp(0.0, 1.0)
}

/// A Q-Link turn on the Looper tab: what it asks of the looper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopTurn {
    /// Loop `n`'s level (the first two banks).
    LoopLevel(usize, f32),
    /// The picked layer's level or pan, by play order.
    Layer(usize, LayerCommand),
    /// The picked loop's Feedback, 0.5 to 1 kept a pass.
    FadeOld(f32),
    /// The picked loop's seam fade, in frames.
    Seam(u32),
    /// Move start / Move end while the loop point view shows: Q1 the
    /// start, Q2 the end, a step a detent (later above 0).
    Move(Edge, i32),
}

/// The Q-Link banks on the Looper tab, in Q-LINK's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopBank {
    /// Loops 1 to 4's levels.
    #[default]
    First,
    /// Loops 5 to 8's levels.
    Second,
    /// The picked loop: Feedback, the picked layer's level and pan, the
    /// seam fade.
    Picked,
}

impl LoopBank {
    pub fn next(self) -> LoopBank {
        match self {
            LoopBank::First => LoopBank::Second,
            LoopBank::Second => LoopBank::Picked,
            LoopBank::Picked => LoopBank::First,
        }
    }

    /// The bank before (SHIFT + Q-LINK).
    pub fn prev(self) -> LoopBank {
        match self {
            LoopBank::First => LoopBank::Picked,
            LoopBank::Second => LoopBank::First,
            LoopBank::Picked => LoopBank::Second,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LoopBank::First => "Loops 1-4",
            LoopBank::Second => "Loops 5-8",
            LoopBank::Picked => "the picked loop",
        }
    }
}

/// A take being recorded: its loop, its layer, and how many hits of each
/// group of sounds went into it.
type Take = (usize, usize, Vec<(&'static str, u32)>);

/// What the looper records from now: the source a take holds, else
/// Records from as picked.
fn now_source(v: &LooperView) -> LoopSource {
    v.taking_from.unwrap_or(v.source)
}

/// A level in dBFS along the input meter, 0 to 1 (-48 dB to 0).
fn input_frac(db: f32) -> f32 {
    if !db.is_finite() {
        return 0.0;
    }
    ((db + 48.0) / 48.0).clamp(0.0, 1.0)
}

/// A free slot's foot with loops in audio cells (phase 4, board 5).
pub const FREE_FOOT: &str = "REC into an empty audio cell fills it";

pub struct LooperPage {
    /// The input's peak, held for about 2 s of updates.
    hold: f32,
    hold_age: u32,
    /// Each loop's layers' names, by what was played into each ("Drums",
    /// "Bass"); kept past an undo, so a redo finds its name again.
    names: Vec<Vec<String>>,
    /// The take recording now: its loop and layer, and how many hits of
    /// each group of sounds went into it.
    take: Option<Take>,
    /// The picked loop's layers' ids and mix as of the last view.
    layers: [Option<LayerInfo>; MAX_LAYERS],
    /// The loop the controls act on.
    selected: usize,
    /// The layer on the Q-Links, play order.
    layer: usize,
    /// The last view, for the Q-Links.
    view: LooperView,
    /// A Q-Link was turned: the cards show what it set, not the view, for
    /// a moment (the looper takes a period or two to answer).
    turned: Option<Instant>,
    /// A loop's level set from the screen a moment ago (its bar, its
    /// Q-Link): the deck shows it until the looper has caught up.
    levelled: [Option<(f32, Instant)>; MAX_LOOPS],
    /// The undo history is open in the lanes' place.
    history_open: bool,
    /// Each loop's place on the Clips grid (Loop tracks), for its name.
    places: [Option<LoopPlace>; MAX_LOOPS],
    /// The song's clock, for the deck's header and a waiting loop's bar.
    transport: TransportInfo,
    /// What the Q-Links turn.
    bank: LoopBank,
    /// The pads are the loop keys: the deck's Pads switch is lit.
    pads_keys: bool,
    /// The mixes Records from offers, named as the Mix tab names them:
    /// the master, the tracks and the submixes, never the Loop track
    /// ([`LooperPage::set_mixes`]).
    mixes: Vec<(String, mpfree_fx::StripId)>,
    /// Dev's "Loops on audio tracks" (phase 4, board 5): where the next
    /// loop goes, for the header ("Vocal · row 3 · Line L"); None with the
    /// switch off ([`LooperPage::set_on_tracks`]).
    on_tracks: Option<String>,
}

impl LooperPage {
    pub fn add(ui: &mut Ui) -> Result<LooperPage, String> {
        ui.add(DECK, LoopDeck::default());
        ui.add(EDITOR, LoopEditor::default());
        ui.add(QLINKS, QLinkColumn::new());
        ui.add(LOOP_ROW, LoopRow::default());
        // Snap anyway and Not now: floated over the page while a snap
        // asks (gui.rs, refresh_looper).
        ui.add(ASK, AskButtons::default());
        ui.add_page(LOOPER_PAGE, LOOPER_LAYOUT)?;
        let mut p = LooperPage {
            hold: f32::NEG_INFINITY,
            hold_age: 0,
            names: vec![Vec::new(); MAX_LOOPS],
            take: None,
            layers: [None; MAX_LAYERS],
            selected: 0,
            layer: 0,
            view: LooperView::default(),
            turned: None,
            levelled: [None; MAX_LOOPS],
            history_open: false,
            places: Default::default(),
            transport: TransportInfo::default(),
            bank: LoopBank::default(),
            pads_keys: false,
            mixes: vec![("Master".to_string(), mpfree_fx::StripId::Master)],
            on_tracks: None,
        };
        p.update(ui, &LooperView::default(), &Settings::default());
        Ok(p)
    }

    /// Each loop's place on the Clips grid, from the time engine's state;
    /// true when one changed (the page is then refreshed).
    pub fn set_places(&mut self, places: [Option<LoopPlace>; MAX_LOOPS]) -> bool {
        if self.places == places {
            return false;
        }
        self.places = places;
        true
    }

    /// Loops on audio tracks (phase 4, board 5): with Dev's switch on,
    /// where the next loop goes ("Vocal · row 3 · Line L"); None with it
    /// off. True when it changed (the page is then refreshed).
    pub fn set_on_tracks(&mut self, into: Option<String>) -> bool {
        if self.on_tracks == into {
            return false;
        }
        self.on_tracks = into;
        true
    }

    /// Loop `n`'s place, when it is a loop cell on an audio track with
    /// Dev's switch on (phase 4): its card and header name it there.
    fn audio_place(&self, n: usize) -> Option<&LoopPlace> {
        self.on_tracks.as_ref()?;
        self.place(n).filter(|p| p.audio)
    }

    /// Loops on audio tracks (phase 4, board 5), with Dev's switch on:
    /// loop `n`'s card named by its place ("Vocal · row 2", the loop's
    /// name under it), its M and S its track's; a free slot says how one
    /// is filled.
    fn on_track_card(&self, n: usize, ch: &mut Channel) {
        if self.on_tracks.is_none() {
            return;
        }
        if let Some(p) = self.audio_place(n) {
            ch.name = format!("{} \u{b7} {}", p.track_name, p.row);
            ch.sub = if p.clip.is_empty() {
                format!("Loop {}", n + 1)
            } else {
                p.clip.clone()
            };
            // A free loop in a free cell (4e.4) says so, as its length did.
            if ch.free {
                ch.sub.push_str(" \u{b7} free");
            }
            if p.mute {
                ch.sub.push_str(" \u{b7} muted");
            }
            ch.length.clear();
            ch.track = format!("{}'S TRACK", p.track_name.to_uppercase());
            ch.track_mute = p.mute;
            ch.track_solo = p.solo;
        } else if ch.state == RingState::Empty && self.place(n).is_none() {
            ch.label = std::mem::replace(&mut ch.name, "Free".to_string());
            // A cleared loop keeps "Cleared · UNDO" under it.
            if !ch.foot.starts_with("Cleared") {
                ch.foot = FREE_FOOT.to_string();
            }
        }
    }

    /// Loop `n` as the screens name it: by its place on the Clips grid,
    /// "Loops B · Loop 2 · Verse", or "Loop 2" while it is in no cell.
    pub fn loop_name(&self, n: usize) -> String {
        match self.places.get(n).and_then(Option::as_ref) {
            Some(p) => p.full(n),
            None => format!("Loop {}", n + 1),
        }
    }

    /// Where loop `n` sits on the Clips grid, if it is in a cell.
    pub fn place(&self, n: usize) -> Option<&LoopPlace> {
        self.places.get(n).and_then(Option::as_ref)
    }

    /// The song's clock moved: the deck's header, and a waiting loop's
    /// bar.
    pub fn set_transport(&mut self, ui: &mut Ui, t: TransportInfo) {
        if self.transport == t {
            return;
        }
        self.transport = t;
        let v = self.view.clone();
        self.refresh_deck(ui, &v);
    }

    /// A pad played a sound of `group` ("Drums", "Bass", "Synth"): it
    /// names the layer recording now, if one is.
    pub fn hit(&mut self, group: &'static str) {
        if let Some((_, _, counts)) = &mut self.take {
            match counts.iter_mut().find(|(g, _)| *g == group) {
                Some((_, n)) => *n += 1,
                None => counts.push((group, 1)),
            }
        }
    }

    /// The picked loop's layers' names as they are (the previews name
    /// theirs).
    pub fn set_names(&mut self, names: &[&str]) {
        self.names[self.selected] = names.iter().map(|n| n.to_string()).collect();
    }

    /// The name of the picked loop's layer `k`, as the lanes show it.
    pub fn layer_name(&self, k: usize) -> String {
        self.names[self.selected]
            .get(k)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Layer {}", k + 1))
    }

    /// Layer `k`'s id and mix as of the last view (for its controls).
    pub fn layer_info(&self, k: usize) -> Option<LayerInfo> {
        self.layers.get(k).copied().flatten()
    }

    /// Loop `n`'s level was set from the screen (its level bar): the deck
    /// keeps it while the looper catches up.
    pub fn level_set(&mut self, n: usize, level: f32) {
        if let Some(l) = self.levelled.get_mut(n) {
            *l = Some((level, Instant::now()));
        }
    }

    /// Whether the pads are the loop keys (the app's status): the deck's
    /// Pads switch follows.
    pub fn set_pads_keys(&mut self, ui: &mut Ui, on: bool) {
        if self.pads_keys == on {
            return;
        }
        self.pads_keys = on;
        let v = self.view.clone();
        self.refresh_deck(ui, &v);
    }

    /// The Q-Links' bank now.
    pub fn bank(&self) -> LoopBank {
        self.bank
    }

    /// Q-LINK: the next bank (SHIFT + Q-LINK, `back`: the one before).
    pub fn next_bank(&mut self, ui: &mut Ui, back: bool) -> LoopBank {
        self.bank = if back {
            self.bank.prev()
        } else {
            self.bank.next()
        };
        let v = self.view.clone();
        self.refresh_qlinks(ui, &v);
        self.bank
    }

    /// Where a layer dragged out of the lanes can be let go: the other
    /// loops on the deck. An empty one takes it as a loop of its own; one
    /// that plays takes it when it is as long (another length, or a loop
    /// recording, says why not).
    fn drop_spots(&self, ui: &Ui, v: &LooperView) -> Vec<DropSpot> {
        let (Some(dr), Some(deck)) = (ui.rect_on(LOOPER_PAGE, DECK), ui.get::<LoopDeck>(DECK))
        else {
            return Vec::new();
        };
        let src = v.selected;
        let s = &v.status;
        let len = |l: &Status| {
            let b = bars_of(l.ticks, v.bar_ticks);
            format!("{b} bar{}", if b == 1 { "" } else { "s" })
        };
        let busy = |l: &Status| l.merging.is_some() || l.copying.is_some() || l.fitting.is_some();
        // The picked loop's layers drag while it plays with nothing busy.
        let held = match s.phase {
            Phase::Playing if !busy(s) => None,
            Phase::Playing => Some(format!(
                "Loop {} is busy: drag its layers once it is done",
                src + 1
            )),
            _ => Some(format!(
                "Loop {} is recording: drag its layers once it plays",
                src + 1
            )),
        };
        let mut out = Vec::new();
        for (n, card) in deck.cards(dr).into_iter().enumerate() {
            let Some(l) = v.loops.get(n).filter(|_| n != src) else {
                continue;
            };
            let refused = held.clone().or_else(|| match l.phase {
                Phase::Empty if l.loading.is_some() => {
                    Some(format!("Loop {} is busy: try again in a moment", n + 1))
                }
                Phase::Empty => None,
                Phase::Playing if l.ticks != s.ticks => Some(format!(
                    "Loop {} is {} and this layer {}: a layer goes only onto a loop of its length",
                    n + 1,
                    len(l),
                    len(s)
                )),
                Phase::Playing if l.layers >= MAX_LAYERS - 1 => {
                    Some(format!("Loop {} has all the layers it can take", n + 1))
                }
                Phase::Playing if busy(l) => {
                    Some(format!("Loop {} is busy: try again in a moment", n + 1))
                }
                Phase::Playing => None,
                _ => Some(format!("Loop {} is recording", n + 1)),
            });
            out.push(DropSpot {
                to: DropTo::Loop(n),
                rect: card,
                refused,
            });
        }
        out
    }

    /// Why a dragged layer can't go to `to`, as the screen says it.
    pub fn why_no_drop(&self, ui: &Ui, to: DropTo) -> String {
        ui.get::<LoopEditor>(EDITOR)
            .and_then(|e| e.drops.iter().find(|d| d.to == to))
            .and_then(|d| d.refused.clone())
            .unwrap_or_else(|| "That loop can't take the layer".to_string())
    }

    /// What letting a dragged layer go on `to` does, as its card says it
    /// (`copy`: SHIFT is held).
    fn drop_words(&self, to: Option<&DropSpot>, copy: bool) -> String {
        let only = self.view.status.layers <= 1;
        match to {
            None => "Let go here: it stays where it is".to_string(),
            Some(DropSpot {
                refused: Some(why), ..
            }) => why.clone(),
            Some(d) => {
                let way = if copy || only { "Copy" } else { "Move" };
                let DropTo::Loop(n) = d.to;
                let empty = self
                    .view
                    .loops
                    .get(n)
                    .is_some_and(|l| l.phase == Phase::Empty);
                if empty {
                    format!("{way} into Loop {}: a loop of its own", n + 1)
                } else {
                    format!("{way} to Loop {}", n + 1)
                }
            }
        }
    }

    /// Keep the deck and the card in step with the lanes' drag: called
    /// after every touch on the Looper tab and every turn of the wheel
    /// (`shift`: SHIFT is held, so letting go copies).
    pub fn sync_drag(&mut self, ui: &mut Ui, shift: bool) {
        // A layer the wheel picked up goes back once nothing holds it: the
        // page changed, or an overlay took the wheel.
        if !ui.focus_grabbed() && ui.get::<LoopEditor>(EDITOR).is_some_and(|e| e.wheel_held()) {
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.wheel_cancel();
            }
        }
        let Some(e) = ui.get::<LoopEditor>(EDITOR) else {
            return;
        };
        let drag = e.drag().and_then(|(k, card)| {
            let landing = e.landing();
            Some(LayerDrag {
                row: e.layers.get(k).cloned()?,
                card,
                at: e.finger()?,
                says: self.drop_words(landing, shift),
                landing: landing.map(|d| (d.to, d.refused.is_none())),
                lit: e
                    .drops
                    .iter()
                    .filter(|d| d.refused.is_none())
                    .map(|d| d.to)
                    .collect(),
            })
        });
        let says = drag.as_ref().map(|d| d.says.clone()).unwrap_or_default();
        if e.says != says {
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.says = says;
            }
        }
        if ui.get::<LoopDeck>(DECK).is_some_and(|d| d.drag != drag) {
            if let Some(d) = ui.get_mut::<LoopDeck>(DECK) {
                d.drag = drag;
            }
        }
    }

    /// The history chips: open the undo history in the lanes' place, or
    /// close it.
    pub fn toggle_history(&mut self, ui: &mut Ui) {
        self.history_open = !self.history_open;
        let v = self.view.clone();
        self.refresh_editor(ui, &v);
    }

    pub fn history_open(&self) -> bool {
        self.history_open
    }

    /// The strips the mixer has (`mixing::throw_strips`): Records from's
    /// mixes are these but the Loop track, which would record itself. The
    /// picker's chips follow.
    pub fn set_mixes(&mut self, ui: &mut Ui, strips: &[(String, mpfree_fx::StripId)]) {
        let loop_track = mpfree_fx::StripId::Track(crate::mixing::LOOP);
        let mixes: Vec<_> = strips
            .iter()
            .filter(|s| s.1 != loop_track)
            .cloned()
            .collect();
        if mixes == self.mixes {
            return;
        }
        self.mixes = mixes;
        if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
            e.set_mixes(self.mixes.iter().map(|m| m.0.clone()).collect());
        }
        let v = self.view.clone();
        self.refresh_deck(ui, &v);
    }

    /// Records from's mix `i` (into the picker's mix chips): its name and
    /// strip.
    pub fn mix(&self, i: usize) -> Option<(String, mpfree_fx::StripId)> {
        self.mixes.get(i).cloned()
    }

    /// Records from, as the panel's choices count it: 0 Pads, 1 Line in, 2
    /// Loops, then the mixes from 3 ([`crate::looper_tab::SOURCE_CHOICES`]
    /// and the picker's mix chips); a mix no longer in the mixer lights
    /// none.
    pub fn source_index(&self, v: &LooperView) -> usize {
        let fixed = crate::looper_tab::SOURCE_CHOICES.len();
        match (v.resample, v.source) {
            (true, _) => 2,
            (false, LoopSource::Pads) => 0,
            (false, LoopSource::Line) => 1,
            (false, LoopSource::Mix(s)) => self
                .mixes
                .iter()
                .position(|m| m.1 == s)
                .map_or(usize::MAX, |i| fixed + i),
            // An audio track's input (phase 4b, Dev's switch): no chip.
            (false, LoopSource::Track(_)) => usize::MAX,
        }
    }

    /// What the looper records from now, as the input chip, an empty
    /// ring's foot and What you played say it: "Pads", "Line
    /// in", "Loops", or the mix's name. While a take holds another source
    /// than the one picked, the take's: a loop started then records it too.
    fn source_word(&self, v: &LooperView) -> String {
        match v.resample {
            true => "Loops".to_string(),
            false => self.word_of(now_source(v)),
        }
    }

    /// A source in a word or two, as the screen names it.
    fn word_of(&self, s: LoopSource) -> String {
        match s {
            LoopSource::Pads => "Pads".to_string(),
            LoopSource::Line => "Line in".to_string(),
            LoopSource::Mix(s) => self.mix_name(s),
            LoopSource::Track(_) => "Track in".to_string(),
        }
    }

    /// The input chip: what the take recording now records from while it
    /// holds another source than the one picked, with the pick as the
    /// next take's (`LooperView::taking_from`); else Records from as
    /// picked.
    fn input_words(&self, v: &LooperView) -> (String, Option<String>) {
        let next = v
            .taking_from
            .filter(|_| !v.resample)
            .map(|_| self.word_of(v.source));
        (self.source_word(v), next)
    }

    /// A mix's name as the Mix tab says it.
    fn mix_name(&self, s: mpfree_fx::StripId) -> String {
        self.mixes
            .iter()
            .find(|m| m.1 == s)
            .map_or_else(|| crate::mixing::name(s), |m| m.0.clone())
    }

    /// A tap on row `row` of the undo history: how many steps back (below
    /// 0) or forward that is. None for now itself.
    pub fn history_steps(&self, row: usize) -> Option<i32> {
        let now = self.view.history.done().len();
        let rows = now + self.view.history.redo().len();
        (row != now && row <= rows).then(|| row as i32 - now as i32)
    }

    /// The picked loop's undo history for the list that holding UNDO
    /// opens: the loop's name, and its rows as the panel lists them.
    pub fn history_rows(&self) -> (String, HistoryList) {
        let v = &self.view;
        (
            self.loop_name(v.selected),
            history_list(&v.history, &self.layers, v.rate),
        )
    }

    /// A tap on a layer's lane: that layer goes on the Q-Links.
    pub fn pick_layer(&mut self, ui: &mut Ui, k: usize) {
        self.layer = k;
        let v = self.view.clone();
        self.refresh_editor(ui, &v);
        self.refresh_qlinks(ui, &v);
    }

    /// Follows the looper's takes: a new one starts counting, one that
    /// ends is named by what was played most into it ("Drums", "Bass 2"),
    /// or "Line in" when the line was the source.
    fn follow_takes(&mut self, v: &LooperView) {
        let s = &v.status;
        let n = v.selected.min(MAX_LOOPS - 1);
        let now = match s.phase {
            Phase::Recording => Some(0),
            Phase::Overdubbing => Some(s.layers.saturating_sub(1)),
            _ => None,
        };
        for (i, l) in v.loops.iter().enumerate() {
            let phase = if i == n { s.phase } else { l.phase };
            if phase == Phase::Empty {
                self.names[i].clear();
            }
        }
        if self.take.as_ref().map(|t| (t.0, t.1)) != now.map(|k| (n, k)) {
            self.take = now.map(|k| (n, k, Vec::new()));
        }
        let Some((n, k, counts)) = &self.take else {
            return;
        };
        let mix_name = match now_source(v) {
            LoopSource::Mix(m) => self.mix_name(m),
            _ => String::new(),
        };
        let base = if v.resample && s.resampling {
            "Loops"
        } else {
            match now_source(v) {
                LoopSource::Line => "Line in",
                LoopSource::Track(_) => "Track in",
                LoopSource::Mix(_) => &mix_name,
                LoopSource::Pads => match counts.iter().max_by_key(|(_, c)| *c) {
                    Some((g, _)) => g,
                    None => "",
                },
            }
        };
        let (n, k) = (*n, *k);
        let names = &self.names[n];
        let name = if base.is_empty() {
            String::new()
        } else {
            let same = (0..k)
                .filter(|&i| {
                    names
                        .get(i)
                        .is_some_and(|m| m == base || m.starts_with(&format!("{base} ")))
                })
                .count();
            if same == 0 {
                base.to_string()
            } else {
                format!("{base} {}", same + 1)
            }
        };
        let names = &mut self.names[n];
        if names.len() <= k {
            names.resize(k + 1, String::new());
        }
        names[k] = name;
    }

    /// The new loop waveform (empty when the loop was cleared), in the
    /// lanes. While the first pass records, the wave grows with it and
    /// the playhead follows its end.
    pub fn set_wave(&mut self, ui: &mut Ui, wave: Arc<LoopWave>) {
        let wave = Some(wave).filter(|w| w.frames > 0);
        let live = live_place(&self.view, wave.as_ref());
        if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
            e.wave = wave;
            if let Some((at, bars)) = live {
                (e.position, e.bars) = (Some(at), bars);
            }
        }
    }

    /// A Q-Link turned `ticks` detents on the Looper tab (SHIFT: `fine`):
    /// a loop's level in the first two banks; the picked loop's Feedback,
    /// the picked layer's level and pan, and the seam fade in the third.
    /// The card follows at once.
    pub fn qlink(&mut self, ui: &mut Ui, k: usize, ticks: i32, fine: bool) -> Option<LoopTurn> {
        if ticks == 0 || k > 3 {
            return None;
        }
        // The loop point view shows: Q1 moves the start, Q2 the end, by
        // its step; Q3 and Q4 move nothing.
        if Self::point_showing(ui) {
            return match k {
                0 => Some(LoopTurn::Move(Edge::Start, ticks)),
                1 => Some(LoopTurn::Move(Edge::End, ticks)),
                _ => None,
            };
        }
        let d = ticks as f32 * if fine { 0.1 } else { 0.5 };
        let step_db = |level: f32| {
            let db = (20.0 * level.max(1e-6).log10()).max(-60.0);
            let db = (db + d).clamp(-60.0, 6.0);
            if db <= -60.0 {
                0.0
            } else {
                10f32.powf(db / 20.0)
            }
        };
        let turn = match self.bank {
            LoopBank::First | LoopBank::Second => {
                let n = k + if self.bank == LoopBank::Second { 4 } else { 0 };
                let l = self.view.loops.get(n)?;
                if l.phase == Phase::Empty {
                    return None;
                }
                let now = self.levelled[n].map_or(l.level, |(v, _)| v);
                let level = step_db(now);
                self.levelled[n] = Some((level, Instant::now()));
                LoopTurn::LoopLevel(n, level)
            }
            LoopBank::Picked => match k {
                0 => {
                    let s = &mut self.view.status;
                    if s.phase == Phase::Empty {
                        return None;
                    }
                    let step = if fine { 0.002 } else { 0.01 };
                    s.fade_old = (s.fade_old + ticks as f32 * step).clamp(FADE_OLD_MIN, 1.0);
                    self.turned = Some(Instant::now());
                    LoopTurn::FadeOld(s.fade_old)
                }
                1 | 2 => {
                    let info = self.layers.get_mut(self.layer)?.as_mut()?;
                    let cmd = if k == 1 {
                        info.level = step_db(info.level);
                        LayerCommand::Level(info.level)
                    } else {
                        let step = if fine { 0.01 } else { 0.02 };
                        info.pan = (info.pan + ticks as f32 * step).clamp(-1.0, 1.0);
                        LayerCommand::Pan(info.pan)
                    };
                    let info = *info;
                    if let Some(slot) = self.view.layers.get_mut(self.layer) {
                        *slot = Some(info);
                    }
                    self.turned = Some(Instant::now());
                    LoopTurn::Layer(self.layer, cmd)
                }
                _ => {
                    let rate = self.view.rate;
                    let s = &mut self.view.status;
                    if !editable(s) {
                        return None;
                    }
                    // A millisecond a detent (SHIFT: a tenth).
                    let step = (rate as f32 / if fine { 10_000.0 } else { 1000.0 }).max(1.0);
                    let seam = (s.seam as f32 + ticks as f32 * step).clamp(0.0, MAX_SEAM as f32);
                    s.seam = seam.round() as usize;
                    self.turned = Some(Instant::now());
                    LoopTurn::Seam(s.seam as u32)
                }
            },
        };
        let v = self.view.clone();
        self.refresh_qlinks(ui, &v);
        self.refresh_deck(ui, &v);
        self.refresh_editor(ui, &v);
        Some(turn)
    }

    /// Loop `n`'s level as the screen shows it: what the screen set a
    /// moment ago, else the looper's.
    fn level_of(&self, n: usize, l: &Status) -> f32 {
        match self.levelled.get(n).copied().flatten() {
            Some((v, at)) if at.elapsed() < Duration::from_millis(400) => v,
            _ => l.level,
        }
    }

    /// Whether the loop point view shows (the Q-Links move its start and
    /// end, and the hint line says so).
    pub fn point_showing(ui: &Ui) -> bool {
        ui.get::<LoopEditor>(EDITOR)
            .is_some_and(|e| e.shown_point().is_some())
    }

    fn refresh_qlinks(&mut self, ui: &mut Ui, v: &LooperView) {
        if let Some(look) = ui
            .get::<LoopEditor>(EDITOR)
            .and_then(|e| e.shown_point())
            .cloned()
        {
            let cards = point_qlinks(v, &look);
            if let Some(q) = ui.get_mut::<QLinkColumn>(QLINKS) {
                if q.cards != cards || q.title != "Loop point" {
                    q.cards = cards;
                    q.title = "Loop point".to_string();
                }
            }
            return;
        }
        let loop_card = |n: usize| {
            let l = &v.loops[n];
            let name = format!("Loop {}", n + 1);
            let mut c = if l.phase == Phase::Empty {
                QLinkCard::new(&name, "empty", "-", 0.0)
            } else {
                let level = self.level_of(n, l);
                QLinkCard::new(&name, "level", &level_text(level), level_frac(level))
            };
            c.color = Some(LOOP_COLOURS[n % LOOP_COLOURS.len()]);
            c
        };
        let cards = match self.bank {
            LoopBank::First => [loop_card(0), loop_card(1), loop_card(2), loop_card(3)],
            LoopBank::Second => [loop_card(4), loop_card(5), loop_card(6), loop_card(7)],
            LoopBank::Picked => {
                let n = v.selected;
                let loop_scope = format!("Loop {}", n + 1);
                let s = &v.status;
                let layer = self.layers.get(self.layer).copied().flatten();
                let scope = self.layer_name(self.layer);
                let (c1, c2) = match layer {
                    Some(l) => {
                        let mut pan =
                            QLinkCard::new("Pan", &scope, &pan_text(l.pan), (l.pan + 1.0) / 2.0);
                        pan.bipolar = true;
                        (
                            QLinkCard::new(
                                "Layer level",
                                &scope,
                                &level_text(l.level),
                                level_frac(l.level),
                            ),
                            pan,
                        )
                    }
                    None => {
                        let mut pan = QLinkCard::new("Pan", "No layer", "-", 0.5);
                        pan.bipolar = true;
                        (QLinkCard::new("Layer level", "No layer", "-", 0.0), pan)
                    }
                };
                let c0 = if s.phase == Phase::Empty {
                    QLinkCard::new("Feedback", &loop_scope, "-", 0.0)
                } else {
                    QLinkCard::new(
                        "Feedback",
                        &loop_scope,
                        &format!("{:.0} %", 100.0 * s.fade_old),
                        fade_old_pos(s.fade_old),
                    )
                };
                let c3 = if s.phase == Phase::Empty {
                    QLinkCard::new("Seam fade", &loop_scope, "-", 0.0)
                } else {
                    QLinkCard::new(
                        "Seam fade",
                        &loop_scope,
                        &seam_text(s.seam, v.rate),
                        s.seam as f32 / MAX_SEAM as f32,
                    )
                };
                [c0, c1, c2, c3]
            }
        };
        let title = match self.bank {
            LoopBank::Picked => format!("Loop {}", v.selected + 1),
            b => b.name().to_string(),
        };
        if let Some(q) = ui.get_mut::<QLinkColumn>(QLINKS) {
            if q.cards != cards || q.title != title {
                q.cards = cards;
                q.title = title;
            }
        }
    }

    /// The deck: each loop's channel, the clock and the memory.
    fn refresh_deck(&mut self, ui: &mut Ui, v: &LooperView) {
        let t = &self.transport;
        let beats = usize::from(t.meter.beats.max(1));
        // The meter's own bar and beat (an eighth in 6/8), not quarters.
        let bar_ticks = t.meter.bar_ticks();
        let running = matches!(t.state, State::Playing | State::Recording);
        let pos = t.position.max(0);
        let beat = (pos / t.meter.beat_ticks().max(1)) as usize % beats;
        let bar = pos / bar_ticks.max(1);
        let head = DeckHead {
            beat: running.then_some((beat, beats)),
            clock: match t.state {
                // With a free group about, whose stop it is (Jay's
                // review, 23 Nov, 5.1).
                State::Stopped if v.free.is_some() => "Song stopped".to_string(),
                State::Stopped => "Stopped".to_string(),
                State::CountIn => "Count-in".to_string(),
                _ => format!("Bar {} · beat {}", bar + 1, beat + 1),
            },
            tempo: format!("{:.0} BPM", t.bpm),
            free_tempo: v
                .free
                .map(|c| format!("Free {} BPM", free_bpm_words(c.bpm))),
            free_plays: v.free_runs(),
            memory: memory(v),
            pads: self.pads_keys,
            input: (self.input_words(v).0, input_frac(self.hold)),
            input_next: self.input_words(v).1,
            capture: capture_button(v),
            gauge: String::new(),
            into: false,
        };
        // Loops on audio tracks (board 5): the loops' gauge, and where the
        // next loop goes in Records from's place.
        let mut head = head;
        if let Some(into) = &self.on_tracks {
            let in_use = v.loops.iter().filter(|l| l.phase != Phase::Empty).count();
            head.gauge = format!("LOOPS {in_use} OF {MAX_LOOPS}");
            head.input.0 = into.clone();
            head.input_next = None;
            head.into = true;
        }
        // An armed loop starts on the next bar line (on the bar PLAY
        // starts while stopped).
        let next_bar = running.then_some(bar + 2);
        // A free loop counts on the free clock: its bars from its bar 1,
        // while it runs.
        let free_runs = v.free_runs();
        let free_bar = v.free.map(|c| (c.meter.bar_ticks().max(1), c.top));
        let free_pos = |c: (i64, i64)| (v.free_now as i64 - c.1).max(0);
        let free_next = free_bar
            .filter(|_| free_runs)
            .map(|c| free_pos(c) / c.0 + 2);
        let foot = self.source_word(v);
        // An empty loop records on the clock Sync was last set to (an armed
        // one, or one recording its first pass, on its own).
        let empty_free = |l: &Status| match l.phase {
            Phase::Empty => v.clock == Clock::Free,
            _ => l.clock == Clock::Free,
        };
        let bar_frames = v.frames_per_beat * f64::from(v.beats_per_bar.max(1));
        let channels: Vec<Channel> = (0..MAX_LOOPS)
            .map(|n| {
                let l = &v.loops[n];
                let free = l.clock == Clock::Free;
                let bars = bars_of(l.ticks, v.bar_ticks_of(l));
                let length = if bars == 0 || l.phase == Phase::Empty {
                    String::new()
                } else if free {
                    "free".to_string()
                } else {
                    bars_word(bars)
                };
                // An armed free loop starts on the free clock's bar line
                // while it runs (its `starts_on` is on that clock), else on
                // the song's.
                let (running, pos, bar_ticks, next_bar, top) = match free_bar {
                    Some(c) if free && free_runs => (true, free_pos(c), c.0, free_next, c.1),
                    _ => (running, pos, bar_ticks, next_bar, 0),
                };
                let state = RingState::of_free(l, next_bar, (free_runs, v.free_cued));
                let level = self.level_of(n, l);
                let (position, ring_bars, centre) = match state {
                    RingState::Empty => (0.0, 0, ("+".to_string(), String::new())),
                    // Capture: on the bar line coming, or copying in.
                    RingState::Waiting(b) if l.capturing.is_some() => {
                        let p = l.capturing.unwrap_or(0.0);
                        (
                            p,
                            0,
                            match b {
                                Some(b) if p == 0.0 && v.heard.waits => {
                                    (b.to_string(), "Capture on bar".to_string())
                                }
                                _ => ("+".to_string(), "Capturing".to_string()),
                            },
                        )
                    }
                    // Starts: When I play, and no note yet: play to start.
                    RingState::Waiting(_) if l.listening => {
                        (0.0, 0, ("Play".to_string(), "to start".to_string()))
                    }
                    // Armed on the bar: the bar it starts on, and in the
                    // last bar before it a count of the beats left, so the
                    // start never comes as a surprise.
                    RingState::Waiting(b) => (
                        0.0,
                        0,
                        match (l.starts_on.filter(|_| running).map(|at| at - top), b) {
                            (Some(at), _) => {
                                let ppq = mpfree_transport::time::PPQ as i64;
                                let ppq = ppq.max(1);
                                let left = ((at - pos).max(0) + ppq - 1) / ppq;
                                if left as usize <= beats {
                                    let left = left.max(1);
                                    (
                                        left.to_string(),
                                        if left == 1 {
                                            "beat to REC"
                                        } else {
                                            "beats to REC"
                                        }
                                        .to_string(),
                                    )
                                } else {
                                    (
                                        (at / bar_ticks.max(1) + 1).to_string(),
                                        "REC on bar".to_string(),
                                    )
                                }
                            }
                            (None, Some(b)) => (b.to_string(), "REC on bar".to_string()),
                            (None, None) => ("▶".to_string(), "REC on PLAY".to_string()),
                        },
                    ),
                    // Recording free (tempo from the first loop): no bars
                    // yet, the arc a turn a second.
                    RingState::Recording if l.free => {
                        let secs = l.recorded as f64 / f64::from(v.rate.max(1));
                        (
                            secs.fract() as f32,
                            1,
                            (
                                "Free".to_string(),
                                if l.ending {
                                    "closing".to_string()
                                } else {
                                    format!("REC {secs:.1} s")
                                },
                            ),
                        )
                    }
                    RingState::Recording => {
                        let done = l.recorded as f64 / bar_frames.max(1.0);
                        match l.bars {
                            Some(b) => (
                                (done / f64::from(b.max(1))).clamp(0.0, 1.0) as f32,
                                b as usize,
                                (
                                    ((done.floor() as u32).min(b.max(1) - 1) + 1).to_string(),
                                    format!("of {b}"),
                                ),
                            ),
                            None => (
                                done.fract() as f32,
                                1,
                                ((done.floor() as u64 + 1).to_string(), "REC".to_string()),
                            ),
                        }
                    }
                    // A free loop while the free loops are stopped: where
                    // it waits, and ▶ (a tap plays them).
                    RingState::Stopped(cued) => (
                        l.position,
                        bars,
                        (
                            "▶".to_string(),
                            if cued { "on the bar" } else { "stopped" }.to_string(),
                        ),
                    ),
                    RingState::Off => (
                        l.position,
                        bars,
                        (
                            "off".to_string(),
                            if l.solo_out {
                                "by solo".to_string()
                            } else {
                                String::new()
                            },
                        ),
                    ),
                    _ => {
                        let at = ((l.position * bars as f32) as usize).min(bars.saturating_sub(1));
                        (
                            l.position,
                            bars,
                            ((at + 1).to_string(), format!("of {bars}")),
                        )
                    }
                };
                let wave = if state.has_sound() {
                    v.ring_waves[n].to_vec()
                } else {
                    Vec::new()
                };
                let meter = match state {
                    RingState::Playing | RingState::Overdubbing if !wave.is_empty() => {
                        let i = ((position * wave.len() as f32) as usize).min(wave.len() - 1);
                        level_frac(f32::from(wave[i]) / 255.0 * level)
                    }
                    _ => 0.0,
                };
                Channel {
                    name: format!("Loop {}", n + 1),
                    length,
                    colour: LOOP_COLOURS[n % LOOP_COLOURS.len()],
                    state,
                    position,
                    bars: ring_bars,
                    centre,
                    wave,
                    level,
                    meter: (meter * 20.0).round() / 20.0,
                    foot: if state.has_sound() {
                        String::new()
                    } else if l.kept && state == RingState::Empty {
                        // Cleared: UNDO on it brings it back.
                        "Cleared · UNDO".to_string()
                    } else if empty_free(l) {
                        format!("{foot} · free")
                    } else {
                        foot.clone()
                    },
                    solo: l.solo,
                    unsaved: v.unsaved[n],
                    free: if state.has_sound() {
                        free
                    } else {
                        empty_free(l)
                    },
                    ..Channel::default()
                }
            })
            .enumerate()
            .map(|(n, mut ch)| {
                self.on_track_card(n, &mut ch);
                ch
            })
            .collect();
        // Play's row shows the same loops.
        if let Some(row) = ui.get_mut::<LoopRow>(LOOP_ROW) {
            if row.channels != channels
                || row.picked != v.selected
                || row.capture != head.capture
                || row.pads != head.pads
            {
                row.channels = channels.clone();
                row.picked = v.selected;
                row.capture = head.capture.clone();
                row.pads = head.pads;
            }
        }
        if let Some(d) = ui.get_mut::<LoopDeck>(DECK) {
            if d.head != head || d.channels != channels || d.picked != v.selected {
                d.head = head;
                d.channels = channels;
                d.picked = v.selected;
            }
        }
    }

    fn refresh_editor(&mut self, ui: &mut Ui, v: &LooperView) {
        let s = &v.status;
        let n = v.selected;
        let empty = s.phase == Phase::Empty;
        let has_sound = matches!(s.phase, Phase::Playing | Phase::Overdubbing);
        // A loop on an audio track goes by its name (board 5: "Vox loop",
        // its place in the line under it).
        let title = match self.audio_place(n) {
            Some(p) if !p.clip.is_empty() => p.clip.clone(),
            _ => format!("Loop {}", n + 1),
        };
        let source = self.source_word(v);
        // A free loop (Sync: Free; an empty loop records on the Sync set
        // last).
        let free = match s.phase {
            Phase::Empty => v.clock == Clock::Free,
            _ => s.clock == Clock::Free,
        };
        let song_plays = matches!(self.transport.state, State::Playing | State::Recording);
        let first_note = match now_source(v).line() {
            false => "the first pad you play",
            true => "the line input's first note",
        };
        let records = match (v.resample, now_source(v)) {
            (true, _) => "records the other loops".to_string(),
            (false, LoopSource::Pads) => "records the pads".to_string(),
            (false, LoopSource::Line) => "records the line input".to_string(),
            (false, LoopSource::Mix(mpfree_fx::StripId::Master)) => {
                "records the whole mix but the loops".to_string()
            }
            (false, LoopSource::Mix(m)) => format!("records {}", self.mix_name(m)),
            (false, LoopSource::Track(_)) => "records its audio track's input".to_string(),
        };
        // "records the pads, free": what a free loop records, as its header
        // and the line over the strip say it (its own time: "free" goes
        // with the length instead).
        let records_as = if free {
            format!("{records}, free")
        } else {
            records.clone()
        };
        let sub = {
            let mut parts = Vec::new();
            if let Some(p) = self.place(n) {
                parts.push(format!("{} · {}", p.track_name, p.row));
            }
            match s.phase {
                Phase::Empty => parts.push(format!("empty · {records_as}")),
                Phase::Armed if s.listening => {
                    parts.push(format!("waits for you to play · {records_as}"))
                }
                Phase::Armed if free && v.free_runs() => {
                    parts.push(format!("waits for the free loops' bar line · {records_as}"))
                }
                Phase::Armed => parts.push(format!("waits for the bar line · {records_as}")),
                // With a Length the closing tap says when its bars are
                // done (nothing can count them yet: Jay's review, 5.5).
                Phase::Recording if s.free && free => parts.push(match s.bars {
                    Some(n) if n > 0 => format!(
                        "recording {} free · tap when {} done",
                        bars_word(n as usize),
                        if n == 1 { "it is" } else { "they are" }
                    ),
                    _ => "recording free · its length sets its own tempo; the song stays stopped"
                        .to_string(),
                }),
                Phase::Recording if free => {
                    parts.push("recording the first pass · free".to_string())
                }
                Phase::Recording if s.free => parts.push(format!(
                    "recording {} · its length sets the tempo",
                    crate::looping::free_words(s.bars)
                )),
                Phase::Recording => parts.push("recording the first pass".to_string()),
                _ => {
                    if free {
                        parts.push("free".to_string());
                    }
                    let bars = bars_of(s.ticks, v.bar_ticks_of(s));
                    if bars > 0 {
                        parts.push(bars_word(bars));
                    }
                    parts.push(format!(
                        "{} layer{}",
                        s.layers,
                        if s.layers == 1 { "" } else { "s" }
                    ));
                    // Stopped with the free clock (muted or not).
                    if free && s.phase == Phase::Playing && !v.free_runs() {
                        parts.push("stopped".to_string());
                    }
                }
            }
            parts.join(" · ")
        };
        // Listen: the loop (or a layer) alone while the music is stopped.
        let running = matches!(self.transport.state, State::Playing | State::Recording)
            || self.transport.state == State::CountIn;
        let listening = v.listening.filter(|&(m, _, _)| m == n);
        // A free loop's layers listen alone only while the free loops are
        // stopped too: everything is.
        let listen = (
            has_sound
                && !running
                && !(free && v.free_runs())
                && s.phase == Phase::Playing
                && s.merging.is_none(),
            listening.is_some_and(|(_, l, _)| l.is_none()),
        );
        // A free loop with sound: the free loops' Stop or Play in Listen's
        // place.
        let free_go = (free && has_sound).then_some((v.free_runs(), v.free_cued));
        // And Make song, left of them.
        let song = free_go.map(|_| v.song);
        let names: Vec<String> = (0..s.layers)
            .map(|k| self.names[n].get(k).cloned().unwrap_or_default())
            .collect();
        let layers: Vec<LayerRow> = (0..s.layers)
            .map(|k| {
                let info = self.layers.get(k).copied().flatten();
                // A layer dragged in from another loop says where from.
                let from = info.and_then(|l| {
                    v.history.done().iter().find_map(|s| match *s {
                        Step::Added(id, o) if id == l.id => Some(o),
                        _ => None,
                    })
                });
                let what = match from {
                    Some(o) => format!("from loop {}", o + 1),
                    None if k == 0 => "first".to_string(),
                    None => format!("dub {k}"),
                };
                let how = match info {
                    Some(l) if l.recording => "recording".to_string(),
                    Some(l) if l.muted => "muted".to_string(),
                    // Feedback has turned it down under the overdubs since.
                    Some(l) if l.faded < 0.995 => format!("at {:.0} %", 100.0 * l.faded),
                    // In place of what was under it.
                    Some(l) if l.replace => "replace".to_string(),
                    Some(l) => level_text(l.level),
                    None => String::new(),
                };
                let tag = if how.is_empty() {
                    what
                } else {
                    format!("{what} · {how}")
                };
                LayerRow {
                    name: names
                        .get(k)
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| format!("Layer {}", k + 1)),
                    tag,
                    colour: LAYER_COLOURS[k % LAYER_COLOURS.len()],
                    muted: info.is_some_and(|l| l.muted),
                    solo: info.is_some_and(|l| l.solo),
                    heard: info.is_none_or(|l| l.audible),
                    level: info.map_or(1.0, |l| l.level),
                    pan: info.map_or(0.0, |l| l.pan),
                    faded: info.map_or(1.0, |l| l.faded),
                    recording: info.is_some_and(|l| l.recording),
                    listening: info
                        .is_some_and(|l| listening.is_some_and(|(_, w, _)| w == Some(l.id))),
                }
            })
            .collect();
        let h = &v.history;
        let strip = if has_sound {
            let layer_colour = |id: u8| {
                v.layers
                    .iter()
                    .position(|l| l.is_some_and(|l| l.id == id))
                    .map(|k| LAYER_COLOURS[k % LAYER_COLOURS.len()])
            };
            let grey = LOOP_COLOURS[7];
            let colour = |s: &Step| match *s {
                Step::Overdub(id) | Step::Added(id, _) => layer_colour(id).unwrap_or(grey),
                _ => grey,
            };
            let mut chips: Vec<Chip> = h
                .done()
                .iter()
                .map(|s| Chip {
                    colour: colour(s),
                    kind: ChipKind::Done,
                })
                .collect();
            if s.phase == Phase::Overdubbing {
                chips.push(Chip {
                    colour: LAYER_COLOURS[s.layers.saturating_sub(1) % LAYER_COLOURS.len()],
                    kind: ChipKind::Recording,
                });
            }
            chips.extend(h.redo().iter().map(|s| Chip {
                colour: colour(s),
                kind: ChipKind::Undone,
            }));
            let reversed = h
                .done()
                .iter()
                .filter(|s| matches!(s, Step::Edit(Edit::Reverse)))
                .count()
                % 2
                == 1;
            let merge = match s.merging {
                Some(f) => (format!("Merging {:.0} %", 100.0 * f), false),
                None => ("Merge".to_string(), editable(s) && s.layers > 1),
            };
            Strip::Plays {
                feedback: s.fade_old,
                overdubs: match s.overdubs {
                    Overdubs::Add => 0,
                    Overdubs::Replace => 1,
                    Overdubs::Once => 2,
                },
                reversed,
                speed: (s.speed.clamp(-1, 1) + 1) as u8,
                length: bars_word(bars_of(s.ticks, v.bar_ticks)),
                edits: editable(s),
                chips,
                undo: !h.done().is_empty() || s.phase == Phase::Overdubbing,
                redo: !h.redo().is_empty(),
                merge,
            }
        } else {
            Strip::Records {
                source: u8::try_from(self.source_index(v)).unwrap_or(u8::MAX),
                // Sync as last set while the loop is empty: it takes it,
                // and so does every empty loop after it. Once it records,
                // its own clock, fixed (Jay's review, 23 Nov, 5.2).
                sync: if takes_sync(s) {
                    (u8::from(v.clock == Clock::Free), true)
                } else {
                    (u8::from(s.clock == Clock::Free), false)
                },
                monitor: (!v.resample && v.source == LoopSource::Line && v.line_on)
                    .then_some(v.monitor),
                // Length: Pattern is the app's choice; the looper only
                // sees the bars it stands for.
                bars: if v.match_pattern {
                    0
                } else {
                    BAR_CHOICES.iter().position(|&b| b == s.bars).unwrap_or(1) as u8
                },
                // A resample starts on the bar.
                starts: match (v.resample, s.starts) {
                    (true, _) => (0, false),
                    (false, Starts::Bar) => (0, true),
                    (false, Starts::Play) => (1, true),
                },
                then: match s.after {
                    After::Play => 0,
                    After::Overdub => 1,
                },
            }
        };
        // What Capture takes into this loop: What you played's window.
        let take = v
            .heard
            .take
            .filter(|_| s.capturing.is_none() && v.capture_into.is_some_and(|c| c.0 == n));
        let empty_look = (layers.is_empty()).then(|| EmptyLook {
            title: format!("WHAT YOU PLAYED · {}", source.to_uppercase()),
            says: match s.phase {
                _ if s.capturing.is_some() => format!(
                    "Capture: Loop {} plays once what you played is in (Capture again: stop)",
                    n + 1
                ),
                Phase::Recording if s.free && s.ending && free => format!(
                    "Loop {} closes once it is {} at {:.0} BPM: then its length sets the free \
                     loops' tempo",
                    n + 1,
                    crate::looping::shortest_words(s.bars),
                    mpfree_looper::LOOP_MAX_BPM
                ),
                Phase::Recording if s.free && free => format!(
                    "Loop {} records {}: REC again closes it, and it plays on at its own tempo",
                    n + 1,
                    crate::looping::free_words(s.bars)
                ),
                Phase::Recording if s.free && s.ending => format!(
                    "Loop {} closes once it is {} at {:.0} BPM: then its length sets the tempo",
                    n + 1,
                    crate::looping::shortest_words(s.bars),
                    mpfree_looper::LOOP_MAX_BPM
                ),
                Phase::Recording if s.free => format!(
                    "Loop {} records {}: REC again closes it, and its length sets the tempo \
                     and starts the song",
                    n + 1,
                    crate::looping::free_words(s.bars)
                ),
                Phase::Armed if s.listening && s.free && free => format!(
                    "Loop {} records free when you play: {first_note} starts it, and REC \
                     again closes it at its own tempo",
                    n + 1,
                ),
                Phase::Armed if s.listening && free => format!(
                    "Loop {} records when you play: {first_note} starts it {}",
                    n + 1,
                    if v.free_runs() {
                        "on the free loops' nearest bar line"
                    } else if song_plays {
                        "on the song's nearest bar line"
                    } else {
                        "and the stopped free loops with it"
                    }
                ),
                Phase::Armed if s.listening && s.free => format!(
                    "Loop {} records free when you play: {} starts it, and REC again sets \
                     the tempo",
                    n + 1,
                    match now_source(v).line() {
                        false => "the first pad you play",
                        true => "the line input's first note",
                    }
                ),
                Phase::Armed if s.listening => format!(
                    "Loop {} records when you play: {} starts it {}",
                    n + 1,
                    match now_source(v).line() {
                        false => "the first pad you play",
                        true => "the line input's first note",
                    },
                    match self.transport.state {
                        State::Playing | State::Recording => "on the nearest bar line",
                        _ => "and the song with it",
                    }
                ),
                Phase::Armed if free && v.free_runs() => {
                    format!("Loop {} records from the free loops' next bar line", n + 1)
                }
                Phase::Armed => format!("Loop {} records from the next bar line", n + 1),
                _ if s.kept => format!(
                    "Loop {} cleared: UNDO brings it back, or tap its ring to record",
                    n + 1
                ),
                // A free loop in your own time: nothing plays, no free
                // clock yet.
                Phase::Empty if free && s.free && !v.resample => format!(
                    "Tap Loop {}'s ring or press REC: it {records} {}, in your own time: REC \
                     again closes it, and it plays on at its own tempo",
                    n + 1,
                    crate::looping::free_words(s.bars)
                ),
                Phase::Empty if free => format!(
                    "Tap Loop {}'s ring or press REC: it {records_as}, {}",
                    n + 1,
                    match (v.free_runs(), song_plays, v.free.is_some()) {
                        (true, _, _) => "from the free loops' next bar line",
                        (false, true, true) => {
                            "from the song's next bar line, and the stopped free loops start with it"
                        }
                        (false, true, false) => "from the song's next bar line",
                        (false, false, _) => "from now, and the stopped free loops start with it",
                    }
                ),
                Phase::Empty if s.free && !v.resample => format!(
                    "Tap Loop {}'s ring or press REC: it {records} {}, and its length sets \
                     the tempo",
                    n + 1,
                    crate::looping::free_words(s.bars)
                ),
                _ => format!("Tap Loop {}'s ring or press REC: it {records}", n + 1),
            },
            waits: match s.phase {
                Phase::Recording if s.free && s.ending => Some(match s.bars {
                    Some(b) if b > 1 => format!("Closing at {b} bars"),
                    _ => "Closing at 1 bar".to_string(),
                }),
                Phase::Recording if s.free && free => Some("REC again closes it".to_string()),
                Phase::Recording if s.free => Some("REC again sets the tempo".to_string()),
                Phase::Armed if s.listening => Some("REC starts when you play".to_string()),
                // On the free clock's bar line, counted from its bar 1.
                Phase::Armed if free && v.free_runs() => v.free.map(|c| {
                    let bar = (v.free_now as i64 - c.top).max(0) / c.meter.bar_ticks().max(1);
                    format!("REC starts on bar {}", bar + 2)
                }),
                Phase::Armed => Some(match self.transport.state {
                    State::Playing | State::Recording => {
                        let bar = self.transport.position.max(0)
                            / self.transport.meter.bar_ticks().max(1);
                        format!("REC starts on bar {}", bar + 2)
                    }
                    _ => "REC starts when PLAY does".to_string(),
                }),
                _ => None,
            },
            input: v.heard.cols.iter().map(|&db| input_frac(db)).collect(),
            bars: v.heard.lines[..v.heard.n_lines].to_vec(),
            take: take.map(|(a, b, first, bars)| {
                (
                    a,
                    b,
                    crate::looper_tab::capture_words(first, bars, v.heard.unit, v.heard.pattern),
                )
            }),
            window: take.map(|t| (t.2, t.3)),
            unit: v.heard.unit,
            pattern: v.heard.pattern,
        });
        let history = self
            .history_open
            .then(|| history_list(h, &self.layers, v.rate));
        let trim_ticks = (editable(s) && s.ticks > 0).then_some(s.ticks);
        let live = live_place(
            v,
            ui.get::<LoopEditor>(EDITOR).and_then(|e| e.wave.as_ref()),
        );
        // While Listen plays, the playhead is where it plays.
        let position = match (live, listening) {
            (Some((at, _)), _) => Some(at),
            (None, Some((_, _, at))) => Some(at),
            (None, None) => (!empty && s.frames > 0).then_some(s.position),
        };
        let bars = live.map_or_else(|| bars_of(s.ticks, v.bar_ticks), |(_, b)| b);
        let picked = self.layer.min(s.layers.saturating_sub(1));
        let send_built = [false, false, false, self.place(n).is_some()];
        let colour = LOOP_COLOURS[n % LOOP_COLOURS.len()];
        let r = ui.rect_on(LOOPER_PAGE, EDITOR);
        if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
            // Smart loop end's chip and the loop point view: as they were
            // while a snap lands (the loop is between its two sides).
            let (chip, point) = if s.snapping.is_some() {
                (e.chip.clone(), e.point.clone())
            } else {
                (smart_chip(v), point_look(v))
            };
            if chip.is_none() && s.snapping.is_none() {
                e.close_point();
            }
            if e.chip != chip || e.point != point {
                e.chip = chip;
                e.point = point;
            }
            if e.title != title
                || e.sub != sub
                || e.colour != colour
                || e.listen != listen
                || e.free != free_go
                || e.song != song
                || e.has_sound != has_sound
                || e.layers != layers
                || e.empty != empty_look
                || e.position != position
                || e.bars != bars
                || e.picked != picked
                || e.strip != strip
                || e.history() != history.as_ref()
                || e.trim_ticks != trim_ticks
                || e.send_built != send_built
            {
                e.title = title;
                e.sub = sub;
                e.colour = colour;
                e.listen = listen;
                e.free = free_go;
                e.song = song;
                e.has_sound = has_sound;
                e.layers = layers;
                e.empty = empty_look;
                e.position = position;
                e.bars = bars;
                e.picked = picked;
                e.strip = strip;
                e.set_history(history);
                e.trim_ticks = trim_ticks;
                e.send_built = send_built;
                if !has_sound && e.send_open() {
                    e.close_send();
                }
                if let Some(r) = r {
                    e.follow_in(r);
                }
            }
        }
    }

    pub fn update(&mut self, ui: &mut Ui, v: &LooperView, _settings: &Settings) {
        let mut v = v.clone();
        v.selected = v.selected.min(MAX_LOOPS - 1);
        v.loops[v.selected] = v.status;
        self.follow_takes(&v);
        if self.selected != v.selected {
            self.layer = 0;
            self.history_open = false;
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.close_send();
            }
        }
        self.selected = v.selected;
        // A Q-Link turned a moment ago: keep what it set until the looper
        // has caught up. Only the values it sets, on the same loop: the
        // layers themselves come from the looper, so a layer an overdub
        // adds, or one a move or delete takes away, shows at once, and a
        // lane's × names the layer the looper has there. (Found 16 Nov by
        // the Looper smoke test in a release build: × on a new overdub,
        // within 400 ms of a turn, sent the id of a layer moved away
        // before, and nothing was deleted.)
        let fresh = self
            .turned
            .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
            && v.selected == self.view.selected;
        if fresh {
            for l in v.layers.iter_mut().flatten() {
                if let Some(set) = self.view.layers.iter().flatten().find(|s| s.id == l.id) {
                    l.level = set.level;
                    l.pan = set.pan;
                }
            }
            v.status.fade_old = self.view.status.fade_old;
            v.status.seam = self.view.status.seam;
            v.loops[v.selected] = v.status;
        }
        self.layers = v.layers;
        self.layer = self.layer.min(v.status.layers.saturating_sub(1));
        let v = &v;
        // The input's peak, held for about 2 s of updates.
        self.hold_age += 1;
        let peak = v.meter_db[0].max(v.meter_db[1]);
        if self.hold_age > 8 || peak > self.hold {
            self.hold = peak;
            self.hold_age = 0;
        }
        self.refresh_deck(ui, v);
        self.refresh_editor(ui, v);
        self.refresh_qlinks(ui, v);
        let drops = self.drop_spots(ui, v);
        if ui
            .get::<LoopEditor>(EDITOR)
            .is_some_and(|e| e.drops != drops)
        {
            if let Some(e) = ui.get_mut::<LoopEditor>(EDITOR) {
                e.drops = drops;
            }
        }
        self.view = v.clone();
        self.update_row(ui, v);
    }

    /// The Play page's row: the picked loop's layers as chips (the loops
    /// themselves come with the deck's, [`LooperPage::refresh_deck`]).
    fn update_row(&mut self, ui: &mut Ui, v: &LooperView) {
        let s = &v.status;
        let info = |k: usize| v.layers.get(k).copied().flatten();
        let layers: Vec<LayerChip> = (0..s.layers)
            .map(|k| LayerChip {
                name: self.layer_name(k),
                muted: info(k).is_some_and(|l| l.muted),
                heard: info(k).is_none_or(|l| l.audible),
            })
            .collect();
        if let Some(row) = ui.get_mut::<LoopRow>(LOOP_ROW) {
            if row.layers != layers || row.picked != v.selected {
                row.layers = layers;
                row.picked = v.selected;
            }
        }
    }
}

/// "1 bar", "4 bars".
fn bars_word(bars: usize) -> String {
    format!("{bars} bar{}", if bars == 1 { "" } else { "s" })
}

/// The free clock's tempo as the screen says it, to a tenth of a BPM:
/// "93.4", or "92" when it is that.
pub(crate) fn free_bpm_words(bpm: f64) -> String {
    let tenth = (bpm * 10.0).round() / 10.0;
    if tenth.fract() == 0.0 {
        format!("{tenth:.0}")
    } else {
        format!("{tenth:.1}")
    }
}

/// The memory every loop shares: "37 s of 5 min", and how full, 0 to 1.
/// The deck's Capture: lit while the picked loop captures (a tap calls it
/// off), faint when there is nothing to take or nowhere to put it.
pub fn capture_button(v: &LooperView) -> CaptureButton {
    let picked = &v.loops[v.selected.min(MAX_LOOPS - 1)];
    if picked.capturing.is_some() {
        return CaptureButton {
            label: "Capturing".to_string(),
            works: true,
            lit: true,
        };
    }
    let bars = match (v.capture_into, v.heard.take) {
        (Some(_), Some((_, _, _, bars))) => Some(bars),
        (Some((_, asked)), None) if v.heard.waits => Some(asked),
        _ => None,
    };
    CaptureButton {
        label: match bars {
            Some(1) => "Capture last bar".to_string(),
            Some(b) => format!("Capture last {b} bars"),
            None => "Capture".to_string(),
        },
        works: bars.is_some(),
        lit: false,
    }
}

fn memory(v: &LooperView) -> (String, f32) {
    let rate = f64::from(v.rate.max(1));
    (
        format!(
            "{} of {}",
            memory_time(v.memory.used as f64 / rate, false),
            memory_time(v.memory.capacity as f64 / rate, true)
        ),
        if v.memory.capacity == 0 {
            0.0
        } else {
            v.memory.used as f32 / v.memory.capacity as f32
        },
    )
}

/// While the first pass records and `wave` is its wave so far: the
/// playhead (how far the pass has got along the wave, 0 to 1) and the bars
/// the wave is drawn against.
fn live_place(v: &LooperView, wave: Option<&Arc<LoopWave>>) -> Option<(f32, usize)> {
    if v.status.phase != Phase::Recording {
        return None;
    }
    let w = wave?;
    let at = w.live()?;
    let bar = v.frames_per_beat * f64::from(v.beats_per_bar.max(1));
    let bars = if bar > 0.0 {
        (w.frames as f64 / bar).round().max(1.0) as usize
    } else {
        0
    };
    Some((at, bars))
}

/// Bars in a loop of `ticks`, rounded (0 before it has a length).
/// A loop's length in whole bars of `bar` ticks (the meter's bar), at
/// least 1 for a loop with any length.
/// A length in frames as whole milliseconds, its size only.
fn ms_of(frames: i64, rate: u32) -> i64 {
    (frames.unsigned_abs() as f64 * 1000.0 / f64::from(rate.max(1))).round() as i64
}

/// The tempo at which `ticks` of the song last `frames`.
fn bpm_of(ticks: i64, frames: i64, rate: u32) -> f64 {
    60.0 * f64::from(rate) * ticks as f64
        / (f64::from(mpfree_transport::time::PPQ) * frames.max(1) as f64)
}

/// A difference in ms with its sign, as the loop point view says it:
/// "−48 ms", "+80 ms".
/// The Q-Links while the loop point view shows: Q1 the start and Q2 the
/// end, each where your move (or else the loop) has it, from your press
/// or your tap, its ring where it is across its window; Q3 and Q4 move
/// nothing.
fn point_qlinks(v: &LooperView, look: &PointLook) -> [QLinkCard; 4] {
    let s = &v.status;
    let at = v
        .smart
        .moving
        .map(|m| m.to)
        .or(s.moved)
        .unwrap_or(Shift::NONE);
    let card = |name: &str, f: i64, pane: &PointPane| {
        let pos = pane.moving.as_ref().map_or(pane.edge.0, |m| m.0);
        QLinkCard::new(
            name,
            "Loop point",
            &signed_ms(f, v.rate),
            pos.clamp(0.0, 1.0),
        )
    };
    [
        card("Start", at.start, &look.start),
        card("End", at.end, &look.end),
        QLinkCard::default(),
        QLinkCard::default(),
    ]
}

fn signed_ms(frames: i64, rate: u32) -> String {
    let ms = ms_of(frames, rate);
    match frames.signum() {
        -1 => format!("\u{2212}{ms} ms"),
        1 => format!("+{ms} ms"),
        _ => "0 ms".to_string(),
    }
}

/// Smart loop end's chip on the picked loop's header (Jay's OK on the
/// drawings, 22 Nov): what it found, with Snap; what a snap did, with Use
/// my tap or Use my press; a close call, with Read as the other bars.
/// None once a side is kept, and when there is nothing to say.
pub(crate) fn smart_chip(v: &LooperView) -> Option<SmartChip> {
    let s = &v.status;
    if v.smart.kept || !matches!(s.phase, Phase::Playing | Phase::Overdubbing) {
        return None;
    }
    let ms = |f: i64| ms_of(f, v.rate);
    // Moved by hand: how far, with Use my tap.
    if let Some(m) = s.moved.filter(|_| hand_moved(s)) {
        let way = |f: i64| if f < 0 { "earlier" } else { "later" };
        let words = match (m.start != 0, m.end != 0) {
            (false, _) => format!("Moved {} ms {}", ms(m.end), way(m.end)),
            (true, false) => format!("Start moved {} ms {}", ms(m.start), way(m.start)),
            (true, true) => format!(
                "Moved: start {} ms {}, end {} ms {}",
                ms(m.start),
                way(m.start),
                ms(m.end),
                way(m.end)
            ),
        };
        return Some(SmartChip {
            words,
            action: ("Use my tap".to_string(), ChipAct::Side(Side::Tap), false),
        });
    }
    if let Some(o) = s.offer {
        let mut parts = Vec::new();
        match o.end.signum() {
            -1 => parts.push(format!("{} ms late", ms(o.end))),
            1 => parts.push(format!("{} ms early", ms(o.end))),
            _ => {}
        }
        match o.start.signum() {
            1 => parts.push(format!("first hit {} ms after REC", ms(o.start))),
            -1 => parts.push(format!("first hit {} ms before REC", ms(o.start))),
            _ => {}
        }
        if !parts.is_empty() {
            return Some(SmartChip {
                words: parts.join(" · "),
                action: ("Snap".to_string(), ChipAct::Side(Side::Snap), true),
            });
        }
    }
    if let Some(m) = s.moved.filter(|m| !m.is_none() && s.offer.is_none()) {
        let (words, label) = if m.end != 0 {
            let way = if m.end < 0 { "earlier" } else { "later" };
            (format!("Snapped {} ms {way}", ms(m.end)), "Use my tap")
        } else {
            (
                format!("Started on your first hit, {} ms after REC", ms(m.start)),
                "Use my press",
            )
        };
        return Some(SmartChip {
            words,
            action: (label.to_string(), ChipAct::Side(Side::Tap), false),
        });
    }
    let b = v.smart.close_call?;
    let bpm = bpm_of(i64::from(b) * v.bar_ticks, s.frames as i64, v.rate);
    Some(SmartChip {
        words: "Close call".to_string(),
        action: (
            format!("Read as {} at {bpm:.0} BPM", bars_words_n(b)),
            ChipAct::Snap(Snap::Bars(b)),
            false,
        ),
    })
}

/// The loop point view for the picked loop, while its chip shows and the
/// looper has read the audio round its start and end
/// ([`crate::looping::SmartView::point`]): the start's window from 100 ms
/// before your press, the end's from 200 ms before your tap, each 300 ms
/// (or round the loop's edge, or a move, when that is out of view), with
/// where the loop is, your press or tap, what was found, the hits heard
/// and a move set up; what the other side (or the move) would make of it;
/// the choice, A/B, Read as, and Move start / Move end's step.
pub(crate) fn point_look(v: &LooperView) -> Option<PointLook> {
    smart_chip(v)?;
    let s = &v.status;
    let audio = v.smart.point?;
    // (Not moved is None: the loop is on your press and your tap.)
    let moved = s.moved.unwrap_or(Shift::NONE);
    let rate = v.rate;
    let hand = hand_moved(s);
    let tap_side = s.offer.is_some() && !hand;
    let snap_side = !tap_side && !moved.is_none() && !hand;
    // Loop memory was full as the pass closed: nothing after the tap was
    // kept, so the end can move earlier, not later.
    let short = s.bounds.is_some_and(|b| b.short);
    let offer = if hand {
        Shift::NONE
    } else {
        s.offer.unwrap_or(Shift::NONE)
    };
    let frames = s.frames as i64;
    let bars = bars_of(s.ticks, v.bar_ticks) as u32;
    let bpm_now = bpm_of(s.ticks, frames, rate);
    let now = if hand {
        "moved by hand"
    } else if tap_side || !snap_side {
        "your tap"
    } else if moved.end != 0 {
        "snapped"
    } else {
        "on your first hit"
    };
    let now = format!("Now: {now}, {} at {bpm_now:.2} BPM", bars_words_n(bars));
    let width = (POINT_MS * f64::from(rate) / 1000.0).round() as i64;
    let moving = v.smart.moving;
    let to = moving.map(|m| m.to);
    // The start: where it is and where it was found, from your press.
    let at = |from: i64, f: i64| ((f - from) as f64 / width.max(1) as f64) as f32;
    let at_start = |f: i64| at(audio.from[0], f);
    let at_end = |f: i64| at(audio.from[1], f);
    let hits = |k: usize| -> Vec<f32> {
        audio.hits[k][..usize::from(audio.n_hits[k])]
            .iter()
            .map(|&h| at(audio.from[k], h))
            .collect()
    };
    let found_start = if hand {
        s.found.map_or(0, |f| f.start)
    } else {
        moved.start + offer.start
    };
    let start = PointPane {
        title: "Start".to_string(),
        note: match (found_start != 0, snap_side) {
            _ if hand && moved.start != 0 => format!(
                "moved by hand, {} ms {} REC",
                ms_of(moved.start, rate),
                if moved.start < 0 { "before" } else { "after" }
            ),
            (false, _) => "the start stays on REC".to_string(),
            (true, false) => format!(
                "your first hit came {} ms after REC",
                ms_of(found_start, rate)
            ),
            (true, true) => format!(
                "started on your first hit, {} ms after REC",
                ms_of(found_start, rate)
            ),
        },
        peaks: audio.start,
        edge: (at_start(moved.start), true),
        you: (at_start(0), "REC".to_string()),
        found: (found_start != 0).then(|| {
            (
                at_start(found_start),
                "first hit".to_string(),
                signed_ms(found_start, rate),
            )
        }),
        span: (audio.from[0], width),
        hits: hits(0),
        // A move set up; else, moved by hand, where you put it.
        moving: to
            .map(|t| t.start)
            .or((hand && moved.start != 0).then_some(moved.start))
            .filter(|&f| to.is_none() || f != moved.start)
            .map(|f| (at_start(f), signed_ms(f, rate))),
    };
    // The end, from your tap.
    let found_end = if hand {
        s.found.map_or(0, |f| f.end)
    } else {
        moved.end + offer.end
    };
    let way = if found_end < 0 { "before" } else { "after" };
    // A/B on the seam: whose end runs into the start this time round
    // (the looper plays the other side's from half way, every other
    // time round): true, the other side's.
    let ab_other = s.ab.filter(|_| v.smart.ab);
    let ab_snaps = ab_other
        .filter(|_| moving.is_none())
        .map(|other| other != snap_side);
    let end = PointPane {
        title: "End".to_string(),
        note: match (ab_snaps, found_end != 0, snap_side) {
            _ if moving.is_some() && ab_other == Some(true) => {
                "now: your move's end, into the start".to_string()
            }
            _ if moving.is_some() && ab_other == Some(false) => {
                "now: the loop's own end, into the start".to_string()
            }
            _ if hand && moved.end != 0 => format!(
                "moved by hand, {} ms {} your tap",
                ms_of(moved.end, rate),
                if moved.end < 0 { "before" } else { "after" }
            ),
            (Some(true), ..) => "now: the snap's end, into the start".to_string(),
            (Some(false), ..) => "now: your tap, into the start".to_string(),
            (None, false, _) => "on your tap".to_string(),
            (None, true, false) => format!(
                "your first hit again, {} ms {way} your tap",
                ms_of(found_end, rate)
            ),
            (None, true, true) => {
                format!("snapped {} ms {way} your tap", ms_of(found_end, rate))
            }
        },
        peaks: audio.end,
        edge: (at_end(moved.end), false),
        you: (at_end(0), "your tap".to_string()),
        found: (found_end != 0).then(|| {
            (
                at_end(found_end),
                if snap_side { "snapped" } else { "end found" }.to_string(),
                signed_ms(found_end, rate),
            )
        }),
        span: (audio.from[1], width),
        hits: hits(1),
        moving: to
            .map(|t| t.end)
            .or((hand && moved.end != 0).then_some(moved.end))
            .filter(|&f| to.is_none() || f != moved.end)
            .map(|f| (at_end(f), signed_ms(f, rate))),
    };
    let back = if moved.end != 0 {
        "Use my tap"
    } else {
        "Use my press"
    };
    let side = |label: &str, side: Side, keep: bool| (label.to_string(), Pick::Side(side, keep));
    let (line, pick) = if let Some(m) = moving {
        let mut pick = Vec::new();
        let line = match m.makes {
            Ok(bpm) if v.smart.ab => {
                pick.push(("Move".to_string(), Pick::Move));
                format!(
                    "A/B: each time round, the end runs into the start as the loop has it, \
                     then as your move would. The tempo stays {bpm_now:.2} BPM until you Move \
                     (then {bpm:.2})."
                )
            }
            Ok(bpm) => {
                pick.push(("Move".to_string(), Pick::Move));
                format!(
                    "Your move makes it {} at {bpm:.2} BPM, from the loop's next downbeat.",
                    bars_words_n(bars)
                )
            }
            Err(why) => format!("Your move can't be made: {why}."),
        };
        pick.push(("Cancel".to_string(), Pick::Cancel));
        (line, pick)
    } else if v.smart.ab {
        let (now, other) = if hand {
            (
                "your move has it",
                if s.found.is_some() {
                    "Snap would"
                } else {
                    "your tap would"
                },
            )
        } else if snap_side {
            ("the snap has it", "your tap would")
        } else {
            ("your tap has it", "Snap would")
        };
        (
            format!(
                "A/B: each time round, the end runs into the start as {now}, then as {other}. \
                 The tempo stays {bpm_now:.2} BPM."
            ),
            if hand {
                vec![
                    side("Keep it", Side::Snap, true),
                    side("Use my tap", Side::Tap, false),
                ]
            } else {
                vec![
                    side("Keep the snap", Side::Snap, true),
                    side("Keep my tap", Side::Tap, true),
                ]
            },
        )
    } else if hand {
        let bpm = bpm_of(s.ticks, frames - moved.longer(), rate);
        (
            format!(
                "Use my tap puts it back on REC and your tap: {} at {bpm:.2} BPM, from the \
                 loop's next downbeat.",
                bars_words_n(bars)
            ),
            vec![
                side("Use my tap", Side::Tap, false),
                side("Keep it", Side::Snap, true),
            ],
        )
    } else if short && !snap_side {
        (
            "Loop memory was full when it closed, so nothing after your tap was kept: \
             the end can move earlier, not later."
                .to_string(),
            if tap_side {
                vec![
                    side("Snap", Side::Snap, false),
                    side("Keep my tap", Side::Tap, true),
                ]
            } else {
                vec![side("Keep my tap", Side::Tap, true)]
            },
        )
    } else if tap_side {
        let bpm = bpm_of(s.ticks, frames + offer.longer(), rate);
        (
            format!(
                "Snap makes it {} at {bpm:.2} BPM, from the loop's next downbeat. \
                 Your tap stays one tap away.",
                bars_words_n(bars)
            ),
            vec![
                side("Snap", Side::Snap, false),
                side("Keep my tap", Side::Tap, true),
            ],
        )
    } else if snap_side {
        let bpm = bpm_of(s.ticks, frames - moved.longer(), rate);
        (
            format!(
                "{back} puts it back: {} at {bpm:.2} BPM, from the loop's next downbeat. \
                 The snap stays one tap away.",
                bars_words_n(bars)
            ),
            vec![
                side(back, Side::Tap, false),
                side("Keep the snap", Side::Snap, true),
            ],
        )
    } else {
        (
            "Nothing to snap: the loop is on REC and your tap. Read as keeps its length \
             and changes the tempo; or move its start or end yourself."
                .to_string(),
            vec![side("Keep my tap", Side::Tap, true)],
        )
    };
    let two_sides = tap_side || snap_side || hand || moving.is_some_and(|m| m.makes.is_ok());
    Some(PointLook {
        now,
        start,
        end,
        line,
        pick,
        ab: two_sides.then_some(v.smart.ab),
        read_as: read_as_choices(v, bars),
        bars_now: bars,
        step: v.smart.move_step,
    })
}

/// Read as's choices for the picked loop, `bars` now: those bars, the
/// close call's other reading, double and half, three at most, each at
/// the tempo its bars make of the loop's length (only those the looper
/// takes, [`LOOP_MIN_BPM`] to [`LOOP_MAX_BPM`]); fewest bars first.
fn read_as_choices(v: &LooperView, bars: u32) -> Vec<(String, u32)> {
    let s = &v.status;
    let bpm = |b: u32| bpm_of(i64::from(b) * v.bar_ticks, s.frames as i64, v.rate);
    let mut picks = vec![bars];
    let others = [
        v.smart.close_call,
        Some(bars * 2),
        bars.is_multiple_of(2).then_some(bars / 2),
    ];
    for b in others.into_iter().flatten() {
        let fits = (LOOP_MIN_BPM..=LOOP_MAX_BPM).contains(&bpm(b));
        if b > 0 && fits && !picks.contains(&b) && picks.len() < 3 {
            picks.push(b);
        }
    }
    picks.sort_unstable();
    picks
        .into_iter()
        .map(|b| (format!("{} · {:.2} BPM", bars_words_n(b), bpm(b)), b))
        .collect()
}

pub(crate) fn bars_of(ticks: i64, bar: i64) -> usize {
    if ticks <= 0 {
        return 0;
    }
    (ticks as f64 / bar.max(1) as f64).round().max(1.0) as usize
}

/// The memory gauge's times: "37 s" under a minute, "1:42" above, and the
/// whole size in minutes ("5 min"; it is a little over, whole blocks).
fn memory_time(seconds: f64, whole: bool) -> String {
    let s = seconds.max(0.0);
    if s < 60.0 {
        format!("{s:.0} s")
    } else if whole {
        format!("{:.0} min", (s / 60.0).floor())
    } else {
        let t = s.round() as u64;
        format!("{}:{:02}", t / 60, t % 60)
    }
}

/// A pan as the card prints it: "L 12", "C", "R 30".
pub fn pan_text(pan: f32) -> String {
    let p = (pan * 50.0).round() as i32;
    match p {
        0 => "C".to_string(),
        p if p < 0 => format!("L {}", -p),
        p => format!("R {p}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looper_deck::RingState;
    use mpfree_ui::{Fonts, SoftCanvas, Theme};

    fn page() -> (Ui, LooperPage) {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add(
            "header",
            mpfree_ui::widgets::Header::new(&["Play", "Looper"]),
        );
        ui.add(
            "hint",
            mpfree_ui::widgets::Label::new("", mpfree_ui::Tone::Dim, mpfree_ui::Align::Left),
        );
        let page = LooperPage::add(&mut ui).unwrap();
        ui.show(LOOPER_PAGE).unwrap();
        ui.paint(&mut SoftCanvas::new(1280, 800));
        (ui, page)
    }

    fn layer(id: u8) -> LayerInfo {
        LayerInfo {
            id,
            level: 1.0,
            pan: 0.0,
            muted: false,
            solo: false,
            audible: true,
            recording: false,
            faded: 1.0,
            replace: false,
            born: u64::from(id) + 1,
            closed: true,
        }
    }

    /// Loop 1 playing four bars at 92 BPM with three layers (the third
    /// muted), loop 2 two bars and muted, 16 s of a minute used.
    fn two_loops() -> LooperView {
        let mut v = LooperView {
            frames_per_beat: 44_100.0 * 60.0 / 92.0,
            ..LooperView::default()
        };
        let s = &mut v.status;
        s.phase = Phase::Playing;
        s.layers = 3;
        s.frames = 460_174;
        s.ticks = 4 * 4 * 960;
        s.level = 1.0;
        s.muted = false;
        s.on = true;
        s.position = 0.3;
        v.loops[0] = v.status;
        let mut second = v.status;
        second.layers = 1;
        second.ticks = 2 * 4 * 960;
        second.muted = true;
        v.loops[1] = second;
        for k in 0..3 {
            v.layers[k] = Some(layer(10 + k as u8));
        }
        if let Some(l) = v.layers[2].as_mut() {
            l.muted = true;
            l.audible = false;
        }
        v.memory = mpfree_looper::Memory {
            used: 16 * 44_100,
            capacity: 60 * 44_100,
        };
        v
    }

    /// Free loops (step F2, the design's mockups 1 to 3): a free loop's
    /// card says "free" where a song loop says its bars, and the deck's
    /// head adds "free" and the free clock's tempo. With Sync on Free an
    /// empty loop says "Pads · free" and records "the pads, free"; Sync's
    /// lit segment is Sync as last set. The header has Stop in Listen's
    /// place while the free loops play, and Play while they are stopped,
    /// when their rings rest with ▶ ("on the bar" while a Play waits).
    #[test]
    fn free_loops_say_free_and_rest_with_play_while_stopped() {
        use mpfree_looper::{Clock, FreeClock};
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.loops[0].clock = Clock::Free;
        v.status = v.loops[0];
        v.clock = Clock::Free;
        let mut c = FreeClock::stopped(93.4, v.rate, mpfree_transport::time::Meter::default(), 0);
        c.running = true;
        v.free = Some(c);
        page.update(&mut ui, &v, &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert_eq!(
            (d.channels[0].length.as_str(), d.channels[0].free),
            ("free", true)
        );
        assert_eq!(
            (d.channels[1].length.as_str(), d.channels[1].free),
            ("2 bars", false)
        );
        assert_eq!(d.head.free_tempo.as_deref(), Some("Free 93.4 BPM"));
        // Both groups named over the deck: whose stop it is, and the
        // free group's own state (Jay's review, 5.1).
        assert_eq!(d.head.clock, "Song stopped");
        assert!(d.head.free_plays);
        assert_eq!(d.channels[2].foot, "Pads · free");
        assert_eq!(d.channels[0].state, RingState::Playing);
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.sub, "free · 4 bars · 3 layers");
        assert_eq!(e.free, Some((true, false)), "Stop in Listen's place");
        // Stopped: at rest with ▶, Play in Listen's place.
        c.running = false;
        v.free = Some(c);
        page.update(&mut ui, &v, &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert_eq!(d.channels[0].state, RingState::Stopped(false));
        assert_eq!(
            d.channels[0].centre,
            ("▶".to_string(), "stopped".to_string())
        );
        assert_eq!(
            d.channels[1].state,
            RingState::Muted,
            "a song loop as it was"
        );
        assert!(!d.head.free_plays, "■ over the deck");
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.sub, "free · 4 bars · 3 layers · stopped");
        assert_eq!(e.free, Some((false, false)));
        // A Play waiting for the song's bar line.
        v.free_cued = true;
        page.update(&mut ui, &v, &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert_eq!(d.channels[0].state, RingState::Stopped(true));
        assert_eq!(d.channels[0].centre.1, "on the bar");
        assert_eq!(
            ui.get::<LoopEditor>(EDITOR).unwrap().free,
            Some((false, true))
        );
        // An empty loop picked: Sync as last set, and how it records.
        v.selected = 2;
        v.status = v.loops[2];
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(
            matches!(
                e.strip,
                Strip::Records {
                    sync: (1, true),
                    ..
                }
            ),
            "{:?}",
            e.strip
        );
        assert_eq!(e.sub, "empty · records the pads, free");
        assert_eq!(e.free, None, "Listen's place keeps Listen");
        v.clock = Clock::Song;
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(
            matches!(
                e.strip,
                Strip::Records {
                    sync: (0, true),
                    ..
                }
            ),
            "{:?}",
            e.strip
        );
        assert_eq!(ui.get::<LoopDeck>(DECK).unwrap().channels[2].foot, "Pads");
        // A free first pass in your own time: Sync is its own, fixed; with
        // a Length the closing tap says when its bars are done (Jay's
        // review, 5.5).
        let words = |ui: &mut Ui, page: &mut LooperPage, v: &mut LooperView, bars| {
            let l = &mut v.loops[2];
            (l.phase, l.clock, l.free, l.bars) = (Phase::Recording, Clock::Free, true, bars);
            v.status = v.loops[2];
            page.update(ui, v, &Settings::default());
            let e = ui.get::<LoopEditor>(EDITOR).unwrap();
            assert!(
                matches!(
                    e.strip,
                    Strip::Records {
                        sync: (1, false),
                        ..
                    }
                ),
                "{:?}",
                e.strip
            );
            e.sub.clone()
        };
        assert_eq!(
            words(&mut ui, &mut page, &mut v, Some(4)),
            "recording 4 bars free · tap when they are done"
        );
        assert_eq!(
            words(&mut ui, &mut page, &mut v, Some(1)),
            "recording 1 bar free · tap when it is done"
        );
        assert_eq!(
            words(&mut ui, &mut page, &mut v, None),
            "recording free · its length sets its own tempo; the song stays stopped"
        );
        ui.paint(&mut SoftCanvas::new(1280, 800));
    }

    /// The free clock's tempo as the deck's head says it: to a tenth, and
    /// whole tempos without one.
    #[test]
    fn the_free_clocks_tempo_reads_to_a_tenth() {
        assert_eq!(free_bpm_words(92.0), "92");
        assert_eq!(free_bpm_words(93.44), "93.4");
        assert_eq!(free_bpm_words(119.96), "120");
    }

    /// The audio tracks rethink's phase 4, board 5: with Dev's "Loops on
    /// audio tracks" on, a loop in an audio cell's card is named by its
    /// place over its clip's name, its M and S are its track's, a free
    /// slot says how one is filled, and the header's gauge and Records
    /// into say where the next loop goes. Off, the cards are as before.
    #[test]
    fn with_loops_on_a_card_is_named_by_its_place_and_its_track() {
        use crate::looper_deck::LoopDeck;
        let (mut ui, mut page) = page();
        let v = two_loops();
        let mut places: [Option<LoopPlace>; MAX_LOOPS] = Default::default();
        places[1] = Some(LoopPlace {
            track: 4,
            slot: 2,
            track_name: "Vocal".into(),
            row: "row 3".into(),
            clip: "Vox loop".into(),
            audio: true,
            mute: true,
            solo: false,
        });
        page.set_places(places);
        page.update(&mut ui, &v, &Settings::default());
        let deck = |ui: &Ui| {
            let d = ui.get::<LoopDeck>(crate::looper_deck::DECK).unwrap();
            (d.head.clone(), d.channels.clone())
        };
        let (head, ch) = deck(&ui);
        assert!(
            ch[1].track.is_empty() && ch[1].name != "Vocal \u{b7} row 3",
            "off: as before"
        );
        assert!(!head.into);
        assert!(page.set_on_tracks(Some("Bass \u{b7} row 1 \u{b7} Line L".into())));
        page.update(&mut ui, &v, &Settings::default());
        let (head, ch) = deck(&ui);
        assert_eq!(ch[1].name, "Vocal \u{b7} row 3");
        assert_eq!(ch[1].sub, "Vox loop \u{b7} muted");
        assert_eq!(ch[1].track, "VOCAL'S TRACK");
        assert!(ch[1].track_mute && !ch[1].track_solo);
        assert!(ch[1].length.is_empty());
        // Loop 1 is in no cell: its card as before. A free slot says how
        // one is filled.
        assert!(ch[0].track.is_empty());
        assert_eq!(
            (ch[2].name.as_str(), ch[2].foot.as_str()),
            ("Free", FREE_FOOT)
        );
        assert!(head.into);
        assert_eq!(head.input.0, "Bass \u{b7} row 1 \u{b7} Line L");
        assert!(head.gauge.starts_with("LOOPS 2 OF 8"), "{}", head.gauge);
        // The same again: nothing changed.
        assert!(!page.set_on_tracks(Some("Bass \u{b7} row 1 \u{b7} Line L".into())));
    }

    /// [`two_loops`] with smart loop end: 48 ms found before the tap.
    fn found_late() -> LooperView {
        let mut v = two_loops();
        v.status.offer = Some(Shift {
            start: 0,
            end: -2117,
        });
        v
    }

    #[test]
    fn the_chip_says_what_smart_loop_end_found_or_did_and_offers_the_way_back() {
        let v = found_late();
        let c = smart_chip(&v).unwrap();
        assert_eq!(c.words, "48 ms late");
        assert_eq!(c.action, ("Snap".into(), ChipAct::Side(Side::Snap), true));
        let mut v = two_loops();
        v.status.offer = Some(Shift {
            start: 3528,
            end: 441,
        });
        assert_eq!(
            smart_chip(&v).unwrap().words,
            "10 ms early · first hit 80 ms after REC"
        );
        // Snapped: the way back.
        let mut v = two_loops();
        v.status.moved = Some(Shift {
            start: 0,
            end: -2117,
        });
        v.status.found = v.status.moved;
        let c = smart_chip(&v).unwrap();
        assert_eq!(c.words, "Snapped 48 ms earlier");
        assert_eq!(
            c.action,
            ("Use my tap".into(), ChipAct::Side(Side::Tap), false)
        );
        v.status.moved = Some(Shift {
            start: 3528,
            end: 0,
        });
        v.status.found = v.status.moved;
        let c = smart_chip(&v).unwrap();
        assert_eq!(c.words, "Started on your first hit, 80 ms after REC");
        assert_eq!(c.action.0, "Use my press");
        // A close call: Read as the other bars, at the tempo they make.
        let mut v = two_loops();
        v.smart.close_call = Some(8);
        let c = smart_chip(&v).unwrap();
        assert_eq!(c.words, "Close call");
        assert_eq!(
            c.action,
            (
                "Read as 8 bars at 184 BPM".into(),
                ChipAct::Snap(Snap::Bars(8)),
                false
            )
        );
        // Kept, or nothing to say: no chip.
        let mut v = found_late();
        v.smart.kept = true;
        assert_eq!(smart_chip(&v), None);
        assert_eq!(smart_chip(&two_loops()), None);
        let mut v = found_late();
        v.status.phase = Phase::Recording;
        assert_eq!(smart_chip(&v), None);
    }

    #[test]
    fn the_loop_point_view_shows_both_sides_and_what_the_other_makes_of_the_loop() {
        let mut v = found_late();
        assert_eq!(point_look(&v), None, "no audio read yet");
        v.smart.point = Some(crate::looping::LoopPoint::new(
            [10; crate::looping::POINT_BARS],
            [20; crate::looping::POINT_BARS],
            44_100,
        ));
        let p = point_look(&v).unwrap();
        assert_eq!(p.now, "Now: your tap, 4 bars at 92.00 BPM");
        assert!(
            p.line
                .starts_with("Snap makes it 4 bars at 92.43 BPM, from the loop's next downbeat."),
            "{}",
            p.line
        );
        assert_eq!(p.pick[0], ("Snap".into(), Pick::Side(Side::Snap, false)));
        assert_eq!(
            p.pick[1],
            ("Keep my tap".into(), Pick::Side(Side::Tap, true))
        );
        assert_eq!(p.ab, Some(false));
        // The tap two thirds across the end's window, the end found 48
        // ms before it, the shade after the loop's end (the tap).
        assert!((p.end.you.0 - 2.0 / 3.0).abs() < 1e-3);
        let (f, name, gap) = p.end.found.clone().unwrap();
        assert!((f - (2.0 / 3.0 - 48.0 / 300.0)).abs() < 2e-3, "{f}");
        assert_eq!(
            (name.as_str(), gap.as_str()),
            ("end found", "\u{2212}48 ms")
        );
        assert_eq!(p.end.edge, (p.end.you.0, false));
        assert_eq!(p.end.peaks, [20; crate::looping::POINT_BARS]);
        assert_eq!(p.start.found, None);
        assert_eq!(p.start.note, "the start stays on REC");
        assert!((p.start.you.0 - 1.0 / 3.0).abs() < 1e-3);
        assert_eq!(
            p.read_as,
            vec![
                ("2 bars · 46.00 BPM".to_string(), 2),
                ("4 bars · 92.00 BPM".to_string(), 4),
                ("8 bars · 184.00 BPM".to_string(), 8),
            ]
        );
        // Snapped: the way back, and the snap stays a tap away.
        v.status.offer = None;
        v.status.moved = Some(Shift {
            start: 0,
            end: -2117,
        });
        v.status.found = v.status.moved;
        let p = point_look(&v).unwrap();
        assert!(p.now.starts_with("Now: snapped, "), "{}", p.now);
        assert!(
            p.line
                .starts_with("Use my tap puts it back: 4 bars at 91.58 BPM"),
            "{}",
            p.line
        );
        assert_eq!(
            p.pick[1],
            ("Keep the snap".into(), Pick::Side(Side::Snap, true))
        );
        assert_eq!(p.end.found.unwrap().1, "snapped");
        // A/B on the seam, from the snap: keep one; the End's note says
        // whose end runs into the start this time round.
        v.smart.ab = true;
        v.status.ab = Some(false);
        let p = point_look(&v).unwrap();
        assert_eq!(p.ab, Some(true));
        assert_eq!(p.pick[0].0, "Keep the snap");
        assert_eq!(p.pick[1].0, "Keep my tap");
        assert_eq!(
            p.line,
            "A/B: each time round, the end runs into the start as the snap has it, then as \
             your tap would. The tempo stays 92.00 BPM."
        );
        assert_eq!(p.end.note, "now: the snap's end, into the start");
        v.status.ab = Some(true);
        assert_eq!(
            point_look(&v).unwrap().end.note,
            "now: your tap, into the start"
        );
        // From your tap.
        v.status.moved = None;
        v.status.offer = Some(Shift {
            start: 0,
            end: -2117,
        });
        let p = point_look(&v).unwrap();
        assert!(
            p.line.starts_with(
                "A/B: each time round, the end runs into the start as your tap has it, then \
                 as Snap would."
            ),
            "{}",
            p.line
        );
        assert_eq!(p.end.note, "now: the snap's end, into the start");
        // A close call alone: nothing to snap, no A/B, Read as.
        let mut v = two_loops();
        v.smart.close_call = Some(8);
        v.smart.point = Some(crate::looping::LoopPoint::new(
            [0; crate::looping::POINT_BARS],
            [0; crate::looping::POINT_BARS],
            44_100,
        ));
        let p = point_look(&v).unwrap();
        assert_eq!(p.ab, None);
        assert_eq!(
            p.pick,
            vec![("Keep my tap".into(), Pick::Side(Side::Tap, true))]
        );
        assert!(p.read_as.iter().any(|r| r.1 == 8));
    }

    /// Move start / Move end in the loop point view: a move set up says
    /// what it makes, with Move and Cancel, and marks your move on its
    /// window; during A/B, whose end comes next; one that can't be made
    /// says why, with Cancel alone. Kept, the loop is moved by hand: the
    /// chip says how far with Use my tap, and the view offers Use my tap
    /// and Keep it.
    #[test]
    fn a_move_in_the_loop_point_view_says_what_it_makes_and_a_kept_one_is_by_hand() {
        use crate::looping::Moving;
        let mut v = found_late();
        v.smart.point = Some(crate::looping::LoopPoint::new(
            [10; crate::looping::POINT_BARS],
            [20; crate::looping::POINT_BARS],
            44_100,
        ));
        // 65 ms before your tap.
        let to = Shift {
            start: 0,
            end: -2_867,
        };
        v.smart.moving = Some(Moving {
            to,
            makes: Ok(92.58),
        });
        let p = point_look(&v).unwrap();
        assert_eq!(
            p.line,
            "Your move makes it 4 bars at 92.58 BPM, from the loop's next downbeat."
        );
        assert_eq!(
            p.pick,
            vec![("Move".into(), Pick::Move), ("Cancel".into(), Pick::Cancel)]
        );
        let (f, gap) = p.end.moving.clone().expect("your move on the end");
        assert_eq!(gap, "\u{2212}65 ms");
        assert!((f - (2.0 / 3.0 - 65.0 / 300.0)).abs() < 2e-3, "{f}");
        assert_eq!(p.start.moving, None, "the start stays");
        assert!(p.end.found.is_some(), "what was found stays drawn");
        // A/B: the loop as it is, then your move.
        v.smart.ab = true;
        v.status.ab = Some(true);
        let p = point_look(&v).unwrap();
        assert_eq!(
            p.line,
            "A/B: each time round, the end runs into the start as the loop has it, then as \
             your move would. The tempo stays 92.00 BPM until you Move (then 92.58)."
        );
        assert_eq!(p.end.note, "now: your move's end, into the start");
        v.status.ab = Some(false);
        assert_eq!(
            point_look(&v).unwrap().end.note,
            "now: the loop's own end, into the start"
        );
        // One that can't be made.
        v.smart.ab = false;
        v.status.ab = None;
        v.smart.moving = Some(Moving {
            to,
            makes: Err("too fast"),
        });
        let p = point_look(&v).unwrap();
        assert_eq!(p.line, "Your move can't be made: too fast.");
        assert_eq!(p.pick, vec![("Cancel".into(), Pick::Cancel)]);
        // Kept: moved by hand, 17 ms past what was found.
        v.smart.moving = None;
        v.status.found = v.status.offer;
        v.status.moved = Some(to);
        v.status.offer = Some(Shift { start: 0, end: 750 });
        let c = smart_chip(&v).unwrap();
        assert_eq!(c.words, "Moved 65 ms earlier");
        assert_eq!(
            c.action,
            ("Use my tap".into(), ChipAct::Side(Side::Tap), false)
        );
        let p = point_look(&v).unwrap();
        assert!(
            p.now.starts_with("Now: moved by hand, 4 bars at "),
            "{}",
            p.now
        );
        assert_eq!(p.end.note, "moved by hand, 65 ms before your tap");
        assert_eq!(p.end.moving.unwrap().1, "\u{2212}65 ms");
        assert_eq!(
            p.end.found.unwrap().2,
            "\u{2212}48 ms",
            "still what was found"
        );
        assert!(
            p.line
                .starts_with("Use my tap puts it back on REC and your tap: 4 bars at "),
            "{}",
            p.line
        );
        assert_eq!(
            p.pick,
            vec![
                ("Use my tap".into(), Pick::Side(Side::Tap, false)),
                ("Keep it".into(), Pick::Side(Side::Snap, true)),
            ]
        );
    }

    /// Loop memory full as the pass closed: the line says the end can
    /// move earlier, not later, in place of what Snap would make or
    /// "nothing to snap"; a snap made says the way back as before.
    #[test]
    fn the_loop_point_view_says_when_loop_memory_was_full() {
        let point = crate::looping::LoopPoint::new(
            [10; crate::looping::POINT_BARS],
            [20; crate::looping::POINT_BARS],
            44_100,
        );
        let short = |v: &mut LooperView| {
            let end = 8_192 + v.status.frames as i64;
            v.status.bounds = Some(mpfree_looper::Bounds {
                src: 8_192,
                start: 8_192,
                captured: (8_192, end),
                kept: end,
                short: true,
            });
        };
        let full = "Loop memory was full when it closed, so nothing after your tap was kept: \
                    the end can move earlier, not later.";
        // The end found before a late tap: Snap still offers it.
        let mut v = found_late();
        v.smart.point = Some(point);
        short(&mut v);
        let p = point_look(&v).unwrap();
        assert_eq!(p.line, full);
        assert_eq!(
            p.pick,
            vec![
                ("Snap".into(), Pick::Side(Side::Snap, false)),
                ("Keep my tap".into(), Pick::Side(Side::Tap, true)),
            ]
        );
        // A close call alone: nothing to snap.
        let mut v = two_loops();
        v.smart.close_call = Some(8);
        v.smart.point = Some(point);
        short(&mut v);
        let p = point_look(&v).unwrap();
        assert_eq!(p.line, full);
        assert_eq!(
            p.pick,
            vec![("Keep my tap".into(), Pick::Side(Side::Tap, true))]
        );
        // Snapped: the way back, as with room to spare; A/B as it was.
        let mut v = found_late();
        v.smart.point = Some(point);
        short(&mut v);
        v.status.offer = None;
        v.status.moved = Some(Shift {
            start: 0,
            end: -2117,
        });
        v.status.found = v.status.moved;
        let p = point_look(&v).unwrap();
        assert!(p.line.starts_with("Use my tap puts it back"), "{}", p.line);
        v.smart.ab = true;
        assert!(point_look(&v).unwrap().line.starts_with("A/B: "));
        // Not full: as before.
        let mut v = found_late();
        v.smart.point = Some(point);
        short(&mut v);
        v.status.bounds.as_mut().unwrap().short = false;
        assert!(point_look(&v).unwrap().line.starts_with("Snap makes it"));
    }

    #[test]
    fn read_as_keeps_to_the_tempos_the_looper_takes() {
        // 1 bar at 200 BPM: 2 bars would be 400, half isn't a bar.
        let mut v = two_loops();
        v.status.ticks = 4 * 960;
        v.status.frames = 52_920;
        let r = read_as_choices(&v, 1);
        assert_eq!(r, vec![("1 bar · 200.00 BPM".to_string(), 1)]);
    }

    #[test]
    fn the_point_view_closes_when_there_is_nothing_left_in_it() {
        let (mut ui, mut page) = page();
        let mut v = found_late();
        v.smart.point = Some(crate::looping::LoopPoint::new(
            [10; crate::looping::POINT_BARS],
            [20; crate::looping::POINT_BARS],
            44_100,
        ));
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get_mut::<LoopEditor>(EDITOR).unwrap();
        assert!(e.chip.is_some() && e.point.is_some());
        e.open_point();
        let _ = crate::looper_tab::take_actions(&mut ui);
        // A snap lands: chip and view as they were meanwhile.
        let mut landing = v.clone();
        landing.status.offer = None;
        landing.status.snapping = Some(1000);
        page.update(&mut ui, &landing, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.chip.as_ref().unwrap().words, "48 ms late");
        assert!(e.point_open());
        // Kept: the chip goes, and the view closes and says so.
        v.smart.kept = true;
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(e.chip.is_none() && !e.point_open());
        assert_eq!(
            crate::looper_tab::take_actions(&mut ui),
            vec![crate::looper_tab::LoopAct::PointView(false)]
        );
    }

    #[test]
    fn the_page_fits_the_deck_four_lanes_and_the_q_links() {
        let (ui, _) = page();
        let deck = ui.rect_of(DECK).unwrap();
        let editor = ui.rect_of(EDITOR).unwrap();
        let q = ui.rect_of(QLINKS).unwrap();
        assert_eq!((deck.y, deck.w, deck.h), (84, 1116, 240), "{deck:?}");
        assert_eq!((editor.y, editor.h), (336, 414), "{editor:?}");
        assert_eq!(q.w, 128);
        let mut e = LoopEditor::default();
        e.layers = vec![LayerRow::default(); 6];
        let lanes = e.lane_rects(LoopEditor::parts(editor).lanes);
        assert_eq!(lanes.len(), 4, "four lanes show");
        assert!(lanes.iter().all(|(_, r)| r.h >= 60), "{lanes:?}");
    }

    /// Audio tracks phase 4c.3: a loop holding a layer not yet on the
    /// stick has its ring's dot.
    #[test]
    fn a_loop_with_a_layer_not_on_the_stick_has_its_dot() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.unsaved[1] = true;
        page.update(&mut ui, &v, &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        let dots: Vec<bool> = d.channels.iter().map(|c| c.unsaved).collect();
        assert_eq!(dots[..3], [false, true, false]);
    }

    #[test]
    fn the_deck_shows_every_loop_and_the_picked_loops_lanes() {
        let (mut ui, mut page) = page();
        page.update(&mut ui, &two_loops(), &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        let rows: Vec<(&str, &str, RingState)> = d
            .channels
            .iter()
            .map(|c| (c.name.as_str(), c.length.as_str(), c.state))
            .collect();
        assert_eq!(rows.len(), 8, "every loop, always");
        assert_eq!(rows[0], ("Loop 1", "4 bars", RingState::Playing));
        assert_eq!(rows[1], ("Loop 2", "2 bars", RingState::Muted));
        assert_eq!(rows[2], ("Loop 3", "", RingState::Empty));
        // Loop 1 is 30 % through: in its second bar of four.
        assert_eq!(d.channels[0].centre, ("2".to_string(), "of 4".to_string()));
        assert_eq!(d.channels[0].bars, 4);
        assert_eq!(d.channels[2].centre.0, "+");
        assert_eq!(d.channels[2].foot, "Pads", "what an empty one records");
        assert_eq!(d.picked, 0);
        assert!(d.channels.iter().all(|c| !c.unsaved));
        assert_eq!(d.head.memory.0, "16 s of 1 min");
        assert!((d.head.memory.1 - 16.0 / 60.0).abs() < 1e-4);
        assert_eq!(d.head.clock, "Stopped");
        // Above a minute, minutes and seconds, and the size in minutes.
        assert_eq!(super::memory_time(102.4, false), "1:42");
        assert_eq!(super::memory_time(300.4, true), "5 min");
        assert_eq!(super::memory_time(1200.9, true), "20 min");
        assert_eq!(super::memory_time(59.4, true), "59 s");
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.title, "Loop 1");
        assert_eq!(e.sub, "4 bars · 3 layers");
        assert_eq!(ui.get::<LoopDeck>(DECK).unwrap().head.input.0, "Pads");
        assert!(e.has_sound);
        let tags: Vec<&str> = e.layers.iter().map(|l| l.tag.as_str()).collect();
        assert_eq!(tags, ["first · 0.0 dB", "dub 1 · 0.0 dB", "dub 2 · muted"]);
        assert!(e.layers[2].muted && !e.layers[2].heard && e.layers[0].heard);
        let Strip::Plays {
            length,
            edits,
            chips,
            undo,
            merge,
            ..
        } = &e.strip
        else {
            panic!("a loop that plays shows how it can change");
        };
        assert_eq!((length.as_str(), *edits, *undo), ("4 bars", true, false));
        assert!(chips.is_empty() && merge.1);
        assert_eq!(e.send_built, [false, false, false, false]);
        // An empty loop picked: how it will record.
        let mut v = two_loops();
        v.selected = 5;
        v.status = v.loops[5];
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.title, "Loop 6");
        assert_eq!(e.sub, "empty · records the pads");
        assert!(e.layers.is_empty() && !e.has_sound);
        let look = e.empty.as_ref().unwrap();
        assert_eq!(look.title, "WHAT YOU PLAYED · PADS");
        assert!(look.says.contains("ring"), "{}", look.says);
        assert!(
            matches!(
                e.strip,
                Strip::Records {
                    source: 0,
                    monitor: None,
                    then: 0,
                    ..
                }
            ),
            "{:?}",
            e.strip
        );
        assert_eq!(ui.get::<LoopDeck>(DECK).unwrap().picked, 5);
        ui.paint(&mut SoftCanvas::new(1280, 800));
    }

    /// An empty loop picked shows What you played: the bars heard, and
    /// Capture over the last ones when it is where Capture goes. The
    /// deck's Capture says how many bars, is faint with nothing to take,
    /// and lit while the picked loop captures.
    #[test]
    fn what_you_played_and_the_decks_capture() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.selected = 4;
        v.status = v.loops[4];
        v.heard.lines[..3].copy_from_slice(&[(9, 0.2), (10, 0.45), (11, 0.7)]);
        v.heard.n_lines = 3;
        v.heard.take = Some((0.2, 0.7, 9, 2));
        v.capture_into = Some((4, 2));
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let look = e.empty.as_ref().unwrap();
        assert_eq!(look.bars, vec![(9, 0.2), (10, 0.45), (11, 0.7)]);
        assert_eq!(
            look.take,
            Some((0.2, 0.7, "Capture bars 9 and 10".to_string()))
        );
        assert_eq!(look.input.len(), crate::looping::HEARD_COLS);
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert_eq!(
            d.head.capture,
            CaptureButton {
                label: "Capture last 2 bars".into(),
                works: true,
                lit: false
            }
        );
        // Capture goes elsewhere: no button over the bars here.
        v.capture_into = Some((5, 2));
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert_eq!(e.empty.as_ref().unwrap().take, None);
        // Nothing heard on the bars: faint.
        v.heard.take = None;
        page.update(&mut ui, &v, &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert_eq!(
            (d.head.capture.label.as_str(), d.head.capture.works),
            ("Capture", false)
        );
        // Waiting for the bar line: it takes the bars it asks for then.
        v.heard.waits = true;
        assert_eq!(capture_button(&v).label, "Capture last 2 bars");
        // The picked loop captures: lit.
        v.status.capturing = Some(0.3);
        v.loops[4] = v.status;
        page.update(&mut ui, &v, &Settings::default());
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert!(d.head.capture.lit && d.head.capture.label == "Capturing");
        // Its ring waits, as an armed loop's (a tap calls it off).
        let c = &d.channels[4];
        assert_eq!((c.state, c.position), (RingState::Waiting(None), 0.3));
        assert_eq!(c.centre.1, "Capturing");
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(e
            .empty
            .as_ref()
            .unwrap()
            .says
            .starts_with("Capture: Loop 5 plays"));
        ui.paint(&mut SoftCanvas::new(1280, 800));
    }

    #[test]
    fn a_waiting_loop_counts_the_beats_in_its_last_bar() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.selected = 4;
        v.loops[4].phase = Phase::Armed;
        // Armed to start on bar 14's line (tick 13 bars in).
        v.loops[4].starts_on = Some(13 * 4 * 960);
        v.status = v.loops[4];
        page.update(&mut ui, &v, &Settings::default());
        let at = |page: &mut LooperPage, ui: &mut Ui, position| {
            page.set_transport(
                ui,
                TransportInfo {
                    state: State::Playing,
                    position,
                    bpm: 120.0,
                    ..TransportInfo::default()
                },
            );
            ui.get::<LoopDeck>(DECK).unwrap().channels[4].centre.clone()
        };
        // More than a bar away: the bar it starts on.
        assert_eq!(
            at(&mut page, &mut ui, 11 * 4 * 960 + 10),
            ("14".to_string(), "REC on bar".to_string())
        );
        // In the last bar: the beats left.
        assert_eq!(
            at(&mut page, &mut ui, 12 * 4 * 960 + 10),
            ("4".to_string(), "beats to REC".to_string())
        );
        assert_eq!(
            at(&mut page, &mut ui, 12 * 4 * 960 + 3 * 960 + 10),
            ("1".to_string(), "beat to REC".to_string())
        );
    }

    #[test]
    fn a_waiting_loop_says_the_bar_it_starts_on() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.selected = 4;
        v.status = v.loops[4];
        v.status.phase = Phase::Armed;
        v.status.bars = Some(2);
        v.source = LoopSource::Line;
        v.line_on = true;
        page.update(&mut ui, &v, &Settings::default());
        // Bar 12, beat 2.
        let t = TransportInfo {
            state: State::Playing,
            position: (11 * 4 + 1) * 960,
            bpm: 92.0,
            ..TransportInfo::default()
        };
        page.set_transport(&mut ui, t);
        let d = ui.get::<LoopDeck>(DECK).unwrap();
        assert_eq!(d.channels[4].state, RingState::Waiting(Some(13)));
        assert_eq!(d.channels[4].centre.0, "13");
        assert_eq!(d.channels[4].foot, "Line in");
        assert_eq!(d.head.clock, "Bar 12 · beat 2");
        assert_eq!(d.head.beat, Some((1, 4)));
        assert_eq!(d.head.tempo, "92 BPM");
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let look = e.empty.as_ref().unwrap();
        assert_eq!(look.waits.as_deref(), Some("REC starts on bar 13"));
        assert_eq!(look.title, "WHAT YOU PLAYED · LINE IN");
        assert_eq!(
            e.strip,
            Strip::Records {
                source: 1,
                sync: (0, false),
                monitor: Some(false),
                bars: 3,
                starts: (0, true),
                then: 0
            },
            "the line in shows its monitor; 2 bars is the fourth length (after Pattern); \
             armed, its Sync is its own"
        );
        // Records from the loops.
        v.resample = true;
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(ui.get::<LoopDeck>(DECK).unwrap().head.input.0, "Loops");
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(matches!(
            e.strip,
            Strip::Records {
                source: 2,
                monitor: None,
                ..
            }
        ));
    }

    #[test]
    fn a_ring_shows_the_first_pass_recording_through_its_bars() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.selected = 2;
        v.status = v.loops[2];
        v.status.phase = Phase::Recording;
        v.status.bars = Some(4);
        let bar = v.frames_per_beat * 4.0;
        v.status.recorded = (bar * 2.5) as usize;
        page.update(&mut ui, &v, &Settings::default());
        let ch = &ui.get::<LoopDeck>(DECK).unwrap().channels[2];
        assert_eq!(ch.state, RingState::Recording);
        assert_eq!(ch.centre, ("3".to_string(), "of 4".to_string()));
        assert!((ch.position - 0.625).abs() < 0.01, "{}", ch.position);
        // Free: round each bar, counting them.
        v.status.bars = None;
        page.update(&mut ui, &v, &Settings::default());
        let ch = &ui.get::<LoopDeck>(DECK).unwrap().channels[2];
        assert_eq!(ch.centre, ("3".to_string(), "REC".to_string()));
        assert!((ch.position - 0.5).abs() < 0.01, "{}", ch.position);
    }

    #[test]
    fn layers_are_named_by_what_was_played_into_them_in_each_loop() {
        let (mut ui, mut page) = page();
        let mut v = LooperView::default();
        fn step(
            ui: &mut Ui,
            v: &mut LooperView,
            page: &mut LooperPage,
            phase: Phase,
            layers: usize,
            hits: &[&'static str],
        ) {
            v.status.phase = phase;
            v.status.layers = layers;
            v.status.frames = if phase == Phase::Empty { 0 } else { 1000 };
            page.update(ui, v, &Settings::default());
            for h in hits {
                page.hit(h);
            }
            page.update(ui, v, &Settings::default());
        }
        let drums = ["Drums", "Drums", "Bass"];
        step(&mut ui, &mut v, &mut page, Phase::Recording, 1, &drums);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 1, &["Bass"]);
        let bass = ["Bass", "Drums", "Bass"];
        step(&mut ui, &mut v, &mut page, Phase::Overdubbing, 2, &bass);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 2, &[]);
        step(
            &mut ui,
            &mut v,
            &mut page,
            Phase::Overdubbing,
            3,
            &["Drums"],
        );
        step(&mut ui, &mut v, &mut page, Phase::Playing, 3, &[]);
        step(&mut ui, &mut v, &mut page, Phase::Overdubbing, 4, &[]);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 4, &[]);
        let names: Vec<String> = (0..4).map(|k| page.layer_name(k)).collect();
        assert_eq!(names, ["Drums", "Bass", "Drums 2", "Layer 4"]);
        // Play's loop row names the picked loop's layers the same.
        let row = ui.get::<LoopRow>(LOOP_ROW).unwrap();
        let chips: Vec<&str> = row.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(chips, names);
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let lanes: Vec<&str> = e.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(lanes, names);
        // Loop 2 has names of its own; loop 1 keeps its.
        let first = v.status;
        v.loops[0] = first;
        v.selected = 1;
        v.status = v.loops[1];
        step(&mut ui, &mut v, &mut page, Phase::Recording, 1, &["Synth"]);
        step(&mut ui, &mut v, &mut page, Phase::Playing, 1, &[]);
        assert_eq!(page.layer_name(0), "Synth");
        v.loops[1] = v.status;
        v.selected = 0;
        v.status = first;
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(page.layer_name(2), "Drums 2");
        // A clear forgets them.
        step(&mut ui, &mut v, &mut page, Phase::Empty, 0, &[]);
        assert_eq!(page.layer_name(0), "Layer 1");
    }

    #[test]
    fn the_undo_history_names_its_steps_and_opens_in_the_lanes() {
        let v = two_loops();
        let layers = v.layers;
        let id = |k: usize| layers[k].unwrap().id;
        let text = |s: Step| step_text(s, &layers, 44_100);
        assert_eq!(text(Step::Overdub(id(1))), "Overdub · Layer 2");
        assert_eq!(text(Step::Delete(id(2))), "Delete · Layer 3");
        assert_eq!(text(Step::Delete(99)), "Delete a layer");
        assert_eq!(text(Step::Edit(Edit::Nudge(-1))), "Nudge ← 1/16");
        assert_eq!(text(Step::Edit(Edit::Nudge(2))), "Nudge 2 × 1/16 →");
        assert_eq!(text(Step::Edit(Edit::SeamFade(441))), "Seam fade 10 ms");
        assert_eq!(text(Step::Edit(Edit::Half)), "½ length");
        assert_eq!(text(Step::Edit(Edit::Speed(-1))), "Speed ½×");
        assert_eq!(text(Step::Edit(Edit::Speed(1))), "Speed 2×");
        assert_eq!(text(Step::Edit(Edit::Move(-2_205))), "Move ← 50 ms");
        assert_eq!(text(Step::Edit(Edit::Resize(441))), "End → 10 ms");
        assert_eq!(text(Step::Edit(Edit::Turn(882))), "Bar 1 → 20 ms");
        let snap = |start, end| text(Step::Snap(mpfree_looper::Shift { start, end }));
        assert_eq!(snap(0, -2_205), "Snap · End ← 50 ms");
        assert_eq!(snap(3_528, -441), "Snap · Start → 80 ms · End ← 10 ms");
        assert_eq!(snap(-3_528, 0), "Snap · Start ← 80 ms");
        assert_eq!(text(Step::Bars(4)), "Read as 4 bars");
        assert_eq!(text(Step::Bars(1)), "Read as 1 bar");
        assert_eq!(text(Step::Merge(id(0))), "Merge layers");
        let (mut ui, mut page) = page();
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        assert!(e.history().is_none());
        // The loop plays, so the ruler's grips trim it.
        assert_eq!(e.trim_ticks, Some(4 * 4 * 960));
        let s = &v.status;
        assert_eq!(
            trim_command(960, 3840, s),
            Some(Command::Edit(Edit::Trim {
                from: 960,
                to: 3840
            }))
        );
        assert_eq!(trim_command(960, 960, s), None);
        assert_eq!(trim_command(0, 4 * 4 * 960 + 240, s), None);
        page.toggle_history(&mut ui);
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let h = e.history().unwrap();
        assert_eq!(
            (h.rows.as_slice(), h.now),
            (&["Start: the first pass".to_string()][..], 0)
        );
        assert_eq!(page.history_steps(0), None, "now is no step");
        ui.paint(&mut SoftCanvas::new(1280, 800));
        // Picking another loop closes it.
        let mut other = v.clone();
        other.selected = 1;
        other.status = v.loops[1];
        page.update(&mut ui, &other, &Settings::default());
        assert!(!page.history_open());
        assert!(ui.get::<LoopEditor>(EDITOR).unwrap().history().is_none());
    }

    #[test]
    fn the_controls_ask_for_the_looper_edits_while_the_loop_plays() {
        let v = two_loops();
        let mut s = v.status;
        assert_eq!(
            strip_command(Ctl::Half, &s),
            Some(Command::Edit(Edit::Half))
        );
        assert_eq!(
            strip_command(Ctl::Double, &s),
            Some(Command::Edit(Edit::Double))
        );
        assert_eq!(
            strip_command(Ctl::Reverse, &s),
            Some(Command::Edit(Edit::Reverse))
        );
        assert_eq!(strip_command(Ctl::Merge, &s), Some(Command::Merge));
        assert_eq!(
            strip_command(Ctl::Then(1), &s),
            Some(Command::SetAfter(After::Overdub))
        );
        assert_eq!(strip_command(Ctl::Undo, &s), None, "UNDO's own");
        // Overdubs: Add, Replace (in place of what was there), or Once (one
        // pass, then back to playing).
        assert_eq!(
            strip_command(Ctl::Overdubs(1), &s),
            Some(Command::SetOverdubs(Overdubs::Replace))
        );
        assert_eq!(
            strip_command(Ctl::Overdubs(2), &s),
            Some(Command::SetOverdubs(Overdubs::Once))
        );
        assert_eq!(
            strip_command(Ctl::Overdubs(0), &s),
            Some(Command::SetOverdubs(Overdubs::Add))
        );
        // Speed: half, as recorded, double; the speed it has does nothing.
        assert_eq!(
            strip_command(Ctl::Speed(0), &s),
            Some(Command::Edit(Edit::Speed(-1)))
        );
        assert_eq!(
            strip_command(Ctl::Speed(2), &s),
            Some(Command::Edit(Edit::Speed(1)))
        );
        assert_eq!(strip_command(Ctl::Speed(1), &s), None);
        let mut half = s;
        half.speed = -1;
        assert_eq!(
            strip_command(Ctl::Speed(1), &half),
            Some(Command::Edit(Edit::Speed(0)))
        );
        // One layer has nothing to merge, and says so.
        s.layers = 1;
        assert_eq!(strip_command(Ctl::Merge, &s), None);
        assert!(why_faint(Ctl::Merge, &s).starts_with("Merge needs two layers"));
        // While overdubbing, or merging, no edit works.
        s.phase = Phase::Overdubbing;
        assert_eq!(strip_command(Ctl::Half, &s), None);
        assert!(why_faint(Ctl::Half, &s).contains("not while it overdubs"));
        s.phase = Phase::Playing;
        s.merging = Some(0.4);
        assert_eq!(strip_command(Ctl::Reverse, &s), None);
        assert_eq!(why_faint(Ctl::Reverse, &s), "Wait for the merge to finish");
        // The panel's Merge says how far it has got.
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.status.merging = Some(0.4);
        page.update(&mut ui, &v, &Settings::default());
        let Strip::Plays { merge, edits, .. } = &ui.get::<LoopEditor>(EDITOR).unwrap().strip else {
            panic!("plays");
        };
        assert_eq!(
            (merge.0.as_str(), merge.1, *edits),
            ("Merging 40 %", false, false)
        );
        // The strip shows the loop's speed, its length in bars with it.
        let mut v = two_loops();
        v.status.speed = -1;
        v.status.ticks *= 2;
        page.update(&mut ui, &v, &Settings::default());
        let Strip::Plays { speed, length, .. } = &ui.get::<LoopEditor>(EDITOR).unwrap().strip
        else {
            panic!("plays");
        };
        assert_eq!((*speed, length.as_str()), (0, "8 bars"));
    }

    #[test]
    fn the_q_links_turn_the_loops_levels_then_the_picked_loop() {
        let (mut ui, mut page) = page();
        let v = two_loops();
        page.update(&mut ui, &v, &Settings::default());
        let card = |ui: &Ui, k: usize| {
            let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
            (q.cards[k].name.clone(), q.cards[k].value.clone())
        };
        assert_eq!(page.bank(), LoopBank::First);
        assert_eq!(card(&ui, 0), ("Loop 1".to_string(), "0.0 dB".to_string()));
        assert_eq!(card(&ui, 2), ("Loop 3".to_string(), "-".to_string()));
        // Q1: loop 1's level, half a dB a detent; the deck follows.
        let Some(LoopTurn::LoopLevel(0, l)) = page.qlink(&mut ui, 0, -4, false) else {
            panic!("Q1 turns loop 1's level");
        };
        assert!((l - 10f32.powf(-2.0 / 20.0)).abs() < 1e-4);
        assert_eq!(card(&ui, 0).1, "-2.0 dB");
        assert!((ui.get::<LoopDeck>(DECK).unwrap().channels[0].level - l).abs() < 1e-4);
        // The looper's next view (from before the turn) does not undo it.
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(card(&ui, 0).1, "-2.0 dB");
        // An empty loop has no level to turn.
        assert_eq!(page.qlink(&mut ui, 2, 1, false), None);
        // Q-LINK: loops 5 to 8, then the picked loop.
        assert_eq!(page.next_bank(&mut ui, false), LoopBank::Second);
        assert_eq!(card(&ui, 0).0, "Loop 5");
        assert_eq!(page.next_bank(&mut ui, false), LoopBank::Picked);
        page.pick_layer(&mut ui, 1);
        assert_eq!(ui.get::<LoopEditor>(EDITOR).unwrap().picked, 1);
        assert_eq!(card(&ui, 0), ("Feedback".to_string(), "100 %".to_string()));
        assert_eq!(
            card(&ui, 1),
            ("Layer level".to_string(), "0.0 dB".to_string())
        );
        // Q2: the layer's level; SHIFT in tenths.
        let Some(LoopTurn::Layer(1, LayerCommand::Level(l))) = page.qlink(&mut ui, 1, 2, false)
        else {
            panic!("Q2 turns the picked layer's level");
        };
        assert!((l - 10f32.powf(1.0 / 20.0)).abs() < 1e-4);
        page.qlink(&mut ui, 1, -3, true);
        assert_eq!(card(&ui, 1).1, "0.7 dB");
        // Q3: its pan.
        assert_eq!(
            page.qlink(&mut ui, 2, -6, false),
            Some(LoopTurn::Layer(1, LayerCommand::Pan(-0.12)))
        );
        assert_eq!(card(&ui, 2).1, "L 6");
        // Q1: Feedback, a percent a detent; Q4: the seam fade, a
        // millisecond a detent.
        let Some(LoopTurn::FadeOld(k)) = page.qlink(&mut ui, 0, -2, false) else {
            panic!("Q1 turns the loop's Feedback");
        };
        assert!((k - 0.98).abs() < 1e-4, "{k}");
        assert_eq!(card(&ui, 0).1, "98 %");
        let before = v.status.seam;
        let Some(LoopTurn::Seam(f)) = page.qlink(&mut ui, 3, 10, false) else {
            panic!("Q4 turns the loop's seam fade");
        };
        assert_eq!(f as usize, before + 441);
        assert_eq!(card(&ui, 3).1, seam_text(before + 441, 44_100));
        assert_eq!(page.qlink(&mut ui, 0, 0, false), None);
        page.update(&mut ui, &v, &Settings::default());
        assert_eq!(card(&ui, 1).1, "0.7 dB");
        assert_eq!(card(&ui, 2).1, "L 6");
        assert_eq!(page.layer_info(1).map(|l| l.id), Some(11));
        // And round to loops 1 to 4.
        assert_eq!(page.next_bank(&mut ui, false), LoopBank::First);
    }

    #[test]
    fn a_turn_holds_what_it_set_but_the_layers_follow_the_looper() {
        let (mut ui, mut page) = page();
        let v = two_loops();
        page.update(&mut ui, &v, &Settings::default());
        page.next_bank(&mut ui, false);
        page.next_bank(&mut ui, false);
        page.pick_layer(&mut ui, 0);
        page.qlink(&mut ui, 1, 2, false);
        let turned = 10f32.powf(1.0 / 20.0);
        // Within the hold, the looper's view: layer 11 moved to another
        // loop and an overdub added layer 13.
        let mut w = v.clone();
        w.layers = [None; MAX_LAYERS];
        for (k, id) in [10, 12, 13].into_iter().enumerate() {
            w.layers[k] = Some(layer(id));
        }
        page.update(&mut ui, &w, &Settings::default());
        let ids: Vec<u8> = (0..MAX_LAYERS)
            .filter_map(|k| page.layer_info(k).map(|l| l.id))
            .collect();
        assert_eq!(ids, [10, 12, 13], "each lane's × names the looper's layer");
        let level = page.layer_info(0).map_or(0.0, |l| l.level);
        assert!(
            (level - turned).abs() < 1e-4,
            "the turned level holds: {level}"
        );
        // Loop 2 picked: its layer 10 is its own, nothing held from loop 1.
        let mut x = w.clone();
        x.selected = 1;
        x.status = x.loops[1];
        x.layers = [None; MAX_LAYERS];
        x.layers[0] = Some(layer(10));
        page.update(&mut ui, &x, &Settings::default());
        assert_eq!(page.layer_info(0).map(|l| l.level), Some(1.0));
    }

    #[test]
    fn a_dragged_layer_goes_onto_a_loop_of_its_length_or_an_empty_one() {
        let (mut ui, mut page) = page();
        let mut v = two_loops();
        v.loops[3] = v.status;
        v.loops[3].layers = 2;
        page.update(&mut ui, &v, &Settings::default());
        let e = ui.get::<LoopEditor>(EDITOR).unwrap();
        let spot = |n: usize| e.drops.iter().find(|d| d.to == DropTo::Loop(n)).cloned();
        assert!(spot(0).is_none(), "not its own loop");
        let two = spot(1).unwrap();
        assert!(
            two.refused
                .as_deref()
                .unwrap()
                .contains("2 bars and this layer 4 bars"),
            "{two:?}"
        );
        assert_eq!(spot(2).unwrap().refused, None, "an empty loop: its own");
        assert_eq!(spot(3).unwrap().refused, None, "as long");
        let cards = ui
            .get::<LoopDeck>(DECK)
            .unwrap()
            .cards(ui.rect_of(DECK).unwrap());
        assert_eq!(spot(2).unwrap().rect, cards[2]);
        let words = page.drop_words(spot(2).as_ref(), false);
        assert_eq!(words, "Move into Loop 3: a loop of its own");
        assert_eq!(page.drop_words(spot(3).as_ref(), true), "Copy to Loop 4");
    }

    #[test]
    fn a_pan_reads_as_the_cards_print_it() {
        assert_eq!(pan_text(0.0), "C");
        assert_eq!(pan_text(-0.24), "L 12");
        assert_eq!(pan_text(0.6), "R 30");
    }

    #[test]
    fn a_loops_bars_count_by_the_meter() {
        assert_eq!(bars_of(0, 4 * 960), 0);
        assert_eq!(bars_of(4 * 960, 4 * 960), 1);
        assert_eq!(bars_of(7 * 960, 4 * 960), 2);
        // 6/8: a bar is six eighths, three quarters' worth of ticks.
        let six_eight = mpfree_transport::time::Meter::new(6, 8)
            .unwrap()
            .bar_ticks();
        assert_eq!(six_eight, 3 * 960);
        assert_eq!(
            bars_of(six_eight, six_eight),
            1,
            "not 0, as four quarters a bar made it"
        );
        assert_eq!(bars_of(2 * six_eight, six_eight), 2);
    }
}

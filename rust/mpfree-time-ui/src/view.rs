//! The time engine's screens: what they show, worked out from the engine,
//! and what their touches mean.
//!
//! - The **transport bar** sits under the header on the Play page and the
//!   Clips page: stop, play, record, replace or overdub, bar.beat.tick,
//!   tempo, meter, swing, loop, Undo, and the arp's chip on the right.
//! - The **Clips page** is the launch grid (tracks across, their 16 clip
//!   slots down), the record settings, and the takes with Undo and Redo.
//!
//! The screens do not own the engine. The audio thread copies a
//! [`TransportInfo`] after each period; the control thread builds a
//! [`TimeState`] from it, the project, the launches and the takes, and
//! calls [`TimePage::update`]. Touches come back as [`TimeAction`]s for the
//! app to apply: transport commands go to the audio thread's queue, the
//! rest are edits on the control thread ([`launches_after`] works out the
//! new launches). The header, the pads and the hint line belong to the app.

use crate::widgets::{BarButton, BarItem, CellState, ClipGrid, GridCell, GridTrack, Run, TakeItem, TakeList,
                     TransportBar, GRID_COLS};
use mpfree_play::{Launch, Take, Takes};
use mpfree_seq::{Project, Speed, SLOTS};
use mpfree_transport::time::{Meter, Swing, Tick, MAX_BPM, MIN_BPM, PPQ};
use mpfree_transport::transport::MAX_COUNT_IN;
use mpfree_transport::{Command, State, Transport};
use mpfree_ui::widgets::{Chip, OptionList, OptionRow};
use mpfree_ui::{Color, EventKind, Tone, Ui, UiEvent};

/// Widget names the page adds. The app's own are `header` and `hint`.
pub const TRANSPORT: &str = "transport";
pub const CLIPS: &str = "clips";
pub const RECORD: &str = "record";
pub const TAKES: &str = "takes";
/// The page's name, for the header tab.
pub const CLIPS_PAGE: &str = "Clips";
/// The bar in a layout spec: 44 pixels under the header.
pub const BAR_SPEC: &str = "transport:44";
/// The Clips page: the grid, and beside it the record settings over the takes.
pub const CLIPS_LAYOUT: &str = "col(header:60, transport:44, row(clips:*, col(record:*, takes:240):430):*, hint:26)";
pub const CLIPS_HINT: &str =
    "Tap a clip: it plays from the next bar, or stops   ·   + records into an empty slot   ·   a track's name arms it";
/// The record settings, top to bottom.
pub const RECORD_ROWS: [&str; 5] = ["Armed track", "Record mode", "Count-in", "Loop, bars", "Tempo, BPM"];
/// The loop lengths the Loop row steps through, in bars from bar 1 (0 is off).
pub const LOOP_BARS: [i64; 6] = [0, 1, 2, 4, 8, 16];
/// Track colours, by track number.
pub const TRACK_COLORS: [u32; 8] = [0xE8A33D, 0x3DDC97, 0x6EA8FE, 0xFF8A3D, 0xC77DFF, 0xFF6FA8, 0x2EC4B6, 0x9AA8FF];

/// `layout` with the transport bar under the header, for the app's Play
/// page and the arp's layouts: `col(header:60, ...` becomes
/// `col(header:60, transport:44, ...`. None if it does not start that way.
pub fn with_transport(layout: &str) -> Option<String> {
    const HEAD: &str = "col(header:60, ";
    let rest = layout.strip_prefix(HEAD)?;
    Some(format!("{HEAD}{BAR_SPEC}, {rest}"))
}

pub fn track_color(track: usize) -> Color {
    Color::rgb(TRACK_COLORS[track % TRACK_COLORS.len()])
}

// ---------------------------------------------------------------- state

/// The transport as the screen shows it. Plain values: the audio thread
/// copies one after each period without allocating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportInfo {
    pub state: State,
    /// The song position at the start of the next period.
    pub position: Tick,
    pub bpm: f64,
    pub meter: Meter,
    pub swing: Swing,
    pub loop_range: Option<(Tick, Tick)>,
    pub overdub: bool,
    /// Bars of count-in before recording from stopped.
    pub count_in: u8,
}

impl Default for TransportInfo {
    fn default() -> Self {
        TransportInfo { state: State::Stopped, position: 0, bpm: 120.0, meter: Meter::default(),
                        swing: Swing::default(), loop_range: None, overdub: false, count_in: 0 }
    }
}

impl TransportInfo {
    pub fn of(t: &Transport) -> TransportInfo {
        TransportInfo { state: t.state(), position: t.position(), bpm: t.bpm(), meter: t.meter(), swing: t.swing(),
                        loop_range: t.loop_range(), overdub: t.overdub(), count_in: t.count_in() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipInfo {
    pub name: String,
    /// One pass, in the clip's own ticks.
    pub length: Tick,
    pub speed: Speed,
    pub notes: usize,
}

impl ClipInfo {
    /// One pass, in song ticks.
    pub fn song_len(&self) -> f64 {
        self.speed.to_song(self.length as f64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub name: String,
    pub mute: bool,
    pub slots: Vec<Option<ClipInfo>>,
}

/// One take as the list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeRow {
    pub number: u32,
    /// The track and slot it recorded into (of its first note).
    pub at: Option<(usize, usize)>,
    pub added: usize,
    pub erased: usize,
}

impl TakeRow {
    pub fn of(t: &Take) -> TakeRow {
        let at = t.added.first().or(t.erased.first()).map(|&(track, slot, _)| (track, slot));
        TakeRow { number: t.number, at, added: t.added.len(), erased: t.erased.len() }
    }
}

/// Everything the time engine's screens show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeState {
    pub transport: TransportInfo,
    pub tracks: Vec<TrackInfo>,
    pub launches: Vec<Launch>,
    pub armed: Option<usize>,
    /// The takes that can be undone, oldest first, and the ones that can be
    /// redone, the next one last (as [`Takes`] keeps them).
    pub done: Vec<TakeRow>,
    pub undone: Vec<TakeRow>,
    /// Status chips on the right of the bar (the arp's).
    pub chips: Vec<Chip>,
}

impl TimeState {
    pub fn from_engine(transport: TransportInfo, project: &Project, launches: &[Launch], armed: Option<usize>,
                       takes: &Takes) -> TimeState {
        let tracks = project
            .tracks
            .iter()
            .map(|t| TrackInfo {
                name: t.name.clone(),
                mute: t.mute,
                slots: t
                    .slots
                    .iter()
                    .map(|s| {
                        s.as_ref().map(|c| ClipInfo { name: c.name.clone(), length: c.length, speed: c.speed,
                                                      notes: c.notes.len() })
                    })
                    .collect(),
            })
            .collect();
        TimeState { transport, tracks, launches: launches.to_vec(), armed,
                    done: takes.done().iter().map(TakeRow::of).collect(),
                    undone: takes.undone().iter().map(TakeRow::of).collect(), chips: Vec::new() }
    }

    pub fn clip(&self, track: usize, slot: usize) -> Option<&ClipInfo> {
        self.tracks.get(track)?.slots.get(slot)?.as_ref()
    }

    /// The clip launched on `track`, if any.
    pub fn launch(&self, track: usize) -> Option<&Launch> {
        self.launches.iter().find(|l| l.track == track)
    }

    fn track_name(&self, track: usize) -> String {
        self.tracks.get(track).map_or_else(|| format!("Track {}", track + 1), |t| t.name.clone())
    }
}

// ---------------------------------------------------------------- actions

/// What a touch on the time engine's screens asks for. The app applies it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeAction {
    /// For the audio thread's command queue.
    Transport(Command),
    Undo,
    Redo,
    /// Arm a track for recording, or none.
    Arm(Option<usize>),
    /// Play this clip from the next bar (see [`launches_after`]).
    Launch { track: usize, slot: usize },
    /// Stop the track's clip.
    StopClip(usize),
    /// Make a clip in this empty slot, launch it and arm its track.
    NewClip { track: usize, slot: usize },
    /// Chip `i` on the bar was tapped.
    Chip(usize),
}

/// Where a clip launched now starts: on the next bar line while the
/// transport rolls (now, if the position is on one), else at the start of
/// the bar the position is in.
pub fn launch_origin(t: &TransportInfo) -> Tick {
    if t.state.rolling() {
        t.meter.next_bar(t.position)
    } else {
        t.position - t.position.rem_euclid(t.meter.bar_ticks())
    }
}

/// The launches after `action`: Launch replaces the track's clip with one
/// starting at [`launch_origin`]; StopClip removes it. Other actions leave
/// them as they are. Until the player can queue launches (roadmap T9), the
/// old clip stops when the new view is swapped in, not on the bar.
pub fn launches_after(launches: &[Launch], action: &TimeAction, t: &TransportInfo) -> Vec<Launch> {
    let mut out = launches.to_vec();
    match *action {
        TimeAction::Launch { track, slot } | TimeAction::NewClip { track, slot } => {
            out.retain(|l| l.track != track);
            out.push(Launch { track, slot, origin: launch_origin(t) });
        }
        TimeAction::StopClip(track) => out.retain(|l| l.track != track),
        _ => {}
    }
    out
}

/// What a touch means, given what the screen shows.
pub fn action(ev: &UiEvent, st: &TimeState) -> Option<TimeAction> {
    let t = &st.transport;
    match (ev.widget.as_str(), &ev.kind) {
        (TRANSPORT, EventKind::Tab(i)) => Some(match BarButton::ALL.get(*i)? {
            BarButton::Stop => TimeAction::Transport(Command::Stop),
            BarButton::Play => TimeAction::Transport(Command::Play),
            BarButton::Rec => TimeAction::Transport(Command::Record),
            BarButton::Mode => TimeAction::Transport(Command::ToggleOverdub),
            BarButton::Undo => TimeAction::Undo,
        }),
        (TRANSPORT, EventKind::Select(i)) => Some(TimeAction::Chip(*i)),
        (CLIPS, EventKind::Tab(track)) => {
            Some(TimeAction::Arm(if st.armed == Some(*track) { None } else { Some(*track) }))
        }
        (CLIPS, EventKind::Select(i)) => {
            let (track, slot) = (i / SLOTS, i % SLOTS);
            if track >= st.tracks.len() {
                None
            } else if st.clip(track, slot).is_none() {
                Some(TimeAction::NewClip { track, slot })
            } else if st.launch(track).is_some_and(|l| l.slot == slot) {
                Some(TimeAction::StopClip(track))
            } else {
                Some(TimeAction::Launch { track, slot })
            }
        }
        (RECORD, EventKind::Step { index, delta }) => {
            let d = (*delta).signum();
            match *index {
                0 => {
                    // None, then each track in turn.
                    let n = st.tracks.len() as i64 + 1;
                    let now = st.armed.map_or(0, |a| a as i64 + 1);
                    let next = (now + i64::from(d)).rem_euclid(n);
                    Some(TimeAction::Arm(if next == 0 { None } else { Some(next as usize - 1) }))
                }
                1 => Some(TimeAction::Transport(Command::ToggleOverdub)),
                2 => {
                    let bars = (i32::from(t.count_in) + d).clamp(0, i32::from(MAX_COUNT_IN)) as u8;
                    (bars != t.count_in).then_some(TimeAction::Transport(Command::SetCountIn(bars)))
                }
                3 => {
                    let bar = t.meter.bar_ticks();
                    let now = match t.loop_range {
                        None => 0,
                        Some((a, b)) => LOOP_BARS
                            .iter()
                            .position(|&n| a == 0 && b == n * bar)
                            // A loop set elsewhere steps from the nearest length.
                            .unwrap_or_else(|| LOOP_BARS.iter().rposition(|&n| n * bar <= b - a).unwrap_or(1)),
                    };
                    let next = (now as i32 + d).clamp(0, LOOP_BARS.len() as i32 - 1) as usize;
                    let range = (next > 0).then(|| (0, LOOP_BARS[next] * bar));
                    (range != t.loop_range).then_some(TimeAction::Transport(Command::SetLoop(range)))
                }
                4 => {
                    let bpm = (t.bpm + f64::from(d)).clamp(MIN_BPM, MAX_BPM);
                    (bpm != t.bpm).then_some(TimeAction::Transport(Command::SetTempo(bpm)))
                }
                _ => None,
            }
        }
        (TAKES, EventKind::Tab(0)) => Some(TimeAction::Undo),
        (TAKES, EventKind::Tab(1)) => Some(TimeAction::Redo),
        _ => None,
    }
}

// ---------------------------------------------------------------- texts

/// A clip's length as the grid shows it: "2 bars", "6 beats", "7/16".
pub fn length_label(song_ticks: f64, meter: Meter) -> String {
    let whole = |unit: i64| {
        let n = song_ticks / unit as f64;
        (n >= 1.0 && (n - n.round()).abs() < 1e-6).then_some(n.round() as i64)
    };
    let plural = |n: i64, one: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") };
    if let Some(n) = whole(meter.bar_ticks()) {
        plural(n, "bar")
    } else if let Some(n) = whole(meter.beat_ticks()) {
        plural(n, "beat")
    } else if let Some(n) = whole(i64::from(PPQ / 4)) {
        format!("{n}/16")
    } else {
        format!("{} ticks", song_ticks.round() as i64)
    }
}

/// The loop as the bar shows it: "off", "1" (one bar), "1–4", or positions.
pub fn loop_label(range: Option<(Tick, Tick)>, meter: Meter) -> String {
    let Some((a, b)) = range else { return "off".to_string() };
    let bar = meter.bar_ticks();
    if a % bar == 0 && b % bar == 0 {
        let (first, last) = (a / bar + 1, b / bar);
        if first == last { format!("{first}") } else { format!("{first}–{last}") }
    } else {
        format!("{}–{}", meter.position(a), meter.position(b))
    }
}

fn bar_beat(tick: Tick, meter: Meter) -> String {
    let p = meter.position(tick);
    format!("{}.{}", p.bar, p.beat)
}

fn take_title(st: &TimeState, row: &TakeRow) -> String {
    match row.at {
        Some((track, slot)) => format!("Take {} · {} / {}", row.number, st.track_name(track), slot + 1),
        None => format!("Take {}", row.number),
    }
}

fn take_detail(row: &TakeRow) -> String {
    if row.erased > 0 { format!("+{} −{}", row.added, row.erased) } else { format!("+{}", row.added) }
}

// ---------------------------------------------------------------- what the widgets show

/// The bar for `st`.
pub fn bar_of(st: &TimeState) -> TransportBar {
    let t = &st.transport;
    let mut bar = TransportBar::new();
    bar.run = match t.state {
        State::Stopped => Run::Stopped,
        State::CountIn => Run::CountIn,
        State::Playing => Run::Playing,
        State::Recording => Run::Recording,
    };
    bar.position = t.meter.position(t.position).to_string();
    let swing = if t.swing.amount == 50 { "off".to_string() } else { format!("{}%", t.swing.amount) };
    bar.items = vec![BarItem::new("BPM", format!("{:.1}", t.bpm), Tone::Normal),
                     BarItem::new("", format!("{}/{}", t.meter.beats, t.meter.unit), Tone::Normal),
                     BarItem::new("Swing", swing, Tone::Normal),
                     BarItem::new("Loop", loop_label(t.loop_range, t.meter),
                                  if t.loop_range.is_some() { Tone::Accent } else { Tone::Dim })];
    if t.state == State::CountIn {
        let bars = if t.count_in == 1 { "1 bar".to_string() } else { format!("{} bars", t.count_in) };
        bar.items.insert(0, BarItem::new("Count-in", bars, Tone::Warn));
    }
    bar.overdub = t.overdub;
    bar.undo = st.done.last().map(|r| format!("take {}", r.number));
    bar.chips = st.chips.clone();
    bar
}

/// Cell `slot` of `track` for `st`.
pub fn cell_of(st: &TimeState, track: usize, slot: usize) -> GridCell {
    let Some(clip) = st.clip(track, slot) else { return GridCell::default() };
    let t = &st.transport;
    let len = clip.song_len().max(1.0);
    let notes = if clip.notes == 1 { "1 note".to_string() } else { format!("{} notes", clip.notes) };
    let mut cell = GridCell { state: CellState::Stopped, name: clip.name.clone(), detail: length_label(len, t.meter),
                              foot: notes, progress: 0.0 };
    let Some(l) = st.launch(track).filter(|l| l.slot == slot) else { return cell };
    if l.origin > t.position {
        cell.state = CellState::Queued;
        cell.foot = format!("from {}", bar_beat(l.origin, t.meter));
        return cell;
    }
    // In steps of 1/64 of a pass, so the ring is not redrawn every frame.
    let frac = ((t.position - l.origin) as f64).rem_euclid(len) / len;
    cell.progress = ((frac * 64.0).floor() / 64.0) as f32;
    // The ring takes the foot's place.
    cell.foot.clear();
    if t.state == State::Recording && st.armed == Some(track) {
        cell.state = CellState::Recording;
        cell.detail = "recording".to_string();
    } else {
        cell.state = CellState::Playing;
    }
    cell
}

/// The grid's tracks for `st`.
pub fn grid_tracks(st: &TimeState) -> Vec<GridTrack> {
    st.tracks
        .iter()
        .enumerate()
        .map(|(i, tr)| GridTrack {
            name: tr.name.clone(),
            color: track_color(i),
            armed: st.armed == Some(i),
            mute: tr.mute,
            cells: (0..tr.slots.len()).map(|s| cell_of(st, i, s)).collect(),
        })
        .collect()
}

/// The record settings' rows.
pub fn record_rows(st: &TimeState) -> Vec<OptionRow> {
    let t = &st.transport;
    let row = |name: &str, value: String| OptionRow { name: name.to_string(), value };
    let armed = st.armed.map_or_else(|| "none".to_string(), |a| st.track_name(a));
    let count_in = match t.count_in {
        0 => "off".to_string(),
        1 => "1 bar".to_string(),
        n => format!("{n} bars"),
    };
    vec![row(RECORD_ROWS[0], armed),
         row(RECORD_ROWS[1], if t.overdub { "Overdub" } else { "Replace" }.to_string()),
         row(RECORD_ROWS[2], count_in),
         row(RECORD_ROWS[3], loop_label(t.loop_range, t.meter)),
         row(RECORD_ROWS[4], format!("{:.1}", t.bpm))]
}

/// The takes newest first: the undone ones (dimmed), then the ones Undo
/// can take back; and which row Undo takes back next.
pub fn take_items(st: &TimeState) -> (Vec<TakeItem>, Option<usize>) {
    let item = |r: &TakeRow, undone: bool| TakeItem { title: take_title(st, r), detail: take_detail(r),
                                                       color: r.at.map(|(track, _)| track_color(track)), undone };
    let mut items: Vec<TakeItem> = st.undone.iter().map(|r| item(r, true)).collect();
    items.extend(st.done.iter().rev().map(|r| item(r, false)));
    let current = (!st.done.is_empty()).then_some(st.undone.len());
    (items, current)
}

// ---------------------------------------------------------------- the page

/// The Clips page and the transport bar in the app's [`Ui`]: the app adds
/// them once, calls [`TimePage::update`] when the state changes, and turns
/// touches into actions with [`action`].
#[derive(Clone, Debug, Default)]
pub struct TimePage {
    /// The first slot shown (the wheel scrolls it).
    pub first_row: usize,
    /// The first track shown.
    pub first_track: usize,
}

impl TimePage {
    /// Add the bar, the grid, the record settings, the takes and the Clips
    /// page. The app's `header` and `hint` must be in `ui` already.
    pub fn add_to(ui: &mut Ui) -> Result<TimePage, String> {
        ui.add(TRANSPORT, TransportBar::new());
        ui.add(CLIPS, ClipGrid::new("Clips", SLOTS));
        ui.add(RECORD, OptionList::new("Record"));
        ui.add(TAKES, TakeList::new("Takes"));
        ui.add_page(CLIPS_PAGE, CLIPS_LAYOUT)?;
        Ok(TimePage::default())
    }

    /// Rows of the grid shown at once (0 if the grid is not on screen).
    pub fn visible_rows(&self, ui: &Ui) -> usize {
        match (ui.rect_of(CLIPS), ui.get::<ClipGrid>(CLIPS)) {
            (Some(r), Some(g)) => g.visible_rows(r),
            _ => 0,
        }
    }

    /// Scroll the grid `delta` rows down (up if negative).
    pub fn scroll(&mut self, ui: &Ui, delta: i32) {
        let rows = self.visible_rows(ui).max(1);
        let last = SLOTS.saturating_sub(rows) as i64;
        self.first_row = (self.first_row as i64 + i64::from(delta)).clamp(0, last) as usize;
    }

    /// Show `st`. Widgets whose contents did not change are left alone, so
    /// they are not redrawn.
    pub fn update(&self, ui: &mut Ui, st: &TimeState) {
        let bar = bar_of(st);
        let same_bar = ui.get::<TransportBar>(TRANSPORT).is_some_and(|b| {
            b.run == bar.run && b.position == bar.position && b.items == bar.items && b.overdub == bar.overdub
                && b.undo == bar.undo && b.chips == bar.chips
        });
        if !same_bar {
            if let Some(b) = ui.get_mut::<TransportBar>(TRANSPORT) {
                b.run = bar.run;
                b.position = bar.position;
                b.items = bar.items;
                b.overdub = bar.overdub;
                b.undo = bar.undo;
                b.chips = bar.chips;
            }
        }
        let tracks = grid_tracks(st);
        let rows = self.visible_rows(ui).max(1);
        let title = format!("Clips · slots {}–{} of {SLOTS}", self.first_row + 1, (self.first_row + rows).min(SLOTS));
        let legend = if st.tracks.len() > GRID_COLS {
            format!("tracks {}–{} of {}", self.first_track + 1, (self.first_track + GRID_COLS).min(st.tracks.len()),
                    st.tracks.len())
        } else {
            "ring: one pass   ·   amber: next bar   ·   red: recording".to_string()
        };
        let same_grid = ui.get::<ClipGrid>(CLIPS).is_some_and(|g| {
            g.tracks == tracks && g.title == title && g.legend == legend && g.first_row == self.first_row
                && g.first_track == self.first_track
        });
        if !same_grid {
            if let Some(g) = ui.get_mut::<ClipGrid>(CLIPS) {
                g.tracks = tracks;
                g.title = title;
                g.legend = legend;
                g.first_row = self.first_row;
                g.first_track = self.first_track;
            }
        }
        let rows = record_rows(st);
        if ui.get::<OptionList>(RECORD).is_some_and(|l| l.rows != rows) {
            if let Some(l) = ui.get_mut::<OptionList>(RECORD) {
                l.rows = rows;
                l.right = "wheel or arrows".to_string();
            }
        }
        let (items, current) = take_items(st);
        let undo = st.done.last().map(|r| format!("take {}", r.number));
        let redo = st.undone.last().map(|r| format!("take {}", r.number));
        let same_takes = ui.get::<TakeList>(TAKES).is_some_and(|l| {
            l.items == items && l.current == current && l.undo == undo && l.redo == redo
        });
        if !same_takes {
            if let Some(l) = ui.get_mut::<TakeList>(TAKES) {
                l.items = items;
                l.current = current;
                l.undo = undo;
                l.redo = redo;
                l.right = "SHIFT + UNDO: redo".to_string();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Note, Track};
    use mpfree_ui::widgets::{Header, Label};
    use mpfree_ui::{Align, Fonts, Theme};

    const BAR: Tick = 3840;

    fn state() -> TimeState {
        let mut p = Project::new();
        for (i, name) in ["Drums", "Bass", "Stab"].iter().enumerate() {
            let mut t = Track::new(name, i as u16);
            let mut c = Clip::new("A", 2 * BAR);
            c.add_note(Note { tick: 0, len: 10, note: 1, velocity: 100 }).unwrap();
            t.slots[0] = Some(c);
            t.slots[1] = Some(Clip::new("B", 1680));
            p.tracks.push(t);
        }
        let launches = [Launch { track: 0, slot: 0, origin: 0 }, Launch { track: 1, slot: 1, origin: 5 * BAR }];
        let t = TransportInfo { state: State::Playing, position: 4 * BAR + 960, ..TransportInfo::default() };
        TimeState::from_engine(t, &p, &launches, Some(2), &Takes::default())
    }

    fn ev(widget: &str, kind: EventKind) -> UiEvent {
        UiEvent { widget: widget.to_string(), kind }
    }

    #[test]
    fn the_bar_goes_under_the_header() {
        let play = "col(header:60, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)";
        assert_eq!(with_transport(play).unwrap(),
                   "col(header:60, transport:44, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)");
        assert_eq!(with_transport("col(log:*)"), None);
    }

    #[test]
    fn the_clips_page_fits_the_screen() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        assert_eq!(ui.rect_of(TRANSPORT).map(|r| (r.y, r.h)), Some((84, 44)));
        assert!(page.visible_rows(&ui) >= 6, "{}", page.visible_rows(&ui));
        // Every record setting is in view at once (58 px a row).
        let record = ui.rect_of(RECORD).unwrap();
        assert!(record.h - 14 * 2 - 34 >= 58 * RECORD_ROWS.len() as i32, "{record:?}");
        // Three takes and the buttons.
        let takes = ui.rect_of(TAKES).unwrap();
        assert!(takes.h - 14 * 2 - 34 - 48 - 8 >= 3 * 38, "{takes:?}");
    }

    #[test]
    fn cells_show_what_each_clip_is_doing() {
        let mut st = state();
        let c = cell_of(&st, 0, 0);
        assert_eq!((c.state, c.detail.as_str(), c.foot.as_str()), (CellState::Playing, "2 bars", ""));
        assert_eq!(cell_of(&st, 1, 0).foot, "1 note");
        // Bar 5 beat 2 of a 2-bar clip from bar 1: an eighth of the way round.
        assert_eq!(c.progress, 0.125);
        let q = cell_of(&st, 1, 1);
        assert_eq!((q.state, q.detail.as_str(), q.foot.as_str()), (CellState::Queued, "7/16", "from 6.1"));
        assert_eq!(cell_of(&st, 1, 0).state, CellState::Stopped);
        assert_eq!(cell_of(&st, 0, 2).state, CellState::Empty);
        st.launches.push(Launch { track: 2, slot: 0, origin: 0 });
        assert_eq!(cell_of(&st, 2, 0).state, CellState::Playing);
        st.transport.state = State::Recording;
        let r = cell_of(&st, 2, 0);
        assert_eq!((r.state, r.detail.as_str()), (CellState::Recording, "recording"));
        assert_eq!(cell_of(&st, 0, 0).state, CellState::Playing, "only the armed track records");
    }

    #[test]
    fn taps_on_the_grid_launch_stop_make_and_arm() {
        let st = state();
        let tap = |track: usize, slot: usize| action(&ev(CLIPS, EventKind::Select(track * SLOTS + slot)), &st);
        assert_eq!(tap(0, 0), Some(TimeAction::StopClip(0)));
        assert_eq!(tap(0, 1), Some(TimeAction::Launch { track: 0, slot: 1 }));
        assert_eq!(tap(1, 1), Some(TimeAction::StopClip(1)), "a queued clip stops too");
        assert_eq!(tap(2, 5), Some(TimeAction::NewClip { track: 2, slot: 5 }));
        assert_eq!(tap(3, 0), None, "no such track");
        assert_eq!(action(&ev(CLIPS, EventKind::Tab(0)), &st), Some(TimeAction::Arm(Some(0))));
        assert_eq!(action(&ev(CLIPS, EventKind::Tab(2)), &st), Some(TimeAction::Arm(None)), "arming again disarms");
    }

    #[test]
    fn the_bar_and_takes_buttons_map_to_commands() {
        let st = state();
        let bar = |b: BarButton| action(&ev(TRANSPORT, EventKind::Tab(b.index())), &st);
        assert_eq!(bar(BarButton::Stop), Some(TimeAction::Transport(Command::Stop)));
        assert_eq!(bar(BarButton::Play), Some(TimeAction::Transport(Command::Play)));
        assert_eq!(bar(BarButton::Rec), Some(TimeAction::Transport(Command::Record)));
        assert_eq!(bar(BarButton::Mode), Some(TimeAction::Transport(Command::ToggleOverdub)));
        assert_eq!(bar(BarButton::Undo), Some(TimeAction::Undo));
        assert_eq!(action(&ev(TRANSPORT, EventKind::Select(0)), &st), Some(TimeAction::Chip(0)));
        assert_eq!(action(&ev(TAKES, EventKind::Tab(0)), &st), Some(TimeAction::Undo));
        assert_eq!(action(&ev(TAKES, EventKind::Tab(1)), &st), Some(TimeAction::Redo));
    }

    #[test]
    fn record_settings_step_within_their_range() {
        let mut st = state();
        let step = |st: &TimeState, index: usize, delta: i32| action(&ev(RECORD, EventKind::Step { index, delta }), st);
        // Armed: Stab (2) -> none -> Drums (0), and back.
        assert_eq!(step(&st, 0, 1), Some(TimeAction::Arm(None)));
        st.armed = None;
        assert_eq!(step(&st, 0, 1), Some(TimeAction::Arm(Some(0))));
        assert_eq!(step(&st, 0, -1), Some(TimeAction::Arm(Some(2))));
        assert_eq!(step(&st, 1, -1), Some(TimeAction::Transport(Command::ToggleOverdub)));
        assert_eq!(step(&st, 2, -1), None, "count-in is already off");
        assert_eq!(step(&st, 2, 1), Some(TimeAction::Transport(Command::SetCountIn(1))));
        st.transport.count_in = MAX_COUNT_IN;
        assert_eq!(step(&st, 2, 1), None);
        assert_eq!(step(&st, 3, 1), Some(TimeAction::Transport(Command::SetLoop(Some((0, BAR))))));
        st.transport.loop_range = Some((0, 4 * BAR));
        assert_eq!(step(&st, 3, 1), Some(TimeAction::Transport(Command::SetLoop(Some((0, 8 * BAR))))));
        assert_eq!(step(&st, 3, -1), Some(TimeAction::Transport(Command::SetLoop(Some((0, 2 * BAR))))));
        // A loop set elsewhere (bars 3 to 5) steps from the nearest length.
        st.transport.loop_range = Some((2 * BAR, 5 * BAR));
        assert_eq!(step(&st, 3, 1), Some(TimeAction::Transport(Command::SetLoop(Some((0, 4 * BAR))))));
        assert_eq!(step(&st, 4, 1), Some(TimeAction::Transport(Command::SetTempo(121.0))));
        st.transport.bpm = MAX_BPM;
        assert_eq!(step(&st, 4, 1), None);
    }

    #[test]
    fn a_launch_starts_on_the_next_bar_and_replaces_the_tracks_clip() {
        let st = state();
        let t = st.transport;
        let after = launches_after(&st.launches, &TimeAction::Launch { track: 0, slot: 1 }, &t);
        assert_eq!(after, vec![Launch { track: 1, slot: 1, origin: 5 * BAR }, Launch { track: 0, slot: 1, origin: 5 * BAR }]);
        let stopped = launches_after(&after, &TimeAction::StopClip(1), &t);
        assert_eq!(stopped, vec![Launch { track: 0, slot: 1, origin: 5 * BAR }]);
        // On a bar line: now. Stopped: from the start of the bar.
        let on_bar = TransportInfo { position: 3 * BAR, ..t };
        assert_eq!(launch_origin(&on_bar), 3 * BAR);
        let stopped = TransportInfo { state: State::Stopped, position: 3 * BAR + 100, ..t };
        assert_eq!(launch_origin(&stopped), 3 * BAR);
        // Counting in from bar 0: bar 1.
        let count = TransportInfo { state: State::CountIn, position: -1000, ..t };
        assert_eq!(launch_origin(&count), 0);
        assert_eq!(launches_after(&st.launches, &TimeAction::Undo, &t), st.launches);
    }

    #[test]
    fn takes_list_newest_first_with_the_undone_dimmed() {
        let mut st = state();
        let row = |n: u32| TakeRow { number: n, at: Some((2, 0)), added: n as usize, erased: 0 };
        st.done = vec![row(1), row(2)];
        st.undone = vec![row(4), row(3)];
        let (items, current) = take_items(&st);
        let titles: Vec<(&str, bool)> = items.iter().map(|i| (i.title.as_str(), i.undone)).collect();
        assert_eq!(titles, vec![("Take 4 · Stab / 1", true), ("Take 3 · Stab / 1", true),
                                ("Take 2 · Stab / 1", false), ("Take 1 · Stab / 1", false)]);
        assert_eq!(current, Some(2));
        assert_eq!(items[2].detail, "+2");
        st.done[1].erased = 5;
        assert_eq!(take_items(&st).0[2].detail, "+2 −5");
        st.done.clear();
        assert_eq!(take_items(&st).1, None);
    }

    #[test]
    fn labels() {
        let m = Meter::default();
        assert_eq!(length_label(3840.0, m), "1 bar");
        assert_eq!(length_label(7680.0, m), "2 bars");
        assert_eq!(length_label(1920.0, m), "2 beats");
        assert_eq!(length_label(5760.0, m), "6 beats");
        assert_eq!(length_label(1680.0, m), "7/16");
        assert_eq!(length_label(100.0, m), "100 ticks");
        assert_eq!(loop_label(None, m), "off");
        assert_eq!(loop_label(Some((0, 3840)), m), "1");
        assert_eq!(loop_label(Some((0, 4 * 3840)), m), "1–4");
        assert_eq!(loop_label(Some((960, 3840)), m), "1.2.000–2.1.000");
    }

    #[test]
    fn update_redraws_only_what_changed() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        let mut st = state();
        page.update(&mut ui, &st);
        ui.paint(&mut c);
        page.update(&mut ui, &st);
        assert!(!ui.needs_paint(), "nothing changed");
        st.transport.position += 1;
        page.update(&mut ui, &st);
        let drawn = ui.paint(&mut c);
        assert_eq!(drawn, vec![ui.rect_of(TRANSPORT).unwrap()], "one tick on: only the bar");
    }
}

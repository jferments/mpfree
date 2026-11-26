//! What the Looper tab shows: the looper's view, the loop point view, smart
//! loop end's and Make song's buttons, and Move start / Move end.

use super::*;

/// What the looper heard over the last [`HEARD_BARS`] bars (or more), for
/// an empty loop's What you played: its level, the bar lines in it, and
/// what Capture into the picked loop would take now (its window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeardWave {
    /// Each column's peak, oldest first, dBFS (-inf silent).
    pub cols: [f32; HEARD_COLS],
    /// The bar lines in it, oldest first: each bar's number (from 1) and
    /// where it starts, 0 to 1; `n_lines` of them.
    pub lines: [(i64, f32); HEARD_MAX_BARS as usize + 1],
    pub n_lines: usize,
    /// The bars the window grows and shrinks by: the pattern's length
    /// while Length is Pattern and a pattern plays (`pattern` true), else
    /// the Length's bars, else 1.
    pub unit: u32,
    pub pattern: bool,
    /// What Capture takes now: where it starts and ends (0 to 1), its
    /// first bar's number and how many bars.
    pub take: Option<(f32, f32, i64, u32)>,
    /// Capture now waits for the bar line coming (the song is in a bar's
    /// last beat): it takes the bars up to that line.
    pub waits: bool,
}

impl Default for HeardWave {
    fn default() -> Self {
        HeardWave {
            cols: [f32::NEG_INFINITY; HEARD_COLS],
            lines: [(0, 0.0); HEARD_MAX_BARS as usize + 1],
            n_lines: 0,
            unit: 1,
            pattern: false,
            take: None,
            waits: false,
        }
    }
}

/// The loop point view's windows: how many slices each draws.
pub const POINT_BARS: usize = 120;

/// Each window's length, ms: the start's from 100 ms before your press,
/// the end's from 200 ms before your tap ([`POINT_START_FROM_MS`],
/// [`POINT_END_FROM_MS`]), so what smart loop end looks at (a tenth of a
/// second each way) is in view.
pub const POINT_MS: f64 = 300.0;

pub const POINT_START_FROM_MS: f64 = -100.0;

pub const POINT_END_FROM_MS: f64 = -200.0;

/// The audio round a loop's start and end as smart loop end left it, for
/// the loop point view: each window's peaks, 0 to 255 (full scale), in
/// [`POINT_BARS`] slices. Plain data, so a view holds it without
/// allocating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopPoint {
    pub start: [u8; POINT_BARS],
    pub end: [u8; POINT_BARS],
    /// Where each window begins, in frames from your press (the start's)
    /// and your tap (the end's): [`POINT_START_FROM_MS`] and
    /// [`POINT_END_FROM_MS`], unless the loop's edge, or a move, is out of
    /// that view, when the window is round it.
    pub from: [i64; 2],
    /// The hits heard in each window, in frames from your press and your
    /// tap, earliest first; `n_hits` of each are used.
    pub hits: [[i64; POINT_HITS]; 2],
    pub n_hits: [u8; 2],
}

/// The most hits a loop point window shows.
pub const POINT_HITS: usize = 16;

impl LoopPoint {
    /// Windows where they always are ([`POINT_START_FROM_MS`],
    /// [`POINT_END_FROM_MS`]) at `rate`, with these peaks and no hits.
    pub fn new(start: [u8; POINT_BARS], end: [u8; POINT_BARS], rate: u32) -> LoopPoint {
        let ms = |m: f64| (m * f64::from(rate) / 1000.0).round() as i64;
        LoopPoint {
            start,
            end,
            from: [ms(POINT_START_FROM_MS), ms(POINT_END_FROM_MS)],
            hits: [[0; POINT_HITS]; 2],
            n_hits: [0; 2],
        }
    }
}

/// Smart loop end on the picked loop, besides what its [`Status`] says
/// (the offer and where the loop is from your press and tap).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SmartView {
    /// The reading was a close call: Read as the other bars, while the
    /// loop is as it was read.
    pub close_call: Option<u32>,
    /// Keep my tap or Keep the snap: the chip is put away until the loop
    /// changes.
    pub kept: bool,
    /// The snap waiting on "... anyway?" ([`LoopHost::snap_asking`]),
    /// and whether it is an Undo or a Redo of Read as ([`ask_words`]).
    pub asking: Option<(Snap, bool)>,
    /// The picked loop's first pass waits on a Length it couldn't keep
    /// ([`Status::asks`]): how it read. The hint line asks
    /// ([`length_ask_words`]), Read as and Discard answer.
    pub length: Option<Reading>,
    /// A/B on the seam runs on the picked loop ([`LoopHost::set_ab`]):
    /// each time round its last beat runs into the downbeat as the other
    /// side has it, then as it is ([`mpfree_looper::Status::ab`] says
    /// which comes next).
    pub ab: bool,
    /// The loop point view's audio, while the view is open.
    pub point: Option<LoopPoint>,
    /// A move set up in the loop point view (Move start / Move end).
    pub moving: Option<Moving>,
    /// The step Move start / Move end goes by.
    pub move_step: MoveStep,
}

/// Make song's button on a free loop's header ([`LoopHost::make_song`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SongButton {
    /// A tap makes a song of the free loops.
    #[default]
    Ready,
    /// The song plays or counts in: faint, and a tap says why
    /// ([`MAKE_SONG_STOP_FIRST`]).
    Faint,
    /// Make song waits for the free loops' bar line: lit.
    Waits,
}

/// Make song's faint button, tapped while the song plays: why, on the
/// hint line.
pub const MAKE_SONG_STOP_FIRST: &str =
    "Stop the song first: Make song moves its tempo and bar lines";

/// A side of smart loop end's choice: the loop as you closed it (your
/// press and tap), or as it found (the snap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Tap,
    Snap,
}

/// Move start / Move end: which of the loop's edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

/// How far one step of Move start / Move end goes ([`MoveAct::Step`]): a
/// sixteenth note at the loop's tempo, a millisecond, or to the next hit heard
/// in the window. Kept for the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MoveStep {
    #[default]
    Sixteenth,
    Ms,
    Hit,
}

impl MoveStep {
    pub const ALL: [MoveStep; 3] = [MoveStep::Sixteenth, MoveStep::Ms, MoveStep::Hit];

    /// As the loop point view names it.
    pub fn label(self) -> &'static str {
        match self {
            MoveStep::Sixteenth => "1/16",
            MoveStep::Ms => "1 ms",
            MoveStep::Hit => "Hit",
        }
    }
}

/// The loop point view's Move start / Move end ([`LoopHost::move_act`]):
/// a move is set up first (drawn, and heard with A/B), then kept with
/// Move, which lands it as a snap does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveAct {
    /// Put an edge here, in frames from your press (the start) or your
    /// tap (the end): a finger on the window, or a hit tapped.
    To(Edge, i64),
    /// One step earlier (-1) or later (+1), by [`MoveStep`].
    Step(Edge, i8),
    /// The step's size.
    Scale(MoveStep),
    /// Move: the loop goes there on its next downbeat, with the tempo.
    Keep,
    /// The move put away; the loop stays as it is.
    Cancel,
}

/// A move set up in the loop point view, not yet kept
/// ([`SmartView::moving`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moving {
    /// Where the start and the end would go, from your press and tap.
    pub to: Shift,
    /// The tempo it would make (its bars kept), or why it can't be made.
    pub makes: Result<f64, &'static str>,
}

/// What the Looper tab shows (sent with the timing, and on changes).
#[derive(Clone, Debug, PartialEq)]
pub struct LooperView {
    /// Smart loop end on the picked loop, for its chip and the loop point
    /// view.
    pub smart: SmartView,
    pub status: Status,
    pub stats: Stats,
    /// Records from as picked: what the next take records.
    pub source: LoopSource,
    /// What the take recording now records from, while it holds another
    /// source than the one picked ([`LoopHost::settle`]).
    pub taking_from: Option<LoopSource>,
    /// Records from the other loops: Record on an empty loop resamples
    /// ([`LoopHost::resample`]).
    pub resample: bool,
    pub line_on: bool,
    pub line: LineStats,
    pub monitor: bool,
    /// The recorded source's peaks since the last view, dBFS (-inf when
    /// silent).
    pub meter_db: [f32; 2],
    /// The loop's place in the song: beats per bar, the frames of one of
    /// those beats (a quarter in 4/4, an eighth in 6/8) at the tempo it
    /// was recorded at, for the bar lines, and the bar's length in ticks
    /// ([`Meter::bar_ticks`](mpfree_transport::time::Meter::bar_ticks)),
    /// for counting a loop's bars.
    pub beats_per_bar: u32,
    pub frames_per_beat: f64,
    pub bar_ticks: Tick,
    /// The loop's layers in play order (the first pass first) with their
    /// mix, for the speakers; None past the last.
    pub layers: [Option<LayerInfo>; MAX_LAYERS],
    /// Every loop, for the Looper tab's list; `status` is `loops[selected]`.
    pub loops: [Status; MAX_LOOPS],
    pub selected: usize,
    /// The memory all the loops share (frames).
    pub memory: Memory,
    /// Frames per second, for the lengths in seconds.
    pub rate: u32,
    /// The picked loop's undo history.
    pub history: History,
    /// Each loop's small wave, for its cells on the Clips grid.
    pub cell_waves: [CellWave; MAX_LOOPS],
    /// Each loop's wave round its ring on the Looper tab's deck.
    pub ring_waves: [RingWave; MAX_LOOPS],
    /// The loop a whole loop copies into ([`LoopHost::copy_loop`]), until
    /// every layer is in.
    pub copy_into: Option<usize>,
    /// What the looper heard over the last bars (What you played).
    pub heard: HeardWave,
    /// Where Capture goes now: the picked loop if it is empty, else the
    /// first empty one (None: none is), and the bars it asks for.
    pub capture_into: Option<(usize, u32)>,
    /// Tempo from the first loop is on, and the song can take one now
    /// ([`LoopHost::set_tempo_from_loop`]).
    pub free_tempo: bool,
    /// Length: Pattern is picked ([`PATTERN_BARS`]): a new loop takes the
    /// length of the pattern playing.
    pub match_pattern: bool,
    /// Listen plays: the loop, its layer (None: the whole loop) and
    /// where it has got to, 0 to 1 ([`LoopHost::listen`]).
    pub listening: Option<(usize, Option<u8>, f32)>,
    /// The pattern's length in bars as last told
    /// ([`LoopHost::set_pattern_bars`]); None: no pattern plays.
    pub pattern_bars: Option<u32>,
    /// Sync as last set ([`LoopHost::clock`]): the clock every empty loop
    /// records on.
    pub clock: Clock,
    /// The free clock while a free loop holds audio
    /// ([`Looper::free_clock`]), where it is now ([`Looper::free_now`]),
    /// and whether a Play for the stopped free loops waits for the song's
    /// bar line ([`Looper::free_cued`]).
    pub free: Option<FreeClock>,
    pub free_now: f64,
    pub free_cued: bool,
    /// Make song's button on a free loop's header
    /// ([`LoopHost::make_song`]).
    pub song: SongButton,
    /// Loops on audio tracks: each slot's loop's lasting id (0: none
    /// yet), so the grid's loop cells find their loops
    /// ([`LoopHost::loop_ids`]).
    pub ids: [u128; MAX_LOOPS],
    /// Each loop holds a layer not yet on the stick (audio tracks phase
    /// 4c.3, with recordings on: [`LoopHost::unsaved_layers`]): its ring's
    /// dot.
    pub unsaved: [bool; MAX_LOOPS],
}

/// A free clock as a transport span: playing, straight (no swing), on its
/// anchor, tempo and meter (the looper copies a song span into a free
/// clock the other way, `FreeClock::of_song`), so whatever follows a span
/// keeps the free loops' time. Its offset, length and frame are each
/// period's to set.
pub fn lead_span(c: &FreeClock) -> Span {
    Span {
        offset: 0,
        len: 0,
        frame: c.anchor_frame,
        state: State::Playing,
        overdub: false,
        anchor_frame: c.anchor_frame,
        anchor_tick: c.anchor_tick,
        bpm: c.bpm,
        rate: c.rate,
        swing: Swing::default(),
        meter: c.meter,
    }
}

impl LooperView {
    /// The free loops play: the free clock is set and runs.
    pub fn free_runs(&self) -> bool {
        self.free.is_some_and(|c| c.running)
    }

    /// Loop `n` is a free loop with sound, stopped with the free clock: it
    /// waits at its place on bar 1, and PLAY, Play or its ring starts it.
    /// Not while muted: its ring unmutes it first, as any loop's.
    pub fn free_stopped(&self, n: usize) -> bool {
        self.loops.get(n).is_some_and(|l| {
            l.clock == Clock::Free
                && l.phase == Phase::Playing
                && !l.muted
                && self.free.is_some_and(|c| !c.running)
        })
    }

    /// A free loop's bar in ticks (the free clock's meter), else the
    /// song's.
    pub fn bar_ticks_of(&self, s: &Status) -> Tick {
        match (s.clock, self.free) {
            (Clock::Free, Some(c)) => c.meter.bar_ticks(),
            _ => self.bar_ticks,
        }
    }
}

impl Default for LooperView {
    fn default() -> Self {
        let l = Looper::new(44_100, 0);
        LooperView {
            smart: SmartView::default(),
            status: l.status(),
            stats: l.stats(),
            source: LoopSource::Pads,
            taking_from: None,
            resample: false,
            line_on: false,
            line: LineStats::default(),
            monitor: false,
            meter_db: [f32::NEG_INFINITY; 2],
            beats_per_bar: 4,
            frames_per_beat: 0.0,
            bar_ticks: 4 * PPQ as Tick,
            layers: [None; MAX_LAYERS],
            loops: [l.status(); MAX_LOOPS],
            selected: 0,
            memory: l.memory(),
            rate: 44_100,
            history: l.history(),
            cell_waves: [[0; CELL_WAVE]; MAX_LOOPS],
            ring_waves: [[0; RING_WAVE]; MAX_LOOPS],
            copy_into: None,
            heard: HeardWave::default(),
            capture_into: None,
            free_tempo: false,
            match_pattern: false,
            pattern_bars: None,
            ids: [0; MAX_LOOPS],
            listening: None,
            unsaved: [false; MAX_LOOPS],
            clock: Clock::Song,
            free: None,
            free_now: 0.0,
            free_cued: false,
            song: SongButton::Ready,
        }
    }
}

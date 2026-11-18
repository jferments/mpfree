//! Musical time: ticks, the tempo map, the meter, bar/beat positions and
//! swing. Plain values with no state, shared by the transport and by every
//! engine that turns ticks into frames.

/// Ticks per quarter note, the same resolution as `mpfree-arp`.
pub const PPQ: u32 = 960;

/// Slowest and fastest tempo, the same range as `mpfree-core::ClickEngine`.
pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 400.0;

/// Tempo segments in one map.
pub const MAX_SEGMENTS: usize = 32;

/// The shortest tempo segment and the shortest loop: one 1/16. This bounds
/// how many boundaries can fall inside one audio period.
pub const MIN_SPAN_TICKS: i64 = (PPQ / 4) as i64;

/// A song position in ticks. Negative during a count-in before bar 1.
pub type Tick = i64;

/// Frames from tick `a` to tick `b` at a constant tempo. Computed from the
/// two ticks directly, never by adding up periods, so the answer is the same
/// however the audio is cut into periods.
pub fn frames_between(a: Tick, b: Tick, bpm: f64, rate: u32) -> f64 {
    (b - a) as f64 * 60.0 * f64::from(rate) / (bpm * f64::from(PPQ))
}

/// The inverse of [`frames_between`]: ticks covered by `frames` frames.
pub fn ticks_in(frames: f64, bpm: f64, rate: u32) -> f64 {
    frames * bpm * f64::from(PPQ) / (60.0 * f64::from(rate))
}

pub fn valid_bpm(bpm: f64) -> bool {
    bpm.is_finite() && (MIN_BPM..=MAX_BPM).contains(&bpm)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// First tick of this tempo. The first segment's start is ignored: its
    /// tempo also covers everything before it, including a count-in.
    pub start: Tick,
    pub bpm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    Empty,
    TooManySegments,
    BadTempo,
    /// Segments must start in order, at least [`MIN_SPAN_TICKS`] apart.
    BadOrder,
    /// Only a segment with another after it can ramp: the ramp ends on the
    /// next segment's start, at its tempo.
    RampAtEnd,
}

/// A ramp changes tempo on every line of its own grid, counted from the
/// ramp's start: a staircase of 1/16 steps that follows the glide. The
/// last step takes any remainder, so no step is shorter than a 1/16.
pub const RAMP_STEP: Tick = MIN_SPAN_TICKS;

/// Tempo over the song: steps from one tempo to the next at chosen ticks,
/// and ramps that glide from one segment's tempo to the next one's.
/// Fixed capacity, `Copy`, no allocation.
///
/// A ramp glides linearly in ticks (halfway through it, the tempo is
/// halfway). The transport plays it as a staircase: on each line of the
/// ramp's 1/16 grid it takes a new constant tempo, and each line falls on
/// the whole frame nearest to where the true glide puts it, counted from
/// the ramp's start. Between lines frames follow ticks in a straight line.
/// [`TempoMap::frames_between`] and [`TempoMap::tick_after`] follow that
/// same staircase, so anything that turns ticks into frames (audio tracks,
/// the screen) agrees with the transport to the frame, whatever the period
/// size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoMap {
    segments: [Segment; MAX_SEGMENTS],
    len: usize,
    /// Bit `i`: segment `i` ramps to segment `i + 1`'s tempo.
    ramp_bits: u32,
}

/// Frames per tick at 1 BPM: frames for `n` ticks at `bpm` are
/// `n * k / bpm`.
fn frames_k(rate: u32) -> f64 {
    60.0 * f64::from(rate) / f64::from(PPQ)
}

/// One ramp, from tick `t0` at `b0` to tick `t1` at `b1`.
#[derive(Clone, Copy, Debug)]
struct Glide {
    t0: Tick,
    t1: Tick,
    b0: f64,
    b1: f64,
}

impl Glide {
    /// Steps in the ramp, at least one.
    fn steps(self) -> i64 {
        ((self.t1 - self.t0) / RAMP_STEP).max(1)
    }

    /// Line `j` of the ramp (0 is its start, `steps()` its end).
    fn line(self, j: i64) -> Tick {
        if j >= self.steps() {
            self.t1
        } else {
            self.t0 + j * RAMP_STEP
        }
    }

    /// The step a (fractional) tick inside the ramp falls in.
    fn step_of(self, x: f64) -> i64 {
        (((x - self.t0 as f64) / RAMP_STEP as f64).floor() as i64).clamp(0, self.steps() - 1)
    }

    /// The true glide's tempo at `x`.
    fn bpm_at(self, x: f64) -> f64 {
        self.b0 + (self.b1 - self.b0) * (x - self.t0 as f64) / (self.t1 - self.t0) as f64
    }

    /// Frames from `t0` to `x` along the true glide: the integral of
    /// `k / bpm(t)`, which is `k L / (b1 - b0) ln(bpm(x) / b0)`, written so
    /// it stays exact as `b1` nears `b0`.
    fn exact(self, x: f64, k: f64) -> f64 {
        let dx = x - self.t0 as f64;
        let u = (self.b1 - self.b0) * dx / ((self.t1 - self.t0) as f64 * self.b0);
        let g = if u.abs() < 1e-12 {
            1.0 - u / 2.0
        } else {
            u.ln_1p() / u
        };
        k * dx / self.b0 * g
    }

    /// Frames from `t0` to line `j`: whole frames, so the transport, which
    /// starts the ramp on a whole frame, lands on every line exactly.
    fn line_frames(self, j: i64, k: f64) -> f64 {
        self.exact(self.line(j) as f64, k).round()
    }

    /// Frames from `t0` to `x` as played: straight between lines. Before
    /// the ramp (only the first segment has a before) the start tempo
    /// holds.
    fn frames(self, x: f64, k: f64) -> f64 {
        if x < self.t0 as f64 {
            return (x - self.t0 as f64) * k / self.b0;
        }
        let j = self.step_of(x);
        let (a, b) = (self.line(j) as f64, self.line(j + 1) as f64);
        let (fa, fb) = (self.line_frames(j, k), self.line_frames(j + 1, k));
        fa + (x - a) * (fb - fa) / (b - a)
    }

    /// The inverse of [`Glide::frames`]: the tick `f` frames after `t0`.
    fn tick_at(self, f: f64, k: f64) -> f64 {
        if f < 0.0 {
            return self.t0 as f64 + f * self.b0 / k;
        }
        // The true glide's inverse finds the step, give or take one.
        let y = f * (self.b1 - self.b0) / (k * (self.t1 - self.t0) as f64);
        let h = if y.abs() < 1e-12 {
            1.0 + y / 2.0
        } else {
            y.exp_m1() / y
        };
        let mut j = self.step_of(self.t0 as f64 + f * self.b0 / k * h);
        while j + 1 < self.steps() && self.line_frames(j + 1, k) <= f {
            j += 1;
        }
        while j > 0 && self.line_frames(j, k) > f {
            j -= 1;
        }
        let (a, b) = (self.line(j) as f64, self.line(j + 1) as f64);
        let (fa, fb) = (self.line_frames(j, k), self.line_frames(j + 1, k));
        a + (f - fa) * (b - a) / (fb - fa)
    }
}

impl TempoMap {
    pub fn constant(bpm: f64) -> Option<TempoMap> {
        TempoMap::new(&[Segment { start: 0, bpm }]).ok()
    }

    pub fn new(segments: &[Segment]) -> Result<TempoMap, MapError> {
        if segments.is_empty() {
            return Err(MapError::Empty);
        }
        if segments.len() > MAX_SEGMENTS {
            return Err(MapError::TooManySegments);
        }
        if segments.iter().any(|s| !valid_bpm(s.bpm)) {
            return Err(MapError::BadTempo);
        }
        if segments
            .windows(2)
            .any(|w| w[1].start - w[0].start < MIN_SPAN_TICKS)
        {
            return Err(MapError::BadOrder);
        }
        let mut map = TempoMap {
            segments: [segments[0]; MAX_SEGMENTS],
            len: segments.len(),
            ramp_bits: 0,
        };
        map.segments[..segments.len()].copy_from_slice(segments);
        Ok(map)
    }

    /// Segment `i` glides from its tempo to the next segment's, arriving
    /// on the next segment's start.
    pub fn ramp(mut self, i: usize) -> Result<TempoMap, MapError> {
        if i + 1 >= self.len {
            return Err(MapError::RampAtEnd);
        }
        self.ramp_bits |= 1 << i;
        Ok(self)
    }

    /// Whether segment `i` ramps to the next one.
    pub fn is_ramp(&self, i: usize) -> bool {
        i < self.len && (self.ramp_bits >> i) & 1 == 1
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.len]
    }

    fn index_at(&self, tick: Tick) -> usize {
        // At most 32 entries: a scan is as fast as a search and simpler.
        let mut i = 0;
        while i + 1 < self.len && self.segments[i + 1].start <= tick {
            i += 1;
        }
        i
    }

    /// [`TempoMap::index_at`] for a fractional tick.
    fn index_of(&self, x: f64) -> usize {
        let mut i = 0;
        while i + 1 < self.len && self.segments[i + 1].start as f64 <= x {
            i += 1;
        }
        i
    }

    fn glide(&self, i: usize) -> Option<Glide> {
        self.is_ramp(i).then(|| Glide {
            t0: self.segments[i].start,
            t1: self.segments[i + 1].start,
            b0: self.segments[i].bpm,
            b1: self.segments[i + 1].bpm,
        })
    }

    /// Where segment `i` ends, as a fractional tick (infinity for the last).
    fn end_of(&self, i: usize) -> f64 {
        if i + 1 < self.len {
            self.segments[i + 1].start as f64
        } else {
            f64::INFINITY
        }
    }

    /// Frames from `x` to `y` (`x <= y`), both inside segment `i`.
    fn piece_frames(&self, i: usize, x: f64, y: f64, k: f64) -> f64 {
        match self.glide(i) {
            Some(g) => g.frames(y, k) - g.frames(x, k),
            None => (y - x) * k / self.segments[i].bpm,
        }
    }

    /// The tick `f` frames after `x` (`f` may be negative) inside segment
    /// `i`.
    fn piece_tick(&self, i: usize, x: f64, f: f64, k: f64) -> f64 {
        match self.glide(i) {
            Some(g) => g.tick_at(g.frames(x, k) + f, k),
            None => x + f * self.segments[i].bpm / k,
        }
    }

    /// The tempo at `tick`: on a ramp, the glide's own tempo there.
    pub fn bpm_at(&self, tick: Tick) -> f64 {
        let i = self.index_at(tick);
        match self.glide(i) {
            Some(g) if tick >= g.t0 => g.bpm_at(tick as f64),
            _ => self.segments[i].bpm,
        }
    }

    /// The tempo the transport plays from `tick` to the next change
    /// ([`TempoMap::next_change_after`]) at sample rate `rate`. On a ramp it
    /// is the step's tempo, set so the next line lands on its whole frame
    /// (counted from `tick`, which the transport puts on a whole frame);
    /// elsewhere the segment's tempo.
    pub fn play_bpm(&self, tick: Tick, rate: u32) -> f64 {
        let i = self.index_at(tick);
        match self.glide(i) {
            Some(g) if tick >= g.t0 => {
                let k = frames_k(rate);
                let j = g.step_of(tick as f64);
                let frames = (g.line_frames(j + 1, k) - g.frames(tick as f64, k))
                    .round()
                    .max(1.0);
                (g.line(j + 1) - tick) as f64 * k / frames
            }
            _ => self.segments[i].bpm,
        }
    }

    /// The first tempo change after `tick`, if there is one: the next
    /// segment's start, or on a ramp the next line of its grid.
    pub fn next_change_after(&self, tick: Tick) -> Option<Tick> {
        let i = self.index_at(tick);
        if let Some(g) = self.glide(i) {
            if tick < g.t0 {
                return Some(g.t0);
            }
            return Some(g.line(g.step_of(tick as f64) + 1));
        }
        (i + 1 < self.len).then(|| self.segments[i + 1].start)
    }

    /// Every tempo change as (tick, tempo), in order: each plain
    /// segment's start, and each step of a ramp with the step's average
    /// tempo (the true glide's, whatever the sample rate). For a MIDI
    /// file's tempo track, which can't glide either.
    pub fn changes(&self) -> impl Iterator<Item = (Tick, f64)> + '_ {
        let (mut i, mut j) = (0, 0);
        std::iter::from_fn(move || {
            let s = *self.segments.get(i).filter(|_| i < self.len)?;
            let Some(g) = self.glide(i) else {
                i += 1;
                return Some((s.start, s.bpm));
            };
            let (a, b) = (g.line(j), g.line(j + 1));
            let bpm = (b - a) as f64 / (g.exact(b as f64, 1.0) - g.exact(a as f64, 1.0));
            j += 1;
            if j == g.steps() {
                (i, j) = (i + 1, 0);
            }
            Some((a, bpm))
        })
    }

    /// Frames from tick `a` to tick `b` (fractional, either order) along
    /// the map, as the transport plays it.
    pub fn frames_between(&self, a: f64, b: f64, rate: u32) -> f64 {
        if b < a {
            return -self.frames_between(b, a, rate);
        }
        let k = frames_k(rate);
        let mut i = self.index_of(a);
        let mut t = a;
        let mut f = 0.0;
        loop {
            let end = self.end_of(i);
            f += self.piece_frames(i, t, end.min(b), k);
            if b <= end {
                return f;
            }
            t = end;
            i += 1;
        }
    }

    /// The tick (fractional) `frames` frames after tick `a` along the map;
    /// before it, for negative `frames`. The inverse of
    /// [`TempoMap::frames_between`].
    pub fn tick_after(&self, a: f64, frames: f64, rate: u32) -> f64 {
        let k = frames_k(rate);
        let mut i = self.index_of(a);
        let mut t = a;
        let mut left = frames;
        if frames >= 0.0 {
            loop {
                let end = self.end_of(i);
                if end.is_infinite() || left <= self.piece_frames(i, t, end, k) {
                    return self.piece_tick(i, t, left, k);
                }
                left -= self.piece_frames(i, t, end, k);
                t = end;
                i += 1;
            }
        }
        loop {
            if i == 0 {
                return self.piece_tick(0, t, left, k);
            }
            let start = self.segments[i].start as f64;
            let room = self.piece_frames(i, start, t, k);
            if -left <= room {
                return self.piece_tick(i, t, left, k);
            }
            left += room;
            t = start;
            i -= 1;
        }
    }
}

/// Beats per bar and the note value of one beat (4/4, 3/4, 6/8, 7/8 ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter {
    pub beats: u8,
    pub unit: u8,
}

impl Default for Meter {
    fn default() -> Self {
        Meter { beats: 4, unit: 4 }
    }
}

impl Meter {
    pub fn new(beats: u8, unit: u8) -> Option<Meter> {
        ((1..=32).contains(&beats) && matches!(unit, 1 | 2 | 4 | 8 | 16))
            .then_some(Meter { beats, unit })
    }

    pub fn beat_ticks(self) -> i64 {
        i64::from(PPQ) * 4 / i64::from(self.unit)
    }

    pub fn bar_ticks(self) -> i64 {
        self.beat_ticks() * i64::from(self.beats)
    }

    /// Bar, beat and tick for a song position, counted from 1 like the MPC
    /// shows them (1.1.000 is the start). Before the start, bars count down
    /// through 0, -1 ... so a one-bar count-in reads 0.1 to 0.4.
    pub fn position(self, tick: Tick) -> Position {
        let bar = tick.div_euclid(self.bar_ticks());
        let in_bar = tick.rem_euclid(self.bar_ticks());
        Position {
            bar: bar + 1,
            beat: (in_bar / self.beat_ticks()) as u8 + 1,
            tick: (in_bar % self.beat_ticks()) as u16,
        }
    }

    /// The first bar line at or after `tick`.
    pub fn next_bar(self, tick: Tick) -> Tick {
        ceil_to(tick, self.bar_ticks())
    }
}

/// Bar.beat.tick, all counted from 1 except the tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub bar: i64,
    pub beat: u8,
    pub tick: u16,
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{:03}", self.bar, self.beat, self.tick)
    }
}

/// The first multiple of `step` at or after `tick`.
pub fn ceil_to(tick: Tick, step: i64) -> Tick {
    let r = tick.rem_euclid(step);
    if r == 0 {
        tick
    } else {
        tick + step - r
    }
}

/// Swing: every second line of a grid moves later. Applied by whoever
/// schedules notes (sequencer track, arp), never by the clock itself, so a
/// track can override the transport's swing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swing {
    /// Grid in ticks: 480 swings 1/8 notes, 240 swings 1/16 notes.
    pub grid: u32,
    /// 50 is straight, 66 is close to triplet feel, 75 the most. The same
    /// scale as the arp's `Settings::swing` and the MPC's swing.
    pub amount: u8,
}

impl Default for Swing {
    fn default() -> Self {
        Swing {
            grid: PPQ / 4,
            amount: 50,
        }
    }
}

impl Swing {
    pub fn new(grid: u32, amount: u8) -> Option<Swing> {
        (grid > 0 && (PPQ * 4).is_multiple_of(grid) && (50..=75).contains(&amount))
            .then_some(Swing { grid, amount })
    }

    /// Where a straight tick lands once swung. The off-beat line moves from
    /// `grid` to `grid * amount / 50` within each pair of grid steps; ticks
    /// between lines stretch with it, so microtiming keeps its shape and
    /// order. The arp's rule (odd steps later by (swing - 50) / 50 of a
    /// step) is the same thing for notes on the lines.
    pub fn apply(self, tick: f64) -> f64 {
        if self.amount == 50 {
            return tick;
        }
        let g = f64::from(self.grid);
        let pair = 2.0 * g;
        let base = (tick / pair).floor() * pair;
        let x = tick - base;
        let off = g * f64::from(self.amount) / 50.0;
        let y = if x < g {
            x * off / g
        } else {
            off + (x - g) * (pair - off) / g
        };
        base + y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_from_one_and_down_through_a_count_in() {
        let m = Meter::default();
        assert_eq!(m.position(0).to_string(), "1.1.000");
        assert_eq!(m.position(960 * 5 + 7).to_string(), "2.2.007");
        assert_eq!(m.position(-960).to_string(), "0.4.000");
        assert_eq!(m.position(-3840).to_string(), "0.1.000");
        assert_eq!(m.position(-3841).to_string(), "-1.4.959");
        let m = Meter::new(6, 8).unwrap();
        assert_eq!(m.bar_ticks(), 2880);
        assert_eq!(m.position(2880 + 480 * 5).to_string(), "2.6.000");
        assert!(Meter::new(4, 3).is_none() && Meter::new(0, 4).is_none());
    }

    #[test]
    fn frames_and_ticks_invert() {
        assert_eq!(frames_between(0, 960, 120.0, 44_100), 22_050.0);
        assert_eq!(frames_between(960, 0, 120.0, 44_100), -22_050.0);
        assert_eq!(ticks_in(22_050.0, 120.0, 44_100), 960.0);
    }

    #[test]
    fn tempo_maps_check_their_segments() {
        let s = |start, bpm| Segment { start, bpm };
        assert_eq!(TempoMap::new(&[]), Err(MapError::Empty));
        assert_eq!(TempoMap::new(&[s(0, 10.0)]), Err(MapError::BadTempo));
        assert_eq!(
            TempoMap::new(&[s(0, 120.0), s(100, 90.0)]),
            Err(MapError::BadOrder)
        );
        assert_eq!(
            TempoMap::new(&[s(0, 120.0); 33]),
            Err(MapError::TooManySegments)
        );
        let m = TempoMap::new(&[s(0, 120.0), s(3840, 90.0), s(7680, 140.0)]).unwrap();
        assert_eq!(m.bpm_at(-500), 120.0);
        assert_eq!(m.bpm_at(3839), 120.0);
        assert_eq!(m.bpm_at(3840), 90.0);
        assert_eq!(m.bpm_at(100_000), 140.0);
        assert_eq!(m.next_change_after(-5), Some(3840));
        assert_eq!(m.next_change_after(3840), Some(7680));
        assert_eq!(m.next_change_after(7680), None);
    }

    /// 90 to 140 BPM over four bars, then 140 for a bar, then 100.
    fn ramp_map() -> TempoMap {
        let s = |start, bpm| Segment { start, bpm };
        TempoMap::new(&[s(0, 90.0), s(15_360, 140.0), s(19_200, 100.0)])
            .unwrap()
            .ramp(0)
            .unwrap()
    }

    #[test]
    fn a_ramp_glides_from_one_segment_to_the_next_in_1_16_steps() {
        let s = |start, bpm| Segment { start, bpm };
        let steps = TempoMap::new(&[s(0, 90.0), s(15_360, 140.0)]).unwrap();
        assert_eq!(steps.ramp(1), Err(MapError::RampAtEnd));
        let m = ramp_map();
        assert!(m.is_ramp(0) && !m.is_ramp(1) && !m.is_ramp(2) && !m.is_ramp(40));
        // The glide's tempo: halfway through the ticks is halfway.
        assert_eq!(m.bpm_at(-10), 90.0);
        assert_eq!(m.bpm_at(0), 90.0);
        assert_eq!(m.bpm_at(7_680), 115.0);
        assert_eq!(m.bpm_at(15_360), 140.0);
        assert_eq!(m.bpm_at(19_200), 100.0);
        // A change on every 1/16 line of the ramp, then the segments.
        assert_eq!(m.next_change_after(-5), Some(0));
        assert_eq!(m.next_change_after(0), Some(240));
        assert_eq!(m.next_change_after(250), Some(480));
        assert_eq!(m.next_change_after(15_120), Some(15_360));
        assert_eq!(m.next_change_after(15_360), Some(19_200));
        assert_eq!(m.next_change_after(19_200), None);
        // A ramp that isn't whole steps: the last step takes the rest.
        let odd = TempoMap::new(&[s(0, 160.0), s(1_000, 70.0)])
            .unwrap()
            .ramp(0)
            .unwrap();
        assert_eq!(odd.next_change_after(479), Some(480));
        assert_eq!(odd.next_change_after(720), Some(1_000));
    }

    #[test]
    fn a_ramp_takes_the_true_glides_time_with_every_line_on_a_whole_frame() {
        let m = ramp_map();
        let rate = 48_000;
        let k = 60.0 * 48_000.0 / 960.0;
        // k L / (b1 - b0) ln(b1 / b0): about 8.48 s, between four bars at
        // 90 (10.7 s) and at 140 (6.9 s).
        let exact = k * 15_360.0 / 50.0 * (140.0f64 / 90.0).ln();
        let whole = m.frames_between(0.0, 15_360.0, rate);
        assert!((whole - exact).abs() <= 0.5, "{whole} vs {exact}");
        let mut last = 0.0;
        for line in (240..=15_360).step_by(240) {
            let f = m.frames_between(0.0, f64::from(line), rate);
            assert_eq!(f, f.round(), "line {line}");
            // Each step is shorter than the one before: faster and faster.
            let step = f - last;
            let at_90 = 240.0 * k / 90.0;
            assert!(step < at_90 && step > 240.0 * k / 140.0, "line {line}");
            last = f;
        }
        // Between lines, frames follow ticks in a straight line.
        let (a, b) = (
            m.frames_between(0.0, 480.0, rate),
            m.frames_between(0.0, 720.0, rate),
        );
        assert!((m.frames_between(0.0, 600.0, rate) - (a + b) / 2.0).abs() < 1e-9);
        // Before the ramp, its start tempo holds.
        assert_eq!(m.frames_between(-960.0, 0.0, rate), 960.0 * k / 90.0);
        // After it, plain segments.
        assert!((m.frames_between(15_360.0, 19_200.0, rate) - 3_840.0 * k / 140.0).abs() < 1e-9);
        // Order and joins.
        assert_eq!(
            m.frames_between(7_000.5, 100.0, rate),
            -m.frames_between(100.0, 7_000.5, rate)
        );
        let (x, y, z) = (-500.0, 9_123.25, 20_000.0);
        let joined = m.frames_between(x, y, rate) + m.frames_between(y, z, rate);
        assert!((joined - m.frames_between(x, z, rate)).abs() < 1e-6);
    }

    #[test]
    fn a_ramp_with_one_tempo_is_that_tempo() {
        let s = |start, bpm| Segment { start, bpm };
        let flat = TempoMap::new(&[s(0, 120.0), s(3_840, 120.0)]).unwrap();
        let ramp = flat.ramp(0).unwrap();
        for t in [-100.0, 0.0, 1.0, 239.5, 1_000.0, 3_840.0, 5_000.0] {
            // A 1/16 at 120 is 6000 frames at 48 kHz: the very same.
            assert_eq!(
                ramp.frames_between(0.0, t, 48_000),
                flat.frames_between(0.0, t, 48_000)
            );
            // At 44.1 kHz it is 5512.5: lines move to whole frames.
            let d = ramp.frames_between(0.0, t, 44_100) - flat.frames_between(0.0, t, 44_100);
            assert!(d.abs() <= 0.5, "{t}: {d}");
        }
        assert_eq!(ramp.play_bpm(0, 48_000), 120.0);
        assert_eq!(ramp.play_bpm(1_000, 48_000), 120.0);
    }

    #[test]
    fn changes_list_every_segment_and_every_ramp_step() {
        let s = |start, bpm| Segment { start, bpm };
        let steps = TempoMap::new(&[s(0, 120.0), s(3_840, 90.0)]).unwrap();
        assert_eq!(
            steps.changes().collect::<Vec<_>>(),
            [(0, 120.0), (3_840, 90.0)]
        );
        let m = ramp_map();
        let c: Vec<(Tick, f64)> = m.changes().collect();
        assert_eq!(c.len(), 64 + 2);
        assert_eq!(c[1].0, 240);
        assert_eq!(c[63].0, 15_120);
        assert_eq!(&c[64..], [(15_360, 140.0), (19_200, 100.0)]);
        // Each step's tempo sits between the glide's at its two ends, and
        // the steps take the true glide's time.
        let mut minutes = 0.0;
        for (n, w) in c[..65].windows(2).enumerate() {
            let ((a, bpm), (b, _)) = (w[0], w[1]);
            assert!(bpm > m.bpm_at(a) && bpm < m.bpm_at(b), "step {n}");
            minutes += (b - a) as f64 / (bpm * f64::from(PPQ));
        }
        let exact = 15_360.0 / 50.0 / f64::from(PPQ) * (140.0f64 / 90.0).ln();
        assert!((minutes - exact).abs() < 1e-12);
    }

    #[test]
    fn tick_after_inverts_frames_between_both_ways() {
        let m = ramp_map();
        for rate in [22_050, 44_100, 48_000, 192_000] {
            for a in [-2_000.0, 0.0, 17.25, 7_680.0, 15_359.0, 15_360.0, 25_000.0] {
                for f in [
                    -300_000.0, -1_000.0, -0.5, 0.0, 0.5, 777.0, 123_456.0, 900_000.0,
                ] {
                    let t = m.tick_after(a, f, rate);
                    let back = m.frames_between(a, t, rate);
                    assert!(
                        (back - f).abs() < 1e-6,
                        "rate {rate}, {a} + {f}: {t} gives {back}"
                    );
                }
            }
        }
    }

    #[test]
    fn play_bpm_lands_the_next_line_on_its_whole_frame() {
        let m = ramp_map();
        let rate = 44_100;
        let k = 60.0 * 44_100.0 / 960.0;
        // From a line: the step's tempo covers exactly the map's frames.
        for line in (0..15_360).step_by(240) {
            let bpm = m.play_bpm(line, rate);
            let frames = 240.0 * k / bpm;
            let want = m.frames_between(line as f64, (line + 240) as f64, rate);
            assert!((frames - want).abs() < 1e-6, "line {line}");
            assert!(
                bpm > m.bpm_at(line) && bpm < m.bpm_at(line + 240),
                "line {line}"
            );
        }
        // From inside a step (a locate): a whole number of frames to the
        // next line.
        for t in [1, 1_000, 15_359] {
            let next = m.next_change_after(t).unwrap();
            let frames = (next - t) as f64 * k / m.play_bpm(t, rate);
            assert!((frames - frames.round()).abs() < 1e-6, "from {t}");
            assert!(frames >= 1.0);
        }
        // Off the ramp, the segment's tempo.
        assert_eq!(m.play_bpm(-50, rate), 90.0);
        assert_eq!(m.play_bpm(15_360, rate), 140.0);
        assert_eq!(m.play_bpm(30_000, rate), 100.0);
    }

    #[test]
    fn swing_moves_off_beats_and_keeps_order() {
        let s = Swing::new(240, 66).unwrap();
        assert_eq!(s.apply(0.0), 0.0);
        assert!((s.apply(240.0) - 240.0 * 66.0 / 50.0).abs() < 1e-9);
        assert_eq!(s.apply(480.0), 480.0);
        assert!((s.apply(720.0) - (480.0 + 240.0 * 66.0 / 50.0)).abs() < 1e-9);
        let mut last = -1.0;
        for t in 0..2000 {
            let y = s.apply(f64::from(t));
            assert!(y > last, "order kept at {t}");
            last = y;
        }
        assert_eq!(Swing::default().apply(123.5), 123.5);
        assert!(Swing::new(240, 80).is_none() && Swing::new(7, 60).is_none());
    }

    #[test]
    fn ceil_to_rounds_up_to_the_grid() {
        assert_eq!(ceil_to(0, 240), 0);
        assert_eq!(ceil_to(1, 240), 240);
        assert_eq!(ceil_to(-1, 240), 0);
        assert_eq!(ceil_to(-241, 240), -240);
        assert_eq!(Meter::default().next_bar(3841), 7680);
    }
}

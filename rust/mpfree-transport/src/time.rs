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
}

/// Tempo over the song: steps from one tempo to the next at chosen ticks.
/// Fixed capacity, `Copy`, no allocation. (Ramps, when they come, are
/// another segment kind; the transport already cuts its periods at every
/// segment boundary.)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoMap {
    segments: [Segment; MAX_SEGMENTS],
    len: usize,
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
        if segments.windows(2).any(|w| w[1].start - w[0].start < MIN_SPAN_TICKS) {
            return Err(MapError::BadOrder);
        }
        let mut map = TempoMap { segments: [segments[0]; MAX_SEGMENTS], len: segments.len() };
        map.segments[..segments.len()].copy_from_slice(segments);
        Ok(map)
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

    /// The tempo at `tick`.
    pub fn bpm_at(&self, tick: Tick) -> f64 {
        self.segments[self.index_at(tick)].bpm
    }

    /// The first segment boundary after `tick`, if there is one.
    pub fn next_change_after(&self, tick: Tick) -> Option<Tick> {
        let i = self.index_at(tick);
        (i + 1 < self.len).then(|| self.segments[i + 1].start)
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
        ((1..=32).contains(&beats) && matches!(unit, 1 | 2 | 4 | 8 | 16)).then_some(Meter { beats, unit })
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
    if r == 0 { tick } else { tick + step - r }
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
        Swing { grid: PPQ / 4, amount: 50 }
    }
}

impl Swing {
    pub fn new(grid: u32, amount: u8) -> Option<Swing> {
        (grid > 0 && (PPQ * 4).is_multiple_of(grid) && (50..=75).contains(&amount)).then_some(Swing { grid, amount })
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
        let y = if x < g { x * off / g } else { off + (x - g) * (pair - off) / g };
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
        assert_eq!(TempoMap::new(&[s(0, 120.0), s(100, 90.0)]), Err(MapError::BadOrder));
        assert_eq!(TempoMap::new(&[s(0, 120.0); 33]), Err(MapError::TooManySegments));
        let m = TempoMap::new(&[s(0, 120.0), s(3840, 90.0), s(7680, 140.0)]).unwrap();
        assert_eq!(m.bpm_at(-500), 120.0);
        assert_eq!(m.bpm_at(3839), 120.0);
        assert_eq!(m.bpm_at(3840), 90.0);
        assert_eq!(m.bpm_at(100_000), 140.0);
        assert_eq!(m.next_change_after(-5), Some(3840));
        assert_eq!(m.next_change_after(3840), Some(7680));
        assert_eq!(m.next_change_after(7680), None);
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

//! The grain voice: schedules grains and mixes them.

use crate::buffer::GrainSource;
use crate::rng::Rng;
use crate::window::{Window, WindowTable};

/// Grains one voice can play at once.
pub const MAX_GRAINS: usize = 64;

/// Locks density and size to the tempo. Grains then start on exact frames
/// of the transport's timeline, so a synced cloud stays on the grid however
/// the audio is cut into periods.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoSync {
    /// Output frames per beat (60 × rate / BPM).
    pub frames_per_beat: f64,
    /// Grains per beat (4 = sixteenths). `None` keeps `Params::density`.
    pub grains_per_beat: Option<f32>,
    /// Grain length in beats. `None` keeps `Params::size_ms`.
    pub size_beats: Option<f32>,
}

/// Everything a performer can turn. Out-of-range values are clamped when
/// grains are made, so any value is safe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Where grains read, 0 to 1 across the source's span.
    pub position: f32,
    /// Random offset of each grain's start, 0 to 1 of the span.
    pub spray: f32,
    /// Grain length, 1 ms to 4 s (the design's range is 2 ms to 2 s).
    pub size_ms: f32,
    /// Grains per second, 0.1 to 1000.
    pub density: f32,
    /// Semitones, ±48.
    pub pitch: f32,
    /// Random pitch per grain, ± this many semitones (0 to 24).
    pub pitch_spray: f32,
    /// Allowed semitones as a 12-bit mask relative to the sample's own pitch
    /// (bit 0 is the original pitch). 0 turns quantising off.
    /// `0b1010_1011_0101` is a major scale.
    pub scale: u16,
    /// How fast the reading position moves through the source: 1 is the
    /// original speed, 0 frozen, negative backwards (−4 to 4).
    pub scan: f32,
    pub window: Window,
    /// Chance that a grain plays backwards, 0 to 1.
    pub reverse: f32,
    /// Random pan per grain, 0 (centre) to 1 (anywhere).
    pub spread: f32,
    /// Linear output gain.
    pub gain: f32,
    pub sync: Option<TempoSync>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            position: 0.0,
            spray: 0.0,
            size_ms: 80.0,
            density: 20.0,
            pitch: 0.0,
            pitch_spray: 0.0,
            scale: 0,
            scan: 0.0,
            window: Window::Hann,
            reverse: 0.0,
            spread: 0.0,
            gain: 1.0,
            sync: None,
        }
    }
}

/// A cap on grains alive at once, shared by every voice the audio thread
/// renders, so the period's cost is bounded however many voices play.
/// Voices count their grains in as they start and out as they end; a voice
/// that is dropped while playing must be [`GrainVoice::kill`]ed first, or
/// its grains stay counted.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub limit: usize,
    live: usize,
    /// Grains refused by this budget since it was made.
    pub skipped: u64,
}

impl Budget {
    pub fn new(limit: usize) -> Self { Budget { limit, live: 0, skipped: 0 } }
    /// Grains alive now across the voices using this budget.
    pub fn live(&self) -> usize { self.live }
}

/// Counters a UI or test can read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub spawned: u64,
    /// Grains not made because the voice or the budget was full.
    pub skipped: u64,
    pub active: usize,
    pub peak_active: usize,
}

/// One grain, for drawing the cloud.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrainView {
    /// Reading position, 0 to 1 across the span.
    pub position: f32,
    /// The window's current value, 0 to 1.
    pub level: f32,
    pub reverse: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Grain {
    pos: f64,
    inc: f64,
    age: u32,
    len: u32,
    dphase: f32,
    gl: f32,
    gr: f32,
}

/// One granular voice: a fixed array of grains, a scheduler and a seeded
/// generator. Make it off the audio thread; every method after that is
/// real-time safe.
#[derive(Clone, Debug)]
pub struct GrainVoice {
    out_rate: u32,
    params: Params,
    window: WindowTable,
    grains: [Grain; MAX_GRAINS],
    active: usize,
    rng: Rng,
    /// Output frames rendered.
    clock: u64,
    /// Voice clock frame of the next free-running grain.
    next_at: f64,
    last_spawn: f64,
    spawning: bool,
    /// How far scanning has moved the reading position, in source frames.
    scan_offset: f64,
    span: (i64, i64),
    stats: Stats,
}

impl GrainVoice {
    pub fn new(out_rate: u32, seed: u64) -> Self {
        let params = Params::default();
        GrainVoice {
            out_rate: out_rate.max(1),
            params,
            window: WindowTable::new(params.window),
            grains: [Grain::default(); MAX_GRAINS],
            active: 0,
            rng: Rng::new(seed),
            clock: 0,
            next_at: 0.0,
            last_spawn: 0.0,
            spawning: false,
            scan_offset: 0.0,
            span: (0, 0),
            stats: Stats::default(),
        }
    }

    pub fn params(&self) -> &Params { &self.params }

    /// Takes effect for the next grain; grains already playing finish as
    /// they started. A new window shape rebuilds the table (1025 values).
    pub fn set_params(&mut self, p: Params) {
        self.params = p;
        self.window.set(p.window);
    }

    /// Restarts the random sequence.
    pub fn reseed(&mut self, seed: u64) { self.rng = Rng::new(seed); }

    /// Starts making grains, the first at the start of the next render.
    /// Scanning restarts from `position`.
    pub fn start(&mut self) {
        self.spawning = true;
        self.next_at = self.clock as f64;
        self.last_spawn = self.clock as f64;
        self.scan_offset = 0.0;
    }

    /// Stops making grains; the ones playing finish (a natural release).
    pub fn stop(&mut self) { self.spawning = false; }

    /// Silences at once and hands its grains back to the budget. Clicks
    /// unless the output is already being faded.
    pub fn kill(&mut self, budget: &mut Budget) {
        self.spawning = false;
        budget.live = budget.live.saturating_sub(self.active);
        self.active = 0;
        self.stats.active = 0;
    }

    /// True while making grains or while any still play.
    pub fn is_active(&self) -> bool { self.spawning || self.active > 0 }
    pub fn is_spawning(&self) -> bool { self.spawning }
    pub fn stats(&self) -> Stats { self.stats }

    /// The reading position (before spray), 0 to 1 across the span of the
    /// last render.
    pub fn playhead(&self) -> f32 {
        let len = (self.span.1 - self.span.0) as f64;
        if len <= 0.0 {
            return 0.0;
        }
        (self.base_offset(len) / len) as f32
    }

    /// Fills `out` with the grains playing now and returns how many.
    pub fn grains(&self, out: &mut [GrainView]) -> usize {
        let (start, end) = self.span;
        let len = ((end - start) as f64).max(1.0);
        let n = self.active.min(out.len());
        for (o, g) in out.iter_mut().zip(&self.grains[..n]) {
            *o = GrainView {
                position: ((g.pos - start as f64) / len).clamp(0.0, 1.0) as f32,
                level: self.window.at(g.age as f32 * g.dphase),
                reverse: g.inc < 0.0,
            };
        }
        n
    }

    fn base_offset(&self, len: f64) -> f64 {
        (f64::from(self.params.position.clamp(0.0, 1.0)) * len + self.scan_offset).rem_euclid(len)
    }

    fn size_frames(&self) -> f64 {
        let p = &self.params;
        let rate = f64::from(self.out_rate);
        let frames = match p.sync.and_then(|s| s.size_beats.map(|b| f64::from(b) * s.frames_per_beat)) {
            Some(f) => f,
            None => f64::from(p.size_ms) * rate / 1000.0,
        };
        frames.clamp(rate * 0.001, rate * 4.0).max(2.0)
    }

    /// Grains per second, synced or not.
    fn density(&self) -> f64 {
        let p = &self.params;
        let d = match p.sync.and_then(|s| s.grains_per_beat.map(|g| (s, g))) {
            Some((s, g)) if s.frames_per_beat > 0.0 => f64::from(g) * f64::from(self.out_rate) / s.frames_per_beat,
            _ => f64::from(p.density),
        };
        d.clamp(0.1, 1000.0)
    }

    /// Synced grain interval in output frames, if density is synced.
    fn sync_interval(&self) -> Option<f64> {
        let s = self.params.sync?;
        let g = s.grains_per_beat?;
        if s.frames_per_beat <= 0.0 || g <= 0.0 {
            return None;
        }
        Some((s.frames_per_beat / f64::from(g)).max(1.0))
    }

    /// Offset in this block (at or after `from`, before `n`) of the next grain.
    fn next_spawn(&mut self, transport_frame: u64, from: usize, n: usize) -> Option<usize> {
        if !self.spawning || from >= n {
            return None;
        }
        if let Some(iv) = self.sync_interval() {
            // Grain k starts on frame round(k × interval) of the transport.
            let t = transport_frame + from as u64;
            let mut k = ((t as f64 - 0.5) / iv).ceil().max(0.0) as u64;
            while ((k as f64) * iv).round() < t as f64 {
                k += 1;
            }
            let f = ((k as f64) * iv).round() as u64;
            let off = f.checked_sub(transport_frame)? as usize;
            return (off < n).then_some(off);
        }
        // A faster density takes effect at once, not after a slow wait.
        let gap = f64::from(self.out_rate) / self.density();
        self.next_at = self.next_at.min(self.last_spawn + gap);
        let off = (self.next_at - self.clock as f64).ceil().max(from as f64);
        (off < n as f64).then_some(off as usize)
    }

    /// Mixes this voice into `left` and `right` (it adds; clear them first
    /// if this is the only voice). `transport_frame` is the absolute frame of
    /// `left[0]`, used only for tempo sync.
    pub fn render<S: GrainSource>(&mut self, src: &S, budget: &mut Budget, transport_frame: u64, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let (start, end) = src.span();
        if end - start < 8 {
            // Nothing readable: let playing grains fall silent with the source.
            budget.live = budget.live.saturating_sub(self.active);
            self.active = 0;
            self.stats.active = 0;
            self.span = (start, end);
            self.clock += n as u64;
            return;
        }
        self.span = (start, end);
        let len = (end - start) as f64;
        let src_ratio = f64::from(src.rate()) / f64::from(self.out_rate);
        let scan = f64::from(self.params.scan.clamp(-4.0, 4.0)) * src_ratio;
        let mut done = 0usize;
        let mut search = 0usize;
        while done < n {
            let next = self.next_spawn(transport_frame, search, n);
            let seg = next.unwrap_or(n);
            self.mix(src, budget, &mut left[done..seg], &mut right[done..seg]);
            self.scan_offset = (self.scan_offset + scan * (seg - done) as f64).rem_euclid(len);
            done = seg;
            if let Some(off) = next {
                self.spawn(src, budget, src_ratio);
                let now = self.clock as f64 + off as f64;
                self.last_spawn = now;
                self.next_at = now + f64::from(self.out_rate) / self.density();
                search = off + 1;
            }
        }
        self.clock += n as u64;
        self.stats.active = self.active;
    }

    fn spawn<S: GrainSource>(&mut self, src: &S, budget: &mut Budget, src_ratio: f64) {
        // Draw the same random numbers every time, used or not, so changing
        // one parameter doesn't reshuffle the others.
        let r_spray = self.rng.bipolar();
        let r_pitch = self.rng.bipolar();
        let r_rev = self.rng.unit();
        let r_pan = self.rng.bipolar();
        if self.active >= MAX_GRAINS || budget.live >= budget.limit {
            self.stats.skipped += 1;
            budget.skipped += 1;
            return;
        }
        let p = self.params;
        let (start, end) = src.span();
        let len = (end - start) as f64;

        let mut semis = p.pitch.clamp(-48.0, 48.0) + p.pitch_spray.clamp(0.0, 24.0) * r_pitch;
        if p.scale & 0xFFF != 0 {
            semis = quantise(semis, p.scale);
        }
        let ratio = f64::from((semis / 12.0).exp2());
        let reverse = r_rev < p.reverse.clamp(0.0, 1.0);
        let speed = ratio * src_ratio;

        // Keep the whole read (with the interpolator's neighbours) in the span.
        let lo = start as f64 + 1.0;
        let hi = end as f64 - 3.0;
        let mut size = self.size_frames();
        if size * speed > hi - lo {
            size = (hi - lo) / speed;
        }
        if size < 2.0 {
            self.stats.skipped += 1;
            return;
        }
        let size = size.floor();
        let extent = (size - 1.0) * speed;
        let pos = start as f64 + (self.base_offset(len) + f64::from(p.spray.clamp(0.0, 1.0) * r_spray) * len * 0.5).rem_euclid(len);
        let pos = if reverse { pos.clamp(lo + extent, hi) } else { pos.clamp(lo, hi - extent) };

        let pan = p.spread.clamp(0.0, 1.0) * r_pan;
        // Overlapping grains add up; keep a dense cloud near the level of a sparse one.
        let overlap = self.density() * size / f64::from(self.out_rate);
        let level = p.gain / (overlap.max(1.0).sqrt() as f32);
        self.grains[self.active] = Grain {
            pos,
            inc: if reverse { -speed } else { speed },
            age: 0,
            len: size as u32,
            dphase: 1.0 / (size as f32 - 1.0),
            gl: level * (1.0 - pan).min(1.0),
            gr: level * (1.0 + pan).min(1.0),
        };
        self.active += 1;
        budget.live += 1;
        self.stats.spawned += 1;
        self.stats.peak_active = self.stats.peak_active.max(self.active);
    }

    fn mix<S: GrainSource>(&mut self, src: &S, budget: &mut Budget, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        if n == 0 {
            return;
        }
        let mut i = 0;
        while i < self.active {
            let g = &mut self.grains[i];
            let m = ((g.len - g.age) as usize).min(n);
            let (mut pos, mut age) = (g.pos, g.age);
            for (l, r) in left[..m].iter_mut().zip(&mut right[..m]) {
                // Grains are kept inside the span, which never starts below
                // frame 0, so truncation is floor (and avoids a libm call).
                let f = pos as i64;
                let [sl, sr] = src.read(f, (pos - f as f64) as f32);
                let w = self.window.at(age as f32 * g.dphase);
                *l += sl * w * g.gl;
                *r += sr * w * g.gr;
                pos += g.inc;
                age += 1;
            }
            g.pos = pos;
            g.age = age;
            if g.age >= g.len {
                budget.live = budget.live.saturating_sub(1);
                self.active -= 1;
                self.grains[i] = self.grains[self.active];
            } else {
                i += 1;
            }
        }
    }
}

/// The nearest allowed semitone to `semis`. `mask` bit n allows semitone n
/// of every octave (bit 0 is the original pitch). Ties go down. A mask with
/// no bits set returns `semis` unchanged.
pub fn quantise(semis: f32, mask: u16) -> f32 {
    if mask & 0xFFF == 0 {
        return semis;
    }
    let mut best = semis;
    let mut dist = f32::INFINITY;
    let lo = semis.floor() as i32 - 12;
    for n in lo..=lo + 25 {
        if mask & (1 << n.rem_euclid(12)) != 0 {
            let d = (n as f32 - semis).abs();
            if d < dist {
                dist = d;
                best = n as f32;
            }
        }
    }
    best
}

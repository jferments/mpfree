//! Smart loop ends: where a free first loop really ends, from the notes
//! you played (the smart loop end plan, step 1).
//!
//! A free first pass closes where you tap. A tap a little late or early
//! puts a stutter at the loop point on every pass and sets the song's
//! tempo a little off. With pads and MIDI the looper knows the frame of
//! every note you played ([`NoteLog`]), so it can look for where the loop
//! really came round: the opening hit played again near the tap.
//!
//! Every note start within the reach of the tap ([`reach`]) is a
//! candidate end, grouped into hits played together. Each is scored on
//! four named parts ([`Candidate`]):
//!
//! 1. **Onset:** the hit there matches the loop's opening hit: the same
//!    pad or note (any note of a first chord), weighted by how loud that
//!    note was in the opening and how alike the two velocities are.
//! 2. **Rhythm:** the loop's own notes, before the reach, against a beat
//!    grid of the length that end gives (the bar count the reading takes
//!    for it), weighted by accent, so a laid-back snare or a hi-hat off
//!    the beat counts less than the kick on it. This works even if you
//!    stop playing at the tap.
//! 3. **Context:** the notes after the opening come round again after the
//!    candidate, when you kept playing.
//! 4. **Prior:** how near it is to your tap, as a tie-break.
//!
//! Your tap is scored the same way (no onset, its own rhythm, context and
//! a full prior). The best candidate is taken only when its score reaches
//! [`THRESHOLD`] and leads everything else, the tap included, by
//! [`MARGIN`]. Otherwise the tap stands ([`Verdict`]), and the host counts,
//! logs and shows that. Notes the log had to drop near the reach keep the
//! tap too. Nothing allocates: the log is fixed, and a decision works on
//! the stack.

use mpfree_transport::time::Tick;

/// A note started: a pad hit or a MIDI note-on, keyed on what was played
/// (not on what sounds), on the song's frame clock: the frame it sounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteStart {
    pub frame: u64,
    /// Where it was played: the track the pad or key played on (a MIDI
    /// route's key plays the pad that plays its note on its track).
    pub source: u16,
    /// The pad or note number.
    pub key: u8,
    pub velocity: u8,
}

/// What smart loop end weighs: a note started ([`NoteStart`], pads and
/// MIDI) or a hit heard ([`crate::hits::Hit`], the line input and mixes).
/// A decision ([`decide`]) and the beat inside a loop ([`InnerBeat`])
/// work the same way on either.
pub trait Event: Copy {
    /// Nothing, to fill a fixed store.
    const NONE: Self;
    /// The frame it sounded, on the song's frame clock.
    fn frame(&self) -> u64;
    /// How hard it was played, 0 to 1.
    fn level(&self) -> f64;
    /// How alike two are, 0 (another pad, note or sound) to 1 (the same).
    fn alike(&self, o: &Self) -> f64;
    /// The accent weight: soft ones count far less than loud ones.
    fn weight(&self) -> f64 {
        let v = self.level();
        v * v
    }
}

/// Two events at least this alike are the same pad, note or sound for
/// the context part of a decision.
pub const ALIKE: f64 = 0.5;

impl Event for NoteStart {
    const NONE: NoteStart = NoteStart {
        frame: 0,
        source: 0,
        key: 0,
        velocity: 0,
    };

    fn frame(&self) -> u64 {
        self.frame
    }

    fn level(&self) -> f64 {
        f64::from(self.velocity.min(127)) / 127.0
    }

    /// The same pad or note on the same track: 1, else 0.
    fn alike(&self, o: &NoteStart) -> f64 {
        if self.source == o.source && self.key == o.key {
            1.0
        } else {
            0.0
        }
    }
}

/// Notes the log keeps from a pass's start: the opening and the start of
/// the loop's rhythm.
pub const HEAD: usize = 128;
/// The most recent notes it keeps after those, rolling: the reach and the
/// end of the loop's rhythm.
pub const TAIL: usize = 128;
/// Hits played together: notes within this many milliseconds of a hit's
/// first note are one hit (a chord, a flam).
pub const TOGETHER_MS: f64 = 30.0;
/// The reach either side of the tap, at most: a 1/16 note if that is
/// shorter (above 150 BPM).
pub const REACH_MS: f64 = 100.0;
/// How near a later note must come to where the opening's note would
/// come round, to count for context.
pub const CONTEXT_MS: f64 = 20.0;
/// Candidates scored at most (hits within the reach).
pub const MAX_CANDIDATES: usize = 16;

/// The weights of the four parts. Set on synthetic performances for now,
/// to be tuned on real takes in the device sessions, wrong corrections
/// first.
pub const W_ONSET: f64 = 1.0;
pub const W_RHYTHM: f64 = 1.0;
pub const W_CONTEXT: f64 = 0.6;
pub const W_PRIOR: f64 = 0.2;
/// The score the best candidate must reach: the opening hit played again
/// just as it was is enough on its own; a softer one needs the rhythm too.
pub const THRESHOLD: f64 = 0.9;
/// How far ahead of everything else (the tap included) it must be.
pub const MARGIN: f64 = 0.4;

/// A pass's notes (or hits), from its start: the first [`HEAD`], then the
/// last [`TAIL`]. Ones the tail had to drop are counted, and the newest
/// frame dropped kept, so a decision knows whether its window is whole.
#[derive(Clone, Debug)]
pub struct EventLog<E: Event> {
    head: [E; HEAD],
    n_head: usize,
    tail: [E; TAIL],
    /// Where the next tail note goes.
    next: usize,
    n_tail: usize,
    dropped: u64,
    /// The newest frame of a note dropped, if any was.
    lost_to: Option<u64>,
}

/// The notes of a pass (pads and MIDI).
pub type NoteLog = EventLog<NoteStart>;

impl<E: Event> Default for EventLog<E> {
    fn default() -> EventLog<E> {
        EventLog::new()
    }
}

impl<E: Event> EventLog<E> {
    pub fn new() -> EventLog<E> {
        EventLog {
            head: [E::NONE; HEAD],
            n_head: 0,
            tail: [E::NONE; TAIL],
            next: 0,
            n_tail: 0,
            dropped: 0,
            lost_to: None,
        }
    }

    /// Forget every note (a new pass).
    pub fn clear(&mut self) {
        self.n_head = 0;
        self.n_tail = 0;
        self.next = 0;
        self.dropped = 0;
        self.lost_to = None;
    }

    /// A note started (or a hit heard).
    pub fn push(&mut self, n: E) {
        if self.n_head < HEAD {
            self.head[self.n_head] = n;
            self.n_head += 1;
            return;
        }
        if self.n_tail == TAIL {
            let old = self.tail[self.next];
            self.dropped += 1;
            self.lost_to = Some(self.lost_to.map_or(old.frame(), |f| f.max(old.frame())));
        } else {
            self.n_tail += 1;
        }
        self.tail[self.next] = n;
        self.next = (self.next + 1) % TAIL;
    }

    /// A note started on `frame` that was lost before it reached the
    /// log: counted like one dropped, so [`NoteLog::whole_from`] knows.
    pub fn lost(&mut self, frame: u64) {
        self.dropped += 1;
        self.lost_to = Some(self.lost_to.map_or(frame, |f| f.max(frame)));
    }

    /// Keep only the notes from frame `from` on, the first of them at the
    /// head (a pass starting there).
    pub fn keep_from(&mut self, from: u64) {
        let mut buf = [E::NONE; HEAD + TAIL];
        let n = self.sorted(&mut buf);
        let lost = self.lost_to.filter(|&f| f >= from);
        let dropped = if lost.is_some() { self.dropped } else { 0 };
        self.clear();
        for &x in buf[..n].iter().filter(|x| x.frame() >= from) {
            self.push(x);
        }
        self.dropped += dropped;
        self.lost_to = self.lost_to.max(lost);
    }

    /// Notes the log had to drop since it was cleared.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// How many notes it holds.
    pub fn len(&self) -> usize {
        self.n_head + self.n_tail
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether every note from frame `from` on is still kept.
    pub fn whole_from(&self, from: u64) -> bool {
        self.lost_to.is_none_or(|f| f < from)
    }

    /// The notes it holds, in frame order, into `out`; how many.
    pub(crate) fn sorted(&self, out: &mut [E; HEAD + TAIL]) -> usize {
        let mut n = 0;
        for &x in &self.head[..self.n_head] {
            out[n] = x;
            n += 1;
        }
        let first = if self.n_tail == TAIL { self.next } else { 0 };
        for k in 0..self.n_tail {
            out[n] = self.tail[(first + k) % TAIL];
            n += 1;
        }
        // Mostly in order already (pads and keys within a period can
        // interleave): an insertion sort, stable, allocation-free.
        for i in 1..n {
            let x = out[i];
            let mut j = i;
            while j > 0 && out[j - 1].frame() > x.frame() {
                out[j] = out[j - 1];
                j -= 1;
            }
            out[j] = x;
        }
        n
    }
}

/// The fewest notes inside a loop (after its opening hit) that make its
/// beat evidence for the bar reading ([`InnerBeat`]).
pub const MIN_BEAT_NOTES: usize = 4;

/// The beat inside a loop: its notes, in frame order, from its opening
/// hit, as evidence for how many bars it is ([`InnerBeat::fit`],
/// [`crate::reading::read_bars_with`]). Allocation-free: the notes are
/// copied onto the stack.
pub struct InnerBeat<E: Event> {
    notes: [E; HEAD + TAIL],
    n: usize,
    rate: u32,
    /// The loop's opening hit (its grid starts there) and its length.
    origin: u64,
    len: usize,
    /// Beats in a bar, by the meter.
    beats: f64,
}

impl<E: Event> InnerBeat<E> {
    /// The notes of `log` in a loop of `len` frames from frame `from` (the
    /// press, or the first hit), with bars of `bar` ticks and beats of
    /// `beat`. Its grid starts on the opening hit: the first note within
    /// the reach of `from`, else `from` itself. The opening's own notes
    /// are left out: they fall on the grid whatever the bar count.
    pub fn new(
        log: &EventLog<E>,
        rate: u32,
        from: u64,
        len: usize,
        bar: Tick,
        beat: Tick,
    ) -> InnerBeat<E> {
        let mut all = [E::NONE; HEAD + TAIL];
        let n = log.sorted(&mut all);
        let all = &all[..n];
        let reach = reach(rate, f64::NAN);
        let open = all
            .iter()
            .position(|x| x.frame() >= from && x.frame() <= from + reach);
        let origin = open.map_or(from, |i| all[i].frame());
        let skip = open.map_or(0, |i| hit_at(all, i, frames(rate, TOGETHER_MS)).1);
        let end = from + len as u64;
        let mut notes = [E::NONE; HEAD + TAIL];
        let mut k = 0;
        for &x in all[skip..]
            .iter()
            .filter(|x| x.frame() > origin && x.frame() < end)
        {
            notes[k] = x;
            k += 1;
        }
        InnerBeat {
            notes,
            n: k,
            rate,
            origin,
            len,
            beats: bar as f64 / beat.max(1) as f64,
        }
    }

    /// How well the notes fall on the beats of `bars` bars over the loop,
    /// weighted by accent, 0 to 1: each scores by how near a beat it falls
    /// (a bell a twentieth of a beat wide, 10 to 30 ms), as the rhythm
    /// part of a decision does. None with fewer than [`MIN_BEAT_NOTES`].
    pub fn fit(&self, bars: u32) -> Option<f64> {
        if self.n < MIN_BEAT_NOTES || bars == 0 {
            return None;
        }
        let beat = self.len as f64 / (f64::from(bars) * self.beats);
        if !(beat.is_finite() && beat > 1.0) {
            return None;
        }
        let rate = f64::from(self.rate);
        let sigma = (beat * 0.05).clamp(0.010 * rate, 0.030 * rate);
        let (mut sum, mut wsum) = (0.0, 0.0);
        for x in &self.notes[..self.n] {
            let t = (x.frame() - self.origin) as f64;
            let off = t - (t / beat).round() * beat;
            let w = x.weight();
            sum += w * (-(off / sigma) * (off / sigma)).exp();
            wsum += w;
        }
        (wsum > 0.0).then(|| sum / wsum)
    }
}

/// The reach either side of a tap, in frames: [`REACH_MS`], or a 1/16
/// note at `bpm` if that is shorter.
pub fn reach(rate: u32, bpm: f64) -> u64 {
    let ms = if bpm.is_finite() && bpm > 0.0 {
        REACH_MS.min(60_000.0 / (bpm * 4.0))
    } else {
        REACH_MS
    };
    frames(rate, ms)
}

fn frames(rate: u32, ms: f64) -> u64 {
    (ms * f64::from(rate) / 1000.0).round() as u64
}

/// What a decision is asked about one free first pass.
#[derive(Clone, Copy, Debug)]
pub struct Ask {
    pub rate: u32,
    /// The pass's first frame (the press, or your first note).
    pub start: u64,
    /// Your closing tap: the end the loop has without a correction.
    pub tap: u64,
    /// The reach either side of the tap ([`reach`]).
    pub reach: u64,
    /// The last frame heard so far: notes after it aren't known yet.
    pub heard_to: u64,
    /// The bar, and the beat inside it, in ticks (the meter's), for the
    /// rhythm's grid.
    pub bar: Tick,
    pub beat: Tick,
}

/// One end scored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// The loop's end: where it comes round.
    pub end: u64,
    pub onset: f64,
    pub rhythm: f64,
    pub context: f64,
    pub prior: f64,
    pub score: f64,
}

/// Why the tap stood, or that it moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A clear winner: the end moves to it.
    Moved,
    /// No note within the reach.
    NoMatch,
    /// Candidates, but none clearly ahead.
    NotClear,
    /// Notes near the reach were dropped (the log was full): no decision.
    Lost,
    /// By ear: the finder didn't answer in time ([`crate::find`]).
    Late,
}

impl Verdict {
    /// The hint line's reason for a tap that stood.
    pub fn why(self) -> &'static str {
        match self {
            Verdict::Moved => "a clear match",
            Verdict::NoMatch => "no note near it",
            Verdict::NotClear => "no clear match near it",
            Verdict::Lost => "too many notes to follow",
            Verdict::Late => "it couldn't listen in time",
        }
    }

    /// [`Verdict::why`], or for hits heard on the line input or a mix
    /// (`heard`) [`Verdict::why_heard`].
    pub fn why_by(self, heard: bool) -> &'static str {
        if heard {
            self.why_heard()
        } else {
            self.why()
        }
    }

    /// The same, for hits heard on the line input or a mix.
    pub fn why_heard(self) -> &'static str {
        match self {
            Verdict::NoMatch => "no hit near it",
            Verdict::Lost => "too many hits to follow",
            v => v.why(),
        }
    }
}

/// What a decision found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub verdict: Verdict,
    /// The end to use: the winner's when moved, else the tap.
    pub end: u64,
    /// The best candidate and the runner-up (the tap, when it was), for
    /// the log line.
    pub best: Option<Candidate>,
    pub second: Option<Candidate>,
    /// The tap, scored.
    pub tap: Candidate,
    /// Frames from the pass's start to its opening hit, when that came
    /// within the reach of the start (a loop started on REC a moment
    /// before your first hit): where a smart start puts the loop's start.
    /// 0 otherwise. Every end here is measured from there: the loop is
    /// `start + lead..end`, and from the press, as long, `start..end -
    /// lead`.
    pub lead: u64,
}

impl Decision {
    /// How far the end moved from the tap, in frames: negative when the
    /// tap was late.
    pub fn moved_by(&self, tap: u64) -> i64 {
        self.end as i64 - tap as i64
    }
}

/// Decide where a free first pass ends, from `log`'s notes. `bars_for`
/// gives the bar count the reading takes for a loop of so many frames
/// (None when it can't read one). Real-time safe.
pub fn decide<E: Event>(
    log: &EventLog<E>,
    ask: &Ask,
    bars_for: &dyn Fn(usize) -> Option<u32>,
) -> Decision {
    let mut buf = [E::NONE; HEAD + TAIL];
    let n = log.sorted(&mut buf);
    let notes = &buf[..n];
    let together = frames(ask.rate, TOGETHER_MS);
    // The opening hit: the first notes from the pass's start, played
    // together, within the reach of the start.
    let open = notes
        .iter()
        .position(|x| x.frame() >= ask.start && x.frame() <= ask.start + ask.reach);
    let opening = open.map(|i| hit_at(notes, i, together));
    // The loop is measured from it (a loop started on REC a moment before
    // your first hit starts on the hit): an end is where it comes round.
    let lead = opening.map_or(0, |o| notes[o.0].frame() - ask.start);
    let lo = ask.tap.saturating_sub(ask.reach).max(ask.start + lead + 1);
    let hi = (ask.tap + ask.reach).min(ask.heard_to);
    let ctx = Ctx {
        notes,
        ask,
        together,
        opening,
        lead,
        bars_for,
    };
    let tap = ctx.score(ask.tap, None);
    // Notes dropped within the reach: no decision.
    if !log.whole_from(lo) {
        return Decision {
            verdict: Verdict::Lost,
            end: ask.tap,
            best: None,
            second: None,
            tap,
            lead,
        };
    }
    // Candidates: each hit within the reach (the first note of each group
    // played together).
    let mut cands = [tap; MAX_CANDIDATES];
    let mut n_c = 0;
    let mut i = notes.partition_point(|x| x.frame() < lo);
    while i < notes.len() && notes[i].frame() <= hi && n_c < MAX_CANDIDATES {
        let (first, last) = hit_at(notes, i, together);
        cands[n_c] = ctx.score(notes[first].frame(), Some((first, last)));
        n_c += 1;
        i = last;
    }
    if n_c == 0 {
        return Decision {
            verdict: Verdict::NoMatch,
            end: ask.tap,
            best: None,
            second: None,
            tap,
            lead,
        };
    }
    // The best, and the best of the rest with the tap.
    let mut b = 0;
    for k in 1..n_c {
        if cands[k].score > cands[b].score {
            b = k;
        }
    }
    let best = cands[b];
    let mut second = tap;
    for (k, c) in cands[..n_c].iter().enumerate() {
        if k != b && c.score > second.score {
            second = *c;
        }
    }
    let clear = best.score >= THRESHOLD && best.score - second.score >= MARGIN;
    Decision {
        verdict: if clear {
            Verdict::Moved
        } else {
            Verdict::NotClear
        },
        end: if clear { best.end } else { ask.tap },
        best: Some(best),
        second: Some(second),
        tap,
        lead,
    }
}

/// The hit (notes played together) that starts at note `i`: its first
/// note and one past its last.
fn hit_at<E: Event>(notes: &[E], i: usize, together: u64) -> (usize, usize) {
    let f = notes[i].frame();
    let mut j = i + 1;
    while j < notes.len() && notes[j].frame() - f <= together {
        j += 1;
    }
    (i, j)
}

struct Ctx<'a, E: Event> {
    notes: &'a [E],
    ask: &'a Ask,
    together: u64,
    /// The opening hit's notes.
    opening: Option<(usize, usize)>,
    /// Frames from the start to the opening hit.
    lead: u64,
    bars_for: &'a dyn Fn(usize) -> Option<u32>,
}

impl<E: Event> Ctx<'_, E> {
    /// Score the loop ending on frame `end`, with the hit `hit` there (None
    /// for the tap).
    fn score(&self, end: u64, hit: Option<(usize, usize)>) -> Candidate {
        let onset = hit.map_or(0.0, |h| self.onset(h));
        let rhythm = self.rhythm(end);
        let context = self.context(end);
        let dist = end.abs_diff(self.ask.tap) as f64;
        let prior = (1.0 - dist / self.ask.reach.max(1) as f64).max(0.0);
        let score = W_ONSET * onset + W_RHYTHM * rhythm + W_CONTEXT * context + W_PRIOR * prior;
        Candidate {
            end,
            onset,
            rhythm,
            context,
            prior,
            score,
        }
    }

    /// The hit `h` against the opening: the best of its notes that the
    /// opening played too (by how alike they are: the same pad or note,
    /// or a sound of the same tone), by how loud that was in the opening
    /// (its accent) and how alike the two levels are.
    fn onset(&self, h: (usize, usize)) -> f64 {
        let Some(o) = self.opening else {
            return 0.0;
        };
        let open = &self.notes[o.0..o.1];
        let loudest = open
            .iter()
            .map(|x| x.level())
            .fold(0.0, f64::max)
            .max(1.0 / 127.0);
        let mut best: f64 = 0.0;
        for x in &self.notes[h.0..h.1] {
            for y in open {
                let same = x.alike(y);
                if same <= 0.0 {
                    continue;
                }
                let accent = y.level() / loudest;
                let alike = 1.0 - (x.level() - y.level()).abs();
                best = best.max(same * accent * (0.5 + 0.5 * alike));
            }
        }
        best
    }

    /// The loop's notes after the opening hit up to the reach, against a
    /// beat grid from the opening of the length `end` gives: each note
    /// scores how near a beat it falls (a bell a twentieth of a beat wide,
    /// 10 to 30 ms), weighted by its accent. The same notes for every end,
    /// so the scores compare. The opening's own notes sit on the grid
    /// whatever the length, so they don't count. 0 when there are none.
    fn rhythm(&self, end: u64) -> f64 {
        let len = end.saturating_sub(self.ask.start + self.lead) as usize;
        let Some(bars) = (self.bars_for)(len).filter(|&b| b > 0) else {
            return 0.0;
        };
        let beats = self.ask.bar as f64 / self.ask.beat.max(1) as f64;
        let beat = len as f64 / (f64::from(bars) * beats);
        if !(beat.is_finite() && beat > 1.0) {
            return 0.0;
        }
        let rate = f64::from(self.ask.rate);
        let sigma = (beat * 0.05).clamp(0.010 * rate, 0.030 * rate);
        let from = self.ask.start + self.lead;
        let to = self.ask.tap.saturating_sub(self.ask.reach);
        let (mut sum, mut wsum) = (0.0, 0.0);
        let after = self.opening.map_or(0, |o| o.1);
        for x in self.notes[after..]
            .iter()
            .filter(|x| x.frame() >= from && x.frame() < to)
        {
            let t = (x.frame() - from) as f64;
            let off = t - (t / beat).round() * beat;
            let w = x.weight();
            sum += w * (-(off / sigma) * (off / sigma)).exp();
            wsum += w;
        }
        if wsum > 0.0 {
            sum / wsum
        } else {
            0.0
        }
    }

    /// Whether the notes after the opening hit come round again after the
    /// hit on frame `at` (the loop's end plus the lead): each such note
    /// heard by now that has a match within [`CONTEXT_MS`], by accent. 0
    /// when nothing can be compared yet.
    fn context(&self, at: u64) -> f64 {
        let Some(o) = self.opening else {
            return 0.0;
        };
        let open_at = self.notes[o.0].frame();
        let window = self.ask.heard_to.saturating_sub(at);
        let tol = frames(self.ask.rate, CONTEXT_MS);
        let (mut got, mut wsum) = (0.0, 0.0);
        for x in &self.notes[o.1..] {
            let r = x.frame() - open_at;
            if r <= self.together {
                continue;
            }
            if r + tol > window || x.frame() >= self.ask.tap.saturating_sub(self.ask.reach) {
                break;
            }
            let want = at + r;
            let lo = want.saturating_sub(tol);
            let i = self.notes.partition_point(|y| y.frame() < lo);
            let hit = self.notes[i..]
                .iter()
                .take_while(|y| y.frame() <= want + tol)
                .any(|y| y.alike(x) >= ALIKE);
            let w = x.weight();
            wsum += w;
            if hit {
                got += w;
            }
        }
        if wsum > 0.0 {
            got / wsum
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const BAR: Tick = 3840;
    const BEAT: Tick = 960;

    fn ms(x: f64) -> u64 {
        frames(RATE, x)
    }

    fn note(at_ms: f64, key: u8, velocity: u8) -> NoteStart {
        NoteStart {
            frame: 10_000 + ms(at_ms),
            source: 0,
            key,
            velocity,
        }
    }

    /// A beat played at `bpm` for `bars` bars of 4/4 from 0 ms, plus the
    /// next downbeat: kick (36) on 1 and 3, snare (38) on 2 and 4, hats
    /// (42) on the 8ths, each note `late_ms` off where `jitter` puts it.
    fn beat(bpm: f64, bars: u32, extra: f64, jitter: &dyn Fn(usize) -> f64) -> Vec<NoteStart> {
        let eighth = 30_000.0 / bpm;
        let mut v = Vec::new();
        let n = (bars * 8) as usize;
        for k in 0..=n {
            let t = k as f64 * eighth + jitter(k);
            if k % 4 == 0 {
                v.push(note(t, 36, 120));
            } else if k % 2 == 0 {
                v.push(note(t + 15.0, 38, 110)); // laid back
            }
            v.push(note(t, 42, 60));
        }
        // Keep playing past the downbeat for `extra` ms.
        let mut k = n + 1;
        while (k - n) as f64 * eighth < extra {
            let t = k as f64 * eighth;
            if k.is_multiple_of(2) {
                v.push(note(t + 15.0, 38, 110));
            }
            v.push(note(t, 42, 60));
            k += 1;
        }
        v
    }

    fn log_of(notes: &[NoteStart]) -> NoteLog {
        let mut l = NoteLog::new();
        for &n in notes {
            l.push(n);
        }
        l
    }

    fn bars_at(len: usize) -> Option<u32> {
        crate::read_bars(len, RATE, BAR, None, None).map(|r| r.bars)
    }

    fn ask(tap_ms: f64, heard_ms: f64, bpm: f64) -> Ask {
        Ask {
            rate: RATE,
            start: 10_000,
            tap: 10_000 + ms(tap_ms),
            reach: reach(RATE, bpm),
            heard_to: 10_000 + ms(heard_ms),
            bar: BAR,
            beat: BEAT,
        }
    }

    #[test]
    fn a_late_tap_moves_to_the_downbeat_you_played() {
        // Two bars at 120 BPM: the downbeat comes round at 4000 ms. The
        // tap came 60 ms late; Auto heard to the end of the reach.
        let notes = beat(120.0, 2, 300.0, &|_| 0.0);
        let a = ask(4060.0, 4160.0, 120.0);
        let d = decide(&log_of(&notes), &a, &bars_at);
        assert_eq!(d.verdict, Verdict::Moved, "{d:?}");
        assert_eq!(d.end, 10_000 + ms(4000.0));
        assert_eq!(d.moved_by(a.tap), -(ms(60.0) as i64));
        let b = d.best.unwrap();
        assert!(b.onset > 0.9, "{b:?}");
        assert!(b.rhythm > d.tap.rhythm + 0.2, "{d:?}");
    }

    #[test]
    fn an_early_tap_moves_too_and_a_tap_on_time_stays_put() {
        let notes = beat(100.0, 1, 400.0, &|_| 0.0);
        // One bar at 100 BPM is 2400 ms; a tap 45 ms early.
        let d = decide(&log_of(&notes), &ask(2355.0, 2455.0, 100.0), &bars_at);
        assert_eq!((d.verdict, d.end), (Verdict::Moved, 10_000 + ms(2400.0)));
        // On time: the end is the tap's own frame, give or take nothing.
        let d = decide(&log_of(&notes), &ask(2400.0, 2500.0, 100.0), &bars_at);
        assert_eq!(d.end, 10_000 + ms(2400.0));
    }

    #[test]
    fn human_timing_and_stopping_at_the_tap_still_find_the_end() {
        // A few ms either way on every note, and nothing played after the
        // downbeat: the rhythm and the opening hit decide.
        let wobble = |k: usize| [3.0, -4.0, 6.0, -2.0, 0.0, 5.0, -6.0, 2.0][k % 8];
        let mut notes = beat(90.0, 2, 0.0, &wobble);
        notes.retain(|n| n.frame <= 10_000 + ms(5334.0 + 10.0));
        let d = decide(&log_of(&notes), &ask(5290.0, 5434.0, 90.0), &bars_at);
        assert_eq!(d.verdict, Verdict::Moved, "{d:?}");
        // The opening kick came 3 ms after the start and so did the
        // downbeat's: the loop is the bars between them, from the kick.
        let want = 10_000 + ms(16.0 * 30_000.0 / 90.0 + wobble(16));
        assert!(d.end.abs_diff(want) <= 1, "{} against {want}", d.end);
        assert_eq!(d.lead, ms(wobble(0)));
    }

    #[test]
    fn no_note_near_the_tap_or_no_clear_winner_keeps_the_tap() {
        // Stopped playing well before the tap: nothing in the reach.
        let mut notes = beat(120.0, 2, 0.0, &|_| 0.0);
        notes.retain(|n| n.frame < 10_000 + ms(3700.0));
        let d = decide(&log_of(&notes), &ask(4000.0, 4100.0, 120.0), &bars_at);
        assert_eq!((d.verdict, d.end), (Verdict::NoMatch, 10_000 + ms(4000.0)));
        // Two kicks of the same strength 50 ms apart either side of the
        // tap, nothing else after: neither is clearly the one.
        let mut notes: Vec<NoteStart> = (0..8).map(|k| note(k as f64 * 500.0, 36, 120)).collect();
        notes.push(note(3975.0, 36, 120));
        notes.push(note(4025.0, 36, 120));
        let d = decide(&log_of(&notes), &ask(4000.0, 4100.0, 120.0), &bars_at);
        assert_eq!((d.verdict, d.end), (Verdict::NotClear, 10_000 + ms(4000.0)));
        assert_eq!(Verdict::NotClear.why(), "no clear match near it");
    }

    #[test]
    fn the_opening_hit_played_again_is_enough_on_its_own() {
        // One kick, nothing else, then the same kick 2 s later and a tap
        // 60 ms after it: no rhythm to go on, but the hit is the opening
        // played again.
        let notes = [note(0.0, 36, 120), note(2000.0, 36, 120)];
        let d = decide(&log_of(&notes), &ask(2060.0, 2160.0, 120.0), &bars_at);
        assert_eq!(
            (d.verdict, d.end),
            (Verdict::Moved, 10_000 + ms(2000.0)),
            "{d:?}"
        );
        assert_eq!(
            d.best.unwrap().rhythm,
            0.0,
            "the opening itself isn't rhythm"
        );
        // Much softer, it's no longer clear on its own.
        let notes = [note(0.0, 36, 120), note(2000.0, 36, 30)];
        let d = decide(&log_of(&notes), &ask(2060.0, 2160.0, 120.0), &bars_at);
        assert_eq!(d.verdict, Verdict::NotClear, "{d:?}");
    }

    #[test]
    fn notes_lost_before_the_log_near_the_tap_keep_it() {
        let notes = beat(120.0, 2, 300.0, &|_| 0.0);
        let mut log = log_of(&notes);
        log.lost(10_000 + ms(3990.0));
        let d = decide(&log, &ask(4060.0, 4160.0, 120.0), &bars_at);
        assert_eq!((d.verdict, d.end), (Verdict::Lost, 10_000 + ms(4060.0)));
        assert_eq!(log.dropped(), 1);
        // One lost long before the reach doesn't matter.
        let mut log = log_of(&notes);
        log.lost(10_000 + ms(1000.0));
        let d = decide(&log, &ask(4060.0, 4160.0, 120.0), &bars_at);
        assert_eq!(d.verdict, Verdict::Moved);
    }

    #[test]
    fn a_soft_hat_near_the_tap_is_not_the_downbeat() {
        // The opening is a kick with a hat; near an on-time tap only an
        // off-beat hat sounds (a 16th at 160 BPM is 94 ms, inside the
        // 100 ms reach, so the reach is that 16th): the tap stands.
        let sixteenth = 15_000.0 / 160.0;
        let mut notes = Vec::new();
        for k in 0..8 {
            let t = k as f64 * sixteenth * 2.0;
            if k % 4 == 0 {
                notes.push(note(t, 36, 120));
            }
            notes.push(note(t, 42, 50));
        }
        // One bar ends at 1500 ms; the player stops, but one soft hat
        // comes a 16th after the last eighth, 47 ms before the end.
        notes.push(note(1500.0 - sixteenth / 2.0, 42, 45));
        let d = decide(&log_of(&notes), &ask(1500.0, 1594.0, 160.0), &bars_at);
        assert_ne!(d.verdict, Verdict::Moved, "{d:?}");
        assert_eq!(d.end, 10_000 + ms(1500.0));
    }

    #[test]
    fn a_loop_started_on_rec_before_its_first_hit_keeps_that_lead() {
        // REC at 0 ms, the first kick at 80 ms, one bar at 120 BPM: the
        // kick comes round at 2080 ms, where the loop from it ends. A tap
        // 50 ms late of it, and further than the reach from where the loop
        // from the press, as long, would end (2000 ms).
        let notes: Vec<NoteStart> = beat(120.0, 1, 200.0, &|_| 0.0)
            .into_iter()
            .map(|n| NoteStart {
                frame: n.frame + ms(80.0),
                ..n
            })
            .collect();
        let d = decide(&log_of(&notes), &ask(2130.0, 2230.0, 120.0), &bars_at);
        assert_eq!(
            (d.verdict, d.end, d.lead),
            (Verdict::Moved, 10_000 + ms(2080.0), ms(80.0))
        );
    }

    #[test]
    fn the_beat_inside_says_three_bars_where_the_length_alone_says_four() {
        use crate::reading::{read_bars, read_bars_with, Sure};
        // Three bars of 4/4 at 100 BPM, kick, snare and hats: 7.2 s, which
        // by its length alone reads as 4 bars at 133.
        let notes = beat(100.0, 3, 0.0, &|_| 0.0);
        let len = ms(7200.0) as usize;
        let inner = InnerBeat::new(&log_of(&notes), RATE, 10_000, len, BAR, BEAT);
        let fit = |b| inner.fit(b).unwrap();
        assert!(fit(3) > fit(4) + 0.3, "3: {}, 4: {}", fit(3), fit(4));
        assert!(fit(3) > fit(2) + 0.3, "3: {}, 2: {}", fit(3), fit(2));
        let r = read_bars_with(len, RATE, BAR, None, None, &|b| inner.fit(b)).unwrap();
        assert_eq!((r.bars, r.sure, r.other), (3, Sure::Clear, None), "{r:?}");
        assert!((r.bpm - 100.0).abs() < 0.01);
        assert_eq!(read_bars(len, RATE, BAR, None, None).unwrap().bars, 4);
        // The same notes over 4 bars' length at 133 don't make 3 bars.
        let len = ms(9600.0) as usize;
        let inner = InnerBeat::new(&log_of(&notes), RATE, 10_000, len, BAR, BEAT);
        let r = read_bars_with(len, RATE, BAR, None, None, &|b| inner.fit(b)).unwrap();
        assert_eq!(r.bars, 4, "{r:?}");
    }

    #[test]
    fn twice_the_bars_fits_about_as_well_so_the_preference_keeps_its_say() {
        use crate::reading::{read_bars, read_bars_with};
        // One bar at 100 BPM: its 8th-note hats are quarters of 2 bars at
        // 200. The beat leaves the reading as the length alone has it.
        let notes = beat(100.0, 1, 0.0, &|_| 0.0);
        let len = ms(2400.0) as usize;
        let inner = InnerBeat::new(&log_of(&notes), RATE, 10_000, len, BAR, BEAT);
        let r = read_bars_with(len, RATE, BAR, None, None, &|b| inner.fit(b)).unwrap();
        let plain = read_bars(len, RATE, BAR, None, None).unwrap();
        assert_eq!((r.bars, r.sure), (plain.bars, plain.sure));
        assert!(r.beat.unwrap() > 0.6, "{r:?}");
        assert_eq!(plain.beat, None);
        // Near the old edge (1 bar at 79 or 2 at 158) it stays a close
        // call: the beat can't tell those apart.
        let notes = beat(79.0, 1, 0.0, &|_| 0.0);
        let len = ms(4.0 * 60_000.0 / 79.0) as usize;
        let inner = InnerBeat::new(&log_of(&notes), RATE, 10_000, len, BAR, BEAT);
        let r = read_bars_with(len, RATE, BAR, None, None, &|b| inner.fit(b)).unwrap();
        assert_eq!(r.sure, crate::reading::Sure::Ambiguous, "{r:?}");
    }

    #[test]
    fn a_few_notes_are_no_evidence() {
        let notes: Vec<NoteStart> = beat(100.0, 1, 0.0, &|_| 0.0)
            .into_iter()
            .filter(|n| n.key == 36)
            .collect();
        // The opening kick, then only two more inside the bar.
        let inner = InnerBeat::new(
            &log_of(&notes),
            RATE,
            10_000,
            ms(2400.0) as usize,
            BAR,
            BEAT,
        );
        assert_eq!(inner.fit(1), None);
    }

    #[test]
    fn the_log_keeps_the_start_and_the_end_and_counts_what_it_drops() {
        let mut l = NoteLog::new();
        for k in 0..(HEAD + TAIL + 10) as u64 {
            l.push(NoteStart {
                frame: k * 100,
                ..NoteStart::NONE
            });
        }
        assert_eq!(l.len(), HEAD + TAIL);
        assert_eq!(l.dropped(), 10);
        let mut out = [NoteStart::NONE; HEAD + TAIL];
        let n = l.sorted(&mut out);
        assert_eq!(out[0].frame, 0);
        assert_eq!(out[HEAD].frame, (HEAD as u64 + 10) * 100);
        assert_eq!(out[n - 1].frame, (HEAD + TAIL + 9) as u64 * 100);
        assert!(!l.whole_from(HEAD as u64 * 100 + 900));
        assert!(l.whole_from(HEAD as u64 * 100 + 1000));
        // Out of order within a period: sorted on the way out.
        let mut l = NoteLog::new();
        for f in [300, 100, 200] {
            l.push(NoteStart {
                frame: f,
                ..NoteStart::NONE
            });
        }
        let n = l.sorted(&mut out);
        assert_eq!(
            out[..n].iter().map(|x| x.frame).collect::<Vec<_>>(),
            [100, 200, 300]
        );
        // A pass starting at 150 keeps what came from there on.
        l.keep_from(150);
        let n = l.sorted(&mut out);
        assert_eq!(
            out[..n].iter().map(|x| x.frame).collect::<Vec<_>>(),
            [200, 300]
        );
        l.clear();
        assert!(l.is_empty() && l.whole_from(0));
    }

    #[test]
    fn the_reach_is_100_ms_or_a_sixteenth() {
        assert_eq!(reach(RATE, 120.0), ms(100.0));
        assert_eq!(reach(RATE, 150.0), ms(100.0));
        assert_eq!(reach(RATE, 200.0), ms(75.0));
        assert_eq!(reach(RATE, f64::NAN), ms(100.0));
    }
}

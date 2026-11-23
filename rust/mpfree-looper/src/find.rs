//! The finder: where a free first loop by ear really starts and ends
//! (the smart loop end plan, step 3).
//!
//! On the line input or a mix the looper hears hits ([`crate::hits`]), not
//! notes. A decision on them needs more than a period can spare, so the
//! looper asks the finder, on a thread of its own, through one job that
//! goes back and forth ([`FindJob`]): the take's generation, what the
//! decision is asked ([`Ask`]), its hits, and its audio around the
//! opening and around the reach of the tap, copied once when asked
//! ([`crate::Looper::find_job`]). The answer comes back in the same job
//! ([`crate::Looper::found`]); one for a take that has gone (a newer
//! generation) is dropped.
//!
//! [`find`] works coarse to fine:
//!
//! 1. **Rank.** The hits are scored as notes are ([`decide`]): onset (a
//!    hit about as loud with the opening's tone), rhythm, context and the
//!    tap's prior, with the same threshold and margin. Their frames are
//!    coarse, within a step of the hit track.
//! 2. **The start.** The loop's opening hit is placed on its first sample
//!    at or over [`crate::PLAY_LEVEL`] (-40 dB) near where the hit track
//!    heard it, as a loop started when you play by ear is.
//! 3. **The end.** The best candidate, and the runner-up when it is a hit
//!    too, is placed where its sound lines up with the opening's: the
//!    frame where the audio from there best matches the audio from the
//!    start ([`line_up`], every [`STEP_FINE`] frames near the coarse hit
//!    after every [`STEP_COARSE`]). So both ends are measured the same
//!    way, on the sound itself. If the runner-up lines up clearly better
//!    than the best, or the best hardly at all, the tap stands
//!    ([`Verdict::NotClear`]).
//!
//! Nothing here allocates once the job is made ([`FindJob::new`], off the
//! audio thread), so the finder could run anywhere; it runs on its own
//! thread so that a long job elsewhere can't make Auto miss its moment.

use crate::ends::{decide, Ask, Candidate, Decision, Event, Verdict};
use crate::hits::{HitLog, HOP};
use crate::PLAY_LEVEL;

/// Frames before a hit that a line-up compares.
pub const PRE: usize = 64;
/// Frames a line-up compares, at most (46 ms at 44.1 kHz).
pub const WIDTH: usize = 2048;
/// The fewest it compares: a candidate heard for less than this after
/// it keeps its coarse frame.
pub const MIN_WIDTH: usize = 256;
/// How far either side of where the hit track heard it an end is looked
/// for (two steps: the hit track's error at each end).
pub const SEARCH: usize = 2 * HOP;
/// A line-up is tried every this many frames first, then at every frame
/// near the best.
pub const STEP_COARSE: usize = 4;
pub const STEP_FINE: usize = 1;
/// How well the best candidate must line up with the opening (0 to 1, a
/// normalized correlation) to move the end.
pub const MIN_LINE_UP: f32 = 0.5;
/// A runner-up lining up better than the best by this much leaves the
/// end in doubt: the tap stands.
pub const LINE_UP_MARGIN: f32 = 0.2;
/// The opening's start is looked for from this far before where the hit
/// track heard it to this far after.
pub const START_BACK: usize = 2 * HOP;
pub const START_ON: usize = HOP;

/// Frames of audio kept around the opening: its start's search, the
/// line-up's frames before, and the frames it compares.
pub const OPEN_FRAMES: usize = START_BACK + PRE + WIDTH + START_ON;
/// Frames kept around the reach beyond twice the reach itself.
pub const NEAR_EXTRA: usize = 2 * (SEARCH + PRE) + WIDTH;

/// One job for the finder, made once and passed back and forth.
#[derive(Clone, Debug)]
pub struct FindJob {
    /// The take it is for: an answer is used only while the looper still
    /// waits for this one.
    pub gen: u64,
    pub ask: Ask,
    /// The bar count the reading takes for the tap's length, for the
    /// rhythm part.
    pub bars: u32,
    pub hits: HitLog,
    /// Mono audio of the first pass from song frame `open_from`, around
    /// the opening hit; `open_len` frames of it heard.
    pub open_from: u64,
    pub open: Box<[f32]>,
    pub open_len: usize,
    /// Mono audio from song frame `near_from`, around the reach of the
    /// tap; `near_len` frames of it heard.
    pub near_from: u64,
    pub near: Box<[f32]>,
    pub near_len: usize,
    /// The finder's answer.
    pub answer: Option<Decision>,
    /// How well the end found lines up with the opening (0 to 1), for
    /// the log; 0 when it wasn't placed on the audio.
    pub line_up: f32,
}

impl FindJob {
    /// A job with room for a reach of up to `reach` frames either side of
    /// a tap ([`crate::ends::reach`] at `rate`). Allocates: call it off
    /// the audio thread.
    pub fn new(reach: u64) -> FindJob {
        FindJob {
            gen: 0,
            ask: Ask {
                rate: 0,
                start: 0,
                tap: 0,
                reach: 0,
                heard_to: 0,
                bar: 0,
                beat: 0,
            },
            bars: 0,
            hits: HitLog::new(),
            open_from: 0,
            open: vec![0.0; OPEN_FRAMES].into_boxed_slice(),
            open_len: 0,
            near_from: 0,
            near: vec![0.0; 2 * reach as usize + NEAR_EXTRA].into_boxed_slice(),
            near_len: 0,
            answer: None,
            line_up: 0.0,
        }
    }

    /// The song frame the opening's audio is kept from, for an opening
    /// hit the hit track heard on `at`.
    pub fn open_from_for(at: u64) -> u64 {
        at.saturating_sub((START_BACK + PRE) as u64)
    }

    /// The song frame the reach's audio is kept from, for a tap on `tap`
    /// with a reach of `reach`.
    pub fn near_from_for(tap: u64, reach: u64) -> u64 {
        tap.saturating_sub(reach + (SEARCH + PRE) as u64)
    }
}

/// Answer `job` (see the module notes): its decision into
/// [`FindJob::answer`]. Allocation-free.
pub fn find(job: &mut FindJob) {
    let ask = job.ask;
    let bars = job.bars;
    let mut d = decide(&job.hits, &ask, &|_| Some(bars));
    job.line_up = 0.0;
    // The opening hit, on its first sample over -40 dB.
    let open_at = ask.start + d.lead;
    let has_opening = first_hit(&job.hits, ask.start, ask.reach).is_some();
    let start = if has_opening {
        start_by_ear(job, open_at).unwrap_or(open_at)
    } else {
        open_at
    };
    if has_opening {
        d.lead = start.saturating_sub(ask.start);
    }
    // The end, lined up with the opening's sound.
    if d.verdict == Verdict::Moved {
        let best = d.best.map(|c| c.end).unwrap_or(d.end);
        // Where the start's sound is, against where the hit track heard
        // the opening: the end's sound is as far from its hit.
        let shift = start as i64 - open_at as i64;
        match line_up(job, start, best as i64 + shift) {
            Some((end, fit)) => {
                job.line_up = fit;
                let other = d
                    .second
                    .filter(|c| c.end != ask.tap && c.onset > 0.0)
                    .and_then(|c| line_up(job, start, c.end as i64 + shift));
                if fit < MIN_LINE_UP || other.is_some_and(|(_, o)| o > fit + LINE_UP_MARGIN) {
                    d.verdict = Verdict::NotClear;
                    d.end = ask.tap;
                } else {
                    d.end = end;
                    if let Some(b) = d.best.as_mut() {
                        b.end = end;
                    }
                }
            }
            // Too little heard after it to compare: the hit track's frame,
            // moved as the start was.
            None => {
                d.end = (best as i64 + shift).max(0) as u64;
                if let Some(b) = d.best.as_mut() {
                    b.end = d.end;
                }
            }
        }
    }
    job.answer = Some(d);
}

/// The first hit within `reach` of `start` (the opening), if any.
fn first_hit(hits: &HitLog, start: u64, reach: u64) -> Option<u64> {
    let mut buf = [crate::hits::Hit::NONE; crate::ends::HEAD + crate::ends::TAIL];
    let n = hits.sorted(&mut buf);
    buf[..n]
        .iter()
        .map(|h| h.frame())
        .find(|&f| f >= start && f <= start + reach)
}

/// The first frame at or over [`PLAY_LEVEL`] from [`START_BACK`] before
/// `at` to [`START_ON`] after it, in the opening's audio.
fn start_by_ear(job: &FindJob, at: u64) -> Option<u64> {
    let from = at.saturating_sub(START_BACK as u64);
    let to = at + START_ON as u64;
    (from..=to).find(|&f| {
        f.checked_sub(job.open_from)
            .map(|i| i as usize)
            .filter(|&i| i < job.open_len)
            .is_some_and(|i| job.open[i].abs() >= PLAY_LEVEL)
    })
}

/// Where the sound from the start comes round near frame `near`: the
/// frame within [`SEARCH`] of it where the audio from [`PRE`] before
/// best matches the opening's from [`PRE`] before `start` (a normalized
/// correlation over up to [`WIDTH`] frames, every [`STEP_COARSE`] then
/// every [`STEP_FINE`] near the best), and how well (0 to 1). None when
/// fewer than [`MIN_WIDTH`] frames can be compared.
pub fn line_up(job: &FindJob, start: u64, near: i64) -> Option<(u64, f32)> {
    let o = start.checked_sub(PRE as u64)?.checked_sub(job.open_from)? as usize;
    let open_have = job.open_len.checked_sub(o)?;
    let lo = near - SEARCH as i64;
    let hi = near + SEARCH as i64;
    // Frames heard after the latest place tried bound the width.
    let last = (hi - PRE as i64) - job.near_from as i64;
    let near_have = (job.near_len as i64 - last).max(0) as usize;
    let width = WIDTH.min(open_have).min(near_have);
    if width < MIN_WIDTH || lo - (PRE as i64) < job.near_from as i64 {
        return None;
    }
    let a = &job.open[o..o + width];
    let aa: f32 = a.iter().map(|x| x * x).sum();
    if aa <= 1e-12 {
        return None;
    }
    let fit = |e: i64| -> f32 {
        let b0 = (e - PRE as i64 - job.near_from as i64) as usize;
        let b = &job.near[b0..b0 + width];
        let (mut ab, mut bb) = (0.0f32, 0.0f32);
        for (x, y) in a.iter().zip(b) {
            ab += x * y;
            bb += y * y;
        }
        if bb <= 1e-12 {
            0.0
        } else {
            ab / (aa * bb).sqrt()
        }
    };
    let mut best = (near, f32::MIN);
    let mut e = lo;
    while e <= hi {
        let f = fit(e);
        if f > best.1 {
            best = (e, f);
        }
        e += STEP_COARSE as i64;
    }
    let (c, _) = best;
    for e in (c - STEP_COARSE as i64 + 1).max(lo)..=(c + STEP_COARSE as i64 - 1).min(hi) {
        if e % STEP_FINE as i64 == 0 {
            let f = fit(e);
            if f > best.1 {
                best = (e, f);
            }
        }
    }
    Some((best.0.max(0) as u64, best.1.max(0.0)))
}

/// A decision the finder didn't make in time: the tap stands.
pub fn late(ask: &Ask) -> Decision {
    let tap = Candidate {
        end: ask.tap,
        onset: 0.0,
        rhythm: 0.0,
        context: 0.0,
        prior: 1.0,
        score: 0.0,
    };
    Decision {
        verdict: Verdict::Late,
        end: ask.tap,
        best: None,
        second: None,
        tap,
        lead: 0,
    }
}

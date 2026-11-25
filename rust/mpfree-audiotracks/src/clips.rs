//! Audio clips as the engine plays them (phase 2, step 2b).
//!
//! One player decides when every track switches, the arranger's
//! (`mpfree-play`). Each period it says, a track at a time, what the
//! track plays at the period's start and up to [`MAX_CHANGES`] changes in
//! the period, each on its frame ([`Answer`]). The engine plays that
//! through the read path song regions use ([`AudioTracks::process_with`]);
//! it has no clip table and no scheduler of its own.
//!
//! - **Voices.** A clip plays in a voice, which copies the clip's regions
//!   when it starts ([`MAX_CLIP_REGIONS`], into room it has from the
//!   start) and never reads the answer's again. So a clip edited or
//!   deleted while it fades out plays on from its own copy. A track has
//!   [`VOICES`] voices, so launches closer together than a fade still
//!   fade; a fifth hands the quietest voice's last sample to the track's
//!   declick, which ramps it to nothing over [`DECLICK`] frames, and is
//!   counted ([`crate::Stats::declicked`]).
//! - **Passes.** A clip's passes count from its origin, `len` ticks each,
//!   on song ticks: each pass plays its regions from the pass's start, as
//!   song regions would there, so they follow the tempo map the same way
//!   (frames from memory or the take's file, gain, fades, missed frames).
//! - **Joins.** Inside a pass, regions meet as song regions do. A pass
//!   meets the next as a loop: audio running straight on joins straight
//!   (or ramps a gain change); a recording that ends right at the pass end
//!   and starts again (a loop file cut to length) joins straight; any
//!   other audio at the seam fades with equal gain, because the audio past
//!   a loop's end is often its start again. A launch, a stop or a scene
//!   crossfades with equal power, except a switch onto the same audio
//!   running on (a legato switch to the same clip), which isn't faded.
//! - **Edits while playing.** A new stamp brings the clip's new regions,
//!   length and last frame at the voice's next pass start, and its gain at
//!   once with a [`FADE`]-frame ramp.
//! - **Rings.** A change marked to ring (a once clip's end) leaves what
//!   is sounding to play on: no new pass or region starts, and the last
//!   region plays to the clip's last frame. Any later change ends the ring
//!   with the usual fade.
//! - **Song mode.** A track playing its song plays its regions; a switch
//!   to or from a clip fades them out or in on the change's frame.
//! - **Jumps.** A loop jump or a locate restarts the voices where the song
//!   lands, the sound before the jump going to the declick.
//! - **Mutes.** The answer's mute, at the start and at each change, ramps
//!   what the track plays (not its input) over [`FADE`] frames from its
//!   frame. The track's own Mute and Solo are as before.
//!
//! Everything here happens on a frame and is the same for any period
//! size, and nothing allocates.

use crate::{Envelope, FollowTempo, Join, Region, FADE};
use mpfree_transport::time::{ticks_in, TempoMap, Tick};

pub use crate::RecordingId;

/// Regions an audio clip holds at most (a budget for the MPC; the save
/// writes a line a region, so raising it changes no file).
pub const MAX_CLIP_REGIONS: usize = 16;
/// Voices a track has: a clip playing, and up to three fading out.
pub const VOICES: usize = 4;
/// Frames the declick ramps a voice's last sample to nothing over.
pub const DECLICK: usize = 32;
/// Changes a track has at most in a period; the player keeps the last
/// ones and counts the rest as superseded.
pub const MAX_CHANGES: usize = 4;

/// One region of a clip: clip ticks `start..end` play recording `rec`
/// from frame `src` at its recorded rate (or following the tempo, phase
/// 5). A song region's fields, on the clip's ticks, naming the recording
/// by its lasting id. Each region has its own Follow tempo (`follow` and
/// `keep_pitch`, as a song region's), and its own made-at tempo in `bpm`:
/// 0 for its recording's ([`TakeInfo::bpm`](crate::TakeInfo), none for
/// a take whose tempo changed as it recorded), else [`Region::bpm`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipRegion {
    pub start: Tick,
    pub end: Tick,
    pub rec: RecordingId,
    pub src: i64,
    pub phase: f32,
    pub gain: f32,
    pub fade_in: u32,
    pub fade_out: u32,
    pub into: u32,
    pub left: u32,
    pub follow: FollowTempo,
    pub keep_pitch: bool,
    pub bpm: f64,
}

/// A clip as a track plays it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlay<'a> {
    /// Its regions, in order (the first [`MAX_CLIP_REGIONS`] play).
    pub regions: &'a [ClipRegion],
    /// The song tick its passes count from.
    pub origin: f64,
    /// Its pass length in ticks.
    pub len: Tick,
    /// The frame of its last region's recording a ring plays to.
    pub last_frame: i64,
    /// The clip's own gain, on top of each region's.
    pub gain: f32,
    pub clip: u32,
    /// Changes whenever anything in it does.
    pub stamp: u64,
}

/// A loop clip as a track plays it (phase 4, step 4b): the looper renders
/// it, and [`crate::loop_play`] says where and how loud.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopPlay {
    /// The loop's lasting id (`mpfree_seq::LoopId`'s).
    pub id: u128,
    /// The song tick its passes count from.
    pub origin: f64,
    /// Its pass length in ticks.
    pub len: Tick,
    pub clip: u32,
    /// Changes whenever the clip does.
    pub stamp: u64,
}

/// What a track plays.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Plays<'a> {
    Nothing,
    /// Its song regions.
    #[default]
    Song,
    Clip(ClipPlay<'a>),
    /// A loop clip: the looper's to render ([`crate::loop_play`]); the
    /// region player plays nothing for it.
    Loop(LoopPlay),
}

impl<'a> Plays<'a> {
    /// What the region player plays for it: nothing for a loop.
    pub(crate) fn for_regions(self) -> Plays<'a> {
        match self {
            Plays::Loop(_) => Plays::Nothing,
            p => p,
        }
    }
}

/// What a track plays from frame `frame` of the period on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change<'a> {
    pub frame: u32,
    /// The song tick it lands on: the line, a whole tick, where the
    /// frame's own tick may be half a frame either side. Which pass and
    /// region a clip is in there, and whether a block's last region ends
    /// right there, go by it; the sound is placed by the period's spans.
    pub tick: f64,
    pub plays: Plays<'a>,
    pub muted: bool,
    /// What was sounding plays on to its end ([`Plays`] still starts).
    pub ring: bool,
}

/// What a track plays in a period: at its start, and from each change
/// on, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Answer<'a> {
    pub start: Plays<'a>,
    pub muted: bool,
    pub changes: &'a [Change<'a>],
    /// Changes the player dropped this period (more than
    /// [`MAX_CHANGES`]): counted ([`crate::Stats::superseded`]).
    pub superseded: u32,
}

/// The gain of edge `k` frames into a [`FADE`] for a join: rising with
/// `k` (a fade out reads it backwards).
pub(crate) fn edge(j: Join, k: usize, fade: &[f32; FADE + 1]) -> f32 {
    match j {
        Join::Fade => fade[k],
        Join::Ramp => k as f32 / FADE as f32,
        Join::Straight => 1.0,
    }
}

fn rise(j: Join) -> u64 {
    if j == Join::Straight {
        0
    } else {
        FADE as u64
    }
}

/// The frames a sound is heard in: from frame `from`, opening with
/// `open`, until frame `to`, closing with `close` (absolute frames).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Window {
    pub from: u64,
    pub open: Join,
    pub to: u64,
    pub close: Join,
}

impl Window {
    pub(crate) const OPEN: Window = Window {
        from: 0,
        open: Join::Straight,
        to: u64::MAX,
        close: Join::Straight,
    };
    pub(crate) const SHUT: Window = Window {
        from: u64::MAX,
        open: Join::Straight,
        to: u64::MAX,
        close: Join::Straight,
    };

    pub(crate) fn opening(from: u64, open: Join) -> Window {
        Window {
            from,
            open,
            to: u64::MAX,
            close: Join::Straight,
        }
    }

    /// Closes from frame `to` with `close`, unless it closes sooner.
    pub(crate) fn close_at(&mut self, to: u64, close: Join) {
        if to < self.to {
            self.to = to.max(self.from);
            self.close = close;
        }
    }

    pub(crate) fn closing(&self) -> bool {
        self.to != u64::MAX
    }

    pub(crate) fn at(&self, f: u64, fade: &[f32; FADE + 1]) -> f32 {
        if f < self.from {
            return 0.0;
        }
        let mut g = 1.0;
        let k = f - self.from;
        if k < FADE as u64 {
            g *= edge(self.open, k as usize, fade);
        }
        if f >= self.to {
            let k = f - self.to;
            if k >= rise(self.close) {
                return 0.0;
            }
            g *= edge(self.close, FADE - k as usize, fade);
        }
        g
    }

    /// All `len` frames from `f0` fully heard.
    pub(crate) fn plain(&self, f0: u64, len: usize) -> bool {
        f0 >= self.from.saturating_add(rise(self.open)) && f0 + len as u64 <= self.to
    }

    /// None of the `len` frames from `f0` heard.
    pub(crate) fn silent(&self, f0: u64, len: usize) -> bool {
        f0 + len as u64 <= self.from || self.over(f0)
    }

    /// Closed for good by frame `f`.
    pub(crate) fn over(&self, f: u64) -> bool {
        f >= self.over_from()
    }

    /// The first frame it is closed for good from (never, while open).
    pub(crate) fn over_from(&self) -> u64 {
        self.to.saturating_add(rise(self.close))
    }
}

/// A gain that moves to a new value over [`FADE`] frames from frame `at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glide {
    from: f32,
    to: f32,
    at: u64,
}

impl Glide {
    pub(crate) const fn steady(g: f32) -> Glide {
        Glide {
            from: g,
            to: g,
            at: 0,
        }
    }

    pub(crate) fn at(&self, f: u64) -> f32 {
        if f <= self.at {
            self.from
        } else if f >= self.at + FADE as u64 {
            self.to
        } else {
            self.from + (self.to - self.from) * (f - self.at) as f32 / FADE as f32
        }
    }

    /// Goes to `g` from frame `f`.
    pub(crate) fn set(&mut self, g: f32, f: u64) {
        if g != self.to {
            self.from = self.at(f);
            self.to = g;
            self.at = f;
        }
    }

    /// Its gain over all frames from `f0`, if it doesn't move there.
    pub(crate) fn settled(&self, f0: u64) -> Option<f32> {
        (self.from == self.to || f0 >= self.at + FADE as u64).then_some(self.to)
    }
}

/// What a sound is heard through on top of its own gain: its window and
/// a gliding gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    pub win: Window,
    pub glide: Glide,
}

impl Shape {
    pub(crate) const OPEN: Shape = Shape {
        win: Window::OPEN,
        glide: Glide::steady(1.0),
    };

    pub(crate) fn at(&self, f: u64, fade: &[f32; FADE + 1]) -> f32 {
        self.win.at(f, fade) * self.glide.at(f)
    }

    /// Its gain over the `len` frames from `f0`, if it is one gain there.
    pub(crate) fn steady(&self, f0: u64, len: usize) -> Option<f32> {
        if self.win.plain(f0, len) {
            self.glide.settled(f0)
        } else if self.win.silent(f0, len) {
            Some(0.0)
        } else {
            None
        }
    }
}

/// Ticks a launch's tick may be off its line's by, as a span gives it
/// back from a frame.
const EPS: f64 = 1e-6;

/// Ticks a change's tick may be off a line by. Changes land on lines,
/// whole ticks (and blocks and their clips' regions start and end on
/// them); one made at a period's start has its frame's tick, within half
/// a frame of the line, and half a frame is well under half a tick.
pub(crate) const LINE: f64 = 0.5;

/// A take number no take has: a region whose recording isn't here plays
/// silence.
pub(crate) const NO_TAKE_ID: u16 = u16::MAX;

/// A clip's regions as a voice plays them: on its ticks, takes found.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Set {
    pub regions: [Region; MAX_CLIP_REGIONS],
    pub n: usize,
    pub origin: f64,
    pub len: Tick,
    pub last_frame: i64,
    /// The passes it plays: `first..=last`.
    pub first: i64,
    pub last: i64,
    pub stamp: u64,
}

impl Set {
    pub(crate) const EMPTY: Set = Set {
        regions: [Region::EMPTY; MAX_CLIP_REGIONS],
        n: 0,
        origin: 0.0,
        len: 1,
        last_frame: 0,
        first: 0,
        last: i64::MAX,
        stamp: 0,
    };

    /// The pass song tick `tick` falls in.
    pub(crate) fn pass_at(&self, tick: f64) -> i64 {
        ((tick - self.origin + EPS) / self.len as f64).floor() as i64
    }

    pub(crate) fn base(&self, p: i64) -> f64 {
        self.origin + p as f64 * self.len as f64
    }

    pub(crate) fn fill(&mut self, c: &ClipPlay, takes: &[(u16, f64); MAX_CLIP_REGIONS]) {
        self.n = c.regions.len().min(MAX_CLIP_REGIONS);
        for (i, r) in c.regions.iter().take(self.n).enumerate() {
            let (take, made_at) = takes[i];
            self.regions[i] = Region {
                start: r.start,
                end: r.end,
                take,
                src: r.src,
                phase: r.phase,
                gain: r.gain,
                fade_in: r.fade_in,
                fade_out: r.fade_out,
                into: r.into,
                left: r.left,
                follow: r.follow,
                keep_pitch: r.keep_pitch,
                bpm: if r.bpm == 0.0 { made_at } else { r.bpm },
            };
        }
        self.origin = c.origin;
        self.len = c.len.max(1);
        self.last_frame = c.last_frame;
        self.stamp = c.stamp;
        self.first = 0;
        self.last = i64::MAX;
    }

    pub(crate) fn uses(&self, id: u16) -> bool {
        self.regions[..self.n].iter().any(|r| r.take == id)
    }
}

/// One clip playing on a track, or fading out.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Voice {
    pub on: bool,
    pub clip: u32,
    /// The answer's origin it started with: a change with the same clip
    /// and origin carries it on (a mute, an edit); another origin is a
    /// launch.
    pub origin: f64,
    pub cur: Set,
    /// A new stamp's regions, from `cur`'s next pass start on.
    pub next: Option<Set>,
    pub shape: Shape,
    /// The song tick it started at (or restarted at after a jump).
    pub t_start: f64,
    /// Rings from song tick `ring_at`: nothing starts from there, and the
    /// last region plays to the last frame.
    pub ring: bool,
    pub ring_at: f64,
    /// The song block it played ends at song tick `end` (infinite until
    /// then): nothing of its from there on plays, and a region ending
    /// right there meets what follows with `end_join`, as the song's
    /// regions meet (phase 2, step 2d).
    pub end: f64,
    pub end_join: Join,
    /// Its track recorded over it: heard again, fading in, after.
    pub held: bool,
    /// A jump: it starts again where the song lands.
    pub restart: bool,
    /// Its last frame out (left, right), for the declick.
    pub last: [f32; 2],
    /// The frame it started on.
    pub since: u64,
}

impl Voice {
    pub(crate) const OFF: Voice = Voice {
        on: false,
        clip: 0,
        origin: 0.0,
        cur: Set::EMPTY,
        next: None,
        shape: Shape::OPEN,
        t_start: 0.0,
        ring: false,
        ring_at: 0.0,
        end: f64::INFINITY,
        end_join: Join::Fade,
        held: false,
        restart: false,
        last: [0.0; 2],
        since: 0,
    };

    /// Starts `c` on frame `f`, song tick `tick`, opening with `open`.
    pub(crate) fn start(
        &mut self,
        c: &ClipPlay,
        takes: &[(u16, f64); MAX_CLIP_REGIONS],
        f: u64,
        tick: f64,
        open: Join,
    ) {
        self.cur.fill(c, takes);
        self.cur.first = self.cur.pass_at(tick);
        self.next = None;
        self.on = true;
        self.clip = c.clip;
        self.origin = c.origin;
        self.shape = Shape {
            win: Window::opening(f, open),
            glide: Glide::steady(c.gain),
        };
        self.t_start = tick;
        self.ring = false;
        self.end = f64::INFINITY;
        self.end_join = Join::Fade;
        self.held = false;
        self.restart = false;
        self.last = [0.0; 2];
        self.since = f;
    }

    /// The newest stamp it has.
    pub(crate) fn stamp(&self) -> u64 {
        self.next.as_ref().map_or(self.cur.stamp, |s| s.stamp)
    }

    /// `c`'s new stamp from the next pass start after song tick `tick`;
    /// its gain at once, gliding from frame `f`.
    pub(crate) fn restamp(
        &mut self,
        c: &ClipPlay,
        takes: &[(u16, f64); MAX_CLIP_REGIONS],
        f: u64,
        tick: f64,
    ) {
        let k = self.cur.pass_at(tick).max(self.cur.first);
        self.cur.last = self.cur.last.min(k);
        let mut s = Set::EMPTY;
        s.fill(c, takes);
        s.origin = self.cur.base(k + 1);
        self.next = Some(s);
        self.shape.glide.set(c.gain, f);
    }

    /// Rings from song tick `tick`: the passes before it finish, nothing
    /// new starts.
    pub(crate) fn ring_from(&mut self, tick: f64) {
        // The last pass that started before the ring.
        let s = &mut self.cur;
        let p = ((tick - s.origin - EPS) / s.len as f64).ceil() as i64 - 1;
        s.last = s.last.min(p.max(s.first));
        self.next = None;
        self.ring = true;
        self.ring_at = tick;
    }

    /// Stops at song tick `tick`, where the song block it played ends:
    /// what starts there or later doesn't play, and a region ending right
    /// there meets what follows with `join` (it fades out past its end by
    /// itself, as a song region does).
    pub(crate) fn end_at(&mut self, tick: f64, join: Join) {
        self.end = tick;
        self.end_join = join;
    }

    /// One of its regions runs on through song tick `tick` (started
    /// before it, ends after it).
    pub(crate) fn runs_through(&self, tick: f64) -> bool {
        [Some(&self.cur), self.next.as_ref()]
            .into_iter()
            .flatten()
            .any(|s| {
                let p = ((tick - s.origin) / s.len as f64).floor() as i64;
                if p < s.first || p > s.last {
                    return false;
                }
                let t = tick - s.base(p);
                s.regions[..s.n]
                    .iter()
                    .any(|r| (r.start as f64) < t - LINE && t + LINE < r.end as f64)
            })
    }

    /// The audio it plays runs on through song tick `tick`: a region runs
    /// through it, or one ends right there and what it plays next is the
    /// same audio running on (the next part of the recording, or a
    /// recording that loops as made from its pass end into its start).
    /// `frames` gives a take's frames, if it is here.
    pub(crate) fn runs_on(
        &self,
        map: &TempoMap,
        rate: u32,
        tick: f64,
        frames: &dyn Fn(u16) -> Option<usize>,
    ) -> bool {
        if self.runs_through(tick) {
            return true;
        }
        let s = &self.cur;
        let p = s.pass_at(tick - LINE);
        if s.n == 0 || p < s.first || p > s.last {
            return false;
        }
        let base = s.base(p);
        let Some(i) = s.regions[..s.n]
            .iter()
            .position(|r| (base + r.end as f64 - tick).abs() < LINE)
        else {
            return false;
        };
        let a = &s.regions[i];
        let len_a = a.frames_to(map, base + a.start as f64, base + a.end as f64, rate);
        let b = if i + 1 < s.n {
            Some(&s.regions[i + 1]).filter(|b| b.start == a.end)
        } else if a.end != s.len {
            None
        } else if p < s.last {
            Some(&s.regions[0])
        } else {
            self.next
                .as_ref()
                .filter(|nx| nx.n > 0)
                .map(|nx| &nx.regions[0])
        };
        let Some(b) = b.filter(|b| b.start == 0 || i + 1 < s.n) else {
            return false;
        };
        let a_to = a.src + a.src_len(len_a, rate);
        b.take == a.take
            && a.take != NO_TAKE_ID
            && b.per_tick(rate) == a.per_tick(rate)
            && (b.src == a_to || (i + 1 == s.n && frames(a.take).is_some_and(|n| a_to == n as i64)))
    }

    pub(crate) fn uses(&self, id: u16) -> bool {
        self.on && (self.cur.uses(id) || self.next.as_ref().is_some_and(|s| s.uses(id)))
    }

    /// The region it plays at song tick `tick` (or the next to come) and
    /// the one after, for the stream layer's cursors: each with the song
    /// tick it starts at, the frames a ring plays it to (0 otherwise), and
    /// its cursor (regions in turn take turns).
    pub(crate) fn upcoming(&self, tick: f64, tail: f64) -> [Option<(Region, f64, i64, usize)>; 2] {
        let mut out = [None; 2];
        let s = &self.cur;
        if s.n == 0 {
            return out;
        }
        let mut k = 0;
        let from = s.pass_at(tick - tail).max(s.first);
        let to = s.last.min(from + 2);
        for p in from..=to {
            let base = s.base(p);
            for (i, r) in s.regions[..s.n].iter().enumerate() {
                let at = base + r.start as f64;
                if (self.ring && at >= self.ring_at) || at + LINE > self.end {
                    return out;
                }
                let ringing = self.ring && p == s.last && i == s.n - 1;
                if !ringing && base + r.end as f64 + tail <= tick {
                    continue;
                }
                let more = if ringing { s.last_frame - r.src } else { 0 };
                let slot = (p * s.n as i64 + i as i64).rem_euclid(2) as usize;
                out[k] = Some((*r, at, more, slot));
                k += 1;
                if k == 2 {
                    return out;
                }
            }
        }
        out
    }

    /// The take, frame and gain it plays at song tick `tick`, if any.
    pub(crate) fn heard(&self, map: &TempoMap, rate: u32, tick: f64) -> Option<(u16, i64, f32)> {
        let s = &self.cur;
        let p = s.pass_at(tick);
        if p < s.first || p > s.last || tick + LINE > self.end {
            return None;
        }
        let base = s.base(p);
        let t = tick - base;
        let r = s.regions[..s.n]
            .iter()
            .find(|r| r.start as f64 <= t && t < r.end as f64)?;
        let at = base + r.start as f64;
        let len_f = r.frames_to(map, at, base + r.end as f64, rate);
        let e = r.frames_to(map, at, tick, rate);
        let g = r.gain * Envelope::of(r, len_f).at(e) * self.shape.glide.to;
        Some((r.take, r.src + r.src_frames_to(map, at, tick, rate), g))
    }

    /// [`Voice::heard`] for the audio running into song tick `tick`: a
    /// region ending right there gives its last frame's place, as if it
    /// ran on (where [`Voice::heard`] finds the next region, or the next
    /// pass, or nothing past the voice's last pass). For a switch at a
    /// song block's end to a block whose clip plays on from that very
    /// frame (Song → scenes cuts a region at a section's edge into two
    /// clips: phase 2, step 2d).
    pub(crate) fn heard_into(
        &self,
        map: &TempoMap,
        rate: u32,
        tick: f64,
    ) -> Option<(u16, i64, f32)> {
        let s = &self.cur;
        // Half a tick before: the pass that runs into `tick`.
        let p = s.pass_at(tick - 0.5);
        if p < s.first || p > s.last || tick > self.end + LINE {
            return None;
        }
        let base = s.base(p);
        let t = tick - base;
        let r = s.regions[..s.n]
            .iter()
            .find(|r| (r.start as f64) < t && t <= r.end as f64)?;
        let at = base + r.start as f64;
        let len_f = r.frames_to(map, at, base + r.end as f64, rate);
        let e = r.frames_to(map, at, tick, rate);
        let g = r.gain * Envelope::of(r, len_f).at(e) * self.shape.glide.to;
        Some((r.take, r.src + r.src_frames_to(map, at, tick, rate), g))
    }
}

/// What a track plays now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mode {
    /// Not known (at first, and after the transport stops): the next
    /// answer's start applies as it is, with nothing to fade from.
    Unknown,
    Nothing,
    Song,
    Clip {
        clip: u32,
        voice: usize,
    },
}

/// A sample ramped to nothing over [`DECLICK`] frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Declick {
    v: [f32; 2],
    left: u32,
}

impl Declick {
    pub(crate) const NONE: Declick = Declick {
        v: [0.0; 2],
        left: 0,
    };

    /// Takes `s` over from the next frame, on top of what it still holds.
    pub(crate) fn hand(&mut self, s: [f32; 2]) {
        let k = self.left as f32 / DECLICK as f32;
        self.v = [self.v[0] * k + s[0], self.v[1] * k + s[1]];
        self.left = DECLICK as u32;
    }

    pub(crate) fn add(&mut self, dst: &mut [f32]) {
        for d in dst.chunks_exact_mut(2) {
            if self.left == 0 {
                break;
            }
            let g = self.left as f32 / DECLICK as f32;
            d[0] += self.v[0] * g;
            d[1] += self.v[1] * g;
            self.left -= 1;
        }
    }
}

/// A track's clip playing: its voices, its song's window, the answer's
/// mute and the declick.
#[derive(Clone, Debug)]
pub(crate) struct Lane {
    pub voices: [Voice; VOICES],
    pub song: Window,
    /// The song ticks its song plays between, where it switched to its
    /// song with nothing running through (from) and away from it (to):
    /// what ended by `song_from` or starts from `song_to` on isn't heard,
    /// and a region starting or ending right there fades by itself, as at
    /// a gap, its window open meanwhile (a block's edges: step 2d).
    pub song_from: f64,
    pub song_to: f64,
    pub mode: Mode,
    /// 1 heard, 0 muted by the answer.
    pub mute: Glide,
    pub declick: Declick,
}

impl Lane {
    pub(crate) fn new() -> Lane {
        Lane {
            voices: [Voice::OFF; VOICES],
            song: Window::OPEN,
            song_from: f64::NEG_INFINITY,
            song_to: f64::INFINITY,
            mode: Mode::Unknown,
            mute: Glide::steady(1.0),
            declick: Declick::NONE,
        }
    }

    /// The transport stopped: nothing sounds, and the next answer starts
    /// afresh.
    pub(crate) fn stop(&mut self) {
        for v in &mut self.voices {
            v.on = false;
        }
        self.mode = Mode::Unknown;
        self.declick = Declick::NONE;
        self.song = Window::OPEN;
        self.song_from = f64::NEG_INFINITY;
        self.song_to = f64::INFINITY;
    }

    /// A voice for a clip starting on frame `f`: a free one, else the
    /// quietest (the one furthest into its fade out, else the oldest),
    /// whose last sample goes to the declick. True if one was taken.
    pub(crate) fn free_voice(&mut self) -> (usize, bool) {
        if let Some(i) = self.voices.iter().position(|v| !v.on) {
            return (i, false);
        }
        let key = |v: &Voice| {
            if v.shape.win.closing() {
                (0, v.shape.win.to)
            } else {
                (1, v.since)
            }
        };
        let i = (0..VOICES)
            .min_by_key(|&i| key(&self.voices[i]))
            .unwrap_or(0);
        let last = self.voices[i].last;
        self.declick.hand(last);
        self.voices[i].on = false;
        (i, true)
    }

    pub(crate) fn uses(&self, id: u16) -> bool {
        self.voices.iter().any(|v| v.uses(id))
    }

    /// A loop jump or a locate: what sounded goes to the declick; a clip
    /// playing starts again where the song lands, and one fading out or
    /// ringing is over.
    pub(crate) fn jump(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.on) {
            self.declick.hand(v.last);
            v.last = [0.0; 2];
            if v.ring || v.shape.win.closing() {
                v.on = false;
            } else {
                v.restart = true;
            }
        }
        // The song plays on where it lands; one switched away from is over.
        self.song_from = f64::NEG_INFINITY;
        if self.song_to.is_finite() {
            self.song = Window::SHUT;
            self.song_to = f64::INFINITY;
        }
    }

    /// The track records over its clips (Replace): they go on silently.
    pub(crate) fn hold(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.on) {
            v.held = true;
            v.last = [0.0; 2];
        }
        self.declick = Declick::NONE;
    }

    /// Before a run from frame `f`, song tick `tick`: voices start again
    /// after a jump, and fade in again after a recording over them (one
    /// fading out then is over).
    pub(crate) fn resume(&mut self, f: u64, tick: f64) {
        for v in self.voices.iter_mut().filter(|v| v.on) {
            if v.restart {
                v.restart = false;
                if let Some(nx) = v.next.take() {
                    let origin = v.cur.origin;
                    v.cur = nx;
                    v.cur.origin = origin;
                }
                v.cur.last = i64::MAX;
                v.cur.first = v.cur.pass_at(tick);
                v.t_start = tick;
                v.shape.win = Window::opening(f, Join::Fade);
            }
            if v.held {
                v.held = false;
                if v.ring || v.shape.win.closing() {
                    v.on = false;
                } else {
                    v.shape.win.from = v.shape.win.from.max(f);
                    v.shape.win.open = Join::Fade;
                }
            }
        }
    }

    pub(crate) fn any_voice(&self) -> bool {
        self.voices.iter().any(|v| v.on)
    }
}

/// How the regions either side of a pass's end meet: `a` the last of a
/// pass, ending at it if `a_ends`, `b` the first of the next, starting at
/// it if `b_starts`; `a_frames` the frames of `a`'s recording.
#[allow(clippy::too_many_arguments)]
pub(crate) fn seam(
    a: &Region,
    len_a: i64,
    a_ends: bool,
    b: &Region,
    len_b: i64,
    b_starts: bool,
    a_frames: Option<usize>,
    rate: u32,
) -> Join {
    if !(a_ends && b_starts) {
        return Join::Fade;
    }
    // Where `a`'s recording ends as it plays.
    let a_to = a.src + a.src_len(len_a, rate);
    if b.take == a.take && b.src == a_to {
        // The same audio running straight on.
        return crate::join_run(a, len_a, b, len_b, rate);
    }
    if b.take == a.take && a_frames.is_some_and(|n| a_to == n as i64) {
        // A recording that ends right at the pass end loops as made.
        return Join::Straight;
    }
    Join::Ramp
}

impl crate::Reader<'_> {
    /// The copy region `i` of `set` plays in its pass from song tick
    /// `base`, on lane `owner` for a voice started on frame `since`
    /// ([`crate::Reader::source`]): decided as it is first heard, or only
    /// looked up.
    /// Inlined, so a region not on Stretch costs one test.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn clip_source(
        &mut self,
        owner: u16,
        since: u64,
        set: &Set,
        i: usize,
        base: f64,
        map: &TempoMap,
        rate: u32,
        f: u64,
        decide: bool,
    ) -> Option<usize> {
        if set.regions[i].follow != FollowTempo::Stretch {
            return None;
        }
        self.stretch_source(owner, since, set, i, base, map, rate, f, decide)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn stretch_source(
        &mut self,
        owner: u16,
        since: u64,
        set: &Set,
        i: usize,
        base: f64,
        map: &TempoMap,
        rate: u32,
        f: u64,
        decide: bool,
    ) -> Option<usize> {
        let r = &set.regions[i];
        let frames = self.frames_of(r.take);
        let (ends, reach) = crate::copies::clip_shape(
            &set.regions[..set.n],
            i,
            set.len,
            set.last_frame,
            frames,
            rate,
        );
        let at = base + r.start as f64;
        self.source(owner, since, r, at, ends, reach, map, rate, f, decide)
    }

    /// The frames take `id` has, if it is here.
    fn frames_of(&self, id: u16) -> Option<usize> {
        self.takes
            .get(id as usize)
            .filter(|t| t.live)
            .map(|t| t.frames)
    }

    /// Adds voice `v` into `dst` (a run's frames from frame `f0`, song
    /// tick `tick0`, at `bpm`): the regions of each of its passes the run
    /// hears, as song regions play. False once it has nothing more to
    /// play.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_voice(
        &mut self,
        v: &mut Voice,
        owner: u16,
        map: &TempoMap,
        rate: u32,
        bpm: f64,
        tick0: f64,
        f0: u64,
        dst: &mut [f32],
    ) -> bool {
        let len = dst.len() / 2;
        let fade_t = ticks_in(FADE as f64, bpm, rate) + 1.0;
        let tick1 = tick0 + ticks_in(len as f64, bpm, rate) + 1.0;
        // A new stamp takes over once the old passes are played out.
        if let Some(nx) = v.next {
            if tick0 - fade_t >= v.cur.base(v.cur.last + 1) {
                v.cur = nx;
                v.next = None;
            }
        }
        let placed = |r: &Region, base: f64| -> i64 {
            r.frames_to(map, base + r.start as f64, base + r.end as f64, rate)
        };
        let mut alive = false;
        let v = &*v;
        for which in 0..2 {
            let (set, prev, follow) = match (which, v.next.as_ref()) {
                (0, nx) => (&v.cur, None, nx),
                (_, Some(nx)) => (nx, Some(&v.cur), None),
                _ => break,
            };
            let n = set.n;
            if n == 0 {
                continue;
            }
            let lenp = set.len as f64;
            let hi = (((tick1 - set.origin) / lenp).floor() as i64).min(set.last);
            let mut lo = (((tick0 - fade_t - set.origin) / lenp).floor() as i64).max(set.first);
            if v.ring {
                // The last region rings on long past its pass.
                lo = lo.min(set.last);
            }
            if hi < lo {
                continue;
            }
            for p in lo.max(hi - 64)..=hi {
                let base = set.base(p);
                for i in 0..n {
                    let r = &set.regions[i];
                    let at = base + r.start as f64;
                    if at >= tick1 || (v.ring && at >= v.ring_at) || at + LINE > v.end {
                        break;
                    }
                    // Over before the voice started.
                    if base + r.end as f64 <= v.t_start + EPS {
                        continue;
                    }
                    let ringing = v.ring && p == set.last && i == n - 1;
                    let len0 = placed(r, base);
                    let len_f = if ringing {
                        // To the clip's last frame: its recording's frames,
                        // in the frames they play for.
                        let more = set.last_frame - r.src;
                        len0.max(match r.speed(bpm) {
                            Some(sp) => (more as f64 / sp).ceil() as i64,
                            None => more,
                        })
                    } else {
                        len0
                    };
                    if ringing {
                        let e0 = r.frames_to(map, at, tick0, rate);
                        if e0 >= len_f + FADE as i64 {
                            continue;
                        }
                    } else if base + r.end as f64 + fade_t <= tick0 {
                        continue;
                    }
                    alive = true;
                    let since = v.since;
                    let copy = self.clip_source(owner, since, set, i, base, map, rate, f0, true);
                    // Each join, then the fade if the two play from
                    // different audio (a copy: [`crate::copies`]).
                    let before = if i > 0 {
                        let a = &set.regions[i - 1];
                        let j = if a.end == r.start {
                            crate::join_run(a, placed(a, base), r, len0, rate)
                        } else {
                            Join::Fade
                        };
                        let sa =
                            self.clip_source(owner, since, set, i - 1, base, map, rate, f0, false);
                        crate::copies::join_sources(j, sa, copy)
                    } else if p > set.first {
                        let a = &set.regions[n - 1];
                        let len_a = placed(a, base - lenp);
                        let frames = self.frames_of(a.take);
                        let j = seam(
                            a,
                            len_a,
                            a.end == set.len,
                            r,
                            len0,
                            r.start == 0,
                            frames,
                            rate,
                        );
                        let sa = self.clip_source(
                            owner,
                            since,
                            set,
                            n - 1,
                            base - lenp,
                            map,
                            rate,
                            f0,
                            false,
                        );
                        crate::copies::join_sources(j, sa, copy)
                    } else if let Some(ps) = prev.filter(|ps| ps.n > 0) {
                        let a = &ps.regions[ps.n - 1];
                        let len_a = placed(a, ps.base(ps.last));
                        let frames = self.frames_of(a.take);
                        let j = seam(
                            a,
                            len_a,
                            a.end == ps.len,
                            r,
                            len0,
                            r.start == 0,
                            frames,
                            rate,
                        );
                        let pb = ps.base(ps.last);
                        let sa =
                            self.clip_source(owner, since, ps, ps.n - 1, pb, map, rate, f0, false);
                        crate::copies::join_sources(j, sa, copy)
                    } else if at <= v.t_start + EPS {
                        // It started in this region: the voice's window
                        // fades it in.
                        Join::Straight
                    } else {
                        Join::Fade
                    };
                    let frames = self.frames_of(r.take);
                    let after = if ringing {
                        Join::Fade
                    } else if (base + r.end as f64 - v.end).abs() < LINE {
                        // The end of its song block.
                        v.end_join
                    } else if i + 1 < n {
                        let b = &set.regions[i + 1];
                        let j = if r.end == b.start {
                            crate::join_run(r, len0, b, placed(b, base), rate)
                        } else {
                            Join::Fade
                        };
                        // The next is decided if it is heard this run.
                        let heard = base + (b.start as f64) < tick1;
                        let sb =
                            self.clip_source(owner, since, set, i + 1, base, map, rate, f0, heard);
                        crate::copies::join_sources(j, copy, sb)
                    } else if p < set.last {
                        let b = &set.regions[0];
                        let len_b = placed(b, base + lenp);
                        let j = seam(
                            r,
                            len0,
                            r.end == set.len,
                            b,
                            len_b,
                            b.start == 0,
                            frames,
                            rate,
                        );
                        let nb = base + lenp;
                        let heard = nb + (b.start as f64) < tick1;
                        let sb = self.clip_source(owner, since, set, 0, nb, map, rate, f0, heard);
                        crate::copies::join_sources(j, copy, sb)
                    } else if let Some(nx) = follow.filter(|nx| nx.n > 0) {
                        let b = &nx.regions[0];
                        let len_b = placed(b, nx.origin);
                        let j = seam(
                            r,
                            len0,
                            r.end == set.len,
                            b,
                            len_b,
                            b.start == 0,
                            frames,
                            rate,
                        );
                        let heard = nx.origin + (b.start as f64) < tick1;
                        let sb =
                            self.clip_source(owner, since, nx, 0, nx.origin, map, rate, f0, heard);
                        crate::copies::join_sources(j, copy, sb)
                    } else {
                        Join::Fade
                    };
                    let pl = crate::Placed {
                        r,
                        at,
                        len_f,
                        before,
                        after,
                        shape: v.shape,
                        copy,
                    };
                    self.add(&pl, map, rate, bpm, tick0, f0, dst);
                }
            }
        }
        let done = (v.ring && !alive && tick0 >= v.ring_at) || v.shape.win.over(f0 + len as u64);
        !done
    }
}

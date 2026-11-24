//! Loop clips as the track player plays them (the audio tracks rethink's
//! phase 4, step 4b: docs/dev/design/loops-on-audio-tracks.md).
//!
//! The arranger's answer says when a track plays a loop ([`Plays::Loop`]).
//! The looper renders the loop, into a buffer of its own a slot; this
//! module says where and how loud, so the looper renders what is heard and
//! the track hears one crossfade:
//!
//! - **Plan first.** [`LoopPlayer::plan`] reads each track's answer for
//!   the period before anything renders: which loop sounds on which frames,
//!   counted from which origin ([`LoopRun`]), for the host to hand the
//!   looper. Nothing renders here.
//! - **One crossfade, at the track.** A switch onto a loop opens it over
//!   [`FADE`] frames, equal power, from the change's frame, as a take clip
//!   opens; a switch away closes it the same way while what follows opens.
//!   Two loops cross over in two voices ([`LOOP_VOICES`]); a third switch
//!   inside a fade takes the voice that is closing, and is counted
//!   ([`LoopPlayer::stolen`]).
//! - **The same loop from another origin** (a launch from another row
//!   naming it, a block meeting a block of it with another offset) stays
//!   one voice: its run splits on the change's frame and the looper
//!   crossfades its place, as at a locate.
//! - **The answer's mute** ramps the loop over [`FADE`] frames from its
//!   frame, as it ramps the region player's sound. A muted loop's runs go
//!   on (at gain 0), so it comes back in place.
//! - **Mixed in before the track's level** ([`LoopPlayer::mix_into`], from
//!   `AudioTracks::process_with_loops`): each run's frames from its slot's
//!   buffer times its gain, into the track's sound, so the track's level,
//!   pan, mute and solo act on a loop as on its regions.
//! - **Stopped**, nothing plays and the next answer starts afresh, as for
//!   clips: the looper's own stop is the looper's.
//! - **A loop that moves onto a track** ([`LoopPlayer::moving`]: it is
//!   heard already, on the Loop strip, as when its first pass lands in a
//!   loop cell) opens at full gain, not over a fade: it carries on where
//!   it is heard, so there is no dip.
//!
//! - **Listen** (phase 4e.2): the looper playing one loop alone while the
//!   song is stopped is heard through the loop's track. After the plan,
//!   [`LoopPlayer::listen`] names the track, and its mix adds what the
//!   looper's Listen played ([`LISTEN`], asked of the same source) on
//!   every frame at full gain: the looper fades Listen in and out itself.
//!
//! Everything happens on a frame and is the same for any period size, and
//! nothing allocates once made.

use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, State};

use crate::clips::{Answer, Glide, Plays, Window, MAX_CHANGES};
use crate::{Join, FADE, MAX_TRACKS};

/// Loops a track sounds at once: the one playing and one fading out.
pub const LOOP_VOICES: usize = 2;

/// Loops heard off any track at once that [`LoopPlayer::moving`] keeps:
/// the looper's.
pub const MOVING: usize = 8;

/// Runs a track has at most in a period: a voice's run splits at each
/// change of origin, and a change can open one voice as another closes.
pub const MAX_LOOP_RUNS: usize = LOOP_VOICES * (MAX_CHANGES + 2);

const PERIOD: usize = MAX_PERIOD as usize;

/// The id [`LoopPlayer::mix_into`] asks its source for what the looper's
/// Listen played (phase 4e.2); never a loop's (ids are counted up from 1).
pub const LISTEN: u128 = u128::MAX;

/// Where a loop sounds on a track in the period: loop `id`, at the place
/// the song position has counted from `origin` (the song tick its pass 0
/// starts on), on frames `from..to` of the period.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoopRun {
    /// The loop, by its lasting id (`mpfree_seq::LoopId`'s).
    pub id: u128,
    pub origin: f64,
    pub from: u32,
    pub to: u32,
    /// The voice whose gains it is heard through.
    voice: u8,
}

/// A loop a track sounds: which, from where, and through what window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Voice {
    on: bool,
    id: u128,
    origin: f64,
    win: Window,
}

const OFF: Voice = Voice {
    on: false,
    id: 0,
    origin: 0.0,
    win: Window::SHUT,
};

/// What a track plays, as this side sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Not known (at first, and after the transport stops).
    Unknown,
    /// Anything but a loop: the region player's.
    Other,
    Loop(usize),
}

#[derive(Clone, Copy, Debug)]
struct Lane {
    voices: [Voice; LOOP_VOICES],
    mode: Mode,
    /// 1 heard, 0 muted by the answer.
    mute: Glide,
    runs: [LoopRun; MAX_LOOP_RUNS],
    n: usize,
}

const LANE: Lane = Lane {
    voices: [OFF; LOOP_VOICES],
    mode: Mode::Unknown,
    mute: Glide::steady(1.0),
    runs: [LoopRun {
        id: 0,
        origin: 0.0,
        from: 0,
        to: 0,
        voice: 0,
    }; MAX_LOOP_RUNS],
    n: 0,
};

/// Every audio track's loops: what the answers say about them, a period
/// at a time ([`LoopPlayer::plan`]), and their gains for the mix.
pub struct LoopPlayer {
    lanes: [Lane; MAX_TRACKS],
    /// Each track's voices' gains on each frame of the planned period,
    /// [`PERIOD`] a voice, the voices of a track together.
    gains: Vec<f32>,
    fade: [f32; FADE + 1],
    frames: usize,
    stolen: u64,
    dropped: u64,
    /// The loops heard off any track now ([`LoopPlayer::moving`]; 0: none).
    moving: [u128; MOVING],
    /// The track Listen is heard through this period ([`LoopPlayer::listen`]).
    listen: Option<usize>,
    /// The free cells' loops this period, (track, id), the first
    /// `n_free` ([`LoopPlayer::free`]).
    free: [(usize, u128); MOVING],
    n_free: usize,
}

impl Default for LoopPlayer {
    fn default() -> LoopPlayer {
        LoopPlayer::new()
    }
}

impl LoopPlayer {
    /// Allocates its gains (a period's worth a voice); never after.
    pub fn new() -> LoopPlayer {
        LoopPlayer {
            lanes: [LANE; MAX_TRACKS],
            gains: vec![0.0; MAX_TRACKS * LOOP_VOICES * PERIOD],
            fade: crate::fade_table(),
            frames: 0,
            stolen: 0,
            dropped: 0,
            moving: [0; MOVING],
            listen: None,
            free: [(0, 0); MOVING],
            n_free: 0,
        }
    }

    /// After [`LoopPlayer::plan`]: Listen (the looper playing one loop
    /// alone while the song is stopped) is heard through track `t` this
    /// period, at full gain on every frame; None: through none. The next
    /// plan forgets it.
    pub fn listen(&mut self, t: Option<usize>) {
        self.listen = t.filter(|&t| t < MAX_TRACKS);
    }

    /// After [`LoopPlayer::plan`]: loop `id` is a free cell's on track `t`
    /// this period (the audio tracks rethink's phase 4e.4): it keeps time
    /// with the looper's free clock, which launches and fades it, so the
    /// track's mix adds what `src` gives for it on every frame at full
    /// gain, rolling or not, as for Listen, and none of its runs (a launch
    /// or a block of its cell): the song's ticks don't place it. A track
    /// may have several (one fading out as the next fades in); at most
    /// [`MOVING`] in all, the rest dropped. The next plan forgets them.
    pub fn free(&mut self, t: usize, id: u128) {
        if t < MAX_TRACKS && id != 0 && self.n_free < MOVING {
            self.free[self.n_free] = (t, id);
            self.n_free += 1;
        }
    }

    /// Before [`LoopPlayer::plan`]: the loops heard off any track now, by
    /// id (the looper's own mix, the Loop strip; at most [`MOVING`], 0 for
    /// none). One of them that opens on a track moves there: at full gain
    /// from its frame, as it is heard already, not over a fade.
    pub fn moving(&mut self, ids: &[u128]) {
        self.moving = [0; MOVING];
        for (m, &id) in self.moving.iter_mut().zip(ids) {
            *m = id;
        }
    }

    /// Before the period's audio: from each track's answer (track `t`'s
    /// is `answers[t]`; a track past them plays no loop), where its loops
    /// sound and how loud. Read the runs with [`LoopPlayer::runs`].
    pub fn plan(&mut self, block: &Block, answers: &[Answer]) {
        let n = (block.frames as usize).min(PERIOD);
        self.frames = n;
        self.listen = None;
        self.n_free = 0;
        // Only the period's frames of each voice: the rest is never read
        // (the whole buffer would be 256 KiB to clear each period).
        for g in self.gains.chunks_exact_mut(PERIOD) {
            g[..n].fill(0.0);
        }
        for t in 0..MAX_TRACKS {
            self.lanes[t].n = 0;
            let a = answers.get(t).copied().unwrap_or(Answer {
                start: Plays::Nothing,
                ..Answer::default()
            });
            let mut next = 0;
            let mut begun = false;
            for span in block.spans() {
                let from = (span.offset as usize).min(n);
                let end = (span.offset as usize + span.len as usize).min(n);
                let rolling = matches!(span.state, State::Playing | State::Recording);
                if !rolling {
                    self.lanes[t].stop();
                    begun = false;
                    while a
                        .changes
                        .get(next)
                        .is_some_and(|c| (c.frame as usize) < end)
                    {
                        next += 1;
                    }
                    continue;
                }
                let abs = |off: usize| span.frame + (off - span.offset as usize) as u64;
                let mut pos = from;
                if !begun && pos < end {
                    begun = true;
                    self.begin(t, &a.start, a.muted, abs(pos));
                }
                while pos < end {
                    while let Some(c) = a.changes.get(next) {
                        if c.frame as usize > pos {
                            break;
                        }
                        next += 1;
                        self.change(t, &c.plays, c.muted, abs(pos));
                    }
                    let to = a
                        .changes
                        .get(next)
                        .map_or(end, |c| (c.frame as usize).clamp(pos + 1, end));
                    self.sound(t, pos, to, abs(pos));
                    pos = to;
                }
            }
        }
    }

    /// Track `t`'s loop runs in the period planned, in the order they
    /// start (a voice's in frame order).
    pub fn runs(&self, t: usize) -> &[LoopRun] {
        self.lanes.get(t).map_or(&[], |l| &l.runs[..l.n])
    }

    /// Add track `t`'s loops into `dst` (interleaved stereo, the planned
    /// period's frames): each run's frames from its loop's buffer, which
    /// `src` gives by the loop's id (the looper's slot buffer, interleaved,
    /// a frame of it for each of the period's; None plays nothing), times
    /// its gain on that frame. Through the track Listen is heard on, also
    /// what `src` gives for [`LISTEN`], as it is.
    pub fn mix_into<'a>(&self, t: usize, src: impl Fn(u128) -> Option<&'a [f32]>, dst: &mut [f32]) {
        let Some(lane) = self.lanes.get(t) else {
            return;
        };
        if self.listen == Some(t) {
            if let Some(s) = src(LISTEN) {
                let b = (2 * self.frames).min(s.len()).min(dst.len());
                for (d, v) in dst[..b].iter_mut().zip(&s[..b]) {
                    *d += v;
                }
            }
        }
        // A free cell's loop: as it is, on every frame (4e.4).
        let free = &self.free[..self.n_free];
        for &(_, id) in free.iter().filter(|f| f.0 == t) {
            if let Some(s) = src(id) {
                let b = (2 * self.frames).min(s.len()).min(dst.len());
                for (d, v) in dst[..b].iter_mut().zip(&s[..b]) {
                    *d += v;
                }
            }
        }
        for r in &lane.runs[..lane.n] {
            if free.iter().any(|f| f.1 == r.id) {
                continue;
            }
            let Some(s) = src(r.id) else {
                continue;
            };
            let g = &self.gains[(t * LOOP_VOICES + usize::from(r.voice)) * PERIOD..];
            let (a, b) = (r.from as usize, r.to as usize);
            let b = b.min(s.len() / 2).min(dst.len() / 2);
            for j in a..b {
                dst[2 * j] += s[2 * j] * g[j];
                dst[2 * j + 1] += s[2 * j + 1] * g[j];
            }
        }
    }

    /// Track `t`'s gain for its loop voice `v` on frame `j` of the planned
    /// period (for tests and meters).
    pub fn gain(&self, t: usize, v: usize, j: usize) -> f32 {
        if t >= MAX_TRACKS || v >= LOOP_VOICES || j >= self.frames {
            return 0.0;
        }
        self.gains[(t * LOOP_VOICES + v) * PERIOD + j]
    }

    /// Loops cut short because a third switched in inside a fade.
    pub fn stolen(&self) -> u64 {
        self.stolen
    }

    /// Runs not kept because a track had more than [`MAX_LOOP_RUNS`] in a
    /// period (only with launches closer than a few frames).
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Track `t` plays `plays` at frame `f`, its period's start: as it is
    /// if nothing played before; carried on; or a change.
    fn begin(&mut self, t: usize, plays: &Plays, muted: bool, f: u64) {
        let heard = if muted { 0.0 } else { 1.0 };
        let lane = &mut self.lanes[t];
        match (lane.mode, plays) {
            (Mode::Unknown, _) => {
                lane.mute = Glide::steady(heard);
                lane.voices = [OFF; LOOP_VOICES];
                lane.mode = Mode::Other;
                self.change(t, plays, muted, f);
            }
            (Mode::Loop(v), Plays::Loop(l))
                if lane.voices[v].on
                    && lane.voices[v].id == l.id
                    && lane.voices[v].origin == l.origin =>
            {
                lane.mute.set(heard, f);
            }
            (Mode::Other, p) if !matches!(p, Plays::Loop(_)) => lane.mute.set(heard, f),
            _ => self.change(t, plays, muted, f),
        }
    }

    /// Track `t` changes to `plays` on frame `f`: the loop it played
    /// closes, a loop it plays now opens (or carries on, from its new
    /// origin if it has one), and the mute moves.
    fn change(&mut self, t: usize, plays: &Plays, muted: bool, f: u64) {
        let heard = if muted { 0.0 } else { 1.0 };
        let moving = self.moving;
        let lane = &mut self.lanes[t];
        let playing = match lane.mode {
            Mode::Loop(v) if lane.voices[v].on => Some(v),
            _ => None,
        };
        match plays {
            Plays::Loop(l) => match playing {
                // The same loop: one voice, from its new origin if it has
                // one (the looper crossfades the place).
                Some(v) if lane.voices[v].id == l.id => lane.voices[v].origin = l.origin,
                _ => {
                    if let Some(v) = playing {
                        lane.voices[v].win.close_at(f, Join::Fade);
                    }
                    // The loop still closing in the other voice (a switch
                    // away and back inside a fade): that voice opens again
                    // from the gain it has, so a loop is only ever in one
                    // voice and the looper renders one place for it.
                    let back = (0..LOOP_VOICES).find(|&v| {
                        Some(v) != playing && lane.voices[v].on && lane.voices[v].id == l.id
                    });
                    if let Some(v) = back {
                        let w = lane.voices[v].win;
                        let k = f.saturating_sub(w.to).min(FADE as u64);
                        lane.voices[v].win =
                            Window::opening(f.saturating_sub(FADE as u64 - k), Join::Fade);
                        lane.voices[v].origin = l.origin;
                        lane.mode = Mode::Loop(v);
                        lane.mute.set(heard, f);
                        return;
                    }
                    let v = match lane.voices.iter().position(|v| !v.on) {
                        Some(v) => v,
                        None => {
                            // Both sound: the one closing (the other is
                            // the one playing, closing from here too) goes.
                            self.stolen += 1;
                            (0..LOOP_VOICES).find(|&v| Some(v) != playing).unwrap_or(0)
                        }
                    };
                    // Heard already, off the track: it moves on, unfaded.
                    let join = if l.id != 0 && moving.contains(&l.id) {
                        Join::Straight
                    } else {
                        Join::Fade
                    };
                    lane.voices[v] = Voice {
                        on: true,
                        id: l.id,
                        origin: l.origin,
                        win: Window::opening(f, join),
                    };
                    lane.mode = Mode::Loop(v);
                }
            },
            _ => {
                if let Some(v) = playing {
                    lane.voices[v].win.close_at(f, Join::Fade);
                }
                lane.mode = Mode::Other;
            }
        }
        lane.mute.set(heard, f);
    }

    /// Track `t`'s voices over frames `a..b` of the period (absolute frame
    /// `f` at `a`): their gains, and their runs extended or begun.
    fn sound(&mut self, t: usize, a: usize, b: usize, f: u64) {
        let lane = &mut self.lanes[t];
        for v in 0..LOOP_VOICES {
            let voice = lane.voices[v];
            if !voice.on {
                continue;
            }
            if voice.win.over(f) {
                lane.voices[v].on = false;
                continue;
            }
            let g = &mut self.gains[(t * LOOP_VOICES + v) * PERIOD..][..PERIOD];
            for (j, g) in g.iter_mut().enumerate().take(b).skip(a) {
                let at = f + (j - a) as u64;
                *g = voice.win.at(at, &self.fade) * lane.mute.at(at);
            }
            // Its run, to the frame its window closes for good if that is
            // in this stretch: the one it has open, if it runs straight on
            // with the same origin, else a new one.
            let stop = voice.win.over_from();
            let b = if stop < f + (b - a) as u64 {
                a + (stop - f) as usize
            } else {
                b
            };
            let open = lane.runs[..lane.n]
                .iter()
                .rposition(|r| usize::from(r.voice) == v);
            match open {
                Some(k)
                    if lane.runs[k].to as usize == a
                        && lane.runs[k].origin == voice.origin
                        && lane.runs[k].id == voice.id =>
                {
                    lane.runs[k].to = b as u32;
                }
                _ if lane.n < MAX_LOOP_RUNS => {
                    lane.runs[lane.n] = LoopRun {
                        id: voice.id,
                        origin: voice.origin,
                        from: a as u32,
                        to: b as u32,
                        voice: v as u8,
                    };
                    lane.n += 1;
                }
                _ => self.dropped += 1,
            }
            // Closed for good inside this stretch: free from there.
            if voice.win.over(f + (b - a) as u64) {
                lane.voices[v].on = false;
            }
        }
    }
}

impl Lane {
    fn stop(&mut self) {
        self.voices = [OFF; LOOP_VOICES];
        self.mode = Mode::Unknown;
    }
}

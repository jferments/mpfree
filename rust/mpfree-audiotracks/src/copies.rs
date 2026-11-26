//! Tempo copies: Stretch's audio (the audio tracks rethink, phase 5b).
//!
//! A region on [`FollowTempo::Stretch`] stays on its beats at its own
//! pitch by playing a copy of its recording stretched to the tempo,
//! made ahead on the stretch scheduler (`mpfree_stretch::sched`). This is
//! the track player's side of them:
//!
//! - **What a copy is filed by** ([`CopyKey`]): everything that made it.
//!   The recording and its frames then, the region's stretch of it and
//!   where between frames it starts, the frames it plays for, its made-at
//!   tempo and the tempo, the rate, its ends ([`Ends`]), the stretch's
//!   settings and `mpfree_stretch::VERSION`. A region plays a copy only
//!   while the key it works out now is the copy's, so a copy made before
//!   an edit, a tempo change or a new build is never played.
//! - **Held apart from the takes.** The pool is recording space, and a
//!   copy never takes a recording's room: a copy is audio the worker made
//!   ([`Copy`]), held in a table of [`MAX_COPIES`] here, and handed back
//!   to the host to drop off the audio thread ([`Copies::take_dropped`]).
//! - **Played at speed 1.** At a steady tempo a copy is the region's
//!   frames at the tempo, so the frame of it song tick `t` plays is
//!   [`Copy::lead`] plus the frames from the region's start, as an Off
//!   region plays its take: the region's gain, fades and joins as before.
//! - **Latched.** What a region on Stretch plays is decided when it is
//!   first heard (each pass of a clip, each time a song region plays):
//!   its copy if it is here, else Repitch of its recording (Play on time).
//!   The choice holds to its end, so a copy that comes in the middle waits
//!   for the next start and the pitch never moves mid-pass; a tempo change
//!   that leaves its copy behind plays Repitch to its end. A locate, a
//!   jump or the transport stopping starts the latches again.
//! - **Keep pitch through a tempo change** (5b.4). A region on Keep pitch
//!   latched to a copy, or to its recording at its own tempo, doesn't
//!   switch to Repitch when the tempo moves away from it: it plays on from
//!   that audio at speed 1, in pitch, from where it was, drifting from the
//!   bars ([`Drift`]). Once the copy for the tempo now is held it rejoins
//!   on the next bar line (the run's meter), from that copy where the bars
//!   put it, crossfaded with equal gain over [`FADE`] frames, and holds
//!   that copy from there. The host asks for a drifting region's copy at
//!   once ([`Copies::drift_wants`]).
//! - **Where two meet** playing from different audio (a recording and a
//!   copy, or two copies), they meet with the equal-power fade
//!   ([`Join::Fade`]), never straight. A looping clip's one region meets
//!   itself straight on its wrapped copy ([`Ends::Wrapped`]).
//! - **Made** from its source frames by a [`CopyJob`] on the worker:
//!   real audio either side of the region ([`Ends::Edges`]), or the loop
//!   wrapped round with its seam crossfaded, as the looper's prints are.
//!
//! Nothing on the audio thread allocates: the table and the latches are
//! made with the tracks.

use std::sync::Arc;

use mpfree_stretch::{auto_window, Job, Settings, StretchError, TimeMap, MAX_SLOPE, MIN_SLOPE};
use mpfree_transport::time::{valid_bpm, TempoMap, Tick, PPQ};

use crate::{FollowTempo, Join, RecordingId, Region, FADE, MAX_SPEED};

/// Copies the track player holds at most.
pub const MAX_COPIES: usize = 64;
/// Regions' choices of what to play held at once: every region a track's
/// song and its voices can sound at a time, with room to spare.
const MAX_LATCHES: usize = 128;
/// The crossfade at a wrapped copy's loop point, in frames (at most half
/// the copy), as the looper's prints have.
pub const SEAM: usize = 1024;
/// No copy: a latch that plays the recording.
const NONE: u16 = u16::MAX;
/// What [`Copies::source`] gives for a region drifting on Keep pitch:
/// this plus its latch ([`Copies::drifting`]). Never a copy's slot.
pub(crate) const DRIFT: usize = 1 << 16;

/// How a copy's ends are made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ends {
    /// A region's: the recording's own audio either side of it (silence
    /// where it has none), two stretch windows' worth, so its fades, its
    /// crossfades and a ring past its end sound as they would.
    Edges,
    /// A looping clip's one region, whose recording ends where it does:
    /// the loop wrapped round on either side and its seam crossfaded, so
    /// the copy goes round with no click.
    Wrapped,
}

/// Everything a copy was made from ([`CopyKey::of`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CopyKey {
    /// The recording, and its frames when the copy was made: a finished
    /// take never changes, and a take still recording isn't copied.
    pub rec: RecordingId,
    pub frames: u64,
    /// The region's first frame of it, and where between frames it starts
    /// ([`Region::phase`], as bits).
    pub src: i64,
    pub phase: u32,
    /// The recording frames the region covers, and those after `src` the
    /// copy covers: more for a clip's last region, which rings on to the
    /// clip's last frame.
    pub src_len: i64,
    pub reach: i64,
    /// The frames the region plays for at the tempo.
    pub out: i64,
    /// Its made-at tempo and the tempo over it, as bits. The tempo is one
    /// steady tempo in 5b; ramps and steps are 5d's.
    pub made_at: u64,
    pub tempo: u64,
    pub rate: u32,
    pub ends: Ends,
    /// The stretch's settings ([`settings_id`]) and version.
    pub settings: u64,
    pub version: u32,
}

impl CopyKey {
    /// The copy region `r` of recording `rec` (`frames` long) needs at a
    /// steady `bpm`, ending `ends` and covering `reach` of its recording's
    /// frames after its start (at least its own); None unless it is on
    /// Stretch with a made-at tempo and another tempo here, within the
    /// speeds it plays at.
    #[allow(clippy::too_many_arguments)]
    pub fn of(
        r: &Region,
        rec: RecordingId,
        frames: usize,
        bpm: f64,
        rate: u32,
        ends: Ends,
        reach: i64,
    ) -> Option<CopyKey> {
        if r.follow != FollowTempo::Stretch || !valid_bpm(bpm) || rate == 0 {
            return None;
        }
        let sp = r.speed(bpm)?;
        // Within what the stretch makes (10x either way) and the speeds a
        // region plays at.
        if sp == 1.0 || !(1.0 / MAX_SLOPE..=1.0 / MIN_SLOPE).contains(&sp) || sp > MAX_SPEED {
            return None;
        }
        let src_len = r.src_len(0, rate);
        if src_len <= 0 {
            return None;
        }
        let ticks = (r.end - r.start) as f64;
        let out = (f64::from(r.phase) + ticks * 60.0 * f64::from(rate) / (bpm * f64::from(PPQ)))
            .round() as i64;
        if out <= 0 {
            return None;
        }
        Some(CopyKey {
            rec,
            frames: frames as u64,
            src: r.src,
            phase: r.phase.to_bits(),
            src_len,
            reach: reach.max(src_len),
            out,
            made_at: r.bpm.to_bits(),
            tempo: bpm.to_bits(),
            rate,
            ends,
            settings: settings_id(&settings(rate)),
            version: mpfree_stretch::VERSION,
        })
    }

    /// Recording frames an output frame.
    pub fn speed(&self) -> f64 {
        f64::from_bits(self.tempo) / f64::from_bits(self.made_at)
    }

    /// A number for the key, for the scheduler.
    pub fn id(&self) -> u128 {
        use std::hash::{Hash, Hasher};
        let mut a = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut a);
        let mut b = std::collections::hash_map::DefaultHasher::new();
        (0x5b2_u32, self).hash(&mut b);
        (u128::from(a.finish()) << 64) | u128::from(b.finish())
    }

    /// The memory its copy takes, about.
    pub fn bytes(&self, channels: usize) -> u64 {
        let pad = match self.ends {
            Ends::Edges => 2.0 * pad_frames(self.rate) as f64 / self.speed(),
            Ends::Wrapped => 0.0,
        };
        let frames = (self.reach - self.src_len).max(0) as f64 / self.speed() + self.out as f64;
        ((frames + pad).ceil() as u64) * channels as u64 * 4
    }
}

/// The stretch's settings for tempo copies at `rate`: the sampler's
/// defaults (hits kept sharp), with the window worked out, so the key
/// names it.
pub fn settings(rate: u32) -> Settings {
    Settings {
        window: auto_window(rate),
        ..Settings::default()
    }
}

/// A number for `s`, for [`CopyKey::settings`].
pub fn settings_id(s: &Settings) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in [
        u64::from(s.semitones.to_bits()),
        s.window as u64,
        u64::from(s.transients),
        u64::from(s.sensitivity.to_bits()),
    ] {
        h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Recording frames either side of a region that its copy is made with:
/// two stretch windows.
pub fn pad_frames(rate: u32) -> usize {
    2 * auto_window(rate)
}

/// Whether region `r`, a looping clip's only region filling its pass, of
/// a recording `frames` long, goes round as made: its recording ends
/// where it does, so its copy is wrapped ([`Ends::Wrapped`]).
pub fn wraps(r: &Region, frames: usize, rate: u32) -> bool {
    r.src >= 0 && r.src + r.src_len(0, rate) == frames as i64
}

/// How the copy for region `i` of a clip's `regions` (its pass `len`
/// ticks, ringing to its recording's frame `last_frame`) ends, and how
/// much of its recording after its start it covers: a looping clip's one
/// region whose recording (`frames` long) ends where it does goes round
/// ([`Ends::Wrapped`]); the last region covers its recording to the
/// clip's last frame, for a ring; any other its own frames.
pub fn clip_shape(
    regions: &[Region],
    i: usize,
    len: Tick,
    last_frame: i64,
    frames: Option<usize>,
    rate: u32,
) -> (Ends, i64) {
    let r = &regions[i];
    let src_len = r.src_len(0, rate);
    let alone = regions.len() == 1 && r.start == 0 && r.end == len;
    if alone && frames.is_some_and(|f| wraps(r, f, rate)) {
        return (Ends::Wrapped, src_len);
    }
    let reach = if i + 1 == regions.len() {
        src_len.max(last_frame - r.src)
    } else {
        src_len
    };
    (Ends::Edges, reach)
}

/// The one steady tempo over song ticks `at..at + len` of `map`, if the
/// tempo doesn't change there.
pub fn steady(map: &TempoMap, at: f64, len: f64) -> Option<f64> {
    let t = at.floor() as Tick;
    match map.next_change_after(t) {
        Some(c) if (c as f64) < at + len => None,
        _ => Some(map.bpm_at(t)),
    }
}

/// A finished copy, for [`Copies::put`].
#[derive(Clone)]
pub struct Copy {
    pub key: CopyKey,
    /// Its audio, interleaved, `channels` channels (the recording's).
    pub data: Arc<[f32]>,
    pub channels: u8,
    /// Its frame where the region starts; frames before it are the
    /// recording's before the region, stretched (none for a wrapped copy).
    pub lead: i64,
}

impl Copy {
    pub fn frames(&self) -> usize {
        self.data.len() / usize::from(self.channels.max(1))
    }

    pub fn bytes(&self) -> usize {
        self.data.len() * 4
    }
}

impl std::fmt::Debug for Copy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Copy")
            .field("key", &self.key)
            .field("frames", &self.frames())
            .field("channels", &self.channels)
            .field("lead", &self.lead)
            .finish()
    }
}

/// A copy the track player holds, as the host sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CopyView {
    pub key: CopyKey,
    pub bytes: usize,
    /// The frame it was last played in (0: never).
    pub heard: u64,
    /// A region playing now plays it: it can't be dropped.
    pub playing: bool,
}

struct Held {
    copy: Copy,
    /// One more for each copy put, so a latch to a slot since refilled
    /// never plays the new one.
    serial: u32,
    heard: u64,
}

/// What a region heard plays: its copy (slot and serial) or its
/// recording ([`NONE`]).
#[derive(Clone, Copy)]
struct Latch {
    /// The region as it plays this time ([`instance`]); 0 is a free entry.
    id: u64,
    slot: u16,
    serial: u32,
    heard: u64,
    /// The tempo (bits) the audio it plays is on the bars at: its copy's,
    /// or its made-at tempo when it plays its recording at its own; 0 when
    /// it plays Repitch (Keep pitch has nothing to keep).
    tempo: u64,
    /// On Keep pitch, since the tempo moved away from `tempo`.
    drift: Option<Drift>,
    /// The lane it plays on ([`instance`]'s `owner`): a track's song, or
    /// one of its voices.
    owner: u16,
    /// It plays its recording off its own tempo, its copy wanted and not
    /// here: Repitch for want of its copy (the cells' "repitch now",
    /// phase 5c.2).
    repitch: bool,
}

const FREE: Latch = Latch {
    id: 0,
    slot: NONE,
    serial: 0,
    heard: 0,
    tempo: 0,
    drift: None,
    owner: 0,
    repitch: false,
};

/// What the regions on Stretch a lane heard last play (phase 5c.2, the
/// cells' words): [`Copies::on_stretch`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OnStretch {
    /// One plays Repitch for want of its copy (Play on time, or a copy
    /// that went), to the end of its pass.
    pub repitch: bool,
    /// One plays on in pitch from audio the tempo moved away from (Keep
    /// pitch), off the bars.
    pub drift: bool,
    /// The bar line (song tick) a drifting one rejoins the bars on, once
    /// its copy for the tempo now is here: the soonest.
    pub join: Option<f64>,
}

/// A region on Keep pitch playing on at speed 1 from the audio it was
/// latched to, after the tempo moved away from it (see the module notes).
#[derive(Clone, Copy)]
struct Drift {
    /// The audio it plays on from: the copy in `slot` (its `serial`), or
    /// its recording ([`NONE`]).
    slot: u16,
    serial: u32,
    /// The frame of that audio from the region's start (past the copy's
    /// lead, or the recording's past the region's first frame) that played
    /// on output frame `f`.
    e: i64,
    f: u64,
    /// The bar line (song tick) it rejoins on, and what it rejoins: the
    /// copy in `to_slot` (its `to_serial`) or its recording ([`NONE`]), on
    /// the bars at `to_tempo`.
    join: Option<f64>,
    to_slot: u16,
    to_serial: u32,
    to_tempo: u64,
    /// The copy for the tempo now, for the host to ask for at once.
    want: Option<CopyKey>,
}

/// What a region on Keep pitch is, where a run is: for [`Copies::source`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Keep {
    /// The one steady tempo over it now (bits; 0 if there isn't one), and
    /// its made-at tempo (bits).
    pub tempo: u64,
    pub made_at: u64,
    /// The song ticks it plays from and to as heard, and where between
    /// frames it starts ([`Region::phase`]).
    pub at: f64,
    pub end: f64,
    pub phase: f64,
    pub run: RunAt,
    pub rate: u32,
}

/// Where a run is: its first frame and song tick, the first bar line at
/// or after it (its meter's), and its tempo.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RunAt {
    pub f: u64,
    pub tick: f64,
    pub bar: f64,
    pub bpm: f64,
}

impl RunAt {
    /// Output frames from the run's first frame to song tick `t`, at its
    /// tempo.
    pub(crate) fn frames_to(&self, t: f64, rate: u32) -> i64 {
        frames_over(t - self.tick, self.bpm, rate)
    }
}

/// Output frames `ticks` song ticks take at `bpm`, to the nearest
/// ([`crate::nearest`], as [`Region::frames_to`]).
pub(crate) fn frames_over(ticks: f64, bpm: f64, rate: u32) -> i64 {
    crate::nearest(ticks * frames_per_tick(bpm, rate))
}

/// Frames a song tick at `bpm`.
fn frames_per_tick(bpm: f64, rate: u32) -> f64 {
    60.0 * f64::from(rate) / (bpm * f64::from(PPQ))
}

/// A region drifting on Keep pitch, as a run plays it
/// ([`Copies::drifting`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drifting {
    /// The audio it plays on from (a copy's slot, or None for its
    /// recording) and that audio's frame from the region's start on output
    /// frame `f`.
    pub from: Option<usize>,
    pub e: i64,
    pub f: u64,
    /// The bar line it rejoins on and what it rejoins (a copy's slot, or
    /// None for its recording at its own tempo).
    pub join: Option<f64>,
    pub to: Option<usize>,
}

/// A number for region `r` as it plays this time: on lane `owner` (a
/// track's song or one of its voices), from song tick `at`, the voice
/// having started on frame `since`.
pub(crate) fn instance(owner: u16, at: f64, since: u64, r: &Region) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in [
        u64::from(owner),
        at.to_bits(),
        since,
        u64::from(r.take),
        r.src as u64,
    ] {
        h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
    }
    h | 1
}

/// The track player's copies and what each region heard plays (see the
/// module notes).
pub struct Copies {
    held: Vec<Option<Held>>,
    serial: u32,
    latches: Box<[Latch]>,
    any_latch: bool,
    dropped: Vec<Arc<[f32]>>,
    /// The frame of the latest period, for what counts as playing now.
    now: u64,
}

/// A latch heard this long ago, in frames, still plays (two of the
/// longest periods).
const PLAYING: u64 = 2 * mpfree_transport::transport::MAX_PERIOD as u64;

impl Copies {
    pub(crate) fn new() -> Copies {
        Copies {
            held: (0..MAX_COPIES).map(|_| None).collect(),
            serial: 0,
            latches: vec![FREE; MAX_LATCHES].into_boxed_slice(),
            any_latch: false,
            dropped: Vec::with_capacity(2 * MAX_COPIES),
            now: 0,
        }
    }

    pub(crate) fn set_now(&mut self, frame: u64) {
        self.now = frame;
    }

    /// Takes `c` in, in place of a copy with its key. Gives it back if the
    /// table is full (drop one first) or a copy with its key plays now.
    #[allow(clippy::result_large_err)]
    pub fn put(&mut self, c: Copy) -> Result<(), Copy> {
        if let Some(i) = self.find(&c.key) {
            if self.playing(i) {
                return Err(c);
            }
            self.drop_slot(i);
        }
        let Some(i) = self.held.iter().position(Option::is_none) else {
            return Err(c);
        };
        self.serial = self.serial.wrapping_add(1);
        self.held[i] = Some(Held {
            copy: c,
            serial: self.serial,
            heard: 0,
        });
        Ok(())
    }

    /// Lets the copy with `key` go, unless a region plays it now. True if
    /// it went (or wasn't here).
    pub fn drop_key(&mut self, key: &CopyKey) -> bool {
        match self.find(key) {
            Some(i) if self.playing(i) => false,
            Some(i) => {
                self.drop_slot(i);
                true
            }
            None => true,
        }
    }

    /// Lets every copy go that no region plays now. How many went.
    pub fn clear(&mut self) -> usize {
        let mut n = 0;
        for i in 0..MAX_COPIES {
            if self.held[i].is_some() && !self.playing(i) {
                self.drop_slot(i);
                n += 1;
            }
        }
        n
    }

    /// A copy let go, for the host to drop off the audio thread.
    pub fn take_dropped(&mut self) -> Option<Arc<[f32]>> {
        self.dropped.pop()
    }

    /// The copies held.
    pub fn views(&self) -> impl Iterator<Item = CopyView> + '_ {
        self.held.iter().enumerate().filter_map(|(i, h)| {
            h.as_ref().map(|h| CopyView {
                key: h.copy.key,
                bytes: h.copy.bytes(),
                heard: h.heard,
                playing: self.playing(i),
            })
        })
    }

    /// Whether the copy with `key` is held.
    pub fn has(&self, key: &CopyKey) -> bool {
        self.find(key).is_some()
    }

    /// Memory the copies hold.
    pub fn bytes(&self) -> usize {
        self.held.iter().flatten().map(|h| h.copy.bytes()).sum()
    }

    pub fn len(&self) -> usize {
        self.held.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn find(&self, key: &CopyKey) -> Option<usize> {
        self.held
            .iter()
            .position(|h| h.as_ref().is_some_and(|h| h.copy.key == *key))
    }

    fn drop_slot(&mut self, i: usize) {
        if let Some(h) = self.held[i].take() {
            if self.dropped.len() < self.dropped.capacity() {
                self.dropped.push(h.copy.data);
            } else {
                // Never: the host takes them every period. Kept rather
                // than freed here.
                self.held[i] = Some(h);
            }
        }
    }

    /// Whether a region heard lately plays slot `i`: latched to it, or
    /// drifting on it or about to rejoin it.
    fn playing(&self, i: usize) -> bool {
        let Some(h) = &self.held[i] else {
            return false;
        };
        let on = |slot: u16, serial: u32| slot as usize == i && serial == h.serial;
        self.any_latch
            && self.latches.iter().any(|l| {
                l.id != 0
                    && l.heard + PLAYING >= self.now
                    && (on(l.slot, l.serial)
                        || l.drift.is_some_and(|d| {
                            on(d.slot, d.serial) || (d.join.is_some() && on(d.to_slot, d.to_serial))
                        }))
            })
    }

    /// Whether `slot` still holds the copy put with `serial` (the
    /// recording, [`NONE`], always is).
    fn holds(&self, slot: u16, serial: u32) -> bool {
        slot == NONE
            || self
                .held
                .get(slot as usize)
                .and_then(Option::as_ref)
                .is_some_and(|h| h.serial == serial)
    }

    fn hear(&mut self, slot: u16, f: u64) {
        if let Some(h) = self.held.get_mut(slot as usize).and_then(Option::as_mut) {
            h.heard = h.heard.max(f);
        }
    }

    /// The copies the regions drifting on Keep pitch want, for the host to
    /// ask for first.
    pub(crate) fn drift_wants(&self, f: &mut dyn FnMut(CopyKey)) {
        if !self.any_latch {
            return;
        }
        for l in self.latches.iter() {
            if l.id != 0 && l.heard + PLAYING >= self.now {
                if let Some(k) = l.drift.and_then(|d| d.want) {
                    f(k);
                }
            }
        }
    }

    /// How the region with latch `li` drifts, if it does.
    pub(crate) fn drifting(&self, li: usize) -> Option<Drifting> {
        let d = self.latches.get(li)?.drift?;
        let slot = |s: u16| (s != NONE).then_some(s as usize);
        Some(Drifting {
            from: slot(d.slot),
            e: d.e,
            f: d.f,
            join: d.join,
            to: slot(d.to_slot),
        })
    }

    /// What the regions on Stretch of the lanes `lane` picks heard last
    /// play, if heard lately (phase 5c.2, the cells' words): one on
    /// Repitch for want of its copy, one drifting on Keep pitch, and the
    /// soonest bar line a drifting one rejoins on. Nothing allocates.
    pub(crate) fn on_stretch(&self, lane: &dyn Fn(u16) -> bool) -> OnStretch {
        let mut out = OnStretch::default();
        if !self.any_latch {
            return out;
        }
        let ours = |l: &&Latch| l.id != 0 && lane(l.owner);
        let Some(last) = self
            .latches
            .iter()
            .filter(ours)
            .map(|l| l.heard)
            .max()
            .filter(|&h| h + PLAYING >= self.now)
        else {
            return out;
        };
        for l in self.latches.iter().filter(ours).filter(|l| l.heard == last) {
            match l.drift {
                Some(d) => {
                    out.drift = true;
                    if let Some(j) = d.join {
                        out.join = Some(out.join.map_or(j, |o| o.min(j)));
                    }
                }
                None => out.repitch |= l.repitch,
            }
        }
        out
    }

    /// Forgets every region's choice: a locate, a jump, the transport
    /// stopping.
    pub(crate) fn unlatch(&mut self) {
        if self.any_latch {
            self.latches.fill(FREE);
            self.any_latch = false;
        }
    }

    /// The copy region instance `id` plays, deciding if it hasn't yet and
    /// `decide`: the copy with `key` if one is held (and still the one it
    /// latched), else its recording (None). Heard on frame `f`. A region on
    /// Keep pitch (`keep`) drifting from the bars gives [`DRIFT`] plus its
    /// latch ([`Copies::drifting`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn source(
        &mut self,
        id: u64,
        owner: u16,
        key: Option<CopyKey>,
        f: u64,
        decide: bool,
        keep: Option<&Keep>,
    ) -> Option<usize> {
        if let Some(li) = self.latches.iter().position(|l| l.id == id) {
            let mut l = self.latches[li];
            if decide {
                l.heard = l.heard.max(f);
            }
            let out = self.latched(&mut l, li, key, f, decide, keep);
            self.latches[li] = l;
            return out;
        }
        if !decide {
            return None;
        }
        let found = key.and_then(|k| self.find(&k));
        let (slot, serial) = match found {
            Some(i) => {
                let h = self.held[i].as_mut().expect("found");
                h.heard = h.heard.max(f);
                (i as u16, h.serial)
            }
            None => (NONE, 0),
        };
        // The tempo its audio is on the bars at, for Keep pitch: its
        // copy's, or its own when it plays its recording at its own tempo.
        let tempo = match (found, keep) {
            (Some(_), _) => key.map_or(0, |k| k.tempo),
            (None, Some(k)) if k.tempo != 0 && k.tempo == k.made_at => k.made_at,
            _ => 0,
        };
        // A free entry, else the one heard longest ago.
        let e = match self.latches.iter().position(|l| l.id == 0) {
            Some(e) => e,
            None => (0..MAX_LATCHES)
                .min_by_key(|&e| self.latches[e].heard)
                .unwrap_or(0),
        };
        self.latches[e] = Latch {
            id,
            slot,
            serial,
            heard: f,
            tempo,
            drift: None,
            owner,
            repitch: found.is_none() && key.is_some(),
        };
        self.any_latch = true;
        found
    }

    /// [`Copies::source`] for a region already latched (`l`, entry `li`).
    fn latched(
        &mut self,
        l: &mut Latch,
        li: usize,
        key: Option<CopyKey>,
        f: u64,
        decide: bool,
        keep: Option<&Keep>,
    ) -> Option<usize> {
        if decide && l.drift.is_none() {
            if let Some(k) = keep {
                // On Keep pitch, the tempo moved away from the audio it
                // plays: it plays on from there at speed 1.
                if l.tempo != 0 && k.tempo != l.tempo && self.holds(l.slot, l.serial) {
                    let ticks = k.run.tick - k.at;
                    let e = (k.phase + ticks * frames_per_tick(f64::from_bits(l.tempo), k.rate))
                        .round() as i64;
                    l.drift = Some(Drift {
                        slot: l.slot,
                        serial: l.serial,
                        e,
                        f,
                        join: None,
                        to_slot: NONE,
                        to_serial: 0,
                        to_tempo: 0,
                        want: None,
                    });
                }
            }
        }
        if let Some(d) = l.drift {
            if !decide {
                return Some(DRIFT + li);
            }
            return self.drift(l, d, li, key, f, keep);
        }
        if l.slot == NONE {
            if decide {
                // Off its own tempo now, or back on it.
                l.repitch = key.is_some();
            }
            return None;
        }
        let slot = l.slot as usize;
        let still = matches!((&self.held[slot], key), (Some(h), Some(k)) if h.serial == l.serial && h.copy.key == k);
        if !still {
            // Its copy went, or the tempo moved away from it: its
            // recording to its end. Only looked up on Keep pitch, it is
            // left for its own decision (it may drift).
            if decide || keep.is_none() {
                l.slot = NONE;
                l.tempo = 0;
                l.repitch = key.is_some();
            }
            return None;
        }
        if decide {
            self.hear(l.slot, f);
        }
        Some(slot)
    }

    /// A region drifting on Keep pitch (`l`, its drift `d`), decided on
    /// frame `f`: whether and where it rejoins the bars, and once over the
    /// crossfade, the audio it rejoined.
    fn drift(
        &mut self,
        l: &mut Latch,
        mut d: Drift,
        li: usize,
        key: Option<CopyKey>,
        f: u64,
        keep: Option<&Keep>,
    ) -> Option<usize> {
        let Some(k) = keep.filter(|_| self.holds(d.slot, d.serial)) else {
            // Keep pitch turned off, or its audio went: Repitch to its end.
            l.drift = None;
            l.slot = NONE;
            l.tempo = 0;
            l.repitch = key.is_some();
            return None;
        };
        d.want = key;
        // What it rejoins: its recording back at its own tempo, else the
        // copy for the tempo now, if it is here.
        let target = if k.tempo != 0 && k.tempo == k.made_at {
            Some((NONE, 0, k.made_at))
        } else {
            key.and_then(|kk| {
                let i = self.find(&kk)?;
                let h = self.held[i].as_ref()?;
                Some((i as u16, h.serial, kk.tempo))
            })
        };
        if let Some(join) = d.join {
            // Once its crossfade has begun it finishes; before, it waits
            // for what is wanted now.
            let begun = k.run.tick >= join;
            let same = target == Some((d.to_slot, d.to_serial, d.to_tempo));
            if !self.holds(d.to_slot, d.to_serial) || !(begun || same) {
                d.join = None;
            }
        }
        if d.join.is_none() {
            if let Some((slot, serial, tempo)) = target {
                // On the first bar line from here, if the region still
                // plays there.
                if k.run.bar < k.end {
                    (d.join, d.to_slot, d.to_serial, d.to_tempo) =
                        (Some(k.run.bar), slot, serial, tempo);
                }
            }
        }
        if let Some(join) = d.join {
            if k.run.frames_to(join, k.rate) + FADE as i64 <= 0 {
                // Over the crossfade: it holds what it rejoined.
                (l.slot, l.serial, l.tempo, l.drift) = (d.to_slot, d.to_serial, d.to_tempo, None);
                l.repitch = false;
                if l.slot == NONE {
                    return None;
                }
                self.hear(l.slot, f);
                return Some(l.slot as usize);
            }
            self.hear(d.to_slot, f);
        }
        self.hear(d.slot, f);
        l.drift = Some(d);
        Some(DRIFT + li)
    }

    /// Adds `gain`-weighted frames of the copy in `slot` into the
    /// interleaved stereo `dst` (a run's frames), from where the region is
    /// `e` frames past its start: a wrapped copy goes round, and an edged
    /// one is silent past its ends.
    pub(crate) fn add(&self, slot: usize, e: i64, dst: &mut [f32], gain: &impl Fn(usize) -> f32) {
        let Some(h) = self.held.get(slot).and_then(Option::as_ref) else {
            return;
        };
        let c = &h.copy;
        let k = c.lead + e;
        let ch = usize::from(c.channels.max(1));
        let frames = c.frames() as i64;
        let n = dst.len() / 2;
        if frames == 0 || n == 0 {
            return;
        }
        let mut j = 0usize;
        while j < n {
            let at = k + j as i64;
            let (from, count) = match c.key.ends {
                Ends::Wrapped => {
                    let q = at.rem_euclid(frames);
                    (q, ((frames - q) as usize).min(n - j))
                }
                Ends::Edges => {
                    if at < 0 {
                        j += ((-at) as usize).min(n - j);
                        continue;
                    }
                    if at >= frames {
                        break;
                    }
                    (at, ((frames - at) as usize).min(n - j))
                }
            };
            let s = from as usize * ch;
            crate::add_frames(&c.data[s..s + count * ch], ch, dst, j, gain);
            j += count;
        }
    }
}

/// Two regions that meet, `a` playing `sa` and `b` playing `sb` (a copy's
/// slot, or None for the recording): their join `j`, or the equal-power
/// fade if they play from different audio. The same copy (a looping
/// clip's one region, pass to pass) keeps its join.
pub(crate) fn join_sources(j: Join, sa: Option<usize>, sb: Option<usize>) -> Join {
    if sa == sb {
        j
    } else {
        Join::Fade
    }
}

/// What a copy is made from, taken on the audio thread or from a file:
/// its key, the recording's frames it needs (interleaved, the
/// recording's channels: [`CopySpec::frames_from`] for
/// [`CopySpec::frames_in`] frames, silence where the recording has none),
/// and how to make it. Cheap: [`CopySpec::start`] does the work, on the
/// worker.
pub struct CopySpec {
    pub key: CopyKey,
    pub channels: u8,
    pub audio: Vec<f32>,
}

impl CopySpec {
    /// The first recording frame a copy for `key` is made from.
    pub fn frames_from(key: &CopyKey) -> i64 {
        match key.ends {
            Ends::Edges => key.src - pad_frames(key.rate) as i64,
            Ends::Wrapped => key.src,
        }
    }

    /// The recording frames a copy for `key` is made from.
    pub fn frames_in(key: &CopyKey) -> usize {
        match key.ends {
            Ends::Edges => key.reach as usize + 2 * pad_frames(key.rate),
            Ends::Wrapped => key.src_len as usize,
        }
    }

    /// What the job holds at most, in bytes, for the scheduler's promise.
    pub fn need(key: &CopyKey, channels: u8) -> u64 {
        let input = match key.ends {
            Ends::Edges => Self::frames_in(key),
            Ends::Wrapped => key.src_len as usize + 4 * pad_frames(key.rate),
        } as u64;
        let output = key.bytes(usize::from(channels)) / (4 * u64::from(channels.max(1)));
        // The input the job holds, besides this spec's, which it is made
        // from.
        mpfree_stretch::need(input, channels, output, 0.0) + input * u64::from(channels) * 4
    }

    /// The job that makes the copy: it finds the hits as it starts, so
    /// call it on the worker.
    pub fn start(self) -> Result<CopyJob, StretchError> {
        let key = self.key;
        let ch = usize::from(self.channels);
        if ch == 0 || self.audio.len() != Self::frames_in(&key) * ch {
            return Err(StretchError::Empty);
        }
        let k = 1.0 / key.speed();
        let phase = f64::from(f32::from_bits(key.phase));
        match key.ends {
            Ends::Edges => {
                let pad = pad_frames(key.rate) as f64;
                let total = self.audio.len() / ch;
                let lead = (pad * k).round();
                // The region's start (its frame and fraction) lands on
                // output frame `lead` and its fraction, and its recording
                // runs on from there at the tempo's speed.
                let p1 = (pad + phase, lead + phase);
                let p2 = (p1.0 + key.reach as f64, p1.1 + key.reach as f64 * k);
                let p3 = (total as f64, p2.1 + (total as f64 - p2.0) * k);
                let map = TimeMap::new(vec![(0.0, 0.0), p1, p2, p3])?;
                let job = Job::new(
                    Arc::from(self.audio),
                    self.channels,
                    key.rate,
                    map,
                    settings(key.rate),
                )?;
                Ok(CopyJob {
                    key,
                    channels: self.channels,
                    job,
                    lead: lead as i64,
                    before: 0,
                })
            }
            Ends::Wrapped => {
                // Two windows of the loop either side, taken round it; the
                // middle kept and its start crossfaded from what follows
                // its end ([`CopyJob::finish`]), as the looper's prints.
                let len = self.audio.len() / ch;
                let wrap = pad_frames(key.rate);
                let total = len + 2 * wrap;
                let mut data = Vec::with_capacity(total * ch);
                for i in 0..total {
                    let q = (i + len * (wrap / len + 1) - wrap) % len;
                    data.extend_from_slice(&self.audio[q * ch..(q + 1) * ch]);
                }
                let f = key.out as f64;
                let r = f / len as f64;
                let before = (wrap as f64 * r).round().max(1.0);
                let (w, l) = (wrap as f64, len as f64);
                let map = TimeMap::new(vec![
                    (0.0, 0.0),
                    (w, before),
                    (w + l, before + f),
                    (2.0 * w + l, 2.0 * before + f),
                ])?;
                let job = Job::new(
                    Arc::from(data),
                    self.channels,
                    key.rate,
                    map,
                    settings(key.rate),
                )?;
                Ok(CopyJob {
                    key,
                    channels: self.channels,
                    job,
                    lead: 0,
                    before: before as usize,
                })
            }
        }
    }
}

/// One copy being made ([`CopySpec::start`]), in steps.
pub struct CopyJob {
    key: CopyKey,
    channels: u8,
    job: Job,
    lead: i64,
    /// A wrapped copy's output frames before the loop.
    before: usize,
}

impl CopyJob {
    pub fn key(&self) -> &CopyKey {
        &self.key
    }

    /// Does about `frames` frames of work; true once done.
    pub fn run(&mut self, frames: usize) -> bool {
        self.job.run(frames).finished
    }

    /// How far it has got, 0 to 1.
    pub fn progress(&self) -> f32 {
        self.job.progress().done
    }

    /// The copy, once [`CopyJob::run`] is done.
    pub fn finish(self) -> Option<Copy> {
        let ch = usize::from(self.channels);
        let mut audio = self.job.into_output()?;
        if self.key.ends == Ends::Wrapped {
            let frames = self.key.out as usize;
            let (from, n) = (ch * self.before, ch * frames);
            let seam = SEAM.min(frames / 2).min(self.before);
            if audio.len() < from + n + ch * seam {
                return None;
            }
            for i in 0..seam {
                let w = (i as f32 + 0.5) / seam as f32;
                for c in 0..ch {
                    let after = audio[from + n + ch * i + c];
                    let start = &mut audio[from + ch * i + c];
                    *start = after * (1.0 - w) + *start * w;
                }
            }
            audio.drain(..from);
            audio.truncate(n);
        }
        Some(Copy {
            key: self.key,
            data: Arc::from(audio),
            channels: self.channels,
            lead: self.lead,
        })
    }
}

/// Makes the copy for `spec` at once (for tests and offline use).
pub fn make(spec: CopySpec) -> Result<Copy, StretchError> {
    let mut job = spec.start()?;
    while !job.run(1 << 16) {}
    job.finish().ok_or(StretchError::Empty)
}

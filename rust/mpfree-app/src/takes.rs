//! Freeze and Print, from the mixer's strip menu: a strip recorded as it
//! sounds over its loop, then played in place of its inserts and its
//! instrument (Freeze), or handed to the screen to save as a new sample
//! (Print). The engine is mpfree-fx's [`Print`] and [`Mixer::freeze`];
//! this is the app's part.
//!
//! - **The loop.** A track's own instrument takes the clip playing on its
//!   track. A pad channel, Drums and Synth take the tracks that play on
//!   the pads (those without an instrument of their own): the longest
//!   clip playing there, which every other clip there must fit into a
//!   whole number of times, so the sum repeats with it (a 1-bar and a
//!   4-bar clip freeze as 4 bars; 3 and 4 bars don't freeze). The looper
//!   and the audio tracks follow no clip, so they don't freeze. A print of
//!   any other strip (a return, a bus, the master, the line input, the
//!   looper, the audio tracks) takes the longest clip playing on any
//!   track, and a print needs no fit: it's a sample, not a loop.
//! - **One take at a time,** on its own capture point ([`TAKE_CAPTURE`]),
//!   from the strip's Before fader point: its effects, not its fader, pan
//!   or sends. A freeze is steady (the loop plays once through first, so
//!   its reverb and echoes are already in the take's first frames) and cut
//!   at the loop's end; a print starts at the loop's next start and runs on
//!   until the strip is silent ([`PRINT_TAIL_MS`] at most). The take's
//!   memory is made on the builder thread (control.rs), and a take the
//!   strip is done with goes back there to be dropped.
//! - **Thawing by itself.** A frozen strip plays the loop it took. When
//!   what it took changes, it thaws and says why: another clip launched or
//!   stopped, a clip edited, a track muted or unmuted (a freeze of a
//!   track's own instrument just goes quiet while its track is muted), the
//!   track's instrument changed, or an effect put in, taken out or
//!   switched on or off. A track's clip launched again at another place
//!   moves the frozen loop with it ([`Mixer::set_frozen_origin`]). Turning
//!   an effect's controls on a frozen strip changes nothing until it
//!   thaws. Opening a project thaws everything; frozen strips are not
//!   saved.
//!
//! Every call here runs on the audio loop. Words for the screen are made
//! only when something happens (an ask, a take ending, a thaw).

use mpfree_fx::{
    CapturePoint, Frozen, Mixer, Print, PrintError, PrintJob, PrintState, Printed, StripId, Tail,
};
use mpfree_node::Ctx;
use mpfree_play::NowPlaying;
use mpfree_transport::PPQ;

use crate::mixing::{self, DRUMS, SYNTH, TRACKS};

/// The capture point takes record from: the one below the sampler's.
pub const TAKE_CAPTURE: usize = mixing::SAMPLER_CAPTURE - 1;
/// The longest loop a take records, in seconds (about 22 MB of memory).
pub const MAX_LOOP_S: f64 = 64.0;
/// A print runs on after its loop until the strip is silent, at most this
/// long.
pub const PRINT_TAIL_MS: f32 = 10_000.0;
/// Strips frozen at once (every track and pad channel fits).
pub const MAX_FROZEN: usize = 160;

/// Freeze or Print.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Freeze,
    Print,
}

/// What a strip's sound comes from, for its loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    /// A track's own instrument: track `k`'s clip.
    Track(usize),
    /// The tracks that play on the pads (no instrument of their own).
    Pads,
    /// Any track: a print of a strip that follows no clip of its own.
    All,
}

/// The feed strip `s` follows.
pub fn feed(s: StripId) -> Feed {
    match s {
        StripId::Track(t) if t >= TRACKS => Feed::Track(t - TRACKS),
        StripId::Track(t) if t == DRUMS || t == SYNTH => Feed::Pads,
        StripId::Pad(_) => Feed::Pads,
        _ => Feed::All,
    }
}

/// Whether strip `s` can be frozen: it follows a clip (see the module
/// docs).
pub fn freezable(s: StripId) -> bool {
    feed(s) != Feed::All
}

/// The song as a take needs it: what each track plays now.
pub struct Song<'a> {
    /// Tracks in the project.
    pub tracks: usize,
    /// Tracks with an instrument of their own, a bit each.
    pub own: u128,
    /// The tempo now.
    pub bpm: f64,
    /// What track `t` plays now ([`crate::time::TimeHost::now_playing`]).
    pub now: &'a dyn Fn(usize) -> Option<NowPlaying>,
    /// Which instrument track `t` has of its own, as a number (0: none).
    pub instrument: &'a dyn Fn(usize) -> u64,
}

impl Song<'_> {
    /// Whether track `t` is part of `feed`.
    fn feeds(&self, feed: Feed, t: usize) -> bool {
        match feed {
            Feed::Track(k) => t == k,
            Feed::Pads => t < 128 && self.own & (1 << t) == 0,
            Feed::All => true,
        }
    }

    /// The tracks of `feed` that play now, with what they play.
    fn playing(&self, feed: Feed) -> impl Iterator<Item = (usize, NowPlaying)> + '_ {
        let range = match feed {
            Feed::Track(k) => k..k + 1,
            _ => 0..self.tracks,
        };
        range
            .filter(move |&t| self.feeds(feed, t))
            .filter_map(|t| (self.now)(t).map(|n| (t, n)))
    }

    /// What strip `s` (following `feed`) took, as one number: the clips
    /// its tracks play (slot, length, edits, mute and, for several tracks,
    /// where each lines up), its effects (each one's id and whether it is
    /// on) and, for a track's own instrument, which one it is. A change
    /// thaws it.
    fn sig(&self, s: StripId, feed: Feed, mixer: &Mixer) -> u64 {
        let mut h = Fnv::new();
        if let Some(c) = mixer.chain(s) {
            for slot in 0..c.len() {
                h.add_str(c.node(slot).map_or("", |n| n.info().id));
                h.add(u64::from(c.is_on(slot)));
            }
        }
        if let Feed::Track(k) = feed {
            h.add((self.instrument)(k));
        }
        for (t, n) in self.playing(feed) {
            h.add(t as u64);
            h.add(n.slot as u64);
            h.add(n.song_len.to_bits());
            h.add(n.stamp);
            if !matches!(feed, Feed::Track(_)) {
                h.add(n.origin as u64);
                h.add(u64::from(n.muted));
            }
        }
        h.0
    }

    /// Where a track's own clip starts (its origin) and whether it is
    /// muted: the two changes a frozen track follows rather than thaws.
    fn track_now(&self, feed: Feed) -> Option<(i64, bool)> {
        match feed {
            Feed::Track(k) => (self.now)(k).map(|n| {
                let len = (n.song_len.round() as i64).max(1);
                (n.origin.rem_euclid(len), n.muted)
            }),
            _ => None,
        }
    }

    /// The loop a take of `feed` records: (start beat, beats), or why
    /// there is none. A freeze needs every clip to fit the longest.
    fn loop_of(&self, feed: Feed, kind: Kind, name: &str) -> Result<(f64, f64), String> {
        let mut longest: Option<NowPlaying> = None;
        let mut lens = [0f64; 128];
        let mut n = 0;
        for (_, p) in self
            .playing(feed)
            .filter(|(_, p)| !p.muted && p.song_len > 0.0)
        {
            if n < lens.len() {
                lens[n] = p.song_len;
                n += 1;
            }
            if longest.is_none_or(|l| p.song_len > l.song_len) {
                longest = Some(p);
            }
        }
        let Some(l) = longest else {
            return Err(match feed {
                Feed::Track(_) => {
                    format!("{name}: nothing plays on its track; launch a clip first")
                }
                Feed::Pads => format!("{name}: nothing plays on the pads; launch a clip first"),
                Feed::All => format!("{name}: no clip plays; launch one first"),
            });
        };
        let ppq = f64::from(PPQ);
        if kind == Kind::Freeze {
            for &len in &lens[..n] {
                let r = l.song_len / len;
                if (r - r.round()).abs() > 1e-6 {
                    return Err(format!(
                        "{name}: the clips on the pads don't loop together ({} and {} beats); freeze needs each to fit the longest a whole number of times",
                        beats_words(len / ppq),
                        beats_words(l.song_len / ppq)
                    ));
                }
            }
        }
        let beats = l.song_len / ppq;
        let seconds = beats * 60.0 / self.bpm.max(1.0);
        if seconds > MAX_LOOP_S {
            return Err(format!(
                "{name}: its loop is {seconds:.0} s at this tempo; a take is at most {MAX_LOOP_S:.0} s"
            ));
        }
        // The loop's start nearest the song's: its origin is placed far
        // back, a whole number of passes before where it was launched.
        Ok(((l.origin as f64).rem_euclid(l.song_len) / ppq, beats))
    }
}

fn beats_words(b: f64) -> String {
    if (b - b.round()).abs() < 1e-9 {
        format!("{}", b.round() as i64)
    } else {
        format!("{b:.2}")
    }
}

/// FNV-1a over u64s, for [`Song::sig`].
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn add(&mut self, x: u64) {
        self.bytes(&x.to_le_bytes());
    }
    fn add_str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
        self.add(s.len() as u64);
    }
    fn bytes(&mut self, b: &[u8]) {
        for &b in b {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// A name as a number, for [`Song::instrument`].
pub fn id_number(id: &str) -> u64 {
    let mut h = Fnv::new();
    h.add_str(id);
    h.0
}

/// The take under way.
struct Take {
    strip: StripId,
    kind: Kind,
    feed: Feed,
    job: PrintJob,
    /// What it follows, when it was asked for.
    sig: u64,
    /// The strip's name, for the words and the file.
    name: String,
    /// Its memory, once the builder hands it back.
    print: Option<Print>,
}

/// A frozen strip and what it took.
#[derive(Clone, Copy)]
struct Iced {
    strip: StripId,
    feed: Feed,
    sig: u64,
    /// A track's own clip: where it started and whether it was muted.
    track: Option<(i64, bool)>,
}

/// What [`Takes::period`] hands the loop: at most one a period.
pub enum Said {
    /// A line for the screen.
    Note(String),
    /// A print to save as a new sample, named after its strip.
    Printed(String, Box<Printed>),
}

/// Where a strip's take is, for the mixer screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TakeView {
    pub print: bool,
    /// 0 to 1 while it records; None while it waits for the loop's start.
    pub done: Option<f32>,
}

/// What the take host needs from outside each call: the builder thread
/// that makes a take's memory and drops what is done with (None in the
/// tests: then here).
pub trait Memory {
    /// Asks for a print of `seconds` at `rate`. False if it can't now.
    fn ask(&mut self, rate: u32, seconds: f32) -> bool;
    /// A print asked for, once made.
    fn made(&mut self) -> Option<Print>;
    fn drop_frozen(&mut self, f: Frozen);
    fn drop_print(&mut self, p: Print);
}

/// Freeze and Print (see the module docs).
pub struct Takes {
    take: Option<Take>,
    iced: Vec<Iced>,
    rate: u32,
}

impl Takes {
    /// Made before the sound starts (it allocates).
    pub fn new(rate: u32) -> Takes {
        Takes {
            take: None,
            iced: Vec::with_capacity(MAX_FROZEN),
            rate,
        }
    }

    /// The menu's Freeze or Print on strip `s`, named `name`. Returns the
    /// line for the screen.
    pub fn ask(
        &mut self,
        kind: Kind,
        s: StripId,
        name: &str,
        song: &Song,
        mixer: &Mixer,
        mem: &mut dyn Memory,
    ) -> String {
        if let Some(t) = &self.take {
            return format!(
                "{name}: one take at a time, and {} is {}",
                t.name,
                if t.kind == Kind::Freeze {
                    "freezing"
                } else {
                    "printing"
                }
            );
        }
        let f = feed(s);
        if kind == Kind::Freeze && (!freezable(s) || mixer.frozen(s).is_some()) {
            return format!("{name} can't be frozen");
        }
        let (start, beats) = match song.loop_of(f, kind, name) {
            Ok(l) => l,
            Err(why) => return why,
        };
        let loop_s = (beats * 60.0 / song.bpm.max(1.0)) as f32;
        let (job, seconds) = match kind {
            Kind::Freeze => (PrintJob::freeze(start, beats), loop_s + 0.5),
            Kind::Print => (
                PrintJob {
                    start,
                    beats,
                    steady: false,
                    tail: Tail::UntilSilent {
                        max_ms: PRINT_TAIL_MS,
                    },
                },
                loop_s + PRINT_TAIL_MS / 1000.0 + 0.5,
            ),
        };
        if !mem.ask(self.rate, seconds) {
            return format!("{name}: busy, try again in a moment");
        }
        self.take = Some(Take {
            strip: s,
            kind,
            feed: f,
            job,
            sig: song.sig(s, f, mixer),
            name: name.to_string(),
            print: None,
        });
        let what = match kind {
            Kind::Freeze => "freezing",
            Kind::Print => "printing",
        };
        format!(
            "{name}: {what} its {}-beat loop from its next start (the song must play)",
            beats_words(beats)
        )
    }

    /// The menu's Thaw. Returns the line for the screen.
    pub fn thaw(&mut self, s: StripId, name: &str, mixer: &mut Mixer) -> String {
        self.iced.retain(|i| i.strip != s);
        if mixer.thaw(s) {
            format!("{name}: thawed")
        } else {
            format!("{name} isn't frozen")
        }
    }

    /// The menu's Cancel: the take under way on `s` stops; nothing
    /// changes.
    pub fn cancel(
        &mut self,
        s: StripId,
        mixer: &mut Mixer,
        mem: &mut dyn Memory,
    ) -> Option<String> {
        if self.take.as_ref().is_none_or(|t| t.strip != s) {
            return None;
        }
        let t = self.end(mixer, mem)?;
        Some(format!("{}: take stopped", t.name))
    }

    /// Ends the take under way, if any: its capture point freed and its
    /// memory sent to be dropped.
    fn end(&mut self, mixer: &mut Mixer, mem: &mut dyn Memory) -> Option<Take> {
        let mut t = self.take.take()?;
        mixer.set_capture(TAKE_CAPTURE, None);
        if let Some(p) = t.print.take() {
            mem.drop_print(p);
        }
        Some(t)
    }

    /// Everything thawed at once and the take stopped (a project opens).
    pub fn clear(&mut self, mixer: &mut Mixer, mem: &mut dyn Memory) {
        let _ = self.end(mixer, mem);
        for i in self.iced.drain(..) {
            mixer.thaw(i.strip);
        }
    }

    /// The take on strip `s`, for the screen.
    pub fn view(&self, s: StripId) -> Option<TakeView> {
        let t = self.take.as_ref().filter(|t| t.strip == s)?;
        let done = t.print.as_ref().and_then(|p| match p.state() {
            PrintState::Recording | PrintState::Tail | PrintState::Done => Some(p.progress()),
            _ => None,
        });
        Some(TakeView {
            print: t.kind == Kind::Print,
            done,
        })
    }

    /// One period, after the mixer ran with `ctx`: the take records, ends
    /// or fails; frozen strips follow their clips or thaw; thawed takes go
    /// to be dropped. At most one thing to say a period.
    pub fn period(
        &mut self,
        ctx: &Ctx,
        song: &Song,
        mixer: &mut Mixer,
        mem: &mut dyn Memory,
    ) -> Option<Said> {
        while let Some(f) = mixer.pop_thawed() {
            mem.drop_frozen(f);
        }
        if let Some(said) = self.follow(song, mixer) {
            return Some(said);
        }
        self.record(ctx, song, mixer, mem)
    }

    /// Frozen strips: thaw what changed, follow a relaunched clip, go
    /// quiet with a muted track.
    fn follow(&mut self, song: &Song, mixer: &mut Mixer) -> Option<Said> {
        for k in 0..self.iced.len() {
            let i = self.iced[k];
            if mixer.frozen(i.strip).is_none() {
                // Thawed from elsewhere (the mixer refused it, a project).
                self.iced.swap_remove(k);
                return None;
            }
            let now = song.track_now(i.feed);
            if song.sig(i.strip, i.feed, mixer) != i.sig || (i.track.is_some() && now.is_none()) {
                mixer.thaw(i.strip);
                self.iced.swap_remove(k);
                let name = mixing::name(i.strip);
                return Some(Said::Note(format!("{name}: thawed, what it plays changed")));
            }
            if let (Some((was, was_muted)), Some((origin, muted))) = (i.track, now) {
                if origin != was {
                    mixer.set_frozen_origin(i.strip, origin as f64 / f64::from(PPQ));
                }
                if muted != was_muted {
                    mixer.set_frozen_quiet(i.strip, muted);
                }
                self.iced[k].track = Some((origin, muted));
            }
        }
        None
    }

    /// The take: its memory arriving, recording, ending.
    fn record(
        &mut self,
        ctx: &Ctx,
        song: &Song,
        mixer: &mut Mixer,
        mem: &mut dyn Memory,
    ) -> Option<Said> {
        let t = self.take.as_mut()?;
        if t.print.is_none() {
            let mut p = mem.made()?;
            if !mixer.set_capture(TAKE_CAPTURE, Some((t.strip, CapturePoint::BeforeFader)))
                || p.arm(t.job).is_err()
            {
                mem.drop_print(p);
                let t = self.end(mixer, mem)?;
                return Some(Said::Note(format!("{}: the take couldn't start", t.name)));
            }
            t.print = Some(p);
            // Recording starts with the next period's block.
            return None;
        }
        if song.sig(t.strip, t.feed, mixer) != t.sig {
            let t = self.end(mixer, mem)?;
            return Some(Said::Note(format!(
                "{}: take stopped, what it plays changed",
                t.name
            )));
        }
        let p = t.print.as_mut()?;
        let latency = mixer.capture_latency(TAKE_CAPTURE).unwrap_or(0);
        p.process(ctx, mixer.capture(TAKE_CAPTURE), latency);
        match p.state() {
            PrintState::Done => {
                let printed = p.take()?;
                let mut t = self.end(mixer, mem)?;
                // The spent print's memory moved into the take; what is
                // left is empty.
                let _ = t.print.take();
                Some(self.landed(t, printed, song, mixer, mem))
            }
            PrintState::Failed(e) => {
                let t = self.end(mixer, mem)?;
                Some(Said::Note(format!(
                    "{}: take stopped, {}",
                    t.name,
                    failure(e)
                )))
            }
            _ => None,
        }
    }

    /// A finished take: frozen in place, or handed on to be saved.
    fn landed(
        &mut self,
        t: Take,
        printed: Printed,
        song: &Song,
        mixer: &mut Mixer,
        mem: &mut dyn Memory,
    ) -> Said {
        match t.kind {
            Kind::Print => Said::Printed(t.name, Box::new(printed)),
            Kind::Freeze => {
                let frozen = printed.frozen();
                if self.iced.len() >= self.iced.capacity() {
                    mem.drop_frozen(frozen);
                    return Said::Note(format!("{}: too many strips frozen", t.name));
                }
                match mixer.freeze(t.strip, frozen) {
                    Ok(()) => {
                        let track = song.track_now(t.feed);
                        if let Some((_, true)) = track {
                            mixer.set_frozen_quiet(t.strip, true);
                        }
                        self.iced.push(Iced {
                            strip: t.strip,
                            feed: t.feed,
                            sig: t.sig,
                            track,
                        });
                        Said::Note(format!(
                            "{}: frozen; its effects{} rest until you thaw it",
                            t.name,
                            if matches!(t.feed, Feed::Track(_)) {
                                " and instrument"
                            } else {
                                ""
                            }
                        ))
                    }
                    Err(f) => {
                        mem.drop_frozen(f);
                        Said::Note(format!("{} can't be frozen", t.name))
                    }
                }
            }
        }
    }
}

/// Why a take failed, in words.
fn failure(e: PrintError) -> &'static str {
    match e {
        PrintError::Stopped => "the song stopped",
        PrintError::Jumped => "the song jumped",
        PrintError::TempoChanged => "the tempo changed",
        PrintError::TooLong => "it ran too long",
        PrintError::NoSignal => "the strip gave nothing",
        PrintError::Refused => "it couldn't start",
    }
}

/// [`Memory`] with no builder thread (the tests): made and dropped here.
#[derive(Default)]
pub struct Here {
    made: Option<Print>,
}

impl Memory for Here {
    fn ask(&mut self, rate: u32, seconds: f32) -> bool {
        self.made = Some(Print::new(rate, seconds));
        true
    }
    fn made(&mut self) -> Option<Print> {
        self.made.take()
    }
    fn drop_frozen(&mut self, _: Frozen) {}
    fn drop_print(&mut self, _: Print) {}
}

/// The folder in Samples prints go to.
pub const PRINTS: &str = "Prints";

/// Writes a print as a 32-bit float WAV named `file` in Samples › Prints
/// (made if it isn't there): on the screen's thread for it, never the
/// audio loop. Returns where it went.
pub fn save_print(
    lib: &mpfree_library::Library,
    file: &str,
    p: &Printed,
) -> std::io::Result<mpfree_library::Loc> {
    use mpfree_library::{Loc, Place};
    let samples = Loc::top(Place::Samples);
    let prints = samples.join(PRINTS)?;
    if lib.list(&prints).is_err() {
        lib.new_folder(&samples, PRINTS)?;
    }
    lib.write_new(&prints, file, &p.wav())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixing::{MixHost, LOOP};
    use crate::time::TimeHost;
    use crate::{PERIOD, RATE};
    use mpfree_mixer_ui::MixAction;
    use mpfree_seq::{Clip, Project, Track};
    use mpfree_time_ui::TimeAction;
    use mpfree_transport::Command;

    const BAR: i64 = 3840;
    /// A capture point the tests watch strips on.
    const WATCH: usize = 10;

    struct Quiet;
    impl crate::time::Pads for Quiet {
        fn hit_at(&mut self, _: usize, _: u8, _: u32) -> bool {
            true
        }
        fn release_at(&mut self, _: usize, _: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _: usize, _: u8, _: u32) {}
    }

    /// The synth's sound: a burst on every beat that depends only on the
    /// song beat, so every loop of whole beats repeats it.
    fn sig(beat: f64) -> f32 {
        let f = beat.rem_euclid(1.0) as f32;
        0.3 * (std::f32::consts::TAU * 8.0 * f).sin() * (-4.0 * f).exp()
    }

    /// Track 0: a 1-bar clip in slot 0 and a 4-bar one in slot 1; track 1:
    /// a 3-bar clip and a 2-bar one.
    fn project() -> Project {
        let mut p = Project::new();
        let mut a = Track::new("Keys", 0);
        a.slots[0] = Some(Clip::new("one", BAR));
        a.slots[1] = Some(Clip::new("four", 4 * BAR));
        let mut b = Track::new("Bass", 1);
        b.slots[0] = Some(Clip::new("three", 3 * BAR));
        b.slots[1] = Some(Clip::new("two", 2 * BAR));
        p.tracks.push(a);
        p.tracks.push(b);
        p
    }

    struct Rig {
        time: TimeHost,
        mix: MixHost,
        i: u64,
        /// The synth plays while this is true (and the song rolls).
        tone: bool,
        /// The mixer track the sound goes to instead of the synth's (a
        /// track's own instrument's, standing in for it).
        feed: Option<usize>,
        /// Periods the mixer didn't want the synth's sound.
        unfed: u32,
        said: Vec<String>,
        printed: Vec<Printed>,
    }

    impl Rig {
        fn new() -> Rig {
            let mut r = Rig {
                time: TimeHost::with_project(RATE, 120.0, project()).unwrap(),
                mix: MixHost::new(RATE, 0.0, 0.0),
                i: 0,
                tone: true,
                feed: None,
                unfed: 0,
                said: Vec::new(),
                printed: Vec::new(),
            };
            assert!(r.mix.mixer.set_capture(
                WATCH,
                Some((StripId::Track(SYNTH), CapturePoint::BeforeFader))
            ));
            r.period();
            r
        }

        fn launch(&mut self, track: usize, slot: usize) {
            self.time.act(TimeAction::Launch { track, slot }).unwrap();
        }

        fn ask(&mut self, a: MixAction) -> String {
            let (tracks, bpm) = (self.time.project().tracks.len(), self.time.info().bpm);
            let time = &self.time;
            self.mix
                .take_action(&a, tracks, bpm, &|t| time.now_playing(t))
                .unwrap()
        }

        /// One period as player_session runs it; returns the synth's
        /// Before fader block (watched).
        fn period(&mut self) -> Vec<f32> {
            self.period_at().0
        }

        /// [`Rig::period`], with the song beat the block starts on.
        fn period_at(&mut self) -> (Vec<f32>, f64) {
            self.time.advance(PERIOD as u32);
            self.time.period(PERIOD as u32, &mut Quiet);
            let ctx = crate::mixing::ctx(RATE, self.i * PERIOD as u64, 120.0, self.time.block());
            let fpb = 60.0 * f64::from(RATE) / ctx.bpm;
            let to = self.feed.unwrap_or(SYNTH);
            let wanted = self.mix.mixer.needs_input(StripId::Track(to));
            if !wanted {
                self.unfed += 1;
            }
            let sound: [f32; PERIOD] = std::array::from_fn(|k| {
                if self.tone && ctx.playing && wanted {
                    sig(ctx.beats + k as f64 / fpb)
                } else {
                    0.0
                }
            });
            let synth = if self.feed.is_some() {
                if let Some((l, r)) = self.mix.mixer.track_input(to) {
                    l[..PERIOD].copy_from_slice(&sound);
                    r[..PERIOD].copy_from_slice(&sound);
                }
                [0.0; PERIOD]
            } else {
                sound
            };
            let z = [0.0f32; PERIOD];
            let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
            self.mix.period(
                &ctx,
                (&synth, &synth),
                (&z, &z),
                (&z, &z),
                (&z, &z),
                &mut l,
                &mut r,
            );
            let watched = self
                .mix
                .mixer
                .capture(WATCH)
                .map_or(Vec::new(), |(l, _)| l.to_vec());
            let tracks = self.time.project().tracks.len();
            let time = &self.time;
            match self
                .mix
                .takes_period(&ctx, tracks, &|t| time.now_playing(t))
            {
                Some(Said::Note(l)) => self.said.push(l),
                Some(Said::Printed(name, p)) => {
                    self.said.push(format!("printed {name}"));
                    self.printed.push(*p);
                }
                None => {}
            }
            self.i += 1;
            (watched, ctx.beats)
        }

        /// Runs until the takes say something with `word` in it.
        fn until(&mut self, word: &str, max_s: f64) -> String {
            let max = (max_s * f64::from(RATE) / PERIOD as f64) as usize;
            for _ in 0..max {
                self.period();
                if let Some(l) = self.said.iter().find(|l| l.contains(word)) {
                    return l.clone();
                }
            }
            panic!("never said {word:?}: {:?}", self.said);
        }

        fn beats(&self) -> f64 {
            self.time.info().position as f64 / f64::from(PPQ)
        }
    }

    #[test]
    fn freeze_takes_the_pads_loop_then_plays_it_in_place_of_the_synth() {
        let mut r = Rig::new();
        r.launch(0, 0);
        r.time.command(Command::Play);
        r.period();
        let line = r.ask(MixAction::Freeze(StripId::Track(SYNTH)));
        assert!(line.contains("Synth: freezing its 4-beat loop"), "{line}");
        // Busy: one take at a time.
        let busy = r.ask(MixAction::Print(StripId::Track(LOOP)));
        assert!(busy.contains("one take at a time"), "{busy}");
        assert!(r.mix.takes.view(StripId::Track(SYNTH)).is_some());
        // A steady take: a whole bar plays first, then a bar is taken.
        let done = r.until("frozen", 8.0);
        assert!(done.starts_with("Synth: frozen"), "{done}");
        assert!(r.mix.takes.view(StripId::Track(SYNTH)).is_none());
        assert!(r.beats() >= 8.0, "two bars went by: {}", r.beats());
        // After the crossfade the synth isn't wanted, and the strip plays
        // the take: just what the synth would have played.
        for _ in 0..20 {
            r.period();
        }
        let unfed = r.unfed;
        for _ in 0..400 {
            let (got, b0) = r.period_at();
            for (k, &v) in got.iter().enumerate() {
                let want = sig(b0 + k as f64 / 22_050.0);
                assert!(
                    (v - want).abs() < 1e-4,
                    "frame {k} at beat {b0}: {v} vs {want}"
                );
            }
        }
        assert_eq!(r.unfed - unfed, 400, "the synth rested");
        // The menu offers Thaw; a thaw brings the synth back.
        let st = r.mix.poll_state(std::time::Instant::now());
        assert!(st.is_none() || st.unwrap().takes);
        let thawed = r.ask(MixAction::Thaw(StripId::Track(SYNTH)));
        assert_eq!(thawed, "Synth: thawed");
        r.period();
        assert!(r.mix.mixer.needs_input(StripId::Track(SYNTH)));
    }

    #[test]
    fn a_frozen_strip_thaws_when_what_it_took_changes() {
        let mut r = Rig::new();
        r.launch(0, 0);
        r.time.command(Command::Play);
        r.period();
        r.ask(MixAction::Freeze(StripId::Pad(3)));
        r.until("frozen", 8.0);
        assert!(r.mix.mixer.frozen(StripId::Pad(3)).is_some());
        // Another clip on a track that plays on the pads: it thaws.
        r.launch(1, 1);
        let line = r.until("thawed", 6.0);
        assert_eq!(line, "Pad 4: thawed, what it plays changed");
        for _ in 0..10 {
            r.period();
        }
        assert!(r.mix.mixer.frozen(StripId::Pad(3)).is_none());
        assert!(r.mix.mixer.needs_input(StripId::Pad(3)));
        // Frozen again, then a step put in the playing clip: it thaws.
        r.said.clear();
        r.ask(MixAction::Freeze(StripId::Pad(3)));
        r.until("frozen", 30.0);
        r.said.clear();
        r.time
            .step_action(crate::time::StepAction::Tap {
                pad: 0,
                step: 0,
                velocity: 100,
            })
            .unwrap();
        let line = r.until("thawed", 1.0);
        assert_eq!(line, "Pad 4: thawed, what it plays changed");
    }

    #[test]
    fn a_tracks_own_instrument_rests_while_frozen_and_goes_quiet_while_muted() {
        let mut r = Rig::new();
        let strip = StripId::Track(TRACKS);
        let job = crate::control::InstrumentJob {
            track: 0,
            id: "mpfree.poly",
            values: crate::synth::engine_node("mpfree.poly", RATE)
                .unwrap()
                .params()
                .iter()
                .map(|p| p.default)
                .collect(),
            pads: std::array::from_fn(|p| mpfree_arp::scale::pads::Pad::single(60 + p as i16)),
            tuning: mpfree_node::Tuning::standard(),
        };
        assert!(r.mix.build_instrument(job));
        assert!(matches!(
            r.mix.poll_instruments(),
            Some(crate::mixing::Landed::Playing { track: 0, .. })
        ));
        assert!(r
            .mix
            .mixer
            .set_capture(WATCH, Some((strip, CapturePoint::BeforeFader))));
        r.feed = Some(TRACKS);
        r.launch(0, 0);
        r.time.command(Command::Play);
        r.period();
        // The pads' tracks don't include it, and it takes its own clip.
        let line = r.ask(MixAction::Freeze(strip));
        assert!(line.contains("Track 1: freezing its 4-beat loop"), "{line}");
        let done = r.until("frozen", 8.0);
        assert_eq!(
            done,
            "Track 1: frozen; its effects and instrument rest until you thaw it"
        );
        for _ in 0..50 {
            r.period();
        }
        assert!(
            r.mix.instruments.stats(0).rested >= 40,
            "the instrument rests"
        );
        let loud = |b: &[f32]| b.iter().fold(0f32, |m, v| m.max(v.abs()));
        // Its track muted: the take goes quiet, still frozen.
        r.time.toggle_mute(0).unwrap();
        for _ in 0..40 {
            r.period();
        }
        assert_eq!(loud(&r.period()), 0.0);
        assert!(r.mix.mixer.frozen(strip).is_some());
        // Unmuted: it plays again, in step.
        r.time.toggle_mute(0).unwrap();
        let mut peak = 0f32;
        for _ in 0..200 {
            peak = peak.max(loud(&r.period()));
        }
        assert!(peak > 0.1, "{peak}");
        assert!(r.said.iter().all(|l| !l.contains("thawed")), "{:?}", r.said);
        // An effect put in: it thaws.
        let n = r.mix.mixer.chain(strip).map_or(0, |c| c.len());
        r.mix.add_insert(strip, "mpfree.fx.compressor").unwrap();
        for _ in 0..20 {
            r.period();
            r.mix.poll_built();
            if r.mix.mixer.chain(strip).map_or(0, |c| c.len()) > n {
                break;
            }
        }
        let line = r.until("thawed", 1.0);
        assert_eq!(line, "Track 1: thawed, what it plays changed");
    }

    #[test]
    fn a_take_stops_when_the_song_does() {
        let mut r = Rig::new();
        r.launch(0, 0);
        r.time.command(Command::Play);
        r.period();
        r.ask(MixAction::Freeze(StripId::Track(SYNTH)));
        // Stopped while it waits for the loop: it waits on.
        for _ in 0..400 {
            r.period();
        }
        r.time.command(Command::Stop);
        for _ in 0..100 {
            r.period();
        }
        assert!(r.said.is_empty(), "{:?}", r.said);
        assert!(r.mix.takes.view(StripId::Track(SYNTH)).is_some());
        // Rolling again, a bar plays through, the take starts, and a stop
        // then ends it.
        r.time.command(Command::Play);
        while r
            .mix
            .takes
            .view(StripId::Track(SYNTH))
            .and_then(|v| v.done)
            .is_none()
        {
            r.period();
            assert!(r.beats() < 40.0, "it starts recording");
        }
        r.period();
        r.time.command(Command::Stop);
        let line = r.until("take stopped", 2.0);
        assert_eq!(line, "Synth: take stopped, the song stopped");
        assert!(r.mix.mixer.frozen(StripId::Track(SYNTH)).is_none());
        assert!(r.mix.takes.view(StripId::Track(SYNTH)).is_none());
    }

    #[test]
    fn print_hands_over_the_loop_and_its_tail_to_save() {
        let mut r = Rig::new();
        r.launch(0, 0);
        r.time.command(Command::Play);
        r.period();
        let line = r.ask(MixAction::Print(StripId::Track(SYNTH)));
        assert!(line.contains("Synth: printing its 4-beat loop"), "{line}");
        // The loop's next start is bar 2; the synth stops after bar 2, so
        // the tail is its ring-out and 100 ms of quiet.
        let max = (6.0 * f64::from(RATE) / PERIOD as f64) as usize;
        for _ in 0..max {
            r.tone = r.beats() < 8.0;
            r.period();
            if !r.printed.is_empty() {
                break;
            }
        }
        let p = r.printed.pop().expect("printed");
        assert_eq!(p.loop_frames, 88_200, "one bar at 120");
        assert!(
            (4.0 - 1e-6..4.0 + 1e-6).contains(&p.start),
            "from bar 2: {}",
            p.start
        );
        assert!(
            p.tail_frames() < 10_000,
            "a short tail: {}",
            p.tail_frames()
        );
        assert!(p.peak() > 0.2);
        // The take is the synth from bar 2, frame for frame.
        for k in (0..p.loop_frames).step_by(97) {
            let want = sig(4.0 + k as f64 / 22_050.0);
            assert!(
                (p.left[k] - want).abs() < 1e-4,
                "{k}: {} vs {want}",
                p.left[k]
            );
        }
        assert!(
            r.mix.mixer.frozen(StripId::Track(SYNTH)).is_none(),
            "a print freezes nothing"
        );
    }

    #[test]
    fn a_freeze_says_why_it_cant() {
        let mut r = Rig::new();
        // The looper follows no clip.
        assert_eq!(
            r.ask(MixAction::Freeze(StripId::Track(LOOP))),
            "Loop can't be frozen"
        );
        // Nothing plays on the pads yet.
        let none = r.ask(MixAction::Freeze(StripId::Track(DRUMS)));
        assert_eq!(
            none,
            "Drums: nothing plays on the pads; launch a clip first"
        );
        // 3 bars and 1 bar fit; 3 and 4 don't.
        r.launch(0, 0);
        r.launch(1, 0);
        r.time.command(Command::Play);
        for _ in 0..4 {
            r.period();
        }
        r.launch(0, 1);
        let bar = (2.0 * f64::from(RATE) / PERIOD as f64) as usize + 10;
        for _ in 0..bar {
            r.period();
        }
        let uneven = r.ask(MixAction::Freeze(StripId::Track(SYNTH)));
        assert!(
            uneven.contains("don't loop together (12 and 16 beats)"),
            "{uneven}"
        );
        // A print needs no fit: it takes the longest, 4 bars.
        let print = r.ask(MixAction::Print(StripId::Master));
        assert!(
            print.contains("Master: printing its 16-beat loop"),
            "{print}"
        );
        assert!(r
            .mix
            .takes
            .cancel(StripId::Master, &mut r.mix.mixer, &mut Here::default())
            .is_some());
        // A track's own instrument: its own track's clip only.
        let song = Song {
            tracks: 2,
            own: 0b10,
            bpm: 120.0,
            now: &|t| r.time.now_playing(t),
            instrument: &|_| 1,
        };
        assert_eq!(
            song.loop_of(Feed::Track(1), Kind::Freeze, "Bass"),
            Ok((0.0, 12.0))
        );
        assert_eq!(
            song.loop_of(Feed::Pads, Kind::Freeze, "Drums").map(|l| l.1),
            Ok(16.0)
        );
    }

    #[test]
    fn feeds_and_what_freezes() {
        assert_eq!(feed(StripId::Track(TRACKS + 2)), Feed::Track(2));
        assert_eq!(feed(StripId::Track(DRUMS)), Feed::Pads);
        assert_eq!(feed(StripId::Pad(7)), Feed::Pads);
        assert_eq!(feed(StripId::Track(LOOP)), Feed::All);
        assert!(!freezable(StripId::Master));
        assert!(!freezable(StripId::Return(0)));
        assert!(freezable(StripId::Track(SYNTH)));
    }

    #[test]
    fn a_print_is_saved_in_samples_prints() {
        let dir = std::env::temp_dir().join(format!("mpfree-takes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = mpfree_library::Library::at(&dir).unwrap();
        let p = Printed {
            left: vec![0.25; 100],
            right: vec![-0.25; 100],
            rate: RATE,
            bpm: 120.0,
            start: 0.0,
            beats: 1.0,
            loop_frames: 90,
        };
        let a = save_print(&lib, "Synth 2026-11-14 06.12.wav", &p).unwrap();
        let b = save_print(&lib, "Synth 2026-11-14 06.12.wav", &p).unwrap();
        assert_eq!(a.display(), "Samples / Prints / Synth 2026-11-14 06.12.wav");
        assert_eq!(b.name(), "Synth 2026-11-14 06.12 2.wav");
        let bytes = std::fs::read(lib.file_path(&a).unwrap()).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 100 * 8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

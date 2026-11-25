//! The render foundation (the audio tracks rethink, phase 6.0;
//! docs/dev/design/renders.md): one description of every render, where
//! the audio it makes came from, and what kind of audio that is.
//!
//! - **One render description** ([`RenderDesc`]): what is rendered
//!   ([`Source`]), over what time ([`Range`]), where the sound is taken
//!   ([`Capture`]), how much rings on and plays before, at what quality
//!   and with what chance rolls ([`Quality`], `seed`), from what state
//!   ([`Read`]), for what ([`Purpose`]) and how it may run ([`Runs`]).
//!   Every tool of phase 6 says what it wants made in one, before anything
//!   is rendered. It isn't one way of making sound: the region player,
//!   the looper and Export's pass still do the work, and each answers to
//!   this. [`RenderDesc::check`] refuses a description that can't be made
//!   as it says (a worker render that needs the mixer, a capture its
//!   source has no such point for).
//! - **A fingerprint** ([`RenderDesc::fingerprint`]): 128 bits that are
//!   the same for the same description in any run on any machine, and
//!   differ when any part does, the versions of the algorithms among
//!   them. How it runs isn't in it: the same description made exclusive
//!   or in real time is the same audio.
//! - **Provenance** ([`Provenance`]): each recording a render makes keeps
//!   what it read, by id and revision, its description's fingerprint, its
//!   purpose and when it was made (Jay's rule: every audio representation
//!   can be traced to its source, its timing, its processing and the
//!   project state it came from). It is saved on the recording's take
//!   line ([`Provenance::words`], [`Provenance::from_words`]) and kept
//!   with the take's file name on the worker, so a save writes it again.
//! - **Three kinds of audio** ([`Kind`]): primary (takes, sounds brought
//!   in, committed renders), persistent derived (freezes) and disposable
//!   derived (tempo and reverse copies, renders under way). A take with no
//!   provenance is primary; phase 5's tempo copies are disposable and are
//!   never takes.
//!
//! - **Revisions and what changed** ([`changed`]): what a description
//!   read is still as it was read while each read's revision is the same
//!   now. The tracks give a track's ([`crate::AudioTracks::track_rev`]: a
//!   number for its regions and takes as they stand) and a recording's
//!   ([`crate::AudioTracks::rec_rev`]: its frames), the host the
//!   project's ([`crate::host::AudioHost::generation`]); the app gives a
//!   clip's, a loop's and the mixer's routes'. One check answers whether a
//!   copy, a freeze or a render under way is still right.
//!
//! Phase 5's copies keep their own key ([`crate::copies::CopyKey`]): it
//! is already a whole description of a disposable copy (the recording's
//! frames, the range in ticks with the tempo map's shape over it, the
//! stretch's settings and version), and its hash is their scheduler key.
//!
//! Nothing here runs on the audio thread: descriptions and provenance
//! allocate, and are made by the host, the worker or the screen.

use crate::RecordingId;

/// The render description's format, the first thing every fingerprint
/// holds: raised when what a description says, or how it is fingerprinted,
/// changes, so an older fingerprint never matches a newer description.
pub const FORMAT: u32 = 1;

/// What a render takes its sound from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A recording's own frames: its lasting id, and its length (a
    /// finished recording never changes, so the two name its content).
    Recording { rec: RecordingId, frames: u64 },
    /// An audio clip, by its lasting number in the project.
    Clip(u32),
    /// A loop's layers, by the loop's lasting id.
    Loop(u128),
    /// One track's sound, by its lasting number.
    Track(u32),
    /// Several tracks into one channel, in the order given.
    Tracks(Vec<u32>),
    /// A submix (a bus), by its number.
    Bus(u16),
    /// The whole mix.
    Mix,
}

impl Source {
    /// Whether rendering it needs no instrument, effect or mixer: a
    /// recording, a clip's audio or a loop's layers, read as they are.
    pub fn is_audio(&self) -> bool {
        matches!(
            self,
            Source::Recording { .. } | Source::Clip(_) | Source::Loop(_)
        )
    }
}

/// Over what time a render runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Range {
    /// Frames of the source itself, from `from`, `len` of them (a
    /// recording's frames: Consolidate, Reverse, a Print of a loop).
    Frames { from: i64, len: u64 },
    /// A clip's pass, or the common cycle of several clips, `ticks` long
    /// (a clips Bounce).
    Cycle { ticks: u64 },
    /// Song ticks `from` to `to` (a section, or the arrangement), with
    /// the tempo map over them: its tempo at `from` and its shape from
    /// there ([`crate::copies::shape_of`], 0 for a steady tempo).
    Song {
        from: f64,
        to: f64,
        tempo: f64,
        shape: u64,
    },
}

/// Where a render takes the sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capture {
    /// The source alone, with no mixer: a recording, a clip, a loop.
    Alone,
    /// A strip after its effects, before its fader and mute (where the
    /// sidechains are fed from, and Freeze takes its sound).
    BeforeFader,
    /// A strip after its fader, mute and pan.
    AfterFader,
    /// A bus's output.
    BusOut,
    /// The master, as heard.
    Master,
}

/// The quality a render is made at: what its algorithms are, and which
/// versions of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Quality {
    /// The pitched reads' interpolation version (0: none, read as is).
    pub interp: u32,
    /// The stretch's settings ([`crate::copies::settings_id`]) and its
    /// version (`mpfree_stretch::VERSION`); 0 and 0 for none.
    pub stretch: u64,
    pub stretch_version: u32,
}

/// Something a render reads, whose revision says whether it is still as
/// it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dep {
    /// A recording: its revision is its frames (a finished recording never
    /// changes).
    Recording(RecordingId),
    /// An audio track, by its lasting number: its regions, clips and takes.
    Track(u32),
    /// An audio clip, by its lasting number.
    Clip(u32),
    /// A loop, by its lasting id: its layers.
    Loop(u128),
    /// The mixer's routes: faders, sends, outputs, sidechains.
    Routes,
    /// The project: which one is open (its generation).
    Project,
}

/// A thing read, and its revision when it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Read {
    pub dep: Dep,
    pub rev: u64,
}

/// What a render is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// A disposable copy: made again whenever it is wanted (a tempo copy,
    /// a reverse copy).
    Copy,
    /// A lasting recording in the project: a bounce, a Commit stretch, a
    /// Consolidate, a Print.
    Recording,
    /// A freeze: kept while its track is frozen, made again from its
    /// sources while they are here.
    Freeze,
    /// An export, written outside the project.
    Export,
}

impl Purpose {
    /// The kind of audio it makes in the project, or None for an export
    /// (outside it).
    pub fn kind(self) -> Option<Kind> {
        match self {
            Purpose::Copy => Some(Kind::Disposable),
            Purpose::Recording => Some(Kind::Primary),
            Purpose::Freeze => Some(Kind::Persistent),
            Purpose::Export => None,
        }
    }

    /// Its word on a take line, for the purposes a take can have.
    fn word(self) -> &'static str {
        match self {
            Purpose::Copy => "copy",
            Purpose::Recording => "recording",
            Purpose::Freeze => "freeze",
            Purpose::Export => "export",
        }
    }
}

/// The three kinds of audio, each with its own rules for keeping and
/// cleaning up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Takes, sounds brought in, committed renders: kept while anything
    /// names it, not necessarily made again.
    Primary,
    /// Freezes: kept while in use, made again while their sources and
    /// engines are here.
    Persistent,
    /// Tempo and reverse copies, renders under way: never saved, made
    /// again when wanted.
    Disposable,
}

/// How a render may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Runs {
    /// On the scheduler's worker beside playback: it needs no instrument,
    /// effect or mixer (Consolidate, Reverse, Commit stretch, copies).
    Worker,
    /// Through the live engine, which takes the transport (Export's pass).
    Exclusive,
    /// Through the live engine in real time (outside gear, Listen).
    RealTime,
}

/// One render, described before anything is rendered.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderDesc {
    pub source: Source,
    pub range: Range,
    pub capture: Capture,
    /// Frames that ring on after the range, and frames played before it
    /// to bring delays, reverbs and envelopes to their running state.
    pub tail: u64,
    pub preroll: u64,
    pub quality: Quality,
    /// The chance rolls' seed, so a render can be made again the same.
    pub seed: u64,
    /// The revisions of everything it reads.
    pub reads: Vec<Read>,
    pub purpose: Purpose,
    pub runs: Runs,
}

impl RenderDesc {
    /// A recording's frames `from..from + len` as they are, on the worker:
    /// the render 6.0 makes, which Consolidate, Reverse and Commit stretch
    /// build on. It reads the recording at its length.
    pub fn frames(rec: RecordingId, frames: u64, from: i64, len: u64, purpose: Purpose) -> Self {
        RenderDesc {
            source: Source::Recording { rec, frames },
            range: Range::Frames { from, len },
            capture: Capture::Alone,
            tail: 0,
            preroll: 0,
            quality: Quality::default(),
            seed: 0,
            reads: vec![Read {
                dep: Dep::Recording(rec),
                rev: frames,
            }],
            purpose,
            runs: Runs::Worker,
        }
    }

    /// Whether it can be made as it says, or why not (for the log and
    /// the tests: a tool never offers one that fails this).
    pub fn check(&self) -> Result<(), &'static str> {
        use Capture as C;
        use Source as S;
        let capture_fits = match (&self.source, self.capture) {
            (s, C::Alone) => s.is_audio(),
            (S::Track(_) | S::Tracks(_) | S::Bus(_), C::BeforeFader | C::AfterFader) => true,
            (S::Bus(_), C::BusOut) | (S::Mix, C::Master) => true,
            _ => false,
        };
        if !capture_fits {
            return Err("its source has no such capture point");
        }
        if let S::Tracks(t) = &self.source {
            let mut sorted = t.clone();
            sorted.sort_unstable();
            sorted.dedup();
            if t.len() < 2 || sorted.len() != t.len() {
                return Err("several tracks means two or more, each once");
            }
        }
        if let S::Recording { rec, frames } = self.source {
            if !rec.is_given() || frames == 0 {
                return Err("a recording with no id or no frames");
            }
        }
        match self.range {
            Range::Frames { from, len } => {
                let S::Recording { frames, .. } = self.source else {
                    return Err("a range of frames is a recording's");
                };
                let end = from.checked_add(len as i64).filter(|_| len > 0);
                if from < 0 || end.is_none_or(|e| e as u64 > frames) {
                    return Err("the frames are not all in the recording");
                }
            }
            Range::Cycle { ticks: 0 } => return Err("an empty cycle"),
            Range::Cycle { .. } => {}
            Range::Song {
                from, to, tempo, ..
            } => {
                if !(from.is_finite() && to.is_finite() && from < to)
                    || !(tempo.is_finite() && tempo > 0.0)
                {
                    return Err("a song range must run forward at a tempo");
                }
            }
        }
        if self.runs == Runs::Worker && self.capture != C::Alone {
            return Err("only audio as it is renders on the worker");
        }
        if self.capture != C::Alone && self.purpose == Purpose::Copy {
            return Err("a copy is made from audio alone");
        }
        Ok(())
    }

    /// 128 bits that name this description: the same in any run on any
    /// machine, different when any part is (the reads in any order), how
    /// it runs aside. FNV-1a over a fixed little-endian encoding, the
    /// format's version first.
    pub fn fingerprint(&self) -> u128 {
        let mut h = Fnv::new();
        h.u32(FORMAT);
        match &self.source {
            Source::Recording { rec, frames } => {
                h.u8(1);
                h.u128(rec.0);
                h.u64(*frames);
            }
            Source::Clip(c) => {
                h.u8(2);
                h.u32(*c);
            }
            Source::Loop(l) => {
                h.u8(3);
                h.u128(*l);
            }
            Source::Track(t) => {
                h.u8(4);
                h.u32(*t);
            }
            Source::Tracks(ts) => {
                h.u8(5);
                h.u64(ts.len() as u64);
                for t in ts {
                    h.u32(*t);
                }
            }
            Source::Bus(b) => {
                h.u8(6);
                h.u32(u32::from(*b));
            }
            Source::Mix => h.u8(7),
        }
        match self.range {
            Range::Frames { from, len } => {
                h.u8(1);
                h.u64(from as u64);
                h.u64(len);
            }
            Range::Cycle { ticks } => {
                h.u8(2);
                h.u64(ticks);
            }
            Range::Song {
                from,
                to,
                tempo,
                shape,
            } => {
                h.u8(3);
                h.u64(from.to_bits());
                h.u64(to.to_bits());
                h.u64(tempo.to_bits());
                h.u64(shape);
            }
        }
        h.u8(match self.capture {
            Capture::Alone => 1,
            Capture::BeforeFader => 2,
            Capture::AfterFader => 3,
            Capture::BusOut => 4,
            Capture::Master => 5,
        });
        h.u64(self.tail);
        h.u64(self.preroll);
        h.u32(self.quality.interp);
        h.u64(self.quality.stretch);
        h.u32(self.quality.stretch_version);
        h.u64(self.seed);
        let mut reads = self.reads.clone();
        reads.sort_unstable();
        h.u64(reads.len() as u64);
        for r in &reads {
            match r.dep {
                Dep::Recording(rec) => {
                    h.u8(1);
                    h.u128(rec.0);
                }
                Dep::Track(t) => {
                    h.u8(2);
                    h.u32(t);
                }
                Dep::Clip(c) => {
                    h.u8(3);
                    h.u32(c);
                }
                Dep::Loop(l) => {
                    h.u8(4);
                    h.u128(l);
                }
                Dep::Routes => h.u8(5),
                Dep::Project => h.u8(6),
            }
            h.u64(r.rev);
        }
        h.u8(match self.purpose {
            Purpose::Copy => 1,
            Purpose::Recording => 2,
            Purpose::Freeze => 3,
            Purpose::Export => 4,
        });
        h.0
    }
}

/// FNV-1a, 128 bits.
struct Fnv(u128);

impl Fnv {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    fn new() -> Fnv {
        Fnv(Self::OFFSET)
    }

    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ u128::from(x)).wrapping_mul(Self::PRIME);
        }
    }

    fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    fn u128(&mut self, v: u128) {
        self.bytes(&v.to_le_bytes());
    }
}

/// FNV-1a, 64 bits, over little-endian words: the tracks' revisions
/// ([`crate::AudioTracks::track_rev`]). Allocates nothing.
pub(crate) struct Fnv64(u64);

impl Fnv64 {
    pub(crate) fn new() -> Fnv64 {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn u64(&mut self, v: u64) {
        for x in v.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub(crate) fn u128(&mut self, v: u128) {
        self.u64(v as u64);
        self.u64((v >> 64) as u64);
    }

    pub(crate) fn finish(&self) -> u64 {
        self.0
    }
}

/// The first of `reads` that no longer holds, as `now` says each is now
/// (its revision, or None when it is gone), or None when every one holds
/// (the audio tracks rethink, phase 6.0.2). A render, a freeze or a copy
/// made from `reads` is still right while this is None; otherwise a
/// render under way commits nothing over the newer state, and
/// [`Dep::changed_words`] says why. Allocates nothing.
pub fn changed(reads: &[Read], now: impl Fn(Dep) -> Option<u64>) -> Option<Dep> {
    reads
        .iter()
        .find(|r| now(r.dep) != Some(r.rev))
        .map(|r| r.dep)
}

impl Dep {
    /// What changed, in a musician's words, `name` being what the
    /// musician calls it (a track's or a clip's name, a loop's): for the
    /// log and, from 6.1 on, the line a tool shows when it commits
    /// nothing.
    pub fn changed_words(self, name: &str) -> String {
        match self {
            Dep::Recording(_) => format!("the recording {name} is gone"),
            Dep::Track(_) => format!("{name} was edited while it rendered"),
            Dep::Clip(_) => format!("the clip {name} was edited while it rendered"),
            Dep::Loop(_) => format!("{name} changed while it rendered"),
            Dep::Routes => "the mixer's routing changed while it rendered".into(),
            Dep::Project => "another project opened while it rendered".into(),
        }
    }
}

/// Where a recording a render made came from: what it read, by id and
/// revision, its description's fingerprint, what it is for and when it
/// was made. Kept on its take line; a take with none is primary audio
/// recorded or brought in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// Everything it read, each with its revision then (a recording with
    /// its frames): the recordings it was made from are among them
    /// ([`Provenance::sources`]).
    pub reads: Vec<Read>,
    /// Its description's fingerprint ([`RenderDesc::fingerprint`]).
    pub made: u128,
    /// The render format ([`FORMAT`]) it was described in: a fingerprint
    /// of another format can't be compared with this build's.
    pub format: u32,
    pub purpose: Purpose,
    /// When it was made, in seconds since 1970 (UTC); 0 if not known.
    pub at: u64,
}

impl Provenance {
    /// The provenance of what `desc` makes, made at `at`.
    pub fn of(desc: &RenderDesc, at: u64) -> Provenance {
        let mut reads = desc.reads.clone();
        reads.sort_unstable();
        Provenance {
            reads,
            made: desc.fingerprint(),
            format: FORMAT,
            purpose: desc.purpose,
            at,
        }
    }

    /// The kind of audio it is.
    pub fn kind(&self) -> Kind {
        self.purpose.kind().unwrap_or(Kind::Primary)
    }

    /// The recordings it was made from, each with its frames then.
    pub fn sources(&self) -> impl Iterator<Item = (RecordingId, u64)> + '_ {
        self.reads.iter().filter_map(|r| match r.dep {
            Dep::Recording(rec) => Some((rec, r.rev)),
            _ => None,
        })
    }

    /// Whether `desc` would make the same audio: its fingerprint is this
    /// one's, in this build's format.
    pub fn matches(&self, desc: &RenderDesc) -> bool {
        self.format == FORMAT && self.made == desc.fingerprint()
    }

    /// Its words on a take line, each after a space, before `file=`:
    /// `made=` (32 hex digits) `for=` `ver=` `at=` and, when it read
    /// anything, `reads=` (each a letter and its id, a dot and its
    /// revision, by commas: `r` a recording, `t` a track, `c` a clip,
    /// `l` a loop, `routes`, `project`).
    pub fn words(&self) -> String {
        let mut s = format!(
            " made={:032x} for={} ver={} at={}",
            self.made,
            self.purpose.word(),
            self.format,
            self.at
        );
        if !self.reads.is_empty() {
            s.push_str(" reads=");
            for (i, r) in self.reads.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                match r.dep {
                    Dep::Recording(rec) => s.push_str(&format!("r{rec}")),
                    Dep::Track(t) => s.push_str(&format!("t{t}")),
                    Dep::Clip(c) => s.push_str(&format!("c{c}")),
                    Dep::Loop(l) => s.push_str(&format!("l{l:032x}")),
                    Dep::Routes => s.push_str("routes"),
                    Dep::Project => s.push_str("project"),
                }
                s.push_str(&format!(".{}", r.rev));
            }
        }
        s
    }

    /// From a take line's words (`get` finds a word's value): None when
    /// it has no `made=` (a take recorded or brought in), or why its words
    /// are wrong. A take is a recording or a freeze, never a copy or an
    /// export.
    pub fn from_words<'a>(
        get: impl Fn(&str) -> Option<&'a str>,
    ) -> Result<Option<Provenance>, String> {
        let Some(made) = get("made") else {
            return match ["for", "ver", "at", "reads"]
                .iter()
                .find(|k| get(k).is_some())
            {
                Some(k) => Err(format!("{k}= with no made=")),
                None => Ok(None),
            };
        };
        let made = (made.len() == 32)
            .then(|| u128::from_str_radix(made, 16).ok())
            .flatten()
            .ok_or_else(|| format!("made={made} is not 32 hex digits"))?;
        let purpose = match get("for") {
            Some("recording") => Purpose::Recording,
            Some("freeze") => Purpose::Freeze,
            Some(w) => return Err(format!("for={w} is not a recording or a freeze")),
            None => return Err("made= with no for=".into()),
        };
        let format: u32 = get("ver")
            .and_then(|v| v.parse().ok())
            .filter(|&v| v > 0)
            .ok_or("made= with no ver=")?;
        let at: u64 = match get("at") {
            Some(v) => v.parse().map_err(|_| format!("at={v} is not a time"))?,
            None => 0,
        };
        let mut reads = Vec::new();
        for item in get("reads")
            .unwrap_or("")
            .split(',')
            .filter(|i| !i.is_empty())
        {
            let bad = || format!("reads: {item} is not a read");
            let (what, rev) = item.rsplit_once('.').ok_or_else(bad)?;
            let rev: u64 = rev.parse().map_err(|_| bad())?;
            let dep = match what {
                "routes" => Dep::Routes,
                "project" => Dep::Project,
                _ if what.len() > 1 => {
                    let (k, id) = what.split_at(1);
                    match k {
                        "r" => Dep::Recording(RecordingId::parse(id).ok_or_else(bad)?),
                        "t" => Dep::Track(id.parse().map_err(|_| bad())?),
                        "c" => Dep::Clip(id.parse().map_err(|_| bad())?),
                        "l" if id.len() == 32 => {
                            Dep::Loop(u128::from_str_radix(id, 16).map_err(|_| bad())?)
                        }
                        _ => return Err(bad()),
                    }
                }
                _ => return Err(bad()),
            };
            reads.push(Read { dep, rev });
        }
        reads.sort_unstable();
        Ok(Some(Provenance {
            reads,
            made,
            format,
            purpose,
            at,
        }))
    }
}

/// Seconds since 1970 now (UTC), for [`Provenance::at`]; 0 if the clock
/// is before it.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

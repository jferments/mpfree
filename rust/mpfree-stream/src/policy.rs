//! What a sound is, apart from what it is doing, and the rules that follow
//! from it (disk streaming's P3, step 2): memory or disk, how much of its
//! start stays in memory, and what admission says when a workload isn't
//! covered.
//!
//! What a sound is doing now (playing, cued, the next song, background)
//! stays with the coordinator's hold and job tiers, so a stem never
//! changes class when it is cued. These are pure functions: the
//! coordinator, the sampler and the launcher call them with what they know.

use crate::cursor::MAX_SPEED;

/// The kind of sound. It sets the default memory rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    /// A pad's sound in a drum kit.
    KitSound,
    /// One sample of a keygroup instrument.
    KeygroupSample,
    /// A long sound a pad or note starts and lets play out.
    OneShot,
    /// A backing stem: long, started at a known time.
    Stem,
    /// A loop, played round and round.
    Loop,
    /// A take recorded in MPFree.
    Recording,
}

impl Class {
    /// Whether its starts come without warning (a pad, a MIDI note, a live
    /// keygroup), so its start must already be in memory. The rest start at
    /// a known time (the arranger, a scheduled clip, a transport move) and
    /// are fetched before they start.
    pub fn starts_unannounced(self) -> bool {
        matches!(
            self,
            Class::KitSound | Class::KeygroupSample | Class::OneShot
        )
    }
}

/// How it is read. It sets the read-ahead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    /// Straight through.
    Straight,
    /// Round a loop.
    Looping,
    /// Jumping about (slices, a scrubbed clip).
    Jumping,
    /// At a speed that changes (a pitched keygroup, varispeed).
    VaryingSpeed,
}

/// The musician's rule for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rule {
    /// Memory when it fits, else disk.
    #[default]
    Automatic,
    /// Always in memory ("Keep in memory").
    KeepInMemory,
    /// From disk unless it is already in memory.
    PreferStreaming,
}

/// Where it is stored. It sets its speed and how it fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Device {
    /// MPFree's USB stick.
    Stick,
    /// The SD card, read-only, once the hardware project switches it on.
    Card,
    /// Already in memory (a take just recorded, a sound another engine
    /// holds).
    Memory,
}

/// What a sound is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Kind {
    pub class: Class,
    pub access: Access,
    pub rule: Rule,
    pub device: Device,
}

/// Where a sound plays from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Held whole in memory.
    Whole,
    /// Streamed from its device, its start kept in memory if it needs one
    /// ([`start_frames`]).
    Stream,
}

/// Memory or disk, from what the sound is and whether it fits whole beside
/// what is playing (`fits_whole`, from the shared memory check). A sound
/// already in memory stays there. Keep in memory always asks for memory:
/// when it doesn't fit, admission says so ([`answer`]); it never turns
/// into memory that isn't there. Under Automatic, kit sounds, keygroup
/// samples, one-shots and loops load whole when they fit; stems and
/// recordings, long by nature, stream.
pub fn place(kind: &Kind, fits_whole: bool) -> Place {
    if kind.device == Device::Memory {
        return Place::Whole;
    }
    match kind.rule {
        Rule::KeepInMemory => Place::Whole,
        Rule::PreferStreaming => Place::Stream,
        Rule::Automatic => match kind.class {
            Class::Stem | Class::Recording => Place::Stream,
            _ if fits_whole => Place::Whole,
            _ => Place::Stream,
        },
    }
}

/// How long the stick can take to bring a sound's next audio to the sound
/// thread, in seconds: its worst stall, plus decoding, plus handing it over.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Refill {
    /// The stick's worst stall (measured by the stick test, D1).
    pub stall_s: f64,
    /// Decoding what was read (none for WAV; FLAC and rate conversion
    /// later).
    pub decode_s: f64,
    /// Getting it into the sound thread's cache.
    pub deliver_s: f64,
}

impl Refill {
    /// The whole refill time.
    pub fn total_s(&self) -> f64 {
        let t = self.stall_s + self.decode_s + self.deliver_s;
        if t.is_finite() {
            t.max(0.0)
        } else {
            f64::INFINITY
        }
    }
}

/// The frames of a streamed sound's start to keep in memory, for a file at
/// `rate` frames a second. A sound that starts without warning keeps enough
/// to play through the worst refill: a 100 ms start doesn't survive an
/// 800 ms stall. One read at a changing speed keeps that much at the
/// fastest a cursor reads ([`MAX_SPEED`]). A sound that starts at a known
/// time keeps none: it is fetched before it starts, by its deadline.
pub fn start_frames(kind: &Kind, refill: &Refill, rate: u32) -> u64 {
    if !kind.class.starts_unannounced() {
        return 0;
    }
    let speed = if kind.access == Access::VaryingSpeed {
        MAX_SPEED
    } else {
        1.0
    };
    let frames = refill.total_s() * f64::from(rate) * speed;
    if frames.is_finite() {
        frames.ceil() as u64
    } else {
        u64::MAX
    }
}

/// What admission knows about a workload, to say why it isn't covered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Need {
    /// Bytes more than the shared memory check can give, for the workload
    /// as asked (0 when it fits).
    pub memory_short: u64,
    /// The stick has been measured (the stick test, D1).
    pub measured: bool,
    /// The measured stick reads the workload's streams fast enough.
    pub keeps_up: bool,
    /// The first sound whose start isn't covered, if any.
    pub uncovered: Option<u16>,
    /// Bytes more the uncovered sound would need to be held whole (0 when
    /// it fits whole).
    pub whole_short: u64,
}

/// What admission says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    /// Everything is covered.
    Covered,
    /// Memory is short by this many bytes: unload something, prepare less
    /// or change the cache size.
    MemoryShort { by: u64 },
    /// Nothing measured yet: it plays normally, shown as untested, with no
    /// promise of no dropouts.
    Untested,
    /// The measured stick is too slow for the workload: it can't be
    /// qualified on this stick.
    TooSlow,
    /// Memory is there but the stick can't refill this sound's start in
    /// time: offer Keep in memory for it.
    KeepInMemory { source: u16 },
}

/// Why a workload isn't covered, most binding first. Memory comes before
/// anything else, since it is a hard limit whether or not the stick was
/// measured. Keep in memory is offered only when the whole sound fits:
/// otherwise the answer is the memory it lacks.
pub fn answer(n: &Need) -> Cover {
    if n.memory_short > 0 {
        return Cover::MemoryShort { by: n.memory_short };
    }
    if !n.measured {
        return Cover::Untested;
    }
    if !n.keeps_up {
        return Cover::TooSlow;
    }
    match n.uncovered {
        None => Cover::Covered,
        Some(_) if n.whole_short > 0 => Cover::MemoryShort { by: n.whole_short },
        Some(source) => Cover::KeepInMemory { source },
    }
}

/// Registered sounds (their details only: where, format, length) before a
/// project is told it has very many. From the sampler's stress cases: the
/// biggest real kit, an auto-sampled keygroup of 61 notes with up to 4
/// velocity layers, registers about 250. Numbers from the workstation,
/// until the device round.
pub const REGISTERED_WARN: usize = 256;
/// Registered sounds at most. They cost only their details, so thousands
/// fit in well under a megabyte; what stays tightly bounded is the sources
/// open at once ([`crate::msg::MAX_SOURCES`]) and the voices reading them
/// ([`crate::msg::MAX_CURSORS`]).
pub const REGISTERED_MOST: usize = 4096;

/// What the registry's size says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registry {
    /// Within what the stress cases need.
    Fine,
    /// More than the biggest stress case: allowed, and said (Dev › Debug
    /// until the status monitor shows it).
    Many,
    /// One more would pass [`REGISTERED_MOST`]: the registration is
    /// refused, saying why.
    Full,
}

/// Whether one more sound can be registered beside `count` already.
pub fn registry(count: usize) -> Registry {
    if count >= REGISTERED_MOST {
        Registry::Full
    } else if count >= REGISTERED_WARN {
        Registry::Many
    } else {
        Registry::Fine
    }
}

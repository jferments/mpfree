//! What goes between the sound thread's [`crate::player::Player`] and the
//! coordinator, through two bounded channels made at start.
//!
//! **Neither side ever blocks on them.** The sound thread can't, and the
//! coordinator only polls (`try_recv`, `try_send`), so a send never has to
//! wake a waiting thread, and std's bounded channel then takes no lock and
//! allocates nothing: its buffer is made with the channel.
//!
//! **Who owns a chunk.** The sound thread owns its cache outright; the
//! coordinator owns only the buffers in flight, passed by message, and
//! every buffer comes back ([`FromPlayer::Back`]), so neither side frees
//! one the other made. The coordinator chooses which cache slot a unit
//! goes in and keeps a mirror of the cache: only its own messages change
//! the cache, and they are applied in order, so the mirror is exact.

/// Sources the layer knows at once, at most.
pub const MAX_SOURCES: usize = 256;

/// Cursors (voices reading a source) at once, at most; a cursor's id is
/// below this.
pub const MAX_CURSORS: usize = 512;

use std::sync::Arc;

/// Who asked: the project's generation, the source, and the source's
/// generation. A message for an older generation of either is dropped
/// (counted), never applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub project: u32,
    pub source: u16,
    pub gen: u32,
}

/// Coordinator to sound thread.
#[derive(Debug)]
pub enum ToPlayer {
    /// A new project: every source is forgotten and the cache emptied.
    Project(u32),
    /// Source `tag.source` is now this sound (`tag.gen` newer than any
    /// before it): its old cached audio, if any, is gone.
    Open {
        tag: Tag,
        channels: u16,
        frames: u64,
    },
    /// Source `tag.source` is now this sound, held whole in memory (the
    /// generation rule as for `Open`). Reads are served straight from
    /// `data` (interleaved, `channels` channels). The sound thread never
    /// drops it: it goes back ([`FromPlayer::Resident`]) when the source is
    /// closed or replaced, or if this message is refused.
    Resident {
        tag: Tag,
        channels: u16,
        data: Arc<[f32]>,
    },
    /// The source is unloaded: its cache slots go free, and reads of it
    /// give silence (not counted as missed).
    Close { tag: Tag },
    /// Unit `unit` of the source, decoded to float, to copy into cache slot
    /// `slot` (whatever was there goes). `buf[..len]` holds it; `len` must
    /// be the unit's whole length, or the fill is refused as short.
    Fill {
        tag: Tag,
        unit: u32,
        slot: u32,
        /// The request's number, for the coordinator's books.
        request: u32,
        buf: Vec<f32>,
        len: usize,
    },
    /// Slot `slot` goes free (the coordinator wants its room).
    Free { slot: u32 },
}

/// Where a cursor plays and how it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// The frame it plays now.
    pub frame: u64,
    /// File frames moved per output frame.
    pub speed: f64,
    /// A loop it goes round: frames `start..end`.
    pub looping: Option<(u64, u64)>,
    /// The frame it stops at (a region's end), if before the sound's end.
    pub end: Option<u64>,
}

/// Where one cursor plays a source, posted once a planning round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Where {
    pub tag: Tag,
    pub cursor: u16,
    /// The sound thread's planning round: a cursor not heard of for two
    /// rounds has stopped, and its window is no longer wanted.
    pub round: u32,
    pub place: Place,
}

/// Sound thread to coordinator.
#[derive(Debug)]
pub enum FromPlayer {
    Where(Where),
    /// A buffer back, done with.
    Back(Vec<f32>),
    /// A memory source's frames back: closed, replaced or refused.
    Resident(Arc<[f32]>),
    /// A fill was not applied: `stale` (an older generation) or short.
    Refused {
        tag: Tag,
        unit: u32,
        slot: u32,
        short: bool,
    },
}

//! Sound a node holds, copied out for the host a piece at a time
//! ([`crate::Node::copy_held`]): the granular effect's memory put on a pad
//! (the granular roadmap's step C3).
//!
//! The host makes a [`Held`] off the audio thread, big enough for what it
//! wants, and lends it to the audio thread, which calls
//! [`crate::Node::copy_held`] once a period until the node says it is
//! [`HeldStep::Done`]. Each call copies at most a few thousand frames, so a
//! long memory never costs one period much. Then the host takes the node's
//! settings into it ([`Held::keep_values`]) and sends it back. Nothing on
//! the audio thread allocates: the buffer and the room for the values were
//! made with it.

use crate::Node;

/// Most parameters [`Held::keep_values`] keeps.
pub const HELD_VALUES: usize = 64;

/// A buffer of interleaved frames a node fills with the sound it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    data: Vec<f32>,
    /// Samples it has room for, and the most channels it takes.
    samples: usize,
    most: usize,
    /// Frames it has room for (in the channels of the copy).
    room: usize,
    channels: usize,
    rate: u32,
    /// The span being copied in the node's own frame numbers, and the
    /// node's mark for it (anything that tells it the memory it copies
    /// from is still the same), once the copy has begun.
    span: Option<(u64, u64)>,
    mark: u64,
    values: Vec<(u16, f32)>,
}

/// What a call of [`crate::Node::copy_held`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldStep {
    /// The node holds nothing it can copy (not this kind of node, or its
    /// memory is empty).
    Nothing,
    /// Copied a piece; call again next period.
    More,
    /// The whole span is in.
    Done,
    /// What it was copying changed under it (cleared, or overwritten):
    /// start again with a fresh [`Held`].
    Lost,
}

impl Held {
    /// Room for `frames` frames of up to `channels` (1 or 2) channels.
    /// Allocates (call it off the audio thread).
    pub fn new(channels: usize, frames: usize) -> Held {
        let channels = channels.clamp(1, 2);
        Held {
            data: Vec::with_capacity(frames * channels),
            samples: frames * channels,
            most: channels,
            room: frames,
            channels,
            rate: 0,
            span: None,
            mark: 0,
            values: Vec::with_capacity(HELD_VALUES),
        }
    }

    /// Frames it has room for.
    pub fn room(&self) -> usize {
        self.room
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    /// The node's rate (0 until a copy begins).
    pub fn rate(&self) -> u32 {
        self.rate
    }
    /// The span being copied, in the node's frame numbers.
    pub fn span(&self) -> Option<(u64, u64)> {
        self.span
    }
    /// The node's mark for the span ([`Held::begin`]).
    pub fn mark(&self) -> u64 {
        self.mark
    }
    /// Frames copied so far.
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels
    }
    /// Frames still to copy.
    pub fn left(&self) -> usize {
        self.span
            .map_or(0, |(a, b)| (b - a) as usize - self.frames())
    }
    /// The frames copied, interleaved.
    pub fn data(&self) -> &[f32] {
        &self.data
    }
    /// The frames copied, interleaved, as a vector.
    pub fn into_data(self) -> Vec<f32> {
        self.data
    }
    /// The node's settings when the copy ended (id, value).
    pub fn values(&self) -> &[(u16, f32)] {
        &self.values
    }

    /// For the node: starts copying its frames `from..to` (`channels`, 1
    /// or 2, at `rate`), with a mark of its own for them. Keeps the newest
    /// frames if they don't all fit. False (nothing begun) if the span is
    /// empty or the channels are more than it was made for.
    pub fn begin(&mut self, from: u64, to: u64, channels: usize, rate: u32, mark: u64) -> bool {
        if to <= from || !(1..=self.most).contains(&channels) || self.samples < channels {
            return false;
        }
        // Room in frames of the node's channels.
        let room = (self.samples / channels) as u64;
        self.channels = channels;
        self.room = room as usize;
        self.rate = rate;
        self.span = Some((from.max(to - room.min(to)), to));
        self.mark = mark;
        self.data.clear();
        true
    }

    /// Empties it for another copy, keeping its memory (so the audio
    /// thread can start again without allocating).
    pub fn clear(&mut self) {
        self.data.clear();
        self.values.clear();
        self.span = None;
        (self.channels, self.room) = (self.most, self.samples / self.most);
    }

    /// For the node: the next frame to copy (in its frame numbers) and how
    /// many are left.
    pub fn next(&self) -> Option<(u64, usize)> {
        let (a, _) = self.span?;
        Some((a + self.frames() as u64, self.left()))
    }

    /// For the node: appends interleaved frames, never past the span (the
    /// rest is dropped), so it never allocates.
    pub fn push(&mut self, interleaved: &[f32]) {
        let n = (self.left() * self.channels).min(interleaved.len());
        self.data.extend_from_slice(&interleaved[..n]);
    }

    /// For the host, once the copy is done: keeps `node`'s parameter values
    /// (up to [`HELD_VALUES`], meters left out). Never allocates.
    pub fn keep_values(&mut self, node: &dyn Node) {
        self.values.clear();
        for p in node.params().iter().filter(|p| !p.readonly) {
            if self.values.len() == self.values.capacity() {
                break;
            }
            self.values.push((p.id, node.param(p.id)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_fills_its_span_and_no_more() {
        let mut h = Held::new(2, 10);
        assert_eq!(h.next(), None);
        assert!(h.begin(100, 104, 2, 44_100, 7));
        assert_eq!(h.next(), Some((100, 4)));
        h.push(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(h.next(), Some((102, 2)));
        h.push(&[5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        assert_eq!(h.left(), 0);
        assert_eq!(h.data(), &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        assert_eq!((h.frames(), h.mark(), h.rate()), (4, 7, 44_100));
    }

    #[test]
    fn a_long_span_keeps_the_newest_frames() {
        let mut h = Held::new(2, 10);
        assert!(h.begin(0, 50, 2, 48_000, 0));
        assert_eq!(h.span(), Some((40, 50)));
        // A mono copy has room for twice the frames.
        let mut m = Held::new(2, 10);
        assert!(m.begin(0, 50, 1, 48_000, 0));
        assert_eq!((m.span(), m.channels()), (Some((30, 50)), 1));
    }

    #[test]
    fn nothing_begins_on_an_empty_span_or_too_many_channels() {
        let mut h = Held::new(1, 10);
        assert!(!h.begin(5, 5, 1, 44_100, 0));
        assert!(!h.begin(0, 5, 2, 44_100, 0));
        assert_eq!(h.span(), None);
    }

    #[test]
    fn a_cleared_buffer_takes_another_copy() {
        let mut h = Held::new(2, 4);
        assert!(h.begin(0, 8, 1, 44_100, 0));
        h.push(&[1.0; 8]);
        h.clear();
        assert_eq!(
            (h.span(), h.frames(), h.channels(), h.room()),
            (None, 0, 2, 4)
        );
        assert!(h.begin(0, 2, 2, 44_100, 1));
        assert_eq!(h.next(), Some((0, 2)));
    }

    #[test]
    fn pushes_never_grow_the_buffer() {
        let mut h = Held::new(2, 3);
        let cap = h.data.capacity();
        assert!(h.begin(0, 3, 2, 44_100, 0));
        h.push(&[0.5; 100]);
        assert_eq!((h.frames(), h.data.capacity()), (3, cap));
    }
}

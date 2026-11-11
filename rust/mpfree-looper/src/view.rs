//! A layer's playback as pieces of its audio. The loop edits (half,
//! double, reverse, nudge, trim) change a layer's view, never its audio,
//! so they cost no memory, take no time and undo exactly.

/// Pieces at most in one view. Each edit joins pieces that follow on from
/// each other, so a view needs more than two only after a nudge inside a
/// trimmed loop, a reverse of that, and so on.
pub(crate) const MAX_PIECES: usize = 8;

/// Frames of a layer's audio, played one after another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    /// The audio frame the piece starts on.
    pub start: u32,
    /// Frames it plays. It wraps around the audio, more than once if it is
    /// longer.
    pub len: u32,
    /// Backwards.
    pub back: bool,
}

const NO_PIECE: Piece = Piece {
    start: 0,
    len: 0,
    back: false,
};

/// What a layer plays, in order: pieces of its audio of `size` frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct View {
    pieces: [Piece; MAX_PIECES],
    n: u8,
    size: u32,
    len: u32,
}

impl View {
    pub const EMPTY: View = View {
        pieces: [NO_PIECE; MAX_PIECES],
        n: 0,
        size: 0,
        len: 0,
    };

    /// The whole audio, forwards from its first frame.
    pub fn whole(size: usize) -> View {
        let mut v = View {
            size: size as u32,
            ..View::EMPTY
        };
        v.push(Piece {
            start: 0,
            len: size as u32,
            back: false,
        });
        v
    }

    /// Frames it plays.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Frames of the audio it plays from.
    pub fn size(&self) -> usize {
        self.size as usize
    }

    /// The whole audio, forwards from its first frame: playback frame `p`
    /// is audio frame `p`.
    pub fn is_whole(&self) -> bool {
        self.n == 1
            && self.pieces[0]
                == Piece {
                    start: 0,
                    len: self.size,
                    back: false,
                }
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces[..self.n as usize]
    }

    /// The audio frame `off` frames into a piece (negative: before it, as
    /// if it went on backwards).
    pub fn frame(&self, p: &Piece, off: i64) -> usize {
        let size = i64::from(self.size);
        let at = if p.back {
            i64::from(p.start) - off
        } else {
            i64::from(p.start) + off
        };
        wrap(at, size)
    }

    /// The audio frame just after a piece's last one.
    fn next_after(&self, p: &Piece) -> usize {
        self.frame(p, i64::from(p.len))
    }

    /// Whether playback jumps after piece `i` (into the next, or from the
    /// last back to the first): a cut, which a seam fade smooths.
    pub fn cut_after(&self, i: usize) -> bool {
        let p = &self.pieces[i];
        let q = &self.pieces[(i + 1) % self.n as usize];
        p.back != q.back || self.next_after(p) != q.start as usize
    }

    /// The piece playback frame `at` is in, and how far into it.
    pub fn locate(&self, at: usize) -> (usize, usize) {
        let mut at = at;
        for (i, p) in self.pieces().iter().enumerate() {
            if at < p.len as usize {
                return (i, at);
            }
            at -= p.len as usize;
        }
        let last = self.n as usize - 1;
        (last, self.pieces[last].len as usize - 1)
    }

    /// The audio frame playback frame `at` plays.
    pub fn at(&self, at: usize) -> usize {
        let (i, off) = self.locate(at);
        self.frame(&self.pieces[i], off as i64)
    }

    /// Add a piece at the end, joined to the last if it follows on from
    /// it. False if the view is full.
    fn push(&mut self, p: Piece) -> bool {
        if p.len == 0 {
            return true;
        }
        if self.n > 0 {
            let last = self.pieces[self.n as usize - 1];
            if last.back == p.back && self.next_after(&last) == p.start as usize {
                if let Some(len) = last.len.checked_add(p.len) {
                    self.pieces[self.n as usize - 1].len = len;
                    self.len += p.len;
                    return true;
                }
            }
        }
        if self.n as usize == MAX_PIECES {
            return false;
        }
        self.pieces[self.n as usize] = p;
        self.n += 1;
        self.len += p.len;
        true
    }

    /// Playback frames `from..to`.
    pub fn cut(&self, from: usize, to: usize) -> Option<View> {
        let mut v = View {
            size: self.size,
            ..View::EMPTY
        };
        let mut o = 0usize;
        for p in self.pieces() {
            let (a, b) = (from.max(o), to.min(o + p.len as usize));
            if a < b {
                let start = self.frame(p, (a - o) as i64) as u32;
                if !v.push(Piece {
                    start,
                    len: (b - a) as u32,
                    back: p.back,
                }) {
                    return None;
                }
            }
            o += p.len as usize;
        }
        Some(v)
    }

    /// This, then `other` (of the same audio).
    pub fn then(&self, other: &View) -> Option<View> {
        let mut v = *self;
        self.len.checked_add(other.len)?;
        for p in other.pieces() {
            if !v.push(*p) {
                return None;
            }
        }
        Some(v)
    }

    /// Everything `by` frames later, going round: what played at frame
    /// `p` plays at `p + by` (earlier for a negative `by`).
    pub fn rotate(&self, by: i64) -> Option<View> {
        let len = self.len as i64;
        let d = by.rem_euclid(len) as usize;
        if d == 0 {
            return Some(*self);
        }
        let split = self.len as usize - d;
        self.cut(split, self.len as usize)?
            .then(&self.cut(0, split)?)
    }

    /// Backwards: the last frame first.
    pub fn reverse(&self) -> View {
        let mut v = View {
            size: self.size,
            ..View::EMPTY
        };
        for p in self.pieces().iter().rev() {
            let start = self.frame(p, i64::from(p.len) - 1) as u32;
            // Reversed pieces follow on from each other exactly when the
            // originals did, and there are as many: this always fits.
            v.push(Piece {
                start,
                len: p.len,
                back: !p.back,
            });
        }
        v
    }
}

/// `x.rem_euclid(size)`, without a division when `x` is within a size of
/// the audio (nearly always: a piece's frames, a frame just before one).
#[inline]
pub(crate) fn wrap(x: i64, size: i64) -> usize {
    if (0..size).contains(&x) {
        x as usize
    } else if (size..2 * size).contains(&x) {
        (x - size) as usize
    } else if (-size..0).contains(&x) {
        (x + size) as usize
    } else {
        x.rem_euclid(size) as usize
    }
}

/// Finds playback frames' pieces as [`View::locate`] does, but from the
/// piece it found last instead of from the first: a playing loop's
/// frames move on a little at a time, so most finds cost one compare.
pub(crate) struct Cursor<'a> {
    v: &'a View,
    /// The piece it found last, and the playback frame it starts on.
    i: usize,
    p0: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(v: &'a View) -> Cursor<'a> {
        Cursor { v, i: 0, p0: 0 }
    }

    /// The same as `View::locate(at)`.
    #[inline]
    pub fn locate(&mut self, at: usize) -> (usize, usize) {
        if at < self.p0 {
            self.i = 0;
            self.p0 = 0;
        }
        let pieces = self.v.pieces();
        while self.i < pieces.len() {
            let len = pieces[self.i].len as usize;
            if at - self.p0 < len {
                return (self.i, at - self.p0);
            }
            self.p0 += len;
            self.i += 1;
        }
        self.i = 0;
        self.p0 = 0;
        self.v.locate(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plain model: each playback frame's audio frame.
    fn frames(v: &View) -> Vec<usize> {
        (0..v.len()).map(|p| v.at(p)).collect()
    }

    fn rng(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    #[test]
    fn edits_match_the_plain_model() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..3_000 {
            let size = (rng(&mut seed) % 50 + 1) as usize;
            let mut v = View::whole(size);
            let mut model: Vec<usize> = (0..size).collect();
            assert!(v.is_whole());
            for _ in 0..12 {
                let len = model.len();
                let next = match rng(&mut seed) % 5 {
                    0 => {
                        let a = (rng(&mut seed) % len as u64) as usize;
                        let b = a + 1 + (rng(&mut seed) % (len - a) as u64) as usize;
                        model = model[a..b].to_vec();
                        v.cut(a, b)
                    }
                    1 if len < 400 => {
                        model = model.iter().chain(model.iter()).copied().collect();
                        v.then(&v)
                    }
                    2 => {
                        let by = (rng(&mut seed) % 41) as i64 - 20;
                        let d = by.rem_euclid(len as i64) as usize;
                        model.rotate_right(d);
                        v.rotate(by)
                    }
                    3 => {
                        model.reverse();
                        Some(v.reverse())
                    }
                    _ => continue,
                };
                match next {
                    Some(n) => v = n,
                    // Too many pieces: refused, and the model goes back.
                    None => {
                        model = frames(&v);
                        continue;
                    }
                }
                assert_eq!(frames(&v), model);
                assert!(v.pieces().len() <= MAX_PIECES && v.len() == model.len());
                // Pieces are as few as they can be: no two in a row join.
                for w in v.pieces().windows(2) {
                    assert!(
                        !(w[0].back == w[1].back && v.next_after(&w[0]) == w[1].start as usize),
                        "{v:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn cuts_are_where_playback_jumps() {
        let v = View::whole(100);
        assert!(!v.cut_after(0), "the whole audio goes round without a cut");
        assert!(!v.reverse().cut_after(0) && !v.rotate(30).unwrap().cut_after(0));
        assert!(
            !v.then(&v).unwrap().cut_after(0),
            "a doubled loop goes round twice"
        );
        let half = v.cut(0, 50).unwrap();
        assert!(half.cut_after(0), "frame 49 back to frame 0");
        let r = half.rotate(10).unwrap();
        assert_eq!(r.pieces().len(), 2);
        assert!(
            r.cut_after(0) && !r.cut_after(1),
            "the cut moves inside; the loop point joins"
        );
        // Before a piece is the audio before it, round the recorded loop.
        assert_eq!(
            (r.frame(&r.pieces()[0], -1), r.frame(&r.pieces()[1], -1)),
            (39, 99)
        );
        assert!(half.reverse().cut_after(0));
    }

    #[test]
    fn the_pieces_stay_few() {
        // Nudges in a trimmed loop keep it at two pieces.
        let mut v = View::whole(1000).cut(100, 500).unwrap();
        for k in 0..200 {
            v = v.rotate(if k % 3 == 0 { -37 } else { 61 }).unwrap();
            assert!(v.pieces().len() <= 2);
        }
        // Doubling the whole audio is one piece, twice round.
        let d = View::whole(1000).then(&View::whole(1000)).unwrap();
        assert_eq!((d.pieces().len(), d.len()), (1, 2000));
    }

    #[test]
    fn wrap_is_rem_euclid() {
        for size in [1i64, 2, 7, 1000] {
            for x in -5 * size..5 * size {
                assert_eq!(wrap(x, size), x.rem_euclid(size) as usize, "{x} {size}");
            }
        }
        assert_eq!(wrap(i64::from(u32::MAX), 3), (u32::MAX % 3) as usize);
    }

    #[test]
    fn the_cursor_finds_what_locate_finds() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let v = View::whole(1000)
            .cut(100, 700)
            .unwrap()
            .rotate(250)
            .unwrap();
        let v = v
            .then(&View::whole(1000).cut(0, 300).unwrap().reverse())
            .unwrap();
        assert!(v.pieces().len() >= 3);
        let mut c = Cursor::new(&v);
        let mut at = 0usize;
        for _ in 0..50_000 {
            // Mostly a frame or two on, sometimes back or anywhere, or
            // past the end.
            at = match rng(&mut seed) % 20 {
                0 => (rng(&mut seed) % (v.len() as u64 + 10)) as usize,
                1 => at.saturating_sub((rng(&mut seed) % 50) as usize),
                _ => (at + (rng(&mut seed) % 3) as usize) % (v.len() + 3),
            };
            assert_eq!(c.locate(at), v.locate(at), "at {at}");
        }
    }
}

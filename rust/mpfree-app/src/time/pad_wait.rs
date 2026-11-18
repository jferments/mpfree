//! Pad hits that wait for the launch line (sampler R5.3b). A live hit on
//! a pad that starts on the line (a Loop pad, `Pad::on_launch`) waits,
//! while the transport plays, for the project's launch line, as a clip's
//! launch does; its let-go waits for the line after the one its hit
//! plays on. Anything played on the same pad after one that waits waits
//! too, so a pad's hits and let-gos keep their order. Each keeps the kit
//! pad it was made on (bank × 16 + place), so turning PAD BANK before
//! the line still plays that pad, and its let-go lets go of it. The sound loop
//! plays each on its line through the same path as a hit made then
//! (main.rs, `play_waited`), so the arp, the MIDI effects and the clip
//! recorder hear it there.
//!
//! STOP drops the hits still waiting, with their let-gos (a pad still
//! held then lets go of nothing), and plays the let-gos of hits that
//! sounded; the launch quantise set to at once plays everything now.
//! While a pad's hit waits, its pad blinks ([`PadWaits::hits_waiting`]).
//!
//! The table is fixed: nothing here allocates on the audio thread. A hit
//! that finds it full plays at once, and the sound loop counts and says so.

/// Where a waiting hit came from: one of the 16 pads (the MPC's or the
/// screen's, by its place on the page) or a key on a kit pad (0 to 127).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadFrom {
    Pad(u8),
    Key(u8),
}

/// A pad pressed (`down`, at `velocity` as it was pressed) or let go, on
/// the kit's pad `kit_pad` (bank × 16 + place, as it was when made; a
/// key's own pad).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadWait {
    pub from: PadFrom,
    pub down: bool,
    pub velocity: u8,
    pub kit_pad: u8,
}

/// How many hits and let-gos can wait at once: four on each of the 16
/// pads, which hands don't reach between two lines.
pub const MAX_WAITS: usize = 64;

#[derive(Clone, Copy, Debug)]
struct Waiting {
    w: PadWait,
    /// The lines still to pass before it plays, counting the one it plays
    /// on: 1 plays on the next line.
    lines: u8,
}

/// A set of the places a hit comes from: the 16 pads and the 128 keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Froms {
    pads: u16,
    keys: u128,
}

impl Froms {
    fn has(&self, from: PadFrom) -> bool {
        match from {
            PadFrom::Pad(p) => p < 16 && self.pads & (1 << p) != 0,
            PadFrom::Key(k) => k < 128 && self.keys & (1 << k) != 0,
        }
    }

    fn set(&mut self, from: PadFrom, on: bool) {
        match from {
            PadFrom::Pad(p) if p < 16 => {
                self.pads = (self.pads & !(1 << p)) | (u16::from(on) << p);
            }
            PadFrom::Key(k) if k < 128 => {
                self.keys = (self.keys & !(1 << k)) | (u128::from(on) << k);
            }
            _ => {}
        }
    }
}

/// The hits and let-gos waiting for a line, in the order they were made.
#[derive(Clone, Debug)]
pub struct PadWaits {
    list: [Option<Waiting>; MAX_WAITS],
    n: usize,
    /// The pads still held when STOP dropped their hits: their let-go has
    /// nothing to let go of.
    dropped: Froms,
}

impl Default for PadWaits {
    fn default() -> Self {
        PadWaits {
            list: [None; MAX_WAITS],
            n: 0,
            dropped: Froms::default(),
        }
    }
}

impl PadWaits {
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Whether something played on `from` waits.
    pub fn waiting(&self, from: PadFrom) -> bool {
        self.lines_of(from) > 0
    }

    /// The pads of bank `page` (bit `p` for its pad `p`) whose hit, made
    /// on the pads, waits for its line: they blink until it sounds, while
    /// that bank shows.
    pub fn hits_waiting(&self, page: usize) -> u16 {
        self.list[..self.n]
            .iter()
            .flatten()
            .filter(|x| x.w.down && matches!(x.w.from, PadFrom::Pad(_)))
            .map(|x| usize::from(x.w.kit_pad))
            .filter(|&k| k / 16 == page)
            .fold(0, |bits, k| bits | 1 << (k % 16))
    }

    /// Whether `w` is the let-go of a hit STOP dropped while its pad was
    /// held: it has nothing to let go of, so it plays nothing. A new hit
    /// on that pad forgets it.
    pub fn lets_go_of_dropped(&mut self, w: PadWait) -> bool {
        let was = self.dropped.has(w.from);
        self.dropped.set(w.from, false);
        was && !w.down
    }

    /// The most lines anything played on `from` still waits (0: nothing).
    fn lines_of(&self, from: PadFrom) -> u8 {
        self.list[..self.n]
            .iter()
            .flatten()
            .filter(|x| x.w.from == from)
            .map(|x| x.lines)
            .max()
            .unwrap_or(0)
    }

    /// `w` waits: a hit for the next line, a let-go for the line after its
    /// hit's when that hit waits (and on that hit's kit pad, though the
    /// bank turned since), else the next; never before something played
    /// on the same pad before it. False, and nothing kept, when the table
    /// is full.
    pub fn wait(&mut self, mut w: PadWait) -> bool {
        if self.n >= MAX_WAITS {
            return false;
        }
        if !w.down {
            let hit = self.list[..self.n]
                .iter()
                .flatten()
                .rev()
                .find(|x| x.w.from == w.from && x.w.down);
            if let Some(hit) = hit {
                w.kit_pad = hit.w.kit_pad;
            }
        }
        let before = self.lines_of(w.from);
        let own = if !w.down && before > 0 { before + 1 } else { 1 };
        self.list[self.n] = Some(Waiting {
            w,
            lines: own.max(before),
        });
        self.n += 1;
        true
    }

    /// A launch line passes: `play` gets the ones whose line it is, in the
    /// order they were made, and the rest are a line nearer.
    pub fn line(&mut self, mut play: impl FnMut(PadWait)) {
        for x in self.list[..self.n].iter_mut() {
            let Some(waiting) = x else { continue };
            if waiting.lines <= 1 {
                play(waiting.w);
                *x = None;
            } else {
                waiting.lines -= 1;
            }
        }
        self.pack();
    }

    /// The transport stopped: the hits still waiting are dropped, with
    /// their let-gos (one not made yet, the pad still held, is dropped when
    /// it comes: [`PadWaits::lets_go_of_dropped`]), and `play` gets the
    /// let-gos of hits that sounded, in order. Returns how many hits were
    /// dropped.
    pub fn stop(&mut self, mut play: impl FnMut(PadWait)) -> usize {
        let mut dropped = 0;
        // The pads whose hit was dropped here and not let go of yet.
        let mut gone = Froms::default();
        for x in self.list[..self.n].iter_mut() {
            let Some(waiting) = x.take() else { continue };
            let w = waiting.w;
            if w.down {
                dropped += 1;
                gone.set(w.from, true);
            } else if gone.has(w.from) {
                gone.set(w.from, false);
            } else {
                play(w);
            }
        }
        self.n = 0;
        self.dropped.pads |= gone.pads;
        self.dropped.keys |= gone.keys;
        dropped
    }

    /// Every one plays now, in order (the launch quantise went to at
    /// once).
    pub fn all(&mut self, mut play: impl FnMut(PadWait)) {
        for x in self.list[..self.n].iter_mut() {
            if let Some(waiting) = x.take() {
                play(waiting.w);
            }
        }
        self.n = 0;
    }

    /// The ones left, moved up in order.
    fn pack(&mut self) {
        let mut k = 0;
        for i in 0..self.n {
            if let Some(x) = self.list[i].take() {
                self.list[k] = Some(x);
                k += 1;
            }
        }
        self.n = k;
    }
}

/// What waited and plays this period, each with its frame in the period,
/// in order. Fixed: [`MAX_WAITS`] at most.
#[derive(Clone, Debug)]
pub struct DuePads {
    list: [(PadWait, u32); MAX_WAITS],
    n: usize,
}

impl Default for DuePads {
    fn default() -> Self {
        DuePads {
            list: [(
                PadWait {
                    from: PadFrom::Pad(0),
                    down: false,
                    velocity: 0,
                    kit_pad: 0,
                },
                0,
            ); MAX_WAITS],
            n: 0,
        }
    }
}

impl DuePads {
    pub fn clear(&mut self) {
        self.n = 0;
    }

    /// One more, if there is room (there always is: no more can wait).
    pub(super) fn push(&mut self, w: PadWait, offset: u32) {
        if self.n < MAX_WAITS {
            self.list[self.n] = (w, offset);
            self.n += 1;
        }
    }

    pub fn as_slice(&self) -> &[(PadWait, u32)] {
        &self.list[..self.n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(pad: u8) -> PadWait {
        PadWait {
            from: PadFrom::Pad(pad),
            down: true,
            velocity: 100,
            kit_pad: pad,
        }
    }

    fn up(pad: u8) -> PadWait {
        PadWait {
            from: PadFrom::Pad(pad),
            down: false,
            velocity: 0,
            kit_pad: pad,
        }
    }

    fn line(w: &mut PadWaits) -> Vec<PadWait> {
        let mut got = Vec::new();
        w.line(|x| got.push(x));
        got
    }

    #[test]
    fn a_hit_plays_on_the_next_line_and_its_let_go_on_the_line_after() {
        let mut w = PadWaits::default();
        assert!(w.wait(hit(3)) && w.wait(up(3)));
        assert!(w.waiting(PadFrom::Pad(3)) && !w.waiting(PadFrom::Pad(4)));
        assert_eq!(line(&mut w), [hit(3)]);
        assert_eq!(line(&mut w), [up(3)]);
        assert!(w.is_empty() && line(&mut w).is_empty());
    }

    #[test]
    fn a_let_go_after_its_hit_played_waits_for_the_next_line() {
        let mut w = PadWaits::default();
        w.wait(hit(3));
        assert_eq!(line(&mut w), [hit(3)]);
        w.wait(up(3));
        assert_eq!(line(&mut w), [up(3)]);
    }

    #[test]
    fn a_pads_hits_keep_their_order_and_other_pads_dont_wait_for_them() {
        let mut w = PadWaits::default();
        // Start, let go, hit again (a latched loop's stop) before the line:
        // it starts on the first line and stops on the second.
        w.wait(hit(3));
        w.wait(up(3));
        w.wait(hit(3));
        w.wait(hit(5));
        assert_eq!(line(&mut w), [hit(3), hit(5)]);
        assert_eq!(line(&mut w), [up(3), hit(3)]);
        assert!(w.is_empty());
        // A key and a pad are apart.
        let key = PadWait {
            from: PadFrom::Key(3),
            ..hit(3)
        };
        w.wait(hit(3));
        w.wait(up(3));
        w.wait(key);
        assert_eq!(line(&mut w), [hit(3), key]);
    }

    #[test]
    fn at_once_plays_everything_in_order_and_a_full_table_says_no() {
        let mut w = PadWaits::default();
        w.wait(hit(1));
        w.wait(up(1));
        w.wait(hit(2));
        assert_eq!(w.hits_waiting(0), 0b110);
        let mut got = Vec::new();
        w.all(|x| got.push(x));
        assert_eq!(got, [hit(1), up(1), hit(2)]);
        assert!(w.is_empty() && w.hits_waiting(0) == 0);
        for i in 0..MAX_WAITS {
            assert!(w.wait(hit((i % 16) as u8)));
        }
        assert!(!w.wait(hit(0)));
    }

    #[test]
    fn a_stop_drops_the_hits_waiting_with_their_let_gos() {
        let mut w = PadWaits::default();
        // Pad 1 sounded and its let-go waits: that plays. Pad 2's hit and
        // let-go both wait: dropped. Pad 3's hit waits, still held: dropped,
        // and its let-go when it comes too. A key apart from pad 3.
        w.wait(hit(1));
        assert_eq!(line(&mut w), [hit(1)]);
        w.wait(up(1));
        w.wait(hit(2));
        w.wait(up(2));
        w.wait(hit(3));
        let key = PadWait {
            from: PadFrom::Key(3),
            ..up(3)
        };
        assert_eq!(w.hits_waiting(0), 0b1100);
        let mut got = Vec::new();
        assert_eq!(w.stop(|x| got.push(x)), 2);
        assert_eq!(got, [up(1)]);
        assert!(w.is_empty() && w.hits_waiting(0) == 0);
        assert!(!w.lets_go_of_dropped(up(2)) && !w.lets_go_of_dropped(key));
        assert!(w.lets_go_of_dropped(up(3)));
        assert!(!w.lets_go_of_dropped(up(3)));
        // A new hit forgets it: its let-go plays.
        w.wait(hit(4));
        w.stop(|_| {});
        assert!(!w.lets_go_of_dropped(hit(4)) && !w.lets_go_of_dropped(up(4)));
    }

    #[test]
    fn a_hit_keeps_its_bank_and_its_let_go_follows_it() {
        let mut w = PadWaits::default();
        // A5 hit on bank A, the bank turned to B, then let go: the let-go
        // is A5's. It shows on bank A only.
        w.wait(hit(4));
        w.wait(PadWait {
            kit_pad: 16 + 4,
            ..up(4)
        });
        assert_eq!((w.hits_waiting(0), w.hits_waiting(1)), (1 << 4, 0));
        assert_eq!(line(&mut w), [hit(4)]);
        assert_eq!(line(&mut w), [up(4)]);
        // A hit on bank B shows there; a key's hit shows on no bank.
        let b5 = PadWait {
            kit_pad: 16 + 4,
            ..hit(4)
        };
        w.wait(b5);
        w.wait(PadWait {
            from: PadFrom::Key(16 + 5),
            kit_pad: 16 + 5,
            ..hit(5)
        });
        assert_eq!((w.hits_waiting(0), w.hits_waiting(1)), (0, 1 << 4));
    }
}

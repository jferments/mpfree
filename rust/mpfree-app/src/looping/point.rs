//! The loop point view: Snap or Use my tap, A/B on the seam, Move start /
//! Move end, and the audio and hits it shows.

use super::*;

/// The frames of a loop point window at `rate`.
pub(crate) fn point_frames(rate: u32) -> usize {
    (f64::from(rate) * POINT_MS / 1000.0).round() as usize
}

/// `audio`'s peaks in `out.len()` slices, each its loudest sample, 0 to
/// 255 at full scale.
fn peaks_into(audio: &[[f32; 2]], out: &mut [u8]) {
    let n = out.len().max(1);
    for (i, o) in out.iter_mut().enumerate() {
        let (a, b) = (i * audio.len() / n, (i + 1) * audio.len() / n);
        let peak = audio[a..b]
            .iter()
            .fold(0.0f32, |m, x| m.max(x[0].abs()).max(x[1].abs()));
        *o = (peak.min(1.0) * 255.0).round() as u8;
    }
}

/// The hits in a loop point window's `audio` (Move start / Move end's
/// Hit step and the window's marks), into `out`, as frames from where
/// the window begins plus `from`; how many. A hit is a block of 64 frames
/// whose peak is above -34 dB and four times (12 dB) the loudest of the
/// four blocks before it, at least `apart` frames after the last one; it
/// is placed on the first frame in its block at half that peak.
pub(crate) fn hits_into(audio: &[[f32; 2]], from: i64, apart: u32, out: &mut [i64]) -> u8 {
    const BLOCK: usize = 64;
    let peak = |b: &[[f32; 2]]| {
        b.iter()
            .fold(0.0f32, |m, x| m.max(x[0].abs()).max(x[1].abs()))
    };
    let mut before = [0.0f32; 4];
    let mut n = 0;
    let mut last: Option<usize> = None;
    for (k, b) in audio.chunks(BLOCK).enumerate() {
        let p = peak(b);
        let loudest = before.iter().fold(0.0f32, |m, &x| m.max(x));
        let far = last.is_none_or(|l| k * BLOCK >= l + apart as usize);
        if p > 0.02 && p > 4.0 * loudest && far && n < out.len() {
            let i = b
                .iter()
                .position(|x| x[0].abs().max(x[1].abs()) >= 0.5 * p)
                .unwrap_or(0);
            out[n] = from + (k * BLOCK + i) as i64;
            n += 1;
            last = Some(k * BLOCK + i);
        }
        before.rotate_left(1);
        before[3] = p;
    }
    n as u8
}

/// Whether a loop was moved by hand (Move start / Move end, [`Snap::To`])
/// and is there still: moved, but not to what smart loop end found, nor
/// to it with one edge put back (Use my tap or Use my press after a snap).
pub fn hand_moved(s: &Status) -> bool {
    let Some(m) = s.moved.filter(|m| !m.is_none()) else {
        return false;
    };
    !s.found.is_some_and(|f| {
        m == f
            || m == Shift {
                start: f.start,
                end: 0,
            }
            || m == Shift {
                start: 0,
                end: f.end,
            }
    })
}

/// `base` with `edge` put on `f`.
pub(crate) fn with_edge(base: Shift, edge: Edge, f: i64) -> Shift {
    match edge {
        Edge::Start => Shift { start: f, ..base },
        Edge::End => Shift { end: f, ..base },
    }
}

impl LoopHost {
    /// The loop point view's Snap and Use my tap (or press), Keep my tap
    /// and Keep the snap: the picked loop goes to that side of smart loop
    /// end's choice on its next downbeat, with the tempo, by the snap that
    /// gets there (worked out once no other is under way,
    /// [`LoopHost::tend_snap`]); already there, it stays. `keep`: the chip
    /// is put away once there. A/B stops. The line for the log.
    pub fn side(&mut self, side: Side, keep: bool) -> String {
        let n = self.looper.selected();
        self.snap_ask = None;
        self.ab_stop();
        self.moving = None;
        self.side_want = Some((n, side, keep));
        let what = match (side, keep) {
            (Side::Snap, false) => "Snap",
            (Side::Snap, true) => "Keep the snap",
            (Side::Tap, false) => "Use my tap",
            (Side::Tap, true) => "Keep my tap",
        };
        format!(
            "looper: {what}: Loop {} goes there on its next downbeat, if it isn't there",
            n + 1
        )
    }

    /// A footswitch's Snap or Use my tap (the Learn section's Looper
    /// card), with your hands on your instrument: while the hint line
    /// asks, Snap anyway or Not now; else the picked loop goes to that
    /// side ([`LoopHost::side`]), the chip staying. The line for the log,
    /// also when there is nothing to choose.
    pub fn choose(&mut self, side: Side) -> String {
        let what = match side {
            Side::Snap => "Snap",
            Side::Tap => "Use my tap",
        };
        let asked = match side {
            Side::Snap => self.snap_anyway(),
            Side::Tap => self.snap_not_now(),
        };
        if let Some(line) = asked {
            return format!("{line}, from a footswitch");
        }
        let n = self.looper.selected();
        if self.side_of(n).is_none() {
            return format!(
                "looper: {what} from a footswitch: Loop {} has no snap to choose",
                n + 1
            );
        }
        format!("{}, from a footswitch", self.side(side, false))
    }

    /// A/B on the seam on the picked loop ([`Command::AbSeam`]): each
    /// time round, from half way, its last beat runs into its downbeat as
    /// the other side (the snap, or your tap) has it, then as it is. Its
    /// length and the tempo never change, so the time engine is asked
    /// nothing (the owner's looper review, 5.6). Until a side is kept
    /// ([`LoopHost::side`]) or A/B goes off (the loop stays as it is,
    /// from the next half way). The line for the log, or why not.
    pub fn set_ab(&mut self, on: bool) -> String {
        let n = self.looper.selected();
        if !on {
            self.ab_stop();
            return format!("looper: A/B off: Loop {} stays as it is", n + 1);
        }
        // A move set up: A/B plays it against the loop as it is.
        let s = self.looper.loop_status(n);
        if let Some((s, to)) = s.and_then(|s| self.moving_now(n, &s).map(|to| (s, to))) {
            self.snap_ask = None;
            self.side_want = None;
            if !self.looper.command(Command::AbMove(to)) {
                return "looper: A/B: NOT done, the command queue is full".to_string();
            }
            self.ab = Some((n, s.version));
            return format!(
                "looper: A/B: each time round, Loop {}'s last beat runs into its downbeat as \
                 your move has it, then as it is; the tempo stays",
                n + 1
            );
        }
        let (Some(now), Some(s)) = (self.side_of(n), self.looper.loop_status(n)) else {
            return format!(
                "looper: Loop {}: A/B needs your tap and a snap to swap between",
                n + 1
            );
        };
        self.snap_ask = None;
        self.side_want = None;
        if !self.looper.command(Command::AbSeam(true)) {
            return "looper: A/B: NOT done, the command queue is full".to_string();
        }
        self.ab = Some((n, s.version));
        let other = match now {
            Side::Tap => "the snap",
            Side::Snap => "your tap",
        };
        format!(
            "looper: A/B: each time round, Loop {}'s last beat runs into its downbeat as \
             {other} has it, then as it is; the tempo stays",
            n + 1
        )
    }

    /// A/B goes off, if it is on: the looper finishes the round it is in
    /// on the loop's own side.
    pub(crate) fn ab_stop(&mut self) {
        if self.ab.take().is_some() {
            self.looper.command(Command::AbSeam(false));
        }
    }

    /// The loop point view opened or closed: while open, the picked
    /// loop's audio round its start and end is read for it.
    pub fn set_point_open(&mut self, open: bool) {
        self.point_open = open;
        if !open {
            self.point = None;
        }
    }

    /// Which side of smart loop end's choice loop `n` is on: your tap
    /// while something is on offer, the snap while smart loop end or a
    /// snap moved it and nothing is. None when it is neither (it changed
    /// in another way, or nothing was found).
    fn side_of(&self, n: usize) -> Option<Side> {
        let s = self.looper.loop_status(n)?;
        // Moved by hand: away from your tap, as a snap is.
        if hand_moved(&s) {
            return Some(Side::Snap);
        }
        match (s.offer, s.moved) {
            (Some(_), _) => Some(Side::Tap),
            (None, Some(m)) if !m.is_none() => Some(Side::Snap),
            _ => None,
        }
    }

    /// The snap that takes loop `n` to `side`: Redo or Snap to the snap,
    /// Undo (of a snap) or Use my tap (or press) back. None if it is there
    /// already (or neither side).
    pub(crate) fn snap_to(&self, n: usize, side: Side) -> Option<Snap> {
        let now = self.side_of(n)?;
        if now == side {
            return None;
        }
        let s = self.looper.loop_status(n)?;
        let h = self.looper.loop_history(n)?;
        Some(match side {
            // From a move by hand, back to REC and your tap at once.
            Side::Tap if hand_moved(&s) => Snap::To(Shift::NONE),
            Side::Snap if matches!(h.redo().first(), Some(Step::Snap(_))) => Snap::Redo,
            Side::Snap => Snap::Found,
            Side::Tap if matches!(h.done().last(), Some(Step::Snap(_))) => Snap::Undo,
            Side::Tap if s.moved.is_some_and(|m| m.end != 0) => Snap::Tap,
            Side::Tap => Snap::Press,
        })
    }

    /// Smart loop end on the picked loop, for the screen: the close call
    /// while the loop is as it was read, the chip put away, the question
    /// waiting, A/B, and the loop point view's audio while the view is
    /// open (read again only when the loop, or where smart loop end left
    /// it, changes; kept while a snap lands).
    pub(crate) fn smart_view(&mut self, s: &Status) -> SmartView {
        let n = self.looper.selected();
        if self.point_open {
            self.read_point(n, s);
        }
        SmartView {
            close_call: self
                .close_call
                .filter(|c| c.0 == n && c.1 == s.version)
                .map(|c| c.2),
            kept: self.kept.is_some_and(|k| k == (n, s.version)),
            asking: self
                .snap_ask
                .filter(|a| a.0 == n)
                .map(|a| (a.1, self.reads_again(a.1))),
            length: s.asks,
            ab: self.ab == Some((n, s.version)) && s.ab.is_some(),
            point: self
                .point
                .filter(|p| self.point_open && p.0 == n)
                .map(|p| p.4),
            moving: self.moving_now(n, s).map(|to| Moving {
                to,
                makes: self.looper.snap_plan(n, Snap::To(to), 0).map(|p| p.bpm),
            }),
            move_step: self.move_step,
        }
    }

    /// The move set up on loop `n` (status `s`), while the loop is still
    /// as it was then.
    fn moving_now(&self, n: usize, s: &Status) -> Option<Shift> {
        self.moving
            .filter(|m| m.0 == n && m.1 == s.version)
            .map(|m| m.2)
    }

    /// The loop point view's Move start / Move end on the picked loop
    /// (while its first pass is alone, as kept: [`Status::bounds`]). A
    /// move is set up first, from where the loop is: a finger or a hit
    /// puts an edge somewhere ([`MoveAct::To`]), a step moves it by a
    /// sixteenth note at the loop's tempo, a millisecond or to the next hit
    /// heard ([`MoveAct::Step`], [`MoveAct::Scale`]); A/B, while on, plays
    /// it against the loop as it is ([`Command::AbMove`]). Move keeps it:
    /// a snap ([`Snap::To`]) on the loop's next downbeat, the tempo with
    /// it, logged with what smart loop end found as a labelled example.
    /// Cancel puts it away. The line for the log, when there is one.
    pub fn move_act(&mut self, a: MoveAct) -> Option<String> {
        let n = self.looper.selected();
        if let MoveAct::Scale(st) = a {
            self.move_step = st;
            return None;
        }
        let s = self.looper.loop_status(n)?;
        let Some(b) = s.bounds else {
            self.moving = None;
            return Some(format!(
                "looper: Loop {}: its start and end can't move: it has changed since it closed",
                n + 1
            ));
        };
        let now = s.moved.unwrap_or(Shift::NONE);
        let base = self.moving_now(n, &s).unwrap_or(now);
        let to = match a {
            MoveAct::Scale(_) => return None,
            MoveAct::Cancel => {
                self.moving = None;
                if self.ab.is_some() {
                    self.ab_stop();
                }
                return Some(format!(
                    "looper: Loop {}: the move put away; the loop stays as it is",
                    n + 1
                ));
            }
            MoveAct::Keep => {
                let Some(to) = self.moving_now(n, &s) else {
                    return Some(format!("looper: Loop {}: no move to keep", n + 1));
                };
                self.moving = None;
                let line = self.snap_on(n, Snap::To(to));
                return Some(format!("{line}; {}", self.move_example(n, &s, to)));
            }
            MoveAct::To(edge, f) => with_edge(base, edge, f),
            MoveAct::Step(edge, d) => {
                let at = match edge {
                    Edge::Start => base.start,
                    Edge::End => base.end,
                };
                let f = match self.move_step {
                    MoveStep::Hit => self.next_hit(n, edge, at, d)?,
                    MoveStep::Ms => at + i64::from(d) * i64::from(self.rate / 1000),
                    MoveStep::Sixteenth => {
                        let ticks = i128::from(s.ticks.max(1));
                        let q = i128::from(mpfree_transport::time::PPQ) * s.frames as i128;
                        let sixteenth = ((q + 2 * ticks) / (4 * ticks)) as i64;
                        at + i64::from(d) * sixteenth.max(1)
                    }
                };
                with_edge(base, edge, f)
            }
        };
        // Within what the first pass kept, and a sliver of loop at least.
        let shortest = i64::from(self.rate / 50);
        let (press, tap) = b.captured;
        let start = to
            .start
            .clamp(-press, (tap + to.end - press - shortest).max(-press));
        let end = to.end.clamp(
            (press + start - tap + shortest).min(b.kept - tap),
            b.kept - tap,
        );
        let to = Shift { start, end };
        self.moving = (to != now).then_some((n, s.version, to));
        if self.ab.is_some() {
            let _ = self.looper.command(Command::AbMove(to));
        }
        None
    }

    /// A kept move as a labelled example for smart loop end's corpora:
    /// where you put the start and the end, from your press and tap, and
    /// what it had found.
    fn move_example(&self, n: usize, s: &Status, to: Shift) -> String {
        let r = self.rate;
        let from = |f: i64, you: &str| match f.signum() {
            0 => format!("on {you}"),
            -1 => format!("{:.0} ms before {you}", ms(f, r)),
            _ => format!("{:.0} ms after {you}", ms(f, r)),
        };
        let found = match s.found {
            Some(f) => format!(
                "it found the start {} and the end {}",
                from(f.start, "REC"),
                from(f.end, "your tap")
            ),
            None => "it found nothing".to_string(),
        };
        format!(
            "Loop {} moved by hand: the start {}, the end {} ({found}; a labelled example)",
            n + 1,
            from(to.start, "REC"),
            from(to.end, "your tap")
        )
    }

    /// The next hit heard (Move start / Move end's Hit step) after `at`
    /// (`d` above 0) or before it, in that edge's window, more than 2 ms
    /// away. None if there is none in view.
    fn next_hit(&self, n: usize, edge: Edge, at: i64, d: i8) -> Option<i64> {
        let p = self.point.filter(|p| p.0 == n)?.4;
        let k = usize::from(edge == Edge::End);
        let hits = &p.hits[k][..usize::from(p.n_hits[k])];
        let gap = i64::from(self.rate / 500);
        if d > 0 {
            hits.iter().copied().find(|&h| h > at + gap)
        } else {
            hits.iter().rev().copied().find(|&h| h < at - gap)
        }
    }

    /// The loop point view's audio for loop `n` (status `s`): the windows
    /// round your press and your tap, from the first pass as it was kept.
    fn read_point(&mut self, n: usize, s: &Status) {
        // (Not moved is None.)
        let moved = s.moved.unwrap_or(Shift::NONE);
        let moving = self.moving_now(n, s);
        if self
            .point
            .is_some_and(|p| p.0 == n && p.1 == s.version && p.2 == moved && p.3 == moving)
        {
            return;
        }
        let frames = point_frames(self.rate).min(self.point_buf.len());
        let buf = &mut self.point_buf[..frames];
        let Some((start, len)) = self.looper.read_kept(n, 0, &mut buf[..0]) else {
            return;
        };
        let ms = |m: f64| (m * f64::from(self.rate) / 1000.0).round() as i64;
        let press = start as i64 - moved.start;
        let tap = (start + len) as i64 - moved.end;
        // Each window where it always is, unless the edge (or the move)
        // is out of it: then round that.
        let edge = moving.unwrap_or(moved);
        let width = frames as i64;
        let place = |from: i64, e: i64| {
            if e < from + width / 10 || e > from + width * 9 / 10 {
                e - width / 2
            } else {
                from
            }
        };
        let from = [
            place(ms(POINT_START_FROM_MS), edge.start),
            place(ms(POINT_END_FROM_MS), edge.end),
        ];
        let mut point = LoopPoint {
            start: [0; POINT_BARS],
            end: [0; POINT_BARS],
            from,
            hits: [[0; POINT_HITS]; 2],
            n_hits: [0; 2],
        };
        let silent = self.rate / 30;
        self.looper.read_kept(n, press + from[0], buf);
        peaks_into(buf, &mut point.start);
        point.n_hits[0] = hits_into(buf, from[0], silent, &mut point.hits[0]);
        self.looper.read_kept(n, tap + from[1], buf);
        peaks_into(buf, &mut point.end);
        point.n_hits[1] = hits_into(buf, from[1], silent, &mut point.hits[1]);
        self.point = Some((n, s.version, moved, moving, point));
    }
}

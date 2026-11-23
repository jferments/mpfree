//! Playing a set (live shows B1): which song plays, which one is armed to
//! go next, and what each press of the set's controls does. Pure logic, so
//! every rule is tested here; the screen thread feeds it presses and the
//! song changes' progress, and acts on what it answers.
//!
//! The rules, from the approved plan's "Playing a set":
//! - **Arm, then go.** Next song arms the song after the one playing; a
//!   second press goes. Two presses while stopped too, because a stopped
//!   transport doesn't mean the song is over; with Quick advance (Prefs)
//!   one press goes while stopped. Previous song arms the song before.
//! - **Nothing goes by accident.** A tap on the song playing or the one
//!   armed does nothing, so a double tap never goes. Each control's
//!   presses are cleaned into one: a second within [`DEBOUNCE_MS`] of the
//!   last is the same press. While a change is under way every press is
//!   ignored, except Go while the old song finishes, which goes now.
//! - **Cancel next** drops the armed song without touching the music, and
//!   **Back to the set order** arms the song after the one playing.
//! - A song the check found Not ready can be armed (to see why) but Go
//!   does nothing.

/// A second press of the same control within this many milliseconds is
/// the same press (a footswitch's bounce).
pub const DEBOUNCE_MS: u64 = 50;

/// A press of one of the set's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    NextSong,
    PreviousSong,
    Go,
    CancelNext,
    BackToOrder,
    /// The wheel or – / +: the armed song moves one after (+1) or before
    /// (-1), from the song playing when none is armed.
    Step(i8),
    /// A tap on a song in the set column.
    Arm(usize),
}

/// What a press comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Nothing changes, and the hint line says why (or nothing, for a
    /// press that is the same as the last one).
    Nothing(Option<&'static str>),
    /// This song is armed now.
    Armed(usize),
    /// The armed song is dropped.
    Disarmed,
    /// Start the change to this song.
    Go(usize),
    /// Go again while the old song finishes: switch now.
    GoNow,
}

/// Where a change stands, as the Show view's pill and card say it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Change {
    /// No change under way.
    #[default]
    Idle,
    /// The old song is ending (Let it finish: Go again goes now).
    Finishing,
    /// The old song's takes are being written before the new one opens.
    SavingTakes,
    /// The new song is opening.
    Opening,
    /// The open failed after the old song had gone: nothing plays.
    Failed,
}

/// A set being played.
#[derive(Clone, Debug)]
pub struct Show {
    /// Whether each song may go (its last check found nothing that stops
    /// it), in the set's order.
    can_go: Vec<bool>,
    /// The song playing, or loaded and waiting for PLAY.
    now: usize,
    armed: Option<usize>,
    change: Change,
    /// The song a change is going to.
    going: Option<usize>,
    /// Each control's last accepted press, by the caller's id for it.
    last: Vec<(u32, u64)>,
}

impl Show {
    /// A set of songs whose first song is open. `can_go` holds one entry a
    /// song.
    pub fn new(can_go: Vec<bool>) -> Show {
        Show {
            can_go,
            now: 0,
            armed: None,
            change: Change::Idle,
            going: None,
            last: Vec::new(),
        }
    }

    /// The set changed while it plays: the song playing is song `k` of the
    /// new list, and nothing is armed.
    pub fn start_at(&mut self, k: usize) {
        self.now = k.min(self.songs().saturating_sub(1));
        self.armed = None;
        self.going = None;
        self.change = Change::Idle;
    }

    pub fn now(&self) -> usize {
        self.now
    }

    pub fn armed(&self) -> Option<usize> {
        self.armed
    }

    pub fn change(&self) -> Change {
        self.change
    }

    pub fn songs(&self) -> usize {
        self.can_go.len()
    }

    /// A new check's results.
    pub fn set_can_go(&mut self, can_go: Vec<bool>) {
        if can_go.len() == self.can_go.len() {
            self.can_go = can_go;
        }
    }

    /// The song after the one playing in the set's order, if any.
    pub fn next_in_order(&self) -> Option<usize> {
        (self.now + 1 < self.songs()).then_some(self.now + 1)
    }

    /// A press from control `from` (any id the caller keeps per control:
    /// a footswitch, the wheel, a tap) at `ms` (a monotonic clock), with
    /// the transport `playing` or not and Quick advance on or off.
    pub fn press(
        &mut self,
        press: Press,
        from: u32,
        ms: u64,
        playing: bool,
        quick: bool,
    ) -> Answer {
        if self.bounced(from, ms) {
            return Answer::Nothing(None);
        }
        match self.change {
            Change::Idle => {}
            Change::Finishing if press == Press::Go => return Answer::GoNow,
            Change::Failed => {
                // Only arming another song makes sense with nothing open;
                // Try again and Reopen are the card's own buttons.
                return match press {
                    Press::Arm(k) if k < self.songs() => self.arm(k),
                    _ => Answer::Nothing(Some(
                        "The song didn\u{2019}t open: Try again, Reopen, or tap another song",
                    )),
                };
            }
            _ => return Answer::Nothing(Some("A song is changing: wait for it")),
        }
        match press {
            Press::NextSong | Press::PreviousSong => {
                let target = if press == Press::NextSong {
                    self.next_in_order()
                } else {
                    self.now.checked_sub(1)
                };
                let Some(target) = target else {
                    return Answer::Nothing(Some(if press == Press::NextSong {
                        "This is the set\u{2019}s last song"
                    } else {
                        "This is the set\u{2019}s first song"
                    }));
                };
                if self.armed == Some(target) || (quick && !playing && press == Press::NextSong) {
                    self.armed = Some(target);
                    return self.go();
                }
                self.arm(target)
            }
            Press::Go => self.go(),
            Press::CancelNext => match self.armed.take() {
                Some(_) => Answer::Disarmed,
                None => Answer::Nothing(Some("No song is armed")),
            },
            Press::BackToOrder => match self.next_in_order() {
                Some(k) if self.armed == Some(k) => Answer::Nothing(None),
                Some(k) => self.arm(k),
                None => Answer::Nothing(Some("This is the set\u{2019}s last song")),
            },
            Press::Step(d) => {
                let from = self.armed.unwrap_or(self.now) as i64;
                let mut k = from + i64::from(d.signum());
                if k == self.now as i64 {
                    k += i64::from(d.signum());
                }
                if k < 0 || k >= self.songs() as i64 {
                    return Answer::Nothing(None);
                }
                self.arm(k as usize)
            }
            Press::Arm(k) => {
                if k >= self.songs() || k == self.now || self.armed == Some(k) {
                    Answer::Nothing(None)
                } else {
                    self.arm(k)
                }
            }
        }
    }

    fn bounced(&mut self, from: u32, ms: u64) -> bool {
        match self.last.iter_mut().find(|(id, _)| *id == from) {
            Some((_, t)) if ms.saturating_sub(*t) < DEBOUNCE_MS => true,
            Some((_, t)) => {
                *t = ms;
                false
            }
            None => {
                self.last.push((from, ms));
                false
            }
        }
    }

    fn arm(&mut self, k: usize) -> Answer {
        self.armed = Some(k);
        Answer::Armed(k)
    }

    fn go(&mut self) -> Answer {
        let Some(k) = self.armed else {
            return Answer::Nothing(Some("Arm a song first: Next song, the wheel or a tap"));
        };
        if !self.can_go.get(k).copied().unwrap_or(false) {
            return Answer::Nothing(Some("That song isn\u{2019}t ready: see why on its card"));
        }
        self.change = Change::Finishing;
        self.going = Some(k);
        Answer::Go(k)
    }

    /// The change moved on: the old song is quiet and its takes are being
    /// written, or the new song is opening.
    pub fn changed_to(&mut self, change: Change) {
        if self.going.is_some() || change == Change::Idle {
            self.change = change;
        }
    }

    /// The new song is in: it is the song playing now, and nothing is
    /// armed.
    pub fn opened(&mut self) {
        if let Some(k) = self.going.take() {
            self.now = k;
        }
        self.armed = None;
        self.change = Change::Idle;
    }

    /// The open failed after the old song had gone: nothing plays, and
    /// the song it was going to stays armed for Try again.
    pub fn failed(&mut self) {
        self.armed = self.going.take().or(self.armed);
        self.change = Change::Failed;
    }

    /// Try again (or another song armed, then Go): the change starts over
    /// at its open.
    pub fn retry(&mut self) -> Option<usize> {
        if self.change != Change::Failed {
            return None;
        }
        let k = self.armed?;
        self.going = Some(k);
        self.change = Change::Opening;
        Some(k)
    }

    /// Reopen: the song that was playing opens again, as it was left.
    pub fn reopen(&mut self) -> Option<usize> {
        if self.change != Change::Failed {
            return None;
        }
        self.armed = None;
        self.going = Some(self.now);
        self.change = Change::Opening;
        Some(self.now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 1;
    const WHEEL: u32 = 2;
    const TAP: u32 = 3;

    fn show(n: usize) -> Show {
        Show::new(vec![true; n])
    }

    #[test]
    fn next_song_arms_then_a_second_press_goes_even_while_stopped() {
        let mut s = show(4);
        assert_eq!(
            s.press(Press::NextSong, FS, 0, true, false),
            Answer::Armed(1)
        );
        assert_eq!(
            s.press(Press::NextSong, FS, 400, true, false),
            Answer::Go(1)
        );
        assert_eq!(s.change(), Change::Finishing);
        s.opened();
        assert_eq!((s.now(), s.armed(), s.change()), (1, None, Change::Idle));

        // Stopped: still two presses, unless Quick advance.
        assert_eq!(
            s.press(Press::NextSong, FS, 1000, false, false),
            Answer::Armed(2)
        );
        assert_eq!(
            s.press(Press::CancelNext, FS, 1400, false, false),
            Answer::Disarmed
        );
        assert_eq!(
            s.press(Press::NextSong, FS, 1800, false, true),
            Answer::Go(2)
        );
        s.opened();
        // Playing, Quick advance changes nothing.
        assert_eq!(
            s.press(Press::NextSong, FS, 2200, true, true),
            Answer::Armed(3)
        );
        // Previous song arms the one before the song playing.
        assert_eq!(
            s.press(Press::PreviousSong, FS, 2600, true, false),
            Answer::Armed(1)
        );
        assert_eq!(
            s.press(Press::PreviousSong, FS, 3000, true, false),
            Answer::Go(1)
        );
    }

    #[test]
    fn a_bounce_is_one_press_and_presses_during_a_change_are_ignored() {
        let mut s = show(3);
        assert_eq!(
            s.press(Press::NextSong, FS, 0, true, false),
            Answer::Armed(1)
        );
        // A bounce 30 ms later is the same press: no go.
        assert_eq!(
            s.press(Press::NextSong, FS, 30, true, false),
            Answer::Nothing(None)
        );
        // Another control isn't the same press.
        assert_eq!(s.press(Press::Go, TAP, 40, true, false), Answer::Go(1));
        // During the change: ignored, except Go while it finishes.
        assert!(matches!(
            s.press(Press::NextSong, FS, 500, true, false),
            Answer::Nothing(Some(_))
        ));
        assert_eq!(s.press(Press::Go, FS, 600, true, false), Answer::GoNow);
        s.changed_to(Change::Opening);
        assert!(matches!(
            s.press(Press::Go, FS, 700, true, false),
            Answer::Nothing(Some(_))
        ));
        assert!(matches!(
            s.press(Press::CancelNext, FS, 800, true, false),
            Answer::Nothing(Some(_))
        ));
        s.opened();
        assert_eq!(s.now(), 1);
        // Two quick presses never skip a song: the second goes to song 2,
        // never song 3.
        assert_eq!(
            s.press(Press::NextSong, FS, 1000, true, false),
            Answer::Armed(2)
        );
        assert_eq!(
            s.press(Press::NextSong, FS, 1060, true, false),
            Answer::Go(2)
        );
        s.opened();
        assert!(matches!(
            s.press(Press::NextSong, FS, 2000, true, false),
            Answer::Nothing(Some(_))
        ));
    }

    #[test]
    fn taps_and_the_wheel_arm_but_never_go() {
        let mut s = show(5);
        assert_eq!(
            s.press(Press::Arm(3), TAP, 0, true, false),
            Answer::Armed(3)
        );
        // A tap on the armed song, or the song playing, does nothing.
        assert_eq!(
            s.press(Press::Arm(3), TAP, 300, true, false),
            Answer::Nothing(None)
        );
        assert_eq!(
            s.press(Press::Arm(0), TAP, 600, true, false),
            Answer::Nothing(None)
        );
        assert_eq!(s.armed(), Some(3));
        // Back to the set order arms the song after the one playing.
        assert_eq!(
            s.press(Press::BackToOrder, TAP, 900, true, false),
            Answer::Armed(1)
        );
        // The wheel steps the armed song, skipping the song playing.
        assert_eq!(
            s.press(Press::Step(-1), WHEEL, 1000, true, false),
            Answer::Nothing(None)
        );
        assert_eq!(s.armed(), Some(1));
        assert_eq!(
            s.press(Press::Step(1), WHEEL, 1100, true, false),
            Answer::Armed(2)
        );
        let mut t = show(3);
        t.press(Press::Arm(2), TAP, 0, true, false);
        t.press(Press::Go, FS, 100, true, false);
        t.opened();
        assert_eq!(
            t.press(Press::Step(-1), WHEEL, 200, true, false),
            Answer::Armed(1)
        );
        assert_eq!(
            t.press(Press::Step(-1), WHEEL, 300, true, false),
            Answer::Armed(0)
        );
        // Go with nothing armed says so.
        let mut u = show(2);
        assert!(matches!(
            u.press(Press::Go, FS, 0, true, false),
            Answer::Nothing(Some(_))
        ));
    }

    #[test]
    fn a_song_that_isnt_ready_arms_but_doesnt_go_and_a_failed_open_offers_retry() {
        let mut s = Show::new(vec![true, false, true]);
        assert_eq!(
            s.press(Press::NextSong, FS, 0, true, false),
            Answer::Armed(1)
        );
        assert!(matches!(
            s.press(Press::Go, FS, 300, true, false),
            Answer::Nothing(Some(_))
        ));
        assert_eq!(s.change(), Change::Idle);
        s.set_can_go(vec![true, true, true]);
        assert_eq!(s.press(Press::Go, FS, 600, true, false), Answer::Go(1));
        s.changed_to(Change::SavingTakes);
        s.changed_to(Change::Opening);
        s.failed();
        assert_eq!(
            (s.change(), s.armed(), s.now()),
            (Change::Failed, Some(1), 0)
        );
        // With nothing open, only a tap on another song does anything.
        assert!(matches!(
            s.press(Press::NextSong, FS, 900, false, false),
            Answer::Nothing(Some(_))
        ));
        assert_eq!(s.retry(), Some(1));
        assert_eq!(s.change(), Change::Opening);
        s.failed();
        assert_eq!(s.reopen(), Some(0));
        s.opened();
        assert_eq!((s.now(), s.armed(), s.change()), (0, None, Change::Idle));
        assert_eq!(s.retry(), None);
    }
}

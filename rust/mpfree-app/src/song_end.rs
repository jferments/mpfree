//! How a set's song ends when the next one takes over (live shows B1, the
//! approved plan's "How the old song ends"). The audio loop runs it: the
//! screen's set player starts an ending at Go, and hears when it is over.
//!
//! Every ending is built from the same steps: the transport stops (no new
//! notes; the audio loop sends STOP when the song is rolling), held notes
//! go when the player lets go, voices and effects finish on their own
//! tails, and only then, where the ending asks for it, a short fade of
//! whatever is left.
//!
//! - **Let it finish** ([`End::Finish`], the default): the tails ring out
//!   until the song's sound is quiet (its peak under -60 dBFS for 300 ms
//!   in a row; a rise back over -54 dB starts the 300 ms again), 10 s of
//!   real time at most, then a 50 ms fade. The set player shows FINISHING
//!   with the seconds left, and Go again goes now ([`End::Now`], the 50 ms
//!   fade at once).
//! - **Fade** ([`End::Fade`]): a fade over the song's seconds.
//! - **Cut** ([`End::Cut`]): a 10 ms fade.
//!
//! The fade is a gain of its own on the output, after the mixer and
//! before the metronome: the song's master fader never moves, so the song
//! isn't changed by its ending. Once over, the output stays silent until
//! the next song opens ([`SongEnd::reset`]), so the old song never sounds
//! again over the change.
//!
//! B1 measures quiet on the mixer's output before the metronome: the
//! line input counts while its monitoring is on, and the 10 s limit ends
//! the wait whatever it hears.

/// The peak under which the song counts as quiet: -60 dBFS.
pub const QUIET: f32 = 0.001;
/// A rise over this (-54 dBFS) while waiting starts the quiet time again.
pub const LOUD_AGAIN: f32 = 0.001_995;
/// How long the song stays quiet before it counts as finished.
pub const QUIET_MS: u64 = 300;
/// The longest Let it finish waits, from the press.
pub const LONGEST_MS: u64 = 10_000;
/// The fade after Let it finish, and for Go now.
pub const LAST_FADE_MS: u64 = 50;
/// Cut's fade.
pub const CUT_MS: u64 = 10;

/// How the song playing ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// Let it finish: the tails ring out, until quiet or 10 s.
    Finish,
    /// A fade over this many seconds (1 to 30).
    Fade(u8),
    /// A 10 ms fade.
    Cut,
    /// Go again during Let it finish: the 50 ms fade, now.
    Now,
}

/// What the set player hears of an ending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum News {
    /// Let it finish waits: at most this many seconds left.
    Finishing(u32),
    /// The song is silent: the next one can open.
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// No ending: the output as it is.
    Idle,
    /// Let it finish, waiting for quiet: the frame it went quiet, if it is.
    Ringing { quiet_from: Option<u64> },
    /// The gain falls this much a frame.
    Fading { step: f32 },
    /// Over: silent until the next song opens.
    Silent,
}

/// A song's ending, as the audio loop runs it.
#[derive(Clone, Debug)]
pub struct SongEnd {
    rate: u32,
    phase: Phase,
    gain: f32,
    /// Frames since the ending started.
    frames: u64,
    /// The seconds left last said, so each is said once.
    said: Option<u32>,
}

impl SongEnd {
    pub fn new(rate: u32) -> SongEnd {
        SongEnd {
            rate: rate.max(1),
            phase: Phase::Idle,
            gain: 1.0,
            frames: 0,
            said: None,
        }
    }

    /// An ending is under way or over (the output isn't as it was).
    pub fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// The ending is over and the output silent.
    #[cfg(test)]
    pub fn silent(&self) -> bool {
        self.phase == Phase::Silent
    }

    /// Start `end`. Go now ([`End::Now`]) during Let it finish fades at
    /// once; any other start while an ending runs is ignored, so a second
    /// press never restarts one.
    pub fn start(&mut self, end: End) {
        match (self.phase, end) {
            (Phase::Ringing { .. }, End::Now) | (Phase::Idle, End::Now) => {
                self.fade_over(LAST_FADE_MS)
            }
            (Phase::Idle, End::Finish) => {
                self.frames = 0;
                self.said = None;
                self.phase = Phase::Ringing { quiet_from: None };
            }
            (Phase::Idle, End::Fade(s)) => self.fade_over(u64::from(s.clamp(1, 30)) * 1000),
            (Phase::Idle, End::Cut) => self.fade_over(CUT_MS),
            _ => {}
        }
    }

    fn fade_over(&mut self, ms: u64) {
        let frames = (u64::from(self.rate) * ms / 1000).max(1);
        self.phase = Phase::Fading {
            step: self.gain / frames as f32,
        };
    }

    /// Full sound again, with no ending: the next song opened, or the
    /// song that was ending opens again (Reopen).
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.gain = 1.0;
        self.frames = 0;
        self.said = None;
    }

    /// One period: `peak` is the song's sound this period (before the
    /// metronome), and `left` and `right` take the ending's gain. What the
    /// set player should hear, if anything.
    pub fn period(&mut self, peak: f32, left: &mut [f32], right: &mut [f32]) -> Option<News> {
        let n = left.len().min(right.len()) as u64;
        let start = self.frames;
        self.frames += n;
        match self.phase {
            Phase::Idle => None,
            Phase::Ringing { quiet_from } => {
                let quiet_from = if peak > LOUD_AGAIN {
                    None
                } else if peak < QUIET {
                    Some(quiet_from.unwrap_or(start))
                } else {
                    quiet_from
                };
                let ms = |f: u64| f * 1000 / u64::from(self.rate);
                let quiet = quiet_from.is_some_and(|q| ms(self.frames - q) >= QUIET_MS);
                if quiet || ms(self.frames) >= LONGEST_MS {
                    self.fade_over(LAST_FADE_MS);
                    return None;
                }
                self.phase = Phase::Ringing { quiet_from };
                let left_ms = LONGEST_MS.saturating_sub(ms(self.frames));
                let secs = left_ms.div_ceil(1000) as u32;
                (self.said != Some(secs)).then(|| {
                    self.said = Some(secs);
                    News::Finishing(secs)
                })
            }
            Phase::Fading { step } => {
                for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                    self.gain = (self.gain - step).max(0.0);
                    *l *= self.gain;
                    *r *= self.gain;
                }
                if self.gain > 0.0 {
                    return None;
                }
                self.phase = Phase::Silent;
                Some(News::Ended)
            }
            Phase::Silent => {
                left.fill(0.0);
                right.fill(0.0);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;
    const N: usize = 128;

    /// Runs periods of a constant level until `f` says stop or `max`
    /// periods pass; the news heard and the last period's samples.
    fn run(e: &mut SongEnd, level: f32, max: usize) -> (Vec<News>, usize, [f32; N]) {
        let mut news = Vec::new();
        let mut buf = [level; N];
        for k in 0..max {
            let (mut l, mut r) = ([level; N], [level; N]);
            if let Some(n) = e.period(level, &mut l, &mut r) {
                news.push(n);
                if n == News::Ended {
                    return (news, k + 1, l);
                }
            }
            buf = l;
        }
        (news, max, buf)
    }

    fn periods_for(ms: u64) -> usize {
        (RATE as u64 * ms / 1000).div_ceil(N as u64) as usize
    }

    #[test]
    fn with_no_ending_the_output_is_untouched() {
        let mut e = SongEnd::new(RATE);
        let (mut l, mut r) = ([0.5; N], [0.5; N]);
        assert_eq!(e.period(0.5, &mut l, &mut r), None);
        assert_eq!(l, [0.5; N]);
        assert!(!e.active());
    }

    /// Let it finish waits while the tails ring, then for 300 ms of quiet,
    /// then fades in 50 ms and stays silent.
    #[test]
    fn let_it_finish_waits_for_quiet_then_fades() {
        let mut e = SongEnd::new(RATE);
        e.start(End::Finish);
        // Ringing at -20 dB for a second: no fade, the seconds counted.
        let (news, _, buf) = run(&mut e, 0.1, periods_for(1000));
        assert_eq!(buf, [0.1; N], "the tails are untouched");
        assert_eq!(news.first(), Some(&News::Finishing(10)));
        assert!(news.contains(&News::Finishing(9)));
        assert!(!news.contains(&News::Ended));
        // Quiet: 300 ms, then the 50 ms fade.
        let (news, k, _) = run(&mut e, 0.0005, periods_for(1000));
        assert_eq!(news.last(), Some(&News::Ended));
        let expect = periods_for(QUIET_MS) + periods_for(LAST_FADE_MS);
        assert!(k.abs_diff(expect) <= 2, "{k} vs {expect}");
        assert!(e.silent());
        // Silent until reset.
        let (mut l, mut r) = ([0.7; N], [0.7; N]);
        assert_eq!(e.period(0.7, &mut l, &mut r), None);
        assert_eq!(l, [0.0; N]);
        e.reset();
        let (mut l, mut r) = ([0.7; N], [0.7; N]);
        e.period(0.7, &mut l, &mut r);
        assert_eq!(l, [0.7; N]);
    }

    /// Between -60 and -54 dB the quiet time keeps counting; over -54 dB
    /// it starts again.
    #[test]
    fn a_rise_starts_the_quiet_time_again() {
        let mut e = SongEnd::new(RATE);
        e.start(End::Finish);
        run(&mut e, 0.0005, periods_for(200));
        run(&mut e, 0.0015, periods_for(50));
        let (news, _, _) = run(&mut e, 0.0005, periods_for(40));
        assert_eq!(news.last(), None, "still counting from the first quiet");
        let (news, _, _) = run(&mut e, 0.0005, periods_for(100));
        assert_eq!(
            news.last(),
            Some(&News::Ended),
            "300 ms in all, then the fade"
        );

        let mut e = SongEnd::new(RATE);
        e.start(End::Finish);
        run(&mut e, 0.0005, periods_for(250));
        run(&mut e, 0.01, 1);
        let (news, _, _) = run(&mut e, 0.0005, periods_for(250));
        assert!(!news.contains(&News::Ended), "the rise started it again");
    }

    /// A pad droning (or a mic) never holds it past 10 s; Go again goes
    /// now.
    #[test]
    fn let_it_finish_stops_waiting_at_ten_seconds_or_on_go_now() {
        let mut e = SongEnd::new(RATE);
        e.start(End::Finish);
        let (news, k, _) = run(&mut e, 0.3, periods_for(12_000));
        assert_eq!(news.last(), Some(&News::Ended));
        let expect = periods_for(LONGEST_MS) + periods_for(LAST_FADE_MS);
        assert!(k.abs_diff(expect) <= 2, "{k} vs {expect}");
        assert_eq!(
            news.iter()
                .filter(|n| matches!(n, News::Finishing(_)))
                .count(),
            10,
            "each second said once"
        );

        let mut e = SongEnd::new(RATE);
        e.start(End::Finish);
        run(&mut e, 0.3, periods_for(500));
        e.start(End::Finish);
        e.start(End::Cut);
        assert!(
            matches!(e.phase, Phase::Ringing { .. }),
            "a second start is ignored"
        );
        e.start(End::Now);
        let (news, k, _) = run(&mut e, 0.3, periods_for(1000));
        assert_eq!(news, [News::Ended]);
        assert!(k.abs_diff(periods_for(LAST_FADE_MS)) <= 1);
    }

    /// Fade falls evenly over its seconds; Cut in 10 ms. Neither waits for
    /// quiet.
    #[test]
    fn fade_and_cut_fall_over_their_time() {
        let mut e = SongEnd::new(RATE);
        e.start(End::Fade(2));
        let (_, _, half) = run(&mut e, 0.5, periods_for(1000));
        assert!((half[N - 1] - 0.25).abs() < 0.01, "{}", half[N - 1]);
        let (news, k, _) = run(&mut e, 0.5, periods_for(2000));
        assert_eq!(news, [News::Ended]);
        assert!(k.abs_diff(periods_for(1000)) <= 1);

        let mut e = SongEnd::new(RATE);
        e.start(End::Cut);
        let (news, k, _) = run(&mut e, 0.5, 10);
        assert_eq!(news, [News::Ended]);
        assert_eq!(k, periods_for(CUT_MS));
    }
}

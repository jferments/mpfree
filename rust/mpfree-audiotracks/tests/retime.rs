//! A song time edit on the audio tracks (the Arrange refactor's 3.4):
//! every track's regions move with their stretch of song as one undo step
//! of a song edit, which only the song's UNDO and REDO take back and bring
//! back; the tracks' own Undo stops at it, a recording refuses it, and a
//! full history drops a whole step.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_audiotracks::{
    AudioTracks, Command as A, Input, Piece, Retime, RetimeRefused, SongStep, HISTORY,
};
use mpfree_transport::TransactionId;
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;

fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    frame: u64,
    out: Vec<f32>,
    record: bool,
}

impl Rig {
    fn at(&mut self, frame: u64, c: T) {
        assert!(self.t.command(frame, c));
    }

    fn cmd(&mut self, c: A) {
        assert!(self.a.command(c));
    }

    fn run_to(&mut self, frame: u64) {
        const PERIOD: usize = 256;
        let mut inp = vec![0.0f32; 2 * PERIOD];
        let mut mix = vec![0.0f32; 2 * PERIOD];
        while self.frame < frame {
            let n = (PERIOD as u64).min(frame - self.frame) as usize;
            for j in 0..n {
                let g = self.frame + j as u64;
                let v = if self.record { sig(g) } else { 0.0 };
                (inp[2 * j], inp[2 * j + 1]) = (v, -v);
            }
            mix[..2 * n].fill(0.0);
            assert!(self.t.process(n as u32, &mut self.b));
            let map = *self.t.tempo_map();
            self.a
                .process(&self.b, &map, &inp[..2 * n], &mut mix[..2 * n]);
            self.out.extend_from_slice(&mix[..2 * n]);
            self.frame += n as u64;
        }
    }
}

/// Tracks 0 and 1 armed, bar 1 recorded onto both, then stopped and the
/// input silenced: one region a track.
fn two_tracks() -> Rig {
    let mut r = Rig {
        t: Transport::new(RATE, 240.0).unwrap(),
        a: AudioTracks::new(RATE, 10),
        b: Block::new(),
        frame: 0,
        out: Vec::new(),
        record: true,
    };
    for t in 0..2 {
        r.cmd(A::SetInput(t, Input::Stereo));
        r.cmd(A::Arm(t, true));
    }
    r.at(START, T::Record);
    r.at(START + BAR, T::Stop);
    r.run_to(START + BAR + 500);
    r.record = false;
    for t in 0..2 {
        r.cmd(A::Arm(t, false));
    }
    r.run_to(r.frame + 1);
    assert_eq!((r.a.regions(0).len(), r.a.regions(1).len()), (1, 1));
    r
}

/// A bar of room put in at tick `at` (a bar line after the first).
fn insert_bar(at: i64) -> Retime {
    let before = Piece {
        from: 0,
        end: at,
        to: 0,
    };
    let after = Piece {
        from: at,
        end: i64::MAX,
        to: at + BAR_TICKS,
    };
    let both = [before, after];
    Retime::new(if at > 0 { &both } else { &both[1..] }).unwrap()
}

#[test]
fn a_song_edit_moves_every_track_as_one_step_only_the_song_takes_back() {
    let mut r = two_tracks();
    let before: Vec<_> = (0..2).map(|t| r.a.regions(t).to_vec()).collect();
    r.cmd(A::Retime {
        song: TransactionId(7),
        retime: insert_bar(0),
    });
    r.run_to(r.frame + 1);
    for t in 0..2 {
        let q = r.a.regions(t)[0];
        assert_eq!((q.start, q.end, q.src), (BAR_TICKS, 2 * BAR_TICKS, 0));
    }
    // Played from the start: a bar of silence, then bar 1 as recorded.
    let p = r.frame + 10;
    r.at(p, T::PlayStart);
    r.run_to(p + 2 * BAR + 100);
    assert_eq!(r.out[2 * (p + BAR / 2) as usize], 0.0);
    for k in [100u64, 5_000, 20_000] {
        let g = (p + BAR + k) as usize;
        assert_eq!(r.out[2 * g], 2.0 * sig(START + k), "frame {k}");
    }
    r.at(r.frame, T::Stop);
    r.run_to(r.frame + 1);
    // The tracks' Undo stops at it; the song's takes both tracks back.
    assert_eq!(r.a.undo_is_song(), Some(TransactionId(7)));
    let refused = r.a.stats().refused;
    r.cmd(A::Undo);
    r.run_to(r.frame + 1);
    assert_eq!(r.a.stats().refused, refused + 1);
    assert_eq!(r.a.song_step(TransactionId(7), true), SongStep::Next);
    r.cmd(A::SongUndo(TransactionId(7)));
    r.run_to(r.frame + 1);
    assert_eq!(
        (0..2).map(|t| r.a.regions(t).to_vec()).collect::<Vec<_>>(),
        before
    );
    // Redo stops at it too; the song's REDO brings both back.
    assert_eq!(r.a.redo_is_song(), Some(TransactionId(7)));
    r.cmd(A::SongRedo(TransactionId(7)));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.regions(1)[0].start, BAR_TICKS);
    // An audio edit on top: the song's UNDO waits under it.
    r.cmd(A::Split {
        track: 0,
        at: BAR_TICKS + 100,
    });
    r.run_to(r.frame + 1);
    assert_eq!(r.a.song_step(TransactionId(7), true), SongStep::Under);
    let refused = r.a.stats().refused;
    r.cmd(A::SongUndo(TransactionId(7)));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.stats().refused, refused + 1, "under a split: refused");
    r.cmd(A::Undo);
    r.cmd(A::SongUndo(TransactionId(7)));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.regions(0), &before[0][..], "the split, then the move");
    r.cmd(A::ClearHistory);
    r.run_to(r.frame + 1);
    assert_eq!(r.a.song_step(TransactionId(7), false), SongStep::Gone);
    assert!(!r.a.has_song_steps());
}

#[test]
fn a_recording_refuses_a_song_edit_and_nothing_moves() {
    let mut r = two_tracks();
    r.cmd(A::Arm(1, true));
    r.record = true;
    let p = r.frame + 10;
    r.at(p, T::Record);
    r.run_to(p + 1000);
    assert_eq!(
        r.a.retime_refused(&insert_bar(0)),
        Some(RetimeRefused::Recording(1))
    );
    let regions = r.a.regions(0).to_vec();
    let refused = r.a.stats().refused;
    r.cmd(A::Retime {
        song: TransactionId(3),
        retime: insert_bar(0),
    });
    r.run_to(r.frame + 1);
    assert_eq!(r.a.regions(0), &regions[..]);
    assert_eq!(r.a.stats().refused, refused + 1);
    assert!(!r.a.has_song_steps());
}

#[test]
fn a_full_history_drops_a_song_edit_whole() {
    let mut r = two_tracks();
    let (recorded, _) = r.a.history();
    r.cmd(A::Retime {
        song: TransactionId(9),
        retime: insert_bar(0),
    });
    r.run_to(r.frame + 1);
    assert_eq!(r.a.history(), (recorded + 2, 0), "one step, a track each");
    // Gains on track 0 until the history is full and more: the
    // recordings go, then the song edit, whole, never one track of it.
    for k in 0..HISTORY - 1 {
        r.cmd(A::SetGain {
            track: 0,
            at: BAR_TICKS,
            gain: 0.5 + k as f32 * 0.01,
        });
        r.run_to(r.frame + 1);
    }
    assert_eq!(r.a.history(), (HISTORY - 1, 0));
    assert_eq!(r.a.song_step(TransactionId(9), true), SongStep::Gone);
    assert!(!r.a.has_song_steps());
    // Every step left is one gain: Undo takes them all back, then stops.
    for _ in 0..HISTORY - 1 {
        r.cmd(A::Undo);
    }
    r.run_to(r.frame + 1);
    assert_eq!(r.a.history(), (0, HISTORY - 1));
    assert_eq!(r.a.regions(1)[0].start, BAR_TICKS, "the move stays");
}

#[test]
fn a_song_edit_the_song_forgot_goes_with_every_step_beyond_it() {
    let mut r = two_tracks();
    let gain = |r: &mut Rig, track: usize, gain: f32| {
        let at = r.a.regions(track)[0].start + 10;
        r.cmd(A::SetGain { track, at, gain });
        r.run_to(r.frame + 1);
    };
    // A split under the song edit, a gain over it.
    r.cmd(A::Split {
        track: 0,
        at: 1_000,
    });
    r.cmd(A::Retime {
        song: TransactionId(4),
        retime: insert_bar(0),
    });
    r.run_to(r.frame + 1);
    gain(&mut r, 1, 0.5);
    let regions = r.a.regions(0).to_vec();
    // The song forgot it: it goes, and the split and the recordings under
    // it with it; the gain stays for Undo.
    r.cmd(A::ForgetSong(TransactionId(4)));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.history(), (1, 0));
    assert!(!r.a.has_song_steps());
    assert_eq!(r.a.regions(0), &regions[..], "no region moves");
    r.cmd(A::Undo);
    r.run_to(r.frame + 1);
    assert_eq!(r.a.history(), (0, 1));
    assert_eq!(r.a.regions(0), &regions[..], "Undo stops at the move");
    // On the Redo side: the first of its steps goes, and every step
    // redone after it.
    r.cmd(A::Redo);
    r.cmd(A::Retime {
        song: TransactionId(5),
        retime: insert_bar(BAR_TICKS),
    });
    r.run_to(r.frame + 1);
    gain(&mut r, 1, 0.7);
    r.cmd(A::Undo);
    r.cmd(A::SongUndo(TransactionId(5)));
    r.run_to(r.frame + 1);
    assert_eq!(
        r.a.history(),
        (1, 3),
        "the gain, then the move's two tracks"
    );
    r.cmd(A::ForgetSong(TransactionId(5)));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.history(), (1, 0), "the gain over it goes too");
    assert_eq!(r.a.song_step(TransactionId(5), false), SongStep::Gone);
    // A song edit the history doesn't hold: refused.
    let refused = r.a.stats().refused;
    r.cmd(A::ForgetSong(TransactionId(5)));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.stats().refused, refused + 1);
}

#[test]
fn the_tracks_are_busy_while_an_edit_waits_for_the_period() {
    let mut r = two_tracks();
    r.cmd(A::SetLevel(0, 0.5));
    r.cmd(A::Mute(1, true));
    assert!(!r.a.busy());
    r.cmd(A::Split {
        track: 0,
        at: 1_000,
    });
    assert!(r.a.busy());
    r.run_to(r.frame + 1);
    assert!(!r.a.busy());
}

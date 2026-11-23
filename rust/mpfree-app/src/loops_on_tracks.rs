//! Loops on audio tracks, the loop host's side (the audio tracks rethink,
//! phase 4, step 4b: docs/dev/design/loops-on-audio-tracks.md), behind
//! Dev's "Loops on audio tracks" until 4e.
//!
//! The loop host keeps a table from each slot (a loop number, 0 to 7) to
//! the lasting id of the loop it holds (`mpfree_seq::LoopId`), saved with
//! the loop (`LoopFile::id`). A loop gets its id when it first has sound
//! with the switch on, from the audio tracks' id maker. A slot whose loop
//! a loop cell (or a block of one) names is on that cell's track: each
//! period, before the looper runs, the track player's runs for its id go
//! to the looper ([`mpfree_looper::Looper::play`]), and after it runs its
//! own buffer goes to the track ([`OnTracks::out`]). A slot no loop cell
//! names launches itself, as on the Loop track.
//!
//! Which ids the project names is looked at every [`LOOK_EVERY`] periods
//! (a project's cells and blocks are walked, without allocating), as the
//! audio tracks look at the track list.

use mpfree_audiotracks::{LoopPlayer, MAX_TRACKS};
use mpfree_looper::{Looper, Phase, PlayRun, MAX_LOOPS, MAX_PLAY_RUNS};
use mpfree_seq::{LoopId, Project};

/// Periods between looks at which loops the project names (about 23 ms at
/// 128 frames a period).
pub const LOOK_EVERY: u32 = 8;

/// The slots' loops by id, and which are on audio tracks.
#[derive(Clone, Debug, Default)]
pub struct OnTracks {
    /// Each slot's loop's lasting id (0: none yet).
    ids: [u128; MAX_LOOPS],
    /// The slots whose loop the project names, as last looked.
    named: [bool; MAX_LOOPS],
    /// Periods since the last look.
    since: u32,
    /// Runs left out: a loop told more than [`MAX_PLAY_RUNS`] in a period.
    pub dropped: u64,
    /// Runs the looper refused (it shouldn't: counted to show it didn't).
    pub refused: u64,
}

impl OnTracks {
    /// Slot `n`'s loop's id, if it has one.
    pub fn id(&self, n: usize) -> Option<LoopId> {
        self.ids.get(n).filter(|&&i| i != 0).map(|&i| LoopId(i))
    }

    /// The slot holding loop `id`.
    pub fn slot_of(&self, id: LoopId) -> Option<usize> {
        let id = id.0;
        (id != 0).then(|| self.ids.iter().position(|&i| i == id))?
    }

    /// Slot `n`'s loop's id as saved or given (0: none).
    pub fn set(&mut self, n: usize, id: u128) {
        if let Some(i) = self.ids.get_mut(n) {
            *i = id;
        }
    }

    /// Every slot forgets its loop (an open or New project).
    pub fn forget(&mut self) {
        *self = OnTracks {
            dropped: self.dropped,
            refused: self.refused,
            ..OnTracks::default()
        };
    }

    /// The first slot with sound (armed, recording or playing) and no id
    /// yet: it is given one ([`OnTracks::set`]) before it is told runs.
    pub fn needs_id(&self, looper: &Looper) -> Option<usize> {
        (0..MAX_LOOPS).find(|&n| {
            self.ids[n] == 0
                && looper
                    .loop_status(n)
                    .is_some_and(|s| s.phase != Phase::Empty)
        })
    }

    /// Before the looper's period: each slot whose loop the project names
    /// is told where its runs are ([`LoopPlayer::runs`] of every track, by
    /// its id), silent where there are none; a slot named no more leaves
    /// its track. Does not allocate.
    pub fn tell(&mut self, looper: &mut Looper, project: &Project, loops: &LoopPlayer) {
        if self.since == 0 {
            for n in 0..MAX_LOOPS {
                let id = self.ids[n];
                self.named[n] = id != 0 && project.loop_track(LoopId(id)).is_some();
            }
        }
        self.since = (self.since + 1) % LOOK_EVERY;
        for n in 0..MAX_LOOPS {
            if !self.named[n] {
                if looper.on_track(n) {
                    looper.leave_track(n);
                }
                continue;
            }
            let id = self.ids[n];
            let mut runs = [PlayRun::default(); MAX_PLAY_RUNS];
            let mut k = 0;
            for t in 0..MAX_TRACKS {
                for r in loops.runs(t).iter().filter(|r| r.id == id) {
                    if k == MAX_PLAY_RUNS {
                        self.dropped += 1;
                        continue;
                    }
                    runs[k] = PlayRun {
                        from: r.from,
                        to: r.to,
                        origin: r.origin,
                    };
                    k += 1;
                }
            }
            if !looper.play(n, &runs[..k]) {
                self.refused += 1;
            }
        }
    }

    /// After the looper's period: loop `id`'s sound in it, for its track
    /// (None: no slot holds it, or it isn't on a track).
    pub fn out<'a>(&self, looper: &'a Looper, id: u128) -> Option<&'a [f32]> {
        let n = self.slot_of(LoopId(id))?;
        let outs = looper.outs();
        outs.on_track(n).then(|| outs.loop_out(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Project, Track, TrackId};

    #[test]
    fn a_slot_is_found_by_its_loops_id_and_forgets_it_for_a_new_project() {
        let mut o = OnTracks::default();
        assert_eq!(o.id(3), None);
        o.set(3, 0xabc);
        assert_eq!(o.id(3), Some(LoopId(0xabc)));
        assert_eq!(o.slot_of(LoopId(0xabc)), Some(3));
        assert_eq!(o.slot_of(LoopId(0)), None, "no id is no slot");
        assert_eq!(o.slot_of(LoopId(7)), None);
        o.dropped = 2;
        o.forget();
        assert_eq!(o.id(3), None);
        assert_eq!(o.dropped, 2, "the counts are kept");
    }

    #[test]
    fn only_a_loop_the_project_names_goes_on_a_track() {
        let mut looper = Looper::new(44_100, 4);
        let loops = LoopPlayer::new();
        let mut p = Project::default();
        let mut o = OnTracks::default();
        o.set(0, 5);
        o.set(1, 6);
        o.tell(&mut looper, &p, &loops);
        assert!(!looper.on_track(0) && !looper.on_track(1));
        // A loop cell on an audio track names loop 6: slot 1 goes on it
        // at the next look (told no runs, so silent).
        let mut a = Track::new_audio("Audio 1", 0);
        a.id = TrackId(p.next_track.max(1));
        p.next_track = a.id.0 + 1;
        p.tracks.push(a);
        let at = p.tracks.len() - 1;
        p.tracks[at].slots[0] = Some(Clip::new_audio_loop(LoopId(6), "Loop", 3_840));
        p.give_clip_ids();
        for _ in 0..LOOK_EVERY {
            o.tell(&mut looper, &p, &loops);
        }
        assert!(looper.on_track(1) && !looper.on_track(0));
        assert_eq!(o.refused, 0);
        // Named no more: it leaves the track at the next look.
        p.tracks[at].slots[0] = None;
        for _ in 0..LOOK_EVERY {
            o.tell(&mut looper, &p, &loops);
        }
        assert!(!looper.on_track(1));
    }

    /// The whole chain as the audio thread runs it with Dev's switch on
    /// (main.rs): a loop recorded on the looper gets an id, a loop cell on
    /// an audio track names it, and while the arranger says the cell plays
    /// the loop sounds in that track's channel and not on the Loop track.
    /// With the cell gone it plays on the Loop track again.
    #[test]
    fn a_loop_cell_plays_its_loop_in_its_track_and_not_on_the_loop_track() {
        use crate::audiotracks::TrackHost;
        use crate::looping::LoopHost;
        use crate::looping::PERIOD;
        use mpfree_looper::{Command, Phase};
        use mpfree_play::{AudioAnswer, AudioPlays};
        use mpfree_transport::time::PPQ;
        use mpfree_transport::{Block, Command as T, Transport};

        const RATE: u32 = 44_100;
        let mut h = LoopHost::new(RATE, 20);
        let mut t = Transport::new(RATE, 120.0).unwrap();
        let mut b = Block::new();
        let mut tracks = TrackHost::new(RATE, 10);
        let mut p = Project::new();
        let mut a = Track::new_audio("Audio 1", 0);
        p.give_track_id(&mut a);
        let audio_id = a.id;
        p.tracks.push(a);
        tracks.sync(&p, 0, true);
        let slot = tracks.slots.slot_of(audio_id).expect("an engine track");

        // Loop 1: one bar of a 441 Hz tone from the pads, then playing.
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut f = 0u64;
        while h.looper.status().phase != Phase::Playing {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for i in 0..PERIOD {
                let g = (f + i as u64) as f64;
                l[i] = 0.3 * (g * 441.0 * std::f64::consts::TAU / RATE as f64).sin() as f32;
                r[i] = -l[i];
            }
            h.take_pads(&l, &r);
            h.run(&b, &mut [0.0; PERIOD], &mut [0.0; PERIOD]);
            f += PERIOD as u64;
        }
        h.give_loop_id(|| tracks.fresh_loop_id());
        let id = h.on_tracks.id(0).expect("a loop with sound gets an id");
        assert_ne!(id.0, 0);

        // Its loop cell on the audio track, playing from the loop's own
        // origin, as the arranger would answer once it's launched.
        p.tracks[0].slots[0] = Some(Clip::new_audio_loop(id, "Loop 1", i64::from(4 * PPQ)));
        p.give_clip_ids();
        let c = p.tracks[0].slots[0].as_ref().unwrap();
        let mut playing = AudioAnswer::default();
        playing.start = AudioPlays::Loop {
            id,
            origin: h.looper.loop_status(0).unwrap().origin as f64,
            len: c.length,
            clip: c.id,
            stamp: 1,
        };

        // `n` periods as main.rs runs them: the peaks of the Loop track's
        // output and the audio track's channel over the last 20.
        let mut run = |h: &mut LoopHost, tracks: &mut TrackHost, p: &Project, n: usize| {
            let ans = if p.loop_track(id).is_some() {
                playing
            } else {
                AudioAnswer::default()
            };
            let (mut on_loop, mut on_track) = (0.0f32, 0.0f32);
            for k in 0..n {
                assert!(t.process(PERIOD as u32, &mut b));
                h.take_pads(&[0.0; PERIOD], &[0.0; PERIOD]);
                tracks.sync(p, 0, false);
                tracks.plan_loops(&b, &|_| ans);
                h.give_loop_id(|| tracks.fresh_loop_id());
                h.tell_tracks(p, tracks.loop_player());
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.run(&b, &mut l, &mut r);
                let map = *t.tempo_map();
                let src = |i: u128| h.track_loop(i);
                tracks.period_with_loops(&b, &map, true, h.input(), &|_| ans, &src);
                if k + 20 >= n {
                    on_loop = l.iter().chain(&r).fold(on_loop, |m, v| m.max(v.abs()));
                    let out = tracks.host.tracks.track_out(slot, PERIOD);
                    on_track = out.iter().fold(on_track, |m, v| m.max(v.abs()));
                }
            }
            (on_loop, on_track)
        };

        let periods = 2 * LOOK_EVERY as usize + 400;
        let (on_loop, on_track) = run(&mut h, &mut tracks, &p, periods);
        assert!(h.looper.on_track(0));
        assert!(on_loop < 1e-6, "the Loop track is silent: {on_loop}");
        assert!(on_track > 0.1, "the audio track plays the loop: {on_track}");
        assert_eq!((h.on_tracks.dropped, h.on_tracks.refused), (0, 0));

        p.tracks[0].slots[0] = None;
        let (on_loop, on_track) = run(&mut h, &mut tracks, &p, periods);
        assert!(!h.looper.on_track(0));
        assert!(on_loop > 0.1, "back on the Loop track: {on_loop}");
        assert!(on_track < 1e-6, "the audio track is silent: {on_track}");
    }
}

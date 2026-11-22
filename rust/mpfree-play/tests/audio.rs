//! What each audio track plays, for the audio engine (the audio tracks
//! rethink's phase 2, step 2b): [`Player::audio`] says what played at a
//! period's start and each change on its frame. A launch, a stop, a legato
//! switch, a scene's mute and the mute queue each land on the frame a note
//! on their tick does; a once clip's own end rings, a stop before it cuts;
//! song mode plays the song where no clip is; an edit to a playing clip
//! shows in its stamp. The clips' regions are lent from the view.
//!
//! Every session runs at several period sizes, with the control thread
//! applying each started cue and landed queue at once, a few periods later,
//! much later or never. All of them must tell the same story.

use mpfree_play::{
    AudioPlays, Cue, Event, Launch, MuteCue, Out, PlayView, Player, Sink, MAX_AUDIO_CHANGES,
};
use mpfree_seq::{
    AudioClip, Clip, ClipId, ClipRegion, LaunchGrid, Note, Project, RecId, Speed, Track,
};
use mpfree_transport::{Block, Command, Transport};

const BAR: i64 = 3840;
/// 120 BPM at 48 kHz: 25 frames a tick.
const RATE: u32 = 48_000;
const TICK: u64 = 25;
const VOCAL: usize = 1;

fn at(tick: i64) -> u64 {
    tick as u64 * TICK
}

#[derive(Default)]
struct Log(Vec<(u64, u16, Event)>);
impl Sink for Log {
    fn send(&mut self, o: Out) {
        self.0.push((o.frame, o.track, o.event));
    }
}

#[derive(Clone)]
struct Session {
    project: Project,
    launches: Vec<Launch>,
    song: bool,
    commands: Vec<(u64, Command)>,
    /// Cues the control thread makes at their frame: the cue, a scene's
    /// mute, and its line (None: the clip's own or the project's).
    cues: Vec<(u64, Cue, Option<bool>, Option<LaunchGrid>)>,
    /// Mute queues the control thread makes at their frame.
    queues: Vec<(u64, Vec<(usize, bool)>)>,
    frames: u64,
}

/// What an audio track plays, as the run compares it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Plays {
    Nothing,
    Song,
    Clip {
        clip: ClipId,
        origin: f64,
        len: i64,
        stamp: u64,
        regions: usize,
        gain: f32,
        last_frame: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Change {
    frame: u64,
    plays: Plays,
    muted: bool,
    ring: bool,
}

#[derive(Debug, PartialEq)]
struct Run {
    /// The vocal's story: what it played from frame 0, then each change.
    changes: Vec<Change>,
    /// The note-ons on the other tracks.
    ons: Vec<(u64, u16)>,
    superseded: u64,
}

/// `p` as the run compares it, after checking its regions are the
/// project's clip's.
fn plays(p: &AudioPlays, project: &Project) -> Plays {
    match *p {
        AudioPlays::Nothing => Plays::Nothing,
        AudioPlays::Song => Plays::Song,
        AudioPlays::Clip {
            regions,
            origin,
            len,
            last_frame,
            gain,
            clip,
            stamp,
        } => {
            let c = project
                .cell_clips()
                .map(|(_, _, c)| c)
                .find(|c| c.id == clip)
                .expect("a clip of the project");
            let a = c.audio.as_ref().expect("an audio clip");
            assert_eq!(regions, &a.regions[..]);
            assert_eq!((len, gain, last_frame), (c.length, a.gain, a.last_frame));
            Plays::Clip {
                clip,
                origin,
                len,
                stamp,
                regions: regions.len(),
                gain,
                last_frame,
            }
        }
    }
}

/// Run `s` in periods of `period` frames. The control thread applies a
/// started cue or a landed queue `apply` periods after the one it
/// happened in (None: never).
fn run(s: &Session, period: u32, apply: Option<usize>) -> Run {
    let mut t = Transport::new(RATE, 120.0).unwrap();
    for &(f, c) in &s.commands {
        assert!(t.command(f, c));
    }
    let mut project = s.project.clone();
    let mut launches = s.launches.clone();
    let mut cues: Vec<(Cue, Option<bool>, Option<LaunchGrid>)> = vec![];
    let mut queue: Option<(u32, Vec<(usize, bool)>)> = None;
    let mut ids = 0;
    let compile = |p: &Project,
                   l: &[Launch],
                   c: &[(Cue, Option<bool>, Option<LaunchGrid>)],
                   q: &Option<(u32, Vec<(usize, bool)>)>| {
        let plain: Vec<Cue> = c.iter().map(|x| x.0).collect();
        let mut v = PlayView::compile_cued(p, l, &plain).unwrap();
        v.set_song(p, s.song).unwrap();
        for &(cue, mute, line) in c {
            v.set_cue_mute(cue.track, mute);
            if let Some(line) = line {
                v.set_cue_line(cue.track, line);
            }
        }
        if let Some((id, changes)) = q {
            for &(track, silent) in changes {
                v.set_mute_cue(
                    track,
                    MuteCue {
                        silent,
                        line: LaunchGrid::Bars(1),
                        id: *id,
                    },
                );
            }
        }
        v
    };
    let mut view = compile(&project, &launches, &cues, &queue);
    let (mut player, mut block, mut log) = (Player::new(), Block::new(), Log::default());
    let mut changes: Vec<Change> = vec![];
    let mut waiting: Vec<(usize, Option<u32>, Option<mpfree_play::Started>)> = vec![];
    let (mut next_cue, mut next_queue) = (0, 0);
    while t.frame() < s.frames {
        let f = t.frame();
        let n = period.min((s.frames - f) as u32);
        let mut changed = false;
        for w in &mut waiting {
            if w.0 == 0 {
                if let Some(id) = w.1 {
                    if queue.as_ref().is_some_and(|q| q.0 == id) {
                        for (track, silent) in queue.take().unwrap().1 {
                            project.tracks[track].mute = silent;
                        }
                    }
                }
                if let Some(st) = w.2 {
                    if let Some(m) = st.mute {
                        project.tracks[st.track].mute = m;
                    }
                    let mut plain: Vec<Cue> = cues.iter().map(|x| x.0).collect();
                    st.apply(&mut launches, &mut plain);
                    cues.retain(|x| plain.contains(&x.0));
                }
                changed = true;
            }
            w.0 = w.0.wrapping_sub(1);
        }
        waiting.retain(|w| w.0 != usize::MAX);
        while next_cue < s.cues.len() && s.cues[next_cue].0 <= f {
            let (_, c, m, line) = s.cues[next_cue];
            // The control thread applies what started on the track before
            // it cues the track again, as the control half does.
            for w in waiting
                .iter_mut()
                .filter(|w| w.2.is_some_and(|st| st.track == c.track))
            {
                let st = w.2.take().unwrap();
                if let Some(m) = st.mute {
                    project.tracks[st.track].mute = m;
                }
                let mut plain: Vec<Cue> = cues.iter().map(|x| x.0).collect();
                st.apply(&mut launches, &mut plain);
                cues.retain(|x| plain.contains(&x.0));
            }
            cues.retain(|x| x.0.track != c.track);
            cues.push((c, m, line));
            next_cue += 1;
            changed = true;
        }
        while next_queue < s.queues.len() && s.queues[next_queue].0 <= f {
            ids += 1;
            queue = Some((ids, s.queues[next_queue].1.clone()));
            next_queue += 1;
            changed = true;
        }
        if changed {
            view = compile(&project, &launches, &cues, &queue);
        }
        assert!(t.process(n, &mut block));
        player.process(&view, &block, &mut log);
        let a = player.audio(&view, VOCAL);
        let start = Change {
            frame: f,
            plays: plays(&a.start, &project),
            muted: a.muted,
            ring: false,
        };
        match changes.last() {
            None => changes.push(start),
            // A period starts as the last one ended.
            Some(last) => assert_eq!(
                (last.plays, last.muted),
                (start.plays, start.muted),
                "period at {f}"
            ),
        }
        assert!(a.changes().len() <= MAX_AUDIO_CHANGES);
        for c in a.changes() {
            // On the frame a note on its tick lands on.
            assert_eq!(f + u64::from(c.frame), at(c.tick as i64));
            changes.push(Change {
                frame: f + u64::from(c.frame),
                plays: plays(&c.plays, &project),
                muted: c.muted,
                ring: c.ring,
            });
        }
        let mut landed: Vec<u32> = vec![];
        for l in player.mutes_landed() {
            if !landed.contains(&l.id) {
                landed.push(l.id);
            }
        }
        for st in player.started() {
            // Never applied on its own (None), it still is before the
            // track is cued again.
            waiting.push((apply.unwrap_or(usize::MAX - 1), None, Some(*st)));
        }
        if let Some(d) = apply {
            for id in landed {
                waiting.push((d, Some(id), None));
            }
        }
    }
    // Any track that isn't an audio track plays nothing for the engine.
    assert_eq!(player.audio(&view, 0).start, AudioPlays::Nothing);
    assert_eq!(player.audio(&view, 300).start, AudioPlays::Nothing);
    let ons = log
        .0
        .iter()
        .filter(|e| matches!(e.2, Event::NoteOn { .. }))
        .map(|e| (e.0, e.1))
        .collect();
    Run {
        changes,
        ons,
        superseded: player.stats().audio_superseded,
    }
}

/// The run at every period size and every way of applying, which must
/// all agree.
fn same_everywhere(s: &Session) -> Run {
    let first = run(s, 128, Some(0));
    for period in [1, 64, 1000, 4096] {
        for apply in [Some(0), Some(3), Some(40), None] {
            let r = run(s, period, apply);
            assert_eq!(r, first, "period {period}, applied {apply:?}");
        }
    }
    first
}

fn note(tick: i64) -> Note {
    Note {
        tick,
        len: 10,
        note: 36,
        velocity: 100,
        chance: 100,
        cond: mpfree_seq::Cond::Always,
        ratchet: 1,
        touch: mpfree_seq::Touch::NONE,
        on: true,
        ramp: 0,
        join: mpfree_seq::Join::Attack,
    }
}

/// An audio clip `bars` long of two regions, named `id`.
fn audio_clip(name: &str, id: u32, bars: i64) -> Clip {
    let mut c = Clip::new(name, bars * BAR);
    c.id = ClipId(id);
    let mut a = AudioClip::of(RecId(u128::from(id)), 0, 96_000, BAR / 4, 120.0);
    a.regions
        .push(ClipRegion::new(BAR / 2, bars * BAR, RecId(9), 48_000));
    a.last_frame = 200_000;
    c.audio = Some(a);
    c
}

/// Drums (a note clip on every beat) and a vocal (an audio track) with
/// audio clips A (2 bars, id 1), B (1 bar, id 2) and a once clip C (1 bar,
/// id 3), and the drums playing.
fn session(bars: i64) -> Session {
    let mut p = Project::new();
    let mut drums = Track::new("drums", 1);
    let mut beat = Clip::new("beat", BAR);
    beat.notes = (0..4).map(|k| note(k * BAR / 4)).collect();
    drums.slots[0] = Some(beat);
    let mut vocal = Track::new_audio("vocal", 2);
    vocal.slots[0] = Some(audio_clip("A", 1, 2));
    vocal.slots[1] = Some(audio_clip("B", 2, 1));
    let mut once = audio_clip("C", 3, 1);
    once.once = true;
    vocal.slots[2] = Some(once);
    p.tracks.push(drums);
    p.tracks.push(vocal);
    p.check().unwrap();
    Session {
        project: p,
        launches: vec![Launch {
            track: 0,
            slot: 0,
            origin: 0,
            start: 0,
        }],
        song: false,
        commands: vec![(0, Command::Play)],
        cues: vec![],
        queues: vec![],
        frames: at(bars * BAR),
    }
}

fn launch(slot: usize) -> Cue {
    Cue {
        track: VOCAL,
        slot: Some(slot),
    }
}

const STOP: Cue = Cue {
    track: VOCAL,
    slot: None,
};

fn clip(r: &Run, k: usize) -> (ClipId, f64) {
    match r.changes[k].plays {
        Plays::Clip { clip, origin, .. } => (clip, origin),
        p => panic!("change {k} plays {p:?}"),
    }
}

#[test]
fn an_audio_clip_starts_on_the_bar_line_on_the_frame_the_drums_note_does() {
    let mut s = session(3);
    s.cues = vec![(at(BAR / 3), launch(0), None, None)];
    let r = same_everywhere(&s);
    assert_eq!(r.changes.len(), 2, "{:?}", r.changes);
    assert_eq!(r.changes[0].plays, Plays::Nothing);
    let c = r.changes[1];
    assert_eq!((c.frame, c.muted, c.ring), (at(BAR), false, false));
    assert!(r.ons.contains(&(at(BAR), 0)), "the drums' note on the line");
    // Its passes count from the line: pass 0 is a whole number of its
    // 2-bar passes before it.
    let (id, origin) = clip(&r, 1);
    assert_eq!(id, ClipId(1));
    assert_eq!((BAR as f64 - origin).rem_euclid(2.0 * BAR as f64), 0.0);
    let Plays::Clip { regions, len, .. } = c.plays else {
        unreachable!()
    };
    assert_eq!((regions, len), (2, 2 * BAR));
    assert_eq!(r.superseded, 0);
}

#[test]
fn a_once_clip_rings_at_its_own_end_and_a_stop_or_a_launch_before_it_cuts() {
    // C launched on bar 2; its end, bar 3, is cued in the bar before it,
    // as Perform does (`once_end`).
    let mut s = session(4);
    s.cues = vec![
        (at(BAR / 2), launch(2), None, None),
        (at(BAR + BAR / 2), STOP, None, Some(LaunchGrid::At(2 * BAR))),
    ];
    let r = same_everywhere(&s);
    assert_eq!(r.changes.len(), 3, "{:?}", r.changes);
    assert_eq!(clip(&r, 1).0, ClipId(3));
    assert_eq!(
        r.changes[2],
        Change {
            frame: at(2 * BAR),
            plays: Plays::Nothing,
            muted: false,
            ring: true
        }
    );

    // A stop on a bar line that isn't the end of A's pass: cut.
    let mut s = session(4);
    s.cues = vec![
        (at(BAR / 2), launch(0), None, None),
        (at(BAR + BAR / 2), STOP, None, None),
    ];
    let r = same_everywhere(&s);
    assert_eq!(r.changes[2].frame, at(2 * BAR));
    assert!(!r.changes[2].ring);

    // A once clip stopped before its end, and one followed into another
    // clip at its end: neither rings.
    let mut s = session(4);
    s.project.tracks[VOCAL].slots[2].as_mut().unwrap().length = 2 * BAR;
    s.project.tracks[VOCAL].slots[2]
        .as_mut()
        .unwrap()
        .audio
        .as_mut()
        .unwrap()
        .regions[1]
        .end = 2 * BAR;
    s.cues = vec![
        (at(BAR / 2), launch(2), None, None),
        (at(BAR + BAR / 2), STOP, None, None),
    ];
    let r = same_everywhere(&s);
    assert_eq!(
        (r.changes[2].frame, r.changes[2].ring),
        (at(2 * BAR), false)
    );
    let mut s = session(4);
    s.cues = vec![
        (at(BAR / 2), launch(2), None, None),
        (at(BAR + BAR / 2), launch(1), None, None),
    ];
    let r = same_everywhere(&s);
    assert_eq!(clip(&r, 2).0, ClipId(2));
    assert_eq!(
        (r.changes[2].frame, r.changes[2].ring),
        (at(2 * BAR), false)
    );
}

#[test]
fn a_legato_launch_keeps_the_place_in_the_pass() {
    // A plays from bar 1; B, legato, takes over on bar 2 a bar into its
    // pass, counting its passes from bar 1 as A did.
    let mut s = session(3);
    s.launches.push(Launch {
        track: VOCAL,
        slot: 0,
        origin: 0,
        start: 0,
    });
    let b = s.project.tracks[VOCAL].slots[1].as_mut().unwrap();
    b.legato = true;
    b.length = 2 * BAR;
    b.audio.as_mut().unwrap().regions[1].end = 2 * BAR;
    s.cues = vec![(at(BAR / 2), launch(1), None, None)];
    let r = same_everywhere(&s);
    assert_eq!(r.changes.len(), 2, "{:?}", r.changes);
    assert_eq!(clip(&r, 0), (ClipId(1), 0.0));
    let (id, origin) = clip(&r, 1);
    assert_eq!(id, ClipId(2));
    assert_eq!(r.changes[1].frame, at(BAR));
    // A bar into its pass on the line, not at its start.
    assert_eq!(
        (BAR as f64 - origin).rem_euclid(2.0 * BAR as f64),
        BAR as f64
    );
}

#[test]
fn a_scenes_mute_and_the_mute_queue_ride_along_on_their_lines() {
    // A plays from bar 1; a scene launches B with the vocal muted on bar
    // 2; the mute queue brings it back on bar 3.
    let mut s = session(4);
    s.launches.push(Launch {
        track: VOCAL,
        slot: 0,
        origin: 0,
        start: 0,
    });
    s.cues = vec![(at(BAR / 2), launch(1), Some(true), None)];
    s.queues = vec![(at(BAR + BAR / 2), vec![(VOCAL, false)])];
    let r = same_everywhere(&s);
    assert_eq!(r.changes.len(), 3, "{:?}", r.changes);
    assert_eq!((r.changes[1].frame, r.changes[1].muted), (at(BAR), true));
    assert_eq!(clip(&r, 1).0, ClipId(2));
    assert_eq!(
        (r.changes[2].frame, r.changes[2].muted),
        (at(2 * BAR), false)
    );
    assert_eq!(r.changes[2].plays, r.changes[1].plays);
}

#[test]
fn song_mode_plays_the_song_where_no_clip_is() {
    let mut s = session(3);
    s.song = true;
    s.launches.push(Launch {
        track: VOCAL,
        slot: 1,
        origin: 0,
        start: 0,
    });
    s.cues = vec![(at(BAR / 2), STOP, None, None)];
    let r = same_everywhere(&s);
    assert_eq!(r.changes.len(), 2, "{:?}", r.changes);
    assert_eq!(clip(&r, 0).0, ClipId(2));
    assert_eq!(
        r.changes[1],
        Change {
            frame: at(BAR),
            plays: Plays::Song,
            muted: false,
            ring: false
        }
    );
}

#[test]
fn an_edit_to_a_playing_clip_changes_its_stamp_and_not_its_place() {
    let mut s = session(3);
    s.launches.push(Launch {
        track: VOCAL,
        slot: 0,
        origin: 0,
        start: 0,
    });
    // The edit is no change in a period: the next period starts with the
    // new stamp, and the engine takes it from there.
    let (mut before, mut after) = (None, None);
    let mut t = Transport::new(RATE, 120.0).unwrap();
    assert!(t.command(0, Command::Play));
    let mut p = s.project.clone();
    let mut block = Block::new();
    let mut player = Player::new();
    for k in 0..2 {
        if k == 1 {
            let a = p.tracks[VOCAL].slots[0].as_mut().unwrap();
            a.audio.as_mut().unwrap().gain = 0.5;
        }
        let v = PlayView::compile(&p, &s.launches).unwrap();
        assert!(t.process(256, &mut block));
        player.process(&v, &block, &mut Log::default());
        let a = player.audio(&v, VOCAL);
        assert!(a.changes().is_empty());
        let AudioPlays::Clip {
            origin,
            gain,
            stamp,
            ..
        } = a.start
        else {
            panic!("{:?}", a.start)
        };
        *[&mut before, &mut after][k] = Some((origin, gain, stamp));
    }
    let (b, a) = (before.unwrap(), after.unwrap());
    assert_eq!((b.0, b.1), (0.0, 1.0));
    assert_eq!((a.0, a.1), (0.0, 0.5));
    assert_ne!(a.2, b.2);
    // A speed or a length the check refuses never reaches the player.
    let mut bad = s.project.clone();
    bad.tracks[VOCAL].slots[0].as_mut().unwrap().speed = Speed::new(2, 1).unwrap();
    assert!(PlayView::compile(&bad, &s.launches).is_err());
}

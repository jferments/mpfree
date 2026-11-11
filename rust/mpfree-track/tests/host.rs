//! The track host with real instruments (Poly and FM) and the real mixer.

use mpfree_fx::{Mixer, MixerConfig};
use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed};
use mpfree_play::{Event, Out, Sink, ALL};
use mpfree_synth::{Fm, Poly};
use mpfree_track::{
    Instrument, Outputs, Sum, SwapError, TrackHost, MAX_EVENTS, QUIET_FRAMES, STOP_FADE,
};

const RATE: u32 = 44_100;

fn poly() -> Instrument {
    Instrument::new(Box::new(Poly::new(RATE)), RATE, 1024)
}

fn fm() -> Instrument {
    Instrument::new(Box::new(Fm::new(RATE)), RATE, 1024)
}

/// Each track into its own pair, to see where the sound went.
struct Split {
    tracks: Vec<(Vec<f32>, Vec<f32>)>,
}

impl Split {
    fn new(tracks: usize, block: usize) -> Split {
        Split {
            tracks: vec![(vec![0.0; block], vec![0.0; block]); tracks],
        }
    }
    fn clear(&mut self) {
        for (l, r) in &mut self.tracks {
            l.fill(0.0);
            r.fill(0.0);
        }
    }
    fn peak(&self, t: usize) -> f32 {
        let (l, r) = &self.tracks[t];
        l.iter().chain(r).fold(0f32, |m, x| m.max(x.abs()))
    }
}

impl Outputs for Split {
    fn track(&mut self, track: usize) -> Option<(&mut [f32], &mut [f32])> {
        self.tracks
            .get_mut(track)
            .map(|(l, r)| (l.as_mut_slice(), r.as_mut_slice()))
    }
}

fn ctx(frame: u64) -> Ctx<'static> {
    Ctx::new(RATE, frame)
}

/// Runs `periods` blocks of `block` frames into a Split; returns each
/// block's peak per track.
fn run(
    host: &mut TrackHost,
    out: &mut Split,
    frame: &mut u64,
    periods: usize,
    block: usize,
) -> Vec<Vec<f32>> {
    let mut peaks = Vec::new();
    for _ in 0..periods {
        out.clear();
        host.render(&ctx(*frame), block, out);
        *frame += block as u64;
        peaks.push((0..out.tracks.len()).map(|t| out.peak(t)).collect());
    }
    peaks
}

#[test]
fn a_slot_plays_into_its_own_track_and_routes_move_it() {
    let mut host = TrackHost::new(3, RATE, 1024);
    host.swap(0, Some(poly())).unwrap();
    host.swap(2, Some(fm())).unwrap();
    let mut out = Split::new(3, 128);
    let mut frame = 0;
    host.note_on(0, 0, 60, 100);
    let p = run(&mut host, &mut out, &mut frame, 20, 128);
    assert!(p[19][0] > 0.01, "Poly on track 0: {:?}", p[19]);
    assert_eq!((p[19][1], p[19][2]), (0.0, 0.0), "nothing elsewhere");
    host.note_on(2, 0, 64, 100);
    let p = run(&mut host, &mut out, &mut frame, 20, 128);
    assert!(p[19][2] > 0.01, "FM on track 2");
    // Slot 2 to track 1.
    host.route(2, Some(1));
    assert_eq!(
        (
            host.track_of(2),
            host.slot_of_track(1),
            host.slot_of_track(2)
        ),
        (Some(1), Some(2), None)
    );
    let p = run(&mut host, &mut out, &mut frame, 2, 128);
    assert!(p[1][1] > 0.01 && p[1][2] == 0.0, "{:?}", p[1]);
    // Nowhere: silent, still running.
    host.route(2, None);
    let p = run(&mut host, &mut out, &mut frame, 2, 128);
    assert_eq!((p[1][1], p[1][2]), (0.0, 0.0));
    assert_eq!(host.stats(0).notes, 1);
}

#[test]
fn a_note_two_sources_hold_ends_with_the_last_of_them() {
    let mut host = TrackHost::new(1, RATE, 1024);
    host.swap(0, Some(poly())).unwrap();
    let mut out = Split::new(1, 128);
    let mut frame = 0;
    // A clip and a pad both play E3.
    host.send(Out {
        offset: 0,
        frame: 0,
        track: 0,
        dest: 0,
        event: Event::NoteOn {
            note: 52,
            velocity: 100,
        },
    });
    host.note_on(0, 10, 52, 90);
    run(&mut host, &mut out, &mut frame, 10, 128);
    // The clip's note-off: the pad still holds it.
    host.send(Out {
        offset: 0,
        frame,
        track: 0,
        dest: 0,
        event: Event::NoteOff { note: 52 },
    });
    let p = run(&mut host, &mut out, &mut frame, 200, 128);
    assert!(p[199][0] > 0.01, "still sounding: {}", p[199][0]);
    // The pad lets go: the release, then silence.
    host.note_off(0, 0, 52);
    let p = run(&mut host, &mut out, &mut frame, 600, 128);
    assert_eq!(p[599][0], 0.0);
    // A note-off nobody holds is only counted.
    host.note_off(0, 0, 52);
    let s = host.stats(0);
    assert_eq!((s.notes, s.stray_offs, s.dropped, s.resets), (2, 1, 0, 0));
    // AllOff to every destination.
    host.note_on(0, 0, 60, 100);
    host.send(Out {
        offset: 0,
        frame,
        track: ALL,
        dest: ALL,
        event: Event::AllOff,
    });
    let p = run(&mut host, &mut out, &mut frame, 600, 128);
    assert_eq!(p[599][0], 0.0);
}

#[test]
fn the_same_events_sound_the_same_in_any_block_size() {
    // (frame, slot, event): notes, a shared note, a parameter change.
    let script: Vec<(u64, usize, NodeEvent)> = vec![
        (
            3,
            0,
            NodeEvent::NoteOn {
                note: 48,
                velocity: 100,
            },
        ),
        (
            70,
            1,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 90,
            },
        ),
        (
            500,
            0,
            NodeEvent::NoteOn {
                note: 55,
                velocity: 80,
            },
        ),
        (
            900,
            0,
            NodeEvent::Param {
                id: mpfree_synth::params::CUTOFF,
                value: 600.0,
            },
        ),
        (
            1200,
            1,
            NodeEvent::Param {
                id: mpfree_synth::fm_params::FEEDBACK,
                value: 7.0,
            },
        ),
        (2000, 0, NodeEvent::NoteOff { note: 48 }),
        (2100, 1, NodeEvent::NoteOff { note: 60 }),
        (2600, 0, NodeEvent::NoteOff { note: 55 }),
        (
            3000,
            1,
            NodeEvent::NoteOn {
                note: 62,
                velocity: 120,
            },
        ),
        (4000, 1, NodeEvent::NoteOff { note: 62 }),
    ];
    let go = |block: usize| {
        let mut host = TrackHost::new(2, RATE, 1024);
        host.swap(0, Some(poly())).unwrap();
        host.swap(1, Some(fm())).unwrap();
        let (mut l, mut r) = (vec![0f32; block], vec![0f32; block]);
        let mut out = Vec::new();
        let mut frame = 0u64;
        while frame < 6000 {
            let n = block.min((6000 - frame) as usize);
            for &(at, slot, ev) in &script {
                if (frame..frame + n as u64).contains(&at) {
                    host.event(slot, Timed::new((at - frame) as u32, ev));
                }
            }
            l[..n].fill(0.0);
            r[..n].fill(0.0);
            host.render(
                &ctx(frame),
                n,
                &mut Sum {
                    left: &mut l[..n],
                    right: &mut r[..n],
                },
            );
            out.extend(l[..n].iter().zip(&r[..n]).flat_map(|(a, b)| [*a, *b]));
            frame += n as u64;
        }
        out
    };
    let whole = go(128);
    assert!(whole.iter().any(|x| x.abs() > 0.05));
    for block in [1, 31, 64, 1000] {
        assert!(go(block) == whole, "block size {block}");
    }
}

#[test]
fn a_swapped_out_instrument_rings_out_then_comes_back_to_be_dropped() {
    let mut host = TrackHost::new(1, RATE, 1024);
    host.swap(0, Some(poly())).unwrap();
    let mut out = Split::new(1, 128);
    let mut frame = 0;
    host.note_on(0, 0, 48, 120);
    run(&mut host, &mut out, &mut frame, 20, 128);
    // FM takes the slot: Poly's note keeps sounding (its release), FM plays.
    host.swap(0, Some(fm())).unwrap();
    assert!(host.ringing(0));
    assert!(host.pop_retired().is_none(), "nothing retired yet");
    assert_eq!(
        host.params(0).len(),
        Fm::new(RATE).params().len(),
        "the slot is FM's now"
    );
    let p = run(&mut host, &mut out, &mut frame, 1, 128);
    assert!(p[0][0] > 0.01, "Poly's release rings on");
    // Poly's release ends; after QUIET_FRAMES of quiet it is retired.
    let mut periods = 0;
    while host.ringing(0) {
        run(&mut host, &mut out, &mut frame, 1, 128);
        periods += 1;
        assert!(periods < 3000, "rang out too long");
    }
    assert!(periods * 128 >= QUIET_FRAMES as usize);
    let old = host.pop_retired().expect("Poly comes back");
    assert_eq!(old.node().info().id, "mpfree.poly");
    assert_eq!(host.stats(0).swaps, 1);
    // A swap while one still rings retires the ringing one at once.
    host.note_on(0, 0, 60, 100);
    run(&mut host, &mut out, &mut frame, 5, 128);
    host.swap(0, Some(poly())).unwrap();
    host.swap(0, None).unwrap();
    assert!(host.ringing(0));
    assert_eq!(
        host.pop_retired().map(|i| i.node().info().id),
        Some("mpfree.fm")
    );
    assert!(host.instrument(0).is_none());
    assert_eq!(
        host.swap(5, Some(fm())).map_err(|e| e.0),
        Err(SwapError::NoSlot)
    );
}

#[test]
fn stop_fades_every_slot_out_within_ten_ms() {
    let mut host = TrackHost::new(2, RATE, 1024);
    host.swap(0, Some(poly())).unwrap();
    host.swap(1, Some(fm())).unwrap();
    let mut out = Split::new(2, 128);
    let mut frame = 0;
    for (slot, note) in [(0, 48), (0, 55), (1, 60)] {
        host.note_on(slot, 0, note, 120);
    }
    run(&mut host, &mut out, &mut frame, 100, 128);
    host.stop();
    let p = run(&mut host, &mut out, &mut frame, 6, 128);
    let after = (STOP_FADE as usize).div_ceil(128);
    for block in &p[after..] {
        assert_eq!(block, &vec![0.0, 0.0], "silent after the fade");
    }
    // A pad let go after STOP sends nothing and counts nothing odd.
    host.note_off(0, 0, 48);
    assert_eq!(host.stats(0).stray_offs, 1);
}

/// A node that puts out NaN once asked to.
struct Bad {
    nan: bool,
    resets: u32,
}

static BAD: NodeInfo = NodeInfo {
    id: "test.bad",
    name: "Bad",
    kind: Kind::Instrument,
};

impl Node for Bad {
    fn info(&self) -> &'static NodeInfo {
        &BAD
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        0.0
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {
        self.nan = false;
        self.resets += 1;
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        if events
            .iter()
            .any(|e| matches!(e.ev, NodeEvent::NoteOn { .. }))
        {
            self.nan = true;
        }
        let v = if self.nan { f32::NAN } else { 0.25 };
        for x in left.iter_mut().chain(right.iter_mut()) {
            *x += v;
        }
    }
}

#[test]
fn a_non_finite_block_is_dropped_and_the_node_reset() {
    let mut host = TrackHost::new(1, RATE, 1024);
    host.swap(
        0,
        Some(Instrument::new(
            Box::new(Bad {
                nan: false,
                resets: 0,
            }),
            RATE,
            1024,
        )),
    )
    .unwrap();
    let mut out = Split::new(1, 64);
    let mut frame = 0;
    assert_eq!(run(&mut host, &mut out, &mut frame, 1, 64)[0][0], 0.25);
    host.note_on(0, 0, 60, 100);
    assert_eq!(
        run(&mut host, &mut out, &mut frame, 1, 64)[0][0],
        0.0,
        "the bad block never reaches the track"
    );
    assert_eq!(
        run(&mut host, &mut out, &mut frame, 1, 64)[0][0],
        0.25,
        "reset, playing again"
    );
    assert_eq!(host.stats(0).resets, 1);
}

#[test]
fn a_full_queue_counts_what_it_drops_and_long_blocks_are_counted() {
    let mut host = TrackHost::new(1, RATE, 256);
    host.swap(0, Some(poly())).unwrap();
    for k in 0..MAX_EVENTS + 10 {
        host.param(0, (k % 128) as u32, mpfree_synth::params::CUTOFF, 1000.0);
    }
    assert_eq!(host.stats(0).dropped, 10);
    assert_eq!(
        host.param_value(0, mpfree_synth::params::CUTOFF),
        Some(Poly::new(RATE).param(mpfree_synth::params::CUTOFF)),
        "not applied until the render"
    );
    let mut out = Split::new(1, 512);
    let mut frame = 0;
    run(&mut host, &mut out, &mut frame, 1, 512);
    assert_eq!(host.clipped_frames(), 256);
    assert_eq!(
        host.param_value(0, mpfree_synth::params::CUTOFF),
        Some(1000.0)
    );
    assert_eq!(host.param_value(3, 0), None);
}

#[test]
fn into_the_mixer_and_out_of_the_master() {
    let cfg = MixerConfig {
        inputs: 0,
        pads: 0,
        tracks: 4,
        slots: 2,
        max_latency_ms: 10.0,
    };
    let mut mixer = Mixer::new(cfg, RATE, 256);
    let mut host = TrackHost::new(4, RATE, 256);
    host.swap(1, Some(fm())).unwrap();
    host.note_on(1, 0, 60, 110);
    let (mut l, mut r) = (vec![0f32; 128], vec![0f32; 128]);
    let mut peak = 0f32;
    let mut frame = 0;
    for _ in 0..40 {
        let c = ctx(frame);
        host.render(&c, 128, &mut mixer);
        l.fill(0.0);
        r.fill(0.0);
        mixer.process(&c, &mut l, &mut r);
        peak = peak.max(l.iter().chain(&r).fold(0f32, |m, x| m.max(x.abs())));
        frame += 128;
    }
    assert!(
        peak > 0.01 && peak <= 1.0,
        "FM through track 2 to the master: {peak}"
    );
}

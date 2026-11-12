//! Audio tracks against a real transport: recording plays back sample for
//! sample, latency, punch in and out with crossfades, loop recording into
//! takes, comping with undo and redo, edits, two tracks recording into one
//! memory, a full memory, forgetting takes, and the same output for any
//! period size.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_audiotracks::{AudioTracks, Command as A, Input, Monitor, FADE, PEAK_BLOCK};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;

/// A different value on every frame, so a frame out of place shows.
fn sig(g: u64) -> f32 {
    ((g * 7919) % 20011) as f32 / 20011.0 - 0.5
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    period: usize,
    frame: u64,
    input: Box<dyn Fn(u64) -> (f32, f32)>,
    out: Vec<f32>,
}

impl Rig {
    fn new(seconds: u32, period: usize) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            a: AudioTracks::new(RATE, seconds),
            b: Block::new(),
            period,
            frame: 0,
            input: Box::new(|g| (sig(g), -sig(g))),
            out: Vec::new(),
        }
    }

    fn at(&mut self, frame: u64, c: T) {
        assert!(self.t.command(frame, c));
    }

    fn cmd(&mut self, c: A) {
        assert!(self.a.command(c));
    }

    /// Run periods until `frame` (commands then apply from the next
    /// period, at `frame` when it is a period boundary).
    fn run_to(&mut self, frame: u64) {
        let mut inp = vec![0.0f32; 2 * self.period];
        let mut mix = vec![0.0f32; 2 * self.period];
        while self.frame < frame {
            let n = (self.period as u64).min(frame - self.frame) as usize;
            for j in 0..n {
                let (l, r) = (self.input)(self.frame + j as u64);
                inp[2 * j] = l;
                inp[2 * j + 1] = r;
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

    fn l(&self, g: u64) -> f32 {
        self.out[2 * g as usize]
    }
    fn r(&self, g: u64) -> f32 {
        self.out[2 * g as usize + 1]
    }
}

/// Track 0, stereo, armed: one bar recorded from tick 0 at START, then
/// stopped, rewound and the input silenced.
fn one_bar(period: usize) -> Rig {
    let mut r = Rig::new(10, period);
    r.cmd(A::SetInput(0, Input::Stereo));
    r.cmd(A::Arm(0, true));
    r.at(START, T::Record);
    r.at(START + BAR, T::Stop);
    r.run_to(START + BAR + 500);
    r.input = Box::new(|_| (0.0, 0.0));
    r
}

#[test]
fn a_recording_plays_back_sample_for_sample() {
    let mut r = one_bar(256);
    let take = r.a.take(0).unwrap();
    assert_eq!(take.channels, 2);
    assert_eq!(take.frames, BAR as usize + FADE);
    assert!(!take.recording);
    assert_eq!(take.anchor, 0.0);
    let regions = r.a.regions(0).to_vec();
    assert_eq!(regions.len(), 1);
    assert_eq!(
        (regions[0].start, regions[0].end, regions[0].src),
        (0, BAR_TICKS, 0)
    );
    // Monitoring (Auto, armed) heard the input while recording.
    assert_eq!(r.l(START + 100), sig(START + 100));
    // Playing back (armed, Auto: the track, not the input).
    let p = START + 2 * BAR;
    r.at(p, T::PlayStart);
    r.run_to(p + BAR + 2000);
    for k in FADE as u64..BAR {
        assert_eq!(r.l(p + k), sig(START + k), "frame {k}");
        assert_eq!(r.r(p + k), -sig(START + k), "frame {k}");
    }
    // The fade-in is quiet at the start and rises.
    assert_eq!(r.l(p), 0.0);
    assert!(r.l(p + FADE as u64 / 2).abs() <= sig(START + FADE as u64 / 2).abs());
    // The region ends after a short fade-out past its end.
    for k in BAR + FADE as u64..BAR + 2000 {
        assert_eq!(r.l(p + k), 0.0);
    }
}

#[test]
fn latency_puts_the_take_where_it_was_played() {
    let mut r = Rig::new(10, 128);
    const LAT: u64 = 300;
    // The player plays sig(g) at frame g; it reaches the input LAT later.
    r.input = Box::new(|g| {
        let s = if g >= LAT { sig(g - LAT) } else { 0.0 };
        (s, s)
    });
    r.cmd(A::SetLatency(LAT as u32));
    r.cmd(A::Arm(0, true));
    r.at(START, T::Record);
    r.at(START + BAR, T::Stop);
    r.run_to(START + BAR + 1000);
    // The take kept writing LAT frames past the stop, and a crossfade.
    assert_eq!(r.a.take(0).unwrap().frames, (BAR + LAT) as usize + FADE);
    r.input = Box::new(|_| (0.0, 0.0));
    let p = START + 3 * BAR;
    r.at(p, T::PlayStart);
    r.run_to(p + BAR);
    for k in FADE as u64..BAR - 1 {
        assert_eq!(r.l(p + k), sig(START + k), "frame {k}");
    }
}

#[test]
fn punch_in_and_out_crossfade_over_the_older_take() {
    let mut r = Rig::new(20, 200);
    r.input = Box::new(|_| (0.5, 0.5));
    r.cmd(A::Arm(0, true));
    r.cmd(A::SetMonitor(0, Monitor::Off));
    r.at(START, T::Record);
    r.at(START + 2 * BAR, T::Stop);
    r.run_to(START + 2 * BAR + 100);
    // Play from the start and punch in on bar 2 for half a bar.
    let p = START + 3 * BAR;
    r.input = Box::new(|_| (0.25, 0.25));
    r.at(p, T::PlayStart);
    r.at(p + BAR, T::Record);
    r.at(p + BAR + BAR / 2, T::Record);
    r.at(p + 2 * BAR + 200, T::Stop);
    r.run_to(p + 2 * BAR + 300);
    let regions = r.a.regions(0).to_vec();
    assert_eq!(regions.len(), 3, "{regions:?}");
    assert_eq!(
        (regions[0].take, regions[0].start, regions[0].end),
        (0, 0, BAR_TICKS)
    );
    assert_eq!(
        (regions[1].take, regions[1].start, regions[1].end),
        (1, BAR_TICKS, BAR_TICKS + BAR_TICKS / 2)
    );
    assert_eq!(
        (regions[2].take, regions[2].start),
        (0, BAR_TICKS + BAR_TICKS / 2)
    );
    assert_eq!(regions[2].src, (BAR + BAR / 2) as i64);
    // While recording, the track played nothing past the punch in
    // (monitor off).
    assert_eq!(r.l(p + BAR + 1000), 0.0);
    assert_eq!(r.l(p + BAR - 1000), 0.5);
    // Play it all back.
    r.input = Box::new(|_| (0.0, 0.0));
    let q = p + 3 * BAR;
    r.at(q, T::PlayStart);
    r.run_to(q + 2 * BAR);
    assert_eq!(r.l(q + BAR - 100), 0.5);
    assert_eq!(r.l(q + BAR + 1000), 0.25);
    assert_eq!(r.l(q + BAR + BAR / 2 + 1000), 0.5);
    // The crossfades never dip below the quieter take nor go far above
    // the louder (equal power on a constant).
    for k in q + BAR - 10..q + BAR + BAR / 2 + 2 * FADE as u64 {
        let v = r.l(k);
        assert!((0.25..=0.5 * 1.42).contains(&v), "frame {k}: {v}");
    }
}

/// Track 0 records three passes of a one-bar transport loop, a different
/// level each pass; returns the rig, stopped.
fn three_passes(period: usize) -> Rig {
    let mut r = Rig::new(20, period);
    r.input = Box::new(|g| {
        let pass = g.saturating_sub(START) / BAR;
        let v = [0.1, 0.2, 0.3, 0.0][pass.min(3) as usize];
        (v, v)
    });
    r.cmd(A::Arm(0, true));
    r.at(0, T::SetLoop(Some((0, BAR_TICKS))));
    r.at(START, T::Record);
    r.at(START + 3 * BAR, T::Stop);
    r.run_to(START + 3 * BAR + 10);
    r.input = Box::new(|_| (0.0, 0.0));
    r
}

#[test]
fn loop_recording_makes_a_take_per_pass_and_the_newest_plays() {
    let mut r = three_passes(256);
    let takes: Vec<_> = r.a.takes().collect();
    assert_eq!(takes.len(), 3, "{takes:?}");
    assert!(takes.iter().all(|t| t.group == takes[0].group));
    assert_eq!(takes.iter().map(|t| t.pass).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(r.a.regions(0).len(), 1);
    assert_eq!(r.a.regions(0)[0].take, takes[2].id);
    let p = START + 4 * BAR;
    r.at(p, T::SetLoop(None));
    r.at(p, T::PlayStart);
    r.run_to(p + BAR);
    assert_eq!(r.l(p + 1000), 0.3);
    assert_eq!(r.l(p + BAR - 1000), 0.3);
}

#[test]
fn comping_picks_a_stretch_of_an_older_take_and_undoes() {
    let mut r = three_passes(256);
    let first = r.a.takes().next().unwrap().id;
    r.cmd(A::Comp {
        take: first,
        from: 0,
        to: BAR_TICKS / 2,
    });
    let p = START + 4 * BAR;
    r.at(p, T::SetLoop(None));
    r.at(p, T::PlayStart);
    r.at(p + BAR, T::Stop);
    r.run_to(p + BAR + 10);
    assert_eq!(r.a.regions(0).len(), 2);
    assert_eq!(r.l(p + 1000), 0.1);
    assert_eq!(r.l(p + BAR - 1000), 0.3);
    // The comp boundary crossfades.
    for k in p + BAR / 2 - 10..p + BAR / 2 + 2 * FADE as u64 {
        let v = r.l(k);
        assert!((0.1..=0.43).contains(&v), "{v}");
    }
    r.cmd(A::Undo);
    let q = p + 2 * BAR;
    r.at(q, T::PlayStart);
    r.run_to(q + BAR);
    assert_eq!(r.l(q + 1000), 0.3);
    assert_eq!(r.a.history(), (3, 1));
    r.cmd(A::Redo);
    r.run_to(q + BAR + 10);
    assert_eq!(r.a.regions(0)[0].take, first);
}

#[test]
fn any_period_size_gives_the_same_output() {
    let run = |period: usize| {
        let mut r = three_passes(period);
        let first = r.a.takes().next().unwrap().id;
        r.cmd(A::Comp {
            take: first,
            from: 100,
            to: 2000,
        });
        r.cmd(A::SetPan(0, -0.3));
        let p = START + 4 * BAR + 17;
        r.at(p, T::SetLoop(None));
        r.at(p, T::PlayStart);
        r.run_to(p + BAR + 3000);
        r.out
    };
    let a = run(128);
    for period in [1usize, 7, 999, 4096] {
        let b = run(period);
        // Commands to the tracks apply at period boundaries, and the pan
        // ramps over a period, so compare from where both settled.
        let from = 2 * (START + 4 * BAR + 4200) as usize;
        assert_eq!(a.len(), b.len());
        assert!(a[from..] == b[from..], "period {period}");
    }
}

#[test]
fn edits_split_move_erase_gain_and_fades() {
    let mut r = one_bar(256);
    let half = BAR_TICKS / 2;
    r.cmd(A::Split { track: 0, at: half });
    r.cmd(A::Move {
        track: 0,
        at: half,
        to: 2 * BAR_TICKS,
    });
    r.cmd(A::SetGain {
        track: 0,
        at: 0,
        gain: 0.5,
    });
    r.cmd(A::SetFades {
        track: 0,
        at: 0,
        fade_in: 1000,
        fade_out: 0,
    });
    r.run_to(r.frame + 1);
    let rs = r.a.regions(0).to_vec();
    assert_eq!(rs.len(), 2);
    assert_eq!((rs[0].start, rs[0].end, rs[0].src), (0, half, 0));
    assert_eq!(
        (rs[1].start, rs[1].end, rs[1].src),
        (2 * BAR_TICKS, 2 * BAR_TICKS + half, (BAR / 2) as i64)
    );
    let p = r.frame + 10;
    r.at(p, T::PlayStart);
    r.run_to(p + 3 * BAR);
    assert_eq!(r.l(p + 2000), 0.5 * sig(START + 2000));
    // Within the fade-in: lower.
    assert!(r.l(p + 500).abs() <= 0.5 * sig(START + 500).abs());
    // The moved half plays its audio at bar 3.
    let k = BAR / 2 + 1000;
    assert_eq!(r.l(p + 2 * BAR + 1000), sig(START + k));
    // Erase, and edits on nothing are refused.
    let before = r.a.stats().refused;
    r.cmd(A::Erase {
        track: 0,
        from: 0,
        to: 3 * BAR_TICKS,
    });
    r.cmd(A::Split { track: 0, at: 100 });
    r.cmd(A::Split { track: 9, at: 100 });
    r.run_to(r.frame + 1);
    assert!(r.a.regions(0).is_empty());
    assert_eq!(r.a.stats().refused, before + 2);
}

#[test]
fn two_tracks_record_at_once_into_one_memory() {
    let mut r = Rig::new(4, 64);
    r.cmd(A::Arm(0, true));
    r.cmd(A::SetInput(0, Input::Left));
    r.cmd(A::Arm(1, true));
    r.cmd(A::SetInput(1, Input::Right));
    r.at(START, T::Record);
    r.at(START + BAR, T::Stop);
    r.run_to(START + BAR + 100);
    r.input = Box::new(|_| (0.0, 0.0));
    let t0 = r.a.take(r.a.regions(0)[0].take).unwrap();
    let t1 = r.a.take(r.a.regions(1)[0].take).unwrap();
    assert_eq!((t0.channels, t1.channels), (1, 1));
    for k in [0usize, 1, 5000, 20_000] {
        assert_eq!(r.a.sample(t0.id, k, 0), Some(sig(START + k as u64)));
        assert_eq!(r.a.sample(t1.id, k, 0), Some(-sig(START + k as u64)));
    }
    // Both mono takes together: a bar and a crossfade each.
    // Each take's memory ends on a peak block's edge.
    let each = (BAR as usize + FADE).div_ceil(PEAK_BLOCK) * PEAK_BLOCK;
    assert_eq!(r.a.memory().used, 2 * each);
    // Track 1 alone (solo) plays the right input on both sides.
    r.cmd(A::Solo(1, true));
    let p = r.frame + 1;
    r.at(p, T::PlayStart);
    r.run_to(p + BAR);
    assert_eq!(r.l(p + 3000), -sig(START + 3000));
    assert_eq!(r.r(p + 3000), -sig(START + 3000));
}

#[test]
fn a_full_memory_cuts_the_take_short_and_keeps_it() {
    let mut r = Rig::new(1, 256); // 1 s of stereo
    r.cmd(A::Arm(0, true));
    r.cmd(A::SetInput(0, Input::Stereo));
    r.at(START, T::Record);
    r.at(START + 3 * BAR, T::Stop);
    r.run_to(START + 3 * BAR);
    let t = r.a.take(0).unwrap();
    assert!(t.cut_short);
    assert!(!t.recording);
    // 1 s of stereo, cut to whole peak blocks.
    assert_eq!(t.frames, r.a.memory().capacity / 2);
    assert_eq!(r.a.memory().capacity, 44_100 / PEAK_BLOCK * PEAK_BLOCK);
    assert_eq!(r.a.stats().full, 1);
    assert_eq!(r.a.memory().used, r.a.memory().capacity);
    let end = (t.frames as f64 * BAR_TICKS as f64 / BAR as f64).round() as i64;
    assert_eq!(r.a.regions(0)[0].end, end);
    assert_eq!(r.a.recording(0), None);
}

#[test]
fn forget_gives_memory_back_only_when_nothing_plays_the_take() {
    let mut r = one_bar(256);
    assert!(r.a.memory().used > 0);
    r.cmd(A::Forget(0));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.stats().refused, 1);
    r.cmd(A::Erase {
        track: 0,
        from: 0,
        to: BAR_TICKS,
    });
    r.cmd(A::Forget(0));
    r.run_to(r.frame + 1);
    // Undo could still bring it back.
    assert_eq!(r.a.stats().refused, 2);
    assert!(r.a.in_use(0));
    r.cmd(A::ClearHistory);
    r.cmd(A::Forget(0));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.stats().refused, 2);
    assert_eq!(r.a.memory().used, 0);
    assert!(r.a.take(0).is_none());
}

#[test]
fn disarming_ends_the_recording() {
    let mut r = Rig::new(10, 100);
    r.cmd(A::Arm(0, true));
    r.at(START, T::Record);
    r.run_to(START + BAR / 2);
    assert_eq!(r.a.recording(0), Some(0));
    r.cmd(A::Arm(0, false));
    r.run_to(START + BAR);
    assert_eq!(r.a.recording(0), None);
    let reg = r.a.regions(0)[0];
    assert_eq!((reg.start, reg.end), (0, BAR_TICKS / 2));
    // Still recording on the transport, but not on this track.
    assert!(r.a.take(1).is_none());
}

#[test]
fn inactive_tracks_neither_play_nor_record() {
    let mut r = one_bar(256);
    r.cmd(A::SetActive(false));
    let p = r.frame + 10;
    r.at(p, T::PlayStart);
    r.run_to(p + BAR);
    assert!(r.out[2 * p as usize..].iter().all(|&s| s == 0.0));
    r.at(r.frame, T::Record);
    r.run_to(r.frame + BAR);
    assert!(r.a.take(1).is_none());
}

#[test]
fn takes_export_as_wav() {
    let r = one_bar(256);
    let w = r.a.take_wav(0).unwrap();
    assert_eq!(&w[..4], b"RIFF");
    assert_eq!(w.len(), 44 + (BAR as usize + FADE) * 2 * 4);
    let s = f32::from_le_bytes([w[44], w[45], w[46], w[47]]);
    assert_eq!(s, sig(START));
    assert!(r.a.peaks(0, 0, 1000).is_some());
}

#[test]
fn a_count_in_records_from_where_it_ends() {
    let mut r = Rig::new(10, 300);
    r.cmd(A::Arm(0, true));
    r.at(0, T::SetCountIn(1));
    r.at(START, T::Record);
    r.at(START + 2 * BAR, T::Stop);
    r.run_to(START + 2 * BAR + 200);
    let t = r.a.take(0).unwrap();
    // The count-in bar was not recorded: the take starts on bar 1.
    assert_eq!(t.anchor, 0.0);
    assert_eq!(t.frames, BAR as usize + FADE);
    assert_eq!(r.a.sample(0, 0, 0), Some(sig(START + BAR)));
    let reg = r.a.regions(0)[0];
    assert_eq!((reg.start, reg.end), (0, BAR_TICKS));
}

#[test]
fn a_region_stays_on_its_bar_when_the_tempo_before_it_changes() {
    use mpfree_transport::{Segment, TempoMap};
    let mut r = Rig::new(10, 256);
    r.cmd(A::Arm(0, true));
    r.cmd(A::SetMonitor(0, Monitor::Off));
    // Punch in on bar 2 for one bar.
    r.at(START, T::Play);
    r.at(START + BAR, T::Record);
    r.at(START + 2 * BAR, T::Stop);
    r.run_to(START + 2 * BAR + 200);
    r.input = Box::new(|_| (0.0, 0.0));
    assert_eq!(r.a.regions(0)[0].start, BAR_TICKS);
    // Bar 1 now at 120 BPM (twice as long), the rest at 240.
    let map = TempoMap::new(&[
        Segment {
            start: 0,
            bpm: 120.0,
        },
        Segment {
            start: BAR_TICKS,
            bpm: 240.0,
        },
    ])
    .unwrap();
    r.t.set_tempo_map(map).unwrap();
    let p = r.frame + 100;
    r.at(p, T::PlayStart);
    r.run_to(p + 4 * BAR);
    // Bar 2 starts two seconds in; the take plays from its first frame
    // there, sample for sample.
    let bar2 = p + 2 * BAR;
    assert_eq!(r.l(bar2 - 1), 0.0);
    for k in [FADE as u64, 1000, 20_000] {
        assert_eq!(r.l(bar2 + k), sig(START + BAR + k), "{k}");
    }
}

#[test]
fn a_locate_while_recording_starts_a_new_take_where_it_lands() {
    let mut r = Rig::new(10, 128);
    r.cmd(A::Arm(0, true));
    r.at(START, T::Record);
    r.at(START + BAR, T::Locate(4 * BAR_TICKS));
    r.at(START + 2 * BAR, T::Stop);
    r.run_to(START + 2 * BAR + 200);
    let rs = r.a.regions(0).to_vec();
    assert_eq!(rs.len(), 2, "{rs:?}");
    assert_eq!((rs[0].start, rs[0].end), (0, BAR_TICKS));
    assert_eq!((rs[1].start, rs[1].end), (4 * BAR_TICKS, 5 * BAR_TICKS));
    let (a, b) = (r.a.take(rs[0].take).unwrap(), r.a.take(rs[1].take).unwrap());
    assert_eq!(a.group, b.group);
    assert_eq!(r.a.sample(b.id, 0, 0), Some(sig(START + BAR)));
}

/// The peaks the slow way: every sample.
fn brute(a: &AudioTracks, id: u16, from: usize, to: usize) -> (f32, f32) {
    let t = a.take(id).unwrap();
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for k in from..to.min(t.frames) {
        for c in 0..t.channels {
            let v = a.sample(id, k, c).unwrap();
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    (lo, hi)
}

#[test]
fn peaks_match_every_sample_even_in_reused_memory() {
    let mut r = one_bar(256);
    for (from, to) in [
        (0, 1),
        (0, 22_050),
        (100, 5_000),
        (127, 129),
        (3_333, 22_114),
        (22_000, 99_999),
    ] {
        assert_eq!(
            r.a.peaks(0, from, to),
            Some(brute(&r.a, 0, from, to)),
            "{from}..{to}"
        );
    }
    assert_eq!(r.a.peaks(0, 30_000, 40_000), None);
    // Forget it and record a quieter take into the same memory.
    r.cmd(A::Erase {
        track: 0,
        from: 0,
        to: BAR_TICKS,
    });
    r.cmd(A::ClearHistory);
    r.cmd(A::Forget(0));
    r.run_to(r.frame + 1);
    assert_eq!(r.a.memory().used, 0);
    r.input = Box::new(|g| (0.01 * sig(g), 0.0));
    r.cmd(A::SetInput(0, Input::Left));
    let p = r.frame + 7;
    r.at(p, T::Locate(0));
    r.at(p, T::Record);
    r.run_to(p + BAR / 3);
    // While it records, too.
    let id = r.a.recording(0).unwrap();
    let f = r.a.take(id).unwrap().frames;
    assert_eq!(r.a.peaks(id, 0, f), Some(brute(&r.a, id, 0, f)));
    let (lo, hi) = r.a.peaks(id, 0, f).unwrap();
    assert!(lo >= -0.01 && hi <= 0.01, "{lo} {hi}");
    for (from, to) in [(0, 300), (255, 257), (1000, f)] {
        assert_eq!(r.a.peaks(id, from, to), Some(brute(&r.a, id, from, to)));
    }
}

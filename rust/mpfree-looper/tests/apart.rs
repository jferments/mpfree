//! Each loop on its own output (audio tracks phase 4, step 4a): the
//! looper keeps what each loop and Listen played apart
//! (`Looper::process_each`), and their sum (`LoopOuts::mix_into`) is
//! `Looper::process`'s mix bit for bit: for playback, overdubs, mute,
//! solo, level, launches, Listen, Resample, merges and edits, at any
//! period size. A loop's own sound doesn't change with what the others
//! do, and a loop that didn't sound is silent.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::{Command as L, Edit as E, LayerCommand as Lc, Looper, Phase, MAX_LOOPS};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The transport starts playing here, so bar lines are at START + k * BAR.
const START: u64 = 1_000;

/// Two loopers given the same commands and input: `mixed` runs
/// `process`, `apart` runs `process_each`. Every period, the sum of
/// `apart`'s outputs must be `mixed`'s mix, bit for bit.
struct Twin {
    t: Transport,
    mixed: Looper,
    apart: Looper,
    b: Block,
    frame: u64,
    /// Frames a period, taken in turn.
    periods: Vec<usize>,
    turn: usize,
    input: Vec<f32>,
    mix: Vec<f32>,
    sum: Vec<f32>,
    /// Every frame so far of the mix, of each loop's own output and of
    /// Listen's, interleaved stereo.
    out: Vec<f32>,
    own: Vec<Vec<f32>>,
    listen: Vec<f32>,
    /// Periods in which Listen played, and in which a loop resampled.
    listened: usize,
    resampled: usize,
}

impl Twin {
    fn new(seconds: u32, periods: &[usize]) -> Twin {
        let most = *periods.iter().max().unwrap();
        Twin {
            t: Transport::new(RATE, 240.0).unwrap(),
            mixed: Looper::new(RATE, seconds),
            apart: Looper::new(RATE, seconds),
            b: Block::new(),
            frame: 0,
            periods: periods.to_vec(),
            turn: 0,
            input: vec![0.0; 2 * most],
            mix: vec![0.0; 2 * most],
            sum: vec![0.0; 2 * most],
            out: Vec::new(),
            own: vec![Vec::new(); MAX_LOOPS],
            listen: Vec::new(),
            listened: 0,
            resampled: 0,
        }
    }

    fn at(&mut self, frame: u64, c: T) {
        assert!(self.t.command(frame, c));
    }

    /// The same call on both loopers, with the same answer.
    fn both(&mut self, f: impl Fn(&mut Looper) -> bool) -> bool {
        let a = f(&mut self.mixed);
        assert_eq!(a, f(&mut self.apart), "the twins answered differently");
        a
    }

    fn cmd(&mut self, c: L) {
        assert!(self.both(|l| l.command(c)));
    }

    fn layer(&mut self, id: u8, c: Lc) {
        assert!(self.both(|l| l.layer_command(id, c)));
    }

    fn select(&mut self, n: usize) {
        assert!(self.both(|l| l.select_loop(n)));
    }

    fn phase(&self, n: usize) -> Phase {
        self.apart.loop_status(n).unwrap().phase
    }

    /// Run up to absolute frame `to`, feeding `input(frame)`, and check
    /// every period.
    fn run(&mut self, to: u64, input: &dyn Fn(u64) -> [f32; 2]) {
        while self.frame < to {
            let period = self.periods[self.turn % self.periods.len()];
            self.turn += 1;
            let n = (period as u64).min(to - self.frame) as usize;
            for i in 0..n {
                let x = input(self.frame + i as u64);
                self.input[2 * i] = x[0];
                self.input[2 * i + 1] = x[1];
            }
            self.mix[..2 * n].fill(0.0);
            assert!(self.t.process(n as u32, &mut self.b));
            self.mixed
                .process(&self.b, &self.input[..2 * n], &mut self.mix[..2 * n]);
            self.apart.process_each(&self.b, &self.input[..2 * n]);
            let outs = self.apart.outs();
            assert_eq!(outs.frames(), n);
            self.sum[..2 * n].fill(0.0);
            outs.mix_into(&mut self.sum[..2 * n]);
            for i in 0..2 * n {
                assert_eq!(
                    self.sum[i].to_bits(),
                    self.mix[i].to_bits(),
                    "frame {} channel {}: {} apart, {} mixed",
                    self.frame + (i / 2) as u64,
                    i % 2,
                    self.sum[i],
                    self.mix[i]
                );
            }
            for k in 0..MAX_LOOPS {
                let o = outs.loop_out(k);
                assert_eq!(o.len(), 2 * n);
                if !outs.sounded(k) {
                    assert!(o.iter().all(|&v| v == 0.0), "loop {k} didn't sound");
                }
                self.own[k].extend_from_slice(o);
            }
            match outs.listen() {
                Some((_, l)) => {
                    assert_eq!(l.len(), 2 * n);
                    self.listen.extend_from_slice(l);
                    self.listened += 1;
                }
                None => self.listen.extend(std::iter::repeat_n(0.0, 2 * n)),
            }
            if (0..MAX_LOOPS).any(|k| outs.resampled(k)) {
                self.resampled += 1;
            }
            self.out.extend_from_slice(&self.mix[..2 * n]);
            self.frame += n as u64;
        }
    }
}

/// Deterministic noise, different on each channel.
fn noise(f: u64) -> [f32; 2] {
    let h = |s: u64| {
        let mut x = f.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(s);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 32;
        (x % 10_000) as f32 / 10_000.0 - 0.5
    };
    [h(1), h(2)]
}

fn sine(f: u64) -> [f32; 2] {
    let p = f as f64 * std::f64::consts::TAU / 1000.37;
    [(0.5 * p.sin()) as f32, (0.5 * p.cos()) as f32]
}

fn dc(v: f32) -> impl Fn(u64) -> [f32; 2] {
    move |_| [v, -v]
}

/// A one-bar first pass of `input` on the selected loop, from the next
/// bar line after `from` (the song playing); returns once it plays.
fn record_bar(r: &mut Twin, n: usize, input: &dyn Fn(u64) -> [f32; 2]) {
    r.select(n);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    let mut guard = 0;
    while r.phase(n) != Phase::Playing {
        r.run(r.frame + 1_000, input);
        guard += 1;
        assert!(guard < 200, "loop {n} never played");
    }
}

#[test]
fn the_loops_apart_add_up_to_the_mix_bit_for_bit() {
    let mut r = Twin::new(20, &[97, 128, 333, 1, 4096, 64]);
    r.at(START, T::Play);
    record_bar(&mut r, 0, &noise);
    record_bar(&mut r, 1, &sine);
    // Loop 0 overdubbed, at a level, one layer turned down.
    r.select(0);
    r.cmd(L::Overdub);
    r.run(r.frame + BAR / 2, &dc(0.1));
    r.cmd(L::Overdub);
    r.run(r.frame + BAR, &dc(0.0));
    r.cmd(L::SetLevel(0.7));
    let id = r.apart.layer_info(1).unwrap().id;
    r.layer(id, Lc::Level(0.5));
    r.run(r.frame + BAR / 3, &dc(0.0));
    // Loop 1 muted and back, then soloed and back.
    r.select(1);
    r.cmd(L::Mute(true));
    r.run(r.frame + BAR / 2, &dc(0.0));
    r.cmd(L::Mute(false));
    r.run(r.frame + BAR / 2, &dc(0.0));
    r.cmd(L::Solo(true));
    r.run(r.frame + BAR / 2, &dc(0.0));
    r.cmd(L::Solo(false));
    r.run(r.frame + BAR / 2, &dc(0.0));
    // Loop 1 stopped from a bar line, and launched again.
    let bar = (r.frame - START) / BAR + 1;
    assert!(r.both(|l| l.launch(1, false, Some(bar as i64 * BAR_TICKS))));
    r.run(START + (bar + 1) * BAR, &dc(0.0));
    assert!(r.both(|l| l.launch(1, true, None)));
    r.run(r.frame + BAR / 2, &dc(0.0));
    // Loop 2 resamples the others, then plays with them.
    r.select(2);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Resample);
    let mut guard = 0;
    while r.phase(2) != Phase::Playing {
        r.run(r.frame + 1_000, &dc(0.3));
        guard += 1;
        assert!(guard < 200, "loop 2 never played");
    }
    r.run(r.frame + 2 * BAR, &dc(0.0));
    assert!(r.resampled > 0, "a loop resampled");
    // An edit and a merge on loop 0 while the others play.
    r.select(0);
    r.cmd(L::Edit(E::Reverse));
    r.run(r.frame + BAR / 2, &dc(0.0));
    r.cmd(L::Merge);
    r.run(r.frame + 2 * BAR, &dc(0.0));
    // Stopped: Listen plays loop 0 alone, on its own output.
    let stop = r.frame + 500;
    r.at(stop, T::Stop);
    r.run(stop + 2_000, &dc(0.0));
    assert!(r.both(|l| l.listen(0, None)));
    let from = r.frame;
    r.run(from + BAR, &dc(0.0));
    assert!(r.listened > 0, "Listen played");
    for f in from..r.frame {
        for c in 0..2 {
            let i = 2 * f as usize + c;
            assert_eq!(r.listen[i].to_bits(), r.out[i].to_bits(), "frame {f}");
        }
    }
    assert!(
        r.listen[2 * from as usize..].iter().any(|&v| v != 0.0),
        "Listen was heard"
    );
    r.both(|l| {
        l.stop_listening();
        true
    });
    r.run(r.frame + 2_000, &dc(0.0));
    // Playing again, and cleared one by one.
    let play = r.frame + 100;
    r.at(play, T::Play);
    r.run(play + BAR, &dc(0.0));
    for n in 0..3 {
        r.select(n);
        r.cmd(L::Clear);
        r.run(r.frame + BAR / 2, &dc(0.0));
    }
    assert!(r.out.iter().any(|&v| v != 0.0));
}

/// Presses at random on four loops (records, overdubs, undo and redo,
/// edits, merges, Resample, mute, solo, level, launches, stops, Listen),
/// at odd period sizes: every period, the loops apart add up to the mix
/// bit for bit, and a loop that didn't sound is silent.
#[test]
fn random_presses_keep_the_sum_exact() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut r = Twin::new(8, &[97, 1, 128, 333, 7, 1024, 64]);
    r.at(START, T::Play);
    let mut playing = true;
    for round in 0..900u64 {
        let c = match next() % 20 {
            0 | 1 => L::Record,
            2 | 3 => L::Overdub,
            4 => L::Undo,
            5 => L::Redo,
            6 => L::Edit(E::Half),
            7 => L::Edit(E::Double),
            8 => L::Edit(E::Reverse),
            9 => L::Merge,
            10 => L::Resample,
            11 => L::Mute(next() % 2 == 0),
            12 => L::Solo(next() % 3 == 0),
            13 => L::SetLevel((next() % 9) as f32 / 6.0),
            14 if round % 30 == 0 => L::Clear,
            15 => {
                let n = (next() % 4) as usize;
                r.select(n);
                continue;
            }
            16 => {
                let n = (next() % 4) as usize;
                let on = next() % 2 == 0;
                let at = (next() % 2 == 0).then(|| {
                    let bar = (r.frame.saturating_sub(START)) / BAR + 1;
                    bar as i64 * BAR_TICKS
                });
                r.both(|l| l.launch(n, on, at));
                r.run(r.frame + 200 + next() % 2_000, &dc(0.0));
                continue;
            }
            17 => {
                // The song stops or plays.
                let at = r.frame + 50;
                r.at(at, if playing { T::Stop } else { T::Play });
                playing = !playing;
                r.run(at + 300, &dc(0.0));
                continue;
            }
            18 => {
                let n = (next() % 4) as usize;
                r.both(|l| l.listen(n, None));
                r.run(r.frame + 500 + next() % 3_000, &dc(0.0));
                continue;
            }
            _ => L::SetBars(Some((next() % 2 + 1) as u32)),
        };
        r.cmd(c);
        let input = |f: u64| [0.3 * sine(f)[0], 0.2 * noise(f)[1]];
        r.run(r.frame + 300 + next() % 3_000, &input);
    }
    assert!(r.out.iter().all(|v| v.is_finite()));
    assert!(r.out.iter().any(|&v| v != 0.0), "the loops played");
    assert!(r.resampled > 0, "a loop resampled");
    assert!(r.listened > 0, "Listen played");
}

/// Loop 0 on its own, and loop 0 beside a loop 1 that records, overdubs,
/// is muted, turned down, edited and cleared: loop 0's own output is the
/// same, bit for bit.
#[test]
fn a_loop_s_own_sound_doesnt_change_with_what_the_others_do() {
    let periods = [128, 97, 333];
    let mut alone = Twin::new(12, &periods);
    let mut beside = Twin::new(12, &periods);
    for r in [&mut alone, &mut beside] {
        r.at(START, T::Play);
        record_bar(r, 0, &noise);
    }
    // Loop 1 only beside, while loop 0 plays on in both.
    let to = beside.frame;
    alone.run(to, &dc(0.0));
    record_bar(&mut beside, 1, &sine);
    beside.select(1);
    beside.cmd(L::Overdub);
    beside.run(beside.frame + BAR / 2, &dc(0.2));
    beside.cmd(L::Overdub);
    beside.run(beside.frame + BAR / 2, &dc(0.0));
    beside.cmd(L::SetLevel(0.3));
    beside.cmd(L::Edit(E::Reverse));
    beside.run(beside.frame + BAR / 2, &dc(0.0));
    beside.cmd(L::Mute(true));
    beside.run(beside.frame + BAR / 2, &dc(0.0));
    beside.cmd(L::Clear);
    beside.run(beside.frame + BAR, &dc(0.0));
    alone.run(beside.frame, &dc(0.0));
    assert_eq!(alone.frame, beside.frame);
    let a = &alone.own[0];
    let b = &beside.own[0];
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(
            x.to_bits(),
            y.to_bits(),
            "frame {} channel {}",
            i / 2,
            i % 2
        );
    }
    assert!(a.iter().any(|&v| v != 0.0), "loop 0 played");
    // Loop 1's own output beside had its sound, and alone none.
    assert!(beside.own[1].iter().any(|&v| v != 0.0));
    assert!(alone.own[1].iter().all(|&v| v == 0.0));
    // Beside, the mix differs from loop 0 alone: it was loop 1's sound
    // that made the difference, not loop 0's.
    assert_ne!(alone.out, beside.out);
}

use super::*;
use crate::workload::{saw_levels, Formants, Supersaw};

/// A small deterministic random source.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
    /// -1 to 1.
    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// A table of `len` entries with sample values in -1 to 1 and steps in the
/// size a real table's are (a difference of up to 2 over the phase bits
/// below the index).
fn table(rng: &mut Rng, len: usize) -> Vec<[f32; 2]> {
    let per = len as f32 / 4_294_967_296.0;
    (0..len)
        .map(|_| [rng.unit(), rng.unit() * 2.0 * per])
        .collect()
}

fn shift_of(len: usize) -> u32 {
    32 - len.trailing_zeros()
}

fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|v| v.to_bits()).collect()
}

/// The twin against a plain f64 sum, one frame at a time.
#[test]
fn the_twin_reads_each_table_linearly_and_sums_in_order() {
    let mut rng = Rng(7);
    let len = 512;
    let t = table(&mut rng, len);
    let shift = shift_of(len);
    let mut ph = [3u32 << 30, 17, u32::MAX - 5];
    let inc = [1u32 << 22, 12_345_678, 3_000_000_001];
    let gain = [0.5f32, -0.25, 0.125];
    let start = ph;
    let mut buf = vec![9.0f32; 37];
    plain::table_sum::<false>(&t, shift, &mut buf, &mut ph, &inc, &gain);
    for (n, &y) in buf.iter().enumerate() {
        let mut want = 0.0f64;
        for k in 0..3 {
            let p = start[k].wrapping_add(inc[k].wrapping_mul(n as u32));
            let [a, s] = t[(p >> shift) as usize];
            let frac = f64::from(p & ((1 << shift) - 1));
            want += (f64::from(a) + f64::from(s) * frac) * f64::from(gain[k]);
        }
        assert!((f64::from(y) - want).abs() < 1e-5, "{n}: {y} vs {want}");
    }
    for k in 0..3 {
        assert_eq!(ph[k], start[k].wrapping_add(inc[k].wrapping_mul(37)));
    }
}

#[test]
fn writing_starts_from_zero_and_adding_keeps_what_is_there() {
    let mut rng = Rng(11);
    let t = table(&mut rng, 1024);
    let shift = shift_of(1024);
    let (inc, gain) = ([1u32 << 24, 3 << 22], [0.5f32, 0.5]);
    let mut a = vec![5.0f32; 20];
    let mut b = vec![5.0f32; 20];
    let (mut pa, mut pb) = ([0u32, 1 << 31], [0u32, 1 << 31]);
    plain::table_sum::<false>(&t, shift, &mut a, &mut pa, &inc, &gain);
    plain::table_sum::<true>(&t, shift, &mut b, &mut pb, &inc, &gain);
    for (x, y) in a.iter().zip(&b) {
        assert!((y - x - 5.0).abs() < 1e-5, "{x} {y}");
    }
    // A silent first oscillator writes +0, never -0.
    let mut z = vec![1.0f32; 8];
    plain::table_sum::<false>(&t, shift, &mut z, &mut [0], &[0], &[-0.0]);
    assert!(z.iter().all(|x| x.to_bits() == 0), "{z:?}");
}

#[test]
fn any_block_size_gives_the_same_samples_on_either_path() {
    let mut rng = Rng(3);
    let t = table(&mut rng, 2048);
    let shift = shift_of(2048);
    let inc: Vec<u32> = (0..7).map(|_| rng.next() >> 6).collect();
    let gain: Vec<f32> = (0..7).map(|_| rng.unit()).collect();
    let start: Vec<u32> = (0..7).map(|_| rng.next()).collect();
    let whole = {
        let mut out = vec![0.0f32; 300];
        let mut ph = start.clone();
        plain::table_sum::<false>(&t, shift, &mut out, &mut ph, &inc, &gain);
        out
    };
    for path in [Path::Plain, Path::Neon] {
        let mut cut = vec![0.0f32; 300];
        let mut ph = start.clone();
        let mut at = 0;
        for n in [1, 7, 16, 64, 100, 3, 109] {
            table_sum_on::<false>(path, &t, shift, &mut cut[at..at + n], &mut ph, &inc, &gain);
            at += n;
        }
        assert_eq!(bits(&whole), bits(&cut), "{path:?}");
    }
}

#[test]
fn no_oscillator_or_no_frame_changes_nothing() {
    let t = vec![[0.5f32, 0.0]; 512];
    let mut buf = vec![2.0f32; 9];
    for path in [Path::Plain, Path::Neon] {
        table_sum_on::<false>(path, &t, 23, &mut buf, &mut [], &[], &[]);
        assert!(buf.iter().all(|&x| x == 2.0));
        let mut ph = [5u32];
        table_sum_on::<false>(path, &t, 23, &mut [], &mut ph, &[9], &[1.0]);
        assert_eq!(ph, [5]);
    }
}

#[test]
fn the_shortest_slice_sets_how_many_oscillators_play() {
    let mut rng = Rng(5);
    let t = table(&mut rng, 512);
    let mut a = vec![0.0f32; 12];
    let mut b = vec![0.0f32; 12];
    let mut pa = [1u32, 2, 3];
    let mut pb = [1u32, 2];
    for path in [Path::Plain, Path::Neon] {
        table_sum_on::<false>(path, &t, 23, &mut a, &mut pa, &[7, 8, 9], &[0.1, 0.2]);
        table_sum_on::<false>(path, &t, 23, &mut b, &mut pb, &[7, 8], &[0.1, 0.2]);
        assert_eq!(bits(&a), bits(&b));
        assert_eq!(pa[2], 3, "the third phase stays");
    }
}

#[test]
fn the_status_says_which_code_runs() {
    let s = status();
    if neon_available() {
        assert_eq!(s, "NEON");
        assert_eq!(path(), Path::Neon);
    } else {
        assert!(s.starts_with("plain Rust"), "{s}");
        assert_eq!(path(), Path::Plain);
        // Asking for NEON where it can't run runs the twin, and says so.
        let t = vec![[0.0f32, 0.0]; 512];
        let took = table_sum_on::<false>(Path::Neon, &t, 23, &mut [0.0; 8], &mut [0], &[1], &[1.0]);
        assert_eq!(took, Path::Plain);
        assert_eq!(fallbacks(), 0);
    }
}

#[test]
fn the_workload_plays_a_saw_chord_at_a_sensible_level() {
    let levels = saw_levels();
    assert_eq!(levels.len(), 17);
    for lv in &levels {
        assert_eq!(lv.table.len(), 1 << (32 - lv.shift));
        // No denormal anywhere: the paths can't differ on flushing.
        assert!(lv
            .table
            .iter()
            .flatten()
            .all(|x| *x == 0.0 || x.is_normal()));
    }
    let mut s = Supersaw::new(8, 44_100);
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let mut sum = 0.0f64;
    for _ in 0..50 {
        s.block(Path::Plain, &mut l, &mut r);
        sum += l
            .iter()
            .chain(&r)
            .map(|&x| f64::from(x).powi(2))
            .sum::<f64>();
    }
    let rms = (sum / (50.0 * 256.0)).sqrt();
    assert!(rms > 0.05 && rms < 1.0, "{rms}");
}

/// Random filters a real voice could have: every formant somewhere from
/// 50 Hz to 0.45 of the rate with a Q from 0.5 to 50 (Simper's SVF, as
/// the Choir sets them), signed gains (now and then 0), and state already
/// ringing.
fn formant_filters(rng: &mut Rng) -> Formants4 {
    let mut f = Formants4::default();
    for k in 0..FORMANTS {
        for lane in 0..4 {
            let fc = 50.0 + (rng.unit() as f64 * 0.5 + 0.5) * (0.45 * 44_100.0 - 50.0);
            let q = 0.5 + (rng.unit() as f64 * 0.5 + 0.5) * 49.5;
            let g = (std::f64::consts::PI * fc / 44_100.0).tan();
            let a1 = 1.0 / (1.0 + g * (g + 1.0 / q));
            f.a1[k][lane] = a1 as f32;
            f.a2[k][lane] = (g * a1) as f32;
            f.a3[k][lane] = (g * g * a1) as f32;
            f.g[k][lane] = if rng.below(8) == 0 { 0.0 } else { rng.unit() };
            f.s1[k][lane] = rng.unit() * 0.5;
            f.s2[k][lane] = rng.unit() * 0.5;
        }
    }
    f
}

fn rows(rng: &mut Rng, n: usize, scale: f32) -> Vec<[f32; 4]> {
    (0..n)
        .map(|_| std::array::from_fn(|_| rng.unit() * scale))
        .collect()
}

fn row_bits(x: &[[f32; 4]]) -> Vec<u32> {
    x.iter().flatten().map(|v| v.to_bits()).collect()
}

fn state_bits(f: &Formants4) -> Vec<u32> {
    [f.s1, f.s2]
        .iter()
        .flatten()
        .flatten()
        .map(|v| v.to_bits())
        .collect()
}

/// The twin against Simper's band-pass written out in f64, one lane and
/// one frame at a time.
#[test]
fn the_formant_twin_is_five_band_passes_summed() {
    let mut rng = Rng(21);
    let mut f = formant_filters(&mut rng);
    let start = f;
    let src = rows(&mut rng, 64, 1.0);
    let mut out = vec![[9.0f32; 4]; 64];
    plain::formants4(&src, &mut out, &mut f);
    for lane in 0..4 {
        let mut s: Vec<(f64, f64)> = (0..FORMANTS)
            .map(|k| (f64::from(start.s1[k][lane]), f64::from(start.s2[k][lane])))
            .collect();
        for (n, e) in src.iter().enumerate() {
            let e = f64::from(e[lane]);
            let mut want = 0.0f64;
            for (k, (s1, s2)) in s.iter_mut().enumerate() {
                let (a1, a2, a3) = (
                    f64::from(start.a1[k][lane]),
                    f64::from(start.a2[k][lane]),
                    f64::from(start.a3[k][lane]),
                );
                let v3 = e - *s2;
                let v1 = a1 * *s1 + a2 * v3;
                let v2 = *s2 + a2 * *s1 + a3 * v3;
                *s1 = 2.0 * v1 - *s1;
                *s2 = 2.0 * v2 - *s2;
                want += f64::from(start.g[k][lane]) * v1;
            }
            let y = f64::from(out[n][lane]);
            assert!(
                (y - want).abs() < 1e-4,
                "lane {lane} frame {n}: {y} vs {want}"
            );
        }
    }
}

/// One formant at unit peak gain (g = k): a sine at its centre comes out
/// at about its own level, one three octaves up hardly at all.
#[test]
fn a_formant_passes_its_centre_and_little_else() {
    let (fs, fc, q) = (44_100.0f64, 1_000.0f64, 10.0f64);
    let g = (std::f64::consts::PI * fc / fs).tan();
    let a1 = 1.0 / (1.0 + g * (g + 1.0 / q));
    let level = |hz: f64| {
        let mut f = Formants4::default();
        for lane in 0..4 {
            f.a1[0][lane] = a1 as f32;
            f.a2[0][lane] = (g * a1) as f32;
            f.a3[0][lane] = (g * g * a1) as f32;
            f.g[0][lane] = (1.0 / q) as f32;
        }
        let n = 8_192;
        let src: Vec<[f32; 4]> = (0..n)
            .map(|i| [(std::f64::consts::TAU * hz * i as f64 / fs).sin() as f32; 4])
            .collect();
        let mut out = vec![[0.0f32; 4]; n];
        plain::formants4(&src, &mut out, &mut f);
        out[n / 2..]
            .iter()
            .map(|y| y[0].abs())
            .fold(0.0f32, f32::max)
    };
    let (at, far) = (level(fc), level(8.0 * fc));
    assert!((at - 1.0).abs() < 0.02, "{at}");
    assert!(far < 0.03, "{far}");
}

#[test]
fn the_formant_layout_is_the_kernels() {
    use core::mem::{offset_of, size_of};
    // The NEON kernel's offsets (neon.rs): s1 0, s2 80, a1 160, a2 240,
    // a3 320, g 400.
    assert_eq!(offset_of!(Formants4, s1), 0);
    assert_eq!(offset_of!(Formants4, s2), 80);
    assert_eq!(offset_of!(Formants4, a1), 160);
    assert_eq!(offset_of!(Formants4, a2), 240);
    assert_eq!(offset_of!(Formants4, a3), 320);
    assert_eq!(offset_of!(Formants4, g), 400);
    assert_eq!(size_of::<Formants4>(), 480);
}

#[test]
fn formants_give_the_same_samples_in_any_block_size_on_either_path() {
    let mut rng = Rng(8);
    let start = formant_filters(&mut rng);
    let src = rows(&mut rng, 160, 1.0);
    let (whole, end) = {
        let mut f = start;
        let mut out = vec![[0.0f32; 4]; 160];
        plain::formants4(&src, &mut out, &mut f);
        (out, f)
    };
    for path in [Path::Plain, Path::Neon] {
        let mut f = start;
        let mut cut = vec![[0.0f32; 4]; 160];
        let mut at = 0;
        for n in [1, 16, 3, 0, 4, 5, 16, 2, 16, 16, 15, 16, 16, 14, 16, 4] {
            formants4_on(path, &src[at..at + n], &mut cut[at..at + n], &mut f);
            at += n;
        }
        assert_eq!(at, 160);
        assert_eq!(row_bits(&whole), row_bits(&cut), "{path:?}");
        assert_eq!(state_bits(&end), state_bits(&f), "{path:?}");
    }
}

#[test]
fn formants_play_as_many_frames_as_the_shorter_slice() {
    let mut rng = Rng(13);
    let start = formant_filters(&mut rng);
    let src = rows(&mut rng, 12, 1.0);
    for path in [Path::Plain, Path::Neon] {
        let (mut fa, mut fb) = (start, start);
        let mut a = vec![[7.0f32; 4]; 20];
        let mut b = vec![[7.0f32; 4]; 12];
        formants4_on(path, &src, &mut a, &mut fa);
        formants4_on(path, &src, &mut b, &mut fb);
        assert_eq!(row_bits(&a[..12]), row_bits(&b));
        assert!(a[12..].iter().flatten().all(|&x| x == 7.0), "{path:?}");
        assert_eq!(state_bits(&fa), state_bits(&fb));
        // And no frame changes nothing.
        let mut fc = start;
        formants4_on(path, &src[..0], &mut a, &mut fc);
        formants4_on(path, &src, &mut a[..0], &mut fc);
        assert_eq!(fc, start);
    }
}

#[test]
fn the_choir_workload_sings_at_a_sensible_level() {
    let mut s = Formants::new(8, 44_100);
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let mut sum = 0.0f64;
    for _ in 0..100 {
        s.block(Path::Plain, &mut l, &mut r);
        sum += l
            .iter()
            .chain(&r)
            .map(|&x| f64::from(x).powi(2))
            .sum::<f64>();
    }
    let rms = (sum / (100.0 * 256.0)).sqrt();
    assert!(rms > 0.01 && rms < 1.0, "{rms}");
}

/// `mpfree_synth::dsp::poly_blep4` and `poly_blep`, copied: the step
/// correction the bank adds where `waves4` flags.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

fn poly_blep4(t: f32, dt: f32) -> f32 {
    if dt >= 0.25 {
        return poly_blep(t, dt);
    }
    let two = dt + dt;
    if t < two {
        let x = t / dt;
        if x < 1.0 {
            let x2 = x * x;
            2.0 * (-0.5 + x * (2.0 / 3.0) - x2 * x * (1.0 / 3.0) + x2 * x2 * 0.125)
        } else {
            let u = 2.0 - x;
            let u2 = u * u;
            -2.0 * u2 * u2 * (1.0 / 24.0)
        }
    } else if t > 1.0 - two {
        let x = (t - 1.0) / dt;
        if x > -1.0 {
            let x2 = x * x;
            2.0 * (0.5 + x * (2.0 / 3.0) - x2 * x * (1.0 / 3.0) - x2 * x2 * 0.125)
        } else {
            let u = x + 2.0;
            let u2 = u * u;
            2.0 * u2 * u2 * (1.0 / 24.0)
        }
    } else {
        0.0
    }
}

/// Four random oscillators as the bank has them: steps from 0 (a silent
/// note) to past a quarter of a cycle, `dt` the step's share as the bank
/// works it out, now and then exactly 0.25.
fn oscillators(rng: &mut Rng) -> Osc4 {
    let mut o = Osc4::default();
    for lane in 0..4 {
        o.ph[lane] = match rng.below(4) {
            0 => u32::MAX - rng.below(1 << 26),  // about to wrap
            1 => (1 << 31) - rng.below(1 << 26), // about to cross half
            _ => rng.next(),
        };
        let inc = match rng.below(8) {
            0 => 0,
            1 => 1 << 30, // dt exactly 0.25
            2 => rng.next() >> 1,
            _ => rng.next() >> (4 + rng.below(12)),
        };
        o.inc[lane] = inc;
        o.dt[lane] = (f64::from(inc) / 4_294_967_296.0) as f32;
    }
    o
}

/// The twin's waves are the bank's shapes, and a frame it doesn't flag
/// has no step correction: the bank's saw and square with the correction
/// added where flagged are its old per-frame formulas exactly.
#[test]
fn the_wave_twin_flags_every_frame_the_step_correction_touches() {
    let mut rng = Rng(31);
    let mut flagged = 0;
    for case in 0..400 {
        let o = oscillators(&mut rng);
        for shape in [Shape4::Saw, Shape4::Square, Shape4::Cubic] {
            let mut w = [[0.0f32; 4]; 40];
            let mut f = [0u8; 40];
            plain::waves4(shape, &o, &mut w, &mut f);
            for j in 0..40 {
                for lane in 0..4 {
                    let p = o.ph[lane].wrapping_add(o.inc[lane].wrapping_mul(j as u32));
                    let dt = o.dt[lane];
                    let t = (p >> 8) as f32 * (1.0 / 16_777_216.0);
                    let h = (p.wrapping_add(1 << 31) >> 8) as f32 * (1.0 / 16_777_216.0);
                    let on = f[j] >> lane & 1 == 1;
                    flagged += usize::from(on);
                    let ctx = format!("case {case} {shape:?} frame {j} lane {lane}");
                    // The bank's old formulas, and the twin's wave with the
                    // correction added only where flagged.
                    let (old, fixed) = match shape {
                        Shape4::Saw => {
                            let blep = if on { poly_blep4(t, dt) } else { 0.0 };
                            (t + t - 1.0 - poly_blep4(t, dt), w[j][lane] - blep)
                        }
                        Shape4::Square => {
                            let down = if p < 1 << 31 { -1.0 } else { 1.0 };
                            let fix = if on {
                                down - poly_blep4(t, dt) + poly_blep4(h, dt)
                            } else {
                                w[j][lane]
                            };
                            (down - poly_blep4(t, dt) + poly_blep4(h, dt), fix)
                        }
                        Shape4::Cubic => {
                            assert!(!on, "{ctx}");
                            (t * (t - 0.5) * (1.0 - t) * CUBIC_SCALE, w[j][lane])
                        }
                    };
                    assert_eq!(old.to_bits(), fixed.to_bits(), "{ctx}: {old} vs {fixed}");
                }
            }
        }
    }
    assert!(flagged > 1000, "{flagged}");
}

#[test]
fn the_oscillator_layout_is_the_kernels() {
    use core::mem::{offset_of, size_of};
    assert_eq!(offset_of!(Osc4, ph), 0);
    assert_eq!(offset_of!(Osc4, inc), 16);
    assert_eq!(offset_of!(Osc4, dt), 32);
    assert_eq!(size_of::<Osc4>(), 48);
}

/// Random levels for `buses` buses: each oscillator heard on a bus or not
/// (both 0), and now and then starting from or ramping to silence.
fn levels(rng: &mut Rng, buses: usize) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let mut amp = vec![[0.0f32; 4]; buses];
    let mut damp = vec![[0.0f32; 4]; buses];
    for b in 0..buses {
        for lane in 0..4 {
            let (a, d) = match rng.below(6) {
                0 | 1 => (0.0, 0.0),
                2 => (0.0, rng.unit() * 0.01),
                3 => (rng.unit(), 0.0),
                _ => (rng.unit(), rng.unit() * 0.01),
            };
            amp[b][lane] = a;
            damp[b][lane] = d;
        }
    }
    (amp, damp)
}

/// The twin against the bank's own loop: note by note, each bus the note
/// is heard on, frame by frame.
#[test]
fn the_mix_twin_is_the_banks_note_by_note_loop() {
    let mut rng = Rng(17);
    for _ in 0..200 {
        let buses = 1 + rng.below(6) as usize;
        let frames = rng.below(17) as usize;
        let waves = rows(&mut rng, frames, 1.2);
        let (amp, damp) = levels(&mut rng, buses);
        let start: Vec<[f32; MIX_STEP]> = (0..buses)
            .map(|_| std::array::from_fn(|_| rng.unit()))
            .collect();
        let (mut want, mut want_amp) = (start.clone(), amp.clone());
        for lane in 0..4 {
            for b in 0..buses {
                let (mut a, d) = (want_amp[b][lane], damp[b][lane]);
                if a == 0.0 && d == 0.0 {
                    continue;
                }
                for j in 0..frames {
                    want[b][j] += waves[j][lane] * a;
                    a += d;
                }
                want_amp[b][lane] = a;
            }
        }
        let (mut got, mut got_amp) = (start, amp);
        plain::mix4(&waves, &mut got_amp, &damp, &mut got);
        let bits = |x: &[[f32; MIX_STEP]]| -> Vec<u32> {
            x.iter().flatten().map(|v| v.to_bits()).collect()
        };
        assert_eq!(bits(&want), bits(&got));
        assert_eq!(row_bits(&want_amp), row_bits(&got_amp));
    }
}

#[test]
fn the_mix_gives_the_same_samples_for_any_frames_on_either_path() {
    let mut rng = Rng(23);
    for frames in 0..=MIX_STEP {
        let waves = rows(&mut rng, frames, 1.0);
        let (amp, damp) = levels(&mut rng, 6);
        let start: Vec<[f32; MIX_STEP]> = (0..6)
            .map(|_| std::array::from_fn(|_| rng.unit()))
            .collect();
        let (mut a, mut aa) = (start.clone(), amp.clone());
        plain::mix4(&waves, &mut aa, &damp, &mut a);
        for path in [Path::Plain, Path::Neon] {
            let (mut b, mut ba) = (start.clone(), amp.clone());
            mix4_on(path, &waves, &mut ba, &damp, &mut b);
            let bits = |x: &[[f32; MIX_STEP]]| -> Vec<u32> {
                x.iter().flatten().map(|v| v.to_bits()).collect()
            };
            assert_eq!(bits(&a), bits(&b), "{path:?} frames {frames}");
            assert_eq!(row_bits(&aa), row_bits(&ba), "{path:?} frames {frames}");
        }
    }
}

#[test]
fn the_shortest_slice_sets_how_many_buses_mix() {
    let mut rng = Rng(29);
    let waves = rows(&mut rng, 16, 1.0);
    let (amp, damp) = levels(&mut rng, 4);
    for path in [Path::Plain, Path::Neon] {
        let mut buses = vec![[0.5f32; MIX_STEP]; 4];
        let mut a = amp.clone();
        mix4_on(path, &waves, &mut a[..3], &damp, &mut buses);
        assert!(buses[3].iter().all(|&x| x == 0.5), "{path:?}");
        assert_eq!(a[3], amp[3]);
        let mut b = amp.clone();
        let mut fewer = vec![[0.5f32; MIX_STEP]; 2];
        mix4_on(path, &waves, &mut b, &damp, &mut fewer);
        assert_eq!(b[2], amp[2]);
    }
}

/// A random `kc`-step GEMM problem: packed `a`, packed `b` (each a little
/// longer than it needs, as a slice of a bigger panel is) and a block.
fn gemm_case(rng: &mut Rng, kc: usize) -> (Vec<f32>, Vec<f32>, [f32; 32]) {
    let a: Vec<f32> = (0..kc * 4 + rng.below(5) as usize)
        .map(|_| rng.unit())
        .collect();
    let b: Vec<f32> = (0..kc * 8 + rng.below(9) as usize)
        .map(|_| rng.unit())
        .collect();
    let acc = std::array::from_fn(|_| rng.unit());
    (a, b, acc)
}

/// The twin against a plain f64 sum of the same products.
#[test]
fn the_gemm_twin_adds_every_product_to_its_block() {
    let mut rng = Rng(4096);
    for kc in [0, 1, 2, 7, 64, 300] {
        let (a, b, start) = gemm_case(&mut rng, kc);
        let mut acc = start;
        plain::gemm4x8(kc, &a, &b, &mut acc);
        for r in 0..GEMM_MR {
            for j in 0..GEMM_NR {
                let mut want = f64::from(start[r * 8 + j]);
                for k in 0..kc {
                    want += f64::from(a[k * 4 + r]) * f64::from(b[k * 8 + j]);
                }
                let got = f64::from(acc[r * 8 + j]);
                assert!(
                    (got - want).abs() < 5e-4,
                    "kc {kc} [{r}][{j}]: {got} vs {want}"
                );
            }
        }
        if kc == 0 {
            assert_eq!(bits(&acc), bits(&start));
        }
    }
}

#[test]
fn the_gemm_gives_the_same_block_on_either_path() {
    let mut rng = Rng(31);
    for kc in [0, 1, 3, 16, 129] {
        let (a, b, start) = gemm_case(&mut rng, kc);
        let mut want = start;
        plain::gemm4x8(kc, &a, &b, &mut want);
        for path in [Path::Plain, Path::Neon] {
            let mut got = start;
            gemm4x8_on(path, kc, &a, &b, &mut got);
            assert_eq!(bits(&got), bits(&want), "{path:?} kc {kc}");
        }
    }
}

#[test]
#[should_panic]
fn a_gemm_panel_shorter_than_its_steps_panics() {
    let a = vec![0.0f32; 4 * 9];
    let b = vec![0.0f32; 8 * 10];
    let mut acc = [0.0f32; 32];
    gemm4x8(10, &a, &b, &mut acc);
}

/// Half-precision bits by hand: sign, a biased exponent (1 to 30) or 0 for
/// a subnormal, and ten mantissa bits, read as the IEEE format says, in f64.
fn half_by_hand(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = i32::from((h >> 10) & 0x1f);
    let m = f64::from(h & 0x3ff);
    match e {
        0 => sign * m * 2f64.powi(-24),
        31 if m == 0.0 => sign * f64::INFINITY,
        31 => f64::NAN,
        _ => sign * (1.0 + m / 1024.0) * 2f64.powi(e - 15),
    }
}

#[test]
fn every_half_converts_exactly() {
    for h in 0..=u16::MAX {
        let got = plain::half(h);
        let want = half_by_hand(h);
        if want.is_nan() {
            assert_eq!(got.to_bits(), 0x7fc0_0000, "{h:#06x}");
        } else {
            // Every half is an f32 exactly, so the f64 rounds to it with
            // nothing lost, and the sign of a zero carries over.
            assert_eq!(got.to_bits(), (want as f32).to_bits(), "{h:#06x}");
        }
    }
    for (h, v) in [
        (0x3c00u16, 1.0f32),
        (0xc000, -2.0),
        (0x7bff, 65504.0),
        (0x0400, 6.103_515_6e-5),
        (0x0001, 5.960_464_5e-8),
        (0x8000, -0.0),
        (0x7c00, f32::INFINITY),
        (0xfc00, f32::NEG_INFINITY),
    ] {
        assert_eq!(plain::half(h).to_bits(), v.to_bits(), "{h:#06x}");
    }
}

#[test]
fn the_shorter_slice_sets_how_many_halves_convert() {
    let src: Vec<u16> = (0..21).map(|i| 0x3c00 + i).collect();
    for path in [Path::Plain, Path::Neon] {
        let mut dst = vec![9.0f32; 30];
        f16_to_f32_on(path, &src, &mut dst);
        assert!(dst[21..].iter().all(|&x| x == 9.0), "{path:?}");
        let mut fewer = vec![9.0f32; 11];
        f16_to_f32_on(path, &src, &mut fewer);
        let want: Vec<f32> = src[..11].iter().map(|&h| plain::half(h)).collect();
        assert_eq!(bits(&fewer), bits(&want), "{path:?}");
    }
}

// The kernel against its twin: run on the ARMv7 build, under qemu-arm in
// CI (`cargo test --target armv7-unknown-linux-gnueabihf -p mpfree-simd`).
#[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
mod on_arm {
    use super::*;

    /// Under qemu-arm (and on the MPC) NEON is there, so these tests run the
    /// kernel and can't pass by running the twin twice.
    #[test]
    fn neon_is_there() {
        assert!(neon_available(), "no NEON reported by getauxval(AT_HWCAP)");
        assert_eq!(path(), Path::Neon);
        assert_eq!(status(), "NEON");
    }

    /// One random call on both paths; panics with the case if they differ.
    fn check(rng: &mut Rng, case: usize) {
        let len = 1usize << (3 + rng.below(10)); // 8 to 4096 entries
        let shift = shift_of(len);
        let t = table(rng, len);
        let count = 1 + rng.below(9) as usize;
        let frames = rng.below(70) as usize;
        let start: Vec<u32> = (0..count)
            .map(|_| match rng.below(4) {
                0 => u32::MAX - rng.below(64), // about to wrap
                _ => rng.next(),
            })
            .collect();
        let inc: Vec<u32> = (0..count)
            .map(|_| match rng.below(5) {
                0 => 0,
                1 => u32::MAX - rng.below(1 << 20), // a step backwards
                2 => rng.next(),
                _ => rng.next() >> rng.below(20),
            })
            .collect();
        let gain: Vec<f32> = (0..count)
            .map(|_| if rng.below(8) == 0 { 0.0 } else { rng.unit() })
            .collect();
        let fill: Vec<f32> = (0..frames).map(|_| rng.unit() * 3.0).collect();
        for add in [false, true] {
            let (mut a, mut b) = (fill.clone(), fill.clone());
            let (mut pa, mut pb) = (start.clone(), start.clone());
            let took = if add {
                plain::table_sum::<true>(&t, shift, &mut a, &mut pa, &inc, &gain);
                table_sum_on::<true>(Path::Neon, &t, shift, &mut b, &mut pb, &inc, &gain)
            } else {
                plain::table_sum::<false>(&t, shift, &mut a, &mut pa, &inc, &gain);
                table_sum_on::<false>(Path::Neon, &t, shift, &mut b, &mut pb, &inc, &gain)
            };
            assert_eq!(took == Path::Neon, frames >= 4, "case {case}");
            assert_eq!(
                bits(&a),
                bits(&b),
                "case {case}: len {len} oscillators {count} frames {frames} add {add}"
            );
            assert_eq!(pa, pb, "case {case}: phases");
        }
    }

    #[test]
    fn the_kernel_matches_its_twin_bit_for_bit() {
        let mut rng = Rng(0x5eed);
        for case in 0..3000 {
            check(&mut rng, case);
        }
    }

    /// Poly's own sizes: seven saws, 16-frame steps, every level of a real
    /// band-limited saw, both ways of starting.
    #[test]
    fn the_kernel_matches_its_twin_on_every_saw_level() {
        let mut rng = Rng(42);
        for lv in saw_levels() {
            for add in [false, true] {
                let ph: Vec<u32> = (0..7).map(|_| rng.next()).collect();
                let inc: Vec<u32> = (0..7).map(|_| rng.next() >> 4).collect();
                let gain: Vec<f32> = (0..7).map(|_| rng.unit() * 0.4).collect();
                let fill: Vec<f32> = (0..16).map(|_| rng.unit()).collect();
                let (mut a, mut b) = (fill.clone(), fill);
                let (mut pa, mut pb) = (ph.clone(), ph);
                if add {
                    plain::table_sum::<true>(&lv.table, lv.shift, &mut a, &mut pa, &inc, &gain);
                    table_sum_on::<true>(
                        Path::Neon,
                        &lv.table,
                        lv.shift,
                        &mut b,
                        &mut pb,
                        &inc,
                        &gain,
                    );
                } else {
                    plain::table_sum::<false>(&lv.table, lv.shift, &mut a, &mut pa, &inc, &gain);
                    table_sum_on::<false>(
                        Path::Neon,
                        &lv.table,
                        lv.shift,
                        &mut b,
                        &mut pb,
                        &inc,
                        &gain,
                    );
                }
                assert_eq!(bits(&a), bits(&b), "shift {} add {add}", lv.shift);
                assert_eq!(pa, pb);
            }
        }
    }

    /// The whole workload, many blocks, both paths: the same samples.
    #[test]
    fn the_supersaw_workload_sounds_the_same_on_both_paths() {
        let (mut a, mut b) = (Supersaw::new(8, 44_100), Supersaw::new(8, 44_100));
        let mut buf = [[0.0f32; 128]; 4];
        for _ in 0..200 {
            let [la, ra, lb, rb] = &mut buf;
            assert_eq!(a.block(Path::Plain, la, ra), Path::Plain);
            assert_eq!(b.block(Path::Neon, lb, rb), Path::Neon);
            assert_eq!(bits(la), bits(lb));
            assert_eq!(bits(ra), bits(rb));
        }
    }

    /// Values so small their products are denormal: NEON flushes them, and
    /// with the app's flush-to-zero on (the sound threads') the twin's VFP
    /// code flushes them too, so the two still agree. The lowest table
    /// levels (the largest shift, so the smallest steps) and gains near
    /// zero, as the synth library thread asked.
    #[test]
    fn with_flush_to_zero_on_the_kernel_matches_on_denormals_too() {
        assert!(mpfree_lanes::fpu::flush_to_zero());
        let mut rng = Rng(99);
        for case in 0..500 {
            let len = [8usize, 512, 4096][case % 3];
            let shift = shift_of(len);
            let tiny = |rng: &mut Rng| rng.unit() * 1e-36;
            let t: Vec<[f32; 2]> = (0..len)
                .map(|_| [tiny(&mut rng), tiny(&mut rng) * 1e-9])
                .collect();
            let count = 7;
            let ph: Vec<u32> = (0..count).map(|_| rng.next()).collect();
            let inc: Vec<u32> = (0..count).map(|_| rng.next() >> 3).collect();
            let gain: Vec<f32> = (0..count).map(|_| rng.unit() * 1e-3).collect();
            let fill: Vec<f32> = (0..16).map(|_| tiny(&mut rng)).collect();
            let (mut a, mut b) = (fill.clone(), fill);
            let (mut pa, mut pb) = (ph.clone(), ph);
            plain::table_sum::<true>(&t, shift, &mut a, &mut pa, &inc, &gain);
            table_sum_on::<true>(Path::Neon, &t, shift, &mut b, &mut pb, &inc, &gain);
            assert_eq!(bits(&a), bits(&b), "case {case}");
        }
    }

    #[test]
    fn a_table_that_does_not_fit_runs_the_twin_and_is_counted() {
        let t = vec![[0.25f32, 0.0]; 600]; // not a power of two
        let before = fallbacks();
        let mut buf = [0.0f32; 8];
        let took = table_sum_on::<false>(Path::Neon, &t, 23, &mut buf, &mut [0], &[1], &[1.0]);
        assert_eq!(took, Path::Plain);
        assert_eq!(buf, [0.25; 8]);
        assert_eq!(fallbacks(), before + 1);
    }

    /// One random formant call on both paths: the samples, the state and
    /// the untouched rest of `out` all the same.
    fn check_formants(rng: &mut Rng, case: usize) {
        let frames = if case.is_multiple_of(5) {
            17 + rng.below(100) as usize
        } else {
            rng.below(17) as usize // the Choir's steps: 0 to 16 frames
        };
        let extra = rng.below(3) as usize;
        let mut start = formant_filters(rng);
        if case.is_multiple_of(3) {
            // A silent lane (its gains +0) and one with gains of -0: the
            // sum still starts from +0, as the twin's does.
            for k in 0..FORMANTS {
                start.g[k][1] = 0.0;
                start.g[k][2] = -0.0;
            }
        }
        let src = rows(rng, frames, if case.is_multiple_of(7) { 4.0 } else { 1.0 });
        let fill = rows(rng, frames + extra, 3.0);
        let (mut fa, mut fb) = (start, start);
        let (mut a, mut b) = (fill.clone(), fill);
        plain::formants4(&src, &mut a, &mut fa);
        let took = formants4_on(Path::Neon, &src, &mut b, &mut fb);
        assert_eq!(took == Path::Neon, frames > 0, "case {case}");
        assert_eq!(row_bits(&a), row_bits(&b), "case {case}: frames {frames}");
        assert_eq!(state_bits(&fa), state_bits(&fb), "case {case}: state");
        assert_eq!(fa, fb, "case {case}: coefficients untouched");
    }

    #[test]
    fn the_formant_kernel_matches_its_twin_bit_for_bit() {
        let mut rng = Rng(0xf0f0);
        for case in 0..3000 {
            check_formants(&mut rng, case);
        }
    }

    /// A voice dying away: its filters ring on at the edge of the
    /// denormals for many steps of silence. NEON flushes the denormals;
    /// with flush-to-zero on (as on the sound threads) so does the twin,
    /// so the samples and the state stay the same throughout. Flushing
    /// leaves some filters cycling at the smallest normal numbers (about
    /// 1e-37, some 700 dB down) instead of reaching zero: the same bits on
    /// both paths.
    #[test]
    fn with_flush_to_zero_on_a_dying_formant_matches_to_the_last_bit() {
        assert!(mpfree_lanes::fpu::flush_to_zero());
        let mut rng = Rng(77);
        let mut f = formant_filters(&mut rng);
        for k in 0..FORMANTS {
            for lane in 0..4 {
                f.s1[k][lane] *= 1e-36;
                f.s2[k][lane] *= 1e-36;
            }
        }
        let (mut fa, mut fb) = (f, f);
        let silence = vec![[0.0f32; 4]; 16];
        let (mut a, mut b) = (vec![[0.0f32; 4]; 16], vec![[0.0f32; 4]; 16]);
        for step in 0..4000 {
            plain::formants4(&silence, &mut a, &mut fa);
            formants4_on(Path::Neon, &silence, &mut b, &mut fb);
            assert_eq!(row_bits(&a), row_bits(&b), "step {step}");
            assert_eq!(state_bits(&fa), state_bits(&fb), "step {step}");
        }
        // Some products flushed, and nothing grew.
        assert!(a.iter().flatten().any(|&x| x == 0.0), "{a:?}");
        let all = [fa.s1, fa.s2];
        let mut tiny = a.iter().flatten().chain(all.iter().flatten().flatten());
        assert!(tiny.all(|x| x.abs() < 1e-35), "{a:?} {fa:?}");
    }

    #[test]
    fn the_choir_workload_sounds_the_same_on_both_paths() {
        let (mut a, mut b) = (Formants::new(8, 44_100), Formants::new(8, 44_100));
        let mut buf = [[0.0f32; 128]; 4];
        for _ in 0..200 {
            let [la, ra, lb, rb] = &mut buf;
            assert_eq!(a.block(Path::Plain, la, ra), Path::Plain);
            assert_eq!(b.block(Path::Neon, lb, rb), Path::Neon);
            assert_eq!(bits(la), bits(lb));
            assert_eq!(bits(ra), bits(rb));
        }
    }

    #[test]
    fn the_wave_kernel_matches_its_twin_bit_for_bit() {
        let mut rng = Rng(0xba4c);
        for case in 0..2000 {
            let o = oscillators(&mut rng);
            let frames = rng.below(41) as usize;
            for shape in [Shape4::Saw, Shape4::Square, Shape4::Cubic] {
                let (mut wa, mut wb) =
                    (vec![[7.0f32; 4]; frames + 2], vec![[7.0f32; 4]; frames + 2]);
                let (mut fa, mut fb) = (vec![0xffu8; frames + 2], vec![0xffu8; frames + 2]);
                plain::waves4(shape, &o, &mut wa[..frames], &mut fa[..frames]);
                let took = waves4_on(Path::Neon, shape, &o, &mut wb[..frames], &mut fb);
                assert_eq!(took == Path::Neon, frames > 0, "case {case}");
                assert_eq!(
                    row_bits(&wa),
                    row_bits(&wb),
                    "case {case} {shape:?} frames {frames}"
                );
                assert_eq!(fa[..frames], fb[..frames], "case {case} {shape:?} flags");
                assert_eq!(fb[frames..], [0xff, 0xff], "case {case}: past the frames");
            }
        }
    }

    fn bus_bits(x: &[[f32; MIX_STEP]]) -> Vec<u32> {
        x.iter().flatten().map(|v| v.to_bits()).collect()
    }

    #[test]
    fn the_mix_kernel_matches_its_twin_bit_for_bit() {
        let mut rng = Rng(0x0b05);
        for case in 0..3000 {
            let buses = 1 + rng.below(8) as usize;
            let frames = rng.below(17) as usize;
            let waves = rows(&mut rng, frames, 1.5);
            let (mut amp, damp) = levels(&mut rng, buses);
            if case % 4 == 0 {
                // A skipped oscillator whose level is -0: put back as it was.
                amp[0][1] = -0.0;
            }
            let mut start: Vec<[f32; MIX_STEP]> = (0..buses)
                .map(|_| std::array::from_fn(|_| rng.unit() * 2.0))
                .collect();
            if case % 3 == 0 {
                // Buses at -0 that only skipped oscillators touch stay -0.
                start[buses - 1] = [-0.0; MIX_STEP];
                amp[buses - 1] = [0.0, -0.0, 0.0, 0.0];
                if case % 2 == 0 {
                    amp[buses - 1][2] = 0.5;
                }
            }
            let (mut a, mut aa) = (start.clone(), amp.clone());
            let (mut b, mut ba) = (start, amp);
            plain::mix4(&waves, &mut aa, &damp, &mut a);
            let took = mix4_on(Path::Neon, &waves, &mut ba, &damp, &mut b);
            assert_eq!(took == Path::Neon, frames >= 4, "case {case}");
            assert_eq!(
                bus_bits(&a),
                bus_bits(&b),
                "case {case}: {buses} buses, {frames} frames"
            );
            assert_eq!(row_bits(&aa), row_bits(&ba), "case {case}: levels");
        }
    }

    /// Levels fading to nothing: with flush-to-zero on, the paths flush
    /// the same products and levels.
    #[test]
    fn with_flush_to_zero_on_the_mix_kernel_matches_on_tiny_levels() {
        assert!(mpfree_lanes::fpu::flush_to_zero());
        let mut rng = Rng(5150);
        for case in 0..500 {
            let waves = rows(&mut rng, 16, 1.0);
            let (mut amp, mut damp) = levels(&mut rng, 6);
            for (a, d) in amp.iter_mut().zip(damp.iter_mut()) {
                for lane in 0..4 {
                    a[lane] *= 1e-37;
                    d[lane] *= 1e-38;
                }
            }
            let start = vec![[0.0f32; MIX_STEP]; 6];
            let (mut a, mut aa) = (start.clone(), amp.clone());
            let (mut b, mut ba) = (start, amp);
            plain::mix4(&waves, &mut aa, &damp, &mut a);
            mix4_on(Path::Neon, &waves, &mut ba, &damp, &mut b);
            assert_eq!(bus_bits(&a), bus_bits(&b), "case {case}");
            assert_eq!(row_bits(&aa), row_bits(&ba), "case {case}: levels");
        }
    }

    /// The GEMM and half kernels run where VFPv4 is (qemu-arm's processor
    /// has it, as the MPC One's Cortex-A17 does), so their tests below run
    /// the kernels.
    #[test]
    fn vfpv4_is_there_for_the_half_kernel() {
        assert!(crate::neon::half_available(), "no VFPv4 in AT_HWCAP");
        let src = [0x3c00u16; 8];
        let mut dst = [0.0f32; 8];
        assert_eq!(f16_to_f32_on(Path::Neon, &src, &mut dst), Path::Neon);
        assert_eq!(dst, [1.0; 8]);
    }

    #[test]
    fn the_gemm_kernel_matches_its_twin_bit_for_bit() {
        let mut rng = Rng(0x6e44);
        for case in 0..400 {
            let kc = match case % 8 {
                0 => 0,
                1 => 1,
                2 => 2,
                3 => 7,
                4 => 64,
                5 => 256,
                _ => rng.below(600) as usize,
            };
            let (a, b, start) = gemm_case(&mut rng, kc);
            let mut want = start;
            plain::gemm4x8(kc, &a, &b, &mut want);
            let mut got = start;
            assert_eq!(gemm4x8_on(Path::Neon, kc, &a, &b, &mut got), Path::Neon);
            assert_eq!(bits(&got), bits(&want), "case {case}: kc {kc}");
        }
    }

    /// Weights the size real ones are (about ±0.3) and big sums: no product
    /// near the subnormal range, where NEON would flush and the twin not.
    #[test]
    fn the_gemm_kernel_matches_on_a_long_real_sized_sum() {
        let mut rng = Rng(77);
        let kc = 25 * 512; // a 5 × 5 layer's 512 channels
        let a: Vec<f32> = (0..kc * 4).map(|_| rng.unit() * 0.3).collect();
        let b: Vec<f32> = (0..kc * 8).map(|_| rng.unit() * 4.0).collect();
        let (mut want, mut got) = ([0.0f32; 32], [0.0f32; 32]);
        plain::gemm4x8(kc, &a, &b, &mut want);
        gemm4x8_on(Path::Neon, kc, &a, &b, &mut got);
        assert_eq!(bits(&got), bits(&want));
    }

    /// All 65,536 halves, at every start within a group of eight and
    /// lengths that leave a tail for the twin.
    #[test]
    fn the_half_kernel_matches_its_twin_on_every_half() {
        let all: Vec<u16> = (0..=u16::MAX).collect();
        for skip in 0..8 {
            for tail in [0, 1, 7] {
                let src = &all[skip..all.len() - tail];
                let mut want = vec![0.0f32; src.len()];
                plain::f16_to_f32(src, &mut want);
                let mut got = vec![0.0f32; src.len()];
                assert_eq!(f16_to_f32_on(Path::Neon, src, &mut got), Path::Neon);
                assert_eq!(bits(&got), bits(&want), "skip {skip} tail {tail}");
            }
        }
    }
}

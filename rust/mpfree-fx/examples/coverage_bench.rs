//! Times the effects added for MPC coverage on 128-frame stereo blocks, as
//! `fx_bench` does for the older ones, each at its defaults plus the
//! settings that cost the most (true peak, three heads, Lead gain).
//! `FX_BENCH_ONLY` keeps the run of that exact name (or the runs whose
//! name contains it) and
//! `FX_BENCH_BLOCKS` sets the block count, so the ARMv7 instruction count
//! in docs/dev/design/fx.md works the same way: build for
//! `armv7-unknown-linux-gnueabihf`, run under `qemu-arm` with
//! `-one-insn-per-tb -d exec,nochain`, count the `Trace` lines with 10
//! blocks and with 0, and divide the difference by 10.

use std::time::Instant;

use mpfree_fx::mpfree_node::{Ctx, MAX_BLOCK};
use mpfree_fx::{ids, registry};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

/// A run: its name, the effect's id and the settings that differ.
type Run = (&'static str, &'static str, &'static [(u16, f32)]);

fn main() {
    let only = std::env::var("FX_BENCH_ONLY").ok();
    let blocks: usize = std::env::var("FX_BENCH_BLOCKS")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(20 * RATE as usize / BLOCK);
    let runs: &[Run] = &[
        ("Tremolo", "mpfree.fx.tremolo", &[]),
        (
            "Tremolo (Harmonic)",
            "mpfree.fx.tremolo",
            &[(ids::tremolo::MODE, 1.0)],
        ),
        ("Auto-pan", "mpfree.fx.autopan", &[]),
        ("Ring mod", "mpfree.fx.ringmod", &[]),
        (
            "Ring mod (square, sweep)",
            "mpfree.fx.ringmod",
            &[(ids::ringmod::WAVE, 2.0), (ids::ringmod::SWEEP, 2.0)],
        ),
        ("Rotary", "mpfree.fx.rotary", &[(ids::rotary::DRIVE, 40.0)]),
        (
            "Limiter",
            "mpfree.fx.limiter",
            &[(ids::limiter::GAIN, 12.0)],
        ),
        (
            "Limiter (true peak)",
            "mpfree.fx.limiter",
            &[(ids::limiter::GAIN, 12.0), (ids::limiter::TRUE_PEAK, 1.0)],
        ),
        ("Gate", "mpfree.fx.gate", &[]),
        (
            "Gate (key filters)",
            "mpfree.fx.gate",
            &[(ids::gate::KEY_HP, 100.0), (ids::gate::KEY_LP, 5000.0)],
        ),
        ("De-esser", "mpfree.fx.deesser", &[]),
        ("Multiband comp", "mpfree.fx.multiband", &[]),
        ("Tape delay", "mpfree.fx.tape_delay", &[]),
        (
            "Tape delay (three heads)",
            "mpfree.fx.tape_delay",
            &[(ids::tape::HEADS, 2.0)],
        ),
        ("Multi-tap delay", "mpfree.fx.multitap", &[]),
        ("Room reverb", "mpfree.fx.room", &[]),
        ("Plate reverb", "mpfree.fx.plate", &[]),
        ("Spring reverb", "mpfree.fx.spring", &[]),
        ("Amp (Crunch, cab)", "mpfree.fx.amp", &[]),
        (
            "Amp (Lead, cab)",
            "mpfree.fx.amp",
            &[(ids::amp::MODEL, 2.0), (ids::amp::GAIN, 8.0)],
        ),
        ("Exciter", "mpfree.fx.exciter", &[]),
        ("XY FX (Filter)", "mpfree.fx.xy", &[]),
        ("XY FX (Echo)", "mpfree.fx.xy", &[(ids::xyfx::EFFECT, 1.0)]),
        ("XY FX (Wash)", "mpfree.fx.xy", &[(ids::xyfx::EFFECT, 3.0)]),
        (
            "XY FX (closed)",
            "mpfree.fx.xy",
            &[(ids::xyfx::ACTIVE, 0.0)],
        ),
        ("Auto wah", "mpfree.fx.autowah", &[]),
        (
            "Talk box",
            "mpfree.fx.talkbox",
            &[(ids::talkbox::LFO_DEPTH, 60.0)],
        ),
        ("Freq shifter", "mpfree.fx.freqshift", &[]),
        (
            "Freq shifter (Stereo, feedback)",
            "mpfree.fx.freqshift",
            &[
                (ids::freqshift::MODE, 3.0),
                (ids::freqshift::FEEDBACK, 60.0),
            ],
        ),
        (
            "Stereo width",
            "mpfree.fx.width",
            &[(ids::width::DELAY, 4.0)],
        ),
        (
            "Sample delay",
            "mpfree.fx.sample_delay",
            &[(ids::sample_delay::LEFT, 300.0)],
        ),
        ("Channel strip", "mpfree.fx.channel_strip", &[]),
        (
            "Channel strip (gate on)",
            "mpfree.fx.channel_strip",
            &[(ids::channel_strip::GATE, 1.0)],
        ),
        ("Granulator", "mpfree.fx.granulator", &[]),
        (
            "Granulator (dense)",
            "mpfree.fx.granulator",
            &[
                (ids::granulator::SIZE, 500.0),
                (ids::granulator::DENSITY, 80.0),
            ],
        ),
    ];
    println!("Coverage effects, one stereo instance on {BLOCK}-frame blocks at {RATE} Hz:");
    for &(name, id, set) in runs {
        // A name that matches a run exactly keeps only that run, so
        // "Tremolo" can be counted without "Tremolo (Harmonic)".
        let skip = only.as_deref().is_some_and(|f| {
            if runs.iter().any(|r| r.0 == f) {
                name != f
            } else {
                !name.contains(f)
            }
        });
        if skip {
            continue;
        }
        let mut n = registry::make(id).expect(id);
        n.prepare(RATE, MAX_BLOCK);
        for &(p, v) in set {
            n.set_param(p, v);
        }
        n.reset();
        let mut l = vec![0.0f32; BLOCK];
        let mut r = vec![0.0f32; BLOCK];
        let mut seed = 1u32;
        let start = Instant::now();
        for b in 0..blocks {
            for i in 0..BLOCK {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = (seed >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
                l[i] = 0.5 * x;
                r[i] = -0.4 * x;
            }
            n.process(&Ctx::new(RATE, (b * BLOCK) as u64), &[], &mut l, &mut r);
        }
        let secs = start.elapsed().as_secs_f64();
        println!(
            "  {name:<28} {:>6.2} % of real time ({:.2} us per block)",
            100.0 * secs / (blocks * BLOCK) as f64 * RATE as f64,
            1e6 * secs / blocks.max(1) as f64
        );
        std::hint::black_box(&l);
    }
}

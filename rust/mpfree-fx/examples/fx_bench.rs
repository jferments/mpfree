//! Times each effect on 128-frame stereo blocks and prints its share of
//! real time at 48 kHz. Run with `--release`. On the build host this is a
//! desktop core; the MPC One's Cortex-A17 needs its own run before slot
//! limits are set.

use std::time::Instant;

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::{
    Chain, Character, Chorus, Cloud, Compressor, Crush, Delay, DjFilter, Drive, Ducker, Eq,
    Flanger, LoFi, MasterSafety, MultiFilter, Phaser, PitchShifter, TransientShaper,
};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const SECONDS: usize = 20;

type Process<'a> = dyn FnMut(&Ctx, &mut [f32], &mut [f32]) + 'a;

fn time(name: &str, process: &mut Process) {
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let blocks = SECONDS * RATE as usize / BLOCK;
    let mut seed = 1u32;
    let start = Instant::now();
    for b in 0..blocks {
        for i in 0..BLOCK {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (seed >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
            l[i] = 0.5 * n;
            r[i] = -0.4 * n;
        }
        process(&Ctx::new(RATE, (b * BLOCK) as u64), &mut l, &mut r);
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "  {name:<28} {:>6.2} % of real time ({:.2} us per block)",
        100.0 * secs / SECONDS as f64,
        1e6 * secs / blocks as f64
    );
    std::hint::black_box(&l);
}

fn node(name: &str, mut n: Box<dyn Node>, params: &[(u16, f32)]) {
    n.prepare(RATE, MAX_BLOCK);
    for &(id, v) in params {
        n.set_param(id, v);
    }
    n.reset();
    time(name, &mut |ctx, l, r| n.process(ctx, &[], l, r));
}

fn main() {
    println!("mpfree-fx, one stereo instance on {BLOCK}-frame blocks at {RATE} Hz:");
    node(
        "Multi filter (LP 12)",
        Box::new(MultiFilter::new()),
        &[(ids::filter::CUTOFF, 800.0)],
    );
    node(
        "Multi filter (LP 24, drive)",
        Box::new(MultiFilter::new()),
        &[
            (ids::filter::CUTOFF, 800.0),
            (ids::filter::SLOPE, 1.0),
            (ids::filter::DRIVE, 12.0),
        ],
    );
    node(
        "DJ filter",
        Box::new(DjFilter::new()),
        &[(ids::dj_filter::POSITION, -40.0)],
    );
    node("Drive (Eco 2x)", Box::new(Drive::new()), &[]);
    node(
        "Drive (HQ 4x)",
        Box::new(Drive::new()),
        &[(ids::drive::QUALITY, 1.0)],
    );
    node("Crush", Box::new(Crush::new()), &[]);
    for (name, mode) in [
        ("Character (Console)", ids::character::CONSOLE),
        ("Character (Tape)", ids::character::TAPE),
        ("Character (Iron)", ids::character::IRON),
        ("Character (Soften)", ids::character::SOFTEN),
    ] {
        node(
            name,
            Box::new(Character::new()),
            &[(ids::character::MODE, mode), (ids::character::AMOUNT, 70.0)],
        );
    }
    node("Delay", Box::new(Delay::new()), &[]);
    node("Compressor", Box::new(Compressor::new()), &[]);
    node(
        "Ducker (tempo)",
        Box::new(Ducker::new()),
        &[(ids::ducker::SOURCE, 1.0)],
    );
    node("Transient shaper", Box::new(TransientShaper::new()), &[]);
    node("Pitch shifter", Box::new(PitchShifter::new()), &[]);
    node(
        "EQ (4 bands and both cuts)",
        Box::new(Eq::new()),
        &[
            (ids::eq::LOW_SLOPE, 2.0),
            (ids::eq::HIGH_SLOPE, 2.0),
            (ids::eq::gain(0), 3.0),
            (ids::eq::gain(1), -4.0),
            (ids::eq::gain(2), 5.0),
            (ids::eq::gain(3), 2.0),
        ],
    );
    node("Chorus (2 voices)", Box::new(Chorus::new()), &[]);
    node(
        "Chorus (4 voices)",
        Box::new(Chorus::new()),
        &[(ids::chorus::VOICES, 4.0)],
    );
    node("Flanger", Box::new(Flanger::new()), &[]);
    node("Phaser (4 stages)", Box::new(Phaser::new()), &[]);
    node(
        "Phaser (12 stages)",
        Box::new(Phaser::new()),
        &[(ids::phaser::STAGES, 4.0)],
    );
    node(
        "Lo-fi",
        Box::new(LoFi::new()),
        &[(ids::lofi::AMOUNT, 100.0)],
    );
    node("Master safety", Box::new(MasterSafety::new()), &[]);
    for (k, name) in ["Plate", "Room", "Hall", "Cloud", "Echoes"]
        .iter()
        .enumerate()
    {
        for (q, quality) in ["", ", Light", ", Eco"].iter().enumerate() {
            node(
                &format!("Cloud reverb ({name}{quality})"),
                Box::new(Cloud::new()),
                &[
                    (ids::cloud::CHARACTER, k as f32),
                    (ids::cloud::MIX, 100.0),
                    (ids::cloud::QUALITY, q as f32),
                ],
            );
        }
    }
    let mut chain = Chain::new();
    chain.prepare(RATE, MAX_BLOCK);
    chain.push(Box::new(Drive::new()));
    chain.push(Box::new(MultiFilter::new()));
    chain.push(Box::new(Delay::new()));
    chain.push(Box::new(Compressor::new()));
    chain.set_param(1, ids::filter::CUTOFF, 800.0);
    time("Chain: drive, filter, delay, comp", &mut |ctx, l, r| {
        chain.process(ctx, l, r)
    });
}

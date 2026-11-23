//! `--compose`: what the composer (`mpfree-compose`, phase 0C) costs a
//! request, so the MPC timings the design asks for can be measured on the
//! device. The composer never runs on the audio thread: it is a
//! background job, and these are whole requests, not blocks.
//!
//! Each job runs many times with a different seed each time (as many
//! Try agains would), on the second prototype's two-bar C minor riff, and
//! prints its median, p95, p99 and worst.

use mpfree_compose::{
    continue_clip, continue_notes, try_bar, Continue, InstrumentKind, OnClip, PlainNote, Settings,
    Source, GROOVE, JAZZISH,
};
use mpfree_context::groove::{Lane, STEP};
use mpfree_context::listen::{listen, Played};
use mpfree_context::{MusicalContext, PartRole, ReadBudget};
use mpfree_prof::model::Model;
use mpfree_prof::Span;
use mpfree_seq::{Clip, Holds, Note, Project, Track};
use mpfree_transport::time::{Meter, Swing, Tick, PPQ};

const BAR: Tick = 4 * PPQ as Tick;

fn riff() -> Vec<(PlainNote, Source)> {
    [
        (0, 2, 60, 100),
        (2, 1, 63, 88),
        (3, 1, 67, 92),
        (4, 2, 65, 96),
        (6, 2, 63, 86),
        (8, 3, 62, 94),
        (11, 1, 63, 80),
        (12, 4, 60, 90),
        (16, 2, 67, 100),
        (18, 1, 68, 86),
        (19, 1, 67, 84),
        (20, 2, 65, 94),
        (22, 2, 63, 84),
        (24, 2, 62, 92),
        (26, 2, 63, 84),
        (28, 4, 65, 88),
    ]
    .iter()
    .map(|&(on, len, pitch, velocity)| {
        (
            PlainNote {
                at: on * STEP,
                len: len * STEP,
                pitch,
                velocity,
            },
            Source::You,
        )
    })
    .collect()
}

fn played(notes: &[(PlainNote, Source)]) -> Vec<Played> {
    notes
        .iter()
        .map(|(n, _)| Played::new(n.at, n.len, n.pitch, n.velocity))
        .collect()
}

/// 16 bars of 128 notes each (2,048, the reading budget): chords of
/// eight notes on every 1/16 of a C minor vamp.
fn dense() -> Vec<Played> {
    let shapes = [
        [48, 55, 60, 63, 67, 70, 72, 75],
        [53, 60, 65, 68, 72, 75, 77, 80],
    ];
    (0..16 * 16)
        .flat_map(|step: Tick| {
            let shape = shapes[((step / 32) % 2) as usize];
            shape.map(move |p| Played::new(step * STEP, STEP, p, 90))
        })
        .collect()
}

fn context(notes: &[(PlainNote, Source)]) -> MusicalContext {
    let r = listen(
        &played(notes),
        2 * BAR,
        ReadBudget::default(),
        Meter::default(),
    );
    MusicalContext {
        key: r.key,
        chords: r.chords,
        groove: None,
        played_pcs: r.played_pcs,
        motifs: Vec::new(),
        read: Vec::new(),
        cut_short: r.cut_short,
    }
}

fn no_lanes(_: usize, _: u8) -> Option<Lane> {
    None
}

/// A Continue request on the riff.
fn ask<'a>(
    notes: &'a [(PlainNote, Source)],
    context: &'a MusicalContext,
    settings: &'a Settings,
) -> Continue<'a> {
    Continue {
        notes,
        length: 2 * BAR,
        context,
        settings,
        role: PartRole::Melody,
        kind: InstrumentKind::OneNote,
        meter: Meter::default(),
        bpm: 120.0,
        swing: Swing::default(),
        loops: true,
    }
}

/// Median, p95, p99 and worst of `ns`, in µs.
fn spread(mut ns: Vec<u64>) -> [f64; 4] {
    ns.sort_unstable();
    let at = |q: f64| ns[((ns.len() - 1) as f64 * q).round() as usize] as f64 / 1e3;
    [at(0.5), at(0.95), at(0.99), at(1.0)]
}

/// Run `job` `runs` times with seeds 0, 1, 2 ...; its time each run.
fn time(runs: u64, mut job: impl FnMut(u64)) -> [f64; 4] {
    // A few runs first, so the first pass's page faults aren't counted.
    for seed in 0..5 {
        job(1_000_000 + seed);
    }
    spread(
        (0..runs)
            .map(|seed| {
                let span = Span::start();
                job(seed);
                span.elapsed_ns()
            })
            .collect(),
    )
}

pub fn run(quick: bool, model: Option<&Model>, on_device: bool, machine: &str, build: &str) {
    let runs = if quick { 200 } else { 2000 };
    let factor = if on_device {
        1.0
    } else {
        model.map_or(1.0, |m| m.factor().0)
    };
    let place = if on_device {
        "on this MPC"
    } else if model.is_some() {
        "estimated for the MPC"
    } else {
        "on this machine"
    };
    println!("mpfree-bench --compose on {machine}, build {build}");
    println!(
        "Whole composer requests, {runs} runs each with a different seed. Times are {place}{}.",
        if model.is_some() && !on_device {
            format!(" (this machine x {factor:.1})")
        } else {
            String::new()
        }
    );
    println!();
    println!(
        "{:<52} {:>9} {:>9} {:>9} {:>9}",
        "job", "p50 us", "p95 us", "p99 us", "worst us"
    );
    let notes = riff();
    let ctx = context(&notes);
    let settings = |style, bars, seed| Settings {
        style,
        bars,
        seed,
        ..Settings::default()
    };
    let mut rows: Vec<(&str, [f64; 4])> = Vec::new();

    let riff_notes = played(&notes);
    rows.push((
        "read 2 bars (key and chords, 16 notes)",
        time(runs, |_| {
            std::hint::black_box(listen(
                &riff_notes,
                2 * BAR,
                ReadBudget::default(),
                Meter::default(),
            ));
        }),
    ));
    let full = dense();
    rows.push((
        "read 16 bars (2,048 notes, the whole budget)",
        time(runs / 10, |_| {
            std::hint::black_box(listen(
                &full,
                16 * BAR,
                ReadBudget::default(),
                Meter::default(),
            ));
        }),
    ));
    rows.push((
        "Continue 2 -> 10 bars, Groove",
        time(runs, |seed| {
            let s = settings(GROOVE, 8, seed);
            std::hint::black_box(continue_notes(&ask(&notes, &ctx, &s)).ok());
        }),
    ));
    rows.push((
        "Continue 2 -> 18 bars, Jazz-ish",
        time(runs, |seed| {
            let s = settings(JAZZISH, 16, seed);
            std::hint::black_box(continue_notes(&ask(&notes, &ctx, &s)).ok());
        }),
    ));
    let s16 = settings(JAZZISH, 16, 16);
    let made = continue_notes(&ask(&notes, &ctx, &s16)).expect("the riff carries on");
    rows.push((
        "Try again on one bar of 18",
        time(runs, |seed| {
            std::hint::black_box(
                try_bar(
                    &ask(&notes, &ctx, &s16),
                    &made,
                    9,
                    1 + (seed % 60_000) as u16,
                )
                .ok(),
            );
        }),
    ));
    let mut project = Project::new();
    let mut track = Track::new("Lead", 0);
    let mut clip = Clip::new("riff", 2 * BAR);
    clip.holds = Holds::Notes;
    clip.notes = notes
        .iter()
        .map(|(n, _)| Note::new(n.at, n.len, n.pitch, n.velocity))
        .collect();
    track.slots[0] = Some(clip);
    project.tracks.push(track);
    project.give_clip_ids();
    rows.push((
        "Continue a clip in a project, 2 -> 6 bars (read too)",
        time(runs, |seed| {
            let on = OnClip {
                project: &project,
                track: 0,
                slot: 0,
                role: PartRole::Melody,
                others: &[],
                lane_of: &no_lanes,
                transport_swing: Swing::default(),
                meter: Meter::default(),
                bpm: 120.0,
                budget: ReadBudget::default(),
            };
            let s = settings(GROOVE, 4, seed);
            std::hint::black_box(continue_clip(&on, &s, InstrumentKind::OneNote).ok());
        }),
    ));
    for (name, t) in rows {
        println!(
            "{name:<52} {:>9.0} {:>9.0} {:>9.0} {:>9.0}",
            t[0] * factor,
            t[1] * factor,
            t[2] * factor,
            t[3] * factor
        );
    }
    println!();
    println!(
        "The composer runs as a background job at normal priority, never on the audio thread; a request's worst is \
         what a press of Continue or Try again waits for."
    );
}

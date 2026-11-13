//! Which parameters go on which page of an effect, and what the Q-Links
//! turn there.
//!
//! Each native effect names its pages and picks its four Q-Links: what a
//! player reaches for while the music plays (the delay's feedback and mix,
//! the reverb's decay), not whatever comes first in the parameter list.
//! Any other node (a hosted plugin, the sampler's grains) gets the Synth
//! tab's pages from its parameter groups.

use mpfree_fx::ids::{
    character, chorus, cloud, comb, compressor as comp, crush, delay, dj_filter as dj, drive,
    ducker, eq, filter, flanger, lofi, master, perform, phaser, pitch, transient,
};
use mpfree_node::{NodeInfo, ParamInfo};
pub use mpfree_synth_ui::pages::Page;

/// A page: its name, its parameters and its Q-Links, by id.
struct Plan {
    name: String,
    params: Vec<u16>,
    qlinks: Vec<u16>,
}

fn pg(name: &str, params: &[u16], qlinks: &[u16]) -> Plan {
    Plan {
        name: name.to_string(),
        params: params.to_vec(),
        qlinks: qlinks.to_vec(),
    }
}

/// Every id from 0 to `n - 1`.
fn all(n: u16) -> Vec<u16> {
    (0..n).collect()
}

fn known(info: &NodeInfo, count: u16) -> Option<Vec<Plan>> {
    let one = |q: &[u16]| Some(vec![pg(info.name, &all(count), q)]);
    match info.id {
        "mpfree.fx.eq" => {
            let bands = |f: fn(usize) -> u16| (0..eq::BANDS).map(f).collect::<Vec<_>>();
            // Four columns, one per band: frequency, gain, Q, then type.
            let mut p = bands(eq::freq);
            p.extend(bands(eq::gain));
            p.extend(bands(eq::q));
            p.extend(bands(eq::band_type));
            Some(vec![
                pg("Bands", &p, &bands(eq::gain)),
                pg(
                    "Cuts",
                    &[
                        eq::LOW_CUT,
                        eq::LOW_SLOPE,
                        eq::HIGH_CUT,
                        eq::HIGH_SLOPE,
                        eq::OUTPUT,
                    ],
                    &[eq::LOW_CUT, eq::HIGH_CUT, eq::OUTPUT],
                ),
            ])
        }
        "mpfree.fx.filter" => one(&[
            filter::CUTOFF,
            filter::RESONANCE,
            filter::GAIN,
            filter::DRIVE,
        ]),
        "mpfree.fx.dj_filter" => one(&[dj::POSITION, dj::RESONANCE]),
        "mpfree.fx.drive" => one(&[drive::DRIVE, drive::TONE, drive::BIAS, drive::MIX]),
        "mpfree.fx.comb" => one(&[comb::PITCH, comb::FEEDBACK, comb::DAMPING, comb::MIX]),
        "mpfree.fx.crush" => one(&[crush::BITS, crush::RATE, crush::MIX]),
        // Q-Link 1 turns Sync instead of Time while the echo is synced
        // (see `qlinks`).
        "mpfree.fx.delay" => one(&[delay::TIME, delay::FEEDBACK, delay::TONE, delay::MIX]),
        "mpfree.fx.chorus" => one(&[chorus::RATE, chorus::DEPTH, chorus::DELAY, chorus::MIX]),
        "mpfree.fx.flanger" => one(&[
            flanger::RATE,
            flanger::DEPTH,
            flanger::FEEDBACK,
            flanger::MIX,
        ]),
        "mpfree.fx.phaser" => one(&[
            phaser::RATE,
            phaser::DEPTH,
            phaser::CENTER,
            phaser::FEEDBACK,
        ]),
        "mpfree.fx.lofi" => one(&[lofi::AMOUNT, lofi::WOW, lofi::CRACKLE, lofi::MIX]),
        "mpfree.fx.character" => one(&[character::AMOUNT, character::OUTPUT, character::MIX]),
        "mpfree.fx.compressor" => one(&[comp::THRESHOLD, comp::RATIO, comp::ATTACK, comp::MAKEUP]),
        "mpfree.fx.master_safety" => one(&[master::TRIM, master::CEILING, master::RELEASE]),
        "mpfree.fx.ducker" => one(&[
            ducker::DEPTH,
            ducker::THRESHOLD,
            ducker::RELEASE,
            ducker::SHAPE,
        ]),
        "mpfree.fx.transient" => one(&[
            transient::ATTACK,
            transient::SUSTAIN,
            transient::SPEED,
            transient::OUTPUT,
        ]),
        "mpfree.fx.pitch" => one(&[pitch::PITCH, pitch::FINE, pitch::FEEDBACK, pitch::MIX]),
        "mpfree.fx.cloud" => Some(vec![
            pg(
                "Reverb",
                &[
                    cloud::CHARACTER,
                    cloud::QUALITY,
                    cloud::SIZE,
                    cloud::DECAY,
                    cloud::PRE_DELAY,
                    cloud::DIFFUSION,
                    cloud::MODULATION,
                    cloud::MOD_RATE,
                ],
                &[
                    cloud::DECAY,
                    cloud::SIZE,
                    cloud::PRE_DELAY,
                    cloud::DIFFUSION,
                ],
            ),
            pg(
                "Tone",
                &[
                    cloud::DAMPING,
                    cloud::LOW_CUT,
                    cloud::HIGH_CUT,
                    cloud::WIDTH,
                    cloud::EARLY,
                    cloud::MIX,
                    cloud::FREEZE,
                ],
                &[cloud::DAMPING, cloud::HIGH_CUT, cloud::EARLY, cloud::MIX],
            ),
        ]),
        "mpfree.fx.perform" => {
            // The Perform page has the pads and the XY pad; these pages are
            // the same settings as tiles, four pads to a page.
            let mut pages = vec![pg(
                "Perform",
                &[
                    perform::QUANTISE,
                    perform::FADE,
                    perform::DJ_FILTER,
                    perform::ECHO,
                    perform::ECHO_TIME,
                    perform::MIX,
                ],
                &[perform::DJ_FILTER, perform::ECHO, perform::FADE],
            )];
            for b in 0..4 {
                let ids: Vec<u16> = (4 * b..4 * b + 4)
                    .flat_map(|k| (0..4).map(move |o| perform::slot(k) + o))
                    .collect();
                let amounts: Vec<u16> = (4 * b..4 * b + 4)
                    .map(|k| perform::slot(k) + perform::AMOUNT)
                    .collect();
                pages.push(pg(
                    &format!("Pads {}-{}", 4 * b + 1, 4 * b + 4),
                    &ids,
                    &amounts,
                ));
            }
            Some(pages)
        }
        _ => None,
    }
}

/// Whether the effect names its own pages (rather than getting them from
/// its parameter groups).
pub fn named(info: &NodeInfo) -> bool {
    known(info, 0).is_some()
}

/// The pages for a node: its own when it names them, else from its groups.
pub fn plan(info: &NodeInfo, params: &[ParamInfo]) -> Vec<Page> {
    let index = |id: u16| params.iter().position(|p| p.id == id);
    match known(info, params.len() as u16) {
        Some(plans) => plans
            .into_iter()
            .map(|p| Page {
                name: p.name,
                params: p.params.iter().filter_map(|&id| index(id)).collect(),
                qlinks: p.qlinks.iter().filter_map(|&id| index(id)).collect(),
            })
            .filter(|p| !p.params.is_empty())
            .collect(),
        None => mpfree_synth_ui::pages::plan(info, params),
    }
}

/// What Q-Links 1 to 4 turn on `page` (indexes into the parameters), with
/// the values now: the page's choice, except that the delay's Q-Link 1
/// turns Sync while the echo is synced (Time does nothing then).
pub fn qlinks(info: &NodeInfo, page: &Page, params: &[ParamInfo], values: &[f32]) -> Vec<usize> {
    let mut q = mpfree_synth_ui::pages::qlink_targets(page, params);
    if info.id == "mpfree.fx.delay" {
        let at = |id: u16| params.iter().position(|p| p.id == id);
        if let (Some(sync), Some(time)) = (at(delay::SYNC), at(delay::TIME)) {
            if values.get(sync).is_some_and(|&v| v >= 1.0) {
                for k in &mut q {
                    if *k == time {
                        *k = sync;
                    }
                }
            }
        }
    }
    q
}

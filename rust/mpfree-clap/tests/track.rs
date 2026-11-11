//! A CLAP plugin where MPFree's own engines go: the test synth as a track
//! host instrument (notes in, ring-out on a swap, dropped off the audio
//! thread), and the test gain as an insert in a mixer strip's chain.

use mpfree_clap::testplug::{self, gain_params, GAIN_ID, SYNTH_ID};
use mpfree_clap::Library;
use mpfree_fx::Chain;
use mpfree_node::{Ctx, Node};
use mpfree_track::{Instrument, Sum, TrackHost};

const RATE: u32 = 44_100;

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

#[test]
fn a_clap_synth_plays_on_a_track_and_swaps_out_cleanly() {
    let lib = Library::from_entry(&testplug::ENTRY, "linked").expect("entry");
    let synth = lib.create(SYNTH_ID).expect("synth");
    let mut host = TrackHost::new(2, RATE, 256);
    host.swap(0, Some(Instrument::new(Box::new(synth), RATE, 256))).expect("slot 0");
    assert_eq!(host.instrument(0).map(|i| i.node().info().id), Some("clap:org.mpfree.test-synth"));
    assert_eq!(host.params(0).len(), 5);
    host.note_on(0, 3, 69, 120);
    let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
    let mut frame = 0u64;
    let mut loud = 0f32;
    for _ in 0..8 {
        l.fill(0.0);
        r.fill(0.0);
        host.render(&Ctx::new(RATE, frame), 256, &mut Sum { left: &mut l, right: &mut r });
        loud = loud.max(peak(&l));
        frame += 256;
    }
    assert!(loud > 0.01, "the plugin sounds on its track");
    // A parameter from the host (the mod matrix's path) reaches the plugin.
    host.param(0, 0, 1, 50.0);
    host.render(&Ctx::new(RATE, frame), 256, &mut Sum { left: &mut l, right: &mut r });
    frame += 256;
    assert_eq!(host.param_value(0, 1), Some(50.0));
    // Swapped out: it gets AllOff, rings out, and comes back to be dropped.
    host.swap(0, None).expect("swap out");
    let mut back = None;
    for _ in 0..400 {
        l.fill(0.0);
        r.fill(0.0);
        host.render(&Ctx::new(RATE, frame), 256, &mut Sum { left: &mut l, right: &mut r });
        frame += 256;
        if let Some(old) = host.pop_retired() {
            back = Some(old);
            break;
        }
    }
    let old = back.expect("the plugin came back after its ring-out");
    assert_eq!(old.node().info().id, "clap:org.mpfree.test-synth");
    // Dropped here, on the control thread: deactivated, then destroyed.
    drop(old);
}

#[test]
fn a_clap_effect_works_as_a_mixer_insert() {
    let lib = Library::from_entry(&testplug::ENTRY, "linked").expect("entry");
    let mut gain = lib.create(GAIN_ID).expect("gain");
    let id = gain.param_index(gain_params::GAIN).expect("gain param");
    gain.set_param(id, -20.0 * 2f32.log10());
    let mut chain = Chain::new();
    chain.prepare(RATE, 256);
    chain.push(Box::new(gain));
    let (mut l, mut r) = (vec![0.5f32; 256], vec![-0.5f32; 256]);
    // The chain fades a new insert in; after that the gain halves.
    for b in 0..40u64 {
        l.fill(0.5);
        r.fill(-0.5);
        chain.process(&Ctx::new(RATE, b * 256), &mut l, &mut r);
    }
    assert!(l.iter().all(|&x| (x - 0.25).abs() < 1e-4), "{}", l[0]);
    assert!(r.iter().all(|&x| (x + 0.25).abs() < 1e-4));
}

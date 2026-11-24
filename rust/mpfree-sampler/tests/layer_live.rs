//! A layer's grain settings live and locked in the engine (sampler
//! refactor step 17b): `Event::LayerLive` and `Event::LayerLock` play as
//! an edit of the Grain tool would, new voices and playing ones alike,
//! each layer on its own; and the kit's master effect's settings live
//! (`Event::MasterLive`).

use mpfree_sampler::engine::{Engine, EngineView, Event, GrainField, MasterField, LAYER_VALUES};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const PAD: usize = 3;

fn tone(name: &str, step: f32) -> Source {
    Source::new(
        name,
        RATE,
        1,
        (0..22_050).map(|i| (i as f32 * step).sin() * 0.5).collect(),
    )
    .unwrap()
}

fn grain() -> GrainParams {
    GrainParams {
        size_ms: 60.0,
        density: 40.0,
        spray: 0.2,
        scan: 1.0,
        ..GrainParams::default()
    }
}

/// A project with pad `PAD` of two granular layers, the second's grain
/// settings `second`, the first tuned up a fifth (so its live values go
/// in under the tune).
fn project(first: GrainParams, second: GrainParams) -> Project {
    let mut p = Project::new();
    let a = p.add_source(tone("a", 0.05));
    let b = p.add_source(tone("b", 0.031));
    let layer = |s: SourceId, g: GrainParams, tune: i32| {
        let mut r = Region::whole(p.source(s).unwrap());
        r.mode = PlayMode::Granular;
        r.grain = g;
        r.tune_cents = tune;
        Layer::new(r)
    };
    let mut pad = Pad::new("clouds", layer(a, first, 700));
    pad.layers.push(layer(b, second, 0));
    p.set_pad(PAD, Some(pad), None).unwrap();
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(cl, cr);
    }
    l.extend(r);
    l
}

fn hit(e: &mut Engine) {
    e.apply(Event::NoteOn {
        pad: PAD as u8,
        velocity: 100,
    });
}

fn with(f: GrainField, v: f32) -> GrainParams {
    let mut g = grain();
    f.set(&mut g, v);
    g
}

fn live(layer: u8, field: GrainField, value: f32) -> Event {
    Event::LayerLive {
        pad: PAD as u8,
        layer,
        field,
        value,
    }
}

fn lock(layer: u8, field: GrainField, value: f32) -> Event {
    Event::LayerLock {
        pad: PAD as u8,
        layer,
        field,
        value,
    }
}

#[test]
fn a_hit_plays_a_layers_live_grain_setting_as_its_own() {
    let mut e = engine(&project(grain(), grain()));
    e.apply(live(0, GrainField::Size, 200.0));
    e.apply(live(1, GrainField::Density, 9.0));
    hit(&mut e);
    let heard = run(&mut e, 8_192);
    let mut edited = engine(&project(
        with(GrainField::Size, 200.0),
        with(GrainField::Density, 9.0),
    ));
    hit(&mut edited);
    assert_eq!(
        heard,
        run(&mut edited, 8_192),
        "as if the layers were edited"
    );
    let mut plain = engine(&project(grain(), grain()));
    hit(&mut plain);
    assert_ne!(heard, run(&mut plain, 8_192), "and not as their own");
}

#[test]
fn a_lock_with_no_note_moves_the_cloud_still_playing_as_an_edit_does() {
    let p = project(grain(), grain());
    let mut e = engine(&p);
    hit(&mut e);
    let mut heard = run(&mut e, 4_096);
    e.apply(lock(1, GrainField::Size, 250.0));
    heard.extend(run(&mut e, 8_192));

    let mut edited = engine(&p);
    hit(&mut edited);
    let mut want = run(&mut edited, 4_096);
    edited.edit_view(EngineView::build(
        &project(grain(), with(GrainField::Size, 250.0)),
        RATE,
    ));
    want.extend(run(&mut edited, 8_192));
    assert_eq!(heard, want);
    assert_eq!(
        e.kit_layer_lock_value(0, PAD as u8, 1, GrainField::Size),
        Some(250.0)
    );
    assert_eq!(
        e.kit_layer_lock_value(0, PAD as u8, 0, GrainField::Size),
        None,
        "the other layer is left alone"
    );
}

#[test]
fn nan_gives_the_layers_own_back_and_a_live_value_goes_over_a_lock() {
    let mut e = engine(&project(grain(), grain()));
    e.apply(lock(0, GrainField::Pitch, 5.0));
    e.apply(live(0, GrainField::Pitch, -7.0));
    assert_eq!(
        e.kit_layer_live_value(0, PAD as u8, 0, GrainField::Pitch),
        Some(-7.0)
    );
    hit(&mut e);
    let over = run(&mut e, 6_000);
    let mut want = engine(&project(with(GrainField::Pitch, -7.0), grain()));
    hit(&mut want);
    assert_eq!(
        over,
        run(&mut want, 6_000),
        "the live value plays over the lock"
    );

    e.apply(Event::AllOff);
    run(&mut e, 4_000);
    e.apply(live(0, GrainField::Pitch, f32::NAN));
    e.apply(lock(0, GrainField::Pitch, f32::NAN));
    assert_eq!(
        e.kit_layer_lock_value(0, PAD as u8, 0, GrainField::Pitch),
        None
    );
    // The same history with no live value or lock, from its second hit.
    let mut plain = engine(&project(grain(), grain()));
    hit(&mut plain);
    run(&mut plain, 6_000);
    plain.apply(Event::AllOff);
    run(&mut plain, 4_000);
    hit(&mut e);
    hit(&mut plain);
    assert_eq!(run(&mut e, 6_000), run(&mut plain, 6_000), "its own again");
}

#[test]
fn values_are_clamped_and_a_new_view_drops_the_live_ones_but_keeps_the_locks() {
    let p = project(grain(), grain());
    let mut e = engine(&p);
    e.apply(live(1, GrainField::Size, 1e9));
    assert_eq!(
        e.kit_layer_live_value(0, PAD as u8, 1, GrainField::Size),
        Some(GrainField::Size.info().max)
    );
    e.apply(lock(1, GrainField::Spray, 70.0));
    e.set_view(EngineView::build(&p, RATE));
    assert_eq!(
        e.kit_layer_live_value(0, PAD as u8, 1, GrainField::Size),
        None
    );
    assert_eq!(
        e.kit_layer_lock_value(0, PAD as u8, 1, GrainField::Spray),
        Some(70.0)
    );
    // Out of the pads or the layers: nothing.
    e.apply(Event::LayerLive {
        pad: 200,
        layer: 0,
        field: GrainField::Size,
        value: 10.0,
    });
    e.apply(live(MAX_LAYERS as u8, GrainField::Size, 10.0));
    assert_eq!(e.stats().refused_layer_values, 0);
}

#[test]
fn a_full_table_refuses_and_counts_and_an_ended_value_makes_room() {
    let mut e = engine(&project(grain(), grain()));
    let mut n = 0;
    'fill: for layer in 0..MAX_LAYERS as u8 {
        for field in GrainField::ALL {
            if n == LAYER_VALUES {
                break 'fill;
            }
            e.apply(live(layer, field, field.info().min));
            n += 1;
        }
    }
    assert_eq!(e.stats().refused_layer_values, 0);
    // A held one changes in place.
    e.apply(live(0, GrainField::Size, 100.0));
    assert_eq!(e.stats().refused_layer_values, 0);
    e.apply(lock(15, GrainField::Size, 100.0));
    assert_eq!(e.stats().refused_layer_values, 1, "the 65th is refused");
    assert_eq!(
        e.kit_layer_lock_value(0, PAD as u8, 15, GrainField::Size),
        None
    );
    e.apply(live(0, GrainField::Size, f32::NAN));
    e.apply(lock(15, GrainField::Size, 100.0));
    assert_eq!(e.stats().refused_layer_values, 1, "room again");
    assert_eq!(
        e.kit_layer_lock_value(0, PAD as u8, 15, GrainField::Size),
        Some(100.0)
    );
}

#[test]
fn the_master_effects_settings_play_live_until_nan_or_a_new_view() {
    let mut p = project(grain(), grain());
    let mut m = *p.master();
    m.mix = 0.5;
    p.set_master(m, None).unwrap();
    let mut e = engine(&p);
    assert!(e.set_master_fx(Engine::make_master_fx(RATE)).is_none());
    let fx = |e: &Engine| e.master_fx().map(|f| (f.mix(), f.params().size_ms));
    let own = fx(&e);
    assert_eq!(own.map(|o| o.0), Some(0.5));
    e.apply(Event::MasterLive {
        field: MasterField::Mix,
        value: 0.9,
    });
    e.apply(Event::MasterLive {
        field: MasterField::Grain(GrainField::Size),
        value: 400.0,
    });
    assert_eq!(fx(&e), Some((0.9, 400.0)));
    assert_eq!(e.kit_master_live_value(0, MasterField::Mix), Some(0.9));
    e.apply(Event::MasterLive {
        field: MasterField::Mix,
        value: f32::NAN,
    });
    assert_eq!(fx(&e).map(|f| f.0), Some(0.5), "the view's mix again");
    e.set_view(EngineView::build(&p, RATE));
    assert_eq!(fx(&e), own, "a new view drops them");
}

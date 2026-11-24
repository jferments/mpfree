//! A layer's grain settings as parameters (sampler refactor step 17): the
//! engine's view reads each layer's as edited, by its place on the pad.

use mpfree_sampler::engine::{Engine, EngineView};
use mpfree_sampler::grain_field::GrainField;
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn tone(name: &str) -> Source {
    Source::new(
        name,
        RATE,
        1,
        (0..4_410).map(|i| (i as f32 * 0.05).sin() * 0.5).collect(),
    )
    .unwrap()
}

#[test]
fn a_layers_grain_settings_read_as_edited_by_its_place_on_the_pad() {
    let mut p = Project::new();
    let a = p.add_source(tone("a"));
    let b = p.add_source(tone("b"));
    let c = p.add_source(tone("c"));
    let layer = |s: SourceId, size: f32| {
        let mut r = Region::whole(p.source(s).unwrap());
        r.mode = PlayMode::Granular;
        r.grain.size_ms = size;
        // Tune goes into the voice's pitch, not into the setting.
        r.tune_cents = 700;
        Layer::new(r)
    };
    let mut pad = Pad::new("clouds", layer(a, 40.0));
    pad.layers.push(layer(c, 60.0));
    pad.layers.push(layer(b, 90.0));
    p.set_pad(3, Some(pad), None).unwrap();
    let view = EngineView::build(&p, RATE);
    let size = |l: usize| view.layer_grain(3, l).map(|g| GrainField::Size.get(g));
    assert_eq!(size(0), Some(40.0));
    assert_eq!(size(1), Some(60.0));
    assert_eq!(size(2), Some(90.0), "the third layer by its place");
    assert_eq!(size(3), None, "no fourth layer");
    let pitch = view.layer_grain(3, 0).map(|g| GrainField::Pitch.get(g));
    assert_eq!(pitch, Some(0.0), "the layer's own pitch, not its tune");
    assert!(view.layer_grain(4, 0).is_none(), "an empty pad");
    let mut e = Engine::new(RATE, 4);
    e.set_view(view);
    assert_eq!(
        e.kit_layer_grain(0, 3, 2).map(|g| g.size_ms),
        Some(90.0),
        "the engine reads its kit's view"
    );
    assert!(e.kit_layer_grain(1, 3, 2).is_none(), "no second kit");
}

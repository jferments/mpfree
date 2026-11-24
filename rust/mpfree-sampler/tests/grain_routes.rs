//! A grain layer's routes (sampler refactor step 17c): velocity and the
//! key move a grain setting as the voice starts, pressure as it plays,
//! each by its depth times the setting's range; a route off plays as
//! none; and the routes save with the layer.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_sampler::save::{from_text, has_grain_routes, to_text};

const RATE: u32 = 44_100;
const PAD: usize = 2;

fn tone() -> Source {
    Source::new(
        "tone",
        RATE,
        1,
        (0..44_100).map(|i| (i as f32 * 0.04).sin() * 0.5).collect(),
    )
    .unwrap()
}

fn grain() -> GrainParams {
    GrainParams {
        size_ms: 60.0,
        density: 40.0,
        spray: 0.2,
        scan: 0.5,
        ..GrainParams::default()
    }
}

fn route(from: RouteFrom, to: GrainField, depth: f32) -> GrainRoutes {
    let mut r = GrainRoutes::OFF;
    r.0[0] = GrainRoute { from, to, depth };
    r
}

/// A project whose pad `PAD` is one granular layer with grain settings
/// `g` and routes `routes`, on keys from root 60 when `keys`.
fn project(g: GrainParams, routes: GrainRoutes, keys: bool) -> Project {
    let mut p = Project::new();
    let s = p.add_source(tone());
    let mut r = Region::whole(p.source(s).unwrap());
    r.mode = PlayMode::Granular;
    r.grain = g;
    r.grain_routes = routes;
    let mut pad = Pad::new("cloud", Layer::new(r));
    if keys {
        pad.keys = Some(Keys::all(60));
    }
    p.set_pad(PAD, Some(pad), None).unwrap();
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 8);
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

fn hit(e: &mut Engine, velocity: u8) {
    e.apply(Event::NoteOn {
        pad: PAD as u8,
        velocity,
    });
}

/// Grain settings `g` with `f` moved by `amount` of its range.
fn moved(f: GrainField, amount: f32) -> GrainParams {
    let mut g = grain();
    let i = f.info();
    let v = i.from_unit(i.to_unit(f.get(&g)) + amount);
    f.set(&mut g, v);
    g
}

fn heard(p: &Project, velocity: u8) -> Vec<f32> {
    let mut e = engine(p);
    hit(&mut e, velocity);
    run(&mut e, 8_192)
}

#[test]
fn velocity_moves_its_setting_by_the_depth_times_its_range_at_full() {
    let routes = route(RouteFrom::Velocity, GrainField::Size, 0.4);
    let full = heard(&project(grain(), routes, false), 127);
    let want = heard(
        &project(moved(GrainField::Size, 0.4), GrainRoutes::OFF, false),
        127,
    );
    assert_eq!(full, want, "full velocity: 40 % of the size's range longer");
    let half = heard(&project(grain(), routes, false), 64);
    let want = heard(
        &project(
            moved(GrainField::Size, 0.4 * (64.0 / 127.0)),
            GrainRoutes::OFF,
            false,
        ),
        64,
    );
    assert_eq!(half, want, "half as hard, half as far");
    assert_ne!(full, heard(&project(grain(), GrainRoutes::OFF, false), 127));
}

#[test]
fn a_route_off_or_at_no_depth_plays_as_none() {
    let mut routes = route(RouteFrom::Velocity, GrainField::Size, 0.0);
    routes.0[1] = GrainRoute {
        from: RouteFrom::Off,
        to: GrainField::Density,
        depth: 0.8,
    };
    assert_eq!(
        heard(&project(grain(), routes, false), 110),
        heard(&project(grain(), GrainRoutes::OFF, false), 110)
    );
}

#[test]
fn the_key_moves_its_setting_by_the_depth_an_octave_from_the_root() {
    let routes = route(RouteFrom::Key, GrainField::Scan, 0.25);
    let key = |p: &Project, note: u8| {
        let mut e = engine(p);
        e.apply(Event::Key {
            note,
            velocity: 100,
        });
        run(&mut e, 8_192)
    };
    let routed = project(grain(), routes, true);
    let plain = project(grain(), GrainRoutes::OFF, true);
    assert_eq!(key(&routed, 60), key(&plain, 60), "the root leaves it");
    let up = project(moved(GrainField::Scan, 0.25), GrainRoutes::OFF, true);
    assert_eq!(key(&routed, 72), key(&up, 72), "an octave up: a quarter");
    let down = project(moved(GrainField::Scan, -0.25), GrainRoutes::OFF, true);
    assert_eq!(key(&routed, 48), key(&down, 48), "an octave down");
}

#[test]
fn pressure_moves_its_setting_as_the_voice_plays() {
    let routes = route(RouteFrom::Pressure, GrainField::Spray, 0.5);
    let mut e = engine(&project(grain(), routes, false));
    hit(&mut e, 100);
    run(&mut e, 2_048);
    let spray = |e: &Engine| e.cloud_of(PAD as u8, 0).unwrap().spray;
    assert!((spray(&e) - 0.2).abs() < 1e-6, "no pressure: its own");
    e.apply(Event::Pressure {
        pad: PAD as u8,
        value: 127,
    });
    run(&mut e, 4_096);
    assert!(
        (spray(&e) - 0.7).abs() < 1e-4,
        "full: half its range more {}",
        spray(&e)
    );
    e.apply(Event::Pressure {
        pad: PAD as u8,
        value: 0,
    });
    run(&mut e, 4_096);
    assert!((spray(&e) - 0.2).abs() < 1e-6, "let go: back");
}

#[test]
fn an_edit_of_the_routes_reaches_the_cloud_still_playing() {
    let mut e = engine(&project(grain(), GrainRoutes::OFF, false));
    hit(&mut e, 100);
    run(&mut e, 2_048);
    e.apply(Event::Pressure {
        pad: PAD as u8,
        value: 127,
    });
    run(&mut e, 2_048);
    let spray = |e: &Engine| e.cloud_of(PAD as u8, 0).unwrap().spray;
    assert!((spray(&e) - 0.2).abs() < 1e-6);
    let routes = route(RouteFrom::Pressure, GrainField::Spray, -0.1);
    e.edit_view(EngineView::build(&project(grain(), routes, false), RATE));
    run(&mut e, 4_096);
    assert!((spray(&e) - 0.1).abs() < 1e-4, "{}", spray(&e));
}

#[test]
fn routes_save_with_the_layer_and_a_kit_without_them_saves_as_before() {
    let mut routes = route(RouteFrom::Velocity, GrainField::Size, 0.4);
    routes.0[2] = GrainRoute {
        from: RouteFrom::Key,
        to: GrainField::EnvSustain,
        depth: -0.35,
    };
    let p = project(grain(), routes, false);
    let file = |_: &Source| "tone.wav".to_string();
    let text = to_text(&[("Clouds", &p)], 0, &file);
    assert!(has_grain_routes(&text));
    assert!(text.contains("region.grain_route.2.from = key"), "{text}");
    let mut read = |_: &str| Ok(tone());
    let back = from_text(&text, &mut read).unwrap();
    let r = &back.kits[0].1.pad(PAD).unwrap().layers[0].region;
    assert_eq!(r.grain_routes, routes);

    let plain = project(grain(), GrainRoutes::OFF, false);
    let text = to_text(&[("Clouds", &plain)], 0, &file);
    assert!(!has_grain_routes(&text));
    assert!(!text.contains("grain_route"));
}

#[test]
fn a_route_out_of_its_range_is_refused() {
    let mut p = project(grain(), GrainRoutes::OFF, false);
    let mut pad = p.pad(PAD).unwrap().clone();
    pad.layers[0].region.grain_routes = route(RouteFrom::Velocity, GrainField::Size, 1.5);
    assert!(p.set_pad(PAD, Some(pad), None).is_err());
}

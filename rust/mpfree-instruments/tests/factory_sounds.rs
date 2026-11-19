//! Every factory preset file loads as a sound: each preset, with the
//! effects and macros its block brings (if any), gives the engine the very
//! values the engine's own load gives, and no line in it goes unread (a
//! misspelt setting would otherwise be kept as an unknown line and do
//! nothing).

use mpfree_instruments::sound::has_design;
use mpfree_instruments::{Design, Designed, Sound};
use mpfree_node::Node;
use mpfree_synth::{
    Analog, Bass, Choir, Drawbar, Drums, Fm, Keys, Pluck, Poly, Preset, Squelch, Strings, Wavetable,
    Wind,
};

const RATE: u32 = 48_000;

fn check(
    name: &str,
    presets: &[Preset],
    load: impl Fn(&Preset) -> Box<dyn Node>,
    fresh: impl Fn() -> Box<dyn Node>,
) {
    assert!(!presets.is_empty(), "{name}: no presets");
    for p in presets {
        let engine = load(p);
        let mut s = Sound::new(fresh(), &Design::default()).unwrap();
        let read = s
            .load_text(p.text)
            .unwrap_or_else(|e| panic!("{name} {}: {e}", p.name));
        assert_eq!(read.name, p.name);
        assert!(
            read.extra.is_empty(),
            "{name} {}: lines nothing reads: {:?}",
            p.name,
            read.extra
        );
        assert_eq!(s.is_plain(), !has_design(p.text), "{name} {}", p.name);
        for q in engine.params().iter().filter(|q| !q.readonly) {
            assert_eq!(
                s.param(q.id).to_bits(),
                engine.param(q.id).to_bits(),
                "{name} {}: {} is {} as a sound, {} on the engine",
                p.name,
                q.name,
                s.param(q.id),
                engine.param(q.id)
            );
        }
        // Each macro's saved place agrees with what it moves, so the first
        // turn of a macro carries on from the sound as saved, with no jump.
        let macros: Vec<u16> = s
            .params()
            .iter()
            .filter(|q| q.group == "Macros")
            .map(|q| q.id)
            .collect();
        let before: Vec<f32> = s.params().iter().map(|q| s.param(q.id)).collect();
        for id in macros {
            s.set_param(id, s.param(id));
        }
        let mut off = Vec::new();
        for (q, &was) in s.params().iter().zip(&before) {
            let now = s.param(q.id);
            if (now - was).abs() > 1e-3 * (q.max - q.min) {
                off.push(format!("{}: saved {was}, its macro puts {now}", q.name));
            }
        }
        assert!(off.is_empty(), "{name} {}: {off:#?}", p.name);
    }
}

macro_rules! check_engine {
    ($name:literal, $t:ty) => {
        check(
            $name,
            <$t>::presets(),
            |p| {
                let mut e = <$t>::new(RATE);
                e.load_preset(p);
                Box::new(e)
            },
            || Box::new(<$t>::new(RATE)),
        )
    };
}

#[test]
fn every_factory_preset_loads_as_a_sound_to_the_engines_own_values() {
    check_engine!("Poly", Poly);
    check_engine!("Drums", Drums);
    check_engine!("Pluck", Pluck);
    check_engine!("Wavetable", Wavetable);
    check_engine!("Keys", Keys);
    check_engine!("Drawbar", Drawbar);
    check_engine!("Squelch", Squelch);
    check_engine!("Analog", Analog);
    check_engine!("Strings", Strings);
    check_engine!("Bass", Bass);
    check_engine!("FM", Fm);
    check_engine!("Wind", Wind);
    check_engine!("Choir", Choir);
}

#[test]
fn design_lines_are_found_in_a_presets_text() {
    assert!(has_design("mpfree-preset = 1\nfx.1 = mpfree.fx.plate\n"));
    assert!(has_design("macro.2.map = fx-1.mix 0 100\n"));
    assert!(!has_design("mpfree-preset = 1\nfx-1.mix = 30\n"));
    assert!(!has_design("# fx.1 = mpfree.fx.plate\nname = x\n"));
}

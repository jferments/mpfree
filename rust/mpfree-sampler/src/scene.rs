//! Scenes: the kit's pads stored as they are, to morph between with a
//! crossfader and to go back to, as on the Octatrack.
//!
//! - A [`Scene`] is every pad of the kit as it was when stored (every
//!   setting). A kit keeps two, A and B ([`Project::scenes`]), with the
//!   kit when it is saved.
//! - [`morph`] gives the live values ([`crate::engine::Event::PadLive`])
//!   for a crossfader position between A (0) and B (1): for each pad with a
//!   sound in both, its level, pan, tune, cutoff, resonance and the amp
//!   envelope's attack, decay and release, each between its value in A and
//!   in B (the cutoff on an octave scale). A setting that is the same in
//!   both is left to the pad's own, and so is a pad either scene has no
//!   sound on. Nothing in the kit changes: the engine plays the values
//!   until they are cleared ([`clear`]) or the kit's next view comes in.
//! - [`Project::return_to`] puts the pads back as a scene stored them, as
//!   one undo step: mangle the kit, then go back.

use crate::engine::PadField;
use crate::model::{FilterKind, Pad, Project, PADS};

/// Where a pad without a filter is open to: the engine's open low-pass.
pub const OPEN_HZ: f32 = 22_000.0;

/// The kit's pads as they were when stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub(crate) pads: Vec<Option<Pad>>,
}

impl Scene {
    /// The pads of `p` as they are now.
    pub fn of(p: &Project) -> Scene {
        Scene {
            pads: (0..PADS).map(|i| p.pad(i).cloned()).collect(),
        }
    }

    /// A scene of these pads (index = pad), as a saved kit gives them.
    pub fn from_pads(mut pads: Vec<Option<Pad>>) -> Scene {
        pads.resize(PADS, None);
        pads.truncate(PADS);
        Scene { pads }
    }

    pub fn pad(&self, i: usize) -> Option<&Pad> {
        self.pads.get(i)?.as_ref()
    }

    /// How many pads it holds a sound for.
    pub fn sounds(&self) -> usize {
        self.pads.iter().flatten().count()
    }
}

/// Pad `p`'s own value of `field`, in the units
/// [`crate::engine::Event::PadLive`] takes: a pad without a filter is open
/// ([`OPEN_HZ`], no resonance).
pub fn own(p: &Pad, field: PadField) -> f32 {
    let filtered = p.filter.kind != FilterKind::Off;
    match field {
        PadField::Level => p.level_db,
        PadField::Pan => p.pan,
        PadField::Tune => p.tune_cents as f32,
        PadField::Cutoff if filtered => p.filter.cutoff_hz,
        PadField::Cutoff => OPEN_HZ,
        PadField::Resonance if filtered => p.filter.resonance,
        PadField::Resonance => 0.0,
        PadField::Attack => p.amp.attack_ms,
        PadField::Decay => p.amp.decay_ms,
        PadField::Release => p.amp.release_ms,
    }
}

/// The live values at crossfader position `x` (0 is A, 1 is B), as
/// `(pad, field, value)`: see the module notes.
pub fn morph(a: &Scene, b: &Scene, x: f32) -> Vec<(u8, PadField, f32)> {
    let x = if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut out = Vec::new();
    for i in 0..PADS.min(a.pads.len()).min(b.pads.len()) {
        let (Some(pa), Some(pb)) = (a.pad(i), b.pad(i)) else {
            continue;
        };
        for f in PadField::ALL {
            let (va, vb) = (own(pa, f), own(pb, f));
            if va == vb || !va.is_finite() || !vb.is_finite() {
                continue;
            }
            let v = match f {
                // An octave scale, so the sweep sounds even.
                PadField::Cutoff => {
                    let (la, lb) = (va.max(1.0).log2(), vb.max(1.0).log2());
                    (la + (lb - la) * x).exp2()
                }
                _ => va + (vb - va) * x,
            };
            out.push((i as u8, f, f.clamp(v)));
        }
    }
    out
}

/// What clears every value [`morph`] gave for scenes `a` and `b`: each
/// back to the pad's own (NaN).
pub fn clear(a: &Scene, b: &Scene) -> Vec<(u8, PadField, f32)> {
    morph(a, b, 0.0)
        .into_iter()
        .map(|(p, f, _)| (p, f, f32::NAN))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Layer, Region, Source};

    #[test]
    fn the_morph_runs_each_setting_from_a_to_b() {
        let s = Source::new("s", 48_000, 1, vec![0.0; 100]).unwrap();
        let mut p = Project::new();
        p.add_source(s.clone());
        let mut pad = Pad::new("Kick", Layer::new(Region::whole(&s)));
        pad.level_db = -6.0;
        pad.filter.kind = FilterKind::Off;
        p.set_pad(2, Some(pad.clone()), None).unwrap();
        p.set_pad(5, Some(pad.clone()), None).unwrap();
        let a = Scene::of(&p);
        pad.level_db = 0.0;
        pad.tune_cents = 1200;
        pad.filter.kind = FilterKind::LowPass;
        pad.filter.cutoff_hz = 1_375.0;
        p.set_pad(2, Some(pad), None).unwrap();
        p.set_pad(5, None, None).unwrap();
        let b = Scene::of(&p);
        assert_eq!((a.sounds(), b.sounds()), (2, 1));
        let at = |x: f32| {
            let mut m = morph(&a, &b, x);
            m.sort_by_key(|v| v.1 as u8);
            m
        };
        // Pad 5 is in A only; pad 2's other settings are the same.
        let half = at(0.5);
        let fields: Vec<(u8, PadField)> = half.iter().map(|v| (v.0, v.1)).collect();
        assert_eq!(
            fields,
            [
                (2, PadField::Level),
                (2, PadField::Tune),
                (2, PadField::Cutoff)
            ]
        );
        assert_eq!((half[0].2, half[1].2), (-3.0, 600.0));
        // Half way, on octaves, from open (22 kHz) to 1375 Hz.
        assert!((half[2].2 - 5_500.0).abs() < 0.5, "{}", half[2].2);
        assert_eq!(at(0.0)[0].2, -6.0);
        assert_eq!(at(1.0)[1].2, 1200.0);
        assert_eq!(at(f32::NAN)[0].2, -6.0, "nonsense is A");
        assert!(clear(&a, &b).iter().all(|v| v.2.is_nan()));
        assert_eq!(clear(&a, &b).len(), 3);
    }
}

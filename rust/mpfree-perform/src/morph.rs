//! The scene morph (roadmap T10): two rows of the launch grid on the mod
//! engine's scenes A and B, the crossfader between them, and the sound from
//! before to snap back to.
//!
//! A scene stores parameter values (`Scene::values`, in each parameter's
//! own units). The mod engine (`mpfree-mod`) already morphs every
//! parameter both of its scenes A and B hold, by its morph position
//! (`Owner::Morph(0)`, 0 to 1), between the base layer and the macros.
//! [`SceneMorph`] fills A and B from two rows:
//!
//! - [`SceneMorph::load`]: every parameter either row holds goes on both
//!   sides. A parameter only one row holds gets its value now on the other
//!   side, so the morph moves it from where it is and nothing jumps. The
//!   values the parameters have when a morph starts are kept: the snapshot.
//!   Loading other rows while morphing keeps the first snapshot.
//! - [`SceneMorph::snap_back`]: the morph lets go and every parameter it
//!   held goes back to the snapshot (the host sends the values it returns
//!   as `ModEvent::Set`), the morph position back to 0.
//! - [`SceneMorph::keep`]: the morph lets go and the parameters stay where
//!   the morph has them: those values become their own.
//! - [`SceneMorph::store`]: a knob turned while morphing edits the scene
//!   the morph is nearer (the engine does that); this writes A and B back
//!   into their rows, so the project keeps the edit.
//! - [`save_scene`]: a row stores every parameter's own value now (and the
//!   macros'), for a later morph.
//!
//! All of it runs on the control thread; the engine's `process` does the
//! morphing on the audio thread as before.

use mpfree_mod::{ModEngine, ModError, MACROS};
use mpfree_seq::mpfree_param::{Owner, ParamAddr};
use mpfree_seq::{Project, SCENES};

/// What [`SceneMorph::load`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    /// Parameters on both sides now.
    pub morphing: usize,
    /// Values the rows hold for parameters the engine does not know (not
    /// on a strip, a sound or a macro this set has).
    pub unknown: usize,
}

/// Two rows on the morph: see the module notes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneMorph {
    rows: Option<(usize, usize)>,
    /// Each held parameter's own value when the morph started.
    snapshot: Vec<(ParamAddr, f32)>,
}

impl SceneMorph {
    /// The rows on A and B, while a morph is loaded.
    pub fn rows(&self) -> Option<(usize, usize)> {
        self.rows
    }

    /// Put rows `a` and `b` on scenes A and B: see the module notes.
    pub fn load(
        &mut self,
        engine: &mut ModEngine,
        project: &Project,
        a: usize,
        b: usize,
    ) -> Result<Loaded, ModError> {
        if a >= SCENES || b >= SCENES {
            return Err(ModError::Refused);
        }
        let (sa, sb) = (project.scene(a), project.scene(b));
        let mut addrs: Vec<ParamAddr> = Vec::new();
        for s in [sa, sb].into_iter().flatten() {
            for &(addr, _) in &s.values {
                if !addrs.contains(&addr) {
                    addrs.push(addr);
                }
            }
        }
        // Back to the snapshot's own values for what the last rows held
        // and these do not: clear first.
        engine.clear_scenes();
        let mut loaded = Loaded::default();
        for addr in addrs {
            if matches!(addr.owner, Owner::Morph(_) | Owner::Mod(_)) {
                loaded.unknown += 1;
                continue;
            }
            let Some(now) = engine.base(addr) else {
                loaded.unknown += 1;
                continue;
            };
            let now = match self.snapshot.iter().find(|s| s.0 == addr) {
                Some(s) => s.1,
                None => {
                    self.snapshot.push((addr, now));
                    now
                }
            };
            let va = sa.and_then(|s| s.value(addr)).unwrap_or(now);
            let vb = sb.and_then(|s| s.value(addr)).unwrap_or(now);
            engine.set_scene_value(addr, 0, Some(va))?;
            engine.set_scene_value(addr, 1, Some(vb))?;
            loaded.morphing += 1;
        }
        self.rows = Some((a, b));
        Ok(loaded)
    }

    /// Let go and go back: the values to send as `ModEvent::Set` (each held
    /// parameter's snapshot, and the morph position 0).
    pub fn snap_back(&mut self, engine: &mut ModEngine) -> Vec<(ParamAddr, f32)> {
        engine.clear_scenes();
        self.rows = None;
        let mut out = std::mem::take(&mut self.snapshot);
        out.push((ParamAddr::morph_pos(0), 0.0));
        out
    }

    /// Let go and stay: the values to send as `ModEvent::Set` (each held
    /// parameter where the morph had it, and the morph position 0). Read
    /// before the scenes clear, so they are what was last sent.
    pub fn keep(&mut self, engine: &mut ModEngine) -> Vec<(ParamAddr, f32)> {
        let mut out: Vec<(ParamAddr, f32)> = self
            .snapshot
            .iter()
            .filter_map(|&(addr, _)| Some((addr, engine.value(addr)?)))
            .collect();
        engine.clear_scenes();
        self.rows = None;
        self.snapshot.clear();
        out.push((ParamAddr::morph_pos(0), 0.0));
        out
    }

    /// Write the engine's A and B back into their rows (knob turns made
    /// while morphing). Returns how many values changed.
    pub fn store(&self, engine: &ModEngine, project: &mut Project) -> usize {
        let Some((a, b)) = self.rows else {
            return 0;
        };
        let changes: Vec<_> = engine.scene_changes().collect();
        let mut n = 0;
        for (row, side) in [(a, 0), (b, 1)] {
            let Some(scene) = project.scene_mut(row) else {
                continue;
            };
            for &(addr, va, vb) in &changes {
                let v = if side == 0 { va } else { vb };
                if scene.value(addr).is_none_or(|old| (old - v).abs() > 1e-6)
                    && scene.set_value(addr, v).is_ok()
                {
                    n += 1;
                }
            }
        }
        n
    }
}

/// Row `row` stores every parameter's own value now, and every macro's
/// position, in place of what it had for them. Returns how many it holds,
/// or `None` past the last row or when it would hold too many.
pub fn save_scene(engine: &ModEngine, project: &mut Project, row: usize) -> Option<usize> {
    let scene = project.scene_mut(row)?;
    let macros = (0..MACROS as u8).map(ParamAddr::macro_value);
    for addr in engine.targets().chain(macros) {
        if let Some(v) = engine.base(addr) {
            scene.set_value(addr, v).ok()?;
        }
    }
    Some(scene.values.len())
}

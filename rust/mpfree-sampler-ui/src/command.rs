//! A change to one kit in a musician's terms: the sampler's commands
//! (the sampler refactor's step 12, `docs/dev/design/sampler-architecture.md`).
//!
//! A touch on a tab, a Play page Q-Link and a MIDI Learn control that
//! mean the same thing make the same [`Change`], and [`Change::apply`] is
//! the one piece of code that makes it, so they can't drift apart, and
//! each change runs in a test without a screen. A change is one undo
//! step, or joins the step before when it carries that step's gesture
//! (one drag, one turn: `Project::set_pad`).
//!
//! This is the kit's half: what changes in one kit's model. Which kit is
//! the app's half (`mpfree-app`'s `sampler_commands`), which names the
//! kit by its instrument id, never by a track. The tabs' own actions
//! ([`crate::PlayAction`], [`crate::PadsAction`], [`crate::WaveAction`])
//! apply through here too.
//!
//! The commands come in families, one pull request each: a pad's sound
//! (this one), grains, slices and Put on, the recorder, kits and takes.
//! Nothing here touches the screen or the audio thread.

use std::ops::Range;

use mpfree_sampler::engine::PadField;
use mpfree_sampler::model::{
    ChopMode, ChopSet, ModelError, Pad, PadsPlay, Project, Region, SourceId, KEPT_CHOP, MAX_LAYERS,
};
use mpfree_sampler::scene;

use crate::pads_tab::sound_region;
use crate::play_tab::{with_layer_added, with_regions_of};

/// The longest decay a Q-Link sets before the sound plays out: past it
/// the pad holds (sustain 1), as the Play page's Decay reads "open".
pub const DECAY_OPEN_MS: f32 = 4000.0;

/// The shortest decay a Q-Link sets.
pub const DECAY_SHORTEST_MS: f32 = 10.0;

/// A pad's tuning range, in cents either way.
pub const TUNE_CENTS: i32 = 2400;

/// What a Play page Q-Link turns on the pad hit last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadKnob {
    /// Semitones (SHIFT: 10 cents).
    Tune,
    /// The amp envelope's decay to silence; at the top the sound plays out.
    Decay,
}

/// A change to a kit. Pads are numbered from 0 across the kit's banks,
/// layers from 0 within a pad.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Pad `pad` replaced by `to`, or cleared (None): the Pads tab's copy,
    /// paste, clear and put, and its own rows.
    Pad { pad: usize, to: Option<Box<Pad>> },
    /// Pad `pad`'s settings as `to` has them, its layers' regions kept as
    /// the pad has them (the Sample page's part), each slice's own settings
    /// taken from `to` ([`with_regions_of`]): the Play tab and the Sound
    /// section. Refused when the layers differ in number.
    Settings { pad: usize, to: Box<Pad> },
    /// One of pad `pad`'s own settings set to `value`, in the setting's
    /// units (`scene::own`'s): a MIDI Learn control.
    Own {
        pad: usize,
        field: PadField,
        value: f32,
    },
    /// Pad `pad`'s tuning or decay turned by `detents` (`fine`: SHIFT
    /// held, smaller steps): a Play page Q-Link ([`turned`]).
    Turn {
        pad: usize,
        knob: PadKnob,
        detents: i32,
        fine: bool,
    },
    /// Layer `layer` of pad `pad` plays `region`: the Sample page's wave,
    /// its markers and its rows.
    Region {
        pad: usize,
        layer: usize,
        region: Box<Region>,
    },
    /// Layer `layer` of pad `pad`'s gain set to `db`: Normalize.
    Gain { pad: usize, layer: usize, db: f32 },
    /// Layer `layer` of pad `pad` starts its grains at `at` (0 to 1 of its
    /// region, the Grain tool's Position): the grain start found at the
    /// sound's loudest grain once its summary is in. Every other grain
    /// setting is part of the layer's region ([`Change::Region`]), which
    /// the Grain tool sends.
    GrainStart { pad: usize, layer: usize, at: f32 },
    /// Pad `pad` named `pad_name`, the sound its layer `layer` plays named
    /// `sound_name`, or both, as one step (the Sample page's text keyboard).
    /// Names are trimmed and must keep a character; a sound's name is every
    /// pad's that plays it (`Project::rename`).
    Name {
        pad: usize,
        layer: usize,
        pad_name: Option<String>,
        sound_name: Option<String>,
    },
    /// A copy of layer `after` of pad `pad` added just after it, taking the
    /// upper half of its velocities ([`with_layer_added`]).
    AddLayer { pad: usize, after: usize },
    /// Layer `layer` of pad `pad` taken out. A pad keeps one layer.
    RemoveLayer { pad: usize, layer: usize },
    /// Layer `layer` of pad `pad` plays the kit's sound `source` from start
    /// to end, keeping its play mode, gain, tune and channels
    /// ([`sound_region`]).
    LayerSound {
        pad: usize,
        layer: usize,
        source: SourceId,
    },
    /// What the kit's pads play: their own sounds, notes or 16 Levels.
    PadsPlay(PadsPlay),
    /// The kit's key fade, in notes: every keygroup fades over it and the
    /// zones that meet overlap by it.
    KeyFade(u8),
    /// Map to keys: each `(pad, note)` made a keygroup at that root note,
    /// and the pads play notes (`Project::map_to_keys`).
    MapToKeys { roots: Vec<(usize, u8)> },
    /// Chop to pads: the slices of `chop` made into pads of this kit from
    /// the pad or take `from`, one undo step; a take leaves the page in it
    /// (`Project::chop_take`). Onto another kit (Chop to a new kit) is
    /// [`Chop::place_in`], which the app runs with both kits.
    Chop { from: Chopped, chop: Box<Chop> },
    /// The Slice tool's cuts in sound `source` kept with the kit, so they
    /// are there when the tool comes back to it: saved, but no undo step,
    /// and nothing that plays changes ([`Change::plays`]).
    KeepChop {
        source: SourceId,
        mode: ChopMode,
        cuts: Vec<u64>,
    },
}

/// What slices are made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chopped {
    /// A pad of the kit.
    Pad(usize),
    /// A take kept on the Sample page.
    Take(usize),
}

/// Slices of a sound made into pads (the Slice tool's Chop to pads): the
/// chop between `bounds`, each slice's pad made from the pad or take
/// chopped, its layer `layer` first so its settings carry over.
#[derive(Clone, Debug, PartialEq)]
pub struct Chop {
    /// The sound sliced.
    pub source: SourceId,
    /// The chop's name and how it was made.
    pub name: String,
    pub mode: ChopMode,
    /// The region's start, the cuts and its end, in the sound's frames.
    pub bounds: Vec<u64>,
    pub layer: usize,
    /// The first pad the slices go on.
    pub to_pad: usize,
    /// As keygroups of one note each from this note, the pads then playing
    /// notes (`Project::chop_to_keys`).
    pub keys_from: Option<u8>,
    /// All on pad `to_pad` as one sliced pad (`Project::chop_to_one_pad`).
    pub one_pad: bool,
}

impl Chop {
    /// How many slices.
    pub fn count(&self) -> usize {
        self.bounds.len().saturating_sub(1)
    }

    /// The pads the slices fill.
    pub fn pads(&self) -> Range<usize> {
        let n = if self.one_pad { 1 } else { self.count() };
        self.to_pad..self.to_pad + n
    }

    /// The chop made of the sound in `p`, and which of its slices (from
    /// `first`, `count` of them) lie between the bounds. Refused when the
    /// bounds don't rise or the chop doesn't land on them.
    pub fn slices(&self, p: &Project) -> Result<(ChopSet, usize, usize), ModelError> {
        let bounds = &self.bounds;
        if bounds.len() < 2 || bounds.windows(2).any(|w| w[0] >= w[1]) {
            return Err(ModelError::BadSlice);
        }
        let s = p.source(self.source).ok_or(ModelError::UnknownSource)?;
        let c = ChopSet::from_markers(&self.name, s, self.mode, bounds.iter().copied())?;
        let first = usize::from(bounds[0] > 0);
        let count = self.count();
        if c.bounds(first).map(|b| b.0) != Some(bounds[0])
            || c.bounds(first + count - 1).map(|b| b.1) != bounds.last().copied()
        {
            return Err(ModelError::BadSlice);
        }
        Ok((c, first, count))
    }

    /// What each slice's pad is made from: `made_from` with the chopped
    /// layer first.
    fn template(&self, made_from: &Pad) -> Result<Pad, ModelError> {
        let mut t = made_from.clone();
        if self.layer >= t.layers.len() {
            return Err(ModelError::BadPad);
        }
        t.layers.swap(0, self.layer);
        Ok(t)
    }

    /// The slices on `p`'s pads, each made from `made_from`, one undo step.
    fn place(&self, p: &mut Project, made_from: &Pad) -> Result<(), ModelError> {
        let template = self.template(made_from)?;
        let (c, first, count) = self.slices(p)?;
        let made = if self.one_pad {
            p.chop_to_one_pad(c, first, count, self.to_pad, Some(&template))
        } else {
            match self.keys_from {
                Some(from) => p.chop_to_keys(c, first, count, self.to_pad, from, Some(&template)),
                None => p.chop_to_pads(c, first, count, self.to_pad, Some(&template)),
            }
        };
        made.map(|_| ())
    }

    /// Chop to a new kit: the slices on `to`'s pads from `to_pad`, made
    /// from `made_from` (a pad or a take) of `from` as [`Change::Chop`]
    /// makes them, with the sound added to `to`: one undo step of `to`.
    /// `from` doesn't change; a take chopped is let go from it by the
    /// caller. Returns the pads it filled.
    pub fn place_in(
        &self,
        from: &Project,
        made_from: Chopped,
        to: &mut Project,
    ) -> Result<Range<usize>, ModelError> {
        let made_from = match made_from {
            Chopped::Pad(i) => from.pad(i).ok_or(ModelError::BadPad)?,
            Chopped::Take(i) => from.take(i).ok_or(ModelError::BadTake)?,
        };
        let template = self.template(made_from)?;
        let (c, first, count) = self.slices(from)?;
        let s = from.source(c.source).ok_or(ModelError::UnknownSource)?;
        to.chop_to_pads_of(s, c, first, count, self.to_pad, Some(&template))?;
        Ok(self.to_pad..self.to_pad + count)
    }
}

impl Change {
    /// Makes the change in `p`: one undo step, or part of the step before
    /// when `gesture` is that step's on the same pad. A change of the whole
    /// kit is always a step of its own. Refused (nothing changed) when the
    /// pad, layer or sound isn't there, or the result isn't a pad the model
    /// takes.
    pub fn apply(&self, p: &mut Project, gesture: Option<u64>) -> Result<(), ModelError> {
        match self {
            Change::Pad { pad, to } => p.set_pad(*pad, to.as_deref().cloned(), gesture),
            Change::Settings { pad, to } => {
                let now = p.pad(*pad).ok_or(ModelError::BadPad)?;
                let next = with_regions_of(to, now)?;
                p.set_pad(*pad, Some(next), gesture)
            }
            Change::Own { pad, field, value } => {
                let (field, value) = (*field, *value);
                p.edit_pad(*pad, gesture, move |pd| scene::set_own(pd, field, value))
            }
            Change::Turn {
                pad,
                knob,
                detents,
                fine,
            } => {
                let (knob, detents, fine) = (*knob, *detents, *fine);
                p.edit_pad(*pad, gesture, move |pd| turned(pd, knob, detents, fine))
            }
            Change::Region { pad, layer, region } => {
                let layer = has_layer(p, *pad, *layer)?;
                let region = (**region).clone();
                p.edit_pad(*pad, gesture, move |pd| pd.layers[layer].region = region)
            }
            Change::Gain { pad, layer, db } => {
                let (layer, db) = (has_layer(p, *pad, *layer)?, *db);
                p.edit_pad(*pad, gesture, move |pd| {
                    pd.layers[layer].region.gain_db = db
                })
            }
            Change::GrainStart { pad, layer, at } => {
                let layer = has_layer(p, *pad, *layer)?;
                if !(0.0..=1.0).contains(at) {
                    return Err(ModelError::BadRegion("grain start"));
                }
                let at = *at;
                p.edit_pad(*pad, gesture, move |pd| {
                    pd.layers[layer].region.grain.position = at
                })
            }
            Change::Name {
                pad,
                layer,
                pad_name,
                sound_name,
            } => {
                let source = match sound_name {
                    Some(_) => Some(
                        p.pad(*pad)
                            .and_then(|pd| pd.layers.get(*layer))
                            .ok_or(ModelError::BadPad)?
                            .region
                            .source,
                    ),
                    None => None,
                };
                p.rename(
                    pad_name.as_deref().map(|n| (*pad, n)),
                    source.zip(sound_name.as_deref()),
                )
            }
            Change::AddLayer { pad, after } => {
                let now = p.pad(*pad).ok_or(ModelError::BadPad)?;
                if now.layers.len() >= MAX_LAYERS {
                    return Err(ModelError::TooManyLayers);
                }
                let next = with_layer_added(now, *after).ok_or(ModelError::BadPad)?;
                p.set_pad(*pad, Some(next), gesture)
            }
            Change::RemoveLayer { pad, layer } => {
                let now = p.pad(*pad).ok_or(ModelError::BadPad)?;
                if now.layers.len() < 2 || *layer >= now.layers.len() {
                    return Err(ModelError::BadPad);
                }
                let mut next = now.clone();
                next.layers.remove(*layer);
                p.set_pad(*pad, Some(next), gesture)
            }
            Change::LayerSound { pad, layer, source } => {
                let frames = p.source(*source).ok_or(ModelError::UnknownSource)?.frames();
                let mut next = p.pad(*pad).ok_or(ModelError::BadPad)?.clone();
                let l = next.layers.get_mut(*layer).ok_or(ModelError::BadPad)?;
                l.region = sound_region(Some(&l.region), *source, frames);
                p.set_pad(*pad, Some(next), gesture)
            }
            Change::PadsPlay(play) => p.set_pads_play(*play),
            Change::KeyFade(fade) => p.set_key_fade(*fade),
            Change::MapToKeys { roots } => p.map_to_keys(roots).map(|_| ()),
            Change::Chop {
                from: Chopped::Pad(pad),
                chop,
            } => {
                let made_from = p.pad(*pad).ok_or(ModelError::BadPad)?.clone();
                chop.place(p, &made_from)
            }
            Change::Chop {
                from: Chopped::Take(i),
                chop,
            } => p.chop_take(*i, |p, take| chop.place(p, take)),
            Change::KeepChop { source, mode, cuts } => {
                let s = p.source(*source).ok_or(ModelError::UnknownSource)?;
                let c = ChopSet::from_markers(KEPT_CHOP, s, *mode, cuts.iter().copied())?;
                p.keep_chop(c)
            }
        }
    }

    /// The pad it changes, or None for a change of the whole kit (a chop
    /// fills [`Chop::pads`]).
    pub fn pad(&self) -> Option<usize> {
        match self {
            Change::Pad { pad, .. }
            | Change::Settings { pad, .. }
            | Change::Own { pad, .. }
            | Change::Turn { pad, .. }
            | Change::Region { pad, .. }
            | Change::Gain { pad, .. }
            | Change::Name { pad, .. }
            | Change::GrainStart { pad, .. }
            | Change::AddLayer { pad, .. }
            | Change::RemoveLayer { pad, .. }
            | Change::LayerSound { pad, .. } => Some(*pad),
            Change::PadsPlay(_)
            | Change::KeyFade(_)
            | Change::MapToKeys { .. }
            | Change::Chop { .. }
            | Change::KeepChop { .. } => None,
        }
    }

    /// Whether it can rename a pad, replace it or change its type, which
    /// the Play page's pad names show.
    pub fn renames(&self) -> bool {
        match self {
            Change::Pad { .. } | Change::Chop { .. } => true,
            Change::Name { pad_name, .. } => pad_name.is_some(),
            _ => false,
        }
    }
}

impl Change {
    /// Whether it can change what the kit plays, so the kit's view must go
    /// to the audio thread again. A chop kept changes only what is saved.
    pub fn plays(&self) -> bool {
        !matches!(self, Change::KeepChop { .. })
    }
}

/// `layer` if pad `pad` of `p` has it.
fn has_layer(p: &Project, pad: usize, layer: usize) -> Result<usize, ModelError> {
    match p.pad(pad) {
        Some(pd) if layer < pd.layers.len() => Ok(layer),
        _ => Err(ModelError::BadPad),
    }
}

/// `p`'s tuning or decay after a Q-Link turn of `detents`. Tune: a
/// semitone a detent, 10 cents with `fine`, within [`TUNE_CENTS`]. Decay:
/// doubled every 8 detents (32 with `fine`) from [`DECAY_SHORTEST_MS`];
/// turned up to [`DECAY_OPEN_MS`] the sound plays out, and turning down
/// from there starts at it.
pub fn turned(p: &mut Pad, knob: PadKnob, detents: i32, fine: bool) {
    match knob {
        PadKnob::Tune => {
            let step = if fine { 10 } else { 100 };
            p.tune_cents = (p.tune_cents + detents * step).clamp(-TUNE_CENTS, TUNE_CENTS);
        }
        PadKnob::Decay => {
            let max = DECAY_OPEN_MS;
            let open = p.amp.sustain >= 0.999;
            let ms = if open {
                max
            } else {
                p.amp.decay_ms.max(DECAY_SHORTEST_MS)
            };
            let per = if fine { 32.0 } else { 8.0 };
            let to = (ms * (detents as f32 / per).exp2())
                .round()
                .clamp(DECAY_SHORTEST_MS, max);
            if to >= max && detents > 0 {
                p.amp.sustain = 1.0;
                p.amp.decay_ms = 0.0;
            } else {
                p.amp.sustain = 0.0;
                p.amp.decay_ms = to;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_sampler::model::{Layer, Source};

    const RATE: u32 = 48_000;

    /// A kit with a short sound on pad 0 (two layers, a keygroup at C3)
    /// and a second sound that nothing plays yet.
    fn kit() -> (Project, SourceId, SourceId) {
        let mut p = Project::new();
        let hit: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let a = Source::new("Hit", RATE, 1, hit.clone()).expect("a sound");
        let b = Source::new("Other", RATE, 1, hit[..2400].to_vec()).expect("a sound");
        let region = mpfree_sampler::model::Region::whole(&a);
        let (a, b) = (p.add_source(a), p.add_source(b));
        let mut pad = Pad::new("Hit", Layer::new(region));
        pad.layers.push(pad.layers[0].clone());
        p.set_pad(0, Some(pad), None).expect("the pad");
        p.map_to_keys(&[(0, 60)]).expect("a keygroup");
        (p, a, b)
    }

    /// Whether a change was made.
    type Made = Box<dyn Fn(&Project) -> bool>;

    /// Whether two kits' pads and what they play are the same.
    fn same(a: &Project, b: &Project) -> bool {
        (0..mpfree_sampler::model::PADS).all(|i| a.pad(i) == b.pad(i))
            && a.pads_play() == b.pads_play()
    }

    fn steps(p: &mut Project) -> usize {
        let mut n = 0;
        while p.undo() {
            n += 1;
        }
        n
    }

    #[test]
    fn each_change_makes_its_change_and_one_undo_step() {
        let (base, _, other) = kit();
        let pad0 = base.pad(0).cloned().expect("pad 0");
        let mut quiet = pad0.clone();
        quiet.level_db = -12.0;
        let mut short = pad0.layers[0].region.clone();
        short.end = 1000;
        let cases: Vec<(Change, Made)> = vec![
            (
                Change::Pad {
                    pad: 3,
                    to: Some(Box::new(pad0.clone())),
                },
                Box::new(|p: &Project| p.pad(3).is_some()),
            ),
            (
                Change::Pad { pad: 0, to: None },
                Box::new(|p: &Project| p.pad(0).is_none()),
            ),
            (
                Change::Settings {
                    pad: 0,
                    to: Box::new(quiet),
                },
                Box::new(|p: &Project| p.pad(0).unwrap().level_db == -12.0),
            ),
            (
                Change::Own {
                    pad: 0,
                    field: PadField::Pan,
                    value: -0.5,
                },
                Box::new(|p: &Project| p.pad(0).unwrap().pan == -0.5),
            ),
            (
                Change::Turn {
                    pad: 0,
                    knob: PadKnob::Tune,
                    detents: 3,
                    fine: false,
                },
                Box::new(|p: &Project| p.pad(0).unwrap().tune_cents == 300),
            ),
            (
                Change::Region {
                    pad: 0,
                    layer: 1,
                    region: Box::new(short),
                },
                Box::new(|p: &Project| p.pad(0).unwrap().layers[1].region.end == 1000),
            ),
            (
                Change::Gain {
                    pad: 0,
                    layer: 0,
                    db: 4.5,
                },
                Box::new(|p: &Project| p.pad(0).unwrap().layers[0].region.gain_db == 4.5),
            ),
            (
                Change::GrainStart {
                    pad: 0,
                    layer: 1,
                    at: 0.25,
                },
                Box::new(|p: &Project| p.pad(0).unwrap().layers[1].region.grain.position == 0.25),
            ),
            (
                Change::Name {
                    pad: 0,
                    layer: 0,
                    pad_name: Some(" Rim ".to_string()),
                    sound_name: Some("Rim shot".to_string()),
                },
                Box::new(move |p: &Project| {
                    let pd = p.pad(0).unwrap();
                    let sound = p.source(pd.layers[0].region.source).unwrap();
                    pd.name == "Rim" && sound.name == "Rim shot"
                }),
            ),
            (
                Change::AddLayer { pad: 0, after: 0 },
                Box::new(|p: &Project| p.pad(0).unwrap().layers.len() == 3),
            ),
            (
                Change::RemoveLayer { pad: 0, layer: 1 },
                Box::new(|p: &Project| p.pad(0).unwrap().layers.len() == 1),
            ),
            (
                Change::LayerSound {
                    pad: 0,
                    layer: 1,
                    source: other,
                },
                Box::new(move |p: &Project| {
                    let r = &p.pad(0).unwrap().layers[1].region;
                    r.source == other && r.end == 2400
                }),
            ),
            (
                Change::PadsPlay(PadsPlay::Keys { from: 48 }),
                Box::new(|p: &Project| p.pads_play() == PadsPlay::Keys { from: 48 }),
            ),
            (
                Change::KeyFade(3),
                Box::new(|p: &Project| p.key_fade() == Some(3)),
            ),
            (
                Change::MapToKeys {
                    roots: vec![(0, 64)],
                },
                Box::new(|p: &Project| p.pad(0).unwrap().keys.is_some_and(|k| k.root == 64)),
            ),
        ];
        for (change, made) in cases {
            let mut p = base.clone();
            let before = steps(&mut p.clone());
            change
                .apply(&mut p, None)
                .unwrap_or_else(|e| panic!("{change:?}: {e:?}"));
            assert!(made(&p), "{change:?} didn't make its change");
            assert_eq!(
                steps(&mut p.clone()),
                before + 1,
                "{change:?} is one undo step"
            );
            assert!(p.undo());
            assert!(same(&p, &base), "{change:?} undone");
        }
    }

    #[test]
    fn a_turn_in_one_gesture_is_one_undo_step() {
        let (mut p, ..) = kit();
        let before = steps(&mut p.clone());
        for _ in 0..5 {
            let turn = Change::Turn {
                pad: 0,
                knob: PadKnob::Tune,
                detents: 1,
                fine: true,
            };
            turn.apply(&mut p, Some(7)).expect("a turn");
        }
        assert_eq!(p.pad(0).unwrap().tune_cents, 50);
        assert_eq!(steps(&mut p.clone()), before + 1);
        // Another gesture starts another step.
        Change::Own {
            pad: 0,
            field: PadField::Level,
            value: -3.0,
        }
        .apply(&mut p, Some(8))
        .expect("a learned control");
        assert_eq!(steps(&mut p), before + 2);
    }

    #[test]
    fn a_change_that_cant_be_made_changes_nothing() {
        let (base, ..) = kit();
        let refused = [
            Change::Settings {
                pad: 5,
                to: Box::new(base.pad(0).unwrap().clone()),
            },
            Change::Region {
                pad: 0,
                layer: 2,
                region: Box::new(base.pad(0).unwrap().layers[0].region.clone()),
            },
            Change::Gain {
                pad: 9,
                layer: 0,
                db: 1.0,
            },
            Change::RemoveLayer { pad: 0, layer: 4 },
            Change::GrainStart {
                pad: 0,
                layer: 0,
                at: 1.5,
            },
            Change::GrainStart {
                pad: 0,
                layer: 3,
                at: 0.5,
            },
            Change::Name {
                pad: 0,
                layer: 0,
                pad_name: Some("  ".to_string()),
                sound_name: None,
            },
            Change::Name {
                pad: 6,
                layer: 0,
                pad_name: None,
                sound_name: Some("Lost".to_string()),
            },
            Change::LayerSound {
                pad: 0,
                layer: 0,
                source: SourceId(12345),
            },
            Change::Turn {
                pad: 200,
                knob: PadKnob::Decay,
                detents: 1,
                fine: false,
            },
        ];
        for change in refused {
            let mut p = base.clone();
            let before = steps(&mut p.clone());
            assert!(change.apply(&mut p, None).is_err(), "{change:?} refused");
            assert!(same(&p, &base), "{change:?} changed nothing");
            assert_eq!(steps(&mut p), before, "{change:?} left no step");
        }
        // A pad keeps one layer, and has at most MAX_LAYERS.
        let mut p = base.clone();
        Change::RemoveLayer { pad: 0, layer: 1 }
            .apply(&mut p, None)
            .expect("one taken out");
        assert_eq!(
            Change::RemoveLayer { pad: 0, layer: 0 }.apply(&mut p, None),
            Err(ModelError::BadPad)
        );
        for _ in 1..MAX_LAYERS {
            Change::AddLayer { pad: 0, after: 0 }
                .apply(&mut p, None)
                .expect("room for a layer");
        }
        assert_eq!(
            Change::AddLayer { pad: 0, after: 0 }.apply(&mut p, None),
            Err(ModelError::TooManyLayers)
        );
    }

    /// A chop of pad 0's sound (4,800 frames) into four slices from pad 4.
    fn chop(p: &Project) -> Chop {
        Chop {
            source: p.pad(0).unwrap().layers[0].region.source,
            name: "Chop".to_string(),
            mode: ChopMode::Manual,
            bounds: vec![0, 1200, 2400, 3600, 4800],
            layer: 1,
            to_pad: 4,
            keys_from: None,
            one_pad: false,
        }
    }

    #[test]
    fn chop_to_pads_fills_its_pads_in_one_step_from_a_pad_or_a_take() {
        let (base, ..) = kit();
        let c = chop(&base);
        assert_eq!(c.pads(), 4..8);
        let mut p = base.clone();
        let before = steps(&mut p.clone());
        Change::Chop {
            from: Chopped::Pad(0),
            chop: Box::new(c.clone()),
        }
        .apply(&mut p, None)
        .expect("chopped");
        for (k, pad) in (4..8).enumerate() {
            let r = &p.pad(pad).expect("a slice's pad").layers[0].region;
            assert_eq!((r.start, r.end), (1200 * k as u64, 1200 * (k as u64 + 1)));
        }
        assert_eq!(steps(&mut p.clone()), before + 1);
        // One pad, sliced.
        let mut one = base.clone();
        let c1 = Chop {
            one_pad: true,
            ..c.clone()
        };
        assert_eq!(c1.pads(), 4..5);
        Change::Chop {
            from: Chopped::Pad(0),
            chop: Box::new(c1),
        }
        .apply(&mut one, None)
        .expect("on one pad");
        assert!(one.pad(4).is_some() && one.pad(5).is_none());
        // From a take kept on the page: the take leaves with the chop.
        let mut t = base.clone();
        let take = t.pad(0).unwrap().clone();
        let i = t.add_take(take).expect("a take");
        let before = steps(&mut t.clone());
        Change::Chop {
            from: Chopped::Take(i),
            chop: Box::new(c.clone()),
        }
        .apply(&mut t, None)
        .expect("chopped from the take");
        assert!(t.take(i).is_none() && t.pad(7).is_some());
        assert_eq!(steps(&mut t.clone()), before + 1);
        // Onto another kit: that kit gets the sound and the slices; this
        // one doesn't change.
        let mut other = Project::new();
        let pads = c
            .place_in(&base, Chopped::Pad(0), &mut other)
            .expect("onto another kit");
        assert_eq!(pads, 4..8);
        assert!(other
            .pad(4)
            .is_some_and(|pd| other.source(pd.layers[0].region.source).is_some()));
        // Bounds that don't rise are refused, and change nothing.
        let mut bad = base.clone();
        let wrong = Chop {
            bounds: vec![0, 2400, 1200, 4800],
            ..c
        };
        assert_eq!(
            Change::Chop {
                from: Chopped::Pad(0),
                chop: Box::new(wrong),
            }
            .apply(&mut bad, None),
            Err(ModelError::BadSlice)
        );
        assert!(same(&bad, &base));
    }

    #[test]
    fn a_chop_kept_is_no_undo_step_and_plays_nothing_new() {
        let (base, ..) = kit();
        let mut p = base.clone();
        let source = p.pad(0).unwrap().layers[0].region.source;
        let before = steps(&mut p.clone());
        let keep = Change::KeepChop {
            source,
            mode: ChopMode::Grid,
            cuts: vec![1200, 2400],
        };
        assert!(!keep.plays());
        keep.apply(&mut p, None).expect("kept");
        let kept = p.kept_chop(source).expect("the chop kept");
        assert_eq!((kept.name.as_str(), kept.mode), (KEPT_CHOP, ChopMode::Grid));
        assert_eq!(steps(&mut p), before, "no undo step");
        assert!(Change::KeyFade(1).plays());
    }

    #[test]
    fn decay_turned_to_the_top_plays_out_and_turns_down_from_there() {
        let (base, ..) = kit();
        let mut pad = base.pad(0).unwrap().clone();
        pad.amp.sustain = 0.0;
        pad.amp.decay_ms = 1000.0;
        turned(&mut pad, PadKnob::Decay, 8, false);
        assert_eq!((pad.amp.decay_ms, pad.amp.sustain), (2000.0, 0.0));
        turned(&mut pad, PadKnob::Decay, 16, false);
        assert_eq!((pad.amp.decay_ms, pad.amp.sustain), (0.0, 1.0), "open");
        turned(&mut pad, PadKnob::Decay, -8, false);
        assert_eq!((pad.amp.decay_ms, pad.amp.sustain), (2000.0, 0.0));
        turned(&mut pad, PadKnob::Decay, -200, false);
        assert_eq!(pad.amp.decay_ms, DECAY_SHORTEST_MS);
        turned(&mut pad, PadKnob::Tune, 30, false);
        assert_eq!(pad.tune_cents, TUNE_CENTS);
    }

    #[test]
    fn the_tabs_actions_make_the_same_changes() {
        let (base, ..) = kit();
        let mut after = base.pad(0).unwrap().clone();
        after.tune_cents = -700;
        let play = crate::PlayAction::Pad {
            pad: 0,
            after: Box::new(after.clone()),
            gesture: 4,
        };
        let (mut a, mut b) = (base.clone(), base.clone());
        play.apply(&mut a).expect("the tab's");
        Change::Settings {
            pad: 0,
            to: Box::new(after),
        }
        .apply(&mut b, Some(4))
        .expect("the command's");
        assert_eq!(a.pad(0), b.pad(0));
        assert_eq!(
            crate::PadsAction::Set {
                pad: 2,
                after: None,
                gesture: Some(1),
            }
            .change(),
            Some((Change::Pad { pad: 2, to: None }, Some(1)))
        );
        assert_eq!(crate::PlayAction::Pick(3).change(), None);
        assert_eq!(Change::KeyFade(2).pad(), None);
        assert_eq!(Change::RemoveLayer { pad: 4, layer: 0 }.pad(), Some(4));
        let name = |pad_name: Option<&str>| Change::Name {
            pad: 0,
            layer: 0,
            pad_name: pad_name.map(str::to_string),
            sound_name: Some("Kick".to_string()),
        };
        assert!(
            name(Some("Kick")).renames(),
            "the Play page shows pad names"
        );
        assert!(
            !name(None).renames(),
            "a sound's name isn't on the Play page"
        );
    }
}

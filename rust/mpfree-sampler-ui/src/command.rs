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

use mpfree_sampler::engine::PadField;
use mpfree_sampler::model::{ModelError, Pad, PadsPlay, Project, Region, SourceId, MAX_LAYERS};
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
        }
    }

    /// The pad it changes, or None for a change of the whole kit.
    pub fn pad(&self) -> Option<usize> {
        match self {
            Change::Pad { pad, .. }
            | Change::Settings { pad, .. }
            | Change::Own { pad, .. }
            | Change::Turn { pad, .. }
            | Change::Region { pad, .. }
            | Change::Gain { pad, .. }
            | Change::Name { pad, .. }
            | Change::AddLayer { pad, .. }
            | Change::RemoveLayer { pad, .. }
            | Change::LayerSound { pad, .. } => Some(*pad),
            Change::PadsPlay(_) | Change::KeyFade(_) | Change::MapToKeys { .. } => None,
        }
    }

    /// Whether it can rename a pad, replace it or change its type, which
    /// the Play page's pad names show.
    pub fn renames(&self) -> bool {
        match self {
            Change::Pad { .. } => true,
            Change::Name { pad_name, .. } => pad_name.is_some(),
            _ => false,
        }
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

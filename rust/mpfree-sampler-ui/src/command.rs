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
//! The commands came in families, one pull request each: a pad's sound,
//! grains, slices and Put on, the recorder, and kits and takes (a take's
//! edits, loading a pad, the scene strip). Undo and redo stay the kit's
//! own (`Project::undo`). Nothing here touches the screen or the audio
//! thread.

use std::ops::Range;

use mpfree_sampler::autosample::Instrument;
use mpfree_sampler::engine::PadField;
use mpfree_sampler::model::{
    ChopMode, ChopSet, Fade, GrainParams, Layer, ModelError, Pad, PadsPlay, PlayMode, Project,
    Region, Source, SourceId, Warp, KEPT_CHOP, MAX_LAYERS,
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

/// A kit's new pad plays this many voices at once: four copies of one
/// sound is already more than the ear tells apart, and a lock on a long
/// sound repeated can't fill the whole pool (round 4).
pub const KIT_POLYPHONY: u8 = 4;

/// The engine pans a centred sound −3 dB a side. A kit's new pad is
/// raised by this much, so it plays at the level the round 3 mixer played
/// it.
pub const CENTRE_DB: f32 = 3.010_3;

/// A new pad as the kits' own play: centred at the level a kit plays at,
/// [`KIT_POLYPHONY`] voices. A loaded or recorded sound on an empty pad
/// gets one, so it plays as loud as the kit's (a plain `Pad::new` is 3 dB
/// quieter: the engine's centre pan).
pub fn kit_pad(name: &str, layer: Layer) -> Pad {
    let mut pad = Pad::new(name, layer);
    pad.polyphony = KIT_POLYPHONY;
    pad.level_db = CENTRE_DB;
    pad
}

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
    /// A recorded sound kept (a take, a Flatten, a Recall, a grains
    /// capture): on a pad, or as a take on the Sample page, playing as
    /// `lands` says. One undo step, the sound added to the kit with it.
    Keep {
        sound: Recorded,
        onto: Landing,
        lands: Lands,
    },
    /// An auto-sampled instrument landed on the kit's pads from `to`, its
    /// keygroups playing notes. One undo step.
    AutoSampled { inst: Box<AutoSampled>, to: usize },
    /// Layer `layer` of pad `pad` plays `sound`, a Process's new sound made
    /// from its region (`warp::Process::sound`), `scale` of its frames to
    /// each of the region's: its loop, fades and warp tempo moved to match,
    /// one undo step with the sound's adding (`Project::use_processed`).
    Processed {
        pad: usize,
        layer: usize,
        sound: Recorded,
        scale: f64,
    },
    /// One of a pad's own changes (its settings, a layer's region, gain or
    /// grain start, its layers) made to take `take` on the Sample page
    /// instead, as to a pad, the change's pad number unused: one undo
    /// step, or part of its gesture's. Refused for any other change.
    OnTake { take: usize, change: Box<Change> },
    /// Take `take` named `name`, trimmed; it must keep a character.
    NameTake { take: usize, name: String },
    /// Take `take` put on pad `pad`, replacing what the pad had, and let go
    /// from the page: one undo step (Send to pad).
    PutTake { take: usize, pad: usize },
    /// A pad read back from its files (a loaded sound with its sidecar) on
    /// pad `pad`: `to`, with the sounds it plays and its chops on them, as
    /// one undo step. Sounds the kit has already are used as they are.
    LoadPad {
        pad: usize,
        sounds: Vec<Recorded>,
        to: Box<Pad>,
        chops: Vec<ChopSet>,
    },
    /// The kit's pads stored as scene A or B of the scene strip: saved with
    /// the kit, no undo step (storing again replaces it).
    SceneStore(SceneSlot),
    /// Back to scene A: every pad that differs from it put back as one
    /// undo step, those whose sounds are gone left as they are
    /// ([`Made::put_back`]). Nothing stored, or nothing changed since,
    /// changes nothing.
    SceneReturn,
}

/// Scene A or B of the scene strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneSlot {
    A,
    B,
}

impl SceneSlot {
    /// Its place in `Project::scenes`.
    pub fn index(self) -> usize {
        match self {
            SceneSlot::A => 0,
            SceneSlot::B => 1,
        }
    }
}

/// What a change made that the kit doesn't show by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Made {
    /// The kit was already as the change asks: nothing changed and no undo
    /// step was made (a take given the name it has, a return to scene A
    /// with nothing changed since).
    pub nothing: bool,
    /// The take a recorded sound became, its place in `Project::takes`.
    pub take: Option<usize>,
    /// Back to scene A: the pads put back, and those left as they are
    /// because their sounds are gone.
    pub put_back: Option<(usize, usize)>,
}

/// A recorded sound, as a change carries it: two are the same change when
/// they hold the same audio under the same name.
#[derive(Clone, Debug)]
pub struct Recorded(pub Source);

impl PartialEq for Recorded {
    fn eq(&self, other: &Recorded) -> bool {
        self.0.id() == other.0.id() && self.0.name == other.0.name
    }
}

/// An auto-sampled instrument, as a change carries it: two are the same
/// change when they have the same name and takes.
#[derive(Clone, Debug)]
pub struct AutoSampled(pub Instrument);

impl PartialEq for AutoSampled {
    fn eq(&self, other: &AutoSampled) -> bool {
        let ids = |i: &Instrument| i.sources.iter().map(Source::id).collect::<Vec<_>>();
        self.0.name == other.0.name && ids(&self.0) == ids(&other.0)
    }
}

/// Where a recorded sound lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// Pad `pad` of the kit.
    Pad(usize),
    /// A new take kept on the Sample page (Onto on Stay here).
    Take,
}

/// How a recorded sound plays once it lands, and how its pad is made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lands {
    /// Warped at the tempo it was played at (whole bars played to the
    /// project's tempo), following the project's from then on.
    pub warp: Option<Warp>,
    /// Faded in and out over this many frames (Recall's); 0, no fades.
    pub fade: u32,
    /// Plays as grains with these settings (a grains capture).
    pub grains: Option<GrainParams>,
    /// A Flatten's pad: a kit's new pad keeping only the old pad's mute
    /// group, chokes and polyphony, since the sound already has the rest in
    /// it. Otherwise the pad there keeps its settings, the sound in place
    /// of its own (`Pad::replacing`), and an empty pad becomes a kit's new
    /// pad ([`kit_pad`]).
    pub flattened: bool,
    /// The name its undo step goes by in the undo history
    /// (`Project::tag_last`; a Recall's); 0, the usual.
    pub tag: u8,
}

impl Lands {
    /// The new sound's layer, set as it plays.
    fn set(&self, layer: &mut Layer) {
        layer.region.warp = self.warp.clone();
        if self.fade > 0 {
            layer.region.fade_in = Fade {
                frames: self.fade,
                ..Fade::NONE
            };
            layer.region.fade_out = layer.region.fade_in;
        }
        if let Some(g) = self.grains {
            layer.region.mode = PlayMode::Granular;
            layer.region.grain = g;
        }
    }

    /// The pad the new sound's layer goes on, `old` the pad there before.
    fn pad(&self, name: &str, mut layer: Layer, old: Option<&Pad>) -> Pad {
        self.set(&mut layer);
        match old {
            Some(old) if self.flattened => {
                let mut new = kit_pad(name, layer);
                new.mute_group = old.mute_group;
                new.chokes = old.chokes.clone();
                new.polyphony = old.polyphony;
                new
            }
            Some(_) => Pad::replacing(name, layer, old),
            None => kit_pad(name, layer),
        }
    }
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
    /// when `gesture` is that step's on the same pad (or take). A change of
    /// the whole kit is always a step of its own. Refused (nothing changed)
    /// when the pad, take, layer or sound isn't there, or the result isn't
    /// a pad the model takes.
    pub fn apply(&self, p: &mut Project, gesture: Option<u64>) -> Result<(), ModelError> {
        self.make(p, gesture).map(|_| ())
    }

    /// [`Change::apply`], saying what it made that the kit doesn't show:
    /// the take a recorded sound became, the pads a return to scene A put
    /// back, or that nothing changed.
    pub fn make(&self, p: &mut Project, gesture: Option<u64>) -> Result<Made, ModelError> {
        let made = Made::default();
        match self {
            Change::Pad { pad, to } => p.set_pad(*pad, to.as_deref().cloned(), gesture)?,
            Change::Settings { pad, .. }
            | Change::Own { pad, .. }
            | Change::Turn { pad, .. }
            | Change::Region { pad, .. }
            | Change::Gain { pad, .. }
            | Change::GrainStart { pad, .. }
            | Change::AddLayer { pad, .. }
            | Change::RemoveLayer { pad, .. }
            | Change::LayerSound { pad, .. } => {
                let now = p.pad(*pad).ok_or(ModelError::BadPad)?;
                let next = self.next_pad(p, now).unwrap_or(Err(ModelError::BadPad))?;
                p.set_pad(*pad, Some(next), gesture)?;
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
                )?;
            }
            Change::PadsPlay(play) => p.set_pads_play(*play)?,
            Change::KeyFade(fade) => p.set_key_fade(*fade)?,
            Change::MapToKeys { roots } => {
                p.map_to_keys(roots)?;
            }
            Change::Chop {
                from: Chopped::Pad(pad),
                chop,
            } => {
                let made_from = p.pad(*pad).ok_or(ModelError::BadPad)?.clone();
                chop.place(p, &made_from)?;
            }
            Change::Chop {
                from: Chopped::Take(i),
                chop,
            } => p.chop_take(*i, |p, take| chop.place(p, take))?,
            Change::KeepChop { source, mode, cuts } => {
                let s = p.source(*source).ok_or(ModelError::UnknownSource)?;
                let c = ChopSet::from_markers(KEPT_CHOP, s, *mode, cuts.iter().copied())?;
                p.keep_chop(c)?;
            }
            Change::Keep { sound, onto, lands } => {
                let take = match onto {
                    Landing::Pad(pad) => {
                        p.put_source_as(*pad, sound.0.clone(), |name, layer, old| {
                            lands.pad(name, layer, old)
                        })?;
                        None
                    }
                    Landing::Take => Some(p.add_take_of(sound.0.clone(), |name, layer| {
                        lands.pad(name, layer, None)
                    })?),
                };
                if lands.tag != 0 {
                    p.tag_last(lands.tag);
                }
                return Ok(Made { take, ..made });
            }
            Change::AutoSampled { inst, to } => inst.0.clone().put(p, *to)?,
            Change::Processed {
                pad,
                layer,
                sound,
                scale,
            } => {
                p.use_processed(*pad, *layer, sound.0.clone(), *scale)?;
            }
            Change::OnTake { take, change } => {
                let now = p.take(*take).ok_or(ModelError::BadTake)?;
                let next = change
                    .next_pad(p, now)
                    .unwrap_or(Err(ModelError::BadTake))?;
                p.edit_take(*take, gesture, move |t| *t = next)?;
            }
            Change::NameTake { take, name } => {
                let name = name.trim();
                let now = p.take(*take).ok_or(ModelError::BadTake)?;
                if name.is_empty() {
                    return Err(ModelError::BadName);
                }
                if now.name == name {
                    return Ok(Made {
                        nothing: true,
                        ..made
                    });
                }
                p.edit_take(*take, None, |t| t.name = name.to_string())?;
            }
            Change::PutTake { take, pad } => p.put_take_on(*take, *pad)?,
            Change::LoadPad {
                pad,
                sounds,
                to,
                chops,
            } => {
                let sounds = sounds.iter().map(|s| s.0.clone()).collect();
                p.put_pad_with(*pad, sounds, (**to).clone(), chops.clone())?;
            }
            Change::SceneStore(slot) => p.scenes[slot.index()] = Some(scene::Scene::of(p)),
            Change::SceneReturn => {
                let Some(a) = p.scenes[0].clone() else {
                    return Ok(Made {
                        nothing: true,
                        ..made
                    });
                };
                let (n, left) = p.return_to(&a);
                return Ok(Made {
                    nothing: n == 0,
                    put_back: Some((n, left)),
                    ..made
                });
            }
        }
        Ok(made)
    }

    /// For one of a pad's own changes, what `now` (a pad or take of `p`)
    /// becomes; None for any other change.
    fn next_pad(&self, p: &Project, now: &Pad) -> Option<Result<Pad, ModelError>> {
        let layer = |l: usize| {
            if l < now.layers.len() {
                Ok(l)
            } else {
                Err(ModelError::BadPad)
            }
        };
        let mut next = now.clone();
        let made = match self {
            Change::Settings { to, .. } => return Some(with_regions_of(to, now)),
            Change::Own { field, value, .. } => {
                scene::set_own(&mut next, *field, *value);
                Ok(())
            }
            Change::Turn {
                knob,
                detents,
                fine,
                ..
            } => {
                turned(&mut next, *knob, *detents, *fine);
                Ok(())
            }
            Change::Region {
                layer: l, region, ..
            } => layer(*l).map(|l| next.layers[l].region = (**region).clone()),
            Change::Gain { layer: l, db, .. } => {
                layer(*l).map(|l| next.layers[l].region.gain_db = *db)
            }
            Change::GrainStart { layer: l, at, .. } => layer(*l).and_then(|l| {
                if (0.0..=1.0).contains(at) {
                    next.layers[l].region.grain.position = *at;
                    Ok(())
                } else {
                    Err(ModelError::BadRegion("grain start"))
                }
            }),
            Change::AddLayer { after, .. } => {
                if now.layers.len() >= MAX_LAYERS {
                    return Some(Err(ModelError::TooManyLayers));
                }
                return Some(with_layer_added(now, *after).ok_or(ModelError::BadPad));
            }
            Change::RemoveLayer { layer: l, .. } => {
                if now.layers.len() < 2 {
                    Err(ModelError::BadPad)
                } else {
                    layer(*l).map(|l| {
                        next.layers.remove(l);
                    })
                }
            }
            Change::LayerSound {
                layer: l, source, ..
            } => p
                .source(*source)
                .ok_or(ModelError::UnknownSource)
                .and_then(|s| {
                    let l = layer(*l)?;
                    let r = &mut next.layers[l].region;
                    *r = sound_region(Some(&*r), *source, s.frames());
                    Ok(())
                }),
            _ => return None,
        };
        Some(made.map(|()| next))
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
            | Change::LayerSound { pad, .. }
            | Change::PutTake { pad, .. }
            | Change::LoadPad { pad, .. }
            | Change::Processed { pad, .. } => Some(*pad),
            Change::PadsPlay(_)
            | Change::KeyFade(_)
            | Change::MapToKeys { .. }
            | Change::Chop { .. }
            | Change::KeepChop { .. }
            | Change::AutoSampled { .. }
            | Change::OnTake { .. }
            | Change::NameTake { .. }
            | Change::SceneStore(_)
            | Change::SceneReturn => None,
            Change::Keep { onto, .. } => match onto {
                Landing::Pad(pad) => Some(*pad),
                Landing::Take => None,
            },
        }
    }

    /// Whether it can rename a pad, replace it or change its type, which
    /// the Play page's pad names show.
    pub fn renames(&self) -> bool {
        match self {
            Change::Pad { .. }
            | Change::Chop { .. }
            | Change::AutoSampled { .. }
            | Change::PutTake { .. }
            | Change::LoadPad { .. }
            | Change::SceneReturn => true,
            Change::Keep { onto, .. } => matches!(onto, Landing::Pad(_)),
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
    type Shows = Box<dyn Fn(&Project) -> bool>;

    /// Whether two kits' pads, takes and what they play are the same.
    fn same(a: &Project, b: &Project) -> bool {
        (0..mpfree_sampler::model::PADS).all(|i| a.pad(i) == b.pad(i))
            && a.pads_play() == b.pads_play()
            && a.takes() == b.takes()
    }

    /// `base` with a copy of its pad 0 kept as a take on the page, and no
    /// undo history.
    fn with_take(base: &Project) -> Project {
        let mut p = base.clone();
        let take = p.pad(0).cloned().expect("pad 0");
        p.add_take(take).expect("a take");
        p.clear_history();
        p
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
        let cases: Vec<(Change, Shows)> = vec![
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
    fn a_recorded_sound_lands_on_a_pad_or_as_a_take_playing_as_it_says() {
        let (base, ..) = kit();
        let data: Vec<f32> = (0..2400).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let take = Source::new("Take 1", RATE, 1, data).expect("a take");
        let keep = |onto, lands| Change::Keep {
            sound: Recorded(take.clone()),
            onto,
            lands,
        };
        // On an empty pad: a kit's new pad, warped and faded.
        let mut p = base.clone();
        let before = steps(&mut p.clone());
        let lands = Lands {
            warp: Some(Warp::new(92.0)),
            fade: 48,
            ..Lands::default()
        };
        keep(Landing::Pad(6), lands)
            .apply(&mut p, None)
            .expect("kept");
        let pd = p.pad(6).expect("the new pad");
        assert_eq!(
            (pd.name.as_str(), pd.polyphony, pd.level_db),
            ("Take 1", KIT_POLYPHONY, CENTRE_DB)
        );
        let r = &pd.layers[0].region;
        assert_eq!(r.warp.as_ref().map(|w| w.bpm), Some(92.0));
        assert_eq!((r.fade_in.frames, r.fade_out.frames), (48, 48));
        assert!(p.source(take.id()).is_some(), "the sound joins the kit");
        assert_eq!(steps(&mut p.clone()), before + 1);
        // On a pad in use: it keeps its settings; a Flatten keeps only its
        // mute group, chokes and polyphony.
        let mut old = base.pad(0).unwrap().clone();
        old.level_db = -9.0;
        old.mute_group = Some(2);
        old.polyphony = 2;
        let mut q = base.clone();
        q.set_pad(0, Some(old), None).expect("the old pad");
        let mut r1 = q.clone();
        keep(Landing::Pad(0), Lands::default())
            .apply(&mut r1, None)
            .expect("replacing");
        assert_eq!(r1.pad(0).map(|pd| pd.level_db), Some(-9.0));
        let mut r2 = q.clone();
        let flat = Lands {
            flattened: true,
            ..Lands::default()
        };
        keep(Landing::Pad(0), flat)
            .apply(&mut r2, None)
            .expect("flattened");
        let pd = r2.pad(0).unwrap();
        assert_eq!(
            (pd.level_db, pd.mute_group, pd.polyphony),
            (CENTRE_DB, Some(2), 2)
        );
        // A grains capture plays as grains.
        let mut g = base.clone();
        let grains = GrainParams {
            density: 40.0,
            ..GrainParams::default()
        };
        let lands = Lands {
            grains: Some(grains),
            ..Lands::default()
        };
        keep(Landing::Pad(9), lands)
            .apply(&mut g, None)
            .expect("captured");
        let r = &g.pad(9).unwrap().layers[0].region;
        assert_eq!((r.mode, r.grain.density), (PlayMode::Granular, 40.0));
        // As a take on the page: after the takes there.
        let mut t = base.clone();
        keep(Landing::Take, Lands::default())
            .apply(&mut t, None)
            .expect("a take");
        assert_eq!(t.takes().len(), 1);
        assert!(
            (0..mpfree_sampler::model::PADS).all(|i| t.pad(i) == base.pad(i)),
            "no pad changed"
        );
        assert!(keep(Landing::Pad(0), Lands::default()).renames());
        assert!(!keep(Landing::Take, Lands::default()).renames());
        // A Recall's step goes by its name in the undo history.
        let mut rc = base.clone();
        let recall = Lands {
            tag: 1,
            ..Lands::default()
        };
        keep(Landing::Take, recall)
            .apply(&mut rc, None)
            .expect("recalled");
        assert_eq!(rc.history().0.last().map(|i| i.tag), Some(1));
        assert_eq!(steps(&mut rc.clone()), steps(&mut base.clone()) + 1);
    }

    #[test]
    fn a_pads_own_change_made_to_a_take_changes_the_take_as_it_would_the_pad() {
        let (base, _, other) = kit();
        let t0 = with_take(&base);
        let on_take = |change: Change| Change::OnTake {
            take: 0,
            change: Box::new(change),
        };
        let gain = |db| {
            on_take(Change::Gain {
                pad: 0,
                layer: 1,
                db,
            })
        };
        // One turn on the take: one step, no pad touched.
        let mut p = t0.clone();
        gain(-3.0).apply(&mut p, Some(5)).expect("on the take");
        gain(-6.0).apply(&mut p, Some(5)).expect("the same turn");
        assert_eq!(p.take(0).unwrap().layers[1].region.gain_db, -6.0);
        assert!(
            (0..mpfree_sampler::model::PADS).all(|i| p.pad(i) == t0.pad(i)),
            "no pad changed"
        );
        assert_eq!(steps(&mut p.clone()), 1, "one turn, one step");
        assert!(p.undo());
        assert!(same(&p, &t0), "undone");
        // Each of a pad's own changes makes of the take what it makes of
        // the pad the take was copied from.
        let mut quiet = base.pad(0).unwrap().clone();
        quiet.level_db = -12.0;
        let mut short = quiet.layers[0].region.clone();
        short.end = 1000;
        let changes = [
            Change::Settings {
                pad: 0,
                to: Box::new(quiet),
            },
            Change::Own {
                pad: 0,
                field: PadField::Pan,
                value: 0.25,
            },
            Change::Turn {
                pad: 0,
                knob: PadKnob::Decay,
                detents: -4,
                fine: true,
            },
            Change::Region {
                pad: 0,
                layer: 1,
                region: Box::new(short),
            },
            Change::GrainStart {
                pad: 0,
                layer: 0,
                at: 0.5,
            },
            Change::AddLayer { pad: 0, after: 1 },
            Change::RemoveLayer { pad: 0, layer: 0 },
            Change::LayerSound {
                pad: 0,
                layer: 1,
                source: other,
            },
        ];
        for change in changes {
            let (mut a, mut b) = (base.clone(), t0.clone());
            change.apply(&mut a, None).expect("on the pad");
            on_take(change.clone())
                .apply(&mut b, None)
                .unwrap_or_else(|e| panic!("{change:?} on the take: {e:?}"));
            assert_eq!(b.take(0), a.pad(0), "{change:?}");
            assert_eq!(b.pad(0), base.pad(0), "{change:?} left the pad");
        }
        // No take there, no such layer, or not a pad's own change: refused,
        // nothing changed.
        for change in [
            Change::OnTake {
                take: 3,
                change: Box::new(Change::Gain {
                    pad: 0,
                    layer: 0,
                    db: 1.0,
                }),
            },
            on_take(Change::Gain {
                pad: 0,
                layer: 9,
                db: 1.0,
            }),
            on_take(Change::PadsPlay(PadsPlay::Keys { from: 48 })),
            on_take(Change::Pad { pad: 0, to: None }),
        ] {
            let mut p = t0.clone();
            assert!(change.apply(&mut p, None).is_err(), "{change:?} refused");
            assert!(same(&p, &t0) && steps(&mut p) == 0, "{change:?}");
        }
        assert_eq!((gain(1.0).pad(), gain(1.0).renames()), (None, false));
    }

    #[test]
    fn a_take_is_named_and_sent_to_a_pad_one_step_each() {
        let (base, ..) = kit();
        let mut p = with_take(&base);
        let name = |n: &str| Change::NameTake {
            take: 0,
            name: n.to_string(),
        };
        let made = name("  Vox ").make(&mut p, None).expect("named");
        assert!(!made.nothing);
        assert_eq!(p.take(0).unwrap().name, "Vox");
        assert_eq!(steps(&mut p.clone()), 1);
        // The name it has already: nothing changes, no step.
        assert!(name("Vox").make(&mut p, None).expect("the same").nothing);
        assert_eq!(steps(&mut p.clone()), 1);
        assert!(matches!(
            name("   ").apply(&mut p, None),
            Err(ModelError::BadName)
        ));
        let none = Change::NameTake {
            take: 4,
            name: "X".into(),
        };
        assert!(matches!(none.apply(&mut p, None), Err(ModelError::BadTake)));
        // Send to pad: on the pad, off the page, one step.
        let put = Change::PutTake { take: 0, pad: 5 };
        put.apply(&mut p, None).expect("on pad 6");
        assert_eq!(p.pad(5).map(|pd| pd.name.as_str()), Some("Vox"));
        assert!(p.takes().is_empty());
        assert_eq!(steps(&mut p.clone()), 2);
        assert!(p.undo());
        assert_eq!((p.takes().len(), p.pad(5)), (1, None));
        assert_eq!((put.pad(), put.renames()), (Some(5), true));
        assert!(Change::PutTake {
            take: 0,
            pad: mpfree_sampler::model::PADS
        }
        .apply(&mut p, None)
        .is_err());
    }

    #[test]
    fn a_pad_loads_with_its_sounds_and_chops_as_one_step() {
        let (base, ..) = kit();
        let wave =
            |n: usize, f: f32| -> Vec<f32> { (0..n).map(|i| (i as f32 * f).sin() * 0.4).collect() };
        let s = Source::new("Loop", RATE, 1, wave(4800, 0.02)).expect("a sound");
        let stray = Source::new("Stray", RATE, 1, wave(2400, 0.03)).expect("a sound");
        let chop =
            ChopSet::from_markers("Loop", &s, ChopMode::Manual, [1200, 2400]).expect("a chop");
        let pad = kit_pad("Loop", Layer::new(Region::whole(&s)));
        let load = |sounds: Vec<Source>, chops: Vec<ChopSet>| Change::LoadPad {
            pad: 7,
            sounds: sounds.into_iter().map(Recorded).collect(),
            to: Box::new(pad.clone()),
            chops,
        };
        let mut p = base.clone();
        let before = steps(&mut p.clone());
        let loaded = load(vec![s.clone()], vec![chop.clone()]);
        loaded.apply(&mut p, None).expect("loaded");
        assert_eq!(p.pad(7), Some(&pad));
        assert!(p.source(s.id()).is_some());
        assert_eq!(steps(&mut p.clone()), before + 1);
        assert_eq!((loaded.pad(), loaded.renames()), (Some(7), true));
        assert!(p.undo());
        assert!(
            same(&p, &base) && p.source(s.id()).is_none(),
            "undone, sound and all"
        );
        // Without the sound it plays, or with a chop on a sound it doesn't
        // bring: refused, and no sound is left behind.
        let stray_chop =
            ChopSet::from_markers("Stray", &stray, ChopMode::Manual, [1200]).expect("a chop");
        for bad in [
            load(vec![], vec![]),
            load(vec![s.clone()], vec![stray_chop]),
        ] {
            let mut p = base.clone();
            assert!(bad.apply(&mut p, None).is_err());
            assert!(same(&p, &base) && p.source(s.id()).is_none());
            assert_eq!(steps(&mut p), before);
        }
    }

    #[test]
    fn a_processed_sound_takes_its_layers_place_in_one_step() {
        let (base, hit, _) = kit();
        let data: Vec<f32> = (0..9600).map(|i| (i as f32 * 0.025).sin() * 0.5).collect();
        let new = Source::new("Hit x2.00", RATE, 1, data).expect("the new sound");
        let mut p = base.clone();
        let before = steps(&mut p.clone());
        let used = Change::Processed {
            pad: 0,
            layer: 1,
            sound: Recorded(new.clone()),
            scale: 2.0,
        };
        used.apply(&mut p, None).expect("used");
        let pd = p.pad(0).unwrap();
        assert_eq!(pd.layers[1].region.source, new.id());
        assert_eq!(pd.layers[1].region.end, new.frames());
        assert_eq!(pd.layers[0].region.source, hit, "the other layer as it was");
        assert_eq!(steps(&mut p.clone()), before + 1);
        assert!(p.undo());
        assert!(same(&p, &base) && p.source(new.id()).is_none(), "undone");
        assert_eq!((used.pad(), used.renames()), (Some(0), false));
    }

    #[test]
    fn scene_a_comes_back_in_one_step_and_storing_is_no_step() {
        let (base, ..) = kit();
        let mut p = base.clone();
        let before = steps(&mut p.clone());
        let back = Change::SceneReturn
            .make(&mut p, None)
            .expect("nothing stored");
        assert!(back.nothing && back.put_back.is_none());
        Change::SceneStore(SceneSlot::A)
            .apply(&mut p, None)
            .expect("stored");
        assert!(p.scenes[0].is_some() && p.scenes[1].is_none());
        assert_eq!(steps(&mut p.clone()), before, "storing is no undo step");
        let stored = p.clone();
        Change::Own {
            pad: 0,
            field: PadField::Pan,
            value: 0.5,
        }
        .apply(&mut p, None)
        .expect("panned");
        let back = Change::SceneReturn.make(&mut p, None).expect("back");
        assert_eq!((back.nothing, back.put_back), (false, Some((1, 0))));
        assert!(same(&p, &stored));
        assert_eq!(steps(&mut p.clone()), before + 2);
        let again = Change::SceneReturn.make(&mut p, None).expect("again");
        assert_eq!((again.nothing, again.put_back), (true, Some((0, 0))));
        assert_eq!(steps(&mut p.clone()), before + 2);
        assert!(Change::SceneReturn.renames() && Change::SceneStore(SceneSlot::B).plays());
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

//! What the pads play: the engine's view of each kit ([`EngineView`]), built
//! off the audio thread from a kit's model, and swapping a view in (a new kit,
//! or an edit of the one playing).

use super::*;

#[derive(Clone, Debug)]
pub struct ViewLoop {
    pub(super) start: i64,
    pub(super) end: i64,
    pub(super) crossfade: i64,
    pub(super) ping_pong: bool,
}

/// A layer resolved into plain numbers and a handle to its audio.
#[derive(Clone, Debug)]
pub struct ViewLayer {
    pub(super) data: Pcm,
    pub(super) stereo: bool,
    pub(super) pick: ChannelPick,
    pub(super) start: i64,
    pub(super) end: i64,
    pub(super) sustain_loop: Option<ViewLoop>,
    pub(super) fade_in: Fade,
    pub(super) fade_out: Fade,
    pub(super) reverse: bool,
    pub(super) mode: PlayMode,
    pub(super) gain_l: f32,
    pub(super) gain_r: f32,
    /// The region's and the layer's gain, without the pad's level and pan,
    /// and the layer's own pan: live Level and Pan start from these.
    pub(super) unit_gain: f32,
    pub(super) pan: f32,
    /// Source frames per output frame, 32.32 fixed point.
    pub(super) step: i64,
    pub(super) vel_lo: u8,
    pub(super) vel_hi: u8,
    pub(super) src_rate: u32,
    /// Grain settings with the layer's tune and reverse folded in.
    pub(super) grain: mpfree_grain::Params,
    /// The layer's tune in semitones, as `grain` has it (for its own grain
    /// settings with live values in place, [`Live::layer_grain`]).
    pub(super) semis: f32,
    /// The region's grain settings as edited, before the tune and reverse
    /// go in: what a grain setting's parameter reads (sampler refactor
    /// step 17, [`EngineView::layer_grain`]).
    pub(super) own_grain: mpfree_grain::Params,
    /// Its routes from velocity, pressure and the key (step 17c).
    pub(super) routes: GrainRoutes,
    /// The layer's place on its pad (a layer whose sound is missing has
    /// no view, so the view's places can skip one).
    pub(super) place: u8,
    /// What a key or a tune moves ([`pitched_grain`]): a Tone region's
    /// density (its note), or the grains' pitch and, with the region's key
    /// tracking, their size and the scan.
    pub(super) follow: KeyFollow,
    /// Frames a granular voice takes to glide to a new key (0, none).
    pub(super) glide: u32,
    /// The tempo the layer's audio is at, when it is warped (0 if not):
    /// the voice plays it `tempo / warp_bpm` times as fast.
    pub(super) warp_bpm: f64,
    /// It plays a stretched copy, not the source.
    pub(super) copy: bool,
    /// For a stretched copy: where each source frame lands in it.
    pub(super) source_at: Option<CopyMap>,
    /// The sound's octave copies, for a clean read past an octave up
    /// (none for a Vintage pad, a granular region or a stretched copy).
    pub(super) octaves: Option<Arc<OctaveCell>>,
    /// The sound's hits, for a region in the Stretch grain mode
    /// ([`crate::hits`]).
    pub(super) hits: Option<Arc<HitCell>>,
    /// A sliced region's boundaries inside it, in the frames it reads
    /// (R4); empty when it isn't sliced or plays as grains.
    pub(super) cuts: Vec<i64>,
    /// Its slices' own settings, by slice number (R4 part 5), ready to
    /// use: built with the view, so a hit only reads them. Empty when
    /// none has any or it isn't sliced.
    pub(super) slice_sets: Vec<ViewSlice>,
}

/// A slice's own settings as a voice uses them ([`SliceSet`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ViewSlice {
    /// Cents from the pad's pitch.
    pub(super) cents: f32,
    /// Its level as a gain.
    pub(super) gain: f32,
    pub(super) reverse: bool,
    /// A low-pass cutoff in place of the pad's filter's, Hz; 0 for none.
    pub(super) cutoff_hz: f32,
    /// Its fade out, in the frames the layer reads; None: the region's.
    pub(super) fade_out: Option<u32>,
    pub(super) to_end: bool,
}

impl ViewSlice {
    pub(super) const PLAIN: ViewSlice = ViewSlice {
        cents: 0.0,
        gain: 1.0,
        reverse: false,
        cutoff_hz: 0.0,
        fade_out: None,
        to_end: false,
    };

    /// `s` for a layer reading `rate` frames a second.
    pub(super) fn of(s: &SliceSet, rate: u32) -> ViewSlice {
        ViewSlice {
            cents: f32::from(s.tune_st) * 100.0,
            gain: db_to_gain(s.level_db),
            reverse: s.reverse,
            cutoff_hz: s.cutoff_hz.unwrap_or(0.0),
            fade_out: s
                .fade_out_ms
                .map(|ms| (f64::from(ms) * f64::from(rate) / 1000.0).round() as u32),
            to_end: s.to_end,
        }
    }
}

impl ViewLayer {
    /// Whether the layer plays a stretched copy (and how long it is).
    pub fn stretched_frames(&self) -> Option<i64> {
        (self.warp_bpm > 0.0 && self.copy).then_some(self.end - self.start)
    }
}

#[derive(Clone, Debug)]
pub struct ViewPad {
    pub(super) layers: Vec<ViewLayer>,
    pub(super) select: LayerSelect,
    pub(super) polyphony: u8,
    pub(super) mute_group: Option<u8>,
    pub(super) chokes: Vec<usize>,
    pub(super) links: Vec<usize>,
    pub(super) latch: bool,
    pub(super) amp: Ahdsr,
    pub(super) filter: Filter,
    pub(super) filter_env: Ahdsr,
    pub(super) vel_to_level: f32,
    pub(super) vel_to_start: f32,
    pub(super) pressure: PressureRoute,
    pub(super) keys: Option<Keys>,
    /// A keygroup of one key that plays one of the kit's chop slices
    /// (Chop to keys): its key picks the slice, so the kit's tuning
    /// doesn't move it, as it doesn't move a sliced pad's.
    pub(super) slice_key: bool,
    pub(super) wow: f32,
    pub(super) key_fade: u8,
    pub(super) vel_fade: u8,
    /// The pad's own level, pan and tune, for live settings to replace.
    pub(super) level_db: f32,
    pub(super) pan: f32,
    pub(super) tune_cents: i32,
    /// Pitched as older samplers do: no band-limited read.
    pub(super) vintage: bool,
    /// The slice a hit plays on its sliced layers.
    pub(super) slice: u8,
    /// Its LFO, pitch envelope and velocity on pitch and cutoff (sampler
    /// R5 part 4), in their ranges.
    pub(super) lfo: PadLfo,
    pub(super) pitch_env: PitchEnv,
    pub(super) vel_to_cutoff: f32,
    pub(super) vel_to_pitch: f32,
    /// What moves its filter per hit (the filter gap plan's F3), in their
    /// ranges: the key, velocity on the envelope's depth and attack, and
    /// each hit's random.
    pub(super) key_track: f32,
    pub(super) vel_to_env: f32,
    pub(super) vel_to_attack: f32,
    pub(super) cutoff_random: f32,
    pub(super) res_random: f32,
    /// Mono, Legato and Glide for its keys (sampler R5.5).
    pub(super) mono: Mono,
    /// A live hit waits for the launch line ([`crate::model::Pad::on_launch`]):
    /// the host waits, the engine only says so
    /// ([`Engine::kit_starts_on_line`]).
    pub(super) on_launch: bool,
}

impl ViewPad {
    /// Whether its keys play one note at a time.
    pub(super) fn mono(&self) -> bool {
        self.mono.on && self.keys.is_some()
    }

    /// Whether a layer of it is sliced: a note on keys picks a slice, at
    /// the sound's own pitch.
    pub(super) fn sliced(&self) -> bool {
        self.layers.iter().any(|l| !l.cuts.is_empty())
    }

    /// Whether its keys play the kit's tuning: a keygroup, not sliced and
    /// not a slice on a key of its own.
    pub(super) fn tuned(&self) -> bool {
        self.keys.is_some() && !self.slice_key && !self.sliced()
    }
}

/// Whether pad `p` is a keygroup of one key whose every layer plays a
/// slice of one of `project`'s chop sets, as Chop to keys makes them.
fn slice_key(project: &Project, p: &crate::model::Pad) -> bool {
    p.keys.is_some_and(|k| k.lo == k.hi)
        && !p.layers.is_empty()
        && p.layers.iter().all(|l| {
            let r = &l.region;
            project.chop_sets.iter().any(|c| {
                c.source == r.source
                    && (0..c.slices()).any(|i| c.bounds(i) == Some((r.start, r.end)))
            })
        })
}

/// What the audio thread needs from the project, for every pad.
#[derive(Clone, Debug)]
pub struct EngineView {
    pub revision: u64,
    pub(super) pads: Vec<Option<ViewPad>>,
    /// The take the Sample page shows, kept off the pads (the one Sample
    /// page's decision 1): played by [`Event::TakeOn`], its voices on
    /// [`TAKE_PAD`].
    pub(super) take: Option<ViewPad>,
    pub(super) master: MasterFx,
    pub(super) pads_play: PadsPlay,
    /// How notes reach the kit, as its model says ([`KitKeys`]).
    pub(super) keys: KitKeys,
}

/// How a note from a keyboard, a MIDI route or a clip of notes reaches a
/// kit (its [`KeysPlay`], sampler step 15 and live shows decision 6),
/// resolved with its view so the audio thread decides in one step and
/// always agrees with what the kit holds. Fixed size, no heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KitKeys {
    /// What keys play ([`Project::keys_play`]: as picked, else by what the
    /// kit holds).
    pub play: KeysPlay,
    /// The notes some keygroup with a sound holds, bit n for note n.
    pub held: u128,
    /// The note pad 1 plays while the pads play notes
    /// ([`PadsPlay::Keys`]), else None.
    pub pads_from: Option<u8>,
}

impl KitKeys {
    /// A kit of drum pads, as every kit was before keys had a setting.
    pub const PADS: KitKeys = KitKeys {
        play: KeysPlay::Pads,
        held: 0,
        pads_from: None,
    };

    /// Whether some keygroup with a sound holds note `note`.
    pub fn holds(self, note: u8) -> bool {
        note < 128 && self.held >> note & 1 == 1
    }

    /// Whether note `note` plays the keygroups (as a key, through every one
    /// that holds it), not the pad that sends it: always on Keygroups, on
    /// Both where a keygroup holds it, never on Pads.
    pub fn to_keygroups(self, note: u8) -> bool {
        match self.play {
            KeysPlay::Pads => false,
            KeysPlay::Keygroups => true,
            KeysPlay::Both => self.holds(note),
        }
    }

    /// Whether the kit is played as notes, like a synth: its keys play the
    /// keygroups (Keygroups) and its pads play notes too
    /// ([`PadsPlay::Keys`]), so a key the pads reach can go through the
    /// pad that plays its note (the track's arp and REC take it as they
    /// take that pad).
    pub fn takes_notes(self) -> bool {
        self.play == KeysPlay::Keygroups && self.pads_from.is_some()
    }

    /// The pad that plays note `note` while the pads play notes (pad 1 is
    /// [`KitKeys::pads_from`], each pad a semitone up), else None.
    pub fn pad_of_note(self, note: u8) -> Option<u8> {
        let from = self.pads_from?;
        note.checked_sub(from)
            .filter(|&p| usize::from(p) < PADS && note < 128)
    }

    /// The note pad `pad` plays while the pads play notes, else None.
    pub fn note_of_pad(self, pad: usize) -> Option<u8> {
        let n = usize::from(self.pads_from?) + pad;
        (pad < PADS && n < 128).then_some(n as u8)
    }
}

/// The pad number a take's voices carry: past every pad, so no table
/// indexed by pad reaches it (mpfree-claude
/// `sampler/onepage/take-slot-check.md`).
pub const TAKE_PAD: u8 = u8::MAX;

impl EngineView {
    /// Whether a live hit on pad `i` waits for the launch line while the
    /// transport plays (sampler R5.3b): a pad with a sound that has
    /// [`crate::model::Pad::on_launch`], while the pads play pads (not
    /// notes, not 16 Levels).
    pub fn starts_on_line(&self, i: usize) -> bool {
        matches!(self.pads_play, PadsPlay::Pads)
            && self
                .pads
                .get(i)
                .and_then(Option::as_ref)
                .is_some_and(|p| p.on_launch && !p.layers.is_empty())
    }

    /// Whether a hit on pad `i` plays something.
    pub fn has_pad(&self, i: usize) -> bool {
        let sounds = |p: &Option<ViewPad>| p.as_ref().is_some_and(|p| !p.layers.is_empty());
        match self.pads_play {
            PadsPlay::Pads => self.pads.get(i).is_some_and(sounds),
            // With the pads playing notes: a keygroup holds its note.
            PadsPlay::Keys { from } => {
                let note = usize::from(from) + i;
                note < 128
                    && self.pads.iter().any(|p| {
                        sounds(p)
                            && p.as_ref()
                                .and_then(|p| p.keys)
                                .is_some_and(|k| k.contains(note as u8))
                    })
            }
            // With 16 Levels: every pad plays the one pad.
            PadsPlay::Levels { pad, .. } => self.pads.get(usize::from(pad)).is_some_and(sounds),
        }
    }

    /// What the pads play.
    pub fn pads_play(&self) -> PadsPlay {
        self.pads_play
    }

    /// Pad `pad`'s own value of `field`, as edited: what [`Event::PadLive`]
    /// with NaN goes back to, so a caller can give its changes as offsets
    /// on it. Cutoff, Resonance, Drive and envelope depth only where the
    /// pad has a filter; None for a pad without a sound.
    pub fn pad_own(&self, pad: usize, field: PadField) -> Option<f32> {
        let p = self
            .pads
            .get(pad)?
            .as_ref()
            .filter(|p| !p.layers.is_empty())?;
        let filtered = p.filter.kind != FilterKind::Off;
        match field {
            PadField::Level => Some(p.level_db),
            PadField::Pan => Some(p.pan),
            PadField::Tune => Some(p.tune_cents as f32),
            PadField::Cutoff => filtered.then_some(p.filter.cutoff_hz),
            PadField::Resonance => filtered.then_some(p.filter.resonance),
            PadField::Attack => Some(p.amp.attack_ms),
            PadField::Decay => Some(p.amp.decay_ms),
            PadField::Release => Some(p.amp.release_ms),
            PadField::Slice => Some(f32::from(p.slice)),
            PadField::LfoDepth => Some(p.lfo.depth),
            PadField::PitchEnv => Some(p.pitch_env.semitones),
            PadField::PitchDecay => Some(p.pitch_env.decay_ms),
            PadField::Drive => filtered.then_some(p.filter.drive_db),
            PadField::EnvDepth => filtered.then_some(p.filter.env_octaves),
        }
    }

    /// Pad `pad`'s layer `layer`'s grain settings as edited (its region's,
    /// without the tune and reverse the voice plays them with): what a
    /// grain setting's parameter reads (sampler refactor step 17). Kept in
    /// every mode, as the region keeps them. None for a pad without a
    /// sound or a layer it hasn't got.
    pub fn layer_grain(&self, pad: usize, layer: usize) -> Option<&mpfree_grain::Params> {
        let p = self.pads.get(pad)?.as_ref()?;
        let v = p.layers.iter().find(|v| usize::from(v.place) == layer)?;
        Some(&v.own_grain)
    }

    /// How many slices pad `pad` plays from: its most sliced layer's, 1
    /// when none is sliced (or there is no such pad).
    pub fn pad_slices(&self, pad: usize) -> usize {
        self.pads
            .get(pad)
            .and_then(|p| p.as_ref())
            .and_then(|p| p.layers.iter().map(|l| l.cuts.len() + 1).max())
            .unwrap_or(1)
    }

    /// Layer `layer` of pad `pad`, as resolved.
    pub fn layer(&self, pad: usize, layer: usize) -> Option<&ViewLayer> {
        self.pads.get(pad)?.as_ref()?.layers.get(layer)
    }

    pub fn empty() -> EngineView {
        EngineView {
            revision: 0,
            pads: vec![None; PADS],
            take: None,
            master: MasterFx::default(),
            pads_play: PadsPlay::Pads,
            keys: KitKeys::PADS,
        }
    }

    /// How notes reach the kit ([`KitKeys`]).
    pub fn keys(&self) -> KitKeys {
        self.keys
    }

    /// Resolves every pad of a project for an engine running at `rate`.
    /// Call this off the audio thread; it allocates. Warped regions play
    /// as they were recorded; [`EngineView::build_warped`] plays them at
    /// the project tempo.
    pub fn build(project: &Project, rate: u32) -> EngineView {
        Self::resolve(project, rate, None, None)
    }

    /// As [`EngineView::build`], with warped regions at the tempo of
    /// `warps`: a region warped to stretch plays its stretched copy when
    /// `warps` holds it, and repitches to the tempo until then; one warped
    /// to repitch always repitches. The engine needs the same tempo
    /// ([`Engine::set_tempo`]).
    pub fn build_warped(project: &Project, rate: u32, warps: &mut Warps) -> EngineView {
        Self::resolve(project, rate, Some(warps), None)
    }

    /// As [`EngineView::build_warped`], with take `take` of the project
    /// (the one the Sample page shows) ready to play ([`Event::TakeOn`]).
    pub fn build_with_take(
        project: &Project,
        rate: u32,
        warps: &mut Warps,
        take: Option<usize>,
    ) -> EngineView {
        Self::resolve(project, rate, Some(warps), take)
    }

    /// Whether the view has a take to play.
    pub fn has_take(&self) -> bool {
        self.take.as_ref().is_some_and(|t| !t.layers.is_empty())
    }

    /// Pad `p`'s view, or the take's for [`TAKE_PAD`].
    pub(super) fn pad_or_take(&self, p: usize) -> Option<&ViewPad> {
        if p == usize::from(TAKE_PAD) {
            self.take.as_ref()
        } else {
            self.pads.get(p).and_then(Option::as_ref)
        }
    }

    pub(super) fn resolve(
        project: &Project,
        rate: u32,
        mut warps: Option<&mut Warps>,
        take: Option<usize>,
    ) -> EngineView {
        let mut pads = Vec::with_capacity(PADS);
        // Pad `i`'s view (a take's: `i` is PADS, past every pad).
        let mut view_of = |i: usize, p: &crate::model::Pad| {
            let pad_gain = db_to_gain(p.level_db);
            let layers = p
                .layers
                .iter()
                .enumerate()
                .filter_map(|(li, l)| {
                    let s = project.source(l.region.source)?;
                    let r = &l.region;
                    let (pl, pr) = pan_gains(l.pan + p.pan);
                    let unit_gain = db_to_gain(r.gain_db) * db_to_gain(l.level_db);
                    let g = unit_gain * pad_gain;
                    let cents = f64::from(r.tune_cents + l.tune_cents + p.tune_cents);
                    let ratio = f64::from(s.rate()) / f64::from(rate) * (cents / 1200.0).exp2();
                    let follow = KeyFollow {
                        tone: r.grain_mode == GrainMode::Tone,
                        size: r.grain_keys.size,
                        scan: r.grain_keys.scan,
                    };
                    let mut grain = pitched_grain(r.grain, (cents / 100.0) as f32, follow);
                    // A stretch of a sound with a tempo keeps time.
                    if r.grain_mode == GrainMode::Stretch {
                        grain.scan_tempo = r.warp.as_ref().map(|w| w.bpm);
                    }
                    if r.reverse {
                        grain.reverse = 1.0;
                    }
                    let mut v = ViewLayer {
                        data: s.data().clone(),
                        stereo: s.channels() == 2,
                        pick: r.channels,
                        start: r.start as i64,
                        end: r.end as i64,
                        sustain_loop: r.sustain_loop.map(|lp| ViewLoop {
                            start: lp.start as i64,
                            end: lp.end as i64,
                            crossfade: i64::from(lp.crossfade),
                            ping_pong: lp.mode == LoopMode::PingPong,
                        }),
                        fade_in: r.fade_in,
                        fade_out: r.fade_out,
                        reverse: r.reverse,
                        mode: r.mode,
                        gain_l: g * pl,
                        gain_r: g * pr,
                        unit_gain,
                        pan: l.pan,
                        step: ((ratio * ONE as f64).round() as i64).max(1),
                        vel_lo: l.vel_lo,
                        vel_hi: l.vel_hi,
                        src_rate: s.rate(),
                        grain,
                        semis: (cents / 100.0) as f32,
                        own_grain: r.grain,
                        routes: r.grain_routes,
                        place: li as u8,
                        follow,
                        glide: (r.grain_glide_ms.clamp(0.0, GRAIN_GLIDE_MAX_MS) / 1000.0
                            * rate as f32) as u32,
                        warp_bpm: 0.0,
                        copy: false,
                        source_at: None,
                        octaves: None,
                        hits: None,
                        cuts: if r.mode == PlayMode::Granular {
                            Vec::new()
                        } else {
                            r.cuts().map(|x| x as i64).collect()
                        },
                        slice_sets: if r.mode == PlayMode::Granular || r.slice_count() < 2 {
                            Vec::new()
                        } else {
                            r.slice_sets
                                .iter()
                                .take(r.slice_count())
                                .map(|x| ViewSlice::of(x, s.rate()))
                                .collect()
                        },
                    };
                    if r.mode == PlayMode::Granular && r.grain_mode == GrainMode::Stretch {
                        hits::prepare(s.hits(), s.data(), s.channels(), s.rate());
                        v.hits = Some(s.hits().clone());
                    }
                    if !p.vintage {
                        v.octaves = Some(s.octaves().clone());
                        // Grains go as far up as their pitch and its
                        // spray take them.
                        let up = if r.mode == PlayMode::Granular {
                            let g = &r.grain;
                            let semis = g.pitch.max(0.0) + g.pitch_spray.clamp(0.0, 24.0);
                            ratio * f64::from(semis / 12.0).exp2()
                        } else {
                            // A slice's own pitch up (R4 part 5).
                            let st = r.slice_sets.iter().map(|x| x.tune_st).max();
                            ratio * (f64::from(st.unwrap_or(0).max(0)) / 12.0).exp2()
                        };
                        if may_pass_an_octave(p, up, project.pads_play()) {
                            bandlimit::prepare(s.octaves(), s.data(), s.channels());
                        }
                    }
                    let warp = r.warp.as_ref().filter(|_| r.mode != PlayMode::Granular);
                    if let (Some(w), Some(ws)) = (warp, warps.as_deref_mut()) {
                        // Tune is in the copy, or left out (repitch):
                        // the tempo alone sets the speed.
                        let plain = f64::from(s.rate()) / f64::from(rate);
                        v.step = ((plain * ONE as f64).round() as i64).max(1);
                        v.warp_bpm = f64::from(w.bpm);
                        let copy = match w.method {
                            WarpMethod::Stretch => ws
                                .key_for(r, l.tune_cents + p.tune_cents)
                                .and_then(|k| ws.copy(&k).map(|d| (k, d))),
                            WarpMethod::Repitch => None,
                        };
                        if let Some((k, data)) = copy {
                            let scale = k.scale();
                            let map = CopyMap::new(w, s.rate(), &k);
                            let at = |x: u64| map.at(x as f64).round() as i64;
                            let len = k.frames as i64;
                            v.data = Pcm::F32(data);
                            v.start = 0;
                            v.end = len;
                            v.cuts = r.cuts().map(at).filter(|&x| x > 0 && x < len).collect();
                            v.cuts.dedup();
                            v.sustain_loop = r.sustain_loop.map(|lp| {
                                let (a, b) = (at(lp.start), at(lp.end).min(len));
                                ViewLoop {
                                    start: a,
                                    end: b.max(a + 1),
                                    crossfade: ((f64::from(lp.crossfade) * scale) as i64)
                                        .min((b - a) / 2)
                                        .min(a),
                                    ping_pong: lp.mode == LoopMode::PingPong,
                                }
                            });
                            let fade = |f: Fade| Fade {
                                frames: (f64::from(f.frames) * scale).round() as u32,
                                ..f
                            };
                            v.fade_in = fade(r.fade_in);
                            v.fade_out = fade(r.fade_out);
                            if i64::from(v.fade_in.frames) + i64::from(v.fade_out.frames) > len {
                                (v.fade_in, v.fade_out) = (r.fade_in, r.fade_out);
                            }
                            // The copy is at the tempo it was made for.
                            v.warp_bpm *= (r.frames() as f64) / (len as f64);
                            v.copy = true;
                            v.source_at = Some(map);
                            // The copies are of the source, not this.
                            v.octaves = None;
                        }
                    }
                    Some(v)
                })
                .collect();
            ViewPad {
                layers,
                select: p.select,
                polyphony: p.polyphony,
                mute_group: p.mute_group,
                chokes: p.chokes.clone(),
                links: p
                    .links
                    .iter()
                    .copied()
                    .filter(|&l| l < PADS && l != i)
                    .collect(),
                latch: p.latch,
                on_launch: p.on_launch,
                amp: p.amp,
                // Bypassed, a hit plays as if the filter were off.
                filter: if p.filter_bypass {
                    Filter {
                        kind: FilterKind::Off,
                        ..p.filter
                    }
                } else {
                    p.filter
                },
                filter_env: p.filter_env,
                vel_to_level: p.vel_to_level.clamp(0.0, 1.0),
                vel_to_start: p.vel_to_start.clamp(0.0, 1.0),
                pressure: p.pressure,
                keys: p.keys,
                slice_key: slice_key(project, p),
                wow: p.wow.clamp(0.0, 1.0),
                key_fade: p.key_fade.min(crate::model::MAX_KEY_FADE),
                vel_fade: p.vel_fade.min(crate::model::MAX_VEL_FADE),
                level_db: p.level_db,
                pan: p.pan,
                tune_cents: p.tune_cents,
                vintage: p.vintage,
                slice: p.slice,
                lfo: PadLfo {
                    rate_hz: p
                        .lfo
                        .rate_hz
                        .clamp(*LFO_RATE_HZ.start(), *LFO_RATE_HZ.end()),
                    beats: p
                        .lfo
                        .beats
                        .map(|b| b.clamp(*LFO_BEATS.start(), *LFO_BEATS.end())),
                    depth: p.lfo.depth.clamp(-1.0, 1.0),
                    ..p.lfo
                },
                pitch_env: PitchEnv {
                    semitones: p.pitch_env.semitones.clamp(-PITCH_ENV_ST, PITCH_ENV_ST),
                    decay_ms: p
                        .pitch_env
                        .decay_ms
                        .clamp(*PITCH_DECAY_MS.start(), *PITCH_DECAY_MS.end()),
                },
                vel_to_cutoff: p.vel_to_cutoff.clamp(-1.0, 1.0),
                vel_to_pitch: p.vel_to_pitch.clamp(-12.0, 12.0),
                key_track: p.key_track.clamp(-1.0, 1.0),
                vel_to_env: p.vel_to_env.clamp(0.0, 1.0),
                vel_to_attack: p.vel_to_attack.clamp(0.0, 1.0),
                cutoff_random: p.cutoff_random.clamp(0.0, 1.0),
                res_random: p.res_random.clamp(0.0, 1.0),
                mono: Mono {
                    glide_ms: p.mono.glide_ms.clamp(*GLIDE_MS.start(), *GLIDE_MS.end()),
                    ..p.mono
                },
            }
        };
        for i in 0..PADS {
            // A pad of another kind (an engine's voice, a note out) plays
            // nothing here: the app sends its hits where they go.
            let pad = project.pad(i).filter(|p| p.kind.sounds());
            pads.push(pad.map(|p| view_of(i, p)));
        }
        // The take the Sample page shows, kept off the pads.
        let take = take.and_then(|t| project.take(t)).map(|p| view_of(PADS, p));
        // The notes some keygroup with a sound holds, for Both.
        let held = pads
            .iter()
            .flatten()
            .filter(|p| !p.layers.is_empty())
            .filter_map(|p| p.keys)
            .fold(0u128, |bits, k| {
                (k.lo..=k.hi.min(127)).fold(bits, |b, n| b | 1 << n)
            });
        let keys = KitKeys {
            play: project.keys_play(),
            held,
            pads_from: match project.pads_play() {
                PadsPlay::Keys { from } => Some(from),
                _ => None,
            },
        };
        EngineView {
            revision: project.revision(),
            pads,
            take,
            master: *project.master(),
            pads_play: project.pads_play(),
            keys,
        }
    }
}

/// Whether a layer of pad `p` read `ratio` times its sound's rate (its
/// rate against the output's, and its tune) may be played more than an
/// octave faster than that sound, so its octave copies should be made: a
/// keygroup (its notes can reach anywhere), or tune, pitch pressure, 16
/// Levels on tune and wow together reaching twice the sound's speed.
pub(super) fn may_pass_an_octave(p: &crate::model::Pad, ratio: f64, play: PadsPlay) -> bool {
    if p.keys.is_some() {
        return true;
    }
    let mut up = ratio;
    if p.pressure.target == PressureTarget::Pitch {
        up *= f64::from(p.pressure.depth.abs()).exp2();
    }
    if matches!(
        play,
        PadsPlay::Levels {
            by: Levels::Tune,
            ..
        }
    ) {
        up *= (7.0f64 / 12.0).exp2();
    }
    if p.wow > 0.0 {
        up *= 1.02;
    }
    up >= 1.9
}

impl Engine {
    /// How many kits the engine plays ([`Engine::with_kits`]).
    pub fn kits(&self) -> usize {
        self.views.len()
    }

    /// [`EngineView::pad_own`] of kit `kit`'s pad, as its view has it
    /// (MIDI Learn reads where a learned pad setting stands).
    pub fn kit_pad_own(&self, kit: usize, pad: usize, field: PadField) -> Option<f32> {
        self.views.get(kit)?.pad_own(pad, field)
    }

    /// [`EngineView::layer_grain`] of kit `kit`'s pad.
    pub fn kit_layer_grain(
        &self,
        kit: usize,
        pad: usize,
        layer: usize,
    ) -> Option<&mpfree_grain::Params> {
        self.views.get(kit)?.layer_grain(pad, layer)
    }

    /// [`EngineView::pad_slices`] of kit `kit`'s pad (1 past the kits).
    pub fn kit_pad_slices(&self, kit: usize, pad: usize) -> usize {
        self.views.get(kit).map_or(1, |v| v.pad_slices(pad))
    }

    /// Whether a live hit on kit `kit`'s pad `pad` waits for the launch
    /// line ([`EngineView::starts_on_line`]). Never allocates.
    pub fn kit_starts_on_line(&self, kit: usize, pad: usize) -> bool {
        self.views.get(kit).is_some_and(|v| v.starts_on_line(pad))
    }

    /// Swaps in a new view at once and hands back the old one, so the caller
    /// frees it off the audio thread. Playing voices keep their own handle to
    /// their audio and finish as they started: this is for another kit,
    /// whose pads are other pads ([`Engine::edit_view`] is for an edit).
    /// The master effect takes the new view's settings (a new mix glides
    /// there over 10 ms). The pads' live settings ([`Event::PadLive`]) go
    /// back to the pads' own; the program's offsets stay.
    pub fn set_view(&mut self, view: EngineView) -> EngineView {
        self.set_kit_view(0, view)
    }

    /// [`Engine::set_view`] for kit `kit`: the other kits and their voices
    /// are left as they are. The master effect takes the new view's
    /// settings only if it follows this kit ([`Engine::set_master_kit`]).
    /// A kit out of range gets nothing, and `view` comes back.
    pub fn set_kit_view(&mut self, kit: usize, view: EngineView) -> EngineView {
        let Some(slot) = self.views.get_mut(kit) else {
            return view;
        };
        let old = std::mem::replace(slot, view);
        let live = &mut self.lives[kit];
        live.pads = Live::OWN;
        live.clear_layer_lives();
        live.master = [f32::NAN; GrainField::COUNT + 1];
        self.edit_lane[kit] = false;
        if kit == self.master_kit {
            self.apply_master();
        }
        old
    }

    /// Swaps in an edit of the view playing now (the same kit, edited) as
    /// [`Engine::set_view`] does, and the voices playing hear it: each
    /// voice whose layer still plays the same sound (the same pad and
    /// layer, the same audio) picks up the layer's loop points at once,
    /// and its level, pan, cutoff, resonance and tune as a live setting
    /// does (gliding, so nothing clicks; a new tune at once, as a knob
    /// would). A loop moved behind the voice takes it into the loop, the
    /// step smoothed over [`JUMP_FRAMES`]; a loop taken away lets the
    /// voice play on to its end; a granular voice takes the new grain
    /// settings. What it started with stays: its start and end, fades,
    /// play mode, direction, envelope, filter type and the sound itself
    /// (an edit that changes those, or puts another sound on the pad, is
    /// heard from the next hit). Never allocates.
    pub fn edit_view(&mut self, view: EngineView) -> EngineView {
        self.edit_kit_view(0, view)
    }

    /// [`Engine::edit_view`] for kit `kit`: its voices hear the edit, the
    /// other kits' voices are left as they are. A kit out of range gets
    /// nothing, and `view` comes back.
    pub fn edit_kit_view(&mut self, kit: usize, view: EngineView) -> EngineView {
        let Some(live) = self.lives.get(kit) else {
            return view;
        };
        // The pads whose voices follow a live value, which goes now.
        let live_pads: [bool; PADS] = std::array::from_fn(|p| {
            PadField::ALL
                .iter()
                .any(|&f| f.follows() && !live.pads[p][f as usize].is_nan())
        });
        let old = self.set_kit_view(kit, view);
        let Engine {
            pool,
            views,
            lives,
            tunings,
            tempo,
            ..
        } = self;
        let (view, live, t) = (&views[kit], &lives[kit], &tunings[kit]);
        for (v, g) in pool.iter_mut() {
            let p = usize::from(v.pad);
            // A pad's voices, and the take's (TAKE_PAD: no live values).
            if v.active && usize::from(v.kit) == kit && (p < PADS || v.pad == TAKE_PAD) {
                follow_edit(
                    v,
                    g,
                    view,
                    live,
                    &live.program_of(v, t),
                    live_pads.get(p).copied().unwrap_or(false),
                    *tempo,
                );
            }
        }
        self.edit_lane[kit] = self.pool.split && self.pool.b.is_none();
        old
    }

    /// The revision of kit 0's view.
    pub fn view_revision(&self) -> u64 {
        self.kit_revision(0)
    }

    /// The revision of kit `kit`'s view (0 for a kit out of range).
    pub fn kit_revision(&self, kit: usize) -> u64 {
        self.views.get(kit).map_or(0, |v| v.revision)
    }
}

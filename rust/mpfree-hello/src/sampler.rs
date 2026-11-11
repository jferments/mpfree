//! The pad player's sound from round 6: MPFree's sampler engine
//! (`mpfree-sampler`) in place of the round 3 mixer in `kit.rs`.
//!
//! - Each kit in `kits.bin` becomes a sampler project: each sound is a
//!   source, played whole and one-shot from the pad that sends its note.
//!   Levels match the old mixer (velocity scales the level linearly, and a
//!   sound plays at full scale at velocity 127).
//! - One more kit, "Grains", plays the first kit's sounds as grain clouds
//!   while a pad is held. Pressing harder changes the cloud, by row: the
//!   bottom row (pads 1-4) makes it denser, pads 5-8 make the grains longer,
//!   pads 9-12 move where they read, and the top row (pads 13-16) raises the
//!   pitch up to an octave.
//! - Every engine view is built here, before the sound starts. Picking a kit
//!   swaps views, so the audio loop never builds, allocates or frees one;
//!   sounds already playing finish, as before.
//! - The kits' projects go to the screen thread for the Sample tab
//!   ([`Player::take_projects`]). An edit there builds the kit's view again
//!   on the screen thread and sends it here in a [`ViewSwap`];
//!   [`Player::swap_view`] puts it in place and hands the old view back in
//!   the same box, so the screen thread frees it.

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event, PadOut};
use mpfree_sampler::model::{
    GrainParams, Layer, Pad, PlayMode, PressureRoute, PressureTarget, Project, Region, Source,
};

use crate::kit::{Kit, Sound, MAX_KITS, MAX_VOICES};

/// The engine pans a centred sound −3 dB a side. Pads are raised by this
/// much, so a kit plays at the level the round 3 mixer played it.
const CENTRE_DB: f32 = 3.010_3;

/// What each row of the Grains kit's pads changes with pressure: target,
/// depth.
const GRAIN_ROWS: [(PressureTarget, f32); 4] = [
    (PressureTarget::GrainDensity, 0.75), // ×8 at full pressure
    (PressureTarget::GrainSize, 0.75),    // ×4.8
    (PressureTarget::GrainPosition, 0.5), // half way through the sound
    (PressureTarget::Pitch, 1.0),         // an octave up
];

/// Counters for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub steals: u64,
    pub dropped_events: u64,
    pub peak_grains: usize,
    pub skipped_grains: u64,
}

pub struct Player {
    engine: Engine,
    /// One view per kit. The playing kit's view is inside the engine, so its
    /// place here is empty.
    views: Vec<Option<EngineView>>,
    kit: usize,
    /// Pads with a sound, per kit.
    sounds: Vec<[bool; 16]>,
    peak_grains: usize,
    /// Each kit's project, until the screen thread takes them.
    projects: Vec<Project>,
}

/// A kit's engine view, built again after an edit on the Sample tab, on its
/// way to the audio thread; then the view it replaced, on its way back to
/// the screen thread, which frees it. It travels boxed, so the channels
/// move a pointer and the audio thread never frees anything.
#[derive(Clone, Debug)]
pub struct ViewSwap {
    pub kit: usize,
    pub view: Option<EngineView>,
}

impl PartialEq for ViewSwap {
    /// The same kit and the same project revision.
    fn eq(&self, other: &ViewSwap) -> bool {
        self.kit == other.kit
            && self.view.as_ref().map(|v| v.revision) == other.view.as_ref().map(|v| v.revision)
    }
}

/// How many playheads [`Playheads`] holds.
pub const PLAYHEADS: usize = 8;

/// Where the playing kit's voices are, in source frames (first layers
/// only), for the playheads on the Sample tab. Small and fixed, so the
/// audio thread sends it without allocating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playheads {
    pub kit: u16,
    pub len: u8,
    pub pad: [u8; PLAYHEADS],
    pub frame: [u32; PLAYHEADS],
}

impl Playheads {
    /// The frames of pad `pad`'s voices.
    pub fn of(&self, pad: usize) -> impl Iterator<Item = u64> + '_ {
        let n = usize::from(self.len).min(PLAYHEADS);
        self.pad[..n]
            .iter()
            .zip(&self.frame[..n])
            .filter(move |(p, _)| usize::from(**p) == pad)
            .map(|(_, f)| u64::from(*f))
    }
}

/// The sound's audio as interleaved stereo floats at full scale ±1.
fn floats(s: &Sound) -> Vec<f32> {
    s.audio.iter().map(|&x| f32::from(x) / 32768.0).collect()
}

/// The velocity to send the engine for a hit at `v`. The engine's level
/// is (velocity / 127)²; this makes it v / 127, as the old mixer's was.
pub fn engine_velocity(v: u8) -> u8 {
    if v == 0 {
        return 0;
    }
    let x = f32::from(v.min(127)) / 127.0;
    ((x.sqrt() * 127.0).round() as u8).clamp(1, 127)
}

fn kit_pad(name: &str, region: Region) -> Pad {
    let mut pad = Pad::new(name, Layer::new(region));
    pad.polyphony = MAX_VOICES as u8;
    pad.level_db = CENTRE_DB;
    pad
}

fn grain_pad(name: &str, source: &Source, row: usize) -> Pad {
    let mut r = Region::whole(source);
    r.mode = PlayMode::Granular;
    r.grain = GrainParams {
        position: 0.1,
        spray: 0.05,
        size_ms: 60.0,
        density: 25.0,
        pitch_spray: 0.1,
        scan: 0.25,
        spread: 0.7,
        ..GrainParams::default()
    };
    let mut pad = kit_pad(name, r);
    pad.polyphony = 2;
    pad.amp = Ahdsr {
        attack_ms: 5.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 250.0,
    };
    let (target, depth) = GRAIN_ROWS[row];
    pad.pressure = PressureRoute { target, depth };
    pad
}

impl Player {
    /// Builds every kit's engine view (and, if there is room, the Grains
    /// kit, which is added to `kits` for the screen and the log). The kits'
    /// own audio is handed over to the engine, so their `audio` is empty
    /// afterwards. `pad_notes[p]` is the note pad `p` sends. Call it before
    /// the sound starts: it allocates.
    pub fn new(kits: &mut Vec<Kit>, pad_notes: &[u8; 16], rate: u32) -> Result<Player, String> {
        if kits.is_empty() {
            return Err("no kits".to_string());
        }
        let mut views = Vec::with_capacity(kits.len() + 1);
        let mut sounds = Vec::with_capacity(kits.len() + 1);
        let mut projects = Vec::with_capacity(kits.len() + 1);
        let mut first: Vec<(usize, Source)> = Vec::new();
        for (k, kit) in kits.iter_mut().enumerate() {
            let mut project = Project::new();
            let mut has = [false; 16];
            for (p, &note) in pad_notes.iter().enumerate() {
                let Some(s) = kit.sounds.iter_mut().find(|s| s.note == note) else {
                    continue;
                };
                let source = Source::new(&s.name, rate, 2, floats(s))
                    .map_err(|e| format!("kit {} ({}), {}: {e:?}", k + 1, kit.name, s.name))?;
                s.audio = Vec::new();
                project.add_source(source.clone());
                project
                    .set_pad(p, Some(kit_pad(&s.name, Region::whole(&source))), None)
                    .map_err(|e| format!("kit {} ({}), {}: {e:?}", k + 1, kit.name, s.name))?;
                has[p] = true;
                if k == 0 {
                    first.push((p, source));
                }
            }
            views.push(Some(EngineView::build(&project, rate)));
            sounds.push(has);
            projects.push(project);
        }
        if kits.len() < MAX_KITS {
            let mut project = Project::new();
            let mut has = [false; 16];
            let mut grain_sounds = Vec::new();
            for (p, source) in &first {
                project.add_source(source.clone());
                let name = kits[0]
                    .sounds
                    .iter()
                    .find(|s| s.note == pad_notes[*p])
                    .map_or(String::new(), |s| s.name.clone());
                project
                    .set_pad(*p, Some(grain_pad(&name, source, p / 4)), None)
                    .map_err(|e| format!("Grains, {name}: {e:?}"))?;
                has[*p] = true;
                grain_sounds.push(Sound {
                    note: pad_notes[*p],
                    name,
                    audio: Vec::new(),
                });
            }
            views.push(Some(EngineView::build(&project, rate)));
            sounds.push(has);
            projects.push(project);
            kits.push(Kit {
                name: format!("Grains: {}", kits[0].name),
                sounds: grain_sounds,
            });
        }
        let mut engine = Engine::new(rate, MAX_VOICES);
        engine.set_view(views[0].take().expect("built above"));
        Ok(Player {
            engine,
            views,
            kit: 0,
            sounds,
            peak_grains: 0,
            projects,
        })
    }

    #[cfg(test)]
    pub fn kits(&self) -> usize {
        self.views.len()
    }

    pub fn kit(&self) -> usize {
        self.kit
    }

    /// Which of kit `kit`'s 16 pads have a sound (none past the last kit).
    pub fn sounds(&self, kit: usize) -> [bool; 16] {
        self.sounds.get(kit).copied().unwrap_or([false; 16])
    }

    /// Play kit `k` from the next hit on. Sounds already playing finish.
    /// Swaps views: it never allocates or frees.
    pub fn select(&mut self, k: usize) {
        if k == self.kit || k >= self.views.len() {
            return;
        }
        let Some(view) = self.views[k].take() else {
            return;
        };
        self.views[self.kit] = Some(self.engine.set_view(view));
        self.kit = k;
    }

    /// Every kit's project, in kit order (the Grains kit last), for the
    /// Sample tab. The audio is shared with the engine's views, not copied.
    /// Afterwards the player holds none.
    pub fn take_projects(&mut self) -> Vec<Project> {
        std::mem::take(&mut self.projects)
    }

    /// Puts `swap`'s view in place for kit `swap.kit` (in the engine, if
    /// that kit is playing) and leaves the view it replaces in `swap`, to
    /// be freed off the audio thread. Sounds already playing finish as they
    /// started; the next hit plays the new view, and the pads with a sound
    /// are the view's (a chop adds some). Never allocates or frees.
    /// Returns false, leaving `swap` as it was, if there is no such kit.
    pub fn swap_view(&mut self, swap: &mut ViewSwap) -> bool {
        let Some(view) = swap.view.take() else {
            return false;
        };
        // A chop can put sounds on pads that had none (and undo take them away).
        if let Some(has) = self.sounds.get_mut(swap.kit) {
            for (p, h) in has.iter_mut().enumerate() {
                *h = view.has_pad(p);
            }
        }
        if swap.kit == self.kit {
            swap.view = Some(self.engine.set_view(view));
        } else if let Some(slot) = self.views.get_mut(swap.kit) {
            swap.view = slot.replace(view);
        } else {
            swap.view = Some(view);
            return false;
        }
        true
    }

    /// Where the playing kit's voices are. Never allocates.
    pub fn playheads(&self) -> Playheads {
        let mut p = Playheads {
            kit: self.kit.min(usize::from(u16::MAX)) as u16,
            ..Playheads::default()
        };
        for (pad, layer, at) in self.engine.playheads() {
            let n = usize::from(p.len);
            if layer != 0 || n >= PLAYHEADS {
                continue;
            }
            p.pad[n] = pad;
            p.frame[n] = at.min(u64::from(u32::MAX)) as u32;
            p.len += 1;
        }
        p
    }

    /// Starts pad `pad`'s sound `offset` frames into the next render.
    /// Returns false if the playing kit has no sound on that pad.
    pub fn hit_at(&mut self, pad: usize, velocity: u8, offset: u32) -> bool {
        if pad >= 16 || !self.sounds[self.kit][pad] {
            return false;
        }
        let velocity = engine_velocity(velocity);
        if velocity > 0 {
            self.engine.schedule(
                offset,
                Event::NoteOn {
                    pad: pad as u8,
                    velocity,
                },
            );
        }
        true
    }

    /// The pad was let go, `offset` frames into the next render. One-shot
    /// sounds ignore it; a Grains pad stops making grains and fades out.
    pub fn release_at(&mut self, pad: usize, offset: u32) {
        if pad < 16 {
            self.engine
                .schedule(offset, Event::NoteOff { pad: pad as u8 });
        }
    }

    /// Plays source frames `start..end` of pad `pad`'s layer `layer` once,
    /// from the next render, as the pad made from that slice will play it
    /// (a tap on the Chop page). Only while kit `kit` is the one playing;
    /// returns whether it went to the engine.
    pub fn audition(
        &mut self,
        kit: usize,
        pad: usize,
        layer: usize,
        start: u64,
        end: u64,
        velocity: u8,
    ) -> bool {
        if kit != self.kit || pad >= 16 || layer > usize::from(u8::MAX) {
            return false;
        }
        let clip = |f: u64| u32::try_from(f).unwrap_or(u32::MAX);
        let velocity = engine_velocity(velocity);
        velocity > 0
            && self.engine.schedule(
                0,
                Event::Audition {
                    pad: pad as u8,
                    layer: layer as u8,
                    start: clip(start),
                    end: clip(end),
                    velocity,
                },
            )
    }

    /// Pad pressure, 0 to 127, from the next render on.
    pub fn pressure(&mut self, pad: usize, value: u8) {
        if pad < 16 {
            self.engine.schedule(
                0,
                Event::Pressure {
                    pad: pad as u8,
                    value,
                },
            );
        }
    }

    /// The project tempo, for warped pads (cheap when it hasn't moved:
    /// call it every period).
    pub fn set_tempo(&mut self, bpm: f64) {
        self.engine.set_tempo(bpm);
    }

    /// Fades every sound out (over a few milliseconds; a Grains pad over its
    /// release).
    pub fn stop_all(&mut self) {
        self.engine.apply(Event::AllOff);
    }

    pub fn playing(&self) -> usize {
        self.engine.active_voices()
    }

    pub fn counters(&self) -> Counters {
        let s = self.engine.stats();
        Counters {
            steals: s.steals,
            dropped_events: s.dropped_events,
            peak_grains: self.peak_grains,
            skipped_grains: s.skipped_grains,
        }
    }

    /// Renders the next period into `left` and `right`, scaled by `master`.
    /// Returns the largest absolute sample, before clamping to −1..1 (as the
    /// old mixer did).
    #[cfg(test)]
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], master: f32) -> f32 {
        self.render_dry(left, right);
        master_out(left, right, master)
    }

    /// The engine's sound alone, before the level: the pad player adds the
    /// synth to it, then calls [`master_out`].
    pub fn render_dry(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.engine.render(left, right);
        self.peak_grains = self.peak_grains.max(self.engine.live_grains());
    }

    /// The next `frames` frames with each pad of the playing kit apart, for
    /// the mixer's pad channels: pad `p` (0 to 15) adds its sound into
    /// `out.pad(p)` (see [`Engine::render_routed`]). The pads add up to what
    /// [`Player::render_dry`] renders.
    // The mixer's wiring (round 8) calls it; until then only the tests do.
    #[allow(dead_code)]
    pub fn render_routed(&mut self, frames: usize, out: &mut impl PadOut) {
        self.engine.render_routed(frames, out);
        self.peak_grains = self.peak_grains.max(self.engine.live_grains());
    }
}

/// The level, then the clip to full scale. Returns the peak before the
/// clip (above 1.0 means the period clipped).
pub fn master_out(left: &mut [f32], right: &mut [f32], master: f32) -> f32 {
    let mut peak = 0f32;
    for s in left.iter_mut().chain(right.iter_mut()) {
        *s *= master;
        peak = peak.max(s.abs());
        *s = s.clamp(-1.0, 1.0);
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{self, Mixer};

    const NOTES: [u8; 16] = [
        0x25, 0x24, 0x2A, 0x52, 0x28, 0x26, 0x2E, 0x2C, 0x30, 0x2F, 0x2D, 0x2B, 0x31, 0x37, 0x33,
        0x35,
    ];

    fn tone(frames: usize, f: f32) -> Vec<i16> {
        (0..frames)
            .flat_map(|i| {
                let x = (i as f32 * f).sin();
                [(x * 12000.0) as i16, (x * -9000.0) as i16]
            })
            .collect()
    }

    fn kits() -> Vec<Kit> {
        let a = kit::build(&[
            (NOTES[0], "Kick", &tone(3000, 0.01)[..]),
            (NOTES[5], "Snare", &tone(2000, 0.07)[..]),
        ]);
        let b = kit::build(&[(NOTES[0], "Clap", &tone(1500, 0.03)[..])]);
        kit::parse_kits(&kit::build_kits(&[("House", a), ("Trap", b)])).unwrap()
    }

    fn run(p: &mut Player, periods: usize) -> (Vec<f32>, f32) {
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let mut out = Vec::new();
        let mut peak = 0f32;
        for _ in 0..periods {
            peak = peak.max(p.render(&mut l, &mut r, 0.5));
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        (out, peak)
    }

    fn run_mixer(m: &mut Mixer, periods: usize) -> Vec<f32> {
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let mut out = Vec::new();
        for _ in 0..periods {
            m.render(&mut l, &mut r, 0.5);
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    #[test]
    fn kits_play_as_the_old_mixer_played_them() {
        for (velocity, tolerance) in [(127u8, 1e-4f32), (100, 1e-2), (40, 1e-2)] {
            let mut mixer = Mixer::new(kits());
            let mut ks = kits();
            let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
            // Two hits, the second on its exact frame in the second period.
            assert!(mixer.hit_at(NOTES[0], velocity, 3).is_some());
            assert!(p.hit_at(0, velocity, 3));
            let (want, got) = (run_mixer(&mut mixer, 1), run(&mut p, 1).0);
            mixer.hit_at(NOTES[5], velocity, 77);
            p.hit_at(5, velocity, 77);
            let want: Vec<f32> = want.into_iter().chain(run_mixer(&mut mixer, 30)).collect();
            let got: Vec<f32> = got.into_iter().chain(run(&mut p, 30).0).collect();
            for (k, (a, b)) in want.iter().zip(&got).enumerate() {
                assert!(
                    (a - b).abs() <= tolerance * a.abs().max(0.01),
                    "velocity {velocity}, sample {k}: {a} vs {b}"
                );
            }
            assert!(want.iter().any(|&x| x.abs() > 0.1));
            assert_eq!((mixer.playing(), p.playing()), (0, 0));
        }
    }

    /// Sixteen pad channels, cleared after each block as the mixer clears its own.
    struct Channels([[[f32; 128]; 2]; 16]);

    impl PadOut for Channels {
        fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
            let [l, r] = self.0.get_mut(pad)?;
            Some((&mut l[..], &mut r[..]))
        }
    }

    #[test]
    fn routed_pads_land_on_their_own_channels_and_add_up_to_the_dry_render() {
        let (mut ka, mut kb) = (kits(), kits());
        let mut dry = Player::new(&mut ka, &NOTES, 44_100).unwrap();
        let mut routed = Player::new(&mut kb, &NOTES, 44_100).unwrap();
        let mut ch = Channels([[[0.0; 128]; 2]; 16]);
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let mut peaks = [0f32; 16];
        for period in 0..30 {
            if period == 1 {
                for p in [&mut dry, &mut routed] {
                    assert!(p.hit_at(0, 110, 7));
                    assert!(p.hit_at(5, 90, 64));
                }
            }
            dry.render_dry(&mut l, &mut r);
            routed.render_routed(128, &mut ch);
            for k in 0..128 {
                let sum: f32 = ch.0.iter().map(|c| c[0][k]).sum();
                assert!(
                    (sum - l[k]).abs() < 1e-6,
                    "period {period}, frame {k}: {sum} vs {}",
                    l[k]
                );
            }
            for (pad, c) in ch.0.iter_mut().enumerate() {
                peaks[pad] =
                    peaks[pad].max(c[0].iter().chain(&c[1]).fold(0f32, |m, x| m.max(x.abs())));
                *c = [[0.0; 128]; 2];
            }
        }
        for (pad, peak) in peaks.iter().enumerate() {
            assert_eq!(*peak > 0.05, pad == 0 || pad == 5, "pad {pad}: peak {peak}");
        }
        assert_eq!((dry.playing(), routed.playing()), (0, 0));
    }

    #[test]
    fn a_slice_plays_once_on_the_playing_kit_only() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        // Kick (pad 1) is 3000 frames: frames 1000 to 1500 of it.
        assert!(!p.audition(1, 0, 0, 1000, 1500, 127), "kit 2 isn't playing");
        assert!(!p.audition(0, 16, 0, 1000, 1500, 127));
        assert!(!p.audition(0, 0, 0, 1000, 1500, 0));
        assert!(p.audition(0, 0, 0, 1000, 1500, 127));
        let (out, _) = run(&mut p, 8);
        let last = out.iter().rposition(|x| x.abs() > 1e-6).unwrap() / 2;
        assert!(
            (490..500).contains(&last),
            "the slice is 500 frames (its sine may touch zero at the end): {last}"
        );
        assert_eq!(p.playing(), 0);
    }

    #[test]
    fn a_pad_without_a_sound_reports_it() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert!(!p.hit_at(1, 127, 0));
        assert!(!p.hit_at(16, 127, 0));
        assert!(p.hit_at(5, 127, 0));
        p.select(1);
        assert!(!p.hit_at(5, 127, 0), "the second kit has nothing on pad 6");
    }

    #[test]
    fn switching_kits_lets_playing_sounds_finish() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        p.hit_at(0, 127, 0);
        run(&mut p, 1);
        p.select(1);
        p.select(9); // out of range: ignored
        assert_eq!(p.kit(), 1);
        p.hit_at(0, 127, 0);
        run(&mut p, 1);
        assert_eq!(p.playing(), 2, "the kick keeps going under the clap");
        run(&mut p, 30);
        assert_eq!(p.playing(), 0);
        // And back: the first kit's view is intact.
        p.select(0);
        assert!(p.hit_at(5, 127, 0));
        let (out, _) = run(&mut p, 2);
        assert!(out.iter().any(|&x| x != 0.0));
    }

    #[test]
    fn stop_fades_everything_out_quickly() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        for _ in 0..5 {
            p.hit_at(0, 127, 0);
        }
        run(&mut p, 2);
        p.stop_all();
        let (out, _) = run(&mut p, 4); // 5 ms release, within 4 periods
        assert_eq!(p.playing(), 0);
        assert!(out[out.len() - 256..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn steals_past_the_voice_limit_and_clamps_but_reports_the_peak() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        for _ in 0..MAX_VOICES + 5 {
            p.hit_at(0, 127, 0);
        }
        let (out, peak) = run(&mut p, 2);
        assert_eq!(p.playing(), MAX_VOICES);
        assert!(p.counters().steals >= 5);
        assert!(peak > 1.0, "{peak}");
        assert!(out.iter().all(|&x| (-1.0..=1.0).contains(&x)));
    }

    #[test]
    fn the_grains_kit_plays_the_first_kit_as_clouds_while_held() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert_eq!((ks.len(), p.kits()), (3, 3));
        assert_eq!(ks[2].name, "Grains: House");
        let names: Vec<(u8, &str)> = ks[2]
            .sounds
            .iter()
            .map(|s| (s.note, s.name.as_str()))
            .collect();
        assert_eq!(names, [(NOTES[0], "Kick"), (NOTES[5], "Snare")]);
        assert!(
            ks.iter()
                .all(|k| k.sounds.iter().all(|s| s.audio.is_empty())),
            "audio handed to the engine"
        );
        p.select(2);
        assert!(p.hit_at(5, 127, 0));
        p.pressure(5, 127); // pad 6 is on the second row: longer grains
        run(&mut p, 100);
        assert!(p.counters().peak_grains >= 2, "{:?}", p.counters());
        assert_eq!(p.playing(), 1, "still sounding while held");
        p.release_at(5, 0);
        run(&mut p, 150); // the 250 ms release and the last grains
        assert_eq!(p.playing(), 0);
    }

    #[test]
    fn sixteen_kits_leave_no_room_for_grains() {
        let one = kit::build(&[(NOTES[0], "Kick", &tone(100, 0.01)[..])]);
        let names: Vec<String> = (0..MAX_KITS).map(|k| format!("Kit {k}")).collect();
        let all: Vec<(&str, Vec<u8>)> = names.iter().map(|n| (n.as_str(), one.clone())).collect();
        let mut ks = kit::parse_kits(&kit::build_kits(&all)).unwrap();
        let p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert_eq!((ks.len(), p.kits()), (MAX_KITS, MAX_KITS));
    }

    #[test]
    fn an_edited_view_swaps_in_without_allocating_and_the_old_one_goes_back() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        let mut projects = p.take_projects();
        assert_eq!(projects.len(), 3, "two kits and Grains");
        assert!(p.take_projects().is_empty());
        let (whole, _) = {
            p.hit_at(0, 127, 0);
            run(&mut p, 30)
        };
        // The kick's start moves 1000 frames on: the sound starts later in the audio.
        projects[0]
            .edit_pad(0, None, |pad| pad.layers[0].region.start = 1000)
            .unwrap();
        let revision = projects[0].revision();
        let mut swap = Box::new(ViewSwap {
            kit: 0,
            view: Some(EngineView::build(&projects[0], 44_100)),
        });
        let (ok, calls) = crate::sys::alloc_count::calls(|| p.swap_view(&mut swap));
        assert!(ok);
        assert_eq!(calls, 0, "the swap allocates and frees nothing");
        assert_ne!(
            swap.view.as_ref().unwrap().revision,
            revision,
            "the old view comes back in the box"
        );
        p.hit_at(0, 127, 0);
        let (trimmed, _) = run(&mut p, 30);
        assert_ne!(whole, trimmed);
        assert_eq!(&whole[2000..2100], &trimmed[..100], "plays from frame 1000");
        // A kit not playing takes it in its place; out of range gives it back.
        let mut other = Box::new(ViewSwap {
            kit: 1,
            view: Some(EngineView::build(&projects[1], 44_100)),
        });
        assert!(p.swap_view(&mut other));
        assert!(other.view.is_some());
        let mut none = Box::new(ViewSwap {
            kit: 7,
            view: Some(EngineView::build(&projects[1], 44_100)),
        });
        assert!(!p.swap_view(&mut none));
        assert_eq!(
            none.view.as_ref().map(|v| v.revision),
            Some(projects[1].revision())
        );
        p.select(1);
        assert!(p.hit_at(0, 127, 0));
    }

    #[test]
    fn a_chopped_kit_plays_its_new_pads_once_its_view_swaps_in() {
        use mpfree_sampler::model::ChopSet;
        let mut kits = kits();
        let mut player = Player::new(&mut kits, &NOTES, 44_100).unwrap();
        let mut projects = player.take_projects();
        let empty = (0..16)
            .find(|&p| !player.sounds(0)[p])
            .expect("a pad without a sound in kit 1");
        assert!(!player.hit_at(empty, 100, 0));
        let from = (0..16).find(|&p| player.sounds(0)[p]).unwrap();
        let project = &mut projects[0];
        let source = project
            .source(project.pad(from).unwrap().layers[0].region.source)
            .unwrap()
            .clone();
        let c = ChopSet::grid("chop", &source, 2).unwrap();
        let template = project.pad(from).cloned();
        project
            .chop_to_pads(c, 0, 1, empty, template.as_ref())
            .unwrap();
        let mut swap = ViewSwap {
            kit: 0,
            view: Some(EngineView::build(project, 44_100)),
        };
        assert!(player.swap_view(&mut swap));
        assert!(player.sounds(0)[empty] && player.hit_at(empty, 100, 0));
        // Undone, the pad is silent again.
        project.undo();
        let mut swap = ViewSwap {
            kit: 0,
            view: Some(EngineView::build(project, 44_100)),
        };
        assert!(player.swap_view(&mut swap));
        assert!(!player.hit_at(empty, 100, 0));
    }

    #[test]
    fn playheads_follow_the_playing_kits_voices() {
        let mut ks = kits();
        let mut p = Player::new(&mut ks, &NOTES, 44_100).unwrap();
        assert_eq!(p.playheads().len, 0);
        p.hit_at(0, 127, 0);
        p.hit_at(5, 127, 64);
        run(&mut p, 4);
        let (heads, calls) = crate::sys::alloc_count::calls(|| p.playheads());
        assert_eq!(calls, 0);
        assert_eq!((heads.kit, heads.len), (0, 2));
        let kick: Vec<u64> = heads.of(0).collect();
        let snare: Vec<u64> = heads.of(5).collect();
        assert_eq!((kick.len(), snare.len()), (1, 1));
        assert!(kick[0] > snare[0] && kick[0] <= 512, "{kick:?} {snare:?}");
        assert_eq!(heads.of(3).count(), 0);
    }

    #[test]
    fn velocity_maps_to_a_linear_level() {
        assert_eq!(engine_velocity(0), 0);
        assert_eq!(engine_velocity(127), 127);
        for v in 1..=127u8 {
            let level = (f32::from(engine_velocity(v)) / 127.0).powi(2);
            let want = f32::from(v) / 127.0;
            assert!(
                (level - want).abs() <= 0.01,
                "velocity {v}: {level} vs {want}"
            );
        }
    }
}

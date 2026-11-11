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

use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event};
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
        })
    }

    #[cfg(test)]
    pub fn kits(&self) -> usize {
        self.views.len()
    }

    pub fn kit(&self) -> usize {
        self.kit
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
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], master: f32) -> f32 {
        self.engine.render(left, right);
        self.peak_grains = self.peak_grains.max(self.engine.live_grains());
        let mut peak = 0f32;
        for s in left.iter_mut().chain(right.iter_mut()) {
            *s *= master;
            peak = peak.max(s.abs());
            *s = s.clamp(-1.0, 1.0);
        }
        peak
    }
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

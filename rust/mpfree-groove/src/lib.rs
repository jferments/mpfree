//! Host-first MPFree M2: deterministic step sequencing and fixed-voice sampling.
//!
//! Everything in the render path uses preallocated storage: audio sample banks
//! and timed events are prepared before playback. No device acquisition here.
#![forbid(unsafe_code)]

use std::f64::consts::PI;

pub const STEPS_PER_BAR: usize = 16;
pub const MAX_VOICES: usize = 16;
pub const MAX_BARS: u32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drum {
    Kick,
    Snare,
    Hat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Step {
    /// MIDI-style velocity in 0..=127; 0 means no trigger.
    pub kick: u8,
    pub snare: u8,
    pub hat: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pub steps: [Step; STEPS_PER_BAR],
}

impl Pattern {
    pub fn empty() -> Self {
        Self {
            steps: [Step::default(); STEPS_PER_BAR],
        }
    }

    /// Two kicks, backbeat snares and eighth-note hats, all per bar.
    pub fn basic() -> Self {
        let mut out = Self::empty();
        for i in 0..STEPS_PER_BAR {
            if i == 0 || i == 8 {
                out.steps[i].kick = 120;
            }
            if i == 4 || i == 12 {
                out.steps[i].snare = 105;
            }
            if i % 2 == 0 {
                out.steps[i].hat = 62;
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub frame: u64,
    pub drum: Drum,
    pub velocity: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    SampleRate,
    Tempo,
    Bars,
    ChannelLengthMismatch,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, Copy)]
pub struct GrooveConfig {
    pub sample_rate: u32,
    pub bpm: f64,
    pub bars: u32,
}

impl Default for GrooveConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            bpm: 120.0,
            bars: 2,
        }
    }
}

impl GrooveConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(8_000..=192_000).contains(&self.sample_rate) {
            return Err(ConfigError::SampleRate);
        }
        if !self.bpm.is_finite() || !(20.0..=400.0).contains(&self.bpm) {
            return Err(ConfigError::Tempo);
        }
        if !(1..=MAX_BARS).contains(&self.bars) {
            return Err(ConfigError::Bars);
        }
        Ok(())
    }

    pub fn duration_frames(&self) -> Result<u64, ConfigError> {
        self.validate()?;
        Ok(
            (f64::from(self.bars) * 4.0 * 60.0 * f64::from(self.sample_rate) / self.bpm).round()
                as u64,
        )
    }
}

/// Compile a repeated 16-step pattern to an immutable event list *off* the RT thread.
/// Schedule positions are absolute: rounding each step from the origin prevents drift.
/// Events for the same frame remain ordered Kick → Snare → Hat.
pub fn compile_pattern(pattern: &Pattern, config: GrooveConfig) -> Result<Vec<Event>, ConfigError> {
    config.validate()?;
    let mut events = Vec::with_capacity(config.bars as usize * STEPS_PER_BAR * 3);
    for bar in 0..config.bars {
        for step in 0..STEPS_PER_BAR {
            let position = u64::from(bar) * STEPS_PER_BAR as u64 + step as u64;
            let frame = (position as f64 * 60.0 * f64::from(config.sample_rate)
                / (config.bpm * 4.0))
                .round() as u64;
            let current = pattern.steps[step];
            for (drum, velocity) in [
                (Drum::Kick, current.kick),
                (Drum::Snare, current.snare),
                (Drum::Hat, current.hat),
            ] {
                if velocity != 0 {
                    events.push(Event {
                        frame,
                        drum,
                        velocity,
                    });
                }
            }
        }
    }
    Ok(events)
}

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    drum: Drum,
    position: usize,
    gain: f32,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            drum: Drum::Kick,
            position: 0,
            gain: 0.0,
        }
    }
}

struct SampleBank {
    kick: Box<[f32]>,
    snare: Box<[f32]>,
    hat: Box<[f32]>,
}

fn next_noise(state: &mut u32) -> f32 {
    // Deterministic xorshift generator, seeded with a fixed nonzero constant.
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *state = s;
    (f64::from(s) / f64::from(u32::MAX) * 2.0 - 1.0) as f32
}

impl SampleBank {
    /// Sound synthesis only at construction, never inside render().
    fn new(sample_rate: u32) -> Self {
        let sr = f64::from(sample_rate);
        let kick_length = (0.24 * sr).round() as usize;
        let snare_length = (0.16 * sr).round() as usize;
        let hat_length = (0.09 * sr).round() as usize;
        let mut kick = Vec::with_capacity(kick_length);
        let mut snare = Vec::with_capacity(snare_length);
        let mut hat = Vec::with_capacity(hat_length);

        let mut phase = 0.0f64;
        for i in 0..kick_length {
            let t = i as f64 / sr;
            let freq = 48.0 + 105.0 * (-t * 28.0).exp();
            phase += 2.0 * PI * freq / sr;
            let envelope = (-t * 26.0).exp() * (1.0 - (-t * 1500.0).exp());
            kick.push((0.72 * envelope * phase.sin()) as f32);
        }

        let mut state = 0x1234_5678u32;
        let mut previous = 0.0f32;
        for i in 0..snare_length {
            let t = i as f64 / sr;
            let noise = next_noise(&mut state);
            let highpass = noise - previous * 0.86;
            previous = noise;
            let envelope = (-t * 35.0).exp();
            let snare_tone = (2.0 * PI * 190.0 * t).sin();
            snare.push((envelope * (0.26 * f64::from(highpass) + 0.12 * snare_tone)) as f32);
        }

        previous = 0.0;
        for i in 0..hat_length {
            let t = i as f64 / sr;
            let noise = next_noise(&mut state);
            let highpass = noise - previous * 0.96;
            previous = noise;
            let envelope = (-t * 80.0).exp() * (1.0 - (-t * 1100.0).exp());
            hat.push((0.19 * envelope * f64::from(highpass)) as f32);
        }
        Self {
            kick: kick.into_boxed_slice(),
            snare: snare.into_boxed_slice(),
            hat: hat.into_boxed_slice(),
        }
    }

    fn get(&self, drum: Drum) -> &[f32] {
        match drum {
            Drum::Kick => &self.kick,
            Drum::Snare => &self.snare,
            Drum::Hat => &self.hat,
        }
    }

    pub fn sample_bytes(&self) -> usize {
        (self.kick.len() + self.snare.len() + self.hat.len()) * std::mem::size_of::<f32>()
    }
}

pub struct GrooveEngine {
    config: GrooveConfig,
    events: Vec<Event>,
    bank: SampleBank,
    voices: [Voice; MAX_VOICES],
    next_event: usize,
    frame: u64,
    triggered: u64,
    stolen_voices: u64,
    next_steal: usize,
}

impl GrooveEngine {
    /// All allocations and generated sample creation occur here, outside RT.
    pub fn new(pattern: &Pattern, config: GrooveConfig) -> Result<Self, ConfigError> {
        let events = compile_pattern(pattern, config)?;
        let bank = SampleBank::new(config.sample_rate);
        Ok(Self {
            config,
            events,
            bank,
            voices: [Voice::default(); MAX_VOICES],
            next_event: 0,
            frame: 0,
            triggered: 0,
            stolen_voices: 0,
            next_steal: 0,
        })
    }

    pub fn reset(&mut self) {
        // Must not be invoked concurrently with render.
        self.voices.fill(Voice::default());
        self.next_event = 0;
        self.frame = 0;
        self.triggered = 0;
        self.stolen_voices = 0;
        self.next_steal = 0;
    }

    fn trigger(&mut self, event: Event) {
        let slot = match self.voices.iter().position(|voice| !voice.active) {
            Some(idle) => idle,
            None => {
                let stolen = self.next_steal;
                self.next_steal = (self.next_steal + 1) % MAX_VOICES;
                self.stolen_voices += 1;
                stolen
            }
        };
        self.voices[slot] = Voice {
            active: true,
            drum: event.drum,
            position: 0,
            gain: f32::from(event.velocity) / 127.0,
        };
        self.triggered += 1;
    }

    /// Allocation-free, lock-free bounded 16-voice mono-to-stereo rendering.
    /// Parameters and clip must be fully prepared before the audio thread.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), ConfigError> {
        if left.len() != right.len() {
            return Err(ConfigError::ChannelLengthMismatch);
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            while self.next_event < self.events.len()
                && self.events[self.next_event].frame == self.frame
            {
                let event = self.events[self.next_event];
                self.next_event += 1;
                self.trigger(event);
            }

            let mut sample = 0.0f32;
            for voice in &mut self.voices {
                if !voice.active {
                    continue;
                }
                let data = self.bank.get(voice.drum);
                if let Some(&next) = data.get(voice.position) {
                    sample += next * voice.gain;
                    voice.position += 1;
                    if voice.position == data.len() {
                        voice.active = false;
                    }
                } else {
                    voice.active = false;
                }
            }
            // Conservative hard clamp for proof of concept; production mixer
            // will use gain staging + configurable limiting outside voice path.
            let out = sample.clamp(-0.98, 0.98);
            *l = out;
            *r = out;
            self.frame = self.frame.saturating_add(1);
        }
        Ok(())
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }
    pub fn rendered_frames(&self) -> u64 {
        self.frame
    }
    pub fn triggered_events(&self) -> u64 {
        self.triggered
    }
    pub fn stolen_voices(&self) -> u64 {
        self.stolen_voices
    }
    pub fn prepared_sample_bytes(&self) -> usize {
        self.bank.sample_bytes()
    }
    pub fn config(&self) -> GrooveConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_pattern_counts_and_exact_frame_positions() {
        let config = GrooveConfig::default();
        let events = compile_pattern(&Pattern::basic(), config).unwrap();
        assert_eq!(events.len(), 24);
        let kicks: Vec<_> = events
            .iter()
            .filter(|e| e.drum == Drum::Kick)
            .map(|e| e.frame)
            .collect();
        assert_eq!(kicks, vec![0, 44_100, 88_200, 132_300]);
        assert_eq!(config.duration_frames().unwrap(), 176_400);
        assert!(events.windows(2).all(|w| w[0].frame <= w[1].frame));
    }

    #[test]
    fn non_integer_step_duration_does_not_accumulate_rounding_drift() {
        let config = GrooveConfig {
            bpm: 137.2,
            bars: 64,
            ..GrooveConfig::default()
        };
        let events = compile_pattern(&Pattern::basic(), config).unwrap();
        let last_bar_kick = events
            .iter()
            .find(|e| {
                e.drum == Drum::Kick
                    && e.frame > ((63.0 * 4.0 * 60.0 * 44_100.0 / 137.2) - 5.0) as u64
            })
            .unwrap();
        let expected = (63.0_f64 * 4.0 * 60.0 * 44_100.0 / 137.2).round() as u64;
        assert_eq!(last_bar_kick.frame, expected);
    }

    #[test]
    fn identical_pcm_regardless_of_audio_callback_size() {
        let pattern = Pattern::basic();
        let config = GrooveConfig::default();
        let mut a = GrooveEngine::new(&pattern, config).unwrap();
        let mut b = GrooveEngine::new(&pattern, config).unwrap();
        let n = 44_100 * 4 + 13;
        let mut al = vec![0.0; n];
        let mut ar = vec![0.0; n];
        let mut bl = vec![0.0; n];
        let mut br = vec![0.0; n];
        a.render(&mut al, &mut ar).unwrap();
        for (l, r) in bl.chunks_mut(128).zip(br.chunks_mut(128)) {
            b.render(l, r).unwrap();
        }
        assert_eq!(al, bl);
        assert_eq!(ar, br);
        assert_eq!(al, ar);
        assert_eq!(a.triggered_events(), 24);
        assert_eq!(b.triggered_events(), 24);
        assert_eq!(a.stolen_voices(), 0);
        assert!(al.iter().any(|&x| x != 0.0));
        assert!(al.iter().all(|x| x.is_finite() && x.abs() <= 0.98));
        assert_eq!(a.prepared_sample_bytes(), b.prepared_sample_bytes());
        assert!(a.prepared_sample_bytes() < 200_000);
    }

    #[test]
    fn rejected_configuration_and_buffer_mismatch_are_non_destructive() {
        for invalid in [
            GrooveConfig {
                bpm: f64::NAN,
                ..GrooveConfig::default()
            },
            GrooveConfig {
                bpm: 0.0,
                ..GrooveConfig::default()
            },
            GrooveConfig {
                bars: 0,
                ..GrooveConfig::default()
            },
            GrooveConfig {
                bars: MAX_BARS + 1,
                ..GrooveConfig::default()
            },
            GrooveConfig {
                sample_rate: 0,
                ..GrooveConfig::default()
            },
        ] {
            assert!(GrooveEngine::new(&Pattern::basic(), invalid).is_err());
        }
        let mut engine = GrooveEngine::new(&Pattern::basic(), GrooveConfig::default()).unwrap();
        assert_eq!(
            engine.render(&mut [0.0; 1], &mut [0.0; 2]),
            Err(ConfigError::ChannelLengthMismatch)
        );
        assert_eq!(engine.rendered_frames(), 0);
        assert_eq!(engine.triggered_events(), 0);
    }

    #[test]
    fn bounded_voices_steal_without_allocating() {
        // Force >MAX_VOICES simultaneous voices to exercise deterministic
        // bounded voice stealing even though the default pattern is sparse.
        let mut engine = GrooveEngine::new(&Pattern::basic(), GrooveConfig::default()).unwrap();
        for _ in 0..(MAX_VOICES + 4) {
            engine.trigger(Event {
                frame: 0,
                drum: Drum::Kick,
                velocity: 127,
            });
        }
        assert_eq!(engine.stolen_voices(), 4);
        let mut l = [0.0; 128];
        let mut r = [0.0; 128];
        engine.render(&mut l, &mut r).unwrap();
        assert!(l.iter().all(|x| x.is_finite() && x.abs() <= 0.98));
    }

    #[test]
    fn resetting_repeats_identical_audio() {
        let mut engine = GrooveEngine::new(&Pattern::basic(), GrooveConfig::default()).unwrap();
        let mut a = [0.0; 512];
        let mut b = [0.0; 512];
        let mut c = [0.0; 512];
        let mut d = [0.0; 512];
        engine.render(&mut a, &mut b).unwrap();
        engine.reset();
        engine.render(&mut c, &mut d).unwrap();
        assert_eq!(a, c);
        assert_eq!(b, d);
    }
}

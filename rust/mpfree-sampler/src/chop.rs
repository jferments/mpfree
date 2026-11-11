//! Chop modes: turning a source into a [`ChopSet`] (step S3).
//!
//! [`SourceAnalysis`] runs the analysers once per source, off the audio
//! thread. Every chop mode is then cheap, so a sensitivity knob or a
//! "slices per beat" setting can be turned with the result redrawn at once.
//!
//! | Mode | Here |
//! |---|---|
//! | Transient | [`SourceAnalysis::transient`]: onsets at a sensitivity |
//! | Grid | [`ChopSet::grid`] |
//! | Beat | [`SourceAnalysis::beats`]: from the detected or a typed tempo |
//! | Silence | [`SourceAnalysis::silence`]: one slice per sound |
//! | Manual / live | [`ChopSet::add_marker`], snapped with [`SourceAnalysis::snap`] |

use mpfree_analysis::{estimate_tempo, loop_tempo, nearest_zero_crossing, root_pitch, sound_spans, OnsetAnalysis, Pitch, PitchSettings, SilenceSettings, Tempo};

use crate::model::{ChopMode, ChopSet, ModelError, Source, SourceId};

/// How far a marker may move to reach a zero crossing: 2.9 ms at 44.1 kHz.
pub const SNAP_RADIUS: usize = 128;

/// Analysis of one source, kept while it is being chopped.
#[derive(Clone, Debug)]
pub struct SourceAnalysis {
    source: SourceId,
    mono: Vec<f32>,
    onsets: OnsetAnalysis,
    tempo: Option<Tempo>,
    pitch: Option<Pitch>,
}

impl SourceAnalysis {
    pub fn analyse(source: &Source) -> SourceAnalysis {
        let ch = usize::from(source.channels());
        // A Source is always 1 or 2 channels with whole frames, so these hold.
        let mono = mpfree_analysis::mono(source.data(), ch).expect("source layout is checked on creation");
        let onsets = OnsetAnalysis::analyse(source.data(), ch, source.rate()).expect("source layout is checked on creation");
        let tempo = estimate_tempo(&onsets);
        let pitch = root_pitch(&mono, 1, source.rate(), PitchSettings::default()).expect("mono input");
        SourceAnalysis { source: source.id(), mono, onsets, tempo, pitch }
    }

    pub fn source(&self) -> SourceId { self.source }
    pub fn onsets(&self) -> &OnsetAnalysis { &self.onsets }

    /// The detected tempo, offered to the user, with confidence.
    pub fn tempo(&self) -> Option<Tempo> { self.tempo }

    /// The root pitch of the whole source, if it is pitched.
    pub fn pitch(&self) -> Option<Pitch> { self.pitch }

    /// The root pitch of frames `start..end` (a slice), with `frame`
    /// counted from the start of the source. For a melodic chop, this is
    /// what each slice's keygroup zone or tuning comes from.
    pub fn pitch_of(&self, start: u64, end: u64) -> Option<Pitch> {
        let len = self.mono.len() as u64;
        let (a, b) = (start.min(len), end.min(len));
        if b <= a {
            return None;
        }
        let mut p = root_pitch(&self.mono[a as usize..b as usize], 1, self.onsets.rate(), PitchSettings::default()).ok()??;
        p.frame += a;
        Some(p)
    }

    /// The tempo if the source is a loop cut to a whole number of beats:
    /// exact, from its length, with the beat count chosen by the estimate.
    pub fn loop_tempo(&self) -> Option<f32> {
        loop_tempo(self.onsets.frames(), self.onsets.rate(), self.tempo.map(|t| t.bpm))
    }

    /// The zero crossing nearest `frame`, or `frame` itself if none is close.
    pub fn snap(&self, frame: u64) -> u64 {
        let at = usize::try_from(frame).unwrap_or(usize::MAX);
        nearest_zero_crossing(&self.mono, at, SNAP_RADIUS).map_or(frame, |f| f as u64)
    }

    fn check(&self, source: &Source) -> Result<(), ModelError> {
        if source.id() == self.source { Ok(()) } else { Err(ModelError::UnknownSource) }
    }

    /// Transient chop at a sensitivity from 0 (only the strongest hits) to 1
    /// (every candidate). Markers already sit just before each attack, on a
    /// zero crossing where there is one.
    pub fn transient(&self, name: &str, source: &Source, sensitivity: f32) -> Result<ChopSet, ModelError> {
        self.check(source)?;
        let mut picks = self.onsets.pick(sensitivity);
        // Keep the strongest 127 if a very busy file finds more.
        if picks.len() >= crate::model::MAX_SLICES {
            let mut strong: Vec<_> = self.onsets.candidates().iter().filter(|c| picks.binary_search(&c.frame).is_ok()).collect();
            strong.sort_by(|a, b| b.score.total_cmp(&a.score));
            picks = strong.iter().take(crate::model::MAX_SLICES - 1).map(|c| c.frame).collect();
        }
        ChopSet::from_markers(name, source, ChopMode::Transient, picks)
    }

    /// Beat chop: a slice every `beats_per_slice` beats at `bpm` (or the
    /// loop tempo, else the detected tempo, when `bpm` is None). The first
    /// beat is the first of the strongest hits if it comes within a quarter
    /// of a beat of the start, so a recording that starts a little early
    /// still lines up; otherwise it is the start.
    pub fn beats(&self, name: &str, source: &Source, bpm: Option<f64>, beats_per_slice: f64) -> Result<ChopSet, ModelError> {
        self.check(source)?;
        let bpm = bpm
            .or_else(|| self.loop_tempo().map(f64::from))
            .or_else(|| self.tempo.map(|t| f64::from(t.bpm)))
            .ok_or(ModelError::BadSlice)?;
        let beat = 60.0 / bpm * f64::from(source.rate());
        let first = self.onsets.pick(0.0).first().copied().filter(|&f| (f as f64) < beat * 0.25).unwrap_or(0);
        ChopSet::beats(name, source, bpm, beats_per_slice, first)
    }

    /// Silence chop: one slice per sound, each starting just before it.
    pub fn silence(&self, name: &str, source: &Source, settings: SilenceSettings) -> Result<ChopSet, ModelError> {
        self.check(source)?;
        let spans = sound_spans(&self.mono, 1, source.rate(), settings).map_err(|_| ModelError::BadSlice)?;
        if spans.len() > crate::model::MAX_SLICES { return Err(ModelError::TooManySlices); }
        ChopSet::from_markers(name, source, ChopMode::Silence, spans.iter().map(|s| s.0))
    }

    /// Leading and trailing silence, as the (start, end) to trim to.
    pub fn trim(&self, settings: SilenceSettings) -> Option<(u64, u64)> {
        let spans = sound_spans(&self.mono, 1, self.onsets.rate(), settings).ok()?;
        Some((spans.first()?.0, spans.last()?.1))
    }
}

/// A short drum loop made from scratch, for tests and for trying the chop
/// tool without a WAV file: kick on 1 and 3, snare on 2 and 4, hats on the
/// off-beat eighths, deterministic. Mono at `rate`; returns the audio and
/// the frame of every hit. Exactly `bars` bars long plus `tail` frames.
pub fn demo_beat(bpm: f64, bars: usize, tail: usize, rate: u32) -> (Vec<f32>, Vec<usize>) {
    let r = f64::from(rate);
    let rf = rate as f32;
    let eighth = 60.0 / bpm / 2.0 * r;
    let frames = (eighth * 8.0 * bars as f64).round() as usize + tail;
    let mut out = vec![0.0f32; frames];
    let mut seed = 0x9e37_79b9u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut hits = Vec::new();
    for e in 0..8 * bars {
        let at = (e as f64 * eighth).round() as usize;
        hits.push(at);
        let kind = match e % 8 { 0 | 4 => 0, 2 | 6 => 1, _ => 2 };
        let mut phase = 0.0f32;
        let n = ((match kind { 0 => 0.8, 1 => 0.5, _ => 0.12 }) * r) as usize;
        for i in 0..n.min(frames - at) {
            let t = i as f32 / rf;
            out[at + i] += match kind {
                0 => {
                    phase += std::f32::consts::TAU * (50.0 + 100.0 * (-t * 30.0).exp()) / rf;
                    0.8 * phase.sin() * (-t * 9.0).exp()
                }
                1 => 0.7 * (0.6 * noise() * (-t * 18.0).exp() + 0.4 * (std::f32::consts::TAU * 190.0 * t).sin() * (-t * 25.0).exp()),
                _ => 0.125 * (noise() - noise()) * (-t * 80.0).exp(),
            };
        }
    }
    (out, hits)
}

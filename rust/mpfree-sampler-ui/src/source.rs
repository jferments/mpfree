//! What the screens need to know about a source, worked out once when it is
//! loaded or recorded, never on the audio thread (it allocates).

use std::sync::Arc;

use mpfree_analysis::{
    estimate_tempo, loop_tempo, note_name, root_pitch, Column, OnsetAnalysis, PeakPyramid,
    PitchSettings,
};
use mpfree_sampler::model::{Source, SourceId};
use mpfree_ui::wave::{Peaks, WaveColumn};

/// A source's peak pyramid as the waveform widgets' [`Peaks`]: one lookup
/// per pixel column at any zoom, and the samples themselves when zoomed in
/// closer than the pyramid's finest level.
pub struct PyramidPeaks {
    pyramid: PeakPyramid,
    audio: Arc<[f32]>,
}

impl PyramidPeaks {
    /// None if the source has more than two channels or isn't whole frames.
    pub fn new(audio: Arc<[f32]>, channels: usize) -> Option<PyramidPeaks> {
        let pyramid = PeakPyramid::build(&audio, channels).ok()?;
        Some(PyramidPeaks { pyramid, audio })
    }

    pub fn pyramid(&self) -> &PeakPyramid {
        &self.pyramid
    }
}

impl Peaks for PyramidPeaks {
    fn frames(&self) -> u64 {
        self.pyramid.frames()
    }

    fn channels(&self) -> usize {
        self.pyramid.channels()
    }

    fn column(&self, ch: usize, f0: f64, f1: f64) -> WaveColumn {
        if ch >= self.pyramid.channels() {
            return WaveColumn::default();
        }
        let mut out = [Column::default()];
        self.pyramid
            .columns(Some(&self.audio), ch, f0, (f1 - f0).max(1e-6), &mut out);
        WaveColumn {
            min: out[0].min,
            max: out[0].max,
            rms: out[0].rms,
        }
    }
}

/// A source as the screens see it.
pub struct SourceView {
    pub id: SourceId,
    pub peaks: Arc<dyn Peaks>,
    /// The source mixed to mono, for snapping markers to zero crossings.
    pub mono: Arc<[f32]>,
    /// Beats per minute, detected (or set by the user), if it has a beat.
    pub tempo: Option<f64>,
    /// Its root note, such as "C#3", if it has a clear pitch.
    pub root: Option<String>,
    /// The same note as a MIDI note (60 is C4), for a keygroup's root.
    pub root_note: Option<u8>,
    /// Where its hits start, for chopping by transients (None if it
    /// couldn't be analysed).
    pub onsets: Option<Arc<OnsetAnalysis>>,
}

/// Root notes less certain than this aren't shown.
const ROOT_CONFIDENCE: f32 = 0.5;

impl SourceView {
    /// Analyses a source: peaks, mono mix, onsets, tempo and root note. Takes a
    /// moment for a long source, so run it on a worker or the UI thread
    /// when the source arrives, never on the audio thread.
    pub fn analyse(s: &Source) -> SourceView {
        let ch = usize::from(s.channels());
        let data = s.data();
        let peaks: Arc<dyn Peaks> = match PyramidPeaks::new(data.clone(), ch) {
            Some(p) => Arc::new(p),
            None => Arc::new(Silent),
        };
        let mono: Arc<[f32]> = mpfree_analysis::mono(data, ch).unwrap_or_default().into();
        let onsets = OnsetAnalysis::analyse(data, ch, s.rate())
            .ok()
            .map(Arc::new);
        let tempo = onsets.as_deref().and_then(|o| {
            let est = estimate_tempo(o)?.bpm;
            // A loop cut to whole beats has an exact tempo; use it when it
            // agrees with the estimate.
            let exact = loop_tempo(s.frames(), s.rate(), Some(est))
                .filter(|l| (l / est - 1.0).abs() < 0.03);
            Some(f64::from(exact.unwrap_or(est)))
        });
        let root_note = root_pitch(data, ch, s.rate(), PitchSettings::default())
            .ok()
            .flatten()
            .filter(|p| p.confidence >= ROOT_CONFIDENCE)
            .map(|p| p.note);
        let root = root_note.map(note_name);
        SourceView {
            id: s.id(),
            peaks,
            mono,
            tempo,
            root,
            root_note,
            onsets,
        }
    }
}

/// No peaks: a flat line.
struct Silent;

impl Peaks for Silent {
    fn frames(&self) -> u64 {
        0
    }

    fn channels(&self) -> usize {
        1
    }

    fn column(&self, _ch: usize, _f0: f64, _f1: f64) -> WaveColumn {
        WaveColumn::default()
    }
}

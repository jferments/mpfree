//! What the screens need to know about a source, worked out once when it is
//! loaded or recorded, never on the audio thread (it allocates).

use std::sync::Arc;

/// A sound's pitch over time ([`SourceView::pitch`]).
pub use mpfree_analysis::PitchTrack;
use mpfree_analysis::{
    estimate_tempo, loop_tempo, note_name, pitch_track, root_pitch, Column, OnsetAnalysis,
    PeakPyramid, PitchSettings,
};
use mpfree_sampler::model::{MonoMix, Pcm, Source, SourceId};
use mpfree_ui::wave::{Peaks, WaveColumn};

/// A source's peak pyramid as the waveform widgets' [`Peaks`]: one lookup
/// per pixel column at any zoom, and the samples themselves when zoomed in
/// closer than the pyramid's finest level.
pub struct PyramidPeaks {
    pyramid: PeakPyramid,
    audio: Pcm,
}

impl PyramidPeaks {
    /// None if the source has more than two channels or isn't whole frames.
    pub fn new(audio: Pcm, channels: usize) -> Option<PyramidPeaks> {
        let pyramid = PeakPyramid::build_from(&audio, channels).ok()?;
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
            .columns_from(Some(&self.audio), ch, f0, (f1 - f0).max(1e-6), &mut out);
        WaveColumn {
            min: out[0].min,
            max: out[0].max,
            rms: out[0].rms,
        }
    }
}

/// A source as the screens see it.
#[derive(Clone)]
pub struct SourceView {
    pub id: SourceId,
    pub peaks: Arc<dyn Peaks>,
    /// The same peaks when they are a pyramid (a sound of one or two
    /// channels, once [`SourceView::quick`] has run), for the cache file.
    pub pyramid: Option<Arc<PyramidPeaks>>,
    /// The source mixed to mono as it is read, for snapping markers to
    /// zero crossings and finding its silences ([`MonoMix`]): no copy, so
    /// a view holds only its peaks and hits beside the sound (the sampler
    /// plan's step 8c; a view kept a whole mono copy before, 127 MB for a
    /// 12-minute take at 44.1 kHz). None until [`SourceView::quick`].
    pub mono: Option<MonoMix>,
    /// Beats per minute, detected (or set by the user), if it has a beat.
    pub tempo: Option<f64>,
    /// Its root note, such as "C#3", if it has a clear pitch.
    pub root: Option<String>,
    /// The same note as a MIDI note (60 is C4), for a keygroup's root.
    pub root_note: Option<u8>,
    /// Where its hits start, for chopping by transients (None if it
    /// couldn't be analysed, or isn't yet).
    pub onsets: Option<Arc<OnsetAnalysis>>,
    /// Its pitch over time, for the Slice tool's Pitch method (R4.7): only
    /// followed when that asks for it ([`SourceView::follow_pitch`]; in the
    /// app, off the screen thread), so None until then.
    pub pitch: Option<Arc<PitchTrack>>,
    /// Whether the hits, tempo and root note are worked out. A view made
    /// off the screen thread comes in steps: [`SourceView::pending`] (a
    /// flat line of the right length), then [`SourceView::quick`] (the
    /// peaks), then [`SourceView::finish`] (the rest).
    pub ready: bool,
}

/// Root notes less certain than this aren't shown.
const ROOT_CONFIDENCE: f32 = 0.5;

impl SourceView {
    /// Analyses a source: peaks, onsets, tempo and root note. Takes a
    /// moment for a long source (about 0.2 s a minute of stereo on the
    /// workstation, several times that on the MPC), so run it on a worker,
    /// never on the audio thread; [`SourceView::quick`] and
    /// [`SourceView::finish`] are its two steps.
    pub fn analyse(s: &Source) -> SourceView {
        SourceView::quick(s).finish(s)
    }

    /// A source before anything is worked out: a flat line of its length,
    /// for the screen to show at once.
    pub fn pending(s: &Source) -> SourceView {
        SourceView {
            id: s.id(),
            peaks: Arc::new(Flat {
                frames: s.frames(),
                channels: usize::from(s.channels()),
            }),
            pyramid: None,
            mono: None,
            tempo: None,
            root: None,
            root_note: None,
            onsets: None,
            pitch: None,
            ready: false,
        }
    }

    /// The first step: the peaks, about a seventh of the whole analysis,
    /// so the waveform shows long before the hits, tempo and root note are
    /// known.
    pub fn quick(s: &Source) -> SourceView {
        let ch = usize::from(s.channels());
        let pyramid = PyramidPeaks::new(s.data().clone(), ch).map(Arc::new);
        let peaks: Arc<dyn Peaks> = match &pyramid {
            Some(p) => p.clone(),
            None => Arc::new(Silent),
        };
        SourceView {
            mono: Some(s.mono_mix()),
            peaks,
            pyramid,
            ..SourceView::pending(s)
        }
    }

    /// The second step: the hits, tempo and root note of `s`, with this
    /// view's peaks.
    pub fn finish(&self, s: &Source) -> SourceView {
        // The hits and the note are found in the sound mixed to mono, as
        // they would be from the channels: made here for the analysis and
        // gone after it.
        let mono = s.data().mono(usize::from(s.channels()));
        let mono = mono.as_slice();
        let onsets = OnsetAnalysis::analyse(mono, 1, s.rate()).ok().map(Arc::new);
        let tempo = onsets.as_deref().and_then(|o| {
            let est = estimate_tempo(o)?.bpm;
            // A loop cut to whole beats has an exact tempo; use it when it
            // agrees with the estimate.
            let exact = loop_tempo(s.frames(), s.rate(), Some(est))
                .filter(|l| (l / est - 1.0).abs() < 0.03);
            Some(f64::from(exact.unwrap_or(est)))
        });
        let root_note = root_pitch(mono, 1, s.rate(), PitchSettings::default())
            .ok()
            .flatten()
            .filter(|p| p.confidence >= ROOT_CONFIDENCE)
            .map(|p| p.note);
        let root = root_note.map(note_name);
        SourceView {
            id: s.id(),
            peaks: self.peaks.clone(),
            pyramid: self.pyramid.clone(),
            mono: Some(s.mono_mix()),
            tempo,
            root,
            root_note,
            onsets,
            pitch: self.pitch.clone(),
            ready: true,
        }
    }

    /// The pitch over time of `s`, this view's sound, followed in it mixed
    /// to mono (made for this and gone after it): about a fifth of a second
    /// a minute of sound on the workstation, so the app runs it on its
    /// analysis worker. None before [`SourceView::quick`].
    pub fn follow_pitch(&self, s: &Source) -> Option<Arc<PitchTrack>> {
        self.mono.as_ref()?;
        let mono = s.data().mono(usize::from(s.channels()));
        pitch_track(&mono, 1, s.rate()).ok().map(Arc::new)
    }

    /// This view with its pitch followed.
    pub fn with_pitch(&self, pitch: Arc<PitchTrack>) -> SourceView {
        SourceView {
            pitch: Some(pitch),
            ..self.clone()
        }
    }
}

impl SourceView {
    /// The view as its cache file's bytes ([`cache_file`]); None until it
    /// is [ready](SourceView::ready), or for a sound with no peak pyramid.
    /// See [`SourceView::from_cache`].
    pub fn to_cache(&self) -> Option<Vec<u8>> {
        let pyramid = self.pyramid.as_ref().filter(|_| self.ready)?.pyramid();
        let peaks = pyramid.to_bytes();
        let onsets = self.onsets.as_ref().map_or_else(Vec::new, |o| o.to_bytes());
        let mut out = Vec::with_capacity(48 + peaks.len() + onsets.len());
        out.extend_from_slice(CACHE_MAGIC);
        out.extend_from_slice(&((self.id.0 >> 64) as u64).to_le_bytes());
        out.extend_from_slice(&pyramid.frames().to_le_bytes());
        out.extend_from_slice(&(pyramid.channels() as u32).to_le_bytes());
        out.extend_from_slice(&self.tempo.unwrap_or(f64::NAN).to_le_bytes());
        out.push(self.root_note.unwrap_or(NO_NOTE));
        for part in [&peaks, &onsets] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }
        Some(out)
    }

    /// The view of `s` read from its cache file, ready at once: the peaks,
    /// hits, tempo and root note as they were worked out, without reading
    /// the sound.
    /// None if the bytes are another sound's, of another version, or don't
    /// read; the sound is then analysed as if there were no file.
    pub fn from_cache(s: &Source, b: &[u8]) -> Option<SourceView> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let end = at.checked_add(n)?;
            let p = b.get(at..end)?;
            at = end;
            Some(p)
        };
        if take(8)? != CACHE_MAGIC {
            return None;
        }
        let u64_le = |p: &[u8]| u64::from_le_bytes(p.try_into().expect("8 bytes"));
        let u32_le = |p: &[u8]| u32::from_le_bytes(p.try_into().expect("4 bytes"));
        let id = u64_le(take(8)?);
        let frames = u64_le(take(8)?);
        let channels = u32_le(take(4)?);
        let ch = usize::from(s.channels());
        if !cache_ids(s).contains(&id) || frames != s.frames() || channels as usize != ch {
            return None;
        }
        let tempo = f64::from_le_bytes(take(8)?.try_into().ok()?);
        let tempo = (!tempo.is_nan()).then_some(tempo);
        if tempo.is_some_and(|t| !(t.is_finite() && t > 0.0)) {
            return None;
        }
        let note = take(1)?[0];
        let root_note = (note != NO_NOTE).then_some(note).filter(|n| *n <= 127);
        if note != NO_NOTE && root_note.is_none() {
            return None;
        }
        let n = u32_le(take(4)?) as usize;
        let p = PeakPyramid::from_bytes(take(n)?).ok()?;
        if p.frames() != frames || p.channels() != ch {
            return None;
        }
        let pyramid = Arc::new(PyramidPeaks {
            pyramid: p,
            audio: s.data().clone(),
        });
        let n = u32_le(take(4)?) as usize;
        let onsets = take(n)?;
        let onsets = if onsets.is_empty() {
            None
        } else {
            let o = OnsetAnalysis::from_bytes(onsets).ok()?;
            if o.frames() != frames || o.rate() != s.rate() {
                return None;
            }
            Some(Arc::new(o))
        };
        if at != b.len() {
            return None;
        }
        Some(SourceView {
            id: s.id(),
            peaks: pyramid.clone(),
            pyramid: Some(pyramid),
            mono: Some(s.mono_mix()),
            tempo,
            root: root_note.map(note_name),
            root_note,
            onsets,
            pitch: None,
            ready: true,
        })
    }
}

/// The name of the file a sound's analysis is kept in beside it, in a
/// project's `samples/`: its sound file's name with `.a1.peaks` for
/// `.wav` (`<id>.a1.peaks`, or, for a sound an older build saved under its
/// 64-bit id, that file's name). The analysis's version is in the name, so
/// a newer analysis never takes an older one's file for its own: it
/// analyses the sound again and its save writes a file of its own.
pub fn cache_file(s: &Source) -> String {
    let file = mpfree_sampler::wav::file_name(s);
    let stem = file.strip_suffix(".wav").unwrap_or(&file);
    format!("{stem}.a{CACHE_VERSION}.peaks")
}

/// The ids a cache file of `s` may hold to say it is this sound's: the
/// first 64 bits of its id, which is what a cache file holds since ids
/// have 128 (the sampler plan's step 8), and, for a sound an older build
/// saved under its 64-bit id, that id.
fn cache_ids(s: &Source) -> [u64; 2] {
    let new = (s.id().0 >> 64) as u64;
    let old = s
        .file()
        .and_then(|f| f.strip_suffix(".wav"))
        .and_then(|id| u64::from_str_radix(id, 16).ok());
    [new, old.unwrap_or(new)]
}

/// The analysis's version: raise it with any change to what the analysis
/// finds or to the cache file's layout.
pub const CACHE_VERSION: u32 = 1;
/// The cache file's first bytes.
const CACHE_MAGIC: &[u8; 8] = b"MPFVIEW1";
/// No root note, in the cache file.
const NO_NOTE: u8 = 255;

/// Not worked out yet: a flat line, of the sound's length.
struct Flat {
    frames: u64,
    channels: usize,
}

impl Peaks for Flat {
    fn frames(&self) -> u64 {
        self.frames
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn column(&self, _ch: usize, _f0: f64, _f1: f64) -> WaveColumn {
        WaveColumn::default()
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

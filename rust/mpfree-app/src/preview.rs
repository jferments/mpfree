//! The browser's preview path: a sound file heard once (the browser's
//! Audition, or PLAY on the Pads tab's browser) at the preview level, on the
//! main out over everything else.
//!
//! The screen thread reads and decodes the file, then hands the sound over
//! in a box ([`crate::gui::FromUi::Preview`]). The audio thread mixes it in
//! a period at a time and gives the box back when it ends or is stopped
//! ([`crate::gui::ToUi::PreviewBack`]), so it never frees memory itself.
//! A file at another rate plays at its own pitch (linear interpolation).

use std::sync::Arc;

use mpfree_sampler::model::Source;

/// The preview level the browser starts at, in dB.
pub const LEVEL_DB: f32 = -6.0;
/// The quietest and loudest preview levels, in dB.
pub const LEVEL_MIN_DB: f32 = -40.0;
pub const LEVEL_MAX_DB: f32 = 6.0;
/// Frames a stop takes to fade out, so it never clicks.
const FADE: u32 = 256;

/// A sound being heard once.
pub struct Preview {
    pub name: String,
    data: Arc<[f32]>,
    channels: usize,
    frames: usize,
    rate: u32,
    gain: f32,
    /// Where it is, in the sound's own frames.
    pos: f64,
    /// Frames of fade left once stopped (None: playing on).
    stopping: Option<u32>,
}

impl Preview {
    /// `src` at `level_db` (clamped to the preview's range).
    pub fn new(src: &Source, level_db: f32) -> Preview {
        let channels = usize::from(src.channels()).max(1);
        Preview {
            name: src.name.clone(),
            data: src.data().clone(),
            channels,
            frames: src.data().len() / channels,
            rate: src.rate(),
            gain: db_to_gain(level_db.clamp(LEVEL_MIN_DB, LEVEL_MAX_DB)),
            pos: 0.0,
            stopping: None,
        }
    }

    /// Fades out over the next few periods (then [`Preview::mix`] says it
    /// is done).
    pub fn stop(&mut self) {
        if self.stopping.is_none() {
            self.stopping = Some(FADE);
        }
    }

    /// Adds the next `left.len()` frames at `out_rate` to `left` and
    /// `right`. True once it has played to the end or faded out: give the
    /// box back.
    pub fn mix(&mut self, out_rate: u32, left: &mut [f32], right: &mut [f32]) -> bool {
        let step = f64::from(self.rate) / f64::from(out_rate.max(1));
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let i = self.pos as usize;
            if i + 1 >= self.frames {
                return true;
            }
            let g = match self.stopping {
                Some(0) => return true,
                Some(n) => {
                    self.stopping = Some(n - 1);
                    self.gain * n as f32 / FADE as f32
                }
                None => self.gain,
            };
            let t = (self.pos - i as f64) as f32;
            let (a, b) = (self.frame(i), self.frame(i + 1));
            *l += (a.0 + (b.0 - a.0) * t) * g;
            *r += (a.1 + (b.1 - a.1) * t) * g;
            self.pos += step;
        }
        self.pos as usize + 1 >= self.frames || self.stopping == Some(0)
    }

    fn frame(&self, i: usize) -> (f32, f32) {
        if self.channels == 2 {
            (self.data[2 * i], self.data[2 * i + 1])
        } else {
            let v = self.data[i];
            (v, v)
        }
    }
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview")
            .field("name", &self.name)
            .field("frames", &self.frames)
            .field("rate", &self.rate)
            .field("pos", &self.pos)
            .finish()
    }
}

/// The same sound at the same place (for the screen queue's tests).
impl PartialEq for Preview {
    fn eq(&self, o: &Preview) -> bool {
        Arc::ptr_eq(&self.data, &o.data) && self.pos == o.pos && self.gain == o.gain
    }
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, channels: u8, frames: usize) -> Source {
        let data: Vec<f32> = (0..frames * usize::from(channels))
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.25 })
            .collect();
        Source::new("Tone", rate, channels, data).unwrap()
    }

    #[test]
    fn it_plays_once_at_the_level_and_ends() {
        let mut p = Preview::new(&tone(44_100, 2, 300), 0.0);
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        assert!(!p.mix(44_100, &mut l, &mut r));
        assert_eq!((l[0], r[0]), (0.5, -0.25), "stereo as it is, at 0 dB");
        assert!(!p.mix(44_100, &mut l, &mut r));
        assert!(
            p.mix(44_100, &mut l, &mut r),
            "300 frames end in the third period"
        );
    }

    #[test]
    fn mono_goes_to_both_sides_and_the_level_is_in_db() {
        let mut p = Preview::new(&tone(44_100, 1, 1000), -6.0);
        let (mut l, mut r) = ([0.0f32; 4], [0.0f32; 4]);
        p.mix(44_100, &mut l, &mut r);
        assert!((l[0] - 0.5 * db_to_gain(-6.0)).abs() < 1e-6);
        assert_eq!(l[0], r[0]);
    }

    #[test]
    fn another_rate_plays_at_its_own_pitch() {
        // 22.05 kHz at 44.1 kHz: half a source frame a frame.
        let mut p = Preview::new(&tone(22_050, 1, 10_000), 0.0);
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        p.mix(44_100, &mut l, &mut r);
        assert!((p.pos - 64.0).abs() < 1e-9);
    }

    #[test]
    fn a_stop_fades_out_and_then_it_is_done() {
        let mut p = Preview::new(&tone(44_100, 1, 100_000), 0.0);
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        p.mix(44_100, &mut l, &mut r);
        p.stop();
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        assert!(!p.mix(44_100, &mut l, &mut r));
        assert!(l[127].abs() < l[0].abs(), "fading");
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        assert!(p.mix(44_100, &mut l, &mut r));
        assert!(l[127].abs() < 0.01, "all but silent at the end of the fade");
    }
}

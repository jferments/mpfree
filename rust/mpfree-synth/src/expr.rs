//! Per-note expression for the synths: each note's own slide (pitch, in
//! semitones), tone (0 to 1) and release velocity, from
//! [`NodeEvent::Expression`](mpfree_node::NodeEvent::Expression).
//!
//! [`NoteExprs`] keeps what each MIDI note's next note-on starts with, so a
//! value sent just before a note (as MPE controllers do) is in place when it
//! starts. A note-off takes the note's release velocity and puts the note
//! back to neutral. [`VoiceExpr`] is one voice's copy, gliding to new
//! values over [`EXPR_MS`] so a stream of slide values never zips.
//!
//! With nothing sent every value stays exactly neutral (0), and engines skip
//! the work, so every sound is unchanged to the bit.

use mpfree_node::Expr;

/// How long a voice's slide and tone take to cover 63 % of a jump.
pub const EXPR_MS: f32 = 5.0;

/// Each MIDI note's slide, tone and release velocity, for its next note-on
/// and note-off.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NoteExprs {
    slide: [f32; 128],
    tone: [f32; 128],
    release: [f32; 128],
}

impl NoteExprs {
    pub fn new() -> NoteExprs {
        NoteExprs {
            slide: [0.0; 128],
            tone: [0.0; 128],
            release: [Expr::Release.neutral(); 128],
        }
    }

    /// Keeps a value for `note`. Returns it clamped, or `None` for not a
    /// number (ignored).
    pub fn set(&mut self, note: u8, kind: Expr, value: f32) -> Option<f32> {
        let v = kind.clamp(value)?;
        let n = usize::from(note & 0x7F);
        match kind {
            Expr::Slide => self.slide[n] = v,
            Expr::Tone => self.tone[n] = v,
            Expr::Release => self.release[n] = v,
        }
        Some(v)
    }

    /// What a note-on of `note` starts with: (slide, tone).
    pub fn on(&self, note: u8) -> (f32, f32) {
        let n = usize::from(note & 0x7F);
        (self.slide[n], self.tone[n])
    }

    /// At `note`'s note-off: its release velocity, and the note back to
    /// neutral for its next note-on.
    pub fn off(&mut self, note: u8) -> f32 {
        let n = usize::from(note & 0x7F);
        self.slide[n] = 0.0;
        self.tone[n] = 0.0;
        std::mem::replace(&mut self.release[n], Expr::Release.neutral())
    }

    /// Every note back to neutral (all notes off, a reset).
    pub fn clear(&mut self) {
        *self = NoteExprs::new();
    }
}

/// One voice's slide (semitones) and tone (0 to 1), gliding to targets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct VoiceExpr {
    pub slide: f32,
    pub tone: f32,
    slide_to: f32,
    tone_to: f32,
}

impl VoiceExpr {
    /// A new note: jumps straight to its values.
    pub fn start(&mut self, (slide, tone): (f32, f32)) {
        *self = VoiceExpr {
            slide,
            tone,
            slide_to: slide,
            tone_to: tone,
        };
    }

    /// The voice moves to another note without a new start (mono, legato):
    /// glides to that note's values.
    pub fn retarget(&mut self, (slide, tone): (f32, f32)) {
        self.slide_to = slide;
        self.tone_to = tone;
    }

    /// A new value for this voice's note.
    pub fn aim(&mut self, kind: Expr, value: f32) {
        match kind {
            Expr::Slide => self.slide_to = value,
            Expr::Tone => self.tone_to = value,
            Expr::Release => {}
        }
    }

    /// One control step, `c` the smoothing coefficient ([`expr_coef`]).
    #[inline]
    pub fn step(&mut self, c: f32) {
        if self.slide != self.slide_to {
            self.slide = glide(self.slide, self.slide_to, c, 1e-4);
        }
        if self.tone != self.tone_to {
            self.tone = glide(self.tone, self.tone_to, c, 1e-5);
        }
    }

    /// Whether anything is away from neutral or moving.
    #[inline]
    pub fn active(&self) -> bool {
        self.slide != 0.0 || self.tone != 0.0 || self.slide_to != 0.0 || self.tone_to != 0.0
    }
}

#[inline]
fn glide(x: f32, to: f32, c: f32, eps: f32) -> f32 {
    let n = x + (to - x) * c;
    if (to - n).abs() < eps {
        to
    } else {
        n
    }
}

/// The one-pole coefficient for [`EXPR_MS`] at a control rate in Hz.
pub fn expr_coef(ctrl_rate: f32) -> f32 {
    let n = (EXPR_MS * 0.001 * ctrl_rate).max(1.0);
    1.0 - (-1.0 / n).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_wait_for_the_note_and_clear_at_its_note_off() {
        let mut n = NoteExprs::new();
        assert_eq!(n.set(60, Expr::Slide, 99.0), Some(48.0));
        assert_eq!(n.set(60, Expr::Tone, 0.25), Some(0.25));
        assert_eq!(n.set(60, Expr::Tone, f32::NAN), None);
        assert_eq!(n.set(60, Expr::Release, 0.9), Some(0.9));
        assert_eq!(n.on(60), (48.0, 0.25));
        assert_eq!(n.on(61), (0.0, 0.0));
        assert_eq!(n.off(60), 0.9);
        assert_eq!(n.on(60), (0.0, 0.0));
        assert_eq!(n.off(60), 0.5);
        n.set(200, Expr::Slide, -3.0);
        assert_eq!(
            n.on(72),
            (-3.0, 0.0),
            "notes above 127 wrap like the engines'"
        );
        n.clear();
        assert_eq!(n.on(72), (0.0, 0.0));
    }

    #[test]
    fn a_voice_glides_to_new_values_and_settles_exactly() {
        let mut v = VoiceExpr::default();
        assert!(!v.active());
        v.step(0.5);
        assert_eq!(v, VoiceExpr::default(), "neutral stays exactly neutral");
        v.start((2.0, 0.5));
        assert_eq!((v.slide, v.tone), (2.0, 0.5));
        v.aim(Expr::Slide, 0.0);
        v.aim(Expr::Release, 1.0);
        let c = expr_coef(48_000.0 / 16.0);
        v.step(c);
        assert!(v.slide > 0.0 && v.slide < 2.0);
        for _ in 0..1000 {
            v.step(c);
        }
        assert_eq!(v.slide, 0.0);
        assert!(v.active(), "tone still 0.5");
        v.retarget((0.0, 0.0));
        for _ in 0..1000 {
            v.step(c);
        }
        assert!(!v.active());
    }
}

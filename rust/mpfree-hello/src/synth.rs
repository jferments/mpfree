//! The pad player's synth from round 6: MPFree's Poly (`mpfree-synth`) on
//! the pads, edited on the Synth tab (`mpfree-synth-ui`).
//!
//! - The kit list ends with one more entry, "Synth: Poly". Picking it, or
//!   opening the Synth tab, makes the pads play Poly: pad 1 plays C3 and
//!   each pad after it a semitone higher, so the 16 pads play C3 to D#4.
//!   SHIFT + wheel on the Play page moves them an octave (C0 to C6 at pad 1).
//! - Note Repeat, Latch, Full and Half Level and the velocity curve work as
//!   they do for drums: the note engine (`mpfree-arp`) hands Poly its notes
//!   on their exact frames, and pad pressure goes to the note its pad plays.
//! - The audio thread owns the node. The Synth tab (on the screen thread)
//!   keeps a copy of the values and sends changes, which apply from the
//!   next period.
//! - [`Synth::render`] adds Poly to the period after the sampler. It never
//!   allocates. A block with a non-finite sample is dropped, the node reset
//!   and the reset counted, as the node interface asks of a host.
//! - STOP fades Poly out over 10 ms, then resets it, so no release rings on.
//!
//! Everything that allocates (`new`, `values`, `state`, the labels and log
//! lines) runs before the sound starts or for the screen and the log, never
//! inside `render`.

use mpfree_node::{Ctx, Node, NodeEvent, ParamInfo, Timed};
use mpfree_synth::Poly;
use mpfree_synth_ui::format::value_line;
use mpfree_synth_ui::SynthState;
use mpfree_ui::Color;
use std::time::{Duration, Instant};

/// The kit list's entry for the synth.
pub const KIT_NAME: &str = "Synth: Poly";
/// The preset Poly starts with.
pub const START_PRESET: &str = "Pluck";
/// The note pad 1 plays at octave 0: C3 (60 is middle C, C4).
pub const BASE_NOTE: u8 = 48;
/// How far SHIFT + wheel moves the pads: pad 1 from C0 to C6.
pub const MIN_OCTAVE: i32 = -3;
pub const MAX_OCTAVE: i32 = 3;
/// Resting pad colours on the controller, all from the colours Akai's app
/// was seen sending (`akai::RESTING_COLOURS`): C, the other white keys, and
/// the black keys.
pub const PAD_C: [u8; 3] = [0x2C, 0x00, 0x3F];
pub const PAD_WHITE: [u8; 3] = [0x00, 0x2A, 0x3F];
pub const PAD_BLACK: [u8; 3] = [0x00, 0x08, 0x3F];
/// Note events one period can hold; more are counted and dropped.
pub const MAX_EVENTS: usize = 128;
/// Frames Poly renders at a time into its own buffers.
const SCRATCH: usize = 256;
/// STOP's fade, in frames (10 ms at 44.1 kHz).
const FADE: u32 = 441;
/// How long a parameter must rest before its change is logged.
const LOG_AFTER: Duration = Duration::from_secs(1);

const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// A note's name: 48 is C3, 61 is C#4.
pub fn note_name(note: u8) -> String {
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 1)
}

/// A black key (C#, D#, F#, G#, A#).
fn black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// Counters for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub notes: u64,
    pub peak_voices: usize,
    /// Note events that did not fit in a period, plus notes Poly had no
    /// voice for (both should stay 0).
    pub dropped: u64,
    /// Blocks dropped for a non-finite sample (should stay 0).
    pub resets: u64,
}

pub struct Synth {
    poly: Poly,
    rate: u32,
    events: [Timed; MAX_EVENTS],
    len: usize,
    dropped_events: u64,
    left: [f32; SCRATCH],
    right: [f32; SCRATCH],
    /// The note each pad started, so its note-off ends that note even after
    /// an octave change.
    playing: [Option<u8>; 16],
    octave: i32,
    preset: usize,
    /// Frames left in STOP's fade, while `fading`.
    fade: u32,
    fading: bool,
    notes: u64,
    peak_voices: usize,
    resets: u64,
    /// Indices of the read-only parameters (meters), for the screen.
    meters: Vec<usize>,
}

impl Synth {
    /// Poly at `rate`, with [`START_PRESET`] loaded. Allocates: call it
    /// before the sound starts.
    pub fn new(rate: u32) -> Synth {
        let mut poly = Poly::new(rate);
        let preset = Poly::presets().iter().position(|p| p.name == START_PRESET).unwrap_or(0);
        poly.load_preset(&Poly::presets()[preset]);
        let meters = poly.params().iter().enumerate().filter(|(_, p)| p.readonly).map(|(i, _)| i).collect();
        Synth { poly, rate, events: [Timed::new(0, NodeEvent::AllOff); MAX_EVENTS], len: 0, dropped_events: 0,
                left: [0.0; SCRATCH], right: [0.0; SCRATCH], playing: [None; 16], octave: 0, preset, fade: 0,
                fading: false, notes: 0, peak_voices: 0, resets: 0, meters }
    }

    // ------------------------------------------------------------ notes

    /// The note pad `pad` plays now.
    pub fn note(&self, pad: usize) -> u8 {
        (BASE_NOTE as i32 + 12 * self.octave + pad as i32).clamp(0, 127) as u8
    }

    /// The notes of pad 1 and pad 16.
    pub fn range(&self) -> (u8, u8) {
        (self.note(0), self.note(15))
    }

    /// "C3 to D#4", for the log and the screen.
    pub fn range_text(&self) -> String {
        let (lo, hi) = self.range();
        format!("{} to {}", note_name(lo), note_name(hi))
    }

    #[cfg(test)]
    pub fn octave(&self) -> i32 {
        self.octave
    }

    /// Moves the pads by `delta` octaves, within [`MIN_OCTAVE`] and
    /// [`MAX_OCTAVE`]. Notes already held end on their own pad's note-off.
    /// Returns whether it moved.
    pub fn shift_octave(&mut self, delta: i32) -> bool {
        let o = (self.octave + delta).clamp(MIN_OCTAVE, MAX_OCTAVE);
        let moved = o != self.octave;
        self.octave = o;
        moved
    }

    /// Each pad's note name, for the pad grid.
    pub fn pad_labels(&self) -> Vec<Option<String>> {
        (0..16).map(|p| Some(note_name(self.note(p)))).collect()
    }

    /// The kit list's entry for the synth.
    pub fn kit_view(&self) -> crate::gui::KitView {
        crate::gui::KitView { name: KIT_NAME.to_string(), pads: self.pad_labels(), colour: PAD_WHITE }
    }

    /// Resting pad colours: C, white and black keys.
    pub fn pad_colours(&self) -> [[u8; 3]; 16] {
        let mut out = [[0u8; 3]; 16];
        for (p, c) in out.iter_mut().enumerate() {
            let n = self.note(p);
            *c = if n.is_multiple_of(12) { PAD_C } else if black(n) { PAD_BLACK } else { PAD_WHITE };
        }
        out
    }

    fn push(&mut self, offset: u32, ev: NodeEvent) {
        if self.len < MAX_EVENTS {
            self.events[self.len] = Timed::new(offset, ev);
            self.len += 1;
        } else {
            self.dropped_events += 1;
        }
    }

    /// Pad `pad` starts its note, `offset` frames into the next render. A
    /// pad already sounding ends its note first (Note Repeat's next hit).
    pub fn note_on(&mut self, pad: usize, velocity: u8, offset: u32) {
        if pad >= 16 || velocity == 0 {
            return;
        }
        if let Some(old) = self.playing[pad].take() {
            self.push(offset, NodeEvent::NoteOff { note: old });
        }
        let note = self.note(pad);
        self.push(offset, NodeEvent::NoteOn { note, velocity: velocity.min(127) });
        self.playing[pad] = Some(note);
        self.notes += 1;
    }

    /// Pad `pad` was let go (or its repeat ended), `offset` frames in.
    pub fn note_off(&mut self, pad: usize, offset: u32) {
        if let Some(note) = self.playing.get_mut(pad).and_then(Option::take) {
            self.push(offset, NodeEvent::NoteOff { note });
        }
    }

    /// Pad pressure, 0 to 127, on the note the pad plays.
    pub fn pressure(&mut self, pad: usize, value: u8) {
        if let Some(&Some(note)) = self.playing.get(pad) {
            self.push(0, NodeEvent::Pressure { note, value: value.min(127) });
        }
    }

    /// Every note ends with its release (the pads went back to drums).
    pub fn all_off(&mut self) {
        self.push(0, NodeEvent::AllOff);
        self.playing = [None; 16];
    }

    /// STOP: fade out over 10 ms, then reset, so nothing rings on.
    pub fn stop(&mut self) {
        self.all_off();
        if !self.fading {
            self.fading = true;
            self.fade = FADE;
        }
    }

    // ------------------------------------------------------------ sound

    /// Adds Poly's next `left.len()` frames into `left` and `right`.
    /// `frame` is the absolute frame of the first one. Never allocates.
    pub fn render(&mut self, frame: u64, bpm: f64, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let len = self.len;
        self.len = 0;
        if n == 0 {
            return;
        }
        // Stable insertion sort by frame (few events, no allocation); late
        // events land on the last frame.
        for e in &mut self.events[..len] {
            e.offset = e.offset.min(n as u32 - 1);
        }
        for i in 1..len {
            let mut j = i;
            while j > 0 && self.events[j - 1].offset > self.events[j].offset {
                self.events.swap(j - 1, j);
                j -= 1;
            }
        }
        let (mut start, mut e) = (0usize, 0usize);
        while start < n {
            let end = (start + SCRATCH).min(n);
            let first = e;
            while e < len && (self.events[e].offset as usize) < end {
                self.events[e].offset -= start as u32;
                e += 1;
            }
            let (sl, sr) = (&mut self.left[..end - start], &mut self.right[..end - start]);
            sl.fill(0.0);
            sr.fill(0.0);
            let ctx = Ctx { rate: self.rate, frame: frame + start as u64, bpm, beats: 0.0, playing: false, sidechain: None };
            self.poly.process(&ctx, &self.events[first..e], sl, sr);
            if !sl.iter().chain(sr.iter()).all(|s| s.is_finite()) {
                self.poly.reset();
                self.playing = [None; 16];
                self.resets += 1;
                start = end;
                continue;
            }
            if self.fading {
                for (l, r) in sl.iter_mut().zip(sr.iter_mut()) {
                    let g = self.fade as f32 / FADE as f32;
                    *l *= g;
                    *r *= g;
                    self.fade = self.fade.saturating_sub(1);
                }
                if self.fade == 0 {
                    self.poly.reset();
                    self.playing = [None; 16];
                    self.fading = false;
                }
            }
            for (o, s) in left[start..end].iter_mut().zip(sl.iter()) {
                *o += *s;
            }
            for (o, s) in right[start..end].iter_mut().zip(sr.iter()) {
                *o += *s;
            }
            start = end;
        }
        self.peak_voices = self.peak_voices.max(self.poly.active_voices());
    }

    pub fn active_voices(&self) -> usize {
        self.poly.active_voices()
    }

    pub fn counters(&self) -> Counters {
        Counters { notes: self.notes, peak_voices: self.peak_voices, dropped: self.dropped_events + self.poly.dropped,
                   resets: self.resets }
    }

    // ------------------------------------------------------------ the Synth tab

    /// Sets a parameter from the Synth tab (clamped by the node).
    pub fn set(&mut self, id: u16, value: f32) {
        self.poly.set_param(id, value);
    }

    pub fn value(&self, id: u16) -> f32 {
        self.poly.param(id)
    }

    /// Loads preset `i`; returns its name, or None if there is no such
    /// preset. Notes keep sounding; values glide to the preset's.
    pub fn load_preset(&mut self, i: usize) -> Option<&'static str> {
        let p = Poly::presets().get(i)?;
        self.poly.load_preset(p);
        self.preset = i;
        Some(p.name)
    }

    #[cfg(test)]
    pub fn preset(&self) -> usize {
        self.preset
    }

    pub fn preset_name(&self) -> &'static str {
        Poly::presets().get(self.preset).map_or("", |p| p.name)
    }

    /// Every value, in the node's parameter order (for the Synth tab).
    pub fn values(&self) -> Vec<f32> {
        self.poly.params().iter().map(|p| self.poly.param(p.id)).collect()
    }

    /// The meters' values, by parameter index.
    pub fn meters(&self) -> Vec<(usize, f32)> {
        let params = self.poly.params();
        self.meters.iter().map(|&i| (i, self.poly.param(params[i].id))).collect()
    }

    /// What the Synth tab shows.
    pub fn state(&self) -> SynthState {
        let presets = Poly::presets().iter().map(|p| p.name.to_string()).collect();
        SynthState::from_node(&self.poly, "Pads", Color::from_pad(PAD_WHITE), presets, Some(self.preset))
    }

    pub fn param_info(&self, id: u16) -> Option<&'static ParamInfo> {
        self.poly.params().iter().find(|p| p.id == id)
    }
}

/// Knob turns for the log: one line per parameter once it rests for a
/// second (or another one is turned), not one per detent.
#[derive(Default)]
pub struct ParamLog {
    pending: Option<(u16, f32, Instant)>,
}

impl ParamLog {
    /// A change; returns the line for the previous parameter if this is
    /// another one.
    pub fn set(&mut self, synth: &Synth, id: u16, value: f32, now: Instant) -> Option<String> {
        let line = match self.pending {
            Some((p, _, _)) if p != id => self.flush(synth),
            _ => None,
        };
        self.pending = Some((id, value, now));
        line
    }

    /// The line for a parameter that has rested long enough, if any.
    pub fn poll(&mut self, synth: &Synth, now: Instant) -> Option<String> {
        match self.pending {
            Some((_, _, t)) if now.duration_since(t) >= LOG_AFTER => self.flush(synth),
            _ => None,
        }
    }

    /// The pending line now (a preset load or the end of the session).
    pub fn flush(&mut self, synth: &Synth) -> Option<String> {
        let (id, _, _) = self.pending.take()?;
        let p = synth.param_info(id)?;
        // The node's own (clamped) value, not the one asked for.
        let v = synth.value(id);
        let name = if p.group == p.name { p.name.to_string() } else { format!("{} {}", p.group, p.name) };
        Some(format!("synth: {name} {}", value_line(p, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_synth::params as pp;

    fn run(s: &mut Synth, frame: &mut u64, periods: usize, block: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..periods {
            let (mut l, mut r) = (vec![0f32; block], vec![0f32; block]);
            s.render(*frame, 120.0, &mut l, &mut r);
            *frame += block as u64;
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn pads_play_semitones_from_c3_and_octaves_move_them() {
        let mut s = Synth::new(44_100);
        assert_eq!(s.range(), (48, 63));
        assert_eq!(s.range_text(), "C3 to D#4");
        assert_eq!(s.pad_labels()[1].as_deref(), Some("C#3"));
        assert!(s.shift_octave(1));
        assert_eq!(s.range_text(), "C4 to D#5");
        assert!(s.shift_octave(10));
        assert_eq!(s.octave(), MAX_OCTAVE);
        assert!(!s.shift_octave(1));
        assert!(s.shift_octave(-10));
        assert_eq!(s.range(), (12, 27));
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(0), "C-1");
    }

    #[test]
    fn pad_colours_are_ones_akai_sent() {
        let s = Synth::new(44_100);
        let c = s.pad_colours();
        assert_eq!(c[0], PAD_C);
        assert_eq!(c[1], PAD_BLACK);
        assert_eq!(c[2], PAD_WHITE);
        assert_eq!(c[12], PAD_C);
        for colour in [PAD_C, PAD_WHITE, PAD_BLACK] {
            assert!(crate::akai::RESTING_COLOURS.contains(&colour), "{colour:02X?}");
        }
    }

    #[test]
    fn a_pad_plays_a_note_that_adds_to_the_period_and_ends() {
        let mut s = Synth::new(44_100);
        let mut frame = 0;
        s.note_on(0, 100, 10);
        let mut l = [0.25f32; 128];
        let mut r = [0.25f32; 128];
        s.render(frame, 120.0, &mut l, &mut r);
        frame += 128;
        assert!(l[..10].iter().all(|&x| x == 0.25), "nothing before the note's frame");
        assert!(l.iter().any(|&x| x != 0.25), "Poly adds to what is there");
        assert_eq!(s.active_voices(), 1);
        let sound = run(&mut s, &mut frame, 40, 128);
        let p = peak(&sound);
        assert!(p > 0.05 && p < 1.0, "a note at about -12 dBFS: peak {p}");
        s.note_off(0, 0);
        run(&mut s, &mut frame, 300, 128);
        assert_eq!(s.active_voices(), 0);
        assert!(peak(&run(&mut s, &mut frame, 2, 128)) == 0.0);
        assert_eq!(s.counters(), Counters { notes: 1, peak_voices: 1, dropped: 0, resets: 0 });
    }

    #[test]
    fn the_same_notes_sound_the_same_in_any_block_size() {
        // (frame, pad, on); periods longer than the scratch buffers render in pieces.
        let script = [(3u64, 0usize, true), (64, 4, true), (500, 7, true), (2000, 0, false), (2100, 4, false),
                      (2600, 7, false), (3000, 2, true), (4000, 2, false)];
        let go = |block: usize| {
            let mut s = Synth::new(44_100);
            let mut frame = 0u64;
            let mut out = Vec::new();
            while frame < 6000 {
                let n = block.min((6000 - frame) as usize);
                for &(at, pad, on) in &script {
                    if (frame..frame + n as u64).contains(&at) {
                        let offset = (at - frame) as u32;
                        if on { s.note_on(pad, 90, offset) } else { s.note_off(pad, offset) }
                    }
                }
                out.extend(run(&mut s, &mut frame, 1, n));
            }
            out
        };
        let whole = go(128);
        assert!(peak(&whole) > 0.05);
        for block in [1, 31, 64, 1000] {
            assert!(go(block) == whole, "block size {block}");
        }
    }

    #[test]
    fn stop_fades_out_within_ten_ms() {
        let mut s = Synth::new(44_100);
        s.load_preset(Poly::presets().iter().position(|p| p.name == "Dust Pad").unwrap());
        let mut frame = 0;
        for pad in [0, 4, 7] {
            s.note_on(pad, 120, 0);
        }
        run(&mut s, &mut frame, 200, 128);
        assert_eq!(s.active_voices(), 3);
        s.stop();
        let out = run(&mut s, &mut frame, 4, 128);
        assert_eq!(s.active_voices(), 0);
        let after = &out[2 * FADE as usize..];
        assert!(after.iter().all(|&x| x == 0.0), "silent after the fade");
        // A pad let go after STOP sends nothing.
        s.note_off(0, 0);
        assert_eq!(s.len, 0);
    }

    #[test]
    fn note_repeat_retriggers_and_pressure_follows_the_pad() {
        let mut s = Synth::new(44_100);
        s.note_on(2, 100, 0);
        s.pressure(2, 90);
        s.note_on(2, 100, 50);
        s.pressure(9, 90); // not playing: nothing
        let evs: Vec<NodeEvent> = s.events[..s.len].iter().map(|t| t.ev).collect();
        assert_eq!(evs, vec![NodeEvent::NoteOn { note: 50, velocity: 100 }, NodeEvent::Pressure { note: 50, value: 90 },
                             NodeEvent::NoteOff { note: 50 }, NodeEvent::NoteOn { note: 50, velocity: 100 }]);
        // After an octave change the held pad's note-off still ends its note.
        s.shift_octave(1);
        s.note_off(2, 60);
        assert_eq!(s.events[s.len - 1].ev, NodeEvent::NoteOff { note: 50 });
    }

    #[test]
    fn too_many_events_are_counted_not_kept() {
        let mut s = Synth::new(44_100);
        for _ in 0..MAX_EVENTS + 5 {
            s.note_on(1, 80, 0);
        }
        assert_eq!(s.len, MAX_EVENTS);
        assert!(s.counters().dropped >= 5);
        let mut frame = 0;
        run(&mut s, &mut frame, 1, 128);
        assert_eq!(s.len, 0);
    }

    #[test]
    fn presets_and_values_reach_the_node_and_the_screen() {
        let mut s = Synth::new(44_100);
        assert_eq!(s.preset_name(), START_PRESET);
        let st = s.state();
        assert_eq!(st.info.id, "mpfree.poly");
        assert_eq!(st.preset, Some(s.preset()));
        assert_eq!(st.values, s.values());
        assert_eq!(s.load_preset(3), Some("Supersaw"));
        assert_eq!(s.value(pp::UNISON), 4.0);
        assert_eq!(s.load_preset(99), None);
        s.set(pp::CUTOFF, 1e9);
        assert_eq!(s.value(pp::CUTOFF), s.param_info(pp::CUTOFF).unwrap().max);
        let meters = s.meters();
        assert!(meters.iter().any(|&(i, _)| st.params[i].id == pp::METER_VOICES));
    }

    #[test]
    fn the_audio_threads_part_never_allocates() {
        let mut s = Synth::new(44_100);
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let ((), calls) = crate::sys::alloc_count::calls(|| {
            let mut frame = 0u64;
            for p in 0..3000usize {
                if p % 7 == 0 {
                    s.note_on(p % 16, 40 + (p % 80) as u8, (p % 128) as u32);
                }
                if p % 11 == 0 {
                    s.note_off((p + 5) % 16, 3);
                }
                if p % 5 == 0 {
                    s.pressure(p % 16, (p % 128) as u8);
                }
                if p % 97 == 0 {
                    s.set(pp::CUTOFF, 100.0 + (p % 5000) as f32);
                }
                if p % 401 == 0 {
                    s.load_preset(p / 401 % Poly::presets().len());
                }
                if p % 733 == 0 {
                    s.shift_octave(if p % 2 == 0 { 1 } else { -1 });
                }
                if p == 1500 {
                    s.all_off();
                }
                if p == 2500 {
                    s.stop();
                }
                s.render(frame, 120.0, &mut l, &mut r);
                frame += 128;
                let _ = (s.active_voices(), s.counters());
            }
        });
        assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
        assert!(s.counters().notes > 300);
    }

    #[test]
    fn knob_turns_log_one_line_per_rest() {
        let s = Synth::new(44_100);
        let mut log = ParamLog::default();
        let t0 = Instant::now();
        assert_eq!(log.set(&s, pp::CUTOFF, 300.0, t0), None);
        assert_eq!(log.set(&s, pp::CUTOFF, 310.0, t0), None);
        assert_eq!(log.poll(&s, t0 + Duration::from_millis(500)), None);
        let line = log.set(&s, pp::RESONANCE, 40.0, t0 + Duration::from_millis(600)).unwrap();
        assert!(line.starts_with("synth: Filter Cutoff "), "{line}");
        assert!(log.poll(&s, t0 + Duration::from_millis(1700)).unwrap().starts_with("synth: Filter Resonance "));
        assert_eq!(log.poll(&s, t0 + Duration::from_secs(5)), None);
    }
}

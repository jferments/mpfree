//! MPFree's own drum kits: every kit of the Drums synth, each pad rendered
//! to a sample. Made by the USB creator for the stick's kits partition, so
//! the pad player has kits before (and without) the owner's Akai kits. The
//! sounds are MPFree's own (the synth's code); nothing of Akai's is in them.

use crate::kitfile::{self, Sound, MAX_FRAMES, PAD_NOTES, RATE};
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::drum_params::{BASE_NOTE, PADS};
use mpfree_synth::Drums;

const BLOCK: usize = 256;
/// The longest render of a hit, in seconds.
const MAX_SECONDS: usize = 4;
/// Quieter than this all through (about -80 dB): a pad with no sound.
const SILENT: f32 = 1e-4;

/// Renders pad `pad` (0 to 15) of the loaded kit: interleaved stereo.
fn render_pad(d: &mut Drums, pad: usize) -> Vec<f32> {
    d.reset();
    let note = d.param(BASE_NOTE) as usize + pad;
    let mut out = Vec::new();
    let (mut l, mut r) = ([0f32; BLOCK], [0f32; BLOCK]);
    let mut frame = 0u64;
    let max = (RATE as usize * MAX_SECONDS).min(MAX_FRAMES);
    while out.len() / 2 < max {
        let ev = [Timed::new(
            0,
            NodeEvent::NoteOn {
                note: note.min(127) as u8,
                velocity: 127,
            },
        )];
        let events: &[Timed] = if frame == 0 { &ev } else { &[] };
        l.fill(0.0);
        r.fill(0.0);
        d.process(&Ctx::new(RATE, frame), events, &mut l, &mut r);
        for i in 0..BLOCK {
            out.push(l[i]);
            out.push(r[i]);
        }
        frame += BLOCK as u64;
        if frame > 0 && d.active_voices() == 0 {
            break;
        }
    }
    // Trim the silent end.
    let mut end = out.len() / 2;
    while end > 0 && out[2 * end - 2].abs() < SILENT && out[2 * end - 1].abs() < SILENT {
        end -= 1;
    }
    out.truncate(2 * end);
    out
}

/// The Drums synth's kits as (kit name, sounds), each sound on its pad's
/// note on the MPC One.
pub fn kits() -> Vec<(String, Vec<Sound>)> {
    let mut d = Drums::new(RATE);
    let mut out = Vec::new();
    for preset in Drums::presets() {
        d.load_preset(preset);
        let mut sounds = Vec::new();
        for pad in 0..PADS {
            let audio = render_pad(&mut d, pad);
            if audio.is_empty() {
                continue;
            }
            let peak = audio.iter().fold(0f32, |m, v| m.max(v.abs()));
            // Leave headroom like a sample pack: a hit peaks at about -1 dB at most.
            let gain = if peak > 0.89 { 0.89 / peak } else { 1.0 };
            let audio = audio
                .iter()
                .map(|v| (v * gain * 32767.0).round().clamp(-32768.0, 32767.0) as i16)
                .collect();
            sounds.push(Sound {
                note: PAD_NOTES[pad],
                name: format!("Pad {}", pad + 1),
                audio,
            });
        }
        if !sounds.is_empty() {
            out.push((format!("MPFree {}", preset.name), sounds));
        }
    }
    out
}

/// MPFree's own kits as a kits file (the stick's kits.bin).
pub fn kits_file() -> Vec<u8> {
    let kits: Vec<(String, Vec<u8>)> = kits()
        .into_iter()
        .map(|(n, s)| (n, kitfile::kit_bytes(&s)))
        .collect();
    kitfile::kits_bytes(&kits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drums_kit_renders() {
        let file = kits_file();
        let kits = kitfile::parse_kits(&file).unwrap();
        assert_eq!(kits.len(), Drums::presets().len());
        assert!(file.len() < kitfile::MAX_FILE, "{} bytes", file.len());
        for (name, sounds) in &kits {
            assert!(sounds.len() >= 8, "{name}: {} sounds", sounds.len());
            for s in sounds {
                assert!(
                    s.audio.iter().any(|v| v.unsigned_abs() > 1000),
                    "{name} {}: too quiet",
                    s.name
                );
            }
        }
        // The same every time.
        assert!(kits_file() == file);
    }
}

//! A drum kit made on the workstation by `offline-factory-kit.py` from the
//! owner's eMMC backup, and a small mixer that plays it.
//!
//! The kit file holds already-converted audio, so nothing here decodes WAV:
//!
//! ```text
//! "MPFKIT01"                         8 bytes
//! count                              u32 LE, 1..=16
//! count times:
//!   note                             u8, the controller's pad note
//!   reserved                         3 bytes, zero
//!   frames                           u32 LE, at most MAX_FRAMES
//!   name length                      u16 LE, at most 120
//!   name                             UTF-8
//!   audio                            frames x 2 channels x i16 LE, 44.1 kHz
//! ```

pub const MAGIC: &[u8; 8] = b"MPFKIT01";
pub const MAX_SOUNDS: usize = 16;
/// 10 s at 44.1 kHz per sound.
pub const MAX_FRAMES: u32 = 441_000;
pub const MAX_NAME: usize = 120;
/// Voices playing at once; the oldest stops when a new hit needs room.
pub const MAX_VOICES: usize = 24;

#[derive(Debug)]
pub struct Sound {
    pub note: u8,
    pub name: String,
    /// Interleaved stereo.
    pub audio: Vec<i16>,
}

impl Sound {
    pub fn frames(&self) -> usize {
        self.audio.len() / 2
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len()).ok_or("the kit file is cut short")?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
}

pub fn parse(data: &[u8]) -> Result<Vec<Sound>, String> {
    let mut r = Reader { data, at: 0 };
    if r.take(8)? != MAGIC {
        return Err("not a kit file (wrong magic)".to_string());
    }
    let count = r.u32()? as usize;
    if !(1..=MAX_SOUNDS).contains(&count) {
        return Err(format!("the kit has {count} sounds; 1 to {MAX_SOUNDS} are allowed"));
    }
    let mut sounds: Vec<Sound> = Vec::with_capacity(count);
    for i in 0..count {
        let head = r.take(4)?;
        let note = head[0];
        if note > 127 || head[1..] != [0, 0, 0] {
            return Err(format!("sound {}: bad note or reserved bytes", i + 1));
        }
        if sounds.iter().any(|s| s.note == note) {
            return Err(format!("sound {}: note {note} appears twice", i + 1));
        }
        let frames = r.u32()?;
        if frames == 0 || frames > MAX_FRAMES {
            return Err(format!("sound {}: {frames} frames; 1 to {MAX_FRAMES} are allowed", i + 1));
        }
        let name_len = r.u16()? as usize;
        if name_len > MAX_NAME {
            return Err(format!("sound {}: name too long", i + 1));
        }
        let name = String::from_utf8_lossy(r.take(name_len)?).chars().filter(|c| !c.is_control()).collect();
        let raw = r.take(frames as usize * 4)?;
        let audio = raw.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        sounds.push(Sound { note, name, audio });
    }
    if r.at != data.len() {
        return Err("the kit file has extra bytes at the end".to_string());
    }
    Ok(sounds)
}

struct Voice {
    sound: usize,
    pos: usize,
    gain: f32,
    started: u64,
}

pub struct Mixer {
    pub sounds: Vec<Sound>,
    voices: Vec<Voice>,
    hits: u64,
}

impl Mixer {
    pub fn new(sounds: Vec<Sound>) -> Self {
        Mixer { sounds, voices: Vec::with_capacity(MAX_VOICES), hits: 0 }
    }

    /// Start the sound for this pad note. Velocity scales the level linearly.
    /// Returns the sound's index, or None if the kit has nothing on that note.
    pub fn hit(&mut self, note: u8, velocity: u8) -> Option<usize> {
        let sound = self.sounds.iter().position(|s| s.note == note)?;
        if self.voices.len() >= MAX_VOICES {
            let oldest = (0..self.voices.len()).min_by_key(|&i| self.voices[i].started).unwrap();
            self.voices.swap_remove(oldest);
        }
        self.hits += 1;
        self.voices.push(Voice { sound, pos: 0, gain: velocity.min(127) as f32 / 127.0, started: self.hits });
        Some(sound)
    }

    pub fn playing(&self) -> usize {
        self.voices.len()
    }

    /// Add every playing voice into `left`/`right` (cleared first), scaled by `master`.
    /// Returns the largest absolute sample written, before clamping to -1..1.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], master: f32) -> f32 {
        left.fill(0.0);
        right.fill(0.0);
        let sounds = &self.sounds;
        self.voices.retain_mut(|v| {
            let audio = &sounds[v.sound].audio;
            let frames = audio.len() / 2;
            let n = left.len().min(frames - v.pos);
            let g = v.gain * master / 32768.0;
            for f in 0..n {
                left[f] += audio[2 * (v.pos + f)] as f32 * g;
                right[f] += audio[2 * (v.pos + f) + 1] as f32 * g;
            }
            v.pos += n;
            v.pos < frames
        });
        let mut peak = 0f32;
        for s in left.iter_mut().chain(right.iter_mut()) {
            peak = peak.max(s.abs());
            *s = s.clamp(-1.0, 1.0);
        }
        peak
    }
}

#[cfg(test)]
pub fn build(sounds: &[(u8, &str, &[i16])]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(sounds.len() as u32).to_le_bytes());
    for (note, name, audio) in sounds {
        out.extend_from_slice(&[*note, 0, 0, 0]);
        out.extend_from_slice(&((audio.len() / 2) as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        for s in *audio {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_kit_and_rejects_bad_ones() {
        let data = build(&[(37, "Kick", &[1000, -1000, 2000, -2000]), (36, "Snare", &[5, 6])]);
        let kit = parse(&data).unwrap();
        assert_eq!(kit.len(), 2);
        assert_eq!((kit[0].note, kit[0].name.as_str(), kit[0].frames()), (37, "Kick", 2));
        assert_eq!(kit[1].audio, vec![5, 6]);
        assert!(parse(&data[..data.len() - 1]).unwrap_err().contains("cut short"));
        let mut extra = data.clone();
        extra.push(0);
        assert!(parse(&extra).unwrap_err().contains("extra bytes"));
        let mut magic = data.clone();
        magic[0] = b'X';
        assert!(parse(&magic).is_err());
        let twice = build(&[(37, "A", &[1, 1]), (37, "B", &[1, 1])]);
        assert!(parse(&twice).unwrap_err().contains("twice"));
        assert!(parse(&build(&[])).is_err());
        let empty = build(&[(37, "A", &[])]);
        assert!(parse(&empty).is_err());
    }

    #[test]
    fn mixes_hits_and_stops_at_the_end_of_each_sound() {
        let kick: Vec<i16> = vec![16384; 2 * 3]; // 3 frames at half scale
        let data = build(&[(37, "Kick", &kick)]);
        let mut m = Mixer::new(parse(&data).unwrap());
        assert_eq!(m.hit(99, 127), None);
        assert_eq!(m.hit(37, 127), Some(0));
        let (mut l, mut r) = ([0f32; 2], [0f32; 2]);
        let peak = m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.5, 0.5]);
        assert_eq!(peak, 0.5);
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.5, 0.0]);
        assert_eq!(m.playing(), 0);
        // Two hits add up, and the sum is clamped.
        m.hit(37, 127);
        m.hit(37, 127);
        m.hit(37, 127);
        let peak = m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [1.0, 1.0]);
        assert_eq!(peak, 1.5);
        // Velocity and master scale it.
        let mut m = Mixer::new(parse(&data).unwrap());
        m.hit(37, 0);
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.0, 0.0]);
    }

    #[test]
    fn steals_the_oldest_voice() {
        let data = build(&[(37, "Long", &vec![1i16; 2 * 1000])]);
        let mut m = Mixer::new(parse(&data).unwrap());
        for _ in 0..MAX_VOICES + 5 {
            m.hit(37, 100);
        }
        assert_eq!(m.playing(), MAX_VOICES);
    }
}

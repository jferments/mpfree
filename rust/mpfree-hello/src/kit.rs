//! Drum kits made on the workstation by `offline-factory-kit.py` from the
//! owner's eMMC backup, and a small mixer that plays them.
//!
//! The kits file holds already-converted audio, so nothing here decodes WAV:
//!
//! ```text
//! "MPFKITS1"                         8 bytes
//! kit count                          u32 LE, 1..=16
//! kit count times:
//!   kit name length                  u16 LE, at most 120
//!   kit name                         UTF-8
//!   kit length                       u32 LE, the bytes of the kit that follows
//!   kit                              as below
//! ```
//!
//! Each kit:
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
pub const KITS_MAGIC: &[u8; 8] = b"MPFKITS1";
pub const MAX_KITS: usize = 16;
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

#[derive(Debug)]
pub struct Kit {
    pub name: String,
    pub sounds: Vec<Sound>,
}

fn clean_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().filter(|c| !c.is_control()).collect()
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
        let name = clean_name(r.take(name_len)?);
        let raw = r.take(frames as usize * 4)?;
        let audio = raw.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        sounds.push(Sound { note, name, audio });
    }
    if r.at != data.len() {
        return Err("the kit file has extra bytes at the end".to_string());
    }
    Ok(sounds)
}

/// Parse a kits file (several named kits, each in the single-kit format).
pub fn parse_kits(data: &[u8]) -> Result<Vec<Kit>, String> {
    let mut r = Reader { data, at: 0 };
    if r.take(8)? != KITS_MAGIC {
        return Err("not a kits file (wrong magic)".to_string());
    }
    let count = r.u32()? as usize;
    if !(1..=MAX_KITS).contains(&count) {
        return Err(format!("the file has {count} kits; 1 to {MAX_KITS} are allowed"));
    }
    let mut kits = Vec::with_capacity(count);
    for i in 0..count {
        let name_len = r.u16()? as usize;
        if name_len > MAX_NAME {
            return Err(format!("kit {}: name too long", i + 1));
        }
        let name = clean_name(r.take(name_len)?);
        let len = r.u32()? as usize;
        let sounds = parse(r.take(len)?).map_err(|e| format!("kit {} ({name}): {e}", i + 1))?;
        kits.push(Kit { name, sounds });
    }
    if r.at != data.len() {
        return Err("the kits file has extra bytes at the end".to_string());
    }
    Ok(kits)
}

struct Voice {
    kit: usize,
    sound: usize,
    pos: usize,
    /// Frames of silence before the sound starts (its place in the period).
    wait: usize,
    gain: f32,
    started: u64,
}

pub struct Mixer {
    pub kits: Vec<Kit>,
    kit: usize,
    voices: Vec<Voice>,
    hits: u64,
}

impl Mixer {
    /// Plays the first kit until `select` picks another. `kits` must not be empty.
    pub fn new(kits: Vec<Kit>) -> Self {
        assert!(!kits.is_empty());
        Mixer { kits, kit: 0, voices: Vec::with_capacity(MAX_VOICES), hits: 0 }
    }

    pub fn kit(&self) -> usize {
        self.kit
    }

    /// Play kit `k` from the next hit on. Sounds already playing finish.
    pub fn select(&mut self, k: usize) {
        if k < self.kits.len() {
            self.kit = k;
        }
    }

    pub fn sounds(&self) -> &[Sound] {
        &self.kits[self.kit].sounds
    }

    /// Start the sound for this pad note in the current kit. Velocity scales
    /// the level linearly. Returns the sound's index, or None if the kit has
    /// nothing on that note.
    pub fn hit(&mut self, note: u8, velocity: u8) -> Option<usize> {
        self.hit_at(note, velocity, 0)
    }

    /// Like `hit`, but the sound starts `offset` frames into the next
    /// `render`, so a hit lands on its exact frame (Note Repeat).
    pub fn hit_at(&mut self, note: u8, velocity: u8, offset: usize) -> Option<usize> {
        let kit = self.kit;
        let sound = self.kits[kit].sounds.iter().position(|s| s.note == note)?;
        if self.voices.len() >= MAX_VOICES {
            let oldest = (0..self.voices.len()).min_by_key(|&i| self.voices[i].started).unwrap();
            self.voices.swap_remove(oldest);
        }
        self.hits += 1;
        self.voices.push(Voice { kit, sound, pos: 0, wait: offset, gain: velocity.min(127) as f32 / 127.0,
                                 started: self.hits });
        Some(sound)
    }

    /// Stop every sound at once.
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    pub fn playing(&self) -> usize {
        self.voices.len()
    }

    /// Add every playing voice into `left`/`right` (cleared first), scaled by `master`.
    /// Returns the largest absolute sample written, before clamping to -1..1.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], master: f32) -> f32 {
        left.fill(0.0);
        right.fill(0.0);
        let kits = &self.kits;
        let len = left.len().min(right.len());
        self.voices.retain_mut(|v| {
            if v.wait >= len {
                v.wait -= len;
                return true;
            }
            let start = std::mem::take(&mut v.wait);
            let audio = &kits[v.kit].sounds[v.sound].audio;
            let frames = audio.len() / 2;
            let n = (len - start).min(frames - v.pos);
            let g = v.gain * master / 32768.0;
            for f in 0..n {
                left[start + f] += audio[2 * (v.pos + f)] as f32 * g;
                right[start + f] += audio[2 * (v.pos + f) + 1] as f32 * g;
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
pub fn build_kits(kits: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = KITS_MAGIC.to_vec();
    out.extend_from_slice(&(kits.len() as u32).to_le_bytes());
    for (name, kit) in kits {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(kit.len() as u32).to_le_bytes());
        out.extend_from_slice(kit);
    }
    out
}

#[cfg(test)]
pub fn one_kit(sounds: Vec<Sound>) -> Vec<Kit> {
    vec![Kit { name: "test".to_string(), sounds }]
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
        let mut m = Mixer::new(one_kit(parse(&data).unwrap()));
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
        let mut m = Mixer::new(one_kit(parse(&data).unwrap()));
        m.hit(37, 0);
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.0, 0.0]);
    }

    #[test]
    fn hits_can_start_later_in_the_period() {
        let kick: Vec<i16> = vec![16384; 2 * 3]; // 3 frames at half scale
        let data = build(&[(37, "Kick", &kick)]);
        let mut m = Mixer::new(one_kit(parse(&data).unwrap()));
        let (mut l, mut r) = ([0f32; 4], [0f32; 4]);
        // Two frames in: frames 2 and 3 of this period, then the last one next period.
        assert_eq!(m.hit_at(37, 127, 2), Some(0));
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.0, 0.0, 0.5, 0.5]);
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.5, 0.0, 0.0, 0.0]);
        assert_eq!(m.playing(), 0);
        // An offset past this period waits for the next one.
        m.hit_at(37, 127, 5);
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.0; 4]);
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.0, 0.5, 0.5, 0.5]);
        // stop_all silences everything at once.
        m.hit(37, 127);
        m.stop_all();
        m.render(&mut l, &mut r, 1.0);
        assert_eq!((l, m.playing()), ([0.0; 4], 0));
    }

    #[test]
    fn parses_kits_and_switches_between_them() {
        let a = build(&[(37, "Kick", &[16384, 16384, 16384, 16384])]);
        let b = build(&[(37, "Clap", &[-16384, -16384, -16384, -16384]), (36, "Hat", &[1, 1])]);
        let data = build_kits(&[("House", a.clone()), ("Trap\u{7}", b.clone())]);
        let kits = parse_kits(&data).unwrap();
        assert_eq!(kits.len(), 2);
        assert_eq!((kits[0].name.as_str(), kits[1].name.as_str()), ("House", "Trap"));
        assert_eq!(kits[1].sounds[1].name, "Hat");
        assert!(parse_kits(&data[..data.len() - 1]).is_err());
        let mut extra = data.clone();
        extra.push(0);
        assert!(parse_kits(&extra).unwrap_err().contains("extra bytes"));
        assert!(parse_kits(&a).unwrap_err().contains("wrong magic"));
        assert!(parse_kits(&build_kits(&[])).is_err());
        let mut bad = b.clone();
        bad[0] = b'X';
        assert!(parse_kits(&build_kits(&[("Bad", bad)])).unwrap_err().contains("kit 1 (Bad)"));

        // A hit plays the selected kit; a sound already playing finishes after a switch.
        let mut m = Mixer::new(kits);
        let (mut l, mut r) = ([0f32; 1], [0f32; 1]);
        assert_eq!(m.hit(37, 127), Some(0));
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.5]);
        m.select(1);
        m.select(9); // out of range: ignored
        assert_eq!((m.kit(), m.sounds()[0].name.as_str()), (1, "Clap"));
        assert_eq!(m.hit(37, 127), Some(0));
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [0.0]); // the kick's last frame plus the clap's first
        m.render(&mut l, &mut r, 1.0);
        assert_eq!(l, [-0.5]);
    }

    #[test]
    fn steals_the_oldest_voice() {
        let data = build(&[(37, "Long", &vec![1i16; 2 * 1000])]);
        let mut m = Mixer::new(one_kit(parse(&data).unwrap()));
        for _ in 0..MAX_VOICES + 5 {
            m.hit(37, 100);
        }
        assert_eq!(m.playing(), MAX_VOICES);
    }
}

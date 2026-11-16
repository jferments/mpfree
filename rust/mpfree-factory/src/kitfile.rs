//! The pad player's kits file, written: `MPFKITS1`, a list of named kits,
//! each an `MPFKIT01` kit of up to 16 sounds (44.1 kHz stereo, 16-bit). The
//! pad player's reader and the full layout are in `mpfree-app/src/kit.rs`.

/// One kit's magic.
pub const KIT_MAGIC: &[u8; 8] = b"MPFKIT01";
/// A kits file's magic.
pub const KITS_MAGIC: &[u8; 8] = b"MPFKITS1";
pub const MAX_KITS: usize = 16;
pub const MAX_SOUNDS: usize = 16;
/// 10 s at 44.1 kHz per sound.
pub const MAX_FRAMES: usize = 441_000;
pub const MAX_NAME: usize = 120;
/// The largest kits file the tools make (what the pad player loads in one go).
pub const MAX_FILE: usize = 48 << 20;
pub const RATE: u32 = 44_100;

/// The MPC One's pad notes, pads 1 to 16 (mpfree-controller's
/// `mpc_one::PAD_NOTES`; the pad player's tests check they agree).
pub const PAD_NOTES: [u8; 16] = [
    37, 36, 42, 82, 40, 38, 46, 44, 48, 47, 45, 43, 49, 55, 51, 53,
];

/// One sound: the pad note it plays on, a name and interleaved stereo audio.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub note: u8,
    pub name: String,
    pub audio: Vec<i16>,
}

/// At most `limit` bytes of `text`, never splitting a character.
pub fn utf8_cut(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// One kit's bytes. Sounds past [`MAX_SOUNDS`], empty ones and repeated
/// notes are left out; audio past [`MAX_FRAMES`] is cut.
pub fn kit_bytes(sounds: &[Sound]) -> Vec<u8> {
    let mut kept: Vec<&Sound> = Vec::new();
    for s in sounds {
        if kept.len() < MAX_SOUNDS
            && s.audio.len() >= 2
            && s.note <= 127
            && !kept.iter().any(|k| k.note == s.note)
        {
            kept.push(s);
        }
    }
    let mut out = KIT_MAGIC.to_vec();
    out.extend_from_slice(&(kept.len() as u32).to_le_bytes());
    for s in kept {
        let frames = (s.audio.len() / 2).min(MAX_FRAMES);
        let name = utf8_cut(&s.name, MAX_NAME);
        out.extend_from_slice(&[s.note, 0, 0, 0]);
        out.extend_from_slice(&(frames as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        for v in &s.audio[..frames * 2] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// The bytes one kit takes inside a kits file.
pub fn entry_len(name: &str, kit: &[u8]) -> usize {
    2 + utf8_cut(name, MAX_NAME).len() + 4 + kit.len()
}

/// A kits file from (name, kit bytes) pairs; at most [`MAX_KITS`] are used.
pub fn kits_bytes(kits: &[(String, Vec<u8>)]) -> Vec<u8> {
    let kits = &kits[..kits.len().min(MAX_KITS)];
    let mut out = KITS_MAGIC.to_vec();
    out.extend_from_slice(&(kits.len() as u32).to_le_bytes());
    for (name, kit) in kits {
        let name = utf8_cut(name, MAX_NAME);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(kit.len() as u32).to_le_bytes());
        out.extend_from_slice(kit);
    }
    out
}

/// The kits in a kits file: (name, sounds), checked as the pad player
/// checks them. For tests and reports.
pub fn parse_kits(data: &[u8]) -> Result<Vec<(String, Vec<Sound>)>, String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = data.get(at..at + n).ok_or("the kits file ends early")?;
        at += n;
        Ok(s)
    };
    if take(8)? != KITS_MAGIC {
        return Err("not a kits file".into());
    }
    let count = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    if !(1..=MAX_KITS).contains(&count) {
        return Err(format!("{count} kits"));
    }
    let mut kits = Vec::new();
    for _ in 0..count {
        let nl = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        let name = String::from_utf8_lossy(take(nl)?).into_owned();
        let len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let kit = take(len)?;
        kits.push((name, parse_kit(kit)?));
    }
    if at != data.len() {
        return Err("extra bytes at the end".into());
    }
    Ok(kits)
}

fn parse_kit(d: &[u8]) -> Result<Vec<Sound>, String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = d.get(at..at + n).ok_or("a kit ends early")?;
        at += n;
        Ok(s)
    };
    if take(8)? != KIT_MAGIC {
        return Err("not a kit".into());
    }
    let count = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    if !(1..=MAX_SOUNDS).contains(&count) {
        return Err(format!("{count} sounds"));
    }
    let mut out: Vec<Sound> = Vec::new();
    for _ in 0..count {
        let head = take(4)?;
        let note = head[0];
        if note > 127 || head[1..] != [0, 0, 0] || out.iter().any(|s| s.note == note) {
            return Err("a bad or repeated note".into());
        }
        let frames = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        if frames == 0 || frames > MAX_FRAMES {
            return Err(format!("{frames} frames"));
        }
        let nl = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        if nl > MAX_NAME {
            return Err("a name too long".into());
        }
        let name = String::from_utf8_lossy(take(nl)?).into_owned();
        let audio = take(frames * 4)?
            .chunks(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        out.push(Sound { note, name, audio });
    }
    if at != d.len() {
        return Err("extra bytes in a kit".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let a = Sound {
            note: 37,
            name: "Kick é".into(),
            audio: vec![1, -1, 2, -2],
        };
        let b = Sound {
            note: 36,
            name: "x".repeat(200),
            audio: vec![5; 10],
        };
        let dup = Sound {
            note: 37,
            name: "again".into(),
            audio: vec![9; 4],
        };
        let empty = Sound {
            note: 40,
            name: "empty".into(),
            audio: vec![],
        };
        let kit = kit_bytes(&[a.clone(), b.clone(), dup, empty]);
        let file = kits_bytes(&[("One".into(), kit.clone())]);
        assert_eq!(file.len(), 12 + entry_len("One", &kit));
        let kits = parse_kits(&file).unwrap();
        assert_eq!(kits.len(), 1);
        assert_eq!(kits[0].1.len(), 2);
        assert_eq!(kits[0].1[0], a);
        assert_eq!(kits[0].1[1].name.len(), MAX_NAME);
        assert_eq!(kits[0].1[1].audio, b.audio);
    }

    #[test]
    fn cut_keeps_characters_whole() {
        assert_eq!(utf8_cut("aé", 2), "a");
        assert_eq!(utf8_cut("aé", 3), "aé");
    }
}

//! DX7 voice data: reading a player's own banks and single voices from
//! system exclusive (.syx) files into FM parameters, and writing them back.
//!
//! - A **bank** is 32 voices packed into 128 bytes each: `F0 43 0n 09 20 00`,
//!   4096 bytes of data, a checksum and `F7` (4104 bytes).
//! - A **single voice** is 155 bytes unpacked: `F0 43 0n 00 01 1B`, the
//!   data, a checksum and `F7` (163 bytes).
//!
//! Operators are stored from 6 down to 1. Only voice data is read: nothing
//! is sent anywhere. MPFree ships no Yamaha voices; players load banks
//! they own.

use crate::fm_params::*;

/// One voice: its name and its values as FM parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Dx7Voice {
    pub name: String,
    pub values: Vec<(u16, f32)>,
}

impl Dx7Voice {
    pub fn preset(&self) -> FmPreset {
        FmPreset { name: self.name.clone(), values: self.values.clone() }
    }

    pub fn from_preset(p: &FmPreset) -> Dx7Voice {
        Dx7Voice { name: p.name.clone(), values: p.values.clone() }
    }

    /// A parameter's value: the last one given for it, else its default.
    pub fn value(&self, id: u16) -> f32 {
        self.values.iter().rev().find(|v| v.0 == id).map(|v| v.1).unwrap_or_else(|| info(id).map_or(0.0, |p| p.default))
    }
}

const BANK_LEN: usize = 4104;
const VOICE_LEN: usize = 163;

/// The checksum the DX7 expects: the low 7 bits of minus the sum.
pub fn checksum(data: &[u8]) -> u8 {
    let sum = data.iter().fold(0u32, |s, &b| s.wrapping_add(u32::from(b)));
    ((128 - (sum & 0x7F)) & 0x7F) as u8
}

/// Every voice in a .syx file: a 32-voice bank or one voice. A bad checksum
/// is an error; so is any other format.
pub fn parse(bytes: &[u8]) -> Result<Vec<Dx7Voice>, String> {
    if bytes.len() < 6 || bytes[0] != 0xF0 || bytes[1] != 0x43 {
        return Err("not a Yamaha system exclusive message".to_string());
    }
    match (bytes[3], bytes.len()) {
        (0x09, BANK_LEN) => {
            let data = &bytes[6..6 + 4096];
            if checksum(data) != bytes[6 + 4096] || bytes[BANK_LEN - 1] != 0xF7 {
                return Err("bank checksum or end byte is wrong".to_string());
            }
            Ok(data.chunks(128).map(|v| unpack_voice(v.try_into().expect("128 bytes"))).collect())
        }
        (0x00, VOICE_LEN) => {
            let data = &bytes[6..6 + 155];
            if checksum(data) != bytes[6 + 155] || bytes[VOICE_LEN - 1] != 0xF7 {
                return Err("voice checksum or end byte is wrong".to_string());
            }
            Ok(vec![from_unpacked(data.try_into().expect("155 bytes"))])
        }
        (f, n) => Err(format!("format {f:#04x} with {n} bytes is not a DX7 bank (4104) or voice (163)")),
    }
}

/// The 155-byte unpacked form of a packed 128-byte voice.
pub fn unpack(p: &[u8; 128]) -> [u8; 155] {
    let mut u = [0u8; 155];
    for k in 0..6 {
        let (s, d) = (&p[k * 17..k * 17 + 17], &mut u[k * 21..k * 21 + 21]);
        d[..11].copy_from_slice(&s[..11]);
        d[11] = s[11] & 3; // left curve
        d[12] = (s[11] >> 2) & 3; // right curve
        d[13] = s[12] & 7; // rate scaling
        d[14] = s[13] & 3; // amp mod sensitivity
        d[15] = (s[13] >> 2) & 7; // velocity sensitivity
        d[16] = s[14]; // output level
        d[17] = s[15] & 1; // mode
        d[18] = (s[15] >> 1) & 31; // coarse
        d[19] = s[16]; // fine
        d[20] = (s[12] >> 3) & 15; // detune
    }
    u[126..134].copy_from_slice(&p[102..110]); // pitch envelope
    u[134] = p[110] & 31; // algorithm
    u[135] = p[111] & 7; // feedback
    u[136] = (p[111] >> 3) & 1; // osc key sync
    u[137..141].copy_from_slice(&p[112..116]); // LFO speed, delay, PMD, AMD
    u[141] = p[116] & 1; // LFO key sync
    u[142] = (p[116] >> 1) & 7; // LFO wave
    u[143] = (p[116] >> 4) & 7; // pitch mod sensitivity
    u[144] = p[117]; // transpose
    u[145..155].copy_from_slice(&p[118..128]); // name
    u
}

/// The packed 128-byte form of an unpacked voice.
pub fn pack(u: &[u8; 155]) -> [u8; 128] {
    let mut p = [0u8; 128];
    for k in 0..6 {
        let (s, d) = (&u[k * 21..k * 21 + 21], &mut p[k * 17..k * 17 + 17]);
        d[..11].copy_from_slice(&s[..11]);
        d[11] = (s[11] & 3) | ((s[12] & 3) << 2);
        d[12] = (s[13] & 7) | ((s[20] & 15) << 3);
        d[13] = (s[14] & 3) | ((s[15] & 7) << 2);
        d[14] = s[16];
        d[15] = (s[17] & 1) | ((s[18] & 31) << 1);
        d[16] = s[19];
    }
    p[102..110].copy_from_slice(&u[126..134]);
    p[110] = u[134] & 31;
    p[111] = (u[135] & 7) | ((u[136] & 1) << 3);
    p[112..116].copy_from_slice(&u[137..141]);
    p[116] = (u[141] & 1) | ((u[142] & 7) << 1) | ((u[143] & 7) << 4);
    p[117] = u[144];
    p[118..128].copy_from_slice(&u[145..155]);
    p
}

fn unpack_voice(p: &[u8; 128]) -> Dx7Voice {
    from_unpacked(&unpack(p))
}

/// FM parameters from the 155-byte unpacked form. Out-of-range bytes are
/// clamped by the parameters' ranges when loaded.
pub fn from_unpacked(u: &[u8; 155]) -> Dx7Voice {
    let mut values = Vec::with_capacity(N_PARAMS);
    for k in 0..6 {
        // Stored from operator 6 down to 1.
        let n = 6 - k;
        let s = &u[k * 21..k * 21 + 21];
        let f = |field: u16| op_id(n, field);
        for i in 0..4u16 {
            values.push((f(op::R1 + i), f32::from(s[i as usize])));
            values.push((f(op::L1 + i), f32::from(s[4 + i as usize])));
        }
        values.push((f(op::BREAK), f32::from(s[8])));
        values.push((f(op::L_DEPTH), f32::from(s[9])));
        values.push((f(op::R_DEPTH), f32::from(s[10])));
        values.push((f(op::L_CURVE), f32::from(s[11])));
        values.push((f(op::R_CURVE), f32::from(s[12])));
        values.push((f(op::RATE_SCALE), f32::from(s[13])));
        values.push((f(op::AMS), f32::from(s[14])));
        values.push((f(op::VEL_SENS), f32::from(s[15])));
        values.push((f(op::LEVEL), f32::from(s[16])));
        values.push((f(op::MODE), f32::from(s[17])));
        values.push((f(op::COARSE), f32::from(s[18])));
        values.push((f(op::FINE), f32::from(s[19])));
        values.push((f(op::DETUNE), f32::from(s[20]) - 7.0));
        values.push((f(op::ON), 1.0));
    }
    let g = |i: usize| f32::from(u[i]);
    values.extend_from_slice(&[
        (PEG_R1, g(126)), (PEG_R2, g(127)), (PEG_R3, g(128)), (PEG_R4, g(129)),
        (PEG_L1, g(130)), (PEG_L2, g(131)), (PEG_L3, g(132)), (PEG_L4, g(133)),
        (ALGORITHM, g(134) + 1.0), (FEEDBACK, g(135)), (OSC_SYNC, g(136)),
        (LFO_SPEED, g(137)), (LFO_DELAY, g(138)), (LFO_PMD, g(139)), (LFO_AMD, g(140)),
        (LFO_SYNC, g(141)), (LFO_WAVE, g(142)), (LFO_PMS, g(143)),
        // DX7 transpose 0 to 48, with 24 at C3: no transpose.
        (TRANSPOSE, g(144) - 24.0),
    ]);
    let name: String = u[145..155].iter().map(|&b| if (32..127).contains(&b) { b as char } else { ' ' }).collect();
    Dx7Voice { name: name.trim_end().to_string(), values }
}

/// The 155-byte unpacked form of a voice.
pub fn to_unpacked(voice: &Dx7Voice) -> [u8; 155] {
    let value = |id: u16| voice.value(id);
    let name = voice.name.as_str();
    let mut u = [0u8; 155];
    let b = |v: f32, max: u8| (v.round().max(0.0) as u8).min(max);
    for k in 0..6 {
        let n = 6 - k;
        let f = |field: u16| value(op_id(n, field));
        let s = &mut u[k * 21..k * 21 + 21];
        for i in 0..4u16 {
            s[i as usize] = b(f(op::R1 + i), 99);
            s[4 + i as usize] = b(f(op::L1 + i), 99);
        }
        s[8] = b(f(op::BREAK), 99);
        s[9] = b(f(op::L_DEPTH), 99);
        s[10] = b(f(op::R_DEPTH), 99);
        s[11] = b(f(op::L_CURVE), 3);
        s[12] = b(f(op::R_CURVE), 3);
        s[13] = b(f(op::RATE_SCALE), 7);
        s[14] = b(f(op::AMS), 3);
        s[15] = b(f(op::VEL_SENS), 7);
        // An operator switched off is stored at output level 0.
        s[16] = if f(op::ON) >= 0.5 { b(f(op::LEVEL), 99) } else { 0 };
        s[17] = b(f(op::MODE), 1);
        s[18] = b(f(op::COARSE), 31);
        s[19] = b(f(op::FINE), 99);
        s[20] = b(f(op::DETUNE) + 7.0, 14);
    }
    let pegs = [PEG_R1, PEG_R2, PEG_R3, PEG_R4, PEG_L1, PEG_L2, PEG_L3, PEG_L4];
    for (i, id) in pegs.iter().enumerate() {
        u[126 + i] = b(value(*id), 99);
    }
    u[134] = b(value(ALGORITHM) - 1.0, 31);
    u[135] = b(value(FEEDBACK), 7);
    u[136] = b(value(OSC_SYNC), 1);
    u[137] = b(value(LFO_SPEED), 99);
    u[138] = b(value(LFO_DELAY), 99);
    u[139] = b(value(LFO_PMD), 99);
    u[140] = b(value(LFO_AMD), 99);
    u[141] = b(value(LFO_SYNC), 1);
    u[142] = b(value(LFO_WAVE), 5);
    u[143] = b(value(LFO_PMS), 7);
    u[144] = b(value(TRANSPOSE) + 24.0, 48);
    let mut chars = name.bytes().filter(|c| (32..127).contains(c)).chain(std::iter::repeat(b' '));
    for c in &mut u[145..155] {
        *c = chars.next().unwrap_or(b' ');
    }
    u
}

/// A 32-voice bank file from up to 32 voices; empty slots are the init
/// voice.
pub fn bank(voices: &[Dx7Voice]) -> Vec<u8> {
    let init = Dx7Voice { name: "INIT VOICE".to_string(), values: Vec::new() };
    let mut data = Vec::with_capacity(4096);
    for k in 0..32 {
        data.extend_from_slice(&pack(&to_unpacked(voices.get(k).unwrap_or(&init))));
    }
    let mut out = vec![0xF0, 0x43, 0x00, 0x09, 0x20, 0x00];
    out.extend_from_slice(&data);
    out.push(checksum(&data));
    out.push(0xF7);
    out
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn packing_round_trips_every_byte_in_range() {
        // Every byte a varied value inside its field's range.
        const OP: [u8; 21] = [99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 3, 3, 7, 3, 7, 99, 1, 31, 99, 14];
        const GLOBAL: [u8; 19] = [99, 99, 99, 99, 99, 99, 99, 99, 31, 7, 1, 99, 99, 99, 99, 1, 5, 7, 48];
        for seed in 1..20usize {
            let mut u = [0u8; 155];
            for (i, b) in u.iter_mut().enumerate() {
                let max = if i < 126 { OP[i % 21] } else if i < 145 { GLOBAL[i - 126] } else { 94 };
                *b = ((i * 37 + seed * 11) % (usize::from(max) + 1)) as u8 + if i >= 145 { 32 } else { 0 };
            }
            assert_eq!(unpack(&pack(&u)), u, "seed {seed}");
        }
    }

    #[test]
    fn presets_survive_a_bank_round_trip() {
        let presets = presets();
        let voices: Vec<Dx7Voice> =
            presets.iter().map(|p| Dx7Voice { name: p.name.to_uppercase(), values: p.values.clone() }).collect();
        let file = bank(&voices);
        assert_eq!(file.len(), BANK_LEN);
        let back = parse(&file).unwrap();
        assert_eq!(back.len(), 32);
        for (p, v) in presets.iter().zip(&back) {
            assert_eq!(v.name, p.name.to_uppercase().chars().take(10).collect::<String>().trim_end());
            let want = Dx7Voice::from_preset(p);
            for &(id, value) in &v.values {
                assert_eq!(value, want.value(id), "{}: param {id}", p.name);
            }
        }
        assert_eq!(back[31].name, "INIT VOICE");
    }

    #[test]
    fn a_single_voice_parses() {
        let p = &presets()[1];
        let u = to_unpacked(&Dx7Voice::from_preset(p));
        let mut file = vec![0xF0, 0x43, 0x00, 0x00, 0x01, 0x1B];
        file.extend_from_slice(&u);
        file.push(checksum(&u));
        file.push(0xF7);
        let v = parse(&file).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Glass Keys");
    }

    #[test]
    fn bad_files_are_refused() {
        assert!(parse(&[]).is_err());
        assert!(parse(&[0xF0, 0x41, 0, 9, 0x20, 0]).is_err());
        let mut file = bank(&[]);
        file[100] ^= 1;
        assert!(parse(&file).unwrap_err().contains("checksum"));
        let short = &bank(&[])[..4000];
        assert!(parse(short).is_err());
    }
}

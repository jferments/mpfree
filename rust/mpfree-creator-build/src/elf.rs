//! The checks each program gets before it goes into the creator, read from
//! its ELF headers: a 32-bit ARM program for the hard-float EABI (the MPC
//! One's), linked statically or against the MPC's C library, and in that case
//! asking for glibc 2.39 at most.

/// What the checks read from one program.
#[derive(Debug, PartialEq)]
pub struct Arm {
    /// The program interpreter it asks for, if it is linked dynamically.
    pub interpreter: Option<String>,
    /// The newest `GLIBC_2.N` it asks for (N), if any.
    pub newest_glibc: Option<u32>,
}

const EM_ARM: u16 = 40;
const EF_ARM_EABI_MASK: u32 = 0xff00_0000;
const EF_ARM_EABI_VER5: u32 = 0x0500_0000;
const EF_ARM_ABI_FLOAT_HARD: u32 = 0x0000_0400;
const PT_INTERP: u32 = 3;
const SHT_GNU_VERNEED: u32 = 0x6fff_fffe;

fn u16_at(d: &[u8], at: usize) -> Result<u16, String> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| "cut short".to_string())
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "cut short".to_string())
}

fn c_str(d: &[u8], at: usize) -> Result<&str, String> {
    let rest = d.get(at..).ok_or("a name past the end")?;
    let end = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or("a name with no end")?;
    std::str::from_utf8(&rest[..end]).map_err(|_| "a name that isn't text".to_string())
}

/// Reads `data` as an ARM hard-float EABI 5 program, or says why it isn't one.
pub fn read(data: &[u8]) -> Result<Arm, String> {
    if data.get(..4) != Some(b"\x7fELF") {
        return Err("not an ELF program".into());
    }
    if data.get(4) != Some(&1) || data.get(5) != Some(&1) {
        return Err("not a 32-bit little-endian program".into());
    }
    if u16_at(data, 18)? != EM_ARM {
        return Err("not an ARM program".into());
    }
    let flags = u32_at(data, 36)?;
    if flags & EF_ARM_EABI_MASK != EF_ARM_EABI_VER5 || flags & EF_ARM_ABI_FLOAT_HARD == 0 {
        return Err(format!(
            "not for the hard-float EABI 5 (flags {flags:#x}), which the MPC runs"
        ));
    }
    let phoff = u32_at(data, 28)? as usize;
    let phentsize = u16_at(data, 42)? as usize;
    let phnum = u16_at(data, 44)? as usize;
    let mut interpreter = None;
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if u32_at(data, ph)? == PT_INTERP {
            let off = u32_at(data, ph + 4)? as usize;
            interpreter = Some(c_str(data, off)?.to_string());
        }
    }
    let shoff = u32_at(data, 32)? as usize;
    let shentsize = u16_at(data, 46)? as usize;
    let shnum = u16_at(data, 48)? as usize;
    let mut newest_glibc = None;
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        if u32_at(data, sh + 4)? != SHT_GNU_VERNEED {
            continue;
        }
        let off = u32_at(data, sh + 16)? as usize;
        let count = u32_at(data, sh + 28)? as usize;
        let strtab = shoff + u32_at(data, sh + 24)? as usize * shentsize;
        let names = u32_at(data, strtab + 16)? as usize;
        let mut need = off;
        for _ in 0..count {
            let aux_count = u16_at(data, need + 2)? as usize;
            let mut aux = need + u32_at(data, need + 8)? as usize;
            for _ in 0..aux_count {
                let name = c_str(data, names + u32_at(data, aux + 8)? as usize)?;
                if let Some(minor) = name
                    .strip_prefix("GLIBC_2.")
                    .and_then(|m| m.split('.').next()?.parse::<u32>().ok())
                {
                    newest_glibc = newest_glibc.max(Some(minor));
                }
                aux += u32_at(data, aux + 12)? as usize;
            }
            need += u32_at(data, need + 12)? as usize;
        }
    }
    Ok(Arm {
        interpreter,
        newest_glibc,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A small ELF file: its header, the program headers, and for a dynamic
    /// one an interpreter and the glibc versions it needs.
    pub fn program(flags: u32, interp: Option<&str>, glibc: &[&str]) -> Vec<u8> {
        let mut d = vec![0u8; 52];
        d[..4].copy_from_slice(b"\x7fELF");
        d[4] = 1;
        d[5] = 1;
        d[6] = 1;
        d[18..20].copy_from_slice(&EM_ARM.to_le_bytes());
        d[36..40].copy_from_slice(&flags.to_le_bytes());
        // One program header (PT_INTERP or PT_LOAD) at 52.
        d[28..32].copy_from_slice(&52u32.to_le_bytes());
        d[42..44].copy_from_slice(&32u16.to_le_bytes());
        d[44..46].copy_from_slice(&1u16.to_le_bytes());
        d.resize(84, 0);
        let interp_at = d.len() as u32;
        if let Some(i) = interp {
            d[52..56].copy_from_slice(&PT_INTERP.to_le_bytes());
            d[56..60].copy_from_slice(&interp_at.to_le_bytes());
            d.extend(i.bytes());
            d.push(0);
        } else {
            d[52..56].copy_from_slice(&1u32.to_le_bytes());
        }
        // The string table, then the version needs: one file, one entry per version.
        let strtab_at = d.len();
        let mut strs = vec![0u8];
        let file_name = strs.len() as u32;
        strs.extend(b"libc.so.6\0");
        let mut names = Vec::new();
        for g in glibc {
            names.push(strs.len() as u32);
            strs.extend(g.bytes());
            strs.push(0);
        }
        d.extend(&strs);
        while !d.len().is_multiple_of(4) {
            d.push(0);
        }
        let verneed_at = d.len();
        let mut v = Vec::new();
        v.extend(1u16.to_le_bytes());
        v.extend((glibc.len() as u16).to_le_bytes());
        v.extend(file_name.to_le_bytes());
        v.extend(16u32.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        for (i, n) in names.iter().enumerate() {
            v.extend(0u32.to_le_bytes());
            v.extend(0u16.to_le_bytes());
            v.extend((i as u16 + 2).to_le_bytes());
            v.extend(n.to_le_bytes());
            let next: u32 = if i + 1 < names.len() { 16 } else { 0 };
            v.extend(next.to_le_bytes());
        }
        d.extend(&v);
        // Section headers: null, the string table, the version needs.
        let shoff = d.len() as u32;
        d.extend([0u8; 40]);
        let mut sh = |kind: u32, off: usize, size: usize, link: u32, info: u32| {
            let mut s = [0u8; 40];
            s[4..8].copy_from_slice(&kind.to_le_bytes());
            s[16..20].copy_from_slice(&(off as u32).to_le_bytes());
            s[20..24].copy_from_slice(&(size as u32).to_le_bytes());
            s[24..28].copy_from_slice(&link.to_le_bytes());
            s[28..32].copy_from_slice(&info.to_le_bytes());
            d.extend(s);
        };
        sh(3, strtab_at, strs.len(), 0, 0);
        let needs = if glibc.is_empty() { 0 } else { 1 };
        sh(SHT_GNU_VERNEED, verneed_at, v.len(), 1, needs);
        d[32..36].copy_from_slice(&shoff.to_le_bytes());
        d[46..48].copy_from_slice(&40u16.to_le_bytes());
        d[48..50].copy_from_slice(&3u16.to_le_bytes());
        d
    }

    pub const HARD: u32 = EF_ARM_EABI_VER5 | EF_ARM_ABI_FLOAT_HARD;

    #[test]
    fn a_static_program_has_no_interpreter_and_no_glibc_versions() {
        let r = read(&program(HARD, None, &[])).unwrap();
        assert_eq!(
            r,
            Arm {
                interpreter: None,
                newest_glibc: None
            }
        );
    }

    #[test]
    fn a_dynamic_program_names_its_interpreter_and_the_newest_glibc_it_needs() {
        let p = program(
            HARD,
            Some("/lib/ld-linux-armhf.so.3"),
            &["GLIBC_2.4", "GLIBC_2.34", "GLIBC_2.28", "GCC_3.5"],
        );
        let r = read(&p).unwrap();
        assert_eq!(r.interpreter.as_deref(), Some("/lib/ld-linux-armhf.so.3"));
        assert_eq!(r.newest_glibc, Some(34));
    }

    #[test]
    fn other_programs_are_refused_with_the_reason() {
        assert!(read(b"#!/bin/sh\n").unwrap_err().contains("not an ELF"));
        let soft = program(EF_ARM_EABI_VER5, None, &[]);
        assert!(read(&soft).unwrap_err().contains("hard-float"));
        let mut x86 = program(HARD, None, &[]);
        x86[18] = 3;
        assert!(read(&x86).unwrap_err().contains("not an ARM"));
        let mut wide = program(HARD, None, &[]);
        wide[4] = 2;
        assert!(read(&wide).unwrap_err().contains("32-bit"));
        let short = program(HARD, Some("/lib/ld-linux-armhf.so.3"), &["GLIBC_2.34"]);
        assert!(read(&short[..60]).is_err());
    }
}

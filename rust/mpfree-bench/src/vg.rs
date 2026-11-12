//! Valgrind client requests, for counting each case's instructions under
//! callgrind (`--count`, run by `tools/ci/instr_gate.py`).
//!
//! A client request is a fixed run of instructions that does nothing on a
//! real CPU (four rotates of a register by 128 bits in all, then an
//! exchange of a register with itself) and that Valgrind recognises as a
//! call into the tool. So the same binary runs normally outside Valgrind,
//! where every request returns its default. The sequence and the request
//! numbers are Valgrind's public ABI (valgrind.h and callgrind.h); only
//! x86-64 is written here, since CI counts on x86-64. Elsewhere the
//! requests do nothing.

#![allow(unsafe_code)]

use std::ffi::CString;

/// `VG_USERREQ_TOOL_BASE('C', 'T')`: callgrind's requests.
const CALLGRIND: u64 = ((b'C' as u64) << 24) | ((b'T' as u64) << 16);
const ZERO_STATS: u64 = CALLGRIND + 1;
const DUMP_STATS_AT: u64 = CALLGRIND + 3;
/// `VG_USERREQ__RUNNING_ON_VALGRIND`.
const RUNNING_ON_VALGRIND: u64 = 0x1001;

#[cfg(target_arch = "x86_64")]
fn request(default: u64, req: u64, arg1: u64) -> u64 {
    let args: [u64; 6] = [req, arg1, 0, 0, 0, 0];
    let mut result = default;
    // SAFETY: outside Valgrind the sequence leaves every register as it was
    // (rdi is rotated by 3 + 13 + 61 + 51 = 128 bits; rbx is exchanged with
    // itself) and only rdx, declared here, carries the default back. Under
    // Valgrind the tool reads the six words at rax (alive across the asm) and
    // writes its answer in rdx. Flags change, so no preserves_flags.
    unsafe {
        std::arch::asm!(
            "rol rdi, 3",
            "rol rdi, 13",
            "rol rdi, 61",
            "rol rdi, 51",
            "xchg rbx, rbx",
            in("rax") args.as_ptr(),
            inout("rdx") result,
            options(nostack),
        );
    }
    result
}

#[cfg(not(target_arch = "x86_64"))]
fn request(default: u64, _req: u64, _arg1: u64) -> u64 {
    default
}

/// How many Valgrinds this program runs under: 0 on a real CPU.
pub fn running() -> u64 {
    request(0, RUNNING_ON_VALGRIND, 0)
}

/// Callgrind: forget every count so far.
pub fn zero_stats() {
    request(0, ZERO_STATS, 0);
}

/// Callgrind: write the counts since the last zero or dump to a new file
/// whose `desc: Trigger:` line names `why`, and start again from zero.
pub fn dump_stats_at(why: &str) {
    // A name with a NUL in it would be cut there; ours have none.
    let text = CString::new(why.replace('\0', " ")).unwrap_or_default();
    request(0, DUMP_STATS_AT, text.as_ptr() as u64);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_valgrind_every_request_is_harmless() {
        // Under `valgrind` this test is meaningless; elsewhere it must be 0.
        if running() == 0 {
            zero_stats();
            dump_stats_at("nothing to see");
            assert_eq!(request(42, RUNNING_ON_VALGRIND, 0), 42);
        }
    }
}

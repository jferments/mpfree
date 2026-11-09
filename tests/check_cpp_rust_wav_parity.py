#!/usr/bin/env python3
"""Compare independent C++ and Rust desktop WAV renderers.

No MPC, ALSA, MIDI, USB or network device access. Both processes write only
temporary host WAV files. Floating-point sin/exp may differ between libm
implementations, so compare timing and bounded audio error, not byte equality.
"""
import argparse
from array import array
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile

FMT = struct.Struct("<HHIIHH")
EXPERIMENTS = [
    (4.0, 120.0, 176400, 8),
    (7.0, 109.0, 308700, 13),
    (2.5, 137.2, 110250, 6),
]


def run_renderer(binary: Path, output: Path, seconds: float, bpm: float, frames: int, beats: int):
    proc = subprocess.run(
        [str(binary), str(output), str(seconds), str(bpm)],
        capture_output=True, text=True, check=True, timeout=30,
    )
    match = re.search(r"(\d+) frames; (\d+) beat triggers", proc.stdout)
    if match is None:
        raise AssertionError(f"{binary}: no transport summary: {proc.stdout!r}")
    found = (int(match[1]), int(match[2]))
    if found != (frames, beats):
        raise AssertionError(f"{binary}: got {found}, expected {(frames, beats)}")
    data = output.read_bytes()
    if len(data) != 44 + frames * 8:
        raise AssertionError(f"{binary}: unexpected WAV size: {len(data)}")
    if data[:4] != b"RIFF" or data[8:16] != b"WAVEfmt " or data[36:40] != b"data":
        raise AssertionError(f"{binary}: invalid WAV header")
    if struct.unpack_from("<I", data, 4)[0] != 36 + frames * 8:
        raise AssertionError(f"{binary}: RIFF size incorrect")
    if FMT.unpack_from(data, 20) != (1, 2, 44100, 352800, 8, 32):
        raise AssertionError(f"{binary}: wrong PCM format")
    if struct.unpack_from("<I", data, 40)[0] != frames * 8:
        raise AssertionError(f"{binary}: wrong PCM data length")
    # Both output files are 2ch interleaved signed little-endian PCM.
    if sys.byteorder != "little":
        raise RuntimeError("Parity checker requires a little-endian host")
    pcm = array("i")
    pcm.frombytes(data[44:])
    if pcm.itemsize != 4 or len(pcm) != frames * 2:
        raise AssertionError("expected 4-byte signed PCM integers")
    if pcm[0::2] != pcm[1::2]:
        raise AssertionError(f"{binary}: channels should be identical")
    if not any(pcm):
        raise AssertionError(f"{binary}: rendered silence")
    return pcm


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cpp", type=Path, required=True)
    ap.add_argument("--rust", type=Path, required=True)
    args = ap.parse_args()
    cpp = args.cpp.resolve(strict=True)
    rust = args.rust.resolve(strict=True)
    for seconds, bpm, frames, beats in EXPERIMENTS:
        with tempfile.TemporaryDirectory(prefix="mpfree-parity-") as temp:
            cpp_pcm = run_renderer(cpp, Path(temp, "cpp.wav"), seconds, bpm, frames, beats)
            rust_pcm = run_renderer(rust, Path(temp, "rust.wav"), seconds, bpm, frames, beats)
            # Small libm/round-to-float differences are fine; a mistimed click
            # or wrong sample encoding produces errors vastly beyond this bound.
            largest = max(abs(a - b) for a, b in zip(cpp_pcm, rust_pcm))
            max_allowed = 16384  # ~7.63e-6 full-scale S32; NOT byte equality
            if largest > max_allowed:
                raise AssertionError(
                    f"bpm={bpm}: cross-language max PCM difference {largest} "
                    f"exceeds {max_allowed}; check beat positions, rendering and conversion"
                )
            print(
                f"PASS bpm={bpm}, seconds={seconds}: frames={frames}, beats={beats}, "
                f"max abs S32 difference={largest} / {max_allowed} allowed"
            )
    print("PASS all C++ vs Rust sample-frame, WAV geometry and bounded PCM comparisons")


if __name__ == "__main__":
    main()

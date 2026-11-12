#!/usr/bin/env python3
"""Makes the FLAC files tests/files.rs decodes, with FFmpeg's encoder.

Each file holds the same test signal (`sample` below, whole numbers only,
which tests/files.rs works out the same way), encoded with different
settings so that every kind of subframe, stereo coding and sample size an
encoder writes turns up. Run from this folder: python3 make.py
"""
import os
import struct
import subprocess
import tempfile

N = 4800


def hash32(x):
    x &= 0xFFFFFFFF
    x ^= x >> 16
    x = (x * 0x7FEB352D) & 0xFFFFFFFF
    x ^= x >> 15
    x = (x * 0x846CA68B) & 0xFFFFFFFF
    x ^= x >> 16
    return x


def noise(n, ch, amp):
    return hash32(n * 2 + ch + 1) % (2 * amp + 1) - amp


def tri(n, period, amp):
    p = n % period
    half = period // 2
    v = p if p < half else period - p
    return (v * 2 * amp) // half - amp


def sample(n, ch, bits):
    full = (1 << (bits - 1)) - 1
    seg = n * 8 // N
    if seg == 0:
        v = 0
    elif seg == 1:
        v = tri(n, 97 + 13 * ch, full // 2)
    elif seg == 2:
        v = tri(n, 211, full // 3) + noise(n, ch, full // 64)
    elif seg == 3:
        v = noise(n, ch, full)
    elif seg == 4:
        p = n % 150 - 75
        v = (full * 2 * (5625 - p * p)) // 5625 - full
    elif seg == 5:
        v = full if (n // 3) % 2 == 0 else -full - 1
    elif seg == 6:
        v = full // 4 if (n // 40) % 2 == 0 else -(full // 4)
    else:
        v = noise(n, ch, 3)
    if ch == 1 and seg not in (3, 5):
        v = ((v * 3) >> 2) + noise(n, 7, 2)
    return v


def wav(path, channels, bits, shift=0):
    width = (bits + 7) // 8
    data = bytearray()
    for n in range(N):
        for ch in range(channels):
            v = sample(n, ch, bits - shift) << shift
            data += (v & ((1 << (8 * width)) - 1)).to_bytes(width, "little")
    fmt = struct.pack("<HHIIHH", 1, channels, 44100, 44100 * channels * width,
                      channels * width, bits)
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(data)) + b"WAVE")
        f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt)
        f.write(b"data" + struct.pack("<I", len(data)) + data)


# name: (channels, bits, wasted bits, FFmpeg options)
FILES = {
    "s16-stereo-auto": (2, 16, 0, ["-frame_size", "1152"]),
    "s16-stereo-left-fixed": (2, 16, 0, ["-ch_mode", "left_side", "-lpc_type", "fixed", "-frame_size", "576"]),
    "s16-stereo-right-lpc32": (2, 16, 0, ["-ch_mode", "right_side", "-compression_level", "12",
                                          "-min_prediction_order", "32", "-max_prediction_order", "32",
                                          "-lpc_type", "cholesky", "-exact_rice_parameters", "1"]),
    "s16-mono-verbatim": (1, 16, 0, ["-lpc_type", "none", "-frame_size", "4608"]),
    "s24-stereo-mid": (2, 24, 0, ["-ch_mode", "mid_side", "-compression_level", "8", "-frame_size", "2048"]),
    "s24-mono-wasted": (1, 24, 8, ["-frame_size", "1024"]),
    "s32-stereo": (2, 32, 0, ["-strict", "experimental", "-frame_size", "4096"]),
}

here = os.path.dirname(os.path.abspath(__file__))
with tempfile.TemporaryDirectory() as tmp:
    for name, (channels, bits, shift, opts) in FILES.items():
        src = os.path.join(tmp, name + ".wav")
        wav(src, channels, bits, shift)
        out = os.path.join(here, name + ".flac")
        fmt = "s16" if bits == 16 else "s32"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", src, "-map_metadata", "-1",
                        "-fflags", "+bitexact", "-flags:a", "+bitexact", "-c:a", "flac",
                        "-sample_fmt", fmt, *opts, out], check=True)
        print(name, os.path.getsize(out))

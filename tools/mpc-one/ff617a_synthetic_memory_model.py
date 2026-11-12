"""FF/61/7A synthetic in-memory response validation model (NOT vendor code).

No USB, networking, disk access, loader execution or real device transport.
Only observations backed by the owner's SHA-pinned Ghidra audit are modeled.
A matching synthetic frame is NOT proof the real backend would accept it.
"""
from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
import struct

MIN_HEADER = 24
MAX_SYNTHETIC_FRAME = 1024 * 1024
OBSERVED_COMPARED_IDS = frozenset((
    0x5AA5FE01, 0x5AA5FD02, 0x5AA5FC03,
    0x5AA5FB04, 0x5AA5FA05, 0x5AA5F906,
))


class Result(Enum):
    MALFORMED = "malformed"
    UNKNOWN_IDENTIFIER = "unknown_identifier"
    NONMATCHING_RESPONSE = "nonmatching_response"
    INCOMPLETE = "incomplete"
    ERROR_REPORTED = "error_flag_seen_status_not_interpreted"
    UNKNOWN_FLAGS = "unknown_flags"
    MATCHED_UNVERIFIED = "matched_synthetic_conditions_not_vendor_success"
    EXHAUSTED = "no_conclusive_synthetic_response"


@dataclass(frozen=True)
class Observation:
    result: Result
    identifier: int | None = None
    correlation: int | None = None
    flags: int | None = None
    declared_length: int | None = None
    error_word: int | None = None


def classify_response(data: bytes, expected_identifier: int,
                      expected_correlation: int) -> Observation:
    """Classify *in-memory synthetic bytes*, conservatively; never transmit."""
    if not isinstance(data, bytes) or len(data) < MIN_HEADER or len(data) > MAX_SYNTHETIC_FRAME:
        return Observation(Result.MALFORMED)
    identifier, correlation, flags, declared = (
        struct.unpack_from("<I", data, 4)[0],
        struct.unpack_from("<I", data, 0x10)[0],
        struct.unpack_from("<H", data, 0x14)[0],
        struct.unpack_from("<H", data, 0x16)[0],
    )
    base = dict(identifier=identifier, correlation=correlation,
                flags=flags, declared_length=declared)
    if declared < MIN_HEADER or declared > len(data):
        return Observation(Result.MALFORMED, **base)
    if identifier not in OBSERVED_COMPARED_IDS:
        return Observation(Result.UNKNOWN_IDENTIFIER, **base)
    if expected_identifier not in OBSERVED_COMPARED_IDS:
        return Observation(Result.UNKNOWN_IDENTIFIER, **base)
    if identifier != expected_identifier or correlation != expected_correlation:
        return Observation(Result.NONMATCHING_RESPONSE, **base)
    if flags & ~0x7:
        return Observation(Result.UNKNOWN_FLAGS, **base)
    if not flags & 1:
        return Observation(Result.INCOMPLETE, **base)
    if flags & 2:
        if declared < 0x1c:
            return Observation(Result.MALFORMED, **base)
        word = struct.unpack_from("<I", data, 0x18)[0]
        return Observation(Result.ERROR_REPORTED, error_word=word, **base)
    if flags & 4:
        return Observation(Result.UNKNOWN_FLAGS, **base)
    return Observation(Result.MATCHED_UNVERIFIED, **base)


def replay_synthetic_responses(frames: list[bytes], identifier: int,
                               correlation: int, max_frames: int = 32) -> Observation:
    """Bounded replay. Does not generate or send a response to a real device."""
    if max_frames < 1 or max_frames > 256:
        raise ValueError("max_frames must be in [1,256]")
    for frame in frames[:max_frames]:
        observed = classify_response(frame, identifier, correlation)
        if observed.result in (Result.MATCHED_UNVERIFIED, Result.ERROR_REPORTED,
                               Result.MALFORMED, Result.UNKNOWN_FLAGS,
                               Result.UNKNOWN_IDENTIFIER):
            return observed
    return Observation(Result.EXHAUSTED)


def synthetic_fixture(identifier: int, correlation: int, flags: int,
                      payload: bytes = b"") -> bytes:
    """Pure bytes fixture; never treat as a valid packet for real hardware."""
    if len(payload) + MIN_HEADER > 0xffff:
        raise ValueError("synthetic payload exceeds 16-bit header length")
    buf = bytearray(MIN_HEADER)
    struct.pack_into("<I", buf, 4, identifier)
    struct.pack_into("<I", buf, 0x10, correlation)
    struct.pack_into("<H", buf, 0x14, flags)
    struct.pack_into("<H", buf, 0x16, MIN_HEADER + len(payload))
    return bytes(buf) + payload

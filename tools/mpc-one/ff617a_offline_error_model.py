"""Offline FF/61/7A response/error state approximation from owner Ghidra evidence.

No vendor firmware execution. No USB, I/O, network, device nodes or storage writes.
This is an independent *synthetic* interpreter of observed control-flow branches.
It is not an authenticated protocol implementation or physical-device emulator.
"""
from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
import struct

TRACE_LABEL_IDENTIFIERS = frozenset((
    0x5AA5FE01, 0x5AA5FD02, 0x5AA5FC03,
    0x5AA5FB04, 0x5AA5FA05, 0x5AA5F906,
))
MIN_HEADER_BYTES = 24
MAX_FIXTURE_BYTES = 65535


class Decision(Enum):
    CONTINUE = "continue_receiving"
    EXIT_STATUS = "returned_integer_status"
    INCONCLUSIVE = "model_cannot_establish_behavior"
    LIMIT = "synthetic_replay_limit"


@dataclass(frozen=True)
class TransferFixture:
    """Simulated USB completion: no link to Linux usbdevfs or USB packets."""
    status: int
    data: bytes = b""
    received_length: int | None = None


@dataclass(frozen=True)
class Observation:
    decision: Decision
    reason: str
    status: int | None = None
    diagnostic_identifier_known: bool | None = None
    fields_out: tuple[int, int] | None = None


def classify(
    item: TransferFixture, request_identifier: int, request_correlation: int,
    request_mode_bit0: int = 0, output_available: bool = True,
) -> Observation:
    """Model one receive-loop iteration; derived, not vendor-executed.

    The six 0x5aa5... IDs select trace names in observed Ghidra code.
    The actual response match is against the *outstanding request*, not
    membership in this set. A matching unknown ID is deliberately not
    mislabeled as vendor rejection.
    """
    if not isinstance(item, TransferFixture) or not isinstance(item.status, int):
        raise TypeError("Synthetic TransferFixture with integer status required")
    if item.status != 0:
        return Observation(Decision.EXIT_STATUS, "transport_error_propagated", item.status)
    data = item.data
    if not isinstance(data, bytes) or len(data) > MAX_FIXTURE_BYTES:
        return Observation(Decision.INCONCLUSIVE, "invalid_synthetic_buffer")
    received = len(data) if item.received_length is None else item.received_length
    if not isinstance(received, int) or received < 0 or received > len(data):
        return Observation(Decision.INCONCLUSIVE, "invalid_synthetic_received_length")
    if received < MIN_HEADER_BYTES:
        return Observation(Decision.CONTINUE, "received_less_than_24_bytes")
    identifier = struct.unpack_from("<I", data, 4)[0]
    seq = struct.unpack_from("<I", data, 0x10)[0]
    flags = struct.unpack_from("<H", data, 0x14)[0]
    declared = struct.unpack_from("<H", data, 0x16)[0]
    known = identifier in TRACE_LABEL_IDENTIFIERS
    if received < declared:
        return Observation(Decision.CONTINUE, "received_shorter_than_declared", diagnostic_identifier_known=known)
    if identifier != request_identifier or seq != request_correlation:
        return Observation(Decision.CONTINUE, "request_response_mismatch", diagnostic_identifier_known=known)
    if not flags & 1:
        return Observation(Decision.CONTINUE, "completion_bit0_unset", diagnostic_identifier_known=known)
    # High bits are not explicitly rejected in the sampled vendor branch;
    # conservative modeling deliberately avoids claiming how they behave.
    if flags & ~0x7:
        return Observation(Decision.INCONCLUSIVE, "unmodeled_high_flag_bits", diagnostic_identifier_known=known)
    if flags & 2:
        if declared < 28:
            return Observation(Decision.EXIT_STATUS, "error_frame_short_status_word", -9, known)
        status_word = struct.unpack_from("<I", data, 0x18)[0]
        if status_word == 1:
            return Observation(Decision.EXIT_STATUS, "device_error_word_1", -11, known)
        return Observation(Decision.EXIT_STATUS, "other_device_error_word", -9, known)
    # ARM UBFX/eor sequence at 0x1a5c8: mode bit0 equals frame flag bit2.
    if (request_mode_bit0 & 1) != ((flags >> 2) & 1):
        return Observation(Decision.EXIT_STATUS, "response_mode_bit_mismatch", -9, known)
    fields = struct.unpack_from("<II", data, 8)
    if fields != (0, 0) and not output_available:
        return Observation(Decision.EXIT_STATUS, "nonzero_fields_without_output_buffer", -9, known)
    return Observation(Decision.EXIT_STATUS, "matching_frame_unverified_vendor_success", 0,
                       known, fields if output_available else None)


def replay(
    events: list[TransferFixture], request_identifier: int, request_correlation: int,
    request_mode_bit0: int = 0, output_available: bool = True,
    max_events: int = 32,
) -> tuple[Observation, tuple[Observation, ...]]:
    """Bounded synthetic event replay; no real timeouts or USB transfers."""
    if not 1 <= max_events <= 256:
        raise ValueError("max_events must be 1..256")
    history = []
    for item in events[:max_events]:
        outcome = classify(item, request_identifier, request_correlation,
                           request_mode_bit0, output_available)
        history.append(outcome)
        if outcome.decision is not Decision.CONTINUE:
            return outcome, tuple(history)
    return Observation(Decision.LIMIT, "event_limit_no_status"), tuple(history)


def synthetic_frame(identifier: int, correlation: int, flags: int = 1,
                    fields: tuple[int, int] = (0, 0), payload: bytes = b"") -> bytes:
    """Construct test-only bytes (NOT a valid device command or USB frame)."""
    if not isinstance(payload, bytes) or len(payload) + MIN_HEADER_BYTES > MAX_FIXTURE_BYTES:
        raise ValueError("Invalid synthetic payload")
    header = bytearray(MIN_HEADER_BYTES)
    struct.pack_into("<I", header, 4, identifier)
    struct.pack_into("<II", header, 8, *fields)
    struct.pack_into("<I", header, 0x10, correlation)
    struct.pack_into("<H", header, 0x14, flags)
    struct.pack_into("<H", header, 0x16, MIN_HEADER_BYTES + len(payload))
    return bytes(header) + payload

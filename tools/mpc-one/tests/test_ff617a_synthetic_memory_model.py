"""Synthetic-only tests: no actual vendor code, device, network, or block I/O."""
import importlib.util
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "tools/mpc-one/ff617a_synthetic_memory_model.py"
spec = importlib.util.spec_from_file_location("mpfree_ff617a_memory_tests", SOURCE)
MODEL = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = MODEL
spec.loader.exec_module(MODEL)

ID = 0x5AA5FE01
SEQ = 0x1337


def fixture(flags=1, identifier=ID, seq=SEQ, payload=b""):
    return MODEL.synthetic_fixture(identifier, seq, flags, payload)


class SyntheticFF617AMemoryTests(unittest.TestCase):
    def test_matching_synthetic_frame_is_not_vendor_success(self):
        self.assertEqual(MODEL.classify_response(fixture(), ID, SEQ).result,
                         MODEL.Result.MATCHED_UNVERIFIED)

    def test_short_frame_rejected(self):
        self.assertEqual(MODEL.classify_response(fixture()[:23], ID, SEQ).result,
                         MODEL.Result.MALFORMED)

    def test_truncated_declared_length(self):
        data = fixture(payload=b"ab")[:-1]
        self.assertEqual(MODEL.classify_response(data, ID, SEQ).result,
                         MODEL.Result.MALFORMED)

    def test_wrong_correlation_rejected(self):
        self.assertEqual(MODEL.classify_response(fixture(seq=5), ID, SEQ).result,
                         MODEL.Result.NONMATCHING_RESPONSE)

    def test_constructed_outgoing_magic_is_not_a_compared_id(self):
        self.assertEqual(MODEL.classify_response(
            fixture(identifier=0xA55A0101), ID, SEQ).result,
            MODEL.Result.UNKNOWN_IDENTIFIER)

    def test_range_bound_is_not_an_id(self):
        self.assertEqual(MODEL.classify_response(
            fixture(identifier=0x5AA5FC04), ID, SEQ).result,
            MODEL.Result.UNKNOWN_IDENTIFIER)

    def test_no_completion_bit(self):
        self.assertEqual(MODEL.classify_response(fixture(flags=0), ID, SEQ).result,
                         MODEL.Result.INCOMPLETE)

    def test_error_requires_status_word(self):
        self.assertEqual(MODEL.classify_response(fixture(flags=3), ID, SEQ).result,
                         MODEL.Result.MALFORMED)

    def test_error_status_not_interpreted_as_success(self):
        result = MODEL.classify_response(
            fixture(flags=3, payload=(27).to_bytes(4, "little")), ID, SEQ)
        self.assertEqual((result.result, result.error_word),
                         (MODEL.Result.ERROR_REPORTED, 27))

    def test_unresolved_flag_bit2(self):
        self.assertEqual(MODEL.classify_response(fixture(flags=5), ID, SEQ).result,
                         MODEL.Result.UNKNOWN_FLAGS)

    def test_unknown_high_flags(self):
        self.assertEqual(MODEL.classify_response(fixture(flags=0x81), ID, SEQ).result,
                         MODEL.Result.UNKNOWN_FLAGS)

    def test_bounded_synthetic_replay(self):
        self.assertEqual(MODEL.replay_synthetic_responses(
            [fixture(seq=1), fixture()], ID, SEQ).result,
            MODEL.Result.MATCHED_UNVERIFIED)

    def test_no_match_returns_exhausted(self):
        self.assertEqual(MODEL.replay_synthetic_responses(
            [fixture(seq=1)], ID, SEQ).result, MODEL.Result.EXHAUSTED)

    def test_oversize_input_rejected(self):
        self.assertEqual(MODEL.classify_response(
            b"Z" * (MODEL.MAX_SYNTHETIC_FRAME + 1), ID, SEQ).result,
            MODEL.Result.MALFORMED)


if __name__ == "__main__":
    unittest.main()

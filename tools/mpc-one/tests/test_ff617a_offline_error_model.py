"""Synthetic FF/61/7A status branch tests; no USB/device access."""
import importlib.util
from pathlib import Path
import sys
import unittest

P = Path(__file__).resolve().parents[1] / "ff617a_offline_error_model.py"
spec = importlib.util.spec_from_file_location("ff617a_error_model", P)
MODEL = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = MODEL
spec.loader.exec_module(MODEL)

ID = 0x5AA5FE01
SEQ = 0x1357


def event(flags=1, identifier=ID, correlation=SEQ, fields=(0, 0), payload=b"", received=None):
    return MODEL.TransferFixture(0, MODEL.synthetic_frame(identifier, correlation, flags, fields, payload), received)


class Tests(unittest.TestCase):
    def test_matching_synthetic_status_zero_is_not_vendor_execution(self):
        out = MODEL.classify(event(), ID, SEQ)
        self.assertEqual((out.decision, out.status), (MODEL.Decision.EXIT_STATUS, 0))
        self.assertIn("unverified", out.reason)

    def test_unknown_identifier_may_still_match_request(self):
        unknown = 0x12345678
        out = MODEL.classify(event(identifier=unknown), unknown, SEQ)
        self.assertEqual(out.status, 0)
        self.assertIs(out.diagnostic_identifier_known, False)

    def test_unknown_identifier_not_matching_request_repeats(self):
        out = MODEL.classify(event(identifier=0x12345678), ID, SEQ)
        self.assertEqual((out.decision, out.reason),
                         (MODEL.Decision.CONTINUE, "request_response_mismatch"))

    def test_two_outgoing_control_values_not_special_acceptance_rules(self):
        for identifier in (0xA55A0101, 0xA55A0202):
            with self.subTest(identifier=identifier):
                out = MODEL.classify(event(identifier=identifier), identifier, SEQ)
                self.assertEqual(out.status, 0)
                self.assertIs(out.diagnostic_identifier_known, False)

    def test_received_too_short_repeats(self):
        self.assertEqual(MODEL.classify(event(received=10), ID, SEQ).decision,
                         MODEL.Decision.CONTINUE)

    def test_declared_greater_than_received_repeats(self):
        self.assertEqual(MODEL.classify(event(payload=b"abcd", received=24), ID, SEQ).reason,
                         "received_shorter_than_declared")

    def test_correlation_mismatch_repeats(self):
        self.assertEqual(MODEL.classify(event(correlation=SEQ + 1), ID, SEQ).decision,
                         MODEL.Decision.CONTINUE)

    def test_completion_bit0_unset_repeats(self):
        self.assertEqual(MODEL.classify(event(flags=0), ID, SEQ).reason,
                         "completion_bit0_unset")

    def test_transport_error_propagates(self):
        for code in (-6, -9):
            with self.subTest(code=code):
                out = MODEL.classify(MODEL.TransferFixture(code), ID, SEQ)
                self.assertEqual((out.status, out.reason), (code, "transport_error_propagated"))

    def test_error_flag_with_word_one_returns_negative_eleven(self):
        out = MODEL.classify(event(flags=3, payload=(1).to_bytes(4, "little")), ID, SEQ)
        self.assertEqual((out.status, out.reason), (-11, "device_error_word_1"))

    def test_other_error_word_returns_negative_nine(self):
        out = MODEL.classify(event(flags=3, payload=(7).to_bytes(4, "little")), ID, SEQ)
        self.assertEqual(out.status, -9)

    def test_error_flag_without_status_word_returns_negative_nine(self):
        out = MODEL.classify(event(flags=3), ID, SEQ)
        self.assertEqual(out.status, -9)

    def test_flag_bit2_must_match_request_mode_bit(self):
        self.assertEqual(MODEL.classify(event(flags=5), ID, SEQ, request_mode_bit0=1).status, 0)
        self.assertEqual(MODEL.classify(event(flags=5), ID, SEQ, request_mode_bit0=0).status, -9)
        self.assertEqual(MODEL.classify(event(flags=1), ID, SEQ, request_mode_bit0=1).status, -9)

    def test_nonzero_data_needs_output_buffer(self):
        item = event(fields=(123, 0))
        self.assertEqual(MODEL.classify(item, ID, SEQ, output_available=False).status, -9)
        self.assertEqual(MODEL.classify(item, ID, SEQ, output_available=True).fields_out, (123, 0))

    def test_high_flags_remain_inconclusive(self):
        self.assertEqual(MODEL.classify(event(flags=0x81), ID, SEQ).decision,
                         MODEL.Decision.INCONCLUSIVE)

    def test_multiple_retries_then_status(self):
        out, hist = MODEL.replay([event(correlation=0), event(flags=0), event()], ID, SEQ)
        self.assertEqual([x.decision for x in hist],
                         [MODEL.Decision.CONTINUE, MODEL.Decision.CONTINUE,
                          MODEL.Decision.EXIT_STATUS])
        self.assertEqual(out.status, 0)

    def test_budget_exhaustion(self):
        out, hist = MODEL.replay([event(correlation=0)] * 3, ID, SEQ, max_events=2)
        self.assertEqual(out.decision, MODEL.Decision.LIMIT)
        self.assertEqual(len(hist), 2)


if __name__ == "__main__":
    unittest.main()

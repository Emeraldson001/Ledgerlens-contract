"""Tests for the interface-v3 compatibility shim (issue #1435).

Run: python3 -m unittest tests/test_ledgerlens_v3_shim.py
"""

import json
import os
import subprocess
import sys
import unittest

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
sys.path.insert(0, os.path.join(ROOT, "schemas", "compat"))

import ledgerlens_v3  # noqa: E402

V3_PAYLOAD = {
    "benford_flag": False,
    "confidence": 90,
    "ml_flag": True,
    "model_version": 2,
    "score": 40,
    "timestamp": 1_700_000_000,
}
CURRENT_PAYLOAD = dict(
    V3_PAYLOAD, benford_score=30, commitment=None, ml_score=50, network_score=10
)


class TestLedgerLensV3Shim(unittest.TestCase):
    def test_v3_payload_decodes_unchanged(self):
        score = ledgerlens_v3.RiskScore.from_payload(V3_PAYLOAD)
        self.assertEqual(score.score, 40)
        self.assertTrue(score.ml_flag)

    def test_current_payload_projects_onto_v3_view(self):
        score = ledgerlens_v3.RiskScore.from_payload(CURRENT_PAYLOAD)
        self.assertEqual(score, ledgerlens_v3.RiskScore.from_payload(V3_PAYLOAD))
        self.assertFalse(hasattr(score, "network_score"))

    def test_additive_fields_match_generated_schema(self):
        # The shim's view of "what was added since v3" must equal the
        # generated current schema minus the v3 field set — no drift.
        with open(os.path.join(ROOT, "schemas", "risk_score.schema.json"), encoding="utf-8") as f:
            current = set(json.load(f)["$defs"]["RiskScore"]["properties"])
        self.assertEqual(current, set(ledgerlens_v3.current_fields()))
        self.assertEqual(
            current - set(ledgerlens_v3.V3_FIELDS), set(ledgerlens_v3.ADDITIVE_FIELDS_SINCE_V3)
        )
        self.assertTrue(set(ledgerlens_v3.V3_FIELDS) <= current)

    def test_missing_v3_field_fails_closed(self):
        payload = dict(CURRENT_PAYLOAD)
        del payload["confidence"]
        with self.assertRaisesRegex(ValueError, "missing v3 fields"):
            ledgerlens_v3.RiskScore.from_payload(payload)

    def test_unknown_field_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "unrecognised fields"):
            ledgerlens_v3.RiskScore.from_payload(dict(CURRENT_PAYLOAD, future_field=1))

    def test_out_of_range_score_rejected(self):
        for bad in (101, -1, True, "40"):
            with self.assertRaises(ValueError):
                ledgerlens_v3.RiskScore.from_payload(dict(V3_PAYLOAD, score=bad))

    def test_legacy_consumer_walkthrough(self):
        script = os.path.join(ROOT, "examples", "legacy-v3", "validate.sh")
        result = subprocess.run([script], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

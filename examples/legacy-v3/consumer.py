# Deliberately old, unmodified v3-era integration (issue #1435).
#
# An off-chain risk desk that decodes `get_score` results and applies the v3
# documented decision: block when the score is at/above the threshold or the
# confidence is below the floor. Written against interface v3 and kept frozen
# so docs/migration-guide-v3.md can be validated against it.
#
#   python3 consumer.py <risk_score.json> [threshold] [min_confidence]

import json
import sys

from ledgerlens_v3 import RiskScore


def decide(payload, threshold=75, min_confidence=50):
    score = RiskScore.from_payload(payload)
    if score.score >= threshold or score.confidence < min_confidence:
        return "block"
    return "allow"


if __name__ == "__main__":
    with open(sys.argv[1], encoding="utf-8") as handle:
        payload = json.load(handle)
    args = [int(arg) for arg in sys.argv[2:4]]
    print(decide(payload, *args))

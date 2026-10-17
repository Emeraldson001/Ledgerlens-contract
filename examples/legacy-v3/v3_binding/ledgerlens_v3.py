# FROZEN v3-era binding (issue #1435 validation fixture). Do not update.
#
# This is the strict `RiskScore` decoder a v3 integrator would have written
# from the v3 interface spec: exactly the six v3 fields, nothing else.
# docs/migration-guide-v3.md walks examples/legacy-v3/consumer.py from this
# binding to the compatibility shim in schemas/compat/ledgerlens_v3.py.

from dataclasses import dataclass

V3_FIELDS = ("benford_flag", "confidence", "ml_flag", "model_version", "score", "timestamp")


@dataclass(frozen=True)
class RiskScore:
    benford_flag: bool
    confidence: int
    ml_flag: bool
    model_version: int
    score: int
    timestamp: int

    @classmethod
    def from_payload(cls, payload):
        if set(payload) != set(V3_FIELDS):
            raise ValueError(f"unexpected RiskScore fields: {sorted(payload)}")
        return cls(**payload)

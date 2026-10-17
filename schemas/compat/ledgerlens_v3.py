# Compatibility shim for integrations built against interface v3 (issue #1435).
#
# Drop-in replacement for the v3-era `ledgerlens_v3` RiskScore binding: same
# module name, same class, same `from_payload` API. It accepts both the v3
# six-field `RiskScore` and the current ten-field `RiskScore`, projecting the
# current shape onto the v3 view, so a v3 integration keeps working with zero
# code changes. See docs/migration-guide-v3.md.
#
# Stdlib only. The current field set is read from the generated
# `schemas/risk_score.schema.json` when present, so the shim cannot drift from
# the contract-spec source of truth.

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from typing import Any, Mapping

#: `RiskScore` fields at interface v3, in contract-spec (alphabetical) order.
V3_FIELDS = ("benford_flag", "confidence", "ml_flag", "model_version", "score", "timestamp")

#: Fields appended to `RiskScore` after v3. All are additive and ignorable for
#: a v3 integration (see docs/migration-guide-v3.md, change C1).
ADDITIVE_FIELDS_SINCE_V3 = ("benford_score", "commitment", "ml_score", "network_score")

_SCHEMA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "risk_score.schema.json")


def current_fields() -> tuple:
    """The current `RiskScore` field set, from the generated JSON Schema."""
    try:
        with open(_SCHEMA, encoding="utf-8") as handle:
            schema = json.load(handle)
        return tuple(schema["$defs"]["RiskScore"]["properties"])
    except (OSError, KeyError, ValueError):
        return tuple(sorted(V3_FIELDS + ADDITIVE_FIELDS_SINCE_V3))


@dataclass(frozen=True)
class RiskScore:
    """The v3 `RiskScore` view. Field set and order are frozen at v3."""

    benford_flag: bool
    confidence: int
    ml_flag: bool
    model_version: int
    score: int
    timestamp: int

    @classmethod
    def from_payload(cls, payload: Mapping[str, Any]) -> "RiskScore":
        keys = set(payload)
        missing = [name for name in V3_FIELDS if name not in keys]
        if missing:
            raise ValueError(f"RiskScore payload missing v3 fields: {missing}")
        unknown = sorted(keys - set(V3_FIELDS) - set(ADDITIVE_FIELDS_SINCE_V3))
        if unknown:
            # Unknown keys mean an interface change this shim was not built
            # for: fail closed instead of silently guessing.
            raise ValueError(f"RiskScore payload has unrecognised fields: {unknown}")
        for name in ("score", "confidence"):
            value = payload[name]
            if not isinstance(value, int) or isinstance(value, bool) or not 0 <= value <= 100:
                raise ValueError(f"RiskScore.{name} out of range [0, 100]: {value!r}")
        return cls(**{name: payload[name] for name in V3_FIELDS})

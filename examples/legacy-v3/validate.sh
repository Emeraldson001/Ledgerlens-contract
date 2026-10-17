#!/usr/bin/env sh
# Walks the unmodified v3 integration through docs/migration-guide-v3.md (#1435).
# Exits non-zero if the guide's claims no longer hold.
set -eu
DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$DIR/../.."
run() { PYTHONPATH="$1" python3 "$DIR/consumer.py" "$2" 2>&1 | tail -1; }

echo "1. v3 binding, v3 payload:       $(run "$DIR/v3_binding" "$DIR/score_v3.json")"
echo "2. v3 binding, current payload:  $(run "$DIR/v3_binding" "$DIR/score_current.json")"
echo "3. shim, v3 payload:             $(run "$ROOT/schemas/compat" "$DIR/score_v3.json")"
echo "4. shim, current payload:        $(run "$ROOT/schemas/compat" "$DIR/score_current.json")"

[ "$(run "$DIR/v3_binding" "$DIR/score_v3.json")" = allow ]
run "$DIR/v3_binding" "$DIR/score_current.json" | grep -q "unexpected RiskScore fields"
[ "$(run "$ROOT/schemas/compat" "$DIR/score_v3.json")" = allow ]
[ "$(run "$ROOT/schemas/compat" "$DIR/score_current.json")" = allow ]
echo "migration guide validated: unmodified v3 consumer works on current payloads via the shim"

#!/usr/bin/env sh
# Real read call using only libc, curl, and schemas/c/ledgerlens_score.h (issue #1438).
#
#   examples/c-abi/read.sh <contract C...> <function> [sym:X | u32:N | addr:G...]...
#
# Example (LedgerLens deployment):
#   RPC_URL=https://soroban-testnet.stellar.org examples/c-abi/read.sh "$CONTRACT_ID" get_version
set -eu

RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="${TMPDIR:-/tmp}/ll_read.$$"
trap 'rm -f "$BIN"' EXIT

cc -std=c99 -Wall -Wextra -pedantic -O2 -I"$DIR/../../schemas/c" "$DIR/ll_read.c" -o "$BIN"
TX="$("$BIN" encode "$@")"

RESPONSE="$(curl -sS -X POST "$RPC_URL" -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"simulateTransaction\",\"params\":{\"transaction\":\"$TX\"}}")"

RESULT="$(printf '%s' "$RESPONSE" | sed -n 's/.*"results":.*"xdr":"\([^"]*\)".*/\1/p')"
if [ -z "$RESULT" ]; then
  echo "simulateTransaction returned no result:" >&2
  printf '%s\n' "$RESPONSE" >&2
  exit 1
fi
"$BIN" decode "$RESULT"

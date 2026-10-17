# Example & Reference-Tool Integration Pattern Audit

Issue: #1437
Recurring: yes. This audit is a standing item in the
[release checklist](release-process.md#pre-release-checklist).

## The current recommended integration pattern

From [`docs/consumer-integration-decision-guide.md`](consumer-integration-decision-guide.md)
(§6, §8), [`docs/typed-client-examples.md`](typed-client-examples.md), and
[`docs/interface-versioning-policy.md` §5](interface-versioning-policy.md#5-programmatic-detection):

1. **Use the generated typed client** (`LedgerLensScoreContractClient`), not
   raw `invoke_contract` with hand-built argument vectors.
2. **Feature-detect with `supports_interface`**, never by comparing
   `get_version()` numbers.
3. **Prefer `query_risk_gate_with_confidence`** whenever signal quality
   matters. Probe `cgate` first, and if it is absent fall back to
   `query_risk_gate` with a *stricter* threshold.
4. **Treat the gate as infallible.** Branch on the boolean, with no `try_*`
   wrapper on the gate itself, and treat `false` (unknown, embargoed, paused,
   risky) as *block*.

## Findings

| Example / tool | Pattern before | Status | Action |
|---|---|---|---|
| `examples/amm_gate_example.rs` | Plain `query_risk_gate` only. No `cgate` probe, no confidence floor. Its own doc said it "demonstrates `query_risk_gate`". | **Superseded** (predates §6/§8 of the decision guide) | **Updated.** It now probes `supports_interface("cgate")`, calls `query_risk_gate_with_confidence` with `MIN_CONFIDENCE = 50` (decision guide §2.2 standard AMM), and falls back to `query_risk_gate` at `threshold − 10`. The `swap` signature is unchanged. Its three existing tests are unchanged and assert the same outcomes (their fixtures submit confidence 95 ≥ 50, and an unscored wallet still fails closed). |
| `examples/amm_gate.rs` | Plain `query_risk_gate`, typed client, infallible guard. | **Kept — documented exception** | Deliberately minimal. It is the canonical one-call guard referenced from `docs/interface-spec.md`, and it exists to teach "the gate cannot fail, so no `try_`/`?`". Adding the `cgate` probe would bury that single point. The confidence-aware version is `amm_gate_example.rs`, and the doc comment points readers there. |
| `examples/typed_client_examples.rs` | Typed client. `gate_flow` covers both `query_risk_gate` and `query_risk_gate_with_confidence`. | Current | None. |
| `examples/aggregator_gate_example.rs` | `ledgerlens-aggregator` typed client, before/after `get_last_shard_failure` snapshot. | Current *for the aggregator surface* | None. The aggregator exposes no confidence-gate variant (`query_risk_gate` + `supports_interface` only), so pattern 3 does not apply. |
| `examples/aggregator_shard_pause_example.rs` | Aggregator typed client plus degraded-shard handling. | Current *for the aggregator surface* | None (same reason). Pre-existing: this example does not currently compile in workspace clippy (noted in `docs/risk-score-schema-generation.md`). |
| `examples/oracle_adapter.rs` | Oracle-provider side: the oracle *implements* `get_price` for LedgerLens to call. | Not a gate consumer | None. It demonstrates the provider side of `register_oracle`. Its `invoke_contract` mention describes how LedgerLens calls the oracle, not how a consumer calls LedgerLens. |
| `examples/c-abi/` (new, #1438) | Raw RPC `simulateTransaction` with hand-encoded XDR. | **Documented exception** | Deliberately avoids every client library. That is the whole point: it targets environments with no generated client. |
| `examples/legacy-v3/` (new, #1435) | Frozen v3-era integration. | **Documented exception** | Must stay old. It is the validation fixture for the migration guide. |
| `tools/invocation-fuzzer`, `tools/reference-model/driver`, `tools/replay` | Call every entry point directly, including raw/`try_*` forms. | Not integration samples | None. These are test infrastructure that must exercise every path, including the ones integrators should avoid. |
| `tools/score-submitter`, `tools/ledgerlens-cli` | Service-side submission tooling. | Not gate consumers | None. The consumer gate pattern does not apply. |
| `contracts/mock-amm`, `contracts/mock-lending` | Already confidence-aware gate configuration (CHANGELOG "Mock AMM liquidity gate"). | Current | None. |

## Verification

- `examples/amm_gate_example.rs`: the updated `swap` keeps its signature, so the
  existing tests (`test_swap_with_passing_gate`, `test_swap_with_failing_gate`,
  `test_swap_with_unknown_wallet`) are unchanged and their expected outcomes are
  the same.

  > The example builds as part of `ledgerlens-score`
  > (`cargo test -p ledgerlens-score --example amm_gate_example`). That crate
  > does not compile on `main` at the time of this audit (pre-existing breakage
  > outside this issue's scope), so the test run must be repeated once `main`
  > builds.

## How to repeat this audit

1. List every file under `examples/`, every reference contract under
   `contracts/mock-*`, and every tool under `tools/` that calls a LedgerLens
   entry point.
2. Check each against the four rules above, using the decision guide at the
   release's tag.
3. Update anything that has fallen behind. If an example deliberately teaches
   something narrower, record the reason in the table above instead.
4. Add a row for every new example or tool.

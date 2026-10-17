# Migration Guide: Interface v3 → Current

Issue: #1435
Linked from: [`docs/interface-versioning-policy.md`](interface-versioning-policy.md)

This guide is for integrators who built against the **earliest interface
version still supported** and want a single, example-driven walkthrough of what
changed, instead of reading the changelog from the beginning.

## Which version is the earliest supported?

**Interface v3** (`CONTRACT_VERSION` 3, release `3.0.0`, 2026-06-22).

- The versioning policy is `Effective: v3.0.0+`. Nothing before v3 was ever
  covered by its compatibility guarantees.
- [`docs/deprecation-policy.md`](deprecation-policy.md) keeps deprecated
  symbols callable for at least **two full major interface versions**, and
  v3 is still inside that window.
- The historical-WASM compatibility suite freezes exactly this release:
  `tests/fixtures/historical/ledgerlens-score-v3-8336828.wasm`
  ([`docs/historical-wasm-compatibility.md`](historical-wasm-compatibility.md)).

The "current" side of every comparison below is the `main` source tree
(`CONTRACT_VERSION` 5). The v3 side is the contract-spec read from the frozen
v3 WASM (via the generated [`schemas/c/ledgerlens_score.h`](../schemas/c/ledgerlens_score.h)),
so the inventory comes from actual ABIs rather than memory.

## At a glance

| # | Change | Who is affected | Action required? |
|---|---|---|---|
| C1 | `RiskScore` gained 4 fields | Off-chain/on-chain code that **decodes `RiskScore` directly** with a v3 binding | **Yes**, or use the shim (zero code changes) |
| C2 | `get_version()` returns 5; interface-spec header still says "Interface version: 3" | Code that **compares version numbers** | **Yes**, if you compare versions |
| C3 | `override_rate_limit` gained a required `justification` argument | Operators / admin tooling | **Yes** |
| C4 | `open_score_dispute` gained `challenger` + `bond_salt` and a preceding `commit_dispute_bond` step | Dispute challengers | **Yes** |
| C5 | `verify_membership` gained a required `timestamp` argument | Callers verifying score-commitment proofs | **Yes** |
| C6 | 164 new functions (no removals) | Nobody who does not call them | No (additive) |
| C7 | New capability symbols in `supports_interface` | Nobody | No (additive) |
| C8 | Error enum: 50 variants, identical discriminants | Nobody | No |
| C9 | Gate reads are not strictly side-effect free (record gate-read ledger) | Nobody functionally | No (clarification) |
| C10 | Optional strict gate allowlist / metered gate | Consumers of a deployment that **enables** strict mode | Only if your operator enables it |
| C11 | Gate consumers (`query_risk_gate`, `query_risk_gate_with_confidence`, `supports_interface`) | — | **No.** Same signatures, same boolean contract, verified both directions by the historical-WASM suite |

If you only call the gate functions, **nothing requires action**. Read C2,
then stop.

---

## C1. `RiskScore` gained four fields (action required for direct decoders)

```text
v3:      RiskScore(benford_flag, confidence, ml_flag, model_version, score, timestamp)
current: RiskScore(benford_flag, benford_score, commitment, confidence, ml_flag,
                   ml_score, model_version, network_score, score, timestamp)
```

The new fields are `benford_score`, `ml_score`, `network_score` (all
`u32`, `[0, 100]`) and `commitment` (`Option<Bytes>`). Because contract-spec
fields are ordered alphabetically, they are **interleaved** with the v3 fields,
not appended at the end.

> **Clarification of the versioning policy.** §3.3 of
> `interface-versioning-policy.md` describes field additions as append-only
> tolerant. In practice, a Soroban `#[contracttype]` struct decodes from an
> `ScMap` with an exact field set, and spec order is alphabetical, so a
> six-field v3 binding **cannot** decode the ten-field current struct. This is
> already recorded in `docs/historical-wasm-compatibility.md` ("A consumer that
> directly decodes the historical six-field `RiskScore` must keep its v3 binding
> or migrate explicitly"). Treat C1 as action-required.

**Option A: zero code changes (compatibility shim).** For off-chain
integrations, [`schemas/compat/ledgerlens_v3.py`](../schemas/compat/ledgerlens_v3.py)
is a drop-in replacement for the v3-era `ledgerlens_v3` binding. It has the
same module name, the same `RiskScore` class, and the same `from_payload` API.
It accepts both shapes and projects the current one onto the v3 view:

```sh
# before: PYTHONPATH=<your v3 binding>
PYTHONPATH=schemas/compat python3 your_v3_integration.py
```

The shim still fails closed on a missing v3 field, an out-of-range
`score`/`confidence`, or an **unknown** field. A future interface change it was
not built for is rejected instead of guessed at. Its list of additive fields is
tested against the generated `schemas/risk_score.schema.json`, so it cannot
drift from the contract-spec.

**Option B: migrate.** Regenerate against the current spec
(`schemas/risk_score.{ts,py,schema.json}` or the typed Rust client) and ignore
the new fields if you do not need them.

**On-chain consumers** that call `get_score` through a v3 typed client must
regenerate the client (Option B). There is no on-chain shim, because the
decoding happens inside the host. Gate-only consumers are unaffected (C11).

## C2. Version numbers (action required only if you compare them)

`get_version()` now returns `5`, while `docs/interface-spec.md` still reads
"Interface version: 3", and the decision guide §6 says "`get_version() >= 4`".
Code such as `if client.get_version() == 3 { … }` silently takes the wrong
branch.

**Action:** replace version comparisons with capability detection, as
`interface-versioning-policy.md` §5 already requires:

```rust
// before (v3-era)
if client.get_version() == 3 { use_confidence_gate(); }
// after
if client.supports_interface(&symbol_short!("cgate")) { use_confidence_gate(); }
```

## C3. `override_rate_limit` requires a justification

```text
v3:      override_rate_limit(admin_signers, wallet, asset_pair)
current: override_rate_limit(admin_signers, wallet, asset_pair, justification)
```

**Action (operators):** pass a `justification` (`Bytes`). Overrides are now
recorded in `get_rate_limit_override_log`. `batch_override_rate_limit` keeps
its v3 shape.

## C4. Disputes use commit-reveal bonds

```text
v3:      open_score_dispute(wallet, asset_pair, bond)
current: commit_dispute_bond(challenger, wallet, asset_pair, bond_amount_salt)
         open_score_dispute(challenger, wallet, asset_pair, bond, bond_salt)
```

**Action (challengers):** commit to the bond amount first, then reveal it in
`open_score_dispute` together with the salt. See
[`docs/commit-reveal-flow.md`](commit-reveal-flow.md).

## C5. `verify_membership(commitment, wallet, asset_pair, score, timestamp, proof)`

**Action:** pass the score's `timestamp`. It is now bound into the membership
proof (see [`docs/verkle-commitment.md`](verkle-commitment.md)).

## C6–C8. Additive and unchanged (no action)

- **C6:** No v3 function was removed. 164 functions were added. Notable ones for
  integrators: `get_score_opt` / `get_score_exists` (non-erroring reads),
  `is_score_risky_with_confidence`, `get_interface_metadata`,
  `peek_is_embargoed`, `query_risk_gate_metered`, and
  `register_score_change_hook`.
- **C7:** `supports_interface("score" | "gate" | "cgate")` still answers `true`.
  New symbols are additive.
- **C8:** The `Error` enum has the same 50 variants with the same discriminants
  as v3. Existing error-code handling is unaffected.

## C9–C10. Gate clarifications

- **C9:** `query_risk_gate` records the gate-read ledger in temporary storage
  (flash-loan protection, #300). The earlier "side-effect free" wording was
  corrected (CHANGELOG, Fixed). The return value and the infallible contract are
  unchanged.
- **C10:** If an operator enables `set_gate_enforcement_mode(true)`, callers not
  in `set_gate_callers` receive `false`. Strict mode is off by default. Ask your
  operator before relying on it either way.

## Validation against a deliberately old integration

[`examples/legacy-v3/consumer.py`](../examples/legacy-v3/consumer.py) is an
off-chain risk-desk integration written against v3 and kept **unmodified**. It
imports `RiskScore` from the v3-era binding (frozen in
`examples/legacy-v3/v3_binding/`) and blocks on score ≥ threshold or
confidence < floor.

`examples/legacy-v3/validate.sh` walks it through this guide:

```text
$ examples/legacy-v3/validate.sh
1. v3 binding, v3 payload:       allow
2. v3 binding, current payload:  ValueError: unexpected RiskScore fields: [... 'benford_score', 'commitment', ... 'ml_score', ... 'network_score', ...]
3. shim, v3 payload:             allow
4. shim, current payload:        allow
migration guide validated: unmodified v3 consumer works on current payloads via the shim
```

**Result:**

- Step 2 reproduces the C1 break exactly as this guide predicts.
- Following C1 Option A (only `PYTHONPATH` changes) brings the unmodified
  integration fully up to date for current payloads (step 4) without
  regressing on v3 payloads (step 3).
- C2–C5 do not apply to this integration: it does not compare versions,
  override rate limits, open disputes, or verify proofs. The guide correctly
  identifies them as not-applicable.

Both the walkthrough and the shim are covered by
`python3 -m unittest tests/test_ledgerlens_v3_shim.py` (registered in
`test-tiers.toml`).

On-chain, the equivalent v3 consumers (`mock-amm`, `mock-lending`) are already
validated in both directions by the historical-WASM suite (C11).

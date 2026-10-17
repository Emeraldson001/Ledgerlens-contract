# Cost of Correctness: Optional Safety & Governance Overhead

Issue: #1436
Benchmark: [`contracts/ledgerlens-score/benches/cost_of_correctness.rs`](../contracts/ledgerlens-score/benches/cost_of_correctness.rs)
Generated figures: `docs/reports/cost-of-correctness.<profile>.{json,md}`

This report quantifies what each optional safety and governance mechanism costs
on the hot entry points it touches. The goal is to let an operator choosing a
deployment profile reason about the actual tradeoff. All figures are
**generated from the benchmark suite**, not estimated by hand. The units are the
same soroban-sdk budget units as [`docs/resource-budgets.md`](resource-budgets.md)
(`cpu_instruction_cost`, `memory_bytes_cost`).

## How features are toggled

LedgerLens has two toggle mechanisms, and the report covers both:

1. **Compile-time Cargo feature profiles** (#1193): `core`
   (`--no-default-features`), `standard` (default), and `full`. See
   [`docs/wasm-size-budget.md`](wasm-size-budget.md). Each profile gets its own
   generated report file.
2. **Runtime configuration** (admin setters). The benchmark measures each one
   in isolation on top of a minimal baseline:

| Feature key | Enabled by | Hot paths it touches |
|---|---|---|
| `confidence_floor` | `set_global_min_confidence(70)` | gate queries |
| `embargo` | `set_score_embargo(other, Some(..))` (populated index, measured wallet not embargoed) | gate queries |
| `score_floor_policy` | `set_score_floor_policy(true, 80, 60)` | `submit_score` |
| `flash_protection` | `set_flash_protection_mode(Reject)` | `submit_score` (reads the gate-read ledger); gates write it |
| `gate_allowlist` | `set_gate_enforcement_mode(true)` + `set_gate_callers([consumer])` | gate queries |
| `consumer_read_quota` | `set_consumer_read_quota(consumer, 1000, 1)` | `query_risk_gate_metered` |
| `adaptive_rate_limit` | `set_adaptive_rate_limit(true, 100)` | `submit_score` (effective cooldown) |
| `finality_buffer` | `set_finality_buffer(600)` | `submit_score` (pending-score write path) |
| `admin_multisig` | `set_destructive_multisig(true)` | admin/governance setters only (measured on `set_cooldown`) |

**Not separately toggleable**, so it is always paid and appears only in the
baseline:

- **Rate limiting (cooldown):** `set_cooldown` is bounded to
  `MIN_COOLDOWN_SECS..=MAX_COOLDOWN_SECS` and cannot be turned off.
- **Dispute handling:** disputes are opened per score by challengers and are not
  an operator toggle. On the gate path, the `peek_risk_band_state` check runs
  regardless of configuration.

## Measured scenarios

For each hot entry point (`submit_score` steady-state resubmission,
`query_risk_gate`, `query_risk_gate_metered`) the benchmark records:

- `baseline (minimal)`: a freshly initialized contract with no optional
  feature configured;
- one row per feature: the baseline with **only that feature** enabled. The
  delta against the baseline is the feature's marginal cost;
- `all features enabled`: every feature at once. The difference between this
  row and the sum of the per-feature deltas shows interaction effects.

The soroban-sdk test environment is deterministic, so one sample per scenario
is exact (the same rationale as `entry_point_budgets.rs`).

## Regenerating and checking for drift

```sh
# Regenerate (one report per compiled profile)
cargo bench -p ledgerlens-score --no-default-features --bench cost_of_correctness -- --write
cargo bench -p ledgerlens-score                       --bench cost_of_correctness -- --write
cargo bench -p ledgerlens-score --features full       --bench cost_of_correctness -- --write

# Verify committed figures against a fresh run (exit 1 on drift)
cargo bench -p ledgerlens-score --bench cost_of_correctness -- --check
```

**Tolerance:** ±2% on both CPU and memory, per row. The host is deterministic,
so anything beyond that is a real behavior change, not noise. This is
deliberately tighter than the 10%/20% regression policy in `resource-budgets.md`,
because the goal here is to detect *stale figures*, not to gate performance.

> **Status of the committed figures.** At the time of writing, the
> `ledgerlens-score` crate does not compile on `main` (pre-existing breakage
> from earlier merges, outside this issue's scope). The benchmark therefore
> cannot run yet, and **no figures are committed**. Committing numbers that were
> not produced by the benchmark would defeat the point of this report. The
> first `--write` run after `main` builds produces
> `docs/reports/cost-of-correctness.<profile>.{json,md}`, and `--check` keeps
> them honest from then on.

## Expected attribution (from code inspection, to be confirmed by the figures)

These are structural observations from reading the hot paths. They predict what
the figures will show and explain them once generated.

- **Most gate-path protections are paid even when "disabled".**
  `query_risk_gate_for_consumer` always reads the gate-enforcement mode, the
  embargo flag (`peek_is_embargoed`), the risk-band state, and the global
  confidence floor, whether or not they are configured. Enabling
  `confidence_floor` or `embargo` should therefore add close to zero over the
  baseline. A genuinely cheaper gate would need these checks compiled out (a
  `core`-profile follow-up), not merely left unconfigured.
- **`gate_allowlist` adds an instance read plus an O(callers) `contains`** on
  every gate call. Note that the non-metered `query_risk_gate` passes the
  contract's own address as the consumer. With strict enforcement on, it fails
  closed early unless that address is allow-listed, which makes it *cheaper*
  and also changes its answer. Its row must be read with that in mind.
- **`consumer_read_quota` adds a persistent read and write** (token-bucket
  refill and consume) to `query_risk_gate_metered` only.
- **`finality_buffer` changes the shape of `submit_score`**: the score goes to
  a pending entry instead of the live record. This is the largest per-feature
  change on the write path, both in cost and in semantics.
- **`flash_protection`** adds one temporary-storage read to `submit_score`, and
  the gate-read ledger write is already paid by every `query_risk_gate` call.
- **`admin_multisig`** never touches submission or gate paths. It only adds
  signer-set validation on governed setters.

## Recommended minimal-viable deployment profile

For operators who want the lowest per-call cost:

| Setting | Minimal profile |
|---|---|
| Cargo profile | `core` (`--no-default-features`) |
| Cooldown | `MIN_COOLDOWN_SECS` (cannot be disabled) |
| Global confidence floor | `0` (consumers pass `min_confidence` per call instead) |
| Embargo list | empty |
| Score floor policy | disabled |
| Flash protection | `Warn` (default) |
| Gate allowlist | off (non-strict) |
| Consumer read quotas | none; consumers use `query_risk_gate`, not the metered variant |
| Adaptive rate limit | disabled |
| Finality buffer | `0` (scores are live immediately) |
| Destructive multisig | single admin |

**Protections this profile gives up, stated explicitly:**

- **No global confidence floor.** A consumer that forgets to pass
  `min_confidence` gates on low-confidence scores.
- **No operator embargo.** There is no way to force `false` for a specific
  wallet during an investigation without pausing everything.
- **No score floor.** A previously high-risk wallet can be re-scored straight
  down to a low score.
- **Flash protection only warns.** A same-ledger gate read followed by a
  score submission is logged but not rejected, which leaves the
  read-then-manipulate window open.
- **Any caller can query the gate, without per-consumer rate limiting.** Read
  quotas are a DoS and fair-use control (`docs/dos-read-patterns.md`).
- **No finality buffer.** A bad submission is live on the gate immediately, with
  no hold window in which to catch it before consumers act on it.
- **Single-key governance.** One compromised admin key can change
  configuration and run destructive operations without a second signer.

Operators should choose this profile only when the gate's answer is advisory,
or when the protections are enforced by another layer. Because the
gate-path checks are paid regardless (see above), the measured saving of
*unconfiguring* gate protections is expected to be small. The biggest
reductions come from leaving `finality_buffer` at `0` and from consumers not
using `query_risk_gate_metered`.

## See also

- [`docs/resource-budgets.md`](resource-budgets.md): per-entry-point budgets.
- [`docs/consumer-integration-decision-guide.md` §7](consumer-integration-decision-guide.md#7-resource-usage):
  consumer-side cost guidance.
- [`docs/configuration-safe-defaults.md`](configuration-safe-defaults.md): the
  recommended (safe) defaults this profile deliberately relaxes.

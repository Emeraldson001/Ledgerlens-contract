# Minimal C Header / ABI Description for Non-Generated-Client Integrators (Spike)

Issue: #1438
Status: Spike complete. The recommendation below is ready for maintainer review.

## Summary

Integrators working in an environment with no Soroban client generator
(embedded tooling, a language without a generator yet) have no lower-level
reference for the raw contract ABI. Today their only options are reading the
Rust source or decoding the XDR contract-spec from scratch.

This spike adds a **prototype, dependency-free C header** describing every
contract function signature, user-defined type, and error code. It is generated
from the **same Soroban contract-spec (XDR) source** that `tools/schema-gen`
already uses for the JSON Schema / TypeScript / Python bindings
([`docs/risk-score-schema-generation.md`](risk-score-schema-generation.md)), so
it adds no new source of truth. A ~200-line C99 program that uses only libc and
this header performed a real read call against a live testnet contract
instance through `curl` and RPC `simulateTransaction`.

**Recommendation:** adopt it as an official artifact, published as a byproduct
of the existing schema-generation pipeline and not as a standalone,
hand-maintained file. Maintenance plan below.

## Artifacts

| Path | What it is |
| --- | --- |
| [`tools/schema-gen/src/c_header.rs`](../tools/schema-gen/src/c_header.rs) | Generator: `ScSpecEntry` list → C header. Depends only on the XDR spec types, never on the contract crate. |
| `schema-gen --wasm <path> --c-header` | CLI entry point; writes `<out>/c/ledgerlens_score.h`. `--check` fails if the committed header is stale. |
| [`schemas/c/ledgerlens_score.h`](../schemas/c/ledgerlens_score.h) | Prototype artifact committed for review (2,295 lines; 167 functions, 50 error codes). |
| [`examples/c-abi/ll_read.c`](../examples/c-abi/ll_read.c) | Minimal dependency-free invocation encoder / result decoder. |
| [`examples/c-abi/read.sh`](../examples/c-abi/read.sh) | `cc` + `curl` + `sed` wrapper that performs the read call. |

No existing generated client package (`schemas/risk_score.{schema.json,ts,py}`)
was changed.

## What the header contains

Everything an integrator needs to hand-encode an invocation and decode a
result, and nothing that requires a library:

- **Wire tags**: `LL_SCV_*` `ScValType` discriminants (`LL_SCV_U32 3`,
  `LL_SCV_SYMBOL 15`, `LL_SCV_ADDRESS 18`, …), read from the XDR crate rather
  than written by hand.
- **Primitive shapes**: `ll_address_t` (StrKey text), `ll_symbol_t`,
  `ll_bytes_t`, `ll_bytesN_t` for each fixed size used, `ll_u128_t`/`ll_i128_t`
  hi/lo pairs, and opaque `ll_vec_t`/`ll_map_t`/`ll_val_t`. Each one is commented
  with the `ScVal` arm it travels as.
- **Error codes**: `#define LL_ERROR_SCORE_NOT_FOUND …` for every
  `contracterror` discriminant.
- **Enums / unions**: `#define` constants for enum values and union case tags.
- **Structs**: `typedef struct { … } ll_RiskScore_t;` with fields in
  contract-spec (XDR, alphabetical) order. Each field is annotated with its spec
  type, and the structs are topologically ordered so embedded-by-value structs
  come first.
- **Functions**: for every exported function, the spec signature as a comment
  (same notation as `tests/fixtures/historical/*.abi`), `LL_FN_<NAME>` (the
  function-name symbol), `LL_FN_<NAME>_ARGC`, an `ll_<fn>_args_t` struct in
  argument order, and an `ll_<fn>_ret_t` typedef (`Result<T, E>` becomes
  `{ bool is_ok; T ok; uint32_t err; }`).

The header includes only `<stdbool.h>`, `<stddef.h>` and `<stdint.h>`, and it
compiles cleanly with `cc -std=c99 -Wall -Wextra -pedantic -Werror`.

## Standalone artifact vs. pipeline byproduct

| | Hand-maintained standalone header | **Byproduct of `schema-gen`** ✅ |
| --- | --- | --- |
| Source of truth | A new, separate copy | The contract-spec bytes the other bindings already use |
| Drift | Silent until an integrator hits it | `--check` makes it a CI failure |
| Coverage | Whatever someone remembers to add | Every function, type, and error in `contractspecv0` |
| Works from a deployed WASM | No | Yes (`soroban_spec::read::from_wasm`) |
| Cost | Ongoing manual review | One ~550-line generator module (including unit tests), run in CI |

A standalone header would recreate exactly the problem that
`docs/risk-score-schema-generation.md` set out to remove (a fourth hand-kept
copy). The byproduct approach costs nothing per release.

## Validation

### Generated from the existing schema source

The committed header was generated from the contract-spec embedded in the
locked historical release WASM, which is committed repository data:

```sh
cargo run -p schema-gen -- \
  --wasm tests/fixtures/historical/ledgerlens-score-v3-8336828.wasm --c-header
```

It is built from the same `soroban_spec::read::from_wasm` read path as
`--invocations` and the `RiskScore` WASM pathway.

> **Note on the source WASM.** The current `contracts/ledgerlens-score` crate
> does not compile on `main` at the time of this spike (pre-existing breakage
> outside this issue's scope), so no current release WASM can be built. That
> also means the `schema-gen` binary, which links the contract crate for its
> native `RiskScore` path, cannot be built either. The generator module was
> therefore verified on its own: `c_header.rs` was compiled against the same
> `soroban-sdk`/`soroban-spec` versions in an isolated crate. Its unit tests
> pass, and its output for the fixture WASM is byte-identical to the committed
> header. Once `main` builds, regenerate from the release WASM with the
> command above.

### Unit tests (`c_header.rs`)

- The function name, `ARGC`, argument struct, and `Result` return typedef are
  emitted correctly.
- Error discriminants and `ScVal` wire tags (`U32 = 3`, `ADDRESS = 18`) are
  correct.
- `Option<Bytes>` becomes `{ bool is_some; ll_bytes_t value; }`.
- Struct fields stay in spec order, and a struct is defined before any function
  returning it.
- C keywords are escaped (`default` → `default_`), and `*/` inside spec docs
  cannot terminate a comment.

### A real read call using only the header

`examples/c-abi/ll_read.c` takes the function name, `LL_SCV_*` argument tags and
return-value tags from the header. It hand-encodes a `TransactionEnvelope`
containing one `InvokeHostFunction` operation, with no signatures (simulation
needs none). `read.sh` posts it to RPC `simulateTransaction` with `curl` and
decodes the returned `ScVal`.

There is no public LedgerLens deployment to call, so the live run targeted the
testnet native-asset Stellar Asset Contract. Its `decimals() -> u32` read has
exactly the same wire shape as LedgerLens `get_version() -> u32`:

```text
$ examples/c-abi/read.sh CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC decimals
u32 7
$ examples/c-abi/read.sh CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC balance \
    addr:GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7
ScVal tag 10 (16 bytes) — decode per ledgerlens_score.h     # LL_SCV_I128
```

Against a LedgerLens deployment, swap in its contract ID:

```sh
examples/c-abi/read.sh "$LEDGERLENS_ID" get_version
examples/c-abi/read.sh "$LEDGERLENS_ID" supports_interface sym:cgate
examples/c-abi/read.sh "$LEDGERLENS_ID" query_risk_gate addr:G... sym:XLM_USDC u32:75
```

A call with an unknown function name is rejected by the host (`symbol not found
in slice of strs`). That confirms the envelope reaches contract dispatch, and
that the function symbol is the only contract-specific part of the encoding.

## Gaps found

- **Per-field semantics.** XDR carries no numeric ranges. The header documents
  the type, not the `[0, 100]` domain. If the header is adopted, it should
  reuse `risk_domain_range` the same way the JSON Schema does.
- **Composite values stay opaque.** `vec`/`map`/`val` stay opaque (`ll_vec_t`,
  `ll_val_t`), with the element type recorded only in a comment. That suits a
  reference header, but an integrator who needs to decode `Vec<RiskScore>` still
  has to write a small `SCV_VEC`/`SCV_MAP` walker. A follow-up could ship an
  optional `ledgerlens_score_xdr.h` with static-inline encoders (still
  dependency-free).
- **Function count.** The v3 spec already exports 167 functions. A future
  `--c-header-consumer` mode could limit output to the stable consumer surface
  (`score`, `gate`, `cgate` capabilities) for embedded targets.

## Recommendation and maintenance plan

**Adopt** the header as an officially published artifact:

1. **Where it lives:** `schemas/c/ledgerlens_score.h`, next to the other
   generated bindings, and attached to each GitHub release as
   `ledgerlens_score.h` alongside the WASM (see
   [`docs/release-process.md`](release-process.md)).
2. **How it stays current:** generate it from the *release* WASM with
   `schema-gen --wasm <release wasm> --c-header`, and add
   `schema-gen --wasm <release wasm> --c-header --check` to the same CI job that
   already runs the `RiskScore` drift check. A stale header fails CI exactly as
   a stale `risk_score.ts` does.
3. **Compatibility:** the header follows
   [`docs/interface-versioning-policy.md`](interface-versioning-policy.md).
   Because it is generated, a header diff *is* an ABI diff. Review it with the
   ABI-diff gate (§9.4) and never hand-edit it.
4. **Scope:** it remains a reference description and never becomes a linkable
   client library, so it adds no runtime dependency and no ABI of its own.
5. **Before the first official publish:** regenerate from the current release
   WASM once `main` builds, and add `examples/c-abi/read.sh` to the
   staging-deployment smoke test against the staging contract ID.

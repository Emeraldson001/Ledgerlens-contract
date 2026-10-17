//! "Cost of correctness" report generator (issue #1436).
//!
//! Run (print the report to stdout):
//!   `cargo bench -p ledgerlens-score --bench cost_of_correctness`
//!
//! Regenerate the committed report (one file per Cargo feature profile):
//!   `cargo bench -p ledgerlens-score --bench cost_of_correctness -- --write`
//!   `cargo bench -p ledgerlens-score --no-default-features --bench cost_of_correctness -- --write`
//!   `cargo bench -p ledgerlens-score --features full --bench cost_of_correctness -- --write`
//!
//! Drift check against the committed report (exit non-zero beyond tolerance):
//!   `cargo bench -p ledgerlens-score --bench cost_of_correctness -- --check`
//!
//! For every optional safety / governance feature this harness measures the
//! CPU-instruction and memory-byte cost of the hot entry points it touches
//! (`submit_score`, `query_risk_gate`, `query_risk_gate_metered`) twice: on a
//! minimal baseline with only that feature enabled, and on the baseline with
//! the feature disabled. The difference is the feature's marginal cost. A
//! fully-enabled configuration (every feature at once) is measured as well so
//! the per-feature attribution can be compared against the cumulative total.
//!
//! The soroban-sdk test environment is deterministic, so a single sample per
//! scenario is sufficient (see the design notes in `entry_point_budgets.rs`).
//! Figures are written as JSON (`docs/reports/cost-of-correctness.<profile>.json`)
//! and as a Markdown table (`docs/reports/cost-of-correctness.<profile>.md`).
//! The narrative report is `docs/cost-of-correctness.md`.

use ledgerlens_score::{
    FlashProtectionMode, LedgerLensScoreContract, LedgerLensScoreContractClient,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, Symbol, Vec,
};
use std::fmt::Write as _;
use std::path::PathBuf;

const START_TS: u64 = 1_700_000_000;

/// Relative tolerance used by `--check` when comparing a fresh run with the
/// committed report. The host is deterministic, so any drift beyond this is a
/// real behavior change rather than measurement noise.
const TOLERANCE_PCT: f64 = 2.0;

/// Compiled Cargo feature profile (#1193). Each profile gets its own report
/// file so the compile-time tiers can be compared alongside runtime toggles.
const PROFILE: &str = if cfg!(feature = "dispute") {
    "full"
} else if cfg!(feature = "governance") {
    "standard"
} else {
    "core"
};

fn json_report() -> String {
    format!("docs/reports/cost-of-correctness.{PROFILE}.json")
}

fn md_report() -> String {
    format!("docs/reports/cost-of-correctness.{PROFILE}.md")
}

/// An optional safety or governance feature that can be toggled on top of the
/// minimal baseline configuration.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Feature {
    ConfidenceFloor,
    Embargo,
    ScoreFloorPolicy,
    FlashProtection,
    GateAllowlist,
    ConsumerReadQuota,
    AdaptiveRateLimit,
    FinalityBuffer,
    AdminMultisig,
}

const FEATURES: [Feature; 9] = [
    Feature::ConfidenceFloor,
    Feature::Embargo,
    Feature::ScoreFloorPolicy,
    Feature::FlashProtection,
    Feature::GateAllowlist,
    Feature::ConsumerReadQuota,
    Feature::AdaptiveRateLimit,
    Feature::FinalityBuffer,
    Feature::AdminMultisig,
];

impl Feature {
    fn key(self) -> &'static str {
        match self {
            Feature::ConfidenceFloor => "confidence_floor",
            Feature::Embargo => "embargo",
            Feature::ScoreFloorPolicy => "score_floor_policy",
            Feature::FlashProtection => "flash_protection",
            Feature::GateAllowlist => "gate_allowlist",
            Feature::ConsumerReadQuota => "consumer_read_quota",
            Feature::AdaptiveRateLimit => "adaptive_rate_limit",
            Feature::FinalityBuffer => "finality_buffer",
            Feature::AdminMultisig => "admin_multisig",
        }
    }

    fn toggle(self) -> &'static str {
        match self {
            Feature::ConfidenceFloor => "set_global_min_confidence(70)",
            Feature::Embargo => "set_score_embargo(other_wallet, Some(..))",
            Feature::ScoreFloorPolicy => "set_score_floor_policy(true, 80, 60)",
            Feature::FlashProtection => "set_flash_protection_mode(Reject)",
            Feature::GateAllowlist => "set_gate_enforcement_mode(true) + set_gate_callers",
            Feature::ConsumerReadQuota => "set_consumer_read_quota(consumer, 1000, 1)",
            Feature::AdaptiveRateLimit => "set_adaptive_rate_limit(true, 100)",
            Feature::FinalityBuffer => "set_finality_buffer(600)",
            Feature::AdminMultisig => "set_destructive_multisig(true)",
        }
    }
}

struct Fixture<'a> {
    env: &'a Env,
    client: LedgerLensScoreContractClient<'a>,
    admin: Address,
    wallet: Address,
    consumer: Address,
    pair: Symbol,
}

fn setup<'a>(env: &'a Env, enabled: &[Feature]) -> Fixture<'a> {
    env.mock_all_auths();
    env.budget().reset_unlimited();
    env.ledger().with_mut(|l| l.timestamp = START_TS);

    let contract_id = env.register_contract(None, LedgerLensScoreContract);
    let client = LedgerLensScoreContractClient::new(env, &contract_id);
    let admin = Address::generate(env);
    let service = Address::generate(env);
    client.initialize(&admin, &service);

    let fx = Fixture {
        env,
        client,
        admin,
        wallet: Address::generate(env),
        consumer: Address::generate(env),
        pair: Symbol::new(env, "XLM_USDC"),
    };
    let signers = Vec::from_array(env, [fx.admin.clone()]);

    for feature in enabled {
        match feature {
            Feature::ConfidenceFloor => fx.client.set_global_min_confidence(&70),
            Feature::Embargo => {
                // A populated embargo index, but not on the measured wallet,
                // so the measured path is the "checked and not embargoed" one.
                let other = Address::generate(env);
                fx.client.set_score_embargo(&other, &Some(START_TS + 86_400));
            }
            Feature::ScoreFloorPolicy => {
                fx.client.set_score_floor_policy(&signers, &true, &80, &60)
            }
            Feature::FlashProtection => {
                fx.client.set_flash_protection_mode(&signers, &FlashProtectionMode::Reject)
            }
            Feature::GateAllowlist => {
                fx.client.set_gate_callers(&signers, &Vec::from_array(env, [fx.consumer.clone()]));
                fx.client.set_gate_enforcement_mode(&signers, &true);
            }
            Feature::ConsumerReadQuota => {
                fx.client.set_consumer_read_quota(&signers, &fx.consumer, &1_000, &1)
            }
            Feature::AdaptiveRateLimit => fx.client.set_adaptive_rate_limit(&signers, &true, &100),
            Feature::FinalityBuffer => fx.client.set_finality_buffer(&signers, &600),
            Feature::AdminMultisig => fx.client.set_destructive_multisig(&signers, &true),
        }
    }
    fx
}

fn submit(fx: &Fixture, score: u32) {
    let _ = fx.client.try_submit_score(
        &Vec::new(fx.env),
        &fx.wallet,
        &fx.pair,
        &score,
        &false,
        &false,
        &fx.env.ledger().timestamp(),
        &90,
        &1,
        &None,
    );
}

/// Measure cost of `f()`, returning (cpu_instructions, memory_bytes).
fn measure<F: FnOnce()>(env: &Env, f: F) -> (u64, u64) {
    env.budget().reset_unlimited();
    env.budget().reset_tracker();
    f();
    (env.budget().cpu_instruction_cost(), env.budget().memory_bytes_cost())
}

/// The hot entry points the report attributes cost to.
const ENTRY_POINTS: [&str; 3] = ["submit_score", "query_risk_gate", "query_risk_gate_metered"];

fn run_entry_point(entry: &str, enabled: &[Feature]) -> (u64, u64) {
    let env = Env::default();
    let fx = setup(&env, enabled);
    match entry {
        "submit_score" => {
            // Measure the steady-state (subsequent) submission: one prior
            // score exists so history, rate-limit, and floor state are live.
            submit(&fx, 40);
            env.ledger().with_mut(|l| l.timestamp += 7_200);
            measure(&env, || submit(&fx, 45))
        }
        "query_risk_gate" => {
            submit(&fx, 40);
            env.ledger().with_mut(|l| l.timestamp += 700);
            measure(&env, || {
                let _ = fx.client.query_risk_gate(&fx.wallet, &fx.pair, &50);
            })
        }
        "query_risk_gate_metered" => {
            submit(&fx, 40);
            env.ledger().with_mut(|l| l.timestamp += 700);
            measure(&env, || {
                let _ = fx.client.query_risk_gate_metered(&fx.consumer, &fx.wallet, &fx.pair, &50);
            })
        }
        other => unreachable!("unknown entry point {other}"),
    }
}

/// Admin-path cost of the multisig governance feature, measured on a
/// representative governed setter (`set_cooldown`).
fn run_governance(enabled: bool) -> (u64, u64) {
    let env = Env::default();
    let features: &[Feature] = if enabled { &[Feature::AdminMultisig] } else { &[] };
    let fx = setup(&env, features);
    let signers = Vec::from_array(&env, [fx.admin.clone()]);
    measure(&env, || {
        let _ = fx.client.try_set_cooldown(&signers, &7_200);
    })
}

struct Row {
    entry: &'static str,
    feature: &'static str,
    cpu: u64,
    mem: u64,
}

fn collect() -> std::vec::Vec<Row> {
    let mut rows = std::vec::Vec::new();
    for entry in ENTRY_POINTS {
        let (base_cpu, base_mem) = run_entry_point(entry, &[]);
        rows.push(Row { entry, feature: "baseline (minimal)", cpu: base_cpu, mem: base_mem });
        for feature in FEATURES {
            let (cpu, mem) = run_entry_point(entry, &[feature]);
            rows.push(Row { entry, feature: feature.key(), cpu, mem });
        }
        let (cpu, mem) = run_entry_point(entry, &FEATURES);
        rows.push(Row { entry, feature: "all features enabled", cpu, mem });
    }
    let (cpu, mem) = run_governance(false);
    rows.push(Row { entry: "set_cooldown", feature: "baseline (minimal)", cpu, mem });
    let (cpu, mem) = run_governance(true);
    rows.push(Row { entry: "set_cooldown", feature: "admin_multisig", cpu, mem });
    rows
}

fn to_json(rows: &[Row]) -> String {
    let mut out = String::from("{\n  \"issue\": 1436,\n");
    let _ = writeln!(out, "  \"profile\": \"{PROFILE}\",");
    let _ = writeln!(out, "  \"tolerance_pct\": {TOLERANCE_PCT},");
    out.push_str("  \"rows\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let sep = if i + 1 == rows.len() { "" } else { "," };
        let _ = writeln!(
            out,
            "    {{ \"entry_point\": \"{}\", \"configuration\": \"{}\", \"cpu_instructions\": {}, \"memory_bytes\": {} }}{sep}",
            r.entry, r.feature, r.cpu, r.mem
        );
    }
    out.push_str("  ]\n}\n");
    out
}

fn delta(value: u64, base: u64) -> String {
    let d = value as i128 - base as i128;
    let pct = if base == 0 { 0.0 } else { d as f64 * 100.0 / base as f64 };
    format!("{d:+} ({pct:+.1}%)")
}

fn to_markdown(rows: &[Row]) -> String {
    let mut out = String::new();
    out.push_str("<!-- Generated by `cargo bench -p ledgerlens-score --bench cost_of_correctness -- --write`. Do not edit by hand. -->\n\n");
    let _ = writeln!(out, "# Cost of Correctness — Generated Figures (`{PROFILE}` profile)\n");
    out.push_str(
        "Units are soroban-sdk test-environment budget units (see `docs/resource-budgets.md`).\n",
    );
    out.push_str("Each feature row is the baseline with **only that feature** enabled; the delta column is its marginal cost.\n\n");
    let mut current = "";
    let mut base = (0u64, 0u64);
    for r in rows {
        if r.entry != current {
            current = r.entry;
            base = (r.cpu, r.mem);
            let _ = writeln!(out, "\n## `{current}`\n");
            out.push_str("| Configuration | CPU instructions | Δ CPU vs baseline | Memory bytes | Δ memory vs baseline |\n");
            out.push_str("|---|---:|---:|---:|---:|\n");
        }
        let toggle = FEATURES
            .iter()
            .find(|f| f.key() == r.feature)
            .map(|f| format!(" — `{}`", f.toggle()))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "| {}{toggle} | {} | {} | {} | {} |",
            r.feature,
            r.cpu,
            delta(r.cpu, base.0),
            r.mem,
            delta(r.mem, base.1)
        );
    }
    out
}

fn parse_committed(json: &str) -> std::vec::Vec<(String, String, u64, u64)> {
    let field = |line: &str, key: &str| -> Option<String> {
        let start = line.find(&format!("\"{key}\": "))? + key.len() + 4;
        let rest = &line[start..];
        let value = if let Some(stripped) = rest.strip_prefix('"') {
            stripped[..stripped.find('"')?].to_string()
        } else {
            rest[..rest.find([',', ' ', '}']).unwrap_or(rest.len())].to_string()
        };
        Some(value)
    };
    json.lines()
        .filter(|l| l.contains("\"entry_point\""))
        .filter_map(|l| {
            Some((
                field(l, "entry_point")?,
                field(l, "configuration")?,
                field(l, "cpu_instructions")?.parse().ok()?,
                field(l, "memory_bytes")?.parse().ok()?,
            ))
        })
        .collect()
}

fn within(fresh: u64, committed: u64) -> bool {
    let diff = (fresh as f64 - committed as f64).abs();
    (committed == 0 && fresh == 0) || diff * 100.0 / (committed.max(1) as f64) <= TOLERANCE_PCT
}

fn main() {
    let args: std::vec::Vec<String> = std::env::args().collect();
    let write = args.iter().any(|a| a == "--write");
    let check = args.iter().any(|a| a == "--check");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    let rows = collect();
    let json = to_json(&rows);
    let markdown = to_markdown(&rows);

    if check {
        let committed = std::fs::read_to_string(root.join(json_report()))
            .unwrap_or_else(|e| panic!("reading {}: {e}", json_report()));
        let committed = parse_committed(&committed);
        let mut drift = 0;
        for r in &rows {
            match committed.iter().find(|c| c.0 == r.entry && c.1 == r.feature) {
                Some(c) if within(r.cpu, c.2) && within(r.mem, c.3) => {}
                Some(c) => {
                    drift += 1;
                    eprintln!(
                        "drift: {} / {}: cpu {} -> {}, mem {} -> {}",
                        r.entry, r.feature, c.2, r.cpu, c.3, r.mem
                    );
                }
                None => {
                    drift += 1;
                    eprintln!("drift: {} / {} missing from committed report", r.entry, r.feature);
                }
            }
        }
        if drift > 0 {
            eprintln!(
                "cost_of_correctness: {drift} row(s) outside ±{TOLERANCE_PCT}%; \
                 re-run with `-- --write` and review the diff"
            );
            std::process::exit(1);
        }
        println!("cost_of_correctness: all {} rows within ±{TOLERANCE_PCT}%", rows.len());
        return;
    }

    if write {
        std::fs::write(root.join(json_report()), &json).expect("write JSON report");
        std::fs::write(root.join(md_report()), &markdown).expect("write Markdown report");
        println!("cost_of_correctness: wrote {} and {}", json_report(), md_report());
    } else {
        print!("{markdown}");
    }
}

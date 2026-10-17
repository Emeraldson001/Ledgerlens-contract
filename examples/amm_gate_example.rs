// Example only — not production code

//! Minimal AMM swap implementation with LedgerLens risk gating.
//!
//! This example demonstrates the recommended integration pattern from
//! `docs/consumer-integration-decision-guide.md` (§6, §8):
//! - Importing the LedgerLens contract client.
//! - Feature-detecting `cgate` with `supports_interface` and calling
//!   `query_risk_gate_with_confidence` so signal quality is enforced.
//! - Falling back to `query_risk_gate` with a stricter threshold on
//!   deployments that do not advertise `cgate`.
//! - Handling the gate result and rejecting high-risk wallets.

#![no_std]

use ledgerlens_score::LedgerLensScoreContractClient;
use soroban_sdk::{contract, contracterror, contractimpl, symbol_short, Address, Env, Symbol};

/// Minimum LedgerLens confidence this AMM accepts (standard AMM profile,
/// `docs/consumer-integration-decision-guide.md` §2.2).
const MIN_CONFIDENCE: u32 = 50;

/// How much stricter the plain gate is made when confidence cannot be
/// enforced at the gate (decision guide §6 fallback).
const NO_CONFIDENCE_THRESHOLD_PENALTY: u32 = 10;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AmmError {
    UserHighRisk = 1,
}

#[contract]
pub struct SimpleAMM;

#[contractimpl]
impl SimpleAMM {
    /// Execute a swap between two tokens, enforcing LedgerLens risk gating.
    ///
    /// Before proceeding with the swap, this function calls the LedgerLens gate
    /// to verify that the user's risk score is below the specified threshold
    /// with at least `MIN_CONFIDENCE` confidence.
    /// If the gate returns `false` (user is too risky, embargoed, or unknown),
    /// the swap is rejected.
    pub fn swap(
        env: Env,
        user: Address,
        asset_pair: Symbol,
        amount_in: u64,
        ledgerlens_id: Address,
        gate_threshold: u32,
    ) -> Result<u64, AmmError> {
        // 1. Build the LedgerLens client
        let client = LedgerLensScoreContractClient::new(&env, &ledgerlens_id);

        // 2. Ask the gate. Both gate functions return bool: true only if the
        // score is below the threshold. They never panic and never raise an
        // error — all failure cases (unknown, embargoed, paused) collapse to false.
        // Prefer the confidence-aware gate when the deployment advertises it.
        let passes_gate = if client.supports_interface(&symbol_short!("cgate")) {
            client.query_risk_gate_with_confidence(
                &user,
                &asset_pair,
                &gate_threshold,
                &MIN_CONFIDENCE,
            )
        } else {
            let stricter = gate_threshold.saturating_sub(NO_CONFIDENCE_THRESHOLD_PENALTY);
            client.query_risk_gate(&user, &asset_pair, &stricter)
        };

        // 3. Reject the swap if the user's risk gate fails
        if !passes_gate {
            return Err(AmmError::UserHighRisk);
        }

        // 4. User passed the gate; proceed with swap logic
        // (In a real AMM, this would include pool checks, reserve calculations, etc.)
        let output_amount = Self::compute_swap_output(amount_in);

        Ok(output_amount)
    }

    /// Simplified swap output calculation (not production logic).
    fn compute_swap_output(amount_in: u64) -> u64 {
        // Example: simple 1:1 swap with a 0.3% fee
        (amount_in * 997) / 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledgerlens_score::LedgerLensScoreContract;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{symbol_short, Vec};

    const GATE_THRESHOLD: u32 = 75;

    struct Fixture<'a> {
        env: Env,
        ledgerlens: LedgerLensScoreContractClient<'a>,
        ledgerlens_id: Address,
        amm: SimpleAMMClient<'a>,
    }

    /// Deploys a real `LedgerLensScoreContract` plus a real `SimpleAMM` in the
    /// same `Env`, so `swap` exercises the actual cross-contract call to
    /// `query_risk_gate` rather than a mocked gate check.
    fn setup<'a>() -> Fixture<'a> {
        let env = Env::default();
        env.mock_all_auths();

        let ledgerlens_id = env.register_contract(None, LedgerLensScoreContract);
        let ledgerlens = LedgerLensScoreContractClient::new(&env, &ledgerlens_id);
        let admin = Address::generate(&env);
        let service = Address::generate(&env);
        ledgerlens.initialize(&admin, &service);

        let amm_id = env.register_contract(None, SimpleAMM);
        let amm = SimpleAMMClient::new(&env, &amm_id);

        Fixture { env, ledgerlens, ledgerlens_id, amm }
    }

    /// Submits a score for `wallet`, advancing the ledger past the 1-hour
    /// cooldown first so repeated submissions in the same test never collide.
    fn submit_score(fixture: &Fixture, wallet: &Address, score: u32) {
        fixture.env.ledger().with_mut(|l| l.timestamp += 3_601);
        fixture.ledgerlens.submit_score(
            &Vec::new(&fixture.env),
            wallet,
            &symbol_short!("XLM_USDC"),
            &score,
            &false,
            &false,
            &fixture.env.ledger().timestamp(),
            &95,
            &1,
            &None,
        );
    }

    #[test]
    fn test_swap_with_passing_gate() {
        let fixture = setup();
        let user = Address::generate(&fixture.env);
        submit_score(&fixture, &user, 10); // 10 < GATE_THRESHOLD(75)

        let result = fixture.amm.try_swap(
            &user,
            &symbol_short!("XLM_USDC"),
            &1_000_000,
            &fixture.ledgerlens_id,
            &GATE_THRESHOLD,
        );

        assert_eq!(result, Ok(Ok(997_000)));
    }

    #[test]
    fn test_swap_with_failing_gate() {
        let fixture = setup();
        let user = Address::generate(&fixture.env);
        submit_score(&fixture, &user, 90); // 90 >= GATE_THRESHOLD(75)

        let result = fixture.amm.try_swap(
            &user,
            &symbol_short!("XLM_USDC"),
            &1_000_000,
            &fixture.ledgerlens_id,
            &GATE_THRESHOLD,
        );

        assert_eq!(result, Err(Ok(AmmError::UserHighRisk)));
    }

    #[test]
    fn test_swap_with_unknown_wallet() {
        let fixture = setup();
        let user = Address::generate(&fixture.env); // never scored

        let result = fixture.amm.try_swap(
            &user,
            &symbol_short!("XLM_USDC"),
            &1_000_000,
            &fixture.ledgerlens_id,
            &GATE_THRESHOLD,
        );

        assert_eq!(result, Err(Ok(AmmError::UserHighRisk)));
    }
}

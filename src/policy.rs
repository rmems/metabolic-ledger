// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! ATP / Kelly **policy** state.
//!
//! This module proposes experimental position sizes. It is not an accounting
//! input. [`crate::paper::PaperLedger`] does not import this module, and
//! realized PnL never reads ATP.

use crate::engine::{ENERGY_COMMITMENT, METABOLIC_COST};

/// Experimental energy-budget policy used by the legacy ghost engine.
///
/// Metabolic cost here is an ATP friction term. It is **not** an economic
/// commission. Economic fees live only on accepted fills in [`crate::paper`].
#[derive(Debug, Clone, PartialEq)]
pub struct AtpPolicy {
    /// Fraction of ATP proposed per signal (legacy default 0.08).
    pub energy_commitment: f32,
    /// ATP friction applied by [`crate::engine::execute_buy`] / `execute_sell`.
    pub metabolic_cost: f32,
}

impl AtpPolicy {
    /// Policy matching the historical engine constants.
    pub const LEGACY: Self = Self {
        energy_commitment: ENERGY_COMMITMENT,
        metabolic_cost: METABOLIC_COST,
    };

    /// Proposed quote-unit spend for a buy: `balance_atp * energy_commitment`.
    ///
    /// The adaptive Kelly fraction on [`crate::wallet::GhostWallet`] is a
    /// separate policy output. This method does not read it, and
    /// [`crate::paper::PaperLedger`] does not apply the result.
    pub fn propose_buy_spend(&self, balance_atp: f32) -> f32 {
        balance_atp * self.energy_commitment
    }

    /// Proposed inventory to sell: `inventory * energy_commitment`.
    /// Never applied by [`crate::paper`].
    pub fn propose_sell_qty(&self, inventory: f32) -> f32 {
        inventory * self.energy_commitment
    }

    /// ATP friction on a proposed spend. Experimental; not economic PnL.
    pub fn atp_friction(&self, proposed_spend: f32) -> f32 {
        proposed_spend * self.metabolic_cost
    }
}

impl Default for AtpPolicy {
    fn default() -> Self {
        Self::LEGACY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_does_not_claim_to_be_accounting() {
        let policy = AtpPolicy::LEGACY;
        assert!((policy.propose_buy_spend(500.0) - 40.0).abs() < 1e-6);
        assert!((policy.propose_sell_qty(1_000.0) - 80.0).abs() < 1e-6);
        assert!((policy.atp_friction(40.0) - 0.04).abs() < 1e-6);

        let tight = AtpPolicy {
            energy_commitment: 0.01,
            metabolic_cost: 0.0,
        };
        let wide = AtpPolicy {
            energy_commitment: 0.20,
            metabolic_cost: 0.0,
        };
        assert!((tight.propose_buy_spend(500.0) - 5.0).abs() < 1e-6);
        assert!((wide.propose_buy_spend(500.0) - 100.0).abs() < 1e-6);
        assert!((tight.propose_sell_qty(1_000.0) - 10.0).abs() < 1e-6);
        assert!((wide.propose_sell_qty(1_000.0) - 200.0).abs() < 1e-6);
    }
}

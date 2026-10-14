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

    /// Proposed quote-unit spend for a buy. Never applied by [`crate::paper`].
    pub fn propose_buy_spend(&self, balance_atp: f32, kelly_fraction: f32) -> f32 {
        balance_atp * kelly_fraction
    }

    /// Proposed inventory fraction to sell. Never applied by [`crate::paper`].
    pub fn propose_sell_qty(&self, inventory: f32, kelly_fraction: f32) -> f32 {
        inventory * kelly_fraction
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
        assert!((policy.propose_buy_spend(500.0, 0.08) - 40.0).abs() < 1e-6);
        assert!((policy.atp_friction(40.0) - 0.04).abs() < 1e-6);
    }
}

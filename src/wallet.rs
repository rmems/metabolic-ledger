// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Ghost wallet — multi-asset virtual portfolio with Kelly position sizing.

use crate::engine::{CELLULAR_ATP, ENERGY_COMMITMENT};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

/// Current market prices for all supported assets (USD).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketPrices {
    pub prices: HashMap<String, f32>,
}

impl MarketPrices {
    /// Get the price for a named asset in USD, returning 0.0 if unknown.
    pub fn get(&self, asset: &str) -> f32 {
        *self.prices.get(asset).unwrap_or(&0.0)
    }
}

fn default_kelly_fraction() -> f32 {
    let val = sanitize_kelly_fraction(ENERGY_COMMITMENT);
    debug_assert!(
        val.is_finite() && (KELLY_MIN..=KELLY_MAX).contains(&val),
        "default_kelly_fraction must return a value within the always-valid bounds [KELLY_MIN, KELLY_MAX]"
    );
    val
}

/// Kelly fraction bounds for the always-valid contract (issue #12).
/// Getter `kelly_fraction()` sanitizes to ENERGY_COMMITMENT outside this (defensive).
/// The update logic in `record_pnl_and_update_kelly` uses a tighter operational range
/// for half-Kelly (see below) — this is intentional (wider getter = safety net for
/// any direct internal sets; tighter clamp = conservative updates from real data).
const KELLY_MIN: f32 = 0.01;
const KELLY_MAX: f32 = 0.25;

/// Operational half-Kelly clamp range (used in record_pnl_and_update_kelly).
/// Separate from defensive KELLY_MIN/MAX per design (addresses consistency feedback).
/// f64 because used in f64 Kelly math before final cast to f32.
const KELLY_OPERATIONAL_MIN: f64 = 0.02;
const KELLY_OPERATIONAL_MAX: f64 = 0.20;

/// Sanitize a Kelly fraction to the always-valid contract (finite and within bounds).
fn sanitize_kelly_fraction(f: f32) -> f32 {
    if f.is_finite() && (KELLY_MIN..=KELLY_MAX).contains(&f) {
        f
    } else {
        ENERGY_COMMITMENT
    }
}

fn deserialize_kelly_fraction<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    let f = f32::deserialize(deserializer)?;
    Ok(sanitize_kelly_fraction(f))
}

fn serialize_kelly_fraction<S>(value: &f32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let sanitized = sanitize_kelly_fraction(*value);
    sanitized.serialize(serializer)
}

/// Canonical summary of persistent portfolio accounting (realized PnL per asset, win-rate, etc.).
/// Exported as the single source of truth per issue #3 AC. Allows downstream (e.g. DendriteTrader.jl)
/// to read canonical summaries without duplicating state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioSummary {
    /// Total realized PnL across assets (computed from the per-asset `realized_pnls` ledger).
    pub total_realized_pnl: f32,
    /// Realized PnL broken down per asset.
    pub realized_pnl_per_asset: HashMap<String, f32>,
    /// Win rate (`winning trades / total closed trades`); `None` if no closed trades.
    /// Do not infer this from the deprecated ghost log, which may lose records.
    pub win_rate: Option<f64>,
    /// Total trade actions executed (buys and sells).
    pub trade_count: u64,
    /// Total closed trades (sells).
    pub closed_trade_count: u64,
    /// Current adaptive Kelly fraction (position size).
    ///
    /// The field is private (not `pub`) to enforce the "always valid" contract (see issue #12)
    /// for *all* construction paths: direct construction by consumers must go through
    /// `PortfolioSummary::new(...)` (which sanitizes), deserialization (sanitizes via serde),
    /// or `GhostWallet::summary()` (always produces valid via `kelly_fraction()`).
    ///
    /// **Always valid when using the provided public APIs** (new(), getter, summary(), or
    /// deserializing): sanitized to `ENERGY_COMMITMENT` if the value is non-finite or outside
    /// [KELLY_MIN, KELLY_MAX]. Defaults to ENERGY_COMMITMENT (0.08) and is updated after
    /// ≥10 decisive trades in `record_pnl_and_update_kelly`.
    ///
    /// Deserialization and serialization both sanitize (via `deserialize_kelly_fraction` /
    /// `serialize_kelly_fraction`) for defense-in-depth. Prefer the `current_kelly_fraction()`
    /// getter (or `summary()`) for the guaranteed-valid value. Exposed for downstream
    /// (e.g. DendriteTrader.jl) as single source of truth (see #12).
    #[serde(
        default = "default_kelly_fraction",
        deserialize_with = "deserialize_kelly_fraction",
        serialize_with = "serialize_kelly_fraction"
    )]
    current_kelly_fraction: f32,
}

impl PortfolioSummary {
    /// Constructs a `PortfolioSummary`, ensuring `current_kelly_fraction` is sanitized
    /// to the always-valid contract (finite + within [KELLY_MIN, KELLY_MAX], else
    /// `ENERGY_COMMITMENT`). This enforces the documented contract for manual construction
    /// by downstream consumers (addresses Devin review info on pub field allowing invalid
    /// direct construction).
    ///
    /// Other fields are taken as-is (no sanitization applied beyond kelly).
    pub fn new(
        total_realized_pnl: f32,
        realized_pnl_per_asset: HashMap<String, f32>,
        win_rate: Option<f64>,
        trade_count: u64,
        closed_trade_count: u64,
        current_kelly_fraction: f32,
    ) -> Self {
        Self {
            total_realized_pnl,
            realized_pnl_per_asset,
            win_rate,
            trade_count,
            closed_trade_count,
            current_kelly_fraction: sanitize_kelly_fraction(current_kelly_fraction),
        }
    }

    /// Returns the current adaptive Kelly fraction (always valid per the contract).
    /// Use this (or `GhostWallet::kelly_fraction()` / `summary()`) instead of field access.
    pub fn current_kelly_fraction(&self) -> f32 {
        self.current_kelly_fraction
    }
}

/// Virtual ghost-trading wallet with biological ATP energy model.
pub struct GhostWallet {
    /// ── ATP base (cellular ATP) ────────────────────────────────────────
    /// Initial and current cellular ATP balance (quote units).
    pub balance_atp: f32,

    /// ── Token positions ───────────────────────────────────────────────────
    /// Current holdings per asset.
    pub balances: HashMap<String, f32>,
    /// Entry (weighted average) price per asset for realized PnL calc.
    pub entry_prices: HashMap<String, f32>,

    /// ── Performance tracking ──────────────────────────────────────────────
    /// Cumulative realized PnL across all closed trades.
    pub cumulative_pnl: f32,
    /// Realized PnL per asset from closed trades (sells). Exposes per-asset accounting per issue #3.
    pub realized_pnls: HashMap<String, f32>,
    /// Total number of trades executed.
    pub trade_count: u64,

    /// ── Kelly criterion state ─────────────────────────────────────────────
    pub win_count: u64,
    pub loss_count: u64,
    pub total_win: f32,
    pub total_loss: f32,
    /// Total closed trades (including break-even), incremented on every sell.
    pub closed_trade_count: u64,
    /// Adaptive trade fraction, initialized to `ENERGY_COMMITMENT`.
    /// pub(crate) to prevent external mutation that could produce invalid values.
    /// Consumers must use kelly_fraction() or summary().current_kelly_fraction() (always valid).
    pub(crate) trade_fraction: f32,
}

impl GhostWallet {
    /// Create a new ghost wallet with diversified initial positions.
    pub fn new() -> Self {
        Self {
            balance_atp: CELLULAR_ATP,
            balances: HashMap::new(),
            entry_prices: HashMap::new(),
            cumulative_pnl: 0.0,
            realized_pnls: HashMap::new(),
            trade_count: 0,
            win_count: 0,
            loss_count: 0,
            total_win: 0.0,
            total_loss: 0.0,
            closed_trade_count: 0,
            trade_fraction: ENERGY_COMMITMENT,
        }
    }

    /// Total portfolio value in quote units at current prices.
    pub fn portfolio_value(&self, prices: &HashMap<String, f32>) -> f32 {
        let mut total = self.balance_atp;
        for (asset, qty) in &self.balances {
            total += qty * prices.get(asset).unwrap_or(&0.0);
        }
        total
    }

    /// Get token balance for a named asset.
    pub fn balance(&self, asset: &str) -> f32 {
        *self.balances.get(asset).unwrap_or(&0.0)
    }

    /// Get cost basis (entry price) for a named asset.
    pub fn entry_price(&self, asset: &str) -> f32 {
        *self.entry_prices.get(asset).unwrap_or(&0.0)
    }

    /// Update balance and cost basis after a buy.
    pub(crate) fn apply_buy(&mut self, asset: &str, qty: f32, net_spend: f32) {
        let current_balance = *self.balances.get(asset).unwrap_or(&0.0);
        let current_entry = *self.entry_prices.get(asset).unwrap_or(&0.0);

        let prev_cost = current_entry * current_balance;
        let new_balance = current_balance + qty;

        self.balances.insert(asset.to_string(), new_balance);
        if new_balance > 1e-9 {
            self.entry_prices
                .insert(asset.to_string(), (prev_cost + net_spend) / new_balance);
        }
    }

    /// Reduce balance after a sell.
    pub(crate) fn apply_sell(&mut self, asset: &str, qty: f32) {
        let current_balance = *self.balances.get(asset).unwrap_or(&0.0);
        let new_balance = current_balance - qty;
        if new_balance > 1e-9 {
            self.balances.insert(asset.to_string(), new_balance);
            return;
        }
        self.balances.remove(asset);
        self.entry_prices.remove(asset);
    }

    /// Record closed trade PnL and update Kelly fraction.
    pub fn record_pnl_and_update_kelly(&mut self, pnl: f32) {
        self.closed_trade_count += 1;
        if pnl > 0.0 {
            self.win_count += 1;
            self.total_win += pnl;
        } else if pnl < 0.0 {
            self.loss_count += 1;
            self.total_loss += pnl.abs();
        }

        // Kelly needs probabilities among decisive trades only (p + q = 1);
        // break-even trades must be excluded so `q` is not inflated.
        let decisive = self.win_count + self.loss_count;
        if decisive < 10 {
            return;
        } // need minimum sample

        let win_rate = self.win_count as f64 / decisive as f64;
        let avg_win = if self.win_count > 0 {
            self.total_win as f64 / self.win_count as f64
        } else {
            0.0
        };
        let avg_loss = if self.loss_count > 0 {
            self.total_loss as f64 / self.loss_count as f64
        } else {
            0.0
        };

        if avg_win < 1e-6 || avg_loss < 1e-6 {
            return;
        }

        let b = avg_win / avg_loss;
        let q = 1.0 - win_rate;
        let full_kelly = (win_rate * b - q) / b;
        // Use operational (tighter) range for computed half-Kelly; defensive [KELLY_MIN, KELLY_MAX]
        // is used only by the kelly_fraction() getter as safety net (see consts above).
        let half_kelly =
            (full_kelly * 0.5).clamp(KELLY_OPERATIONAL_MIN, KELLY_OPERATIONAL_MAX) as f32;
        self.trade_fraction = half_kelly;
    }

    /// Win rate from closed trades (returns `None` if no trades).
    pub fn win_rate(&self) -> Option<f64> {
        if self.closed_trade_count == 0 {
            None
        } else {
            Some(self.win_count as f64 / self.closed_trade_count as f64)
        }
    }

    /// Returns the current adaptive Kelly fraction (always valid).
    /// Sanitizes to ENERGY_COMMITMENT if trade_fraction is non-finite or outside [KELLY_MIN, KELLY_MAX].
    /// This enforces the "always valid" contract (see issue #12).
    pub fn kelly_fraction(&self) -> f32 {
        sanitize_kelly_fraction(self.trade_fraction)
    }

    /// Returns a summary of realized PnL (per-asset + total), win-rate, etc.
    /// Satisfies AC for #3: centralizes accounting so consumers read from one place.
    /// Win-rate also computable from trade log by counting positive realized_pnl_usdt on sells.
    pub fn summary(&self) -> PortfolioSummary {
        // Use new() (which sanitizes kelly) for consistency with the always-valid enforcement
        // for all public construction paths.
        PortfolioSummary::new(
            self.realized_pnls.values().sum(),
            self.realized_pnls.clone(),
            self.win_rate(),
            self.trade_count,
            self.closed_trade_count,
            self.kelly_fraction(),
        )
    }
}

impl Default for GhostWallet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_portfolio_summary_deserialize_sanitizes_kelly() {
        let json_missing = r#"{"total_realized_pnl":0.0,"realized_pnl_per_asset":{},"win_rate":null,"trade_count":0,"closed_trade_count":0}"#;
        let summary: PortfolioSummary = serde_json::from_str(json_missing).unwrap();
        assert!(
            (summary.current_kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "missing field defaults to ENERGY_COMMITMENT"
        );

        let json_negative = r#"{"total_realized_pnl":0.0,"realized_pnl_per_asset":{},"win_rate":null,"trade_count":0,"closed_trade_count":0,"current_kelly_fraction":-0.5}"#;
        let summary: PortfolioSummary = serde_json::from_str(json_negative).unwrap();
        assert!(
            (summary.current_kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "negative kelly should sanitize"
        );

        let json_out_of_range = r#"{"total_realized_pnl":0.0,"realized_pnl_per_asset":{},"win_rate":null,"trade_count":0,"closed_trade_count":0,"current_kelly_fraction":0.99}"#;
        let summary: PortfolioSummary = serde_json::from_str(json_out_of_range).unwrap();
        assert!(
            (summary.current_kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "out-of-range kelly should sanitize"
        );
    }

    proptest! {
        #[test]
        fn prop_record_pnl_and_update_kelly(
            pnls in prop::collection::vec(
                (-1_000_000f32..1_000_000f32).prop_filter("non-zero pnl", |x| x.abs() > 1e-3),
                12..20,
            ),
        ) {
            let mut wallet = GhostWallet::new();
            for pnl in &pnls {
                wallet.record_pnl_and_update_kelly(*pnl);
            }
            let kelly = wallet.kelly_fraction();
            prop_assert!((KELLY_MIN..=KELLY_MAX).contains(&kelly));
            // With >=10 decisive non-zero trades the operational update path is exercised.
            let decisive = wallet.win_count + wallet.loss_count;
            if decisive >= 10 {
                prop_assert!(
                    (KELLY_OPERATIONAL_MIN as f32..=KELLY_OPERATIONAL_MAX as f32)
                        .contains(&kelly)
                );
            }
        }

        #[test]
        fn prop_win_rate_matches_manual_count(
            wins in 0..100u64,
            losses in 0..100u64,
            break_even in 0..100u64,
        ) {
            let mut wallet = GhostWallet::new();
            wallet.win_count = wins;
            wallet.loss_count = losses;
            wallet.closed_trade_count = wins + losses + break_even;
            let expected = if wallet.closed_trade_count == 0 {
                None
            } else {
                Some(wins as f64 / wallet.closed_trade_count as f64)
            };
            prop_assert_eq!(wallet.win_rate(), expected);
        }

        #[test]
        fn prop_summary_consistent(
            realized in prop::collection::hash_map(
                "[A-Z]{1,4}",
                -1_000_000f32..1_000_000f32,
                0..10,
            ),
            trade_count in 0..1_000u64,
            closed_trade_count in 0..1_000u64,
        ) {
            let mut wallet = GhostWallet::new();
            wallet.realized_pnls = realized.clone();
            wallet.trade_count = trade_count;
            wallet.closed_trade_count = closed_trade_count;
            let summary = wallet.summary();
            let expected_total: f32 = wallet.realized_pnls.values().sum();
            prop_assert!((summary.total_realized_pnl - expected_total).abs() < 1e-3);
            prop_assert_eq!(&summary.realized_pnl_per_asset, &realized);
            prop_assert_eq!(summary.trade_count, trade_count);
            prop_assert_eq!(summary.closed_trade_count, closed_trade_count);
            prop_assert_eq!(summary.current_kelly_fraction(), wallet.kelly_fraction());
        }
    }
}

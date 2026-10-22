// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Ghost trade execution engine (legacy ATP / Kelly **policy** path).
//!
//! [`execute_buy`] and [`execute_sell`] size from ATP and Kelly. They are not
//! the exact-fill accounting API. Accepted simulated fills go through
//! [`crate::paper::PaperLedger`], which never reads ATP.

use crate::log::{GhostTradeLog, append_ghost_log};
use crate::wallet::GhostWallet;

/// Initial biological energy currency (quote units).
pub const CELLULAR_ATP: f32 = 500.0;

/// Fraction of available energy committed per trade signal (8%).
pub const ENERGY_COMMITMENT: f32 = 0.08;

/// Metabolic cost per action — models spread + slippage (0.1%).
pub const METABOLIC_COST: f32 = 0.001;

/// Execute a ghost buy order.
///
/// Spends `wallet.kelly_fraction() * wallet.balance_atp` ATP (validated always-valid
/// fraction) after deducting the metabolic cost, then updates the weighted-average
/// cost basis.
pub fn execute_buy(
    wallet: &mut GhostWallet,
    asset: &str,
    price: f32,
    step: u64,
    reason: &str,
    log_path: Option<&str>,
) {
    let spend_usdt = wallet.balance_atp * wallet.kelly_fraction();
    if spend_usdt < 0.01 {
        return;
    }

    let fee = spend_usdt * METABOLIC_COST;
    let net_spend = spend_usdt - fee;
    let qty = net_spend / price.max(1e-9);

    wallet.apply_buy(asset, qty, net_spend);
    wallet.balance_atp -= spend_usdt;
    wallet.trade_count += 1;

    let record = GhostTradeLog {
        timestamp: chrono::Utc::now().to_rfc3339(),
        step,
        action: "buy".to_string(),
        asset: asset.to_string(),
        price_usd: price,
        quantity: qty,
        trade_value_usdt: spend_usdt,
        realized_pnl_usdt: -fee,
        balance_atp: wallet.balance_atp,
        cumulative_pnl: wallet.cumulative_pnl,
        reason: reason.to_string(),
    };

    eprintln!(
        "[ghost BUY ] step={:>5}  asset={:<6} qty={:.4} @ ${:.4}  fee=${:.4}",
        step, asset, qty, price, fee
    );

    if let Some(path) = log_path {
        append_ghost_log(&record, path);
    }
}

/// Execute a ghost sell order.
///
/// Sells `wallet.kelly_fraction() * balance[asset]` units (validated always-valid
/// fraction), deducting metabolic cost.
pub fn execute_sell(
    wallet: &mut GhostWallet,
    asset: &str,
    price: f32,
    step: u64,
    reason: &str,
    log_path: Option<&str>,
) {
    let qty = wallet.balance(asset) * wallet.kelly_fraction();
    if qty < 1e-9 {
        return;
    }

    let entry_price = wallet.entry_price(asset);
    let proceeds = qty * price;
    let fee = proceeds * METABOLIC_COST;
    let net_proceeds = proceeds - fee;
    let pnl = (price - entry_price) * qty - fee;

    wallet.apply_sell(asset, qty);
    wallet.balance_atp += net_proceeds;
    wallet.cumulative_pnl += pnl;
    wallet.trade_count += 1;
    if let Some(p) = wallet.realized_pnls.get_mut(asset) {
        *p += pnl;
    } else {
        wallet.realized_pnls.insert(asset.to_string(), pnl);
    }
    wallet.record_pnl_and_update_kelly(pnl);

    let record = GhostTradeLog {
        timestamp: chrono::Utc::now().to_rfc3339(),
        step,
        action: "sell".to_string(),
        asset: asset.to_string(),
        price_usd: price,
        quantity: qty,
        trade_value_usdt: proceeds,
        realized_pnl_usdt: pnl,
        balance_atp: wallet.balance_atp,
        cumulative_pnl: wallet.cumulative_pnl,
        reason: reason.to_string(),
    };

    eprintln!(
        "[ghost SELL] step={:>5}  asset={:<6} qty={:.4} @ ${:.4}  PnL={:+.4}",
        step, asset, qty, price, pnl
    );

    if let Some(path) = log_path {
        append_ghost_log(&record, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_buy_reduces_usdt() {
        let mut wallet = GhostWallet::new();
        let before = wallet.balance_atp;
        execute_buy(&mut wallet, "ASSET_A", 0.03, 1, "test", None);
        assert!(wallet.balance_atp < before);
    }

    #[test]
    fn test_sell_increases_usdt() {
        let mut wallet = GhostWallet::new();
        wallet.balances.insert("ASSET_A".to_string(), 1_000.0);
        wallet.entry_prices.insert("ASSET_A".to_string(), 0.02);
        let before = wallet.balance_atp;
        execute_sell(&mut wallet, "ASSET_A", 0.05, 1, "test", None);
        assert!(
            wallet.balance_atp > before,
            "sell at profit should increase ATP"
        );
    }

    #[test]
    fn test_trade_count_increments() {
        let mut wallet = GhostWallet::new();
        execute_buy(&mut wallet, "ASSET_A", 90.0, 1, "test", None);
        execute_sell(&mut wallet, "ASSET_A", 95.0, 2, "test", None);
        assert_eq!(wallet.trade_count, 2);
    }

    #[test]
    fn test_portfolio_value() {
        let wallet = GhostWallet::new();
        let prices = HashMap::from([
            ("ASSET_A".to_string(), 12.5),
            ("ASSET_B".to_string(), 86.0),
            ("ASSET_C".to_string(), 1_500.0),
        ]);
        let value = wallet.portfolio_value(&prices);
        assert_eq!(value, wallet.balance_atp);
    }

    #[test]
    fn test_realized_pnl_per_asset() {
        let mut wallet = GhostWallet::new();
        wallet.balances.insert("ASSET_A".to_string(), 1_000.0);
        wallet.entry_prices.insert("ASSET_A".to_string(), 0.02);
        execute_sell(&mut wallet, "ASSET_A", 0.05, 1, "test", None);
        let summary = wallet.summary();
        let pnl = summary
            .realized_pnl_per_asset
            .get("ASSET_A")
            .copied()
            .unwrap_or(0.0);
        // qty = 1000 * 0.08 = 80, proceeds=4, fee=0.004, pnl=(0.05-0.02)*80 - 0.004 = 2.396
        let expected = (0.05 - 0.02) * 80.0 - 0.004;
        assert!(
            (pnl - expected).abs() < 1e-6,
            "got {}, expected {}",
            pnl,
            expected
        );
        assert_eq!(summary.total_realized_pnl, wallet.cumulative_pnl);
    }

    #[test]
    fn test_summary_and_win_rate() {
        let mut wallet = GhostWallet::new();
        // sequence: loss then win on ASSET_A (fraction 0.08, but use direct for simplicity)
        wallet.balances.insert("ASSET_A".to_string(), 1_000.0);
        wallet.entry_prices.insert("ASSET_A".to_string(), 0.02);
        execute_sell(&mut wallet, "ASSET_A", 0.01, 1, "loss", None); // loss
        wallet.balances.insert("ASSET_A".to_string(), 1_000.0);
        wallet.entry_prices.insert("ASSET_A".to_string(), 0.02);
        execute_sell(&mut wallet, "ASSET_A", 0.05, 2, "win", None); // win
        let summary = wallet.summary();
        assert_eq!(summary.trade_count, 2);
        assert!(summary.win_rate.is_some());
        let wr = summary.win_rate.unwrap();
        assert!((wr - 0.5).abs() < 1e-6, "win rate {}", wr);
        assert!(summary.realized_pnl_per_asset.contains_key("ASSET_A"));
        assert!(
            (summary.current_kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "kelly {}",
            summary.current_kelly_fraction()
        );
    }

    #[test]
    fn test_kelly_sanitizes_invalids() {
        // Covers NaN, +/-inf, negative, and out-of-range (per #12 always-valid contract)
        let mut wallet = GhostWallet::new();
        wallet.trade_fraction = f32::NAN;
        assert!(
            (wallet.kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "nan kelly"
        );
        wallet.trade_fraction = f32::INFINITY;
        assert!(
            (wallet.kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "inf kelly"
        );
        wallet.trade_fraction = f32::NEG_INFINITY;
        assert!(
            (wallet.kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "neg-inf kelly"
        );
        wallet.trade_fraction = -0.1;
        assert!(
            (wallet.kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "neg kelly"
        );
        wallet.trade_fraction = 0.5;
        assert!(
            (wallet.kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "too large kelly"
        );
        wallet.trade_fraction = 2.0;
        assert!(
            (wallet.kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "way too large kelly"
        );
    }

    #[test]
    fn test_kelly_boundaries() {
        // Exact inclusive [0.01, 0.25] per contract (and CodeRabbit suggestion)
        let mut wallet = GhostWallet::new();
        wallet.trade_fraction = 0.01;
        assert!(
            (wallet.kelly_fraction() - 0.01).abs() < 1e-6,
            "lower bound kelly"
        );
        wallet.trade_fraction = 0.25;
        assert!(
            (wallet.kelly_fraction() - 0.25).abs() < 1e-6,
            "upper bound kelly"
        );
    }

    #[test]
    fn test_kelly_valid_and_summary() {
        // Valid value passes through; summary() also uses the sanitized value
        let mut wallet = GhostWallet::new();
        wallet.trade_fraction = ENERGY_COMMITMENT;
        assert!(
            (wallet.kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "valid kelly"
        );
        let summary = wallet.summary();
        assert!(
            (summary.current_kelly_fraction() - ENERGY_COMMITMENT).abs() < 1e-6,
            "summary kelly"
        );
    }

    #[test]
    fn test_execute_uses_sanitized_kelly_fraction() {
        let mut wallet = GhostWallet::new();
        wallet.trade_fraction = f32::NAN;
        let before = wallet.balance_atp;
        execute_buy(&mut wallet, "ASSET_A", 1.0, 1, "test", None);
        let expected_spend = before * ENERGY_COMMITMENT;
        assert!(
            (before - wallet.balance_atp - expected_spend).abs() < 1e-4,
            "buy should use sanitized kelly fraction"
        );
    }
}

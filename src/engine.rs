// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Ghost trade execution engine (legacy ATP / Kelly **policy** path).
//!
//! [`execute_buy`] and [`execute_sell`] size from ATP and Kelly. They are not
//! the exact-fill accounting API. Accepted fills go through
//! [`crate::ledger::Ledger`], which never reads ATP.
//!
//! `log_path` enables only the deprecated, best-effort ghost log. Its errors
//! are returned and reject the fill before any wallet commit, but a failed
//! write can still leave a partial line. It is not an audit record;
//! use `Ledger` / `AuditRecord` and handle persistence errors separately.

use crate::wallet::GhostWallet;
use crate::{GhostFill, Side, apply_ghost_fill};

/// Initial biological energy currency (quote units).
pub const CELLULAR_ATP: f32 = 500.0;

/// Fraction of available energy committed per trade signal (8%).
pub const ENERGY_COMMITMENT: f32 = 0.08;

/// Metabolic cost per action — models spread + slippage (0.1%).
pub const METABOLIC_COST: f32 = 0.001;

/// Wall-clock fallback used only by the legacy `execute_*` policy path.
///
/// [`apply_ghost_fill`] itself never reads the clock; pass an injected
/// `event_time_ns` for reproducible traces (issue #50).
///
/// If the system clock cannot be represented as a UTC Unix nanosecond
/// timestamp (a pre-1970 clock, or a value outside `i64` nanoseconds), this
/// fallback injects `0` so the fill can still commit. Prefer calling
/// [`apply_ghost_fill`] with an explicit time when the timestamp matters.
fn wall_clock_event_time_ns() -> u64 {
    chrono::Utc::now()
        .timestamp_nanos_opt()
        .and_then(|ns| u64::try_from(ns).ok())
        .unwrap_or(0)
}

/// Minimum ATP spend execute_buy will size. Smaller candidate spends are a
/// no-op so a near-empty wallet cannot open a dust position.
/// Named ATP bankroll floor (issue #49).
pub const ATP_BANKROLL_FLOOR: f32 = 0.01;

/// Ghost-engine prices must be finite and strictly positive before sizing, so
/// qty * price stays consistent with net spend (issue #49).
fn is_valid_trade_price(price: f32) -> bool {
    price.is_finite() && price > 0.0
}

/// Execute a ghost buy order.
///
/// Spends `wallet.kelly_fraction() * wallet.balance_atp` ATP (validated always-valid
/// fraction) after deducting the metabolic cost, then updates the weighted-average
/// cost basis.
///
/// `log_path` is deprecated best-effort diagnostic output, not a reliable
/// record. A log write failure rejects the fill before any wallet commit.
pub fn execute_buy(
    wallet: &mut GhostWallet,
    asset: &str,
    price: f32,
    step: u64,
    reason: &str,
    log_path: Option<&str>,
) {
    let fraction = wallet.kelly_fraction();
    if fraction <= 0.0 {
        return; // stand-down: no size when the estimated edge is not positive (#48)
    }
    if !is_valid_trade_price(price) {
        return;
    }
    let spend_usdt = wallet.balance_atp * fraction;
    if spend_usdt < ATP_BANKROLL_FLOOR {
        return;
    }

    let fee = spend_usdt * METABOLIC_COST;
    let net_spend = spend_usdt - fee;
    let qty = net_spend / price;
    if !qty.is_finite() || qty <= 0.0 {
        return;
    }

    let fill = GhostFill {
        asset,
        side: Side::Buy,
        price,
        quantity: qty,
        fee,
        step,
        reason,
    };
    if let Err(err) = apply_ghost_fill(wallet, fill, log_path, wall_clock_event_time_ns()) {
        eprintln!("[ghost BUY rejected] {err}");
    }
}

/// Execute a ghost sell order.
///
/// Sells `wallet.kelly_fraction() * balance[asset]` units (validated always-valid
/// fraction), deducting metabolic cost.
///
/// `log_path` is deprecated best-effort diagnostic output, not a reliable
/// record. A log write failure rejects the fill before any wallet commit.
pub fn execute_sell(
    wallet: &mut GhostWallet,
    asset: &str,
    price: f32,
    step: u64,
    reason: &str,
    log_path: Option<&str>,
) {
    let fraction = wallet.kelly_fraction();
    if fraction <= 0.0 {
        return; // stand-down: no size when the estimated edge is not positive (#48)
    }
    if !is_valid_trade_price(price) {
        return;
    }
    let qty = wallet.balance(asset) * fraction;
    if qty < 1e-9 {
        return;
    }

    let proceeds = qty * price;
    let fee = proceeds * METABOLIC_COST;
    let fill = GhostFill {
        asset,
        side: Side::Sell,
        price,
        quantity: qty,
        fee,
        step,
        reason,
    };
    if let Err(err) = apply_ghost_fill(wallet, fill, log_path, wall_clock_event_time_ns()) {
        eprintln!("[ghost SELL rejected] {err}");
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

    #[test]
    fn test_execute_buy_noops_at_zero_kelly() {
        let mut wallet = GhostWallet::new();
        wallet.trade_fraction = 0.0;
        let before = wallet.balance_atp;
        execute_buy(&mut wallet, "ASSET_A", 1.0, 1, "test", None);
        assert_eq!(wallet.balance_atp, before, "buy must no-op at fraction 0");
        assert_eq!(wallet.trade_count, 0);
        assert_eq!(wallet.balance("ASSET_A"), 0.0);
    }

    #[test]
    fn test_execute_sell_noops_at_zero_kelly() {
        let mut wallet = GhostWallet::new();
        wallet.balances.insert("ASSET_A".to_string(), 1_000.0);
        wallet.entry_prices.insert("ASSET_A".to_string(), 0.02);
        wallet.trade_fraction = 0.0;
        let before = wallet.balance_atp;
        execute_sell(&mut wallet, "ASSET_A", 0.05, 1, "test", None);
        assert_eq!(wallet.balance_atp, before, "sell must no-op at fraction 0");
        assert_eq!(wallet.trade_count, 0);
        assert_eq!(wallet.balance("ASSET_A"), 1_000.0);
    }

    #[test]
    fn test_stood_down_wallet_sizes_nothing() {
        // End-to-end: 10 decisive losses stand the wallet down; execute_* no-op
        // even with an open position that a sell could otherwise reduce.
        let mut wallet = GhostWallet::new();
        wallet.balances.insert("ASSET_A".to_string(), 1_000.0);
        wallet.entry_prices.insert("ASSET_A".to_string(), 0.02);
        for _ in 0..10 {
            wallet.record_pnl_and_update_kelly(-1.0);
        }
        assert_eq!(wallet.kelly_fraction(), 0.0);
        let before = wallet.balance_atp;
        execute_buy(&mut wallet, "ASSET_A", 1.0, 1, "test", None);
        execute_sell(&mut wallet, "ASSET_A", 1.0, 2, "test", None);
        assert_eq!(wallet.balance_atp, before);
        assert_eq!(wallet.balance("ASSET_A"), 1_000.0);
        assert_eq!(wallet.trade_count, 0);
    }
    fn assert_no_trade_on_non_positive_prices(
        trade: impl Fn(&mut GhostWallet, f32),
        starting_qty: f32,
    ) {
        for price in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut wallet = GhostWallet::new();
            if starting_qty > 0.0 {
                wallet.balances.insert("ASSET_A".to_string(), starting_qty);
                wallet.entry_prices.insert("ASSET_A".to_string(), 0.02);
            }
            let before_atp = wallet.balance_atp;
            trade(&mut wallet, price);
            assert_eq!(wallet.balance_atp, before_atp);
            assert_eq!(wallet.trade_count, 0);
            assert_eq!(wallet.balance("ASSET_A"), starting_qty);
        }
    }

    #[test]
    fn test_execute_rejects_non_positive_prices() {
        assert_no_trade_on_non_positive_prices(
            |wallet, price| execute_buy(wallet, "ASSET_A", price, 1, "test", None),
            0.0,
        );
        assert_no_trade_on_non_positive_prices(
            |wallet, price| execute_sell(wallet, "ASSET_A", price, 1, "test", None),
            1_000.0,
        );
    }

    #[test]
    fn test_execute_buy_tiny_price_matches_net_spend() {
        let mut wallet = GhostWallet::new();
        let price = 1e-12_f32;
        let before = wallet.balance_atp;
        let expected_spend = before * wallet.kelly_fraction();
        execute_buy(&mut wallet, "ASSET_A", price, 1, "test", None);
        let spent = before - wallet.balance_atp;
        let qty = wallet.balance("ASSET_A");
        assert!(spent > 0.0, "buy at a tiny positive price should spend ATP");
        assert!(
            (spent - expected_spend).abs() / expected_spend < 1e-5,
            "gross spend must match the Kelly budget, got {spent} vs {expected_spend}"
        );
        let net = expected_spend * (1.0 - METABOLIC_COST);
        assert!(
            (qty * price - net).abs() / net < 1e-5,
            "qty * price must match net spend, not a 1e-9-clamped price"
        );
        assert!(qty > 1e12, "tiny price must not be clamped up to 1e-9");
    }

    #[test]
    fn test_execute_buy_respects_atp_bankroll_floor() {
        let mut wallet = GhostWallet::new();
        wallet.balance_atp = ATP_BANKROLL_FLOOR / 2.0;
        wallet.trade_fraction = ENERGY_COMMITMENT;
        execute_buy(&mut wallet, "ASSET_A", 1.0, 1, "test", None);
        assert_eq!(wallet.balance_atp, ATP_BANKROLL_FLOOR / 2.0);
        assert_eq!(wallet.trade_count, 0);
        assert_eq!(wallet.balance("ASSET_A"), 0.0);
    }
}

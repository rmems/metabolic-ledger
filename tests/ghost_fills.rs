// Copyright 2026 Limen-Neural
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use metabolic_ledger::{GhostFill, GhostFillError, GhostWallet, Side, apply_ghost_fill};

fn fill(side: Side, price: f32, quantity: f32, fee: f32) -> GhostFill<'static> {
    GhostFill {
        asset: "TEST",
        side,
        price,
        quantity,
        fee,
        step: 1,
        reason: "test",
    }
}

const EVENT_TIME_NS: u64 = 1_700_000_000_000_000_000;

fn apply<'a>(
    wallet: &mut GhostWallet,
    fill: GhostFill<'a>,
) -> Result<metabolic_ledger::GhostTradeLog, GhostFillError> {
    apply_ghost_fill(wallet, fill, None, EVENT_TIME_NS)
}

#[test]
fn quantity_is_not_resized_by_wallet_kelly() {
    let mut wallet = GhostWallet::new();
    let record = apply(&mut wallet, fill(Side::Buy, 1.0, 1.6, 0.0016)).unwrap();
    assert_eq!(record.quantity, 1.6);
    assert_eq!(wallet.balance("TEST"), 1.6);
    assert!((wallet.balance_atp - (500.0 - 1.6016)).abs() < 0.00003);
    assert_eq!(wallet.trade_count, 1);
}

#[test]
fn partial_and_full_closes_include_both_fees_once() {
    let mut wallet = GhostWallet::new();
    for price in [10.0, 20.0] {
        let record = apply(&mut wallet, fill(Side::Buy, price, 2.0, price * 0.002)).unwrap();
        assert_eq!(record.realized_pnl_usdt, 0.0);
    }
    let first = apply(&mut wallet, fill(Side::Sell, 15.0, 1.0, 0.015)).unwrap();
    assert!((first.realized_pnl_usdt + 0.03).abs() < 0.00001);
    let last = apply(&mut wallet, fill(Side::Sell, 15.0, 3.0, 0.045)).unwrap();
    assert!((last.realized_pnl_usdt + 0.09).abs() < 0.00001);
    assert_eq!(wallet.balance("TEST"), 0.0);
    assert!((wallet.cumulative_pnl + 0.12).abs() < 0.00001);
    assert!((wallet.balance_atp - 500.0 - wallet.cumulative_pnl).abs() < 0.00003);
    assert_eq!(wallet.summary().total_realized_pnl, wallet.cumulative_pnl);
    assert_eq!(wallet.win_rate(), Some(0.0));
}

#[test]
fn small_gross_profit_is_a_loss_after_both_fees() {
    let mut wallet = GhostWallet::new();
    apply(&mut wallet, fill(Side::Buy, 100.0, 1.0, 0.1)).unwrap();
    apply(&mut wallet, fill(Side::Sell, 100.1, 1.0, 0.1001)).unwrap();
    assert!((wallet.cumulative_pnl + 0.1001).abs() < 0.00001);
    assert_eq!(wallet.win_count, 0);
    assert_eq!(wallet.loss_count, 1);
}

#[test]
fn invalid_and_unfunded_fills_do_not_mutate_wallet() {
    let mut wallet = GhostWallet::new();
    assert!(matches!(
        apply(&mut wallet, fill(Side::Sell, 1.0, 1.0, 0.0)),
        Err(GhostFillError::InsufficientHoldings)
    ));
    assert!(matches!(
        apply(&mut wallet, fill(Side::Buy, 500.0, 1.0, 0.5)),
        Err(GhostFillError::InsufficientFunds)
    ));
    for price in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
        assert!(apply(&mut wallet, fill(Side::Buy, price, 1.0, 0.0)).is_err());
    }
    for qty in [f32::NAN, f32::INFINITY, 0.0, -1.0, 1e-30] {
        assert!(apply(&mut wallet, fill(Side::Buy, 1.0, qty, 0.0)).is_err());
    }
    assert_eq!(wallet.balance_atp, 500.0);
    assert_eq!(wallet.trade_count, 0);
    assert!(wallet.balances.is_empty());
}

#[test]
fn oversell_does_not_flatten_or_change_counters() {
    let mut wallet = GhostWallet::new();
    apply(&mut wallet, fill(Side::Buy, 1.0, 2.0, 0.002)).unwrap();
    let before = wallet.balance_atp;
    assert!(matches!(
        apply(&mut wallet, fill(Side::Sell, 1.0, 3.0, 0.003)),
        Err(GhostFillError::InsufficientHoldings)
    ));
    assert_eq!(wallet.balance("TEST"), 2.0);
    assert_eq!(wallet.balance_atp, before);
    assert_eq!(wallet.trade_count, 1);
    assert_eq!(wallet.closed_trade_count, 0);
}

#[test]
fn log_errors_are_returned_before_commit_and_success_rows_match_fills() {
    let dir = tempfile::tempdir().unwrap();
    let mut wallet = GhostWallet::new();
    assert!(matches!(
        apply_ghost_fill(
            &mut wallet,
            fill(Side::Buy, 1.0, 2.0, 0.002),
            Some(dir.path().to_str().unwrap()),
            EVENT_TIME_NS,
        ),
        Err(GhostFillError::Log(_))
    ));
    assert_eq!(wallet.balance_atp, 500.0);
    assert_eq!(wallet.trade_count, 0);
    assert!(wallet.balances.is_empty());
    let path = dir.path().join("nested/trades.jsonl");
    for side in [Side::Buy, Side::Sell] {
        apply_ghost_fill(
            &mut wallet,
            fill(side, 1.0, 2.0, 0.002),
            Some(path.to_str().unwrap()),
            EVENT_TIME_NS,
        )
        .unwrap();
    }
    let lines = std::fs::read_to_string(path).unwrap();
    let rows: Vec<serde_json::Value> = lines
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["quantity"], 2.0);
    let logged_pnl: f64 = rows
        .iter()
        .map(|row| row["realized_pnl_usdt"].as_f64().unwrap())
        .sum();
    assert!((logged_pnl + 0.004).abs() < 0.000001);
    assert!((wallet.cumulative_pnl as f64 - logged_pnl).abs() < 0.000001);
}

#[test]
fn injected_event_time_is_reproducible() {
    const NS: u64 = 1_700_000_000_123_456_789;
    let mut a = GhostWallet::new();
    let mut b = GhostWallet::new();
    let first = apply_ghost_fill(&mut a, fill(Side::Buy, 1.0, 1.0, 0.001), None, NS).unwrap();
    let second = apply_ghost_fill(&mut b, fill(Side::Buy, 1.0, 1.0, 0.001), None, NS).unwrap();
    assert_eq!(first.timestamp, second.timestamp);
    let expected = chrono::DateTime::from_timestamp(1_700_000_000, 123_456_789)
        .unwrap()
        .to_rfc3339();
    assert_eq!(first.timestamp, expected);
    assert!(
        first.timestamp.contains("123456789"),
        "log timestamp must keep nanosecond precision, got {}",
        first.timestamp
    );
}

#[test]
fn injected_event_time_formats_u64_max() {
    let mut wallet = GhostWallet::new();
    let record = apply_ghost_fill(
        &mut wallet,
        fill(Side::Buy, 1.0, 1.0, 0.001),
        None,
        u64::MAX,
    )
    .unwrap();
    let expected = chrono::DateTime::from_timestamp(
        (u64::MAX / 1_000_000_000) as i64,
        (u64::MAX % 1_000_000_000) as u32,
    )
    .unwrap()
    .to_rfc3339();
    assert_eq!(record.timestamp, expected);
}

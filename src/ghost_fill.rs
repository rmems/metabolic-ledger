// Copyright 2026 Limen-Neural
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Quantity-taking bridge for the legacy multi-asset `f32` ghost wallet.
//! This is not the integer `Ledger` parity path.

use crate::{GhostTradeLog, GhostWallet, Side};
use std::{error::Error, fmt, io};

/// A caller-sized simulated fill. Fee is non-negative quote-unit friction.
/// No ATP/Kelly sizing or implicit metabolic charge is applied here.
pub struct GhostFill<'a> {
    pub asset: &'a str,
    pub side: Side,
    pub price: f32,
    pub quantity: f32,
    pub fee: f32,
    pub step: u64,
    pub reason: &'a str,
}

#[derive(Debug)]
pub enum GhostFillError {
    InvalidFill,
    InsufficientFunds,
    InsufficientHoldings,
    Log(io::Error),
}

impl fmt::Display for GhostFillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFill => f.write_str("invalid or unrepresentable ghost fill"),
            Self::InsufficientFunds => f.write_str("insufficient ATP for quantity plus fee"),
            Self::InsufficientHoldings => f.write_str("sell exceeds ghost holdings"),
            Self::Log(err) => write!(f, "ghost log write failed: {err}"),
        }
    }
}

impl Error for GhostFillError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Log(err) => Some(err),
            _ => None,
        }
    }
}

/// Format `event_time_ns` (Unix time in nanoseconds) as RFC3339 UTC.
///
/// With current `chrono`, every `u64` value is representable (0 through year
/// 2554). The `Option` only exists because [`chrono::DateTime::from_timestamp`]
/// returns one; a `None` is treated as [`GhostFillError::InvalidFill`].
fn rfc3339_from_event_time_ns(event_time_ns: u64) -> Option<String> {
    let secs = i64::try_from(event_time_ns / 1_000_000_000).ok()?;
    let nsecs = (event_time_ns % 1_000_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nsecs).map(|dt| dt.to_rfc3339())
}

fn invalid_ghost_fill_inputs(wallet: &GhostWallet, fill: &GhostFill<'_>) -> bool {
    fill.asset.is_empty()
        || !fill.price.is_finite()
        || fill.price <= 0.0
        || !fill.quantity.is_finite()
        || fill.quantity <= 0.0
        || !fill.fee.is_finite()
        || fill.fee < 0.0
        || !wallet.balance_atp.is_finite()
        || wallet.balance_atp < 0.0
        || !wallet.balance(fill.asset).is_finite()
        || wallet.balance(fill.asset) < 0.0
        || wallet.trade_count == u64::MAX
}

fn invalid_notional(notional: f32, cost: f32) -> bool {
    !notional.is_finite() || notional <= 0.0 || !cost.is_finite()
}

fn sell_counters_exhausted(wallet: &GhostWallet) -> bool {
    wallet.closed_trade_count == u64::MAX
        || wallet.win_count == u64::MAX
        || wallet.loss_count == u64::MAX
        || wallet
            .win_count
            .checked_add(wallet.loss_count)
            .and_then(|count| count.checked_add(1))
            .is_none()
}

fn invalid_projected_wallet(
    wallet: &GhostWallet,
    next: &GhostWallet,
    asset: &str,
    pnl: f32,
) -> bool {
    !next.balance_atp.is_finite()
        || next.balance_atp < 0.0
        || next.balance_atp == wallet.balance_atp
        || !next.balance(asset).is_finite()
        || next.balance(asset) == wallet.balance(asset)
        || !next.entry_price(asset).is_finite()
        || !pnl.is_finite()
        || !next.cumulative_pnl.is_finite()
        || !next.total_win.is_finite()
        || !next.total_loss.is_finite()
        || next
            .realized_pnls
            .get(asset)
            .is_some_and(|p| !p.is_finite())
}

/// Project the fill onto a cloned wallet, or return an error without
/// mutating the caller's book.
fn project_ghost_fill(
    wallet: &GhostWallet,
    fill: &GhostFill<'_>,
) -> Result<(GhostWallet, f32, f32), GhostFillError> {
    if invalid_ghost_fill_inputs(wallet, fill) {
        return Err(GhostFillError::InvalidFill);
    }
    let notional = fill.price * fill.quantity;
    let cost = notional + fill.fee;
    if invalid_notional(notional, cost) {
        return Err(GhostFillError::InvalidFill);
    }
    let mut next = wallet.clone();
    let pnl = match fill.side {
        Side::Buy => {
            if cost > wallet.balance_atp {
                return Err(GhostFillError::InsufficientFunds);
            }
            next.apply_buy(fill.asset, fill.quantity, cost);
            next.balance_atp -= cost;
            0.0
        }
        Side::Sell => {
            if fill.quantity > wallet.balance(fill.asset) {
                return Err(GhostFillError::InsufficientHoldings);
            }
            if sell_counters_exhausted(wallet) {
                return Err(GhostFillError::InvalidFill);
            }
            let pnl = (fill.price - wallet.entry_price(fill.asset)) * fill.quantity - fill.fee;
            next.apply_sell(fill.asset, fill.quantity);
            next.balance_atp += notional - fill.fee;
            next.cumulative_pnl += pnl;
            *next
                .realized_pnls
                .entry(fill.asset.to_string())
                .or_default() += pnl;
            next.record_pnl_and_update_kelly(pnl);
            pnl
        }
    };
    // Reject float overflow and changes too small to represent on this legacy path.
    if invalid_projected_wallet(wallet, &next, fill.asset, pnl) {
        return Err(GhostFillError::InvalidFill);
    }
    Ok((next, pnl, notional))
}

/// Apply exactly the supplied quantity and fee, or return an error without
/// changing the wallet. The optional JSONL write happens before wallet commit.
/// An I/O failure may leave a partial log line, but never a committed fill.
/// Buy friction enters weighted cost basis; sell PnL and Kelly therefore include
/// both sides' friction. Buy rows have zero realized PnL (no double charge).
///
/// `event_time_ns` is a caller-injected Unix time in nanoseconds (same clock
/// contract as [crate::ledger::Ledger]). This function never reads the wall
/// clock, so equal fills with the same injected time produce equal log
/// timestamps (issue #50).
pub fn apply_ghost_fill(
    wallet: &mut GhostWallet,
    fill: GhostFill<'_>,
    log_path: Option<&str>,
    event_time_ns: u64,
) -> Result<GhostTradeLog, GhostFillError> {
    let (mut next, pnl, notional) = project_ghost_fill(wallet, &fill)?;
    let GhostFill {
        asset,
        side,
        price,
        quantity,
        step,
        reason,
        ..
    } = fill;
    next.trade_count += 1;
    let record = GhostTradeLog {
        timestamp: rfc3339_from_event_time_ns(event_time_ns).ok_or(GhostFillError::InvalidFill)?,
        step,
        action: match side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
        .into(),
        asset: asset.into(),
        price_usd: price,
        quantity,
        trade_value_usdt: notional,
        realized_pnl_usdt: pnl,
        balance_atp: next.balance_atp,
        cumulative_pnl: next.cumulative_pnl,
        reason: reason.into(),
    };
    #[allow(
        deprecated,
        reason = "Legacy diagnostic output retained for compatibility"
    )]
    if let Some(path) = log_path {
        crate::log::append_ghost_log(&record, path).map_err(GhostFillError::Log)?;
    }
    *wallet = next;
    Ok(record)
}

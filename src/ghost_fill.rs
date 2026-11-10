// Copyright 2026 Limen-Neural
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Quantity-taking bridge for the legacy multi-asset `f32` ghost wallet.
//! This is not the integer `PaperLedger` parity path.

use crate::{GhostTradeLog, GhostWallet, Side, log::append_ghost_log};
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

/// Apply exactly the supplied quantity and fee, or return an error without
/// changing the wallet. The optional JSONL write happens before wallet commit.
/// An I/O failure may leave a partial log line, but never a committed fill.
/// Buy friction enters weighted cost basis; sell PnL and Kelly therefore include
/// both sides' friction. Buy rows have zero realized PnL (no double charge).
pub fn apply_ghost_fill(
    wallet: &mut GhostWallet,
    fill: GhostFill<'_>,
    log_path: Option<&str>,
) -> Result<GhostTradeLog, GhostFillError> {
    let GhostFill {
        asset,
        side,
        price,
        quantity,
        fee,
        step,
        reason,
    } = fill;
    if asset.is_empty()
        || !price.is_finite()
        || price <= 0.0
        || !quantity.is_finite()
        || quantity <= 0.0
        || !fee.is_finite()
        || fee < 0.0
        || !wallet.balance_atp.is_finite()
        || wallet.balance_atp < 0.0
        || !wallet.balance(asset).is_finite()
        || wallet.balance(asset) < 0.0
        || wallet.trade_count == u64::MAX
    {
        return Err(GhostFillError::InvalidFill);
    }
    let notional = price * quantity;
    let cost = notional + fee;
    if !notional.is_finite() || notional <= 0.0 || !cost.is_finite() {
        return Err(GhostFillError::InvalidFill);
    }
    let mut next = wallet.clone();
    let pnl = match side {
        Side::Buy => {
            if cost > wallet.balance_atp {
                return Err(GhostFillError::InsufficientFunds);
            }
            next.apply_buy(asset, quantity, cost);
            next.balance_atp -= cost;
            0.0
        }
        Side::Sell => {
            if quantity > wallet.balance(asset) {
                return Err(GhostFillError::InsufficientHoldings);
            }
            if wallet.closed_trade_count == u64::MAX
                || wallet.win_count == u64::MAX
                || wallet.loss_count == u64::MAX
                || wallet
                    .win_count
                    .checked_add(wallet.loss_count)
                    .and_then(|count| count.checked_add(1))
                    .is_none()
            {
                return Err(GhostFillError::InvalidFill);
            }
            let pnl = (price - wallet.entry_price(asset)) * quantity - fee;
            next.apply_sell(asset, quantity);
            next.balance_atp += notional - fee;
            next.cumulative_pnl += pnl;
            *next.realized_pnls.entry(asset.to_string()).or_default() += pnl;
            next.record_pnl_and_update_kelly(pnl);
            pnl
        }
    };
    // Reject float overflow and changes too small to represent on this legacy path.
    if !next.balance_atp.is_finite()
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
    {
        return Err(GhostFillError::InvalidFill);
    }
    next.trade_count += 1;
    let record = GhostTradeLog {
        timestamp: chrono::Utc::now().to_rfc3339(),
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
    if let Some(path) = log_path {
        append_ghost_log(&record, path).map_err(GhostFillError::Log)?;
    }
    *wallet = next;
    Ok(record)
}

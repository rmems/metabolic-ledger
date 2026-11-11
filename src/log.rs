// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Serialize;
use std::io::Write as _;

/// Legacy ghost-trade diagnostic data, serialized as a JSONL line.
///
/// The deprecated [`append_ghost_log`] writer is best-effort and can leave
/// partial lines. This type is not a reliable accounting or audit record.
/// Use [`crate::audit::AuditRecord`] from [`crate::ledger::Ledger`] instead.
#[derive(Debug, Serialize)]
pub struct GhostTradeLog {
    pub timestamp: String,
    pub step: u64,
    pub action: String,
    pub asset: String,
    pub price_usd: f32,
    pub quantity: f32,
    pub trade_value_usdt: f32,
    pub realized_pnl_usdt: f32,
    pub balance_atp: f32,
    pub cumulative_pnl: f32,
    pub reason: String,
}

/// Best-effort append of legacy diagnostic data, **not an audit record**.
///
/// Serialization, directory creation, file opening, and write errors are
/// returned to the caller, but a failed write can still leave a partial line.
/// `apply_ghost_fill` propagates them as [`crate::GhostFillError::Log`] before
/// any wallet commit; `execute_buy` / `execute_sell` reject the fill on error.
///
/// Use [`crate::ledger::Ledger`] and its [`crate::audit::AuditRecord`] rows
/// for accounting. Callers must persist those rows and handle storage errors;
/// the ledger does not write a durable log itself.
///
/// ```compile_fail
/// #![deny(deprecated)]
/// use metabolic_ledger::log::{GhostTradeLog, append_ghost_log};
///
/// fn write_legacy(record: &GhostTradeLog) {
///     let _ = append_ghost_log(record, "trades.jsonl");
/// }
/// ```
#[deprecated(
    note = "Best-effort diagnostics only; not an audit record. Use Ledger/AuditRecord and handle persistence errors."
)]
pub fn append_ghost_log(record: &GhostTradeLog, path: &str) -> std::io::Result<()> {
    let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    if let Some(parent) = std::path::Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{}", line)
}

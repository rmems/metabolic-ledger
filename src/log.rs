// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Serialize;
use std::io::Write as _;

/// Legacy ghost-trade diagnostic data, serialized as a JSONL line.
///
/// The deprecated [`append_ghost_log`] writer is best-effort and can silently
/// lose records. This type is not a reliable accounting or audit record.
/// Use [`crate::audit::AuditRecord`] from [`crate::paper::PaperLedger`] instead.
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
/// Serialization, directory creation, file opening, and write failures are
/// silently ignored. A return from this function does not confirm a write,
/// and the output may be incomplete or contain a partial line.
/// The legacy engine retains this behavior only for compatibility.
///
/// Use [`crate::paper::PaperLedger`] and its [`crate::audit::AuditRecord`] rows
/// for accounting. Callers must persist those rows and handle storage errors;
/// the paper ledger does not write a durable log itself.
///
/// ```compile_fail
/// #![deny(deprecated)]
/// use metabolic_ledger::log::{GhostTradeLog, append_ghost_log};
///
/// fn write_legacy(record: &GhostTradeLog) {
///     append_ghost_log(record, "trades.jsonl");
/// }
/// ```
#[deprecated(
    note = "Best-effort diagnostics only; failures are silently ignored. Use PaperLedger/AuditRecord and handle persistence errors."
)]
pub fn append_ghost_log(record: &GhostTradeLog, path: &str) {
    let Ok(line) = serde_json::to_string(record) else {
        return;
    };
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", line);
    }
}

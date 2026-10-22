// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Deterministic paper-ledger audit records.
//!
//! Event time is injected by the caller. Wall-clock time is never written.

use crate::types::Side;
use serde::{Deserialize, Serialize};

/// One committed audit row. Field order is the canonical serialization order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub event_time_ns: u64,
    pub kind: AuditKind,
    pub fill_id: Option<String>,
    pub order_id: Option<String>,
    pub instrument: String,
    pub side: Option<Side>,
    pub price_ticks: Option<i64>,
    pub qty_lots: Option<i64>,
    pub fee_subunits: Option<i64>,
    pub realized_pnl_delta: i64,
    pub cash_after: i64,
    pub inventory_after: i64,
    pub total_cost_after: i64,
    pub reason: Option<String>,
    /// `true` when a duplicate identical fill was acknowledged without mutation.
    pub idempotent_replay: bool,
}

/// Audit kind. Exhaustive for the M2 subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    Fill,
    Reject,
    Cancel,
    NonFill,
}

impl AuditKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Reject => "reject",
            Self::Cancel => "cancel",
            Self::NonFill => "non_fill",
        }
    }
}

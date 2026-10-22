// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Versioned golden-fixture contract for Julia / Rust ledger parity (RM-1863).
//!
//! Schema id: `metabolic-ledger.parity.v1`.
//!
//! DendriteTrader.jl (RM-1864) can later emit this document. This crate ships
//! hand-verified fixtures in the same shape and will consume Julia files
//! unchanged.

use crate::audit::AuditRecord;
use crate::error::LedgerError;
use crate::paper::{AcceptedFill, ApplyOutcome, NonFillEvent, NonFillKind, PaperLedger};
use crate::types::Side;
use crate::units::Scale;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Schema identifier written into every fixture file.
pub const PARITY_SCHEMA_ID: &str = "metabolic-ledger.parity.v1";

/// Integer schema version.
pub const PARITY_SCHEMA_VERSION: u32 = 1;

/// One golden document: inputs plus expected ledger outputs (or a typed error).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParityFixture {
    pub schema: String,
    pub schema_version: u32,
    /// `hand-verified` now; `dendritetrader.jl` when RM-1864 emits files.
    pub source: String,
    pub name: String,
    pub units: FixtureUnits,
    pub initial: FixtureInitial,
    pub events: Vec<FixtureEvent>,
    /// Optional external mark used only for expected unrealized PnL.
    pub mark_price_ticks: Option<i64>,
    pub expected: FixtureExpected,
    #[serde(default)]
    pub tolerances: FixtureTolerances,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureUnits {
    pub instrument: String,
    pub quote: String,
    pub price_scale: i64,
    pub qty_scale: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureInitial {
    pub cash_subunits: i64,
    #[serde(default)]
    pub inventory_lots: i64,
    #[serde(default)]
    pub total_cost_subunits: i64,
}

/// Tagged event. Unknown kinds fail serde — they are not coerced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixtureEvent {
    Fill {
        fill_id: String,
        event_time_ns: u64,
        instrument: String,
        side: Side,
        price_ticks: i64,
        qty_lots: i64,
        fee_subunits: i64,
    },
    Reject {
        order_id: String,
        event_time_ns: u64,
        instrument: String,
        reason: String,
    },
    Cancel {
        order_id: String,
        event_time_ns: u64,
        instrument: String,
        reason: String,
    },
    NonFill {
        order_id: String,
        event_time_ns: u64,
        instrument: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FixtureExpected {
    pub error: Option<ExpectedError>,
    pub cash_subunits: Option<i64>,
    pub inventory_lots: Option<i64>,
    pub total_cost_subunits: Option<i64>,
    pub weighted_cost_basis_ticks: Option<i64>,
    pub realized_pnl_subunits: Option<i64>,
    pub fees_subunits: Option<i64>,
    pub unrealized_pnl_subunits: Option<i64>,
    pub audit: Option<Vec<AuditRecord>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedError {
    #[serde(rename = "type")]
    pub type_name: String,
}

/// Absolute integer tolerances. The integer path requires zeros.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FixtureTolerances {
    #[serde(default)]
    pub cash_subunits: i64,
    #[serde(default)]
    pub realized_pnl_subunits: i64,
    #[serde(default)]
    pub unrealized_pnl_subunits: i64,
    #[serde(default)]
    pub fees_subunits: i64,
}

/// Load one JSON fixture from disk.
pub fn load_fixture(path: &Path) -> Result<ParityFixture, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let fx: ParityFixture =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if fx.schema != PARITY_SCHEMA_ID {
        return Err(format!(
            "{}: unknown schema {:?} (expected {PARITY_SCHEMA_ID})",
            path.display(),
            fx.schema
        ));
    }
    if fx.schema_version != PARITY_SCHEMA_VERSION {
        return Err(format!(
            "{}: unsupported schema_version {}",
            path.display(),
            fx.schema_version
        ));
    }
    Ok(fx)
}

/// Apply fixture events and compare committed state to `expected`.
pub fn run_fixture(fx: &ParityFixture) -> Result<(), String> {
    if fx.initial.inventory_lots < 0 {
        return Err("initial inventory must be long/flat (>= 0)".into());
    }
    let scale = Scale {
        price_scale: fx.units.price_scale,
        qty_scale: fx.units.qty_scale,
    };
    let mut book = PaperLedger::new(&fx.units.instrument, scale, fx.initial.cash_subunits)
        .map_err(|e| e.to_string())?;

    if fx.initial.inventory_lots != 0 || fx.initial.total_cost_subunits != 0 {
        return Err("nonzero initial inventory is reserved for a later schema; start flat".into());
    }

    let last = fx.events.len().saturating_sub(1);
    for (i, event) in fx.events.iter().enumerate() {
        let result = apply_event(&mut book, event);
        if i == last
            && let Some(ref err) = fx.expected.error
        {
            match result {
                Err(e) if e.type_name() == err.type_name => {}
                Err(e) => {
                    return Err(format!(
                        "expected error {} but got {}",
                        err.type_name,
                        e.type_name()
                    ));
                }
                Ok(_) => {
                    return Err(format!(
                        "expected error {} but event succeeded",
                        err.type_name
                    ));
                }
            }
            return compare_state(fx, &book);
        }
        result.map_err(|e| format!("event {i}: {e}"))?;
    }

    if let Some(ref err) = fx.expected.error {
        return Err(format!(
            "expected error {} but all events succeeded",
            err.type_name
        ));
    }
    compare_state(fx, &book)
}

fn apply_event(book: &mut PaperLedger, event: &FixtureEvent) -> Result<ApplyOutcome, LedgerError> {
    match event {
        FixtureEvent::Fill {
            fill_id,
            event_time_ns,
            instrument,
            side,
            price_ticks,
            qty_lots,
            fee_subunits,
        } => book.apply_fill(AcceptedFill {
            fill_id: fill_id.clone(),
            event_time_ns: *event_time_ns,
            instrument: instrument.clone(),
            side: *side,
            price_ticks: *price_ticks,
            qty_lots: *qty_lots,
            fee_subunits: *fee_subunits,
        }),
        FixtureEvent::Reject {
            order_id,
            event_time_ns,
            instrument,
            reason,
        } => {
            book.record_non_fill(NonFillEvent {
                event_time_ns: *event_time_ns,
                kind: NonFillKind::Reject,
                order_id: order_id.clone(),
                instrument: instrument.clone(),
                reason: reason.clone(),
            })?;
            Ok(ApplyOutcome::Applied)
        }
        FixtureEvent::Cancel {
            order_id,
            event_time_ns,
            instrument,
            reason,
        } => {
            book.record_non_fill(NonFillEvent {
                event_time_ns: *event_time_ns,
                kind: NonFillKind::Cancel,
                order_id: order_id.clone(),
                instrument: instrument.clone(),
                reason: reason.clone(),
            })?;
            Ok(ApplyOutcome::Applied)
        }
        FixtureEvent::NonFill {
            order_id,
            event_time_ns,
            instrument,
            reason,
        } => {
            book.record_non_fill(NonFillEvent {
                event_time_ns: *event_time_ns,
                kind: NonFillKind::NonFill,
                order_id: order_id.clone(),
                instrument: instrument.clone(),
                reason: reason.clone(),
            })?;
            Ok(ApplyOutcome::Applied)
        }
    }
}

fn compare_state(fx: &ParityFixture, book: &PaperLedger) -> Result<(), String> {
    let snap = book
        .snapshot(fx.mark_price_ticks)
        .map_err(|e| e.to_string())?;
    let tol = &fx.tolerances;
    let exp = &fx.expected;

    check_eq(
        "cash_subunits",
        exp.cash_subunits,
        snap.cash_subunits,
        tol.cash_subunits,
    )?;
    check_exact("inventory_lots", exp.inventory_lots, snap.inventory_lots)?;
    check_exact(
        "total_cost_subunits",
        exp.total_cost_subunits,
        snap.total_cost_subunits,
    )?;
    check_exact(
        "weighted_cost_basis_ticks",
        exp.weighted_cost_basis_ticks,
        snap.weighted_cost_basis_ticks,
    )?;
    check_eq(
        "realized_pnl_subunits",
        exp.realized_pnl_subunits,
        snap.realized_pnl_subunits,
        tol.realized_pnl_subunits,
    )?;
    check_eq(
        "fees_subunits",
        exp.fees_subunits,
        snap.fees_subunits,
        tol.fees_subunits,
    )?;
    check_eq(
        "unrealized_pnl_subunits",
        exp.unrealized_pnl_subunits,
        snap.unrealized_pnl_subunits,
        tol.unrealized_pnl_subunits,
    )?;

    if let Some(ref audit) = exp.audit
        && audit != &snap.audit
    {
        return Err(format!(
            "audit mismatch:\n expected {}\n got      {}",
            serde_json::to_string(audit).unwrap_or_default(),
            serde_json::to_string(&snap.audit).unwrap_or_default()
        ));
    }
    Ok(())
}

fn check_exact(name: &str, expected: Option<i64>, got: i64) -> Result<(), String> {
    let Some(exp) = expected else {
        return Ok(());
    };
    if exp != got {
        return Err(format!("{name}: expected {exp}, got {got}"));
    }
    Ok(())
}

fn check_eq(name: &str, expected: Option<i64>, got: i64, tol: i64) -> Result<(), String> {
    let Some(exp) = expected else {
        return Ok(());
    };
    if (exp - got).abs() > tol {
        return Err(format!("{name}: expected {exp} ± {tol}, got {got}"));
    }
    Ok(())
}

/// Directory of shipped fixtures (`<crate>/fixtures/v1`).
pub fn shipped_fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/v1")
}

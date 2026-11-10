// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Exact-fill paper ledger for the declared M2 accounting subset.
//!
//! Supported: one instrument, long or flat inventory (`inventory_lots >= 0`).
//! Buys open or add. Sells reduce or close and must not exceed holdings.
//! Shorts and sells beyond inventory are [`LedgerError::UnsupportedShort`]. A
//! second instrument is [`LedgerError::InstrumentMismatch`]. Derivatives cannot
//! be expressed: [`AcceptedFill`] has no contract, leverage, or funding field.
//! Nothing is clamped.
//!
//! This type does not read ATP, Kelly, or [`crate::policy::AtpPolicy`].

use crate::audit::{AuditKind, AuditRecord};
use crate::error::LedgerError;
use crate::types::Side;
use crate::units::{Scale, checked_add, checked_sub};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An already-decided simulated fill. Quantity is not resized by policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedFill {
    pub fill_id: String,
    /// Caller-supplied order identity; non-empty when present, absent on legacy fills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
    /// Caller-supplied decision identity; non-empty when present, absent on legacy fills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    /// Injected event time (nanoseconds). Not wall clock.
    pub event_time_ns: u64,
    pub instrument: String,
    pub side: Side,
    pub price_ticks: i64,
    pub qty_lots: i64,
    /// Economic fee in quote subunits. Positive is a cost; negative is a rebate.
    pub fee_subunits: i64,
}

/// Non-economic order terminal / empty outcomes. No cash or PnL mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonFillEvent {
    pub event_time_ns: u64,
    pub kind: NonFillKind,
    pub order_id: String,
    pub instrument: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonFillKind {
    Reject,
    Cancel,
    NonFill,
}

impl NonFillKind {
    fn audit_kind(self) -> AuditKind {
        match self {
            Self::Reject => AuditKind::Reject,
            Self::Cancel => AuditKind::Cancel,
            Self::NonFill => AuditKind::NonFill,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FillFingerprint {
    order_id: Option<String>,
    decision_id: Option<String>,
    event_time_ns: u64,
    instrument: String,
    side: Side,
    price_ticks: i64,
    qty_lots: i64,
    fee_subunits: i64,
}

impl FillFingerprint {
    fn from_fill(fill: &AcceptedFill) -> Self {
        Self {
            order_id: fill.order_id.clone(),
            decision_id: fill.decision_id.clone(),
            event_time_ns: fill.event_time_ns,
            instrument: fill.instrument.clone(),
            side: fill.side,
            price_ticks: fill.price_ticks,
            qty_lots: fill.qty_lots,
            fee_subunits: fill.fee_subunits,
        }
    }
}

/// Deterministic single-instrument long/flat ledger.
#[derive(Debug, Clone)]
pub struct PaperLedger {
    instrument: String,
    scale: Scale,
    cash_subunits: i64,
    inventory_lots: i64,
    total_cost_subunits: i64,
    realized_pnl_subunits: i64,
    fees_subunits: i64,
    applied: BTreeMap<String, FillFingerprint>,
    audit: Vec<AuditRecord>,
}

/// Byte-stable economic snapshot (no ATP / Kelly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerSnapshot {
    pub instrument: String,
    pub price_scale: i64,
    pub qty_scale: i64,
    pub cash_subunits: i64,
    pub inventory_lots: i64,
    pub total_cost_subunits: i64,
    pub weighted_cost_basis_ticks: i64,
    pub realized_pnl_subunits: i64,
    pub fees_subunits: i64,
    pub unrealized_pnl_subunits: i64,
    pub audit: Vec<AuditRecord>,
}

impl PaperLedger {
    /// Empty long/flat book for `instrument` with starting cash.
    pub fn new(
        instrument: impl Into<String>,
        scale: Scale,
        cash_subunits: i64,
    ) -> Result<Self, LedgerError> {
        scale.validate()?;
        let instrument = instrument.into();
        if instrument.is_empty() {
            return Err(LedgerError::MalformedInput {
                reason: "instrument must be non-empty".into(),
            });
        }
        Ok(Self {
            instrument,
            scale,
            cash_subunits,
            inventory_lots: 0,
            total_cost_subunits: 0,
            realized_pnl_subunits: 0,
            fees_subunits: 0,
            applied: BTreeMap::new(),
            audit: Vec::new(),
        })
    }

    /// Restore a complete snapshot, including duplicate-fill protection.
    ///
    /// Replays the full audit in order and verifies every row and the final
    /// economic state. Missing or inconsistent history is rejected, not silently
    /// adopted without its fill identities. Snapshot fields and parity-v1 audit
    /// semantics are unchanged. The external unrealized-PnL mark is not restored.
    ///
    /// Persist [`Self::snapshot`] (or deserialize [`Self::snapshot_json`] into
    /// [`LedgerSnapshot`]) without pruning its audit before calling this method.
    /// The snapshot is consumed whether the restore succeeds or fails.
    pub fn from_snapshot(mut snapshot: LedgerSnapshot) -> Result<Self, LedgerError> {
        let scale = Scale {
            price_scale: snapshot.price_scale,
            qty_scale: snapshot.qty_scale,
        };
        let audit = std::mem::take(&mut snapshot.audit);
        // Starting cash is recovered from the first audit row during replay.
        // Only an empty audit adopts the snapshot cash directly.
        let initial_cash = if audit.is_empty() {
            snapshot.cash_subunits
        } else {
            0
        };
        let mut book = Self::new(&snapshot.instrument, scale, initial_cash)?;
        for (index, row) in audit.iter().enumerate() {
            Self::replay_audit_row(&mut book, row, index, scale)?;
            if book.audit.last() != Some(row) {
                return Err(LedgerError::MalformedInput {
                    reason: format!("snapshot audit row {index} does not match replay"),
                });
            }
        }
        snapshot.audit = audit;
        // Unrealized PnL is derived from an external mark, not committed state.
        snapshot.unrealized_pnl_subunits = 0;
        if book.snapshot(None)? != snapshot {
            return Err(LedgerError::MalformedInput {
                reason: "snapshot economic state does not match its complete audit".into(),
            });
        }
        Ok(book)
    }

    /// Replay one persisted audit row. The first row also recovers the
    /// starting cash that the snapshot does not record directly.
    fn replay_audit_row(
        book: &mut Self,
        row: &AuditRecord,
        index: usize,
        scale: Scale,
    ) -> Result<(), LedgerError> {
        match row.kind {
            AuditKind::Fill => {
                let fill = audit_row_fill(row, index)?;
                if index == 0 {
                    book.cash_subunits = initial_cash_after_fill(row, &fill, scale)?;
                }
                book.apply_fill(fill)?;
            }
            AuditKind::Reject | AuditKind::Cancel | AuditKind::NonFill => {
                if index == 0 {
                    book.cash_subunits = row.cash_after;
                }
                book.record_non_fill(audit_row_non_fill(row, index)?)?;
            }
        }
        Ok(())
    }

    pub fn instrument(&self) -> &str {
        &self.instrument
    }

    pub fn scale(&self) -> Scale {
        self.scale
    }

    pub fn cash_subunits(&self) -> i64 {
        self.cash_subunits
    }

    pub fn inventory_lots(&self) -> i64 {
        self.inventory_lots
    }

    pub fn total_cost_subunits(&self) -> i64 {
        self.total_cost_subunits
    }

    pub fn realized_pnl_subunits(&self) -> i64 {
        self.realized_pnl_subunits
    }

    pub fn fees_subunits(&self) -> i64 {
        self.fees_subunits
    }

    pub fn audit(&self) -> &[AuditRecord] {
        &self.audit
    }

    /// Floor average entry in ticks per asset unit; `0` when flat.
    ///
    /// `total_cost_subunits` is quote subunits for the whole position, and
    /// `inventory_lots` counts lots. Multiplying by `qty_scale` converts the
    /// ratio back to the same tick unit as `price_ticks`. Remainder stays in
    /// `total_cost_subunits`. Division is Euclidean (floor), including when a
    /// rebate makes the cost negative.
    ///
    /// The intermediate is computed in `i128`. When the floored result does not
    /// fit in `i64` this returns [`LedgerError::OutOfRange`]. It never saturates
    /// or clamps to `i64::MIN` / `i64::MAX`.
    pub fn checked_weighted_cost_basis_ticks(&self) -> Result<i64, LedgerError> {
        if self.inventory_lots == 0 {
            return Ok(0);
        }
        let numerator = (self.total_cost_subunits as i128) * (self.scale.qty_scale as i128);
        let basis = numerator.div_euclid(self.inventory_lots as i128);
        i64::try_from(basis).map_err(|_| LedgerError::OutOfRange {
            reason: format!("weighted_cost_basis_ticks {basis} does not fit i64"),
        })
    }

    /// Mark-to-market using an **external** mark. Not stored as ledger state.
    pub fn unrealized_pnl(&self, mark_price_ticks: i64) -> Result<i64, LedgerError> {
        if self.inventory_lots == 0 {
            return Ok(0);
        }
        if mark_price_ticks <= 0 {
            return Err(LedgerError::MalformedInput {
                reason: "mark_price_ticks must be positive".into(),
            });
        }
        let mark_value = self
            .scale
            .notional_subunits(mark_price_ticks, self.inventory_lots)?;
        checked_sub(mark_value, self.total_cost_subunits, "unrealized_pnl")
    }

    /// Canonical snapshot. `unrealized_pnl_subunits` is 0 unless `mark` is given.
    ///
    /// Fails with [`LedgerError::OutOfRange`] when a derived value (unrealized PnL
    /// or [`Self::checked_weighted_cost_basis_ticks`]) does not fit in `i64`, so a
    /// clamped number is never serialized or compared.
    pub fn snapshot(&self, mark_price_ticks: Option<i64>) -> Result<LedgerSnapshot, LedgerError> {
        let unrealized = match mark_price_ticks {
            Some(mark) => self.unrealized_pnl(mark)?,
            None => 0,
        };
        Ok(LedgerSnapshot {
            instrument: self.instrument.clone(),
            price_scale: self.scale.price_scale,
            qty_scale: self.scale.qty_scale,
            cash_subunits: self.cash_subunits,
            inventory_lots: self.inventory_lots,
            total_cost_subunits: self.total_cost_subunits,
            weighted_cost_basis_ticks: self.checked_weighted_cost_basis_ticks()?,
            realized_pnl_subunits: self.realized_pnl_subunits,
            fees_subunits: self.fees_subunits,
            unrealized_pnl_subunits: unrealized,
            audit: self.audit.clone(),
        })
    }

    /// Byte-stable JSON of [`Self::snapshot`].
    pub fn snapshot_json(&self, mark_price_ticks: Option<i64>) -> Result<String, LedgerError> {
        let snap = self.snapshot(mark_price_ticks)?;
        serde_json::to_string(&snap).map_err(|e| LedgerError::MalformedInput {
            reason: format!("snapshot serialize: {e}"),
        })
    }

    /// Record reject / cancel / non-fill. Inventory and realized PnL stay put.
    pub fn record_non_fill(&mut self, event: NonFillEvent) -> Result<(), LedgerError> {
        self.check_instrument(&event.instrument)?;
        if event.order_id.is_empty() {
            return Err(LedgerError::MalformedInput {
                reason: "order_id must be non-empty".into(),
            });
        }
        self.audit.push(AuditRecord {
            event_time_ns: event.event_time_ns,
            kind: event.kind.audit_kind(),
            fill_id: None,
            order_id: Some(event.order_id),
            decision_id: None,
            instrument: self.instrument.clone(),
            side: None,
            price_ticks: None,
            qty_lots: None,
            fee_subunits: None,
            realized_pnl_delta: 0,
            cash_after: self.cash_subunits,
            inventory_after: self.inventory_lots,
            total_cost_after: self.total_cost_subunits,
            reason: Some(event.reason),
            idempotent_replay: false,
        });
        Ok(())
    }

    /// Apply an accepted fill exactly once.
    ///
    /// Duplicate `fill_id` with the same payload is idempotent (no second debit).
    /// A conflicting payload is [`LedgerError::DuplicateFillConflict`].
    pub fn apply_fill(&mut self, fill: AcceptedFill) -> Result<ApplyOutcome, LedgerError> {
        self.validate_fill(&fill)?;
        if let Some(prev) = self.applied.get(&fill.fill_id) {
            let next = FillFingerprint::from_fill(&fill);
            if prev == &next {
                self.audit.push(self.replay_record(&fill));
                return Ok(ApplyOutcome::IdempotentReplay);
            }
            return Err(LedgerError::DuplicateFillConflict {
                fill_id: fill.fill_id,
            });
        }

        let planned = self.plan_fill(&fill)?;
        self.commit_fill(fill, planned)
    }

    fn check_instrument(&self, instrument: &str) -> Result<(), LedgerError> {
        if instrument != self.instrument {
            return Err(LedgerError::InstrumentMismatch {
                configured: self.instrument.clone(),
                requested: instrument.to_string(),
            });
        }
        Ok(())
    }

    fn validate_fill(&self, fill: &AcceptedFill) -> Result<(), LedgerError> {
        if fill.fill_id.is_empty() {
            return Err(LedgerError::MalformedInput {
                reason: "fill_id must be non-empty".into(),
            });
        }
        for (name, id) in [
            ("order_id", &fill.order_id),
            ("decision_id", &fill.decision_id),
        ] {
            if id.as_deref() == Some("") {
                return Err(LedgerError::MalformedInput {
                    reason: format!("{name} must be non-empty when provided"),
                });
            }
        }
        self.check_instrument(&fill.instrument)?;
        let _ = self
            .scale
            .notional_subunits(fill.price_ticks, fill.qty_lots)?;
        Ok(())
    }

    fn plan_fill(&self, fill: &AcceptedFill) -> Result<PlannedFill, LedgerError> {
        let notional = self
            .scale
            .notional_subunits(fill.price_ticks, fill.qty_lots)?;
        match fill.side {
            Side::Buy => {
                let debit = checked_add(notional, fill.fee_subunits, "buy debit")?;
                let cash = checked_sub(self.cash_subunits, debit, "cash")?;
                let inventory = checked_add(self.inventory_lots, fill.qty_lots, "inventory")?;
                let total_cost = checked_add(self.total_cost_subunits, debit, "total_cost")?;
                let fees = checked_add(self.fees_subunits, fill.fee_subunits, "fees")?;
                Ok(PlannedFill {
                    cash,
                    inventory,
                    total_cost,
                    realized_delta: 0,
                    realized_pnl: self.realized_pnl_subunits,
                    fees,
                })
            }
            Side::Sell => {
                if fill.qty_lots > self.inventory_lots {
                    return Err(LedgerError::UnsupportedShort {
                        instrument: self.instrument.clone(),
                        inventory_lots: self.inventory_lots,
                        sell_lots: fill.qty_lots,
                    });
                }
                let credit = checked_sub(notional, fill.fee_subunits, "sell credit")?;
                let cash = checked_add(self.cash_subunits, credit, "cash")?;
                let (inventory, total_cost, allocated) = if fill.qty_lots == self.inventory_lots {
                    (0, 0, self.total_cost_subunits)
                } else {
                    let allocated = allocate_cost(
                        self.total_cost_subunits,
                        fill.qty_lots,
                        self.inventory_lots,
                    )?;
                    let inventory = self.inventory_lots - fill.qty_lots;
                    let total_cost =
                        checked_sub(self.total_cost_subunits, allocated, "total_cost")?;
                    (inventory, total_cost, allocated)
                };
                let realized_delta = checked_sub(credit, allocated, "realized_delta")?;
                let realized_pnl =
                    checked_add(self.realized_pnl_subunits, realized_delta, "realized_pnl")?;
                let fees = checked_add(self.fees_subunits, fill.fee_subunits, "fees")?;
                Ok(PlannedFill {
                    cash,
                    inventory,
                    total_cost,
                    realized_delta,
                    realized_pnl,
                    fees,
                })
            }
        }
    }

    fn commit_fill(
        &mut self,
        fill: AcceptedFill,
        planned: PlannedFill,
    ) -> Result<ApplyOutcome, LedgerError> {
        self.cash_subunits = planned.cash;
        self.inventory_lots = planned.inventory;
        self.total_cost_subunits = planned.total_cost;
        self.realized_pnl_subunits = planned.realized_pnl;
        self.fees_subunits = planned.fees;
        self.applied
            .insert(fill.fill_id.clone(), FillFingerprint::from_fill(&fill));
        self.audit.push(AuditRecord {
            event_time_ns: fill.event_time_ns,
            kind: AuditKind::Fill,
            fill_id: Some(fill.fill_id),
            order_id: fill.order_id,
            decision_id: fill.decision_id,
            instrument: self.instrument.clone(),
            side: Some(fill.side),
            price_ticks: Some(fill.price_ticks),
            qty_lots: Some(fill.qty_lots),
            fee_subunits: Some(fill.fee_subunits),
            realized_pnl_delta: planned.realized_delta,
            cash_after: self.cash_subunits,
            inventory_after: self.inventory_lots,
            total_cost_after: self.total_cost_subunits,
            reason: None,
            idempotent_replay: false,
        });
        Ok(ApplyOutcome::Applied)
    }

    fn replay_record(&self, fill: &AcceptedFill) -> AuditRecord {
        AuditRecord {
            event_time_ns: fill.event_time_ns,
            kind: AuditKind::Fill,
            fill_id: Some(fill.fill_id.clone()),
            order_id: fill.order_id.clone(),
            decision_id: fill.decision_id.clone(),
            instrument: self.instrument.clone(),
            side: Some(fill.side),
            price_ticks: Some(fill.price_ticks),
            qty_lots: Some(fill.qty_lots),
            fee_subunits: Some(fill.fee_subunits),
            realized_pnl_delta: 0,
            cash_after: self.cash_subunits,
            inventory_after: self.inventory_lots,
            total_cost_after: self.total_cost_subunits,
            reason: Some("idempotent_replay".into()),
            idempotent_replay: true,
        }
    }
}

/// Result of [`PaperLedger::apply_fill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    IdempotentReplay,
}

struct PlannedFill {
    cash: i64,
    inventory: i64,
    total_cost: i64,
    realized_delta: i64,
    realized_pnl: i64,
    fees: i64,
}

fn missing_audit_field(index: usize, field: &str) -> LedgerError {
    LedgerError::MalformedInput {
        reason: format!("snapshot audit row {index} is missing required field '{field}'"),
    }
}

fn audit_row_fill(row: &AuditRecord, index: usize) -> Result<AcceptedFill, LedgerError> {
    Ok(AcceptedFill {
        fill_id: row
            .fill_id
            .clone()
            .ok_or_else(|| missing_audit_field(index, "fill_id"))?,
        order_id: row.order_id.clone(),
        decision_id: row.decision_id.clone(),
        event_time_ns: row.event_time_ns,
        instrument: row.instrument.clone(),
        side: row.side.ok_or_else(|| missing_audit_field(index, "side"))?,
        price_ticks: row
            .price_ticks
            .ok_or_else(|| missing_audit_field(index, "price_ticks"))?,
        qty_lots: row
            .qty_lots
            .ok_or_else(|| missing_audit_field(index, "qty_lots"))?,
        fee_subunits: row
            .fee_subunits
            .ok_or_else(|| missing_audit_field(index, "fee_subunits"))?,
    })
}

fn audit_row_non_fill(row: &AuditRecord, index: usize) -> Result<NonFillEvent, LedgerError> {
    let kind = match row.kind {
        AuditKind::Reject => NonFillKind::Reject,
        AuditKind::Cancel => NonFillKind::Cancel,
        _ => NonFillKind::NonFill,
    };
    Ok(NonFillEvent {
        event_time_ns: row.event_time_ns,
        kind,
        order_id: row
            .order_id
            .clone()
            .ok_or_else(|| missing_audit_field(index, "order_id"))?,
        instrument: row.instrument.clone(),
        reason: row
            .reason
            .clone()
            .ok_or_else(|| missing_audit_field(index, "reason"))?,
    })
}

/// Invert the first fill's committed cash delta to recover starting cash. The
/// same checked arithmetic was required when the fill originally committed.
fn initial_cash_after_fill(
    row: &AuditRecord,
    fill: &AcceptedFill,
    scale: Scale,
) -> Result<i64, LedgerError> {
    let notional = scale.notional_subunits(fill.price_ticks, fill.qty_lots)?;
    match fill.side {
        Side::Buy => checked_add(
            row.cash_after,
            checked_add(notional, fill.fee_subunits, "buy debit")?,
            "initial cash",
        ),
        Side::Sell => checked_sub(
            row.cash_after,
            checked_sub(notional, fill.fee_subunits, "sell credit")?,
            "initial cash",
        ),
    }
}

/// Truncate toward zero; the last lot closes with remaining `total_cost`.
fn allocate_cost(total_cost: i64, sell_lots: i64, inventory: i64) -> Result<i64, LedgerError> {
    if inventory <= 0 || sell_lots <= 0 {
        return Err(LedgerError::MalformedInput {
            reason: "allocate_cost requires positive inventory and sell qty".into(),
        });
    }
    let allocated = (total_cost as i128)
        .checked_mul(sell_lots as i128)
        .ok_or_else(|| LedgerError::OutOfRange {
            reason: "cost allocation overflowed".into(),
        })?
        / (inventory as i128);
    i64::try_from(allocated).map_err(|_| LedgerError::OutOfRange {
        reason: "allocated cost does not fit i64".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> PaperLedger {
        PaperLedger::new("ASSET_A", Scale::CENTS_WHOLE_LOTS, 100_000).unwrap()
    }

    fn buy(id: &str, t: u64, px: i64, qty: i64, fee: i64) -> AcceptedFill {
        AcceptedFill {
            fill_id: id.into(),
            order_id: None,
            decision_id: None,
            event_time_ns: t,
            instrument: "ASSET_A".into(),
            side: Side::Buy,
            price_ticks: px,
            qty_lots: qty,
            fee_subunits: fee,
        }
    }

    fn sell(id: &str, t: u64, px: i64, qty: i64, fee: i64) -> AcceptedFill {
        AcceptedFill {
            fill_id: id.into(),
            order_id: None,
            decision_id: None,
            event_time_ns: t,
            instrument: "ASSET_A".into(),
            side: Side::Sell,
            price_ticks: px,
            qty_lots: qty,
            fee_subunits: fee,
        }
    }

    /// An idempotent replay appends exactly one row and changes no economics.
    fn assert_replay_only_appends_row(before: &LedgerSnapshot, after: LedgerSnapshot) {
        let mut state = after;
        let row = state.audit.pop().expect("replay appends one row");
        assert!(row.idempotent_replay);
        assert_eq!(state, *before);
    }

    fn assert_restore_fails(snapshot: LedgerSnapshot) {
        assert!(PaperLedger::from_snapshot(snapshot).is_err());
    }

    #[test]
    fn buy_partial_full_close_matches_hand_math() {
        let mut book = ledger();
        book.apply_fill(buy("f1", 1, 1000, 10, 5)).unwrap();
        assert_eq!(book.cash_subunits(), 89_995);
        assert_eq!(book.inventory_lots(), 10);
        assert_eq!(book.total_cost_subunits(), 10_005);
        assert_eq!(book.realized_pnl_subunits(), 0);

        book.apply_fill(sell("f2", 2, 1100, 4, 4)).unwrap();
        assert_eq!(book.cash_subunits(), 94_391);
        assert_eq!(book.inventory_lots(), 6);
        assert_eq!(book.total_cost_subunits(), 6_003);
        assert_eq!(book.realized_pnl_subunits(), 394);

        book.apply_fill(sell("f3", 3, 1100, 6, 6)).unwrap();
        assert_eq!(book.cash_subunits(), 100_985);
        assert_eq!(book.inventory_lots(), 0);
        assert_eq!(book.total_cost_subunits(), 0);
        assert_eq!(book.realized_pnl_subunits(), 985);
        assert_eq!(book.fees_subunits(), 15);
    }

    #[test]
    fn short_and_over_sell_are_typed_errors() {
        let mut book = ledger();
        let err = book.apply_fill(sell("s1", 1, 1000, 1, 0)).unwrap_err();
        assert_eq!(err.type_name(), "unsupported_short");
        assert_eq!(book.cash_subunits(), 100_000);

        book.apply_fill(buy("b1", 2, 1000, 2, 0)).unwrap();
        let err = book.apply_fill(sell("s2", 3, 1000, 3, 0)).unwrap_err();
        assert!(matches!(
            err,
            LedgerError::UnsupportedShort {
                sell_lots: 3,
                inventory_lots: 2,
                ..
            }
        ));
        assert_eq!(book.inventory_lots(), 2);
    }

    #[test]
    fn duplicate_fill_is_idempotent_not_a_second_debit() {
        let mut book = ledger();
        let f = buy("dup", 10, 1000, 10, 5);
        assert_eq!(book.apply_fill(f.clone()).unwrap(), ApplyOutcome::Applied);
        assert_eq!(book.apply_fill(f).unwrap(), ApplyOutcome::IdempotentReplay);
        assert_eq!(book.cash_subunits(), 89_995);
        assert_eq!(book.fees_subunits(), 5);
        assert_eq!(
            book.audit().iter().filter(|r| r.idempotent_replay).count(),
            1
        );

        let mut conflict = buy("dup", 10, 1000, 10, 5);
        conflict.fee_subunits = 6;
        assert!(matches!(
            book.apply_fill(conflict),
            Err(LedgerError::DuplicateFillConflict { .. })
        ));
        assert_eq!(book.fees_subunits(), 5);
    }

    #[test]
    fn non_fills_have_zero_pnl() {
        let mut book = ledger();
        book.record_non_fill(NonFillEvent {
            event_time_ns: 1,
            kind: NonFillKind::Reject,
            order_id: "o1".into(),
            instrument: "ASSET_A".into(),
            reason: "no_liquidity".into(),
        })
        .unwrap();
        book.record_non_fill(NonFillEvent {
            event_time_ns: 2,
            kind: NonFillKind::Cancel,
            order_id: "o2".into(),
            instrument: "ASSET_A".into(),
            reason: "user_cancel".into(),
        })
        .unwrap();
        book.record_non_fill(NonFillEvent {
            event_time_ns: 3,
            kind: NonFillKind::NonFill,
            order_id: "o3".into(),
            instrument: "ASSET_A".into(),
            reason: "passive_resting".into(),
        })
        .unwrap();
        assert_eq!(book.realized_pnl_subunits(), 0);
        assert_eq!(book.cash_subunits(), 100_000);
        assert_eq!(book.inventory_lots(), 0);
        assert_eq!(book.audit().len(), 3);
        assert!(book.audit().iter().all(|r| r.realized_pnl_delta == 0));
    }

    #[test]
    fn snapshot_is_byte_stable() {
        let mut book = ledger();
        book.apply_fill(buy("f1", 1, 1000, 10, 5)).unwrap();
        let a = book.snapshot_json(Some(1100)).unwrap();
        let b = book.snapshot_json(Some(1100)).unwrap();
        assert_eq!(a, b);
        let snap: LedgerSnapshot = serde_json::from_str(&a).unwrap();
        assert_eq!(snap.unrealized_pnl_subunits, 1100 * 10 - 10_005);
    }

    #[test]
    fn failed_validation_does_not_mutate() {
        let mut book = ledger();
        let before = book.snapshot_json(None).unwrap();
        let mut bad = buy("x", 1, 1000, 1, 0);
        bad.instrument = "OTHER".into();
        assert!(book.apply_fill(bad).is_err());
        assert_eq!(book.snapshot_json(None).unwrap(), before);
    }

    #[test]
    fn basis_is_ticks_per_asset_unit_when_qty_scale_exceeds_one() {
        let mut book = PaperLedger::new(
            "ASSET_A",
            Scale {
                price_scale: 100,
                qty_scale: 100,
            },
            100_000,
        )
        .unwrap();
        book.apply_fill(AcceptedFill {
            fill_id: "frac".into(),
            order_id: None,
            decision_id: None,
            event_time_ns: 1,
            instrument: "ASSET_A".into(),
            side: Side::Buy,
            price_ticks: 1000,
            qty_lots: 100,
            fee_subunits: 0,
        })
        .unwrap();
        assert_eq!(book.total_cost_subunits(), 1_000);
        assert_eq!(book.checked_weighted_cost_basis_ticks(), Ok(1_000));
    }

    #[test]
    fn negative_cost_basis_floors() {
        let mut book = ledger();
        book.apply_fill(buy("neg", 1, 1, 2, -3)).unwrap();
        assert_eq!(book.total_cost_subunits(), -1);
        assert_eq!(book.checked_weighted_cost_basis_ticks(), Ok(-1));
    }

    /// Book whose cost basis overflows `i64` once multiplied by `qty_scale`.
    /// `qty_scale = 2`, one lot at 2 ticks (notional 1), and a fee chosen so
    /// `total_cost * 2` lies outside `i64`. Every individual fill step fits.
    fn book_with_basis_fee(fee: i64) -> PaperLedger {
        let mut book = PaperLedger::new(
            "ASSET_A",
            Scale {
                price_scale: 100,
                qty_scale: 2,
            },
            100_000,
        )
        .unwrap();
        book.apply_fill(buy("huge", 1, 2, 1, fee)).unwrap();
        book
    }

    #[test]
    fn cost_basis_above_i64_max_is_out_of_range() {
        let book = book_with_basis_fee(i64::MAX / 2);
        assert_eq!(book.total_cost_subunits(), i64::MAX / 2 + 1);
        let err = book.checked_weighted_cost_basis_ticks().unwrap_err();
        assert!(matches!(err, LedgerError::OutOfRange { .. }), "{err:?}");
        let err = book.snapshot(None).unwrap_err();
        assert!(matches!(err, LedgerError::OutOfRange { .. }), "{err:?}");
        assert!(book.snapshot_json(None).is_err());
    }

    #[test]
    fn cost_basis_below_i64_min_is_out_of_range() {
        let book = book_with_basis_fee(-(i64::MAX / 2) - 10);
        assert!(book.total_cost_subunits() < i64::MIN / 2);
        let err = book.checked_weighted_cost_basis_ticks().unwrap_err();
        assert!(matches!(err, LedgerError::OutOfRange { .. }), "{err:?}");
        assert!(matches!(
            book.snapshot(None),
            Err(LedgerError::OutOfRange { .. })
        ));
    }

    #[test]
    fn cost_basis_at_i64_boundary_is_exact() {
        // total_cost * 2 == i64::MAX - 1 fits and is returned unchanged.
        let book = book_with_basis_fee(i64::MAX / 2 - 1);
        assert_eq!(book.total_cost_subunits(), i64::MAX / 2);
        assert_eq!(book.checked_weighted_cost_basis_ticks(), Ok(i64::MAX - 1));
    }

    #[test]
    fn buy_rebate_larger_than_notional_increases_cash() {
        let mut book = ledger();
        book.apply_fill(buy("rebate-over", 1, 1000, 10, -15_000))
            .unwrap();
        assert_eq!(book.cash_subunits(), 105_000);
        assert_eq!(book.inventory_lots(), 10);
        assert_eq!(book.total_cost_subunits(), -5_000);
        assert_eq!(book.fees_subunits(), -15_000);
        assert_eq!(book.realized_pnl_subunits(), 0);
    }

    #[test]
    fn restored_ledger_replays_already_applied_fill() {
        let mut book = ledger();
        let mut first = buy("r1", 1, 1000, 10, 5);
        first.order_id = Some("ord-1".into());
        first.decision_id = Some("dec-1".into());
        book.apply_fill(first.clone()).unwrap();
        book.record_non_fill(NonFillEvent {
            event_time_ns: 2,
            kind: NonFillKind::Cancel,
            order_id: "ord-2".into(),
            instrument: "ASSET_A".into(),
            reason: "user_cancel".into(),
        })
        .unwrap();
        book.apply_fill(sell("r2", 3, 1100, 4, 4)).unwrap();

        let json = book.snapshot_json(Some(1200)).unwrap();
        let snap: LedgerSnapshot = serde_json::from_str(&json).unwrap();
        let mut restored = PaperLedger::from_snapshot(snap).unwrap();
        assert_eq!(restored.snapshot_json(None), book.snapshot_json(None));
        assert_eq!(restored.audit().len(), 3);

        let before = restored.snapshot(None).unwrap();
        assert_eq!(
            restored.apply_fill(first.clone()).unwrap(),
            ApplyOutcome::IdempotentReplay
        );
        assert_eq!(
            book.apply_fill(first).unwrap(),
            ApplyOutcome::IdempotentReplay
        );
        // parity-v1: a replay appends exactly one replay row, same as before restart.
        assert_replay_only_appends_row(&before, restored.snapshot(None).unwrap());
        assert_eq!(restored.snapshot_json(None), book.snapshot_json(None));

        let conflict = sell("r2", 3, 1100, 4, 5);
        let pre_conflict = restored.snapshot_json(None).unwrap();
        assert!(matches!(
            restored.apply_fill(conflict),
            Err(LedgerError::DuplicateFillConflict { .. })
        ));
        assert_eq!(restored.snapshot_json(None).unwrap(), pre_conflict);
    }

    #[test]
    fn new_fill_after_restore_applies_normally() {
        let mut book = ledger();
        book.apply_fill(buy("n1", 1, 1000, 10, 5)).unwrap();
        let mut restored = PaperLedger::from_snapshot(book.snapshot(None).unwrap()).unwrap();
        let next = sell("n2", 2, 1100, 4, 4);
        assert_eq!(
            restored.apply_fill(next.clone()).unwrap(),
            ApplyOutcome::Applied
        );
        book.apply_fill(next).unwrap();
        assert_eq!(restored.snapshot_json(None), book.snapshot_json(None));
    }

    #[test]
    fn restore_handles_empty_and_non_fill_only_books() {
        let empty = ledger();
        let restored = PaperLedger::from_snapshot(empty.snapshot(None).unwrap()).unwrap();
        assert_eq!(restored.snapshot_json(None), empty.snapshot_json(None));

        let mut book = ledger();
        book.record_non_fill(NonFillEvent {
            event_time_ns: 1,
            kind: NonFillKind::Reject,
            order_id: "o1".into(),
            instrument: "ASSET_A".into(),
            reason: "no_liquidity".into(),
        })
        .unwrap();
        let restored = PaperLedger::from_snapshot(book.snapshot(None).unwrap()).unwrap();
        assert_eq!(restored.snapshot_json(None), book.snapshot_json(None));
    }

    #[test]
    fn restore_rejects_inconsistent_or_pruned_snapshots() {
        let mut book = ledger();
        book.apply_fill(buy("p1", 1, 1000, 10, 5)).unwrap();
        book.apply_fill(buy("p2", 2, 1000, 1, 0)).unwrap();
        let good = book.snapshot(None).unwrap();

        let mut pruned = good.clone();
        pruned.audit.remove(0);
        assert_restore_fails(pruned);

        let mut cleared = good.clone();
        cleared.audit.clear();
        assert_restore_fails(cleared);

        let mut cash = good.clone();
        cash.cash_subunits += 1;
        assert_restore_fails(cash);

        let mut row = good.clone();
        row.audit[1].cash_after += 1;
        assert_restore_fails(row);

        let mut missing = good;
        missing.audit[0].side = None;
        assert!(matches!(
            PaperLedger::from_snapshot(missing),
            Err(LedgerError::MalformedInput { .. })
        ));
    }

    #[test]
    fn other_instrument_is_rejected() {
        let mut book = ledger();
        let err = book
            .apply_fill(AcceptedFill {
                fill_id: "x".into(),
                order_id: None,
                decision_id: None,
                event_time_ns: 1,
                instrument: "ASSET_B".into(),
                side: Side::Buy,
                price_ticks: 1,
                qty_lots: 1,
                fee_subunits: 0,
            })
            .unwrap_err();
        assert!(matches!(err, LedgerError::InstrumentMismatch { .. }));
    }
}

// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Exact-fill paper ledger for the declared M2 accounting subset.
//!
//! Supported: one instrument, long or flat inventory (`inventory_lots >= 0`).
//! Buys open or add. Sells reduce or close and must not exceed holdings.
//! Anything else — shorts, sells beyond inventory, a second instrument,
//! derivatives — is a typed [`LedgerError`], never a clamp.
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

    /// Floor average entry in ticks; `0` when flat. Remainder lives in `total_cost`.
    pub fn weighted_cost_basis_ticks(&self) -> i64 {
        if self.inventory_lots == 0 {
            0
        } else {
            self.total_cost_subunits / self.inventory_lots
        }
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
            weighted_cost_basis_ticks: self.weighted_cost_basis_ticks(),
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
            order_id: None,
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
            order_id: None,
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
            event_time_ns: t,
            instrument: "ASSET_A".into(),
            side: Side::Sell,
            price_ticks: px,
            qty_lots: qty,
            fee_subunits: fee,
        }
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
    fn other_instrument_is_rejected() {
        let mut book = ledger();
        let err = book
            .apply_fill(AcceptedFill {
                fill_id: "x".into(),
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

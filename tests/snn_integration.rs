// Copyright 2026 Limen-Neural
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Maps one synthetic Spikenaut-SNN-style decision onto `AcceptedFill` and
//! through `Ledger::apply_fill`, asserting the committed audit row carries
//! the caller-supplied identities. The decision type is local to this test —
//! Spikenaut-SNN and Limen-Capital are not dependencies. See
//! `docs/integration-surface.md` (RM-2535).

use metabolic_ledger::audit::AuditKind;
use metabolic_ledger::{AcceptedFill, ApplyOutcome, Ledger, Scale, Side};

/// A synthetic upstream decision. The real decision type lives in
/// rmems/Spikenaut-SNN; this crate only needs the already-decided fields.
struct SnnDecision {
    decision_id: String,
    event_time_ns: u64,
    instrument: String,
    side: Side,
    price_ticks: i64,
    qty_lots: i64,
    fee_subunits: i64,
}

/// Map a decided fill onto the ledger input. The ledger never mints or
/// infers identities: `fill_id` and `order_id` come from the consumer
/// (e.g. Limen-Capital's `client_order_id`), `decision_id` from the decision.
fn accepted_fill(decision: &SnnDecision, fill_id: &str, order_id: Option<String>) -> AcceptedFill {
    AcceptedFill {
        fill_id: fill_id.into(),
        order_id,
        decision_id: Some(decision.decision_id.clone()),
        event_time_ns: decision.event_time_ns,
        instrument: decision.instrument.clone(),
        side: decision.side,
        price_ticks: decision.price_ticks,
        qty_lots: decision.qty_lots,
        fee_subunits: decision.fee_subunits,
    }
}

fn snn_buy() -> SnnDecision {
    SnnDecision {
        decision_id: "snn-dec-42".into(),
        event_time_ns: 1_700_000_000_000_000_000,
        instrument: "ASSET_A".into(),
        side: Side::Buy,
        price_ticks: 1000,
        qty_lots: 10,
        fee_subunits: 5,
    }
}

fn book() -> Ledger {
    Ledger::new("ASSET_A", Scale::CENTS_WHOLE_LOTS, 100_000).unwrap()
}

#[test]
fn decision_with_order_id_reaches_the_audit_row_verbatim() {
    let mut book = book();
    assert_eq!(
        book.apply_fill(accepted_fill(
            &snn_buy(),
            "fill-0001",
            Some("lc-order-7".into())
        ))
        .unwrap(),
        ApplyOutcome::Applied
    );

    let [row] = book.audit() else {
        panic!("expected one audit row");
    };
    assert_eq!(row.kind, AuditKind::Fill);
    assert_eq!(row.fill_id.as_deref(), Some("fill-0001"));
    assert_eq!(row.order_id.as_deref(), Some("lc-order-7"));
    assert_eq!(row.decision_id.as_deref(), Some("snn-dec-42"));
    assert!(!row.idempotent_replay);
}

#[test]
fn decision_without_order_id_leaves_the_audit_slot_none() {
    let mut book = book();
    assert_eq!(
        book.apply_fill(accepted_fill(&snn_buy(), "fill-0002", None))
            .unwrap(),
        ApplyOutcome::Applied
    );

    let [row] = book.audit() else {
        panic!("expected one audit row");
    };
    assert_eq!(row.order_id, None);
    assert_eq!(row.decision_id.as_deref(), Some("snn-dec-42"));
}

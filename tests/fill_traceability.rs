// Copyright 2025 Limen-Neural
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use metabolic_ledger::audit::AuditRecord;
use metabolic_ledger::paper::LedgerSnapshot;
use metabolic_ledger::parity::{ParityFixture, load_fixture, run_fixture, shipped_fixture_dir};
use metabolic_ledger::{AcceptedFill, ApplyOutcome, LedgerError, PaperLedger, Scale};
use serde_json::{Value, json};

fn legacy_fill() -> Value {
    json!({
        "fill_id": "f1",
        "event_time_ns": 1,
        "instrument": "ASSET_A",
        "side": "buy",
        "price_ticks": 1000,
        "qty_lots": 10,
        "fee_subunits": 5
    })
}

fn book() -> PaperLedger {
    PaperLedger::new("ASSET_A", Scale::CENTS_WHOLE_LOTS, 100_000).unwrap()
}

#[test]
fn optional_identities_round_trip_through_commit_replay_and_snapshot() {
    for order_id in [None, Some("order-1".to_string())] {
        for decision_id in [None, Some("decision-1".to_string())] {
            let mut fill: AcceptedFill = serde_json::from_value(legacy_fill()).unwrap();
            fill.order_id = order_id.clone();
            fill.decision_id = decision_id.clone();
            let fill: AcceptedFill =
                serde_json::from_str(&serde_json::to_string(&fill).unwrap()).unwrap();
            let mut book = book();
            assert_eq!(
                book.apply_fill(fill.clone()).unwrap(),
                ApplyOutcome::Applied
            );
            assert_eq!(
                book.apply_fill(fill).unwrap(),
                ApplyOutcome::IdempotentReplay
            );
            assert_eq!(book.cash_subunits(), 89_995);
            assert_eq!(book.inventory_lots(), 10);
            assert_eq!(book.total_cost_subunits(), 10_005);
            assert_eq!(book.fees_subunits(), 5);
            let snapshot: LedgerSnapshot =
                serde_json::from_str(&book.snapshot_json(None).unwrap()).unwrap();
            assert_eq!(snapshot.audit, book.audit());
            assert_eq!(snapshot.audit.len(), 2);
            for (i, row) in snapshot.audit.iter().enumerate() {
                assert_eq!(row.fill_id.as_deref(), Some("f1"));
                assert_eq!(row.order_id, order_id);
                assert_eq!(row.decision_id, decision_id);
                assert_eq!(row.idempotent_replay, i == 1);
                assert_eq!(row.realized_pnl_delta, 0);
            }
        }
    }
}

#[test]
fn changing_adding_or_removing_either_identity_conflicts_without_mutation() {
    for field in ["order_id", "decision_id"] {
        for (before, after) in [
            (Value::Null, json!("id-1")),
            (json!("id-1"), Value::Null),
            (json!("id-1"), json!("id-2")),
        ] {
            let mut input = legacy_fill();
            input[field] = before;
            let mut book = book();
            book.apply_fill(serde_json::from_value(input.clone()).unwrap())
                .unwrap();
            let snapshot = book.snapshot_json(None).unwrap();
            input[field] = after;
            assert!(matches!(
                book.apply_fill(serde_json::from_value(input).unwrap()),
                Err(LedgerError::DuplicateFillConflict { .. })
            ));
            assert_eq!(book.snapshot_json(None).unwrap(), snapshot);
        }
    }
}

#[test]
fn legacy_fill_and_audit_json_keep_their_shape() {
    let legacy = legacy_fill();
    let fill: AcceptedFill = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(fill.order_id, None);
    assert_eq!(fill.decision_id, None);
    assert_eq!(serde_json::to_value(&fill).unwrap(), legacy);
    let mut explicit_nulls = legacy;
    explicit_nulls["order_id"] = Value::Null;
    explicit_nulls["decision_id"] = Value::Null;
    assert_eq!(
        serde_json::from_value::<AcceptedFill>(explicit_nulls).unwrap(),
        fill
    );

    let mut book = book();
    book.apply_fill(fill.clone()).unwrap();
    book.apply_fill(fill).unwrap();
    // Literal pre-extension canonical rows, including field order and null slots.
    let committed = r#"{"event_time_ns":1,"kind":"fill","fill_id":"f1","order_id":null,"instrument":"ASSET_A","side":"buy","price_ticks":1000,"qty_lots":10,"fee_subunits":5,"realized_pnl_delta":0,"cash_after":89995,"inventory_after":10,"total_cost_after":10005,"reason":null,"idempotent_replay":false}"#;
    let replay = r#"{"event_time_ns":1,"kind":"fill","fill_id":"f1","order_id":null,"instrument":"ASSET_A","side":"buy","price_ticks":1000,"qty_lots":10,"fee_subunits":5,"realized_pnl_delta":0,"cash_after":89995,"inventory_after":10,"total_cost_after":10005,"reason":"idempotent_replay","idempotent_replay":true}"#;
    for (row, expected) in book.audit().iter().zip([committed, replay]) {
        assert_eq!(row.order_id, None);
        assert_eq!(row.decision_id, None);
        assert_eq!(serde_json::to_string(row).unwrap(), expected);
        assert_eq!(&serde_json::from_str::<AuditRecord>(expected).unwrap(), row);
    }
}

#[test]
fn shipped_v1_traceability_fields_remain_absent_when_reserialized() {
    for entry in std::fs::read_dir(shipped_fixture_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let fixture = load_fixture(&path).unwrap();
        let serialized = serde_json::to_value(&fixture).unwrap();
        assert_eq!(serialized["events"], raw["events"], "{}", path.display());
        if raw["expected"].get("audit").is_some() {
            assert_eq!(
                serialized["expected"]["audit"],
                raw["expected"]["audit"],
                "{}",
                path.display()
            );
        }
    }
}

#[test]
fn parity_replays_additive_traceability_without_changing_shipped_fixtures() {
    let path = shipped_fixture_dir().join("buy.json");
    let mut raw: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    raw["events"][0]["order_id"] = json!("order-1");
    raw["events"][0]["decision_id"] = json!("decision-1");
    raw["expected"]["audit"][0]["order_id"] = json!("order-1");
    raw["expected"]["audit"][0]["decision_id"] = json!("decision-1");
    let replay_event = raw["events"][0].clone();
    raw["events"].as_array_mut().unwrap().push(replay_event);
    let mut replay_row = raw["expected"]["audit"][0].clone();
    replay_row["idempotent_replay"] = json!(true);
    replay_row["reason"] = json!("idempotent_replay");
    replay_row["realized_pnl_delta"] = json!(0);
    raw["expected"]["audit"]
        .as_array_mut()
        .unwrap()
        .push(replay_row);
    let fixture: ParityFixture = serde_json::from_value(raw).unwrap();
    run_fixture(&fixture).unwrap();
}

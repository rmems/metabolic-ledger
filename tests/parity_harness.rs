//! Offline golden-fixture harness for `metabolic-ledger.parity.v1`.
//!
//! Julia-emitted documents (RM-1864) can be dropped next to the hand-verified
//! files in `fixtures/v1/` without changing this test.

use metabolic_ledger::parity::{load_fixture, run_fixture, shipped_fixture_dir};
use metabolic_ledger::{AcceptedFill, PaperLedger, Scale, Side};
use std::fs;

#[test]
fn all_shipped_v1_fixtures_reconcile() {
    let dir = shipped_fixture_dir();
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir).expect("fixtures/v1 must exist") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        names.push(path);
    }
    names.sort();
    assert!(
        names.len() >= 8,
        "expected the hand-verified v1 set, found {}",
        names.len()
    );

    for path in names {
        let fx = load_fixture(&path).unwrap_or_else(|e| panic!("load {}: {e}", path.display()));
        run_fixture(&fx).unwrap_or_else(|e| panic!("parity {}: {e}", fx.name));
    }
}

#[test]
fn same_fixture_is_byte_stable_across_replays() {
    let path = shipped_fixture_dir().join("full_close.json");
    let fx = load_fixture(&path).unwrap();
    let mut snaps = Vec::new();
    for _ in 0..3 {
        let mut book = PaperLedger::new(
            &fx.units.instrument,
            scale_of(&fx),
            fx.initial.cash_subunits,
        )
        .unwrap();
        for event in &fx.events {
            if let metabolic_ledger::parity::FixtureEvent::Fill {
                fill_id,
                order_id,
                decision_id,
                event_time_ns,
                instrument,
                side,
                price_ticks,
                qty_lots,
                fee_subunits,
            } = event
            {
                book.apply_fill(AcceptedFill {
                    fill_id: fill_id.clone(),
                    order_id: order_id.clone(),
                    decision_id: decision_id.clone(),
                    event_time_ns: *event_time_ns,
                    instrument: instrument.clone(),
                    side: *side,
                    price_ticks: *price_ticks,
                    qty_lots: *qty_lots,
                    fee_subunits: *fee_subunits,
                })
                .unwrap();
            }
        }
        snaps.push(book.snapshot_json(None).unwrap());
    }
    assert_eq!(snaps[0], snaps[1]);
    assert_eq!(snaps[1], snaps[2]);
}

#[test]
fn fees_and_qty_applied_once_including_duplicate_id() {
    let mut book = PaperLedger::new("ASSET_A", Scale::CENTS_WHOLE_LOTS, 100_000).unwrap();
    let fill = AcceptedFill {
        fill_id: "once".into(),
        order_id: None,
        decision_id: None,
        event_time_ns: 1,
        instrument: "ASSET_A".into(),
        side: Side::Buy,
        price_ticks: 1000,
        qty_lots: 10,
        fee_subunits: 5,
    };
    book.apply_fill(fill.clone()).unwrap();
    book.apply_fill(fill).unwrap();
    assert_eq!(book.fees_subunits(), 5);
    assert_eq!(book.cash_subunits(), 89_995);
    assert_eq!(book.inventory_lots(), 10);
}

#[test]
fn realized_pnl_never_reads_atp_or_kelly() {
    use metabolic_ledger::{AtpPolicy, GhostWallet};

    let mut wallet = GhostWallet::new();
    wallet.balance_atp = 1.0;
    let policy = AtpPolicy {
        energy_commitment: 0.99,
        metabolic_cost: 0.5,
    };
    let proposal = policy.propose_buy_spend(wallet.balance_atp);
    assert!((proposal - 0.99).abs() < 1e-6);

    let mut book = PaperLedger::new("ASSET_A", Scale::CENTS_WHOLE_LOTS, 100_000).unwrap();
    book.apply_fill(AcceptedFill {
        fill_id: "econ".into(),
        order_id: None,
        decision_id: None,
        event_time_ns: 1,
        instrument: "ASSET_A".into(),
        side: Side::Buy,
        price_ticks: 1000,
        qty_lots: 10,
        fee_subunits: 5,
    })
    .unwrap();
    book.apply_fill(AcceptedFill {
        fill_id: "econ-sell".into(),
        order_id: None,
        decision_id: None,
        event_time_ns: 2,
        instrument: "ASSET_A".into(),
        side: Side::Sell,
        price_ticks: 1100,
        qty_lots: 10,
        fee_subunits: 0,
    })
    .unwrap();

    wallet.balance_atp = 0.0;
    assert_eq!(book.realized_pnl_subunits(), 11000 - 10005);
    assert_eq!(book.realized_pnl_subunits(), 995);
}

#[test]
fn every_shipped_fixture_survives_snapshot_restore() {
    use metabolic_ledger::LedgerSnapshot;

    for path in shipped_fixture_paths() {
        let fx = load_fixture(&path).unwrap();
        let (book, fills, rejected) = replay_fixture(&fx);

        let json = book.snapshot_json(fx.mark_price_ticks).unwrap();
        let snap: LedgerSnapshot = serde_json::from_str(&json).unwrap();
        let mut restored =
            PaperLedger::from_snapshot(snap).unwrap_or_else(|e| panic!("restore {}: {e}", fx.name));
        assert_eq!(
            restored.snapshot_json(fx.mark_price_ticks).unwrap(),
            json,
            "{}",
            fx.name
        );
        for fill in fills {
            assert_replays_idempotently(&mut restored, fill, &fx.name);
        }
        for (fill, type_name) in rejected {
            let err = restored.apply_fill(fill).unwrap_err();
            assert_eq!(err.type_name(), type_name, "{}", fx.name);
        }
    }
}

fn shipped_fixture_paths() -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(shipped_fixture_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
}

enum FixtureStep {
    Fill(AcceptedFill),
    NonFill(metabolic_ledger::NonFillEvent),
}

fn fixture_step(event: &metabolic_ledger::parity::FixtureEvent) -> FixtureStep {
    use metabolic_ledger::parity::FixtureEvent;
    use metabolic_ledger::{NonFillEvent, NonFillKind};

    let non_fill =
        |kind, order_id: &String, event_time_ns: &u64, instrument: &String, reason: &String| {
            FixtureStep::NonFill(NonFillEvent {
                event_time_ns: *event_time_ns,
                kind,
                order_id: order_id.clone(),
                instrument: instrument.clone(),
                reason: reason.clone(),
            })
        };
    match event {
        FixtureEvent::Fill {
            fill_id,
            order_id,
            decision_id,
            event_time_ns,
            instrument,
            side,
            price_ticks,
            qty_lots,
            fee_subunits,
        } => FixtureStep::Fill(AcceptedFill {
            fill_id: fill_id.clone(),
            order_id: order_id.clone(),
            decision_id: decision_id.clone(),
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
        } => non_fill(
            NonFillKind::Reject,
            order_id,
            event_time_ns,
            instrument,
            reason,
        ),
        FixtureEvent::Cancel {
            order_id,
            event_time_ns,
            instrument,
            reason,
        } => non_fill(
            NonFillKind::Cancel,
            order_id,
            event_time_ns,
            instrument,
            reason,
        ),
        FixtureEvent::NonFill {
            order_id,
            event_time_ns,
            instrument,
            reason,
        } => non_fill(
            NonFillKind::NonFill,
            order_id,
            event_time_ns,
            instrument,
            reason,
        ),
    }
}

/// Replay fixture events, returning the book plus the fills that committed
/// (for idempotent-replay checks) and the fills that errored (with their
/// `type_name`, for post-restore error checks).
fn replay_fixture(
    fx: &metabolic_ledger::parity::ParityFixture,
) -> (
    PaperLedger,
    Vec<AcceptedFill>,
    Vec<(AcceptedFill, &'static str)>,
) {
    let mut book =
        PaperLedger::new(&fx.units.instrument, scale_of(fx), fx.initial.cash_subunits).unwrap();
    let mut fills = Vec::new();
    let mut rejected = Vec::new();
    for event in &fx.events {
        match fixture_step(event) {
            FixtureStep::Fill(fill) => match book.apply_fill(fill.clone()) {
                Ok(_) => fills.push(fill),
                Err(e) => rejected.push((fill, e.type_name())),
            },
            FixtureStep::NonFill(event) => book
                .record_non_fill(event)
                .unwrap_or_else(|e| panic!("{}: {e}", fx.name)),
        }
    }
    (book, fills, rejected)
}

/// A fill committed before the snapshot must replay idempotently after
/// restore: one audit row, zero economic change.
fn assert_replays_idempotently(book: &mut PaperLedger, fill: AcceptedFill, name: &str) {
    use metabolic_ledger::ApplyOutcome;
    use metabolic_ledger::LedgerSnapshot;

    let before: LedgerSnapshot = book.snapshot(None).unwrap();
    assert_eq!(
        book.apply_fill(fill).unwrap(),
        ApplyOutcome::IdempotentReplay,
        "{name}"
    );
    let after = book.snapshot(None).unwrap();
    assert_eq!(after.audit.len(), before.audit.len() + 1, "{name}");
    let mut state = after;
    state.audit.pop();
    assert_eq!(state, before, "{name}");
}

fn scale_of(fx: &metabolic_ledger::parity::ParityFixture) -> Scale {
    Scale {
        price_scale: fx.units.price_scale,
        qty_scale: fx.units.qty_scale,
    }
}

# Integration surface: Limen-Capital and Spikenaut-SNN (RM-2535)

`metabolic-ledger` is the accounting and ghost-simulation layer of the HFT
stack. It has no Cargo dependency on — and makes no network call to — the
sibling repos. The integration is a data contract: the consumer maps an
already-decided fill onto `AcceptedFill` and calls `Ledger::apply_fill`.

## From an SNN decision to a ledger audit row

1. **Decide outside this crate.** [rmems/Spikenaut-SNN](https://github.com/rmems/Spikenaut-SNN)
   produces the trading decision; [rmems/Limen-Capital](https://github.com/rmems/Limen-Capital)
   orchestrates it through a risk gate that accepts or rejects the order.
   Sizing authority lives upstream (see Limen-Capital
   [ADR 0001](https://github.com/rmems/Limen-Capital/blob/main/docs/adr/0001-strategy-layer-ownership.md)).
2. **Map the decided fill.** The consumer builds
   `metabolic_ledger::AcceptedFill` from the fill it accepted: integer
   `price_ticks`, `qty_lots`, `fee_subunits`, `event_time_ns`, and the
   identities below.
3. **Apply once.** `Ledger::apply_fill` commits the fill on a
   single-instrument long/flat book and appends an `AuditRecord` carrying
   `fill_id`, `order_id`, and `decision_id` verbatim. The ledger does not
   infer or mint either identity (RM-2470 / GH#34).

[`tests/snn_integration.rs`](../tests/snn_integration.rs) is a runnable
example: a synthetic SNN decision (a struct local to this repo — Spikenaut-SNN
is not a dependency) mapped onto `AcceptedFill`, applied, and the resulting
audit row checked for the same `decision_id` and `order_id`.

## Filling `order_id` and `decision_id`

Both are caller-supplied `Option<String>` on `AcceptedFill`:

- `decision_id` — the upstream decision identity (ADR 0001: monotonically
  increasing per run).
- `order_id` — the consumer's order identity, e.g. Limen-Capital's Alpaca
  `client_order_id` (Limen-Capital issue 42), when one exists.

Contract (enforced in `Ledger::validate_fill` and covered by
`tests/fill_traceability.rs`):

- Preserved verbatim on the committed audit row and on idempotent-replay
  rows (a replayed identical fill appends one row with
  `idempotent_replay = true` and zero realized-PnL delta).
- `Some("")` is `malformed_input`; pass `None` when the caller has no id.
  The ledger does not trim, normalize, or invent ids.
- Changing, adding, or removing either id on the same `fill_id` is
  `duplicate_fill_conflict`, with no state or audit mutation.
- Missing or JSON `null` ids deserialize as `None`; fills without them keep
  the legacy audit shape.

## `GhostFill` is not this path

`apply_ghost_fill` / `GhostFill` is the legacy multi-asset `f32` wallet
bridge. Its fields are `asset`, `side`, `price`, `quantity`, `fee`, `step`,
`reason` — it has **no** `order_id` or `decision_id` fields — and its
`GhostTradeLog` JSONL output is deprecated, best-effort diagnostics, not an
audit record. Fill traceability belongs on `Ledger` / `AuditRecord`.

## What this crate will not do

- No venue I/O, credentials, live market data, or order routing. Callers
  supply already-decided fills; execution happens outside.
- No sizing on the `Ledger` path. `apply_fill` takes the accepted quantity
  as given and never reads `AtpPolicy` or `GhostWallet::kelly_fraction()`.
  Applying a second Kelly layer on top of an upstream proposal is a bug
  (ADR 0001), so the ledger stays accounting-only.
- No live dependency on sibling repos. The cross-repo contracts are
  documents (this page, the accounting contract) and JSON schemas
  (`metabolic-ledger.parity.v1`), not shared crates.

## `AtpPolicy`'s role

`AtpPolicy` (`src/policy.rs`) proposes experimental sizes from
`energy_commitment` and caller-passed balances. Nothing consumes the
proposal: `execute_buy` / `execute_sell` size from
`wallet.kelly_fraction()`, and `Ledger` never applies it. It is an
experiment, not an accounting input.

## Repositories

| Repo | Role | Integration |
| --- | --- | --- |
| [rmems/Spikenaut-SNN](https://github.com/rmems/Spikenaut-SNN) | SNN strategy producing decisions | `decision_id` source; no code dependency |
| [rmems/Limen-Capital](https://github.com/rmems/Limen-Capital) | Orchestration, risk gate, venue adapter | Maps decisions to `AcceptedFill` (`decision_id`, `client_order_id`); see issue 42 and ADR 0001 |
| [rmems/DendriteTrader.jl](https://github.com/rmems/DendriteTrader.jl) | Market replay and research | Offline emitter of `metabolic-ledger.parity.v1` fixtures (RM-1864); see the [golden fixtures](accounting-contract.md#golden-fixtures) contract |

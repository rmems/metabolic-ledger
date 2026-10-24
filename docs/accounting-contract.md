# Paper accounting contract (RM-1863)

This crate now exposes two layers:

| Layer | Types | Role |
| --- | --- | --- |
| **Economic accounting** | `PaperLedger`, `AcceptedFill` | Apply an already-decided simulated fill exactly once |
| **ATP / Kelly policy** | `AtpPolicy`, `execute_buy` / `execute_sell`, `GhostWallet` | Experimental energy sizing. **Realized PnL on `PaperLedger` never reads this state.** |

Legacy `execute_buy` / `execute_sell` still size from ATP × Kelly and write `f32` ghost logs. They are not the parity path. Do not treat `METABOLIC_COST` as an exchange fee.

## Supported subset (M2)

- **One instrument** per `PaperLedger`. A different symbol is `instrument_mismatch`.
- **Long or flat only.** `inventory_lots >= 0`. Buys open or add. Sells reduce or close.
- **Sells may not exceed inventory.** That case is `unsupported_short` (same error as a sell from flat). Nothing is clamped to flatten.
- **No shorts and no multi-instrument books.** A short or oversell is `unsupported_short`. A fill or non-fill for any symbol other than the configured one is `instrument_mismatch`. There is no separate multi-instrument error.
- **No derivatives.** They cannot be expressed: `AcceptedFill` has no contract type, leverage, margin, expiry, or funding field, so there is nothing to reject at runtime. Every fill is treated as a spot fill of the configured instrument.
- `unsupported_subset` (`LedgerError::UnsupportedSubset`) is a reserved error type. No code path returns it at this revision. It stays in the enum and in the `type_name()` contract so a later schema can use it without renaming.
- **Paper only.** No venue, credentials, or network I/O.

Anything outside the subset is rejected. Failed calls leave cash, inventory, cost, realized PnL, and previously committed audit rows unchanged.

## Units

Integer only. No `f32`/`f64` on the paper path.

| Quantity | Type | Meaning |
| --- | --- | --- |
| `price_ticks` | `i64` | Price in ticks. `price_scale` ticks = 1 quote unit. |
| `qty_lots` | `i64` | Size in lots. `qty_scale` lots = 1 asset unit. |
| quote subunits | `i64` | Cash, fees, cost, realized / unrealized PnL. |

Shipped fixtures use `price_scale = 100` (cents) and `qty_scale = 1` (whole lots).

Notional:

```
notional_subunits = (price_ticks * qty_lots) / qty_scale
```

The product must be divisible by `qty_scale` and must fit in `i64`. Otherwise the fill is rejected before mutation.

Sign rules:

- Buy cash change is `-(notional + fee)`.
- Sell cash change is `+(notional - fee)`.
- `fee_subunits > 0` is a commission (paid once).
- `fee_subunits < 0` is a rebate (credited once). Rebate size is not capped at the notional.
- A buy rebate larger than the notional makes `notional + fee` negative, so cash **increases** and `total_cost_subunits` can go negative. That is allowed. The fill is rejected only when the integer add or subtract overflows `i64`.

## Cost basis and realized PnL

The book stores `total_cost_subunits` (integer), not a float average.

- Buy: `total_cost += notional + fee` (buy fees enter the basis).
- Partial sell: `allocated = trunc(total_cost * sell_qty / inventory)` toward zero; realized = `(notional - fee) - allocated`.
- Full close: allocated is the **remaining** `total_cost` so no dust is left on a flat book.

`weighted_cost_basis_ticks` is ticks **per asset unit**, the same unit as `price_ticks`:

```
floor((total_cost_subunits * qty_scale) / inventory_lots)
```

when long, else `0`. Floor is Euclidean, so a negative cost from a large buy rebate rounds down. With `qty_scale = 1` this is `floor(total_cost / inventory)`. Fixtures that need an exact basis should assert `total_cost_subunits` as well.

The product is computed in `i128`. If the floored result does not fit in `i64`, `PaperLedger::checked_weighted_cost_basis_ticks()` returns `out_of_range`. It never saturates to `i64::MIN` / `i64::MAX`. `snapshot`, `snapshot_json`, and the parity harness use the checked value, so an unrepresentable basis fails the snapshot (and the fixture) instead of comparing a clamped number. Ledger state is not changed by this failure; the fill that produced the cost has already been committed under the normal checked arithmetic.

`AtpPolicy::energy_commitment` is the fraction used by `propose_buy_spend` and `propose_sell_qty`. Those helpers do not read the wallet Kelly fraction, and `PaperLedger` does not apply them.

Unrealized / mark-to-market is **not** ledger state. Callers pass an external `mark_price_ticks`:

```
unrealized = notional(mark, inventory) - total_cost
```

## Fill identity and time

- `fill_id` is the stable fill identity (non-empty string).
- `event_time_ns` is injected by the caller. The ledger does not call `Utc::now()`.
- Same `fill_id` + same payload → `ApplyOutcome::IdempotentReplay` (no second debit; an audit row with `idempotent_replay = true`).
- Same `fill_id` + different payload → `duplicate_fill_conflict`.

## Non-fills

`reject`, `cancel`, and `non_fill` append audit rows with `realized_pnl_delta = 0`. They do not change cash, inventory, cost, or fees.

## ATP separation

`AtpPolicy::propose_buy_spend` and `propose_sell_qty` size from `energy_commitment` and the balances the caller passes. `PaperLedger::apply_fill` takes the accepted quantity as given and has no `GhostWallet` / ATP field. Changing ATP or `energy_commitment` after a fill cannot change committed realized PnL.

## Golden fixtures

Schema id: `metabolic-ledger.parity.v1` (version `1`).

Directory: `fixtures/v1/`. Hand-verified files ship now. Do not change DendriteTrader.jl from this repository.

Versioning rules:

- **Existing v1 fixture files are immutable.** Do not edit, rename, or delete a shipped `fixtures/v1/*.json` file to make a test pass, and do not relax its expected values or tolerances.
- **Additional v1 fixtures are allowed only when they keep v1 semantics.** A new file may go into `fixtures/v1/` (for example a DendriteTrader.jl (RM-1864) emission, or a new hand-verified case) only if it uses schema `metabolic-ledger.parity.v1` / `schema_version: 1` exactly as written here, uses only the v1 fields and event kinds, and every existing v1 fixture still passes unchanged.
- **A semantic change needs a new schema version and directory.** Any change to units, sign rules, rounding, cost-basis or PnL formulas, error types, event kinds, audit row shape, or what a field means goes into a new schema id and version (for example `metabolic-ledger.parity.v2` in `fixtures/v2/`). `load_fixture` rejects any schema id or version it does not know, so a v2 file cannot be silently replayed under v1 rules.

Integer path tolerance is `0`. The `tolerances` object exists so a later Julia float oracle can declare a nonzero epsilon without changing the schema id; this crate’s shipped fixtures keep zeros.

Replay: `metabolic_ledger::parity::load_fixture` + `run_fixture`. See `tests/parity_harness.rs`.

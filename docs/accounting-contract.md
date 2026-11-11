# Deterministic accounting contract (RM-1863)

This crate now exposes two layers:

| Layer | Types | Role |
| --- | --- | --- |
| **Economic accounting** | `Ledger`, `AcceptedFill` | Apply an already-decided fill exactly once |
| **ATP / Kelly policy** | `AtpPolicy`, `execute_buy` / `execute_sell`, `GhostWallet` | Experimental energy sizing. **Realized PnL on `Ledger` never reads this state.** |

Legacy `execute_buy` / `execute_sell` still size from ATP × Kelly and write `f32` ghost logs. They are not the parity path. Do not treat `METABOLIC_COST` as an exchange fee.

The legacy multi-asset wallet also exposes `apply_ghost_fill(GhostFill)`, a
quantity-taking compatibility bridge. It applies caller-supplied quantity and
non-negative quote-unit friction without ATP/Kelly resizing. Buy friction enters
weighted cost basis and is realized on partial/full sales, including in adaptive
Kelly statistics. Buy audit rows have zero realized PnL. It returns the actual
`GhostTradeLog` or `GhostFillError`, including log I/O failures before wallet
commit. A failed write may leave a partial JSONL line. This `f32` bridge is not
the deterministic integer parity contract below; the v1 fixtures are unchanged.

## Supported subset (M2)

- **One instrument** per `Ledger`. A different symbol is `instrument_mismatch`.
- **Long or flat only.** `inventory_lots >= 0`. Buys open or add. Sells reduce or close.
- **Sells may not exceed inventory.** That case is `unsupported_short` (same error as a sell from flat). Nothing is clamped to flatten.
- **No shorts and no multi-instrument books.** A short or oversell is `unsupported_short`. A fill or non-fill for any symbol other than the configured one is `instrument_mismatch`. There is no separate multi-instrument error.
- **No derivatives.** They cannot be expressed: `AcceptedFill` has no contract type, leverage, margin, expiry, or funding field, so there is nothing to reject at runtime. Every fill is treated as a spot fill of the configured instrument.
- `unsupported_subset` (`LedgerError::UnsupportedSubset`) is a reserved error type. No code path returns it at this revision. It stays in the enum and in the `type_name()` contract so a later schema can use it without renaming.
- **Accounting only.** No venue, credentials, or network I/O. Callers supply accepted fills; their execution mode is outside this crate.

Anything outside the subset is rejected. Failed calls leave cash, inventory, cost, realized PnL, and previously committed audit rows unchanged.

## Units

Integer only. No `f32`/`f64` on the accounting path.

| Quantity | Type | Meaning |
| --- | --- | --- |
| `price_ticks` | `i64` | Price in ticks. `price_scale` ticks = 1 quote unit. |
| `qty_lots` | `i64` | Size in lots. `qty_scale` lots = 1 asset unit. |
| quote subunits | `i64` | Cash, fees, cost, realized / unrealized PnL. `price_scale` subunits = 1 quote unit. |

Whole-lot fixtures use `price_scale = 100` (cents) and `qty_scale = 1` (whole lots). For fractional quantities, see the recipe below.

Notional:

```
notional_subunits = (price_ticks * qty_lots) / qty_scale
```

The product is computed in `i128`, must be divisible by `qty_scale`, and the resulting notional must fit in `i64`. Otherwise the fill is rejected before mutation.

Sign rules:

- Buy cash change is `-(notional + fee)`.
- Sell cash change is `+(notional - fee)`.
- `fee_subunits > 0` is a commission (paid once).
- `fee_subunits < 0` is a rebate (credited once). Rebate size is not capped at the notional.
- A buy rebate larger than the notional makes `notional + fee` negative, so cash **increases** and `total_cost_subunits` can go negative. That is allowed. The fill is rejected only when the integer add or subtract overflows `i64`.

### Fractional-asset recipe: cent prices and 1e-8 BTC lots

`price_scale` sets both the price representation and the quote-money subunit; it need not equal the market's price-tick count per dollar. To support **every** cent-priced fill down to one satoshi (`0.00000001 BTC`), use:

```rust
use metabolic_ledger::Scale;

let scale = Scale {
    price_scale: 10_000_000_000,
    qty_scale: 100_000_000,
};
```

1. Set `qty_scale = 100_000_000`: one `qty_lots` is one satoshi.
2. A cent times one satoshi is `0.0000000001 USD` (`1e-10 USD`). Choose that as the quote subunit, so `price_scale = 100 * qty_scale = 10_000_000_000`.
3. Encode a price quoted in integer cents as `price_ticks = price_in_cents * 100_000_000`. Encode quantities as integer satoshis, without rounding. Use exact decimal/integer conversion, not floating point.
4. Encode **all** cash, fees/rebates, cost and PnL in the same `1e-10 USD` subunits: `1 USD = 10_000_000_000` subunits and `1 cent = 100_000_000` subunits. Encode external marks exactly like fill prices. Do not leave cash or fees in cents when switching scales.

For integer cent price `c` and integer satoshi quantity `q`, the notional is `(c * 100_000_000 * q) / 100_000_000 = c * q` subunits. Thus division is exact even for a single satoshi. Merely setting `price_scale = 100` and `qty_scale = 100_000_000` is insufficient: at `$60,000.01`, one satoshi would require `6_000_001 / 100_000_000` cents, which is not an integer. Some larger fills happen to divide exactly at that coarser scale; that does not make it safe for all fractional fills.

If `Scale::notional_subunits` still rejects:

- `malformed_input` with a divisibility reason means the encoded price × quantity needs a fraction of the configured quote subunit. Check the conversions and choose a sufficiently fine quote scale for the actual price/quantity granularity. Do not round the notional or resize the accepted fill to suppress the error. Nonpositive scales, prices or quantities are also `malformed_input`.
- `out_of_range` means the exact notional cannot fit in `i64`. Finer scales trade range for precision: at this scale, positive cash/notionals top out at about `$922 million`. Check price/quantity conversions, balances, fees, accumulated costs and derived cost-basis values against their `i64` bounds too.

Changing only `price_scale` metadata does not rescale stored integers; configure the scale and convert all inputs consistently before creating the ledger. A rejected fill leaves state and audit unchanged. Mark valuation uses the same exact-notional check and may fail without changing the ledger.

#### Hand-verified fractional fixture

[`fixtures/v1/fractional_btc_buy.json`](../fixtures/v1/fractional_btc_buy.json) starts flat with `$1`, buys **one satoshi** at `$60,000.01`, pays a `$0.000001` fee, and marks at `$60,001.01`. The following calculations are by hand, not generated by ledger replay:

| Input | Encoded integer |
| --- | --- |
| Starting cash (`1 * 10_000_000_000`) | `10_000_000_000` |
| Buy price (`6_000_001 cents * 100_000_000`) | `600_000_100_000_000` |
| Quantity (`0.00000001 * 100_000_000`) | `1` |
| Fee (`0.000001 * 10_000_000_000`) | `10_000` |
| Mark (`6_000_101 cents * 100_000_000`) | `600_010_100_000_000` |

- Notional: `600_000_100_000_000 * 1 / 100_000_000 = 6_000_001` subunits (`$0.0006000001`).
- Debit / total cost: `6_000_001 + 10_000 = 6_010_001`.
- Cash: `10_000_000_000 - 6_010_001 = 9_993_989_999`; inventory: `1` lot.
- Weighted cost basis: `floor(6_010_001 * 100_000_000 / 1) = 601_000_100_000_000` ticks per BTC.
- Realized PnL: `0` (no sell); accumulated fees: `10_000`.
- Mark value: `600_010_100_000_000 * 1 / 100_000_000 = 6_000_101`; unrealized PnL: `6_000_101 - 6_010_001 = -9_900`.
- The single audit row copies the fill's timestamp (`1`), ID (`btc-f1`), instrument (`BTC`), side (`buy`), price, quantity and fee. It has `cash_after = 9_993_989_999`, `inventory_after = 1`, `total_cost_after = 6_010_001`, `realized_pnl_delta = 0` and `idempotent_replay = false`. `order_id` and `reason` are `null`; `decision_id` is absent.

Replay this file with `load_fixture` + `run_fixture`, or run `cargo test --all-features --test parity_harness all_shipped_v1_fixtures_reconcile` to check it alongside every existing v1 fixture with zero tolerance.

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

The product is computed in `i128`. If the floored result does not fit in `i64`, `Ledger::checked_weighted_cost_basis_ticks()` returns `out_of_range`. It never saturates to `i64::MIN` / `i64::MAX`. `snapshot`, `snapshot_json`, and the parity harness use the checked value, so an unrepresentable basis fails the snapshot (and the fixture) instead of comparing a clamped number. Ledger state is not changed by this failure; the fill that produced the cost has already been committed under the normal checked arithmetic.

`AtpPolicy::energy_commitment` is the fraction used by `propose_buy_spend` and `propose_sell_qty`. Those helpers do not read the wallet Kelly fraction, and `Ledger` does not apply them.

Unrealized / mark-to-market is **not** ledger state. Callers pass an external `mark_price_ticks`:

```
unrealized = notional(mark, inventory) - total_cost
```

## Fill identity and time

- `fill_id` is the stable fill identity (non-empty string).
- `AcceptedFill` accepts caller-supplied `order_id` and `decision_id` as independent `Option<String>` values. The ledger preserves them verbatim on both committed fill audit rows and idempotent replay rows; it does not infer or mint either identity.
- When provided, either optional identity must be non-empty. `Some("")` is `malformed_input`, checked before duplicate detection or mutation; `None` remains valid. IDs are not trimmed or normalized.
- Both optional identities are part of the fill payload for duplicate detection. Changing, adding, or removing either on the same `fill_id` is `duplicate_fill_conflict`, with no state or audit mutation.
- Missing or JSON `null` identities deserialize as `None`. With no identities, fills retain the legacy behavior: audit `order_id` is `null` and `decision_id` is omitted. Non-fill rows retain their existing order identity and have no decision identity.
- `event_time_ns` is injected by the caller. The ledger does not call `Utc::now()`.
- Same `fill_id` + same payload → `ApplyOutcome::IdempotentReplay` (no second debit; an audit row with `idempotent_replay = true`).
- Same `fill_id` + different payload → `duplicate_fill_conflict`.

## Non-fills

`reject`, `cancel`, and `non_fill` append audit rows with `realized_pnl_delta = 0`. They do not change cash, inventory, cost, or fees.

## ATP separation

`AtpPolicy::propose_buy_spend` and `propose_sell_qty` size from `energy_commitment` and the balances the caller passes. `Ledger::apply_fill` takes the accepted quantity as given and has no `GhostWallet` / ATP field. Changing ATP or `energy_commitment` after a fill cannot change committed realized PnL.

## Golden fixtures

Schema id: `metabolic-ledger.parity.v1` (version `1`).

Directory: `fixtures/v1/`. Hand-verified files ship now. Do not change DendriteTrader.jl from this repository.

Versioning rules:

- **Existing v1 fixture files are immutable.** Do not edit, rename, or delete a shipped `fixtures/v1/*.json` file to make a test pass, and do not relax its expected values or tolerances.
- **Additional v1 fixtures are allowed only when they keep v1 semantics.** A new file may go into `fixtures/v1/` (for example a DendriteTrader.jl (RM-1864) emission, or a new hand-verified case) only if it uses schema `metabolic-ledger.parity.v1` / `schema_version: 1` exactly as written here, uses only the v1 fields and event kinds (including the additive traceability extension below), and every existing v1 fixture still passes unchanged.
- **Additive traceability extension (RM-2470).** Fill inputs may optionally carry `order_id` and `decision_id`; audit rows may optionally carry `decision_id` alongside the existing `order_id` slot. New fields default to `None` when missing and are omitted when `None` during serialization. The existing audit `order_id` slot is still serialized, including `null`. Thus legacy fill inputs and audit rows retain their serialized field shape and order, all shipped v1 fixtures remain unchanged, and no accounting semantics change. This backwards-compatible extension stays in v1; removing or reinterpreting fields still requires a new version.
- **A semantic change needs a new schema version and directory.** Any change to units, sign rules, rounding, cost-basis or PnL formulas, error types, event kinds, audit row shape, or what a field means goes into a new schema id and version (for example `metabolic-ledger.parity.v2` in `fixtures/v2/`). `load_fixture` rejects any schema id or version it does not know, so a v2 file cannot be silently replayed under v1 rules.

Integer path tolerance is `0`. The `tolerances` object exists so a later Julia float oracle can declare a nonzero epsilon without changing the schema id; this crate’s shipped fixtures keep zeros.

Replay: `metabolic_ledger::parity::load_fixture` + `run_fixture`. See `tests/parity_harness.rs`.

# CLAUDE.md

Guidance for Claude Code in `rmems/metabolic-ledger`, a Rust library crate (no binary). It does paper accounting and runs ATP/Kelly sizing experiments for SNN portfolio research. Nothing here trades real money.

The authority for accounting behavior is [`docs/accounting-contract.md`](docs/accounting-contract.md). If this file and the contract disagree, the contract wins. Fix this file.

## Invariants

Do not break these. Each one is enforced in code or written in the contract.

1. **Research and paper only.** No venue connectivity, credentials, or network I/O ("Paper only" in the contract). The crate has no network dependencies: just `serde`, `serde_json`, `chrono`, and optional `sentry`. It does no model training. The legacy engine (`src/engine.rs`) executes simulated buys and sells that mutate `GhostWallet` state, and appends JSONL logs that SNN training can use elsewhere.
2. **Economic accounting is separate from ATP/Kelly policy.**
   - `PaperLedger` (`src/paper.rs`) owns cash, inventory, total cost, realized PnL and fees. It can compute unrealized PnL from a mark price the caller passes in.
   - `AtpPolicy` (`src/policy.rs`), `GhostWallet` (`src/wallet.rs`) and `execute_buy` / `execute_sell` (`src/engine.rs`) are experimental sizing policy. They are not accounting inputs.
   - `PaperLedger` does not import the policy module. `apply_fill` takes the accepted quantity as given and never applies a `propose_buy_spend` / `propose_sell_qty` proposal.
   - `tests/parity_harness.rs::realized_pnl_never_reads_atp_or_kelly` guards this.
   - `METABOLIC_COST` is ATP friction, not an exchange fee.
3. **Integer units on the paper path.** Prices are `price_ticks: i64`, sizes are `qty_lots: i64`, and money is in quote subunits (`i64`). Scales come from `Scale` in `src/units.rs`. There is no `f32`/`f64` on this path. Fill arithmetic is checked, and overflow is `LedgerError::OutOfRange`. Derived values are checked too: `PaperLedger::checked_weighted_cost_basis_ticks()` returns `OutOfRange` when the i128 result does not fit in `i64` (no saturation), and `snapshot`, `snapshot_json` and the parity harness propagate that error instead of comparing a clamped number. The legacy `execute_*` / `GhostTradeLog` path is `f32` and is **not** the parity path.
4. **Fees and rebates are applied exactly once per fill.** `fee_subunits > 0` is a commission and `< 0` is a rebate. Buy fees enter the cost basis.
5. **Supported subset: one instrument, long or flat.**
   - Overselling, or selling from flat, is `LedgerError::UnsupportedShort`.
   - A second symbol is `LedgerError::InstrumentMismatch`.
   - Derivatives cannot be expressed: `AcceptedFill` has no contract, leverage, margin, expiry or funding field.
   - `UnsupportedSubset` (`unsupported_subset`) is a reserved type name with no current code path, per the contract. Keep its `type_name()` stable.
   - Nothing is clamped or silently coerced: not fill quantities, positions, or derived snapshot values.
   - A failed call leaves committed state unchanged. `plan_fill` computes the result, and `commit_fill` mutates state only on success.
6. **`fill_id` identity.**
   - Same `fill_id` with the same payload returns `ApplyOutcome::IdempotentReplay`. There is no second debit. The ledger appends an audit row with `idempotent_replay = true` and `realized_pnl_delta = 0`.
   - Same `fill_id` with a different payload is `DuplicateFillConflict`.
7. **Non-fills.** `reject`, `cancel` and `non_fill` go through `record_non_fill`. They append audit rows with `realized_pnl_delta = 0` and do not touch cash, inventory, cost or fees.
8. **Deterministic time.** `event_time_ns` is injected by the caller. The paper ledger and audit records never read the wall clock. Only the legacy `execute_*` path calls `Utc::now()`.
9. **Golden fixtures are a cross-language contract.**
   - The fixtures are `fixtures/v1/*.json`, with schema id `metabolic-ledger.parity.v1` and `schema_version: 1` (`src/parity.rs`).
   - DendriteTrader.jl (RM-1864) is meant to emit the same JSON.
   - `load_fixture` rejects any other schema id or version.
   - Integer tolerances are `0`.
   - Versioning rules are in the contract ("Golden fixtures"): shipped v1 fixture files are immutable; additional v1 files are allowed only if they keep v1 semantics and every existing v1 fixture still passes unchanged; a semantic change needs a new schema version and directory (e.g. `fixtures/v2/`).
   - Do not change DendriteTrader.jl from this repo.

## Verified commands

Run on branch `claude/reject-subset-and-saturation` (main `ddbdf76` merged) with cargo/rustc 1.97.0 on Linux. These match `.github/workflows/ci.yml` and `azure-pipelines.yml`.

| Command | Result at HEAD |
| --- | --- |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass (0 warnings) |
| `cargo build --all-features` | pass |
| `cargo test --all-features` | pass: 28 lib + 4 `parity_harness` + 0 doctests |
| `cargo test --all-features --lib` | pass: 28 tests |
| `cargo test --all-features --test parity_harness` | pass: 4 tests |

Not verified locally: `docker build .` (no Docker daemon in the verification environment). CI builds it with `FEATURES=sentry` via `.github/workflows/docker.yml`.

The README code examples are not compiled as doctests (0 doctests run), so they can drift from the API.

## Architecture map

```
src/
  lib.rs      crate docs and public re-exports
  paper.rs    PaperLedger, AcceptedFill, NonFillEvent, ApplyOutcome (economic accounting)
  units.rs    Scale; checked i64 notional/add/sub
  error.rs    LedgerError (typed, stable type_name() strings used by fixtures)
  audit.rs    AuditRecord / AuditKind (injected event time)
  types.rs    Side
  parity.rs   metabolic-ledger.parity.v1 schema, load_fixture, run_fixture, shipped_fixture_dir
  policy.rs   AtpPolicy (experimental sizing proposals; not accounting)
  wallet.rs   GhostWallet, MarketPrices, PortfolioSummary (f32, adaptive Kelly)
  engine.rs   CELLULAR_ATP / ENERGY_COMMITMENT / METABOLIC_COST, execute_buy / execute_sell (legacy)
  log.rs      GhostTradeLog + JSONL append (legacy path)
tests/
  parity_harness.rs  replays every fixtures/v1/*.json; byte-stability; fee-once; ATP independence
fixtures/v1/   hand-verified golden files (buy, partial/full close, rebate, duplicate fill,
               reject, cancel/non-fill, short rejected, instrument mismatch)
docs/
  accounting-contract.md  authoritative paper-accounting contract (units, sign rules, cost basis)
  transfer-hygiene.md     repo transfer audit notes
```

## Working notes for Claude

- Put new accounting behavior in `paper.rs` and the contract doc together. Add a fixture or unit test for it.
- Run the full check list above before claiming a change is done.
- `Cargo.lock` is gitignored. This is a library crate, and dependency versions resolve at build time.
- Report negative or failing results as they are. Treat any parity "pass" with a nonzero tolerance, or a relaxed fixture, as suspect.

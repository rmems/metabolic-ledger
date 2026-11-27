# AGENTS.md

Guidance for coding agents (Amp, Codex, Cursor, Claude Code, and others) working in this repository.

## Purpose

`metabolic-ledger` is a Rust library that simulates a multi-asset portfolio without real funds,
using biological energy concepts (ATP) to constrain position sizing (see `README.md`). A buy spends
`kelly_fraction()` of the available ATP; a sell sells `kelly_fraction()` of the asset balance and
adds the proceeds back to ATP. Both deduct a metabolic cost (`METABOLIC_COST`) that models
spread/slippage (`src/engine.rs`). A `log_path` enables deprecated, best-effort JSONL diagnostics
(`GhostTradeLog`), not an audit record; write errors reject the fill before commit and a failed
write can leave a partial line. Use `Ledger` / `AuditRecord` for accounting and handle
persistence errors separately. Trades below the minimum
size return without trading or logging. It owns persistent ghost accounting (`GhostWallet`,
realized PnL, win rate, Kelly fraction).

## Layout

| Path | Contents |
|------|----------|
| `src/lib.rs` | Crate root, re-exports, `readme_doctests` README include |
| `src/ledger.rs` | `Ledger`, `AcceptedFill`, `NonFillEvent`, `ApplyOutcome` (integer accounting) |
| `src/units.rs` | `Scale`; checked i64 notional/add/sub |
| `src/error.rs` | `LedgerError` (stable `type_name()` strings used by fixtures) |
| `src/audit.rs` | `AuditRecord` / `AuditKind` (caller-injected event time) |
| `src/types.rs` | `Side` |
| `src/parity.rs` | `metabolic-ledger.parity.v1` schema, `load_fixture`, `run_fixture` |
| `src/policy.rs` | `AtpPolicy` (experimental sizing; not an accounting input) |
| `src/wallet.rs` | `GhostWallet`, `MarketPrices`, per-asset cost basis, `PortfolioSummary` (f32, adaptive Kelly) |
| `src/engine.rs` | ATP-gated `execute_buy` / `execute_sell` (legacy policy path) |
| `src/ghost_fill.rs` | `apply_ghost_fill` / `GhostFill` caller-sized ghost fills |
| `src/log.rs` | `GhostTradeLog`; deprecated best-effort JSONL writer |
| `tests/` | `parity_harness.rs`, `ghost_fills.rs`, `fill_traceability.rs` |
| `fixtures/v1/` | Golden parity fixtures (`metabolic-ledger.parity.v1`; immutable) |
| `docs/` | `accounting-contract.md`, `transfer-hygiene.md`, logo |
| `.github/workflows/` | `ci.yml` |

## Toolchain

- Rust `stable` (CI uses `dtolnay/rust-toolchain` stable with rustfmt, clippy); edition 2024.
- MSRV `rust-version = "1.88"`: edition 2024 needs ≥1.85 and the let-chains in
  `src/parity.rs` raise the floor to 1.88. The `msrv` CI job runs
  `cargo check --all-targets --all-features` and `cargo test --all-features --doc`
  on 1.88.0, so the floor covers the `sentry` feature too; `azure-pipelines.yml`
  mirrors the check.
- `Cargo.lock` is gitignored (library crate).
- Feature `sentry` (off by default). Dev dependency `proptest` for property tests.
- No GPU needed. Default builds need no system packages, but `--all-features` (as in CI) enables
  `sentry`, whose default `native-tls` transport needs `openssl-sys`: install `pkg-config` and the
  OpenSSL headers (`libssl-dev` on Debian/Ubuntu) first.

## Commands (from `.github/workflows/ci.yml`, run on ubuntu, windows and macos)

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-features
cargo test --all-features
cargo +1.88.0 check --all-targets --all-features   # msrv job
cargo +1.88.0 test --all-features --doc            # msrv job: README doctests
cargo test --all-features --doc                    # ubuntu leg (llvm-cov skips doctests)
```

`azure-pipelines.yml` (Linux, Windows, macOS) is secondary CI.

## Conventions visible in the repo

- The README documents the constants `CELLULAR_ATP`, `ENERGY_COMMITMENT` and `METABOLIC_COST`, and
  the "always valid" Kelly fraction (`GhostWallet::kelly_fraction()`, issue #12), as public behaviour.
- CI runs on Windows and macOS too, so keep code and tests platform-neutral (paths, line endings).
- Rust sources carry SPDX license identifier headers.
- Commit subjects mostly follow Conventional Commits with issue numbers.

## M2 guardrails (paper-only scope)

- Reject unsupported positions with an explicit error; never silently coerce shorts or other
  unsupported states.
- Apply fees/rebates and sizing exactly once per fill.
- Keep units consistent (price, quantity, notional, fees) across fill application, PnL and audit
  records.
- Keep ATP-policy state separate from economic accounting and realized PnL.
- No venue connectivity, no live trading, no model training in this crate.
- No Kelly-from-raw-confidence sizing.

## PR metadata hygiene — set at creation, not as a retrofit

Every pull request opened in this repo must carry the following PR metadata, in the same pass it is created. If an item does not apply or cannot be set, state the exception in the PR body before treating the PR as complete:

- **Assignee:** `rmems`.
- **Linear issue ID** in the PR title as a trailing `(RM-xxxx)` or `(LIM-xxxx)`, in the branch name, and in commit subjects — so the GitHub↔Linear integration attaches the PR to its issue natively.
- **Labels** from this repo's existing label vocabulary — mirror the labels on the issue the PR implements; use only existing labels and note in the PR body when none apply.
- **Milestone:** the repo's milestone for the work, when this repo uses milestones and one applies.
- **Issue link:** the PR body references the issue it implements; when an issue is closed, its implementing PR is cited by full URL in the close comment and mirrored on the Linear twin.

Verify with a read after creating the PR. A PR missing applicable metadata without a stated exception is not done.

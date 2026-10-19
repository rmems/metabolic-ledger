# CLAUDE.md

Claude-specific notes only. There is no `AGENTS.md` in this repo; project overview and the accounting contract are in
`README.md` and `docs/accounting-contract.md`, so they are not repeated here.

## Commands

Run from the repo root. "Verified" means it ran successfully in a fresh checkout on Rust 1.97.0 (stable, Linux, CPU only).
There is no `rust-toolchain.toml`; CI uses `dtolnay/rust-toolchain` stable. `Cargo.lock` is gitignored, so do not pass `--locked`.

| Command | Status |
|---|---|
| `cargo fmt --check` | verified |
| `cargo clippy --all-targets -- -D warnings` | verified (default features) |
| `cargo test` | verified (25 unit tests, 4 in `tests/parity_harness.rs`; default features) |
| `cargo build` | verified (default features) |
| `cargo clippy --all-targets --all-features -- -D warnings` | not run (`--all-features` pulls the `sentry` tree; disk was nearly full); CI runs it |
| `cargo build --all-features`, `cargo test --all-features` | not run (same reason); CI runs both on Linux, Windows, macOS |
| `docker build .` (`docker.yml`) | not run (Docker not available) |

CI (`.github/workflows/ci.yml`) runs fmt, clippy, build, and test with `--all-features`; `azure-pipelines.yml` repeats them.
Run fmt and clippy before committing; CI fails on either.

## Architecture map

- `src/paper.rs`: `PaperLedger`, `AcceptedFill`. Exact-fill, integer accounting for one instrument, long or flat.
- `src/units.rs`, `src/types.rs`, `src/error.rs`, `src/audit.rs`: `Scale`, `Side`, typed `LedgerError`, deterministic audit rows.
- `src/parity.rs` + `tests/parity_harness.rs`: loader and replay for `fixtures/v1/*.json` (schema `metabolic-ledger.parity.v1`).
- `src/policy.rs`, `src/engine.rs`, `src/wallet.rs`, `src/log.rs`: ATP/Kelly sizing policy, legacy `execute_buy`/`execute_sell`, `GhostWallet`, JSONL `GhostTradeLog`.
- `docs/accounting-contract.md` is the contract; `docs/transfer-hygiene.md` records the repo transfer audit.

## Invariants and non-goals

- This is a paper-trading simulation library. No venue, credentials, network I/O, or real funds.
- `PaperLedger` stays integer-only (`i64` ticks, lots, subunits) and never reads ATP or Kelly state. Do not add `f32`/`f64` to that path.
- Out-of-subset input (shorts, a second instrument, derivatives, oversized sells) returns a typed `LedgerError`; never clamp or coerce. A failed call must leave state unchanged.
- Duplicate `fill_id` is idempotent, or a conflict if the payload differs. Fees and size apply once.
- `METABOLIC_COST` is a simulation friction constant, not an exchange fee.
- Golden fixtures are shared with DendriteTrader.jl (RM-1864). Changing the schema or a shipped fixture needs matching contract-doc updates.
- `sentry` is an optional, off-by-default feature; keep the default build free of it.
- Source files carry a `Copyright 2025 Limen-Neural` header plus `// SPDX-License-Identifier: MIT OR Apache-2.0`. Keep both on new `.rs` files (`tests/parity_harness.rs` has neither).

## Gotchas

- Copyright notices and the Azure DevOps URL keep `Limen-Neural` on purpose (`docs/transfer-hygiene.md`).
- `Cargo.toml` has `exclude = ["docs/"]` only. There is no `AGENTS.md` to mirror, so this file needs no manifest change.
- `.claude/` is not gitignored (`.gitignore` has only `/target/` and `Cargo.lock`); do not commit local agent state there.
- Never run `cargo publish`.
- Edit only what the task needs.

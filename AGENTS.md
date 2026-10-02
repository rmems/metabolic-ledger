# AGENTS.md

Guidance for coding agents (Amp, Codex, Cursor, Claude Code, and others) working in this repository.

## Purpose

`metabolic-ledger` is a Rust library that simulates a multi-asset portfolio without real funds,
using biological energy concepts (ATP) to constrain position sizing (see `README.md`). A buy spends
`kelly_fraction()` of the available ATP; a sell sells `kelly_fraction()` of the asset balance and
adds the proceeds back to ATP. Both deduct a metabolic cost (`METABOLIC_COST`) that models
spread/slippage (`src/engine.rs`). Executed trades are appended to JSONL (`GhostTradeLog`) for SNN
training data when a `log_path` is passed; trades below the minimum size return without trading or
logging. It owns persistent ghost accounting (`GhostWallet`, realized PnL, win rate, Kelly fraction).

## Layout

| Path | Contents |
|------|----------|
| `src/lib.rs` | Crate root and re-exports |
| `src/wallet.rs` | `GhostWallet`, per-asset cost basis, realized PnL, `PortfolioSummary` |
| `src/engine.rs` | ATP-gated `execute_buy` / `execute_sell` |
| `src/log.rs` | `GhostTradeLog` JSONL audit trail |
| `docs/` | `transfer-hygiene.md`, logo |

## Toolchain

- Rust `stable` (CI uses `dtolnay/rust-toolchain` stable with rustfmt, clippy); edition 2024.
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
```

`azure-pipelines.yml` (Linux, Windows, macOS) and `docker.yml` are secondary CI.

## Conventions visible in the repo

- The README documents the constants `CELLULAR_ATP`, `ENERGY_COMMITMENT` and `METABOLIC_COST`, and
  the "always valid" Kelly fraction (`GhostWallet::kelly_fraction()`, issue #12), as public behaviour.
- CI runs on Windows and macOS too, so keep code and tests platform-neutral (paths, line endings).
- Rust sources carry SPDX license identifier headers.
- Commit subjects mostly follow Conventional Commits with issue numbers.

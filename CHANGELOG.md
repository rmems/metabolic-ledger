# Changelog

## 0.2.0 (unreleased)

### Breaking API rename (RM-2474)

- Rename `PaperLedger` to `Ledger` and `src/paper.rs` / the public `paper`
  module to `src/ledger.rs` / `ledger`. Use `metabolic_ledger::Ledger` or
  `metabolic_ledger::ledger::Ledger`; import `LedgerSnapshot` from
  `metabolic_ledger::ledger`.
- Choose a clean break, with no deprecated type alias or module shim. The
  pre-1.0 minor bump from 0.1.0 to 0.2.0 marks this incompatible Rust API
  change. All other types formerly in `paper` are now in `ledger`; existing
  crate-root re-exports keep their names.
- `Ledger` names the deterministic economic-accounting engine, separate from
  the legacy ATP/Kelly `GhostWallet` and sizing policy. This is a rename only:
  fill processing, `LedgerSnapshot` semantics and JSON bytes, the accounting
  rules, and the parity-v1 schema and audit format are unchanged. Shipped
  `fixtures/v1/*.json` files are unchanged.
- Remove the `Dockerfile`, the `docker.yml` GitHub Actions workflow, and the
  related Docker references in agent config and docs. The crate is consumed as
  a git dependency and no longer publishes a container image.

### Docs & CI (RM-2536)

- Declare `rust-version = "1.88"` (MSRV): edition 2024 needs ≥1.85 and the
  let-chains in `src/parity.rs` raise the floor to 1.88. A dedicated `msrv` CI
  job runs `cargo check --all-targets --all-features` and
  `cargo test --all-features --doc` on 1.88.0; Azure Pipelines mirrors the check.
- README: pin a commit (`rev`) instead of the nonexistent `v0.2.0` tag, note the
  MSRV, and compile the `rust` examples as doctests via a `cfg(doctest)` include.
  The Sentry init example uses the `metabolic_ledger::sentry` re-export and is
  `no_run`; the JSONL `log_path` example is `no_run` too (it writes a file).
- AGENTS.md: the layout table now covers `ledger.rs`, `policy.rs`, `fixtures/`
  and the rest of the tree.

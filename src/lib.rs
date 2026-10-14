// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//!
//! Bio-inspired ghost trading engine with cellular ATP energy metaphors.
//!
//! Ghost trading simulates a multi-asset portfolio without real funds,
//! using biological energy concepts to constrain position sizing:
//!
//! - **ATP** (`CELLULAR_ATP = 500 quote units`) — total available energy
//! - **Energy commitment** (8% per signal) — fraction risked per trade
//! - **Metabolic cost** (0.1% per action) — friction / spread simulation
//!
//! All trades are logged to JSONL for SNN training data and performance analysis.
//!
//! ## Paper accounting (RM-1863)
//!
//! [`paper::PaperLedger`] applies an already-decided fill exactly once on a
//! **single-instrument long/flat** book. Units, fees, and the Julia fixture
//! contract are documented in `docs/accounting-contract.md`.
//! ATP/Kelly live in [`policy`] and the legacy [`engine`] helpers; realized
//! PnL does not read them.
//!
//! ## Provenance
//!
//! Extracted from Eagle-Lander, the author's own private neuromorphic GPU supervisor repository (closed-source).
//! The ghost trading engine ran in production for multi-asset portfolio optimization before being open-sourced
//! as a standalone library.
//!
//! ## References
//!
//! - Kelly, J.L. (1956). A New Interpretation of Information Rate.
//!   *Bell System Technical Journal*, 35(4), 917–926.
//!   <https://doi.org/10.1002/j.1538-7305.1956.tb03809.x>
//!
//! - Thorp, E.O. (1969). Optimal Gambling Systems for Favorable Games.
//!   *Review of the International Statistical Institute*, 37(3), 273–293.
//!   Half-Kelly variance reduction rationale.
//!
//! - Alberts, B. et al. (2002). *Molecular Biology of the Cell* (4th ed.).
//!   ATP as cellular energy currency — conceptual basis for CELLULAR_ATP metaphor.

/// Deterministic paper-ledger audit records (injected event time).
pub mod audit;

/// Typed paper-ledger errors (no silent coerce / clamp).
pub mod error;

/// Ghost trade execution engine with buy/sell logic and metabolic constants.
pub mod engine;

/// Trade log record type and JSONL append helper.
pub mod log;

/// Versioned golden-fixture schema and replay harness.
pub mod parity;

/// Exact-fill paper ledger (economic accounting; no ATP).
pub mod paper;

/// ATP / Kelly sizing policy — experimental, not an accounting input.
pub mod policy;

/// Shared paper-market enums.
pub mod types;

/// Integer price / quantity scales.
pub mod units;

/// Virtual wallet with ATP balance, positions, adaptive Kelly sizing, and persistent
/// accounting (realized PnL per asset via `summary()`).
pub mod wallet;

/// Re-export of primary engine constants and execution functions for convenience.
pub use engine::{CELLULAR_ATP, ENERGY_COMMITMENT, METABOLIC_COST, execute_buy, execute_sell};
/// Re-export of the paper-accounting surface used by the parity harness.
pub use error::LedgerError;
/// Re-export of the trade log record type.
pub use log::GhostTradeLog;
pub use paper::{AcceptedFill, ApplyOutcome, NonFillEvent, NonFillKind, PaperLedger};
pub use policy::AtpPolicy;
pub use types::Side;
pub use units::Scale;
/// Re-export of wallet types for the public API (including `PortfolioSummary` for
/// centralized realized PnL per asset + win-rate, per issue #3).
pub use wallet::{GhostWallet, MarketPrices, PortfolioSummary};

#[cfg(feature = "sentry")]
/// Re-export of the `sentry` crate (when the `sentry` feature is enabled) for
/// error monitoring, tracing, and panic integration in consuming applications.
pub use sentry;

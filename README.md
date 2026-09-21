<p align="center">
  <img src="docs/logo.png" width="220" alt="Metabolic Ledger">
</p>

<h1 align="center">metabolic-ledger</h1>
<p align="center">Bio-inspired ghost trading engine with ATP cellular energy metaphors</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue" alt="MIT/Apache-2.0">
</p>

---

Simulates a multi-asset portfolio without real funds, using biological energy
concepts to constrain position sizing. Each trade signal consumes a fraction of
available ATP, with a metabolic cost modelling spread/slippage. All decisions are
logged to JSONL for SNN training data and post-hoc analysis.

## Features

- `GhostWallet` — dynamic multi-asset portfolio with weighted-average cost basis per asset
- `execute_buy` / `execute_sell` — ATP-gated order execution with metabolic cost
- `CELLULAR_ATP = 500.0` — initial energy budget (quote-unit equivalent)
- `ENERGY_COMMITMENT = 0.08` — 8% of available energy per signal
- `METABOLIC_COST = 0.001` — 0.1% friction per action
- `GhostTradeLog` — JSONL audit trail with timestamp, asset, action, price, quantity, reason, per-trade realized_pnl_usdt
- `PortfolioSummary` / `GhostWallet::summary()` — centralized realized PnL *per asset*, total, win-rate, trade counts, **current_kelly_fraction** (always valid; for #4)
- `GhostWallet::kelly_fraction()` — validated getter for the current Kelly fraction (enforces "always valid" per #12)
- Per-asset realized PnL tracking in `GhostWallet.realized_pnls`
- Kelly fraction auto-update after 10+ trades based on realized win rate
- `win_rate()` and `summary()` for accounting (computable from trade logs too)

## Review note
This branch contains the Wallet/MarketPrices refactor to dynamic asset maps.

## Usage (Git Dependency)

```toml
metabolic-ledger = { git = "https://github.com/rmems/metabolic-ledger" }
```

Recommended for reproducibility:

```toml
# Pin to a release tag
metabolic-ledger = { git = "https://github.com/rmems/metabolic-ledger", tag = "v0.1.0" }

# Or pin to an exact commit
metabolic-ledger = { git = "https://github.com/rmems/metabolic-ledger", rev = "<commit-sha>" }
```

## Quick Start

```rust
use metabolic_ledger::{GhostWallet, execute_buy, execute_sell};
use std::collections::HashMap;

let mut wallet = GhostWallet::new();
let prices: HashMap<String, f32> = HashMap::from([
    ("ASSET_A".to_string(), 12.5),
    ("ASSET_B".to_string(), 86.0),
]);

// SNN fires a BUY signal for ASSET_A
execute_buy(&mut wallet, "ASSET_A", prices["ASSET_A"], 1, "bull signal: confidence=0.92", None);

println!("Portfolio: {:.2}", wallet.portfolio_value(&prices));
println!("ASSET_A units: {:.2}", wallet.balance("ASSET_A"));
```

## Accounting & Kelly state (for downstream)

`PortfolioSummary` is the canonical read-only snapshot of realized accounting. It is safe to serialize/deserialize and always keeps `current_kelly_fraction` inside the valid contract.

```rust
use metabolic_ledger::GhostWallet;

let mut wallet = GhostWallet::new();
// ... run trades ...

let s = wallet.summary();
println!("total pnl: {}", s.total_realized_pnl);
println!("kelly now: {}", s.current_kelly_fraction());

// Per-asset breakdown
for (asset, pnl) in &s.realized_pnl_per_asset {
    println!("{}: {}", asset, pnl);
}
```

> **Round-trip note:** `total_realized_pnl` equals the sum of `realized_pnl_per_asset`. The same totals can be recomputed from a `GhostTradeLog` JSONL stream by summing `realized_pnl_usdt` on rows where `action == "sell"`.

### Deserialize a saved `PortfolioSummary`

```rust
let json = r#"{
    "total_realized_pnl": 12.34,
    "realized_pnl_per_asset": {"ASSET_A": 12.34},
    "win_rate": 0.6,
    "trade_count": 10,
    "closed_trade_count": 5,
    "current_kelly_fraction": 0.08
}"#;
let summary: metabolic_ledger::PortfolioSummary = serde_json::from_str(json).unwrap();
```

## With JSONL Audit Log

```rust
execute_buy(&mut wallet, "ASSET_A", 65.0, 1, "snn_fire", Some("trades.jsonl"));
// Appends: {"ts":"2024-...","asset":"ASSET_A","action":"buy","price":65.0,"quantity":...,...}
```

## Energy Model

```
available_energy = wallet.balance_atp
committed        = available_energy × ENERGY_COMMITMENT   (8%)
units            = committed / price
cost             = committed × METABOLIC_COST             (0.1% friction)
wallet.balance_atp     -= committed + cost
```

Inspired by ATP as cellular energy currency (Alberts et al. 2002) and half-Kelly
position sizing (Kelly 1956; Thorp 1969).

## Provenance

Extracted from Eagle-Lander, the author's own private neuromorphic GPU supervisor
repository (closed-source). Ghost-traded generalized multi-asset portfolios in
production alongside the live SNN supervisor.

## Optional Sentry Integration (error monitoring)

This crate supports optional integration with [Sentry](https://sentry.io) for error tracking, panics, and performance monitoring in consuming applications.

Enable via feature flag (uses the official `sentry` Rust crate):

```toml
[dependencies]
metabolic-ledger = { git = "...", features = ["sentry"] }
# or from crates.io once published
# metabolic-ledger = { version = "0.1", features = ["sentry"] }
```

In your application entrypoint (binary), initialize Sentry **once**:

```rust
#[cfg(feature = "sentry")]
let _guard = sentry::init((
    "https://<YOUR_DSN>@sentry.io/<PROJECT_ID>",
    sentry::ClientOptions {
        release: sentry::release_name!(),
        environment: Some("production".into()),
        // Add more integrations: backtraces, contexts, etc. as needed
        ..Default::default()
    },
));
```

The feature pulls in `sentry` as an optional dependency. See the [sentry-rust docs](https://docs.rs/sentry) for full configuration (tracing, custom events, etc.).

When the `sentry` feature is enabled, you can also access the re-export:

```rust
#[cfg(feature = "sentry")]
use metabolic_ledger::sentry; // if exposed
```

## License

MIT OR Apache-2.0

// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integer quote/quantity scales for paper accounting.
//!
//! Realized PnL is computed in integer quote subunits. There is no floating
//! point on this path.

use crate::error::LedgerError;
use serde::{Deserialize, Serialize};

/// Documented integer scales for one instrument.
///
/// * `price_scale` — ticks in one quote unit (e.g. `100` means ticks are cents).
/// * `qty_scale` — lots in one asset unit (e.g. `1` means lots are whole units).
///
/// Notional in quote subunits is `(price_ticks * qty_lots) / qty_scale`.
/// `qty_scale` must divide `price_ticks * qty_lots` exactly for every accepted
/// fill (the hand-verified fixtures use `qty_scale = 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scale {
    pub price_scale: i64,
    pub qty_scale: i64,
}

impl Scale {
    /// Cents and whole lots — the default fixture scale.
    pub const CENTS_WHOLE_LOTS: Self = Self {
        price_scale: 100,
        qty_scale: 1,
    };

    pub fn validate(self) -> Result<(), LedgerError> {
        if self.price_scale <= 0 {
            return Err(LedgerError::MalformedInput {
                reason: "price_scale must be positive".into(),
            });
        }
        if self.qty_scale <= 0 {
            return Err(LedgerError::MalformedInput {
                reason: "qty_scale must be positive".into(),
            });
        }
        Ok(())
    }

    /// Exact notional in quote subunits, or `OutOfRange` / `MalformedInput`.
    pub fn notional_subunits(self, price_ticks: i64, qty_lots: i64) -> Result<i64, LedgerError> {
        self.validate()?;
        if price_ticks <= 0 {
            return Err(LedgerError::MalformedInput {
                reason: "price_ticks must be positive".into(),
            });
        }
        if qty_lots <= 0 {
            return Err(LedgerError::MalformedInput {
                reason: "qty_lots must be positive".into(),
            });
        }
        let product = (price_ticks as i128)
            .checked_mul(qty_lots as i128)
            .ok_or_else(|| LedgerError::OutOfRange {
                reason: "price_ticks * qty_lots overflowed i128".into(),
            })?;
        if product % (self.qty_scale as i128) != 0 {
            return Err(LedgerError::MalformedInput {
                reason: format!(
                    "qty_scale {} does not divide price_ticks * qty_lots {}",
                    self.qty_scale, product
                ),
            });
        }
        let notional = product / (self.qty_scale as i128);
        i64::try_from(notional).map_err(|_| LedgerError::OutOfRange {
            reason: "notional does not fit i64 quote subunits".into(),
        })
    }
}

/// Checked integer add used by cash and cost updates.
pub fn checked_add(a: i64, b: i64, what: &str) -> Result<i64, LedgerError> {
    a.checked_add(b).ok_or_else(|| LedgerError::OutOfRange {
        reason: format!("{what} overflowed i64"),
    })
}

/// Checked integer subtract.
pub fn checked_sub(a: i64, b: i64, what: &str) -> Result<i64, LedgerError> {
    a.checked_sub(b).ok_or_else(|| LedgerError::OutOfRange {
        reason: format!("{what} overflowed i64"),
    })
}

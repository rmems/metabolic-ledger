// Copyright 2025 Limen-Neural
//
// Licensed under either of Apache License, Version 2.0 or MIT license at your option.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Typed errors for the paper-accounting subset (RM-1863).
//!
//! Unsupported operations are rejected here. They are never silently coerced
//! or clamped.

use std::fmt;

/// Recoverable paper-ledger failure. Failed operations do not mutate committed state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// Inventory would become negative (short) or a sell exceeded holdings.
    UnsupportedShort {
        instrument: String,
        inventory_lots: i64,
        sell_lots: i64,
    },
    /// Event instrument does not match the single configured instrument.
    InstrumentMismatch {
        configured: String,
        requested: String,
    },
    /// Input is missing, zero, negative, or otherwise not representable.
    MalformedInput { reason: String },
    /// Arithmetic would overflow the documented integer money domain.
    OutOfRange { reason: String },
    /// Same `fill_id` seen again with a different payload.
    DuplicateFillConflict { fill_id: String },
    /// Reserved for a feature outside the declared M2 subset.
    ///
    /// No code path returns this at present. A second instrument is
    /// [`Self::InstrumentMismatch`], shorts are [`Self::UnsupportedShort`], and
    /// derivatives cannot be expressed by the current input types. The variant
    /// and its `unsupported_subset` type name are kept stable for the fixture
    /// contract so a later schema can use them.
    UnsupportedSubset { reason: String },
}

impl LedgerError {
    /// Stable machine name for fixtures and the Julia oracle contract.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::UnsupportedShort { .. } => "unsupported_short",
            Self::InstrumentMismatch { .. } => "instrument_mismatch",
            Self::MalformedInput { .. } => "malformed_input",
            Self::OutOfRange { .. } => "out_of_range",
            Self::DuplicateFillConflict { .. } => "duplicate_fill_conflict",
            Self::UnsupportedSubset { .. } => "unsupported_subset",
        }
    }
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedShort {
                instrument,
                inventory_lots,
                sell_lots,
            } => write!(
                f,
                "unsupported short on {instrument}: sell {sell_lots} lots with inventory {inventory_lots}"
            ),
            Self::InstrumentMismatch {
                configured,
                requested,
            } => write!(
                f,
                "instrument mismatch: ledger holds {configured}, event requested {requested}"
            ),
            Self::MalformedInput { reason } => write!(f, "malformed input: {reason}"),
            Self::OutOfRange { reason } => write!(f, "out of range: {reason}"),
            Self::DuplicateFillConflict { fill_id } => {
                write!(f, "duplicate fill conflict: {fill_id}")
            }
            Self::UnsupportedSubset { reason } => write!(f, "unsupported subset: {reason}"),
        }
    }
}

impl std::error::Error for LedgerError {}

// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

//! `Nibble` represents a four-bit unsigned integer.

pub mod nibble_path;

use diem_crypto::HashValue;
#[cfg(feature = "fuzzing")]
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The hardcoded maximum height of a state merkle tree in nibbles.
pub const ROOT_NIBBLE_HEIGHT: usize = HashValue::LENGTH * 2;

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Nibble(u8);

impl From<u8> for Nibble {
    fn from(nibble: u8) -> Self {
        assert!(nibble < 16, "Nibble out of range: {}", nibble);
        Self(nibble)
    }
}

impl From<Nibble> for u8 {
    fn from(nibble: Nibble) -> Self {
        nibble.0
    }
}

impl fmt::LowerHex for Nibble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

#[cfg(feature = "fuzzing")]
impl Arbitrary for Nibble {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        (0..16u8).prop_map(Self::from).boxed()
    }
}

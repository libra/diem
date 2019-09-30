// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module defines physical storage schema for LedgerInfoWithSignatures structure.
//!
//! Serialized LedgerInfoWithSignatures identified by `epoch_num`.
//! ```text
//! |<---key--->|<---------------value------------->|
//! | epoch_num | ledger_info_with_signatures bytes |
//! ```
//!
//! `epoch_num` is serialized in big endian so that records in RocksDB will be in order of it's
//! numeric value.

use crate::schema::ensure_slice_len_eq;
use byteorder::{BigEndian, ReadBytesExt};
use failure::prelude::*;
use libra_proto_conv::{FromProtoBytes, IntoProtoBytes};
use libra_schemadb::{
    define_schema,
    schema::{KeyCodec, ValueCodec},
    DEFAULT_CF_NAME,
};
use libra_types::crypto_proxies::LedgerInfoWithSignatures;
use std::mem::size_of;

define_schema!(
    LedgerInfoSchema,
    u64, /* epoch num */
    LedgerInfoWithSignatures,
    DEFAULT_CF_NAME
);

impl KeyCodec<LedgerInfoSchema> for u64 {
    fn encode_key(&self) -> Result<Vec<u8>> {
        Ok(self.to_be_bytes().to_vec())
    }

    fn decode_key(data: &[u8]) -> Result<Self> {
        ensure_slice_len_eq(data, size_of::<Self>())?;
        Ok((&data[..]).read_u64::<BigEndian>()?)
    }
}

impl ValueCodec<LedgerInfoSchema> for LedgerInfoWithSignatures {
    fn encode_value(&self) -> Result<Vec<u8>> {
        self.clone().into_proto_bytes()
    }

    fn decode_value(data: &[u8]) -> Result<Self> {
        Self::from_proto_bytes(data)
    }
}

#[cfg(test)]
mod test;

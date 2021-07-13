// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

pub mod access_path;
pub mod account_address;
pub mod account_config;
pub mod account_state;
pub mod account_state_blob;
pub mod block_info;
pub mod block_metadata;
pub mod chain_id;
pub mod contract_event;
pub mod diem_id_identifier;
pub mod diem_timestamp;
pub mod epoch_change;
pub mod epoch_state;
pub mod event;
pub mod ledger_info;
pub mod mempool_status;
pub mod move_resource;
pub mod network_address;
pub mod nibble;
pub mod on_chain_config;
pub mod proof;
#[cfg(any(test, feature = "fuzzing"))]
pub mod proptest_types;
pub mod serde_helper;
pub mod state_proof;
#[cfg(any(test, feature = "fuzzing"))]
pub mod test_helpers;
pub mod transaction;
pub mod trusted_state;
pub mod validator_config;
pub mod validator_info;
pub mod validator_signer;
pub mod validator_verifier;
pub mod vm_status;
pub mod waypoint;
pub mod write_set;

pub use account_address::AccountAddress as PeerId;

#[cfg(test)]
mod unit_tests;

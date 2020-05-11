// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

// <Black magic>
// Increase recursion limit to allow for use of select! macro.
#![recursion_limit = "1024"]
// </Black magic>

// Public exports
pub use common::NetworkPublicKeys;
pub use interface::NetworkProvider;

pub mod common;
pub mod connectivity_manager;
pub mod counters;
pub mod error;
pub mod interface;
pub mod peer_manager;
pub mod protocols;
pub mod traits;
pub mod validator_network;

mod peer;
mod sink;
mod transport;

pub type DisconnectReason = peer::DisconnectReason;
pub type ConnectivityRequest = connectivity_manager::ConnectivityRequest;
pub type ProtocolId = protocols::wire::handshake::v1::ProtocolId;
pub type ProtocolCategory = protocols::wire::handshake::v1::ProtocolCategory;

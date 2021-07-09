// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! JSON RPC endpoint
//!
//! Used as public API interface for interacting with Full Nodes
//! It serves HTTP API requests from various external clients (such as wallets)
//!
//! Protocol specification: https://www.jsonrpc.org/specification
//!
//! Module organization:
//! ├── runtime.rs        # implementation of JSON RPC protocol over Websocket
//! ├── tests.rs          # tests

mod counters;
mod logging;

pub mod startup;

mod connection;
mod errors;
mod json_rpc;
mod subscription_types;
mod subscriptions;
mod transport;

#[cfg(test)]
mod tests;

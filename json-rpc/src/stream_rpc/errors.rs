// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("Could not convert {0} to string")]
    CouldNotStringifyMessage(String),
    #[error("Client#{0} is closed")]
    ClientAlreadyClosed(u64),
    #[error("Received disconnect request message from client")]
    ClientWantsToDisconnect,
    #[error("Transport error: {0}")]
    TransportError(String),
}

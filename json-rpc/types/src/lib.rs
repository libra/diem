// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

pub mod stream;

pub mod errors;
pub mod request;
pub mod response;
pub mod views;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum JsonRpcVersion {
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum Id {
    /// Numeric id
    Number(u64),
    /// String id
    String(Box<str>),
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Id::Number(ref v) => std::fmt::Debug::fmt(v, f),
            Id::String(ref v) => std::fmt::Debug::fmt(v, f),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Submit,
    GetMetadata,
    GetAccount,
    GetTransactions,
    GetAccountTransaction,
    GetAccountTransactions,
    GetEvents,
    GetCurrencies,
    GetNetworkStatus,

    //
    // Experimental APIs
    //
    GetStateProof,
    GetAccumulatorConsistencyProof,
    GetAccountStateWithProof,
    GetTransactionsWithProofs,
    GetAccountTransactionsWithProofs,
    GetEventsWithProofs,
}

impl Method {
    pub fn as_str(&self) -> &str {
        match self {
            Method::Submit => "submit",
            Method::GetMetadata => "get_metadata",
            Method::GetAccount => "get_account",
            Method::GetTransactions => "get_transactions",
            Method::GetAccountTransaction => "get_account_transaction",
            Method::GetAccountTransactions => "get_account_transactions",
            Method::GetEvents => "get_events",
            Method::GetCurrencies => "get_currencies",
            Method::GetNetworkStatus => "get_network_status",
            Method::GetStateProof => "get_state_proof",
            Method::GetAccumulatorConsistencyProof => "get_accumulator_consistency_proof",
            Method::GetAccountStateWithProof => "get_account_state_with_proof",
            Method::GetTransactionsWithProofs => "get_transactions_with_proofs",
            Method::GetAccountTransactionsWithProofs => "get_account_transactions_with_proofs",
            Method::GetEventsWithProofs => "get_events_with_proofs",
        }
    }
}

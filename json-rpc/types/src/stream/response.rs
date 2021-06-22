// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    errors::JsonRpcError,
    stream::request::StreamMethod,
    views::{EventView, TransactionView},
    Id, JsonRpcVersion,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum StreamJsonRpcResponseView {
    Transaction(TransactionView),
    Event(EventView),
    SubscribeResult(SubscribeResult),
}

impl StreamJsonRpcResponseView {
    fn from_method(
        method: &StreamMethod,
        value: serde_json::Value,
    ) -> Result<StreamJsonRpcResponseView, serde_json::Error> {
        // The first message in a stream is a `SubscribeResult`
        if let Some(_) = value.get("status") {
            return Ok(Self::SubscribeResult(serde_json::from_value(value)?));
        }
        Ok(match method {
            StreamMethod::SubscribeToTransactions => {
                Self::Transaction(serde_json::from_value(value)?)
            }
            StreamMethod::SubscribeToEvents => Self::Event(serde_json::from_value(value)?),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StreamJsonRpcResponse {
    pub jsonrpc: JsonRpcVersion,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl StreamJsonRpcResponse {
    pub fn parse_result(
        &self,
        method: &StreamMethod,
    ) -> Result<Option<StreamJsonRpcResponseView>, serde_json::Error> {
        Ok(match self.result.clone() {
            None => None,
            Some(result) => Some(StreamJsonRpcResponseView::from_method(method, result)?),
        })
    }

    pub fn result(id: Option<Id>, result: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id,
            result,
            error: None,
        }
    }

    pub fn error(id: Option<Id>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id,
            result: None,
            error: Some(error),
        }
    }
}

impl From<StreamJsonRpcResponse> for serde_json::Value {
    fn from(response: StreamJsonRpcResponse) -> Self {
        serde_json::to_value(&response).unwrap()
    }
}

impl FromStr for StreamJsonRpcResponse {
    type Err = serde_json::Error;

    fn from_str(string: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(string)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum SubscriptionResult {
    #[serde(rename = "OK")]
    OK,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SubscribeResult {
    pub status: SubscriptionResult,
    pub transaction_version: u64,
}

impl SubscribeResult {
    pub fn ok(transaction_version: u64) -> Self {
        Self {
            status: SubscriptionResult::OK,
            transaction_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diem_crypto::HashValue;
    use crate::views::TransactionDataView;
    use crate::views::BytesView;
    use crate::views::VMStatusView;

    fn response_view_helper(method: &StreamMethod, input: String) -> StreamJsonRpcResponseView {
        let response: StreamJsonRpcResponse =
            serde_json::from_str(&input).expect("Could not parse input");
        assert_eq!(
            response.id.clone().expect("Expected ID"),
            Id::String(Box::from("my-id"))
        );
        assert_eq!(response.jsonrpc, JsonRpcVersion::V2);

        let result = response
            .parse_result(method)
            .expect("Err when parsing result")
            .expect("None when parsing result");

        result
    }

    #[test]
    fn test_ok_result_parsing() {
        let input = serde_json::json!({"jsonrpc": "2.0", "id": "my-id", "result": {"status": "OK", "transaction_version": 77}}).to_string();
        let result = response_view_helper(&StreamMethod::SubscribeToTransactions, input);

        let expected = StreamJsonRpcResponseView::SubscribeResult(SubscribeResult {
            status: SubscriptionResult::OK,
            transaction_version: 77,
        });
        assert_eq!(result, expected);
    }

    #[test]
    fn test_data_result_parsing() {
        // 022099ba24b9f96dc26b96e84e80d19e861fb5f47a54182286126787f36e382b3b567c00000000000000fa4009ba5fc5050001652f27413456938eaa059f65231647cc652f27413456938eaa059f65231647cc
        let input = serde_json::json!({"jsonrpc":"2.0","id":"my-id","result":{"version":124,"transaction":{"type":"blockmetadata","timestamp_usecs":1624389817286906 as u64},"hash":"496176cd664651d81673832598c2dcdc47e9d2f900121a464351610bfa6d29fa","bytes":"0000","events":[],"vm_status":{"type":"executed"},"gas_used":100000000}}).to_string();
        let result = response_view_helper(&StreamMethod::SubscribeToTransactions, input);

        let expected = StreamJsonRpcResponseView::Transaction(TransactionView {
            version: 124,
            transaction: TransactionDataView::BlockMetadata { timestamp_usecs: 1624389817286906 },
            hash: HashValue::from_hex("496176cd664651d81673832598c2dcdc47e9d2f900121a464351610bfa6d29fa").expect("Could not parse HashValue hex"),
            bytes: BytesView::from(vec![0, 0]),
            events: vec![],
            vm_status: VMStatusView::Executed,
            gas_used: 100000000,
        });
        assert_eq!(result, expected);
    }
}

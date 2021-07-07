// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{errors::JsonRpcError, request::RawJsonRpcRequest, Id, JsonRpcVersion};
use diem_types::event::EventKey;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StreamJsonRpcRequest {
    jsonrpc: JsonRpcVersion,
    #[serde(flatten)]
    pub method_request: StreamMethodRequest,
    pub id: Id,
}

impl StreamJsonRpcRequest {
    pub fn new(method_request: StreamMethodRequest, id: Id) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            method_request,
            id,
        }
    }

    fn finish_parsing(
        jsonrpc: serde_json::Value,
        method: serde_json::Value,
        params: serde_json::Value,
        id: Id,
    ) -> Result<Self, (JsonRpcError, Option<StreamMethod>, Option<Id>)> {
        let jsonrpc: JsonRpcVersion = serde_json::from_value(jsonrpc)
            .map_err(|_| (JsonRpcError::invalid_request(), None, Some(id.clone())))?;
        let method: StreamMethod = serde_json::from_value(method)
            .map_err(|_| (JsonRpcError::method_not_found(), None, Some(id.clone())))?;
        let method_request = StreamMethodRequest::from_value(method, params).map_err(|_| {
            (
                JsonRpcError::invalid_params(method.as_str()),
                Some(method),
                Some(id.clone()),
            )
        })?;

        Ok(StreamJsonRpcRequest {
            jsonrpc,
            method_request,
            id,
        })
    }

    pub fn method_name(&self) -> &'static str {
        self.method_request.method_name()
    }
}

impl FromStr for StreamJsonRpcRequest {
    type Err = (JsonRpcError, Option<StreamMethod>, Option<Id>);

    fn from_str(string: &str) -> Result<Self, (JsonRpcError, Option<StreamMethod>, Option<Id>)> {
        let RawJsonRpcRequest {
            jsonrpc,
            method,
            params,
            id,
        } = serde_json::from_str(string)
            .map_err(|_| (JsonRpcError::invalid_format(), None, None))?;
        StreamJsonRpcRequest::finish_parsing(jsonrpc, method, params, id)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "method", content = "params")]
pub enum StreamMethodRequest {
    SubscribeToTransactions(SubscribeToTransactionsParams),
    SubscribeToEvents(SubscribeToEventsParams),
    Unsubscribe,
}

impl StreamMethodRequest {
    pub fn method_name(&self) -> &'static str {
        self.method().as_str()
    }

    pub fn from_value(
        method: StreamMethod,
        value: serde_json::Value,
    ) -> Result<Self, serde_json::Error> {
        let method_request = match method {
            StreamMethod::SubscribeToTransactions => {
                StreamMethodRequest::SubscribeToTransactions(serde_json::from_value(value)?)
            }
            StreamMethod::SubscribeToEvents => {
                StreamMethodRequest::SubscribeToEvents(serde_json::from_value(value)?)
            }
            StreamMethod::Unsubscribe => StreamMethodRequest::Unsubscribe,
        };

        Ok(method_request)
    }

    pub fn method(&self) -> StreamMethod {
        match self {
            StreamMethodRequest::SubscribeToTransactions(_) => {
                StreamMethod::SubscribeToTransactions
            }
            StreamMethodRequest::SubscribeToEvents(_) => StreamMethod::SubscribeToEvents,
            StreamMethodRequest::Unsubscribe => StreamMethod::Unsubscribe,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMethod {
    SubscribeToTransactions,
    SubscribeToEvents,
    Unsubscribe,
}

impl StreamMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            StreamMethod::SubscribeToTransactions => "subscribe_to_transactions",
            StreamMethod::SubscribeToEvents => "subscribe_to_events",
            StreamMethod::Unsubscribe => "unsubscribe",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SubscribeToEventsParams {
    pub event_key: EventKey,
    pub event_seq_num: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SubscribeToTransactionsParams {
    pub starting_version: u64,
    pub include_events: Option<bool>,
}

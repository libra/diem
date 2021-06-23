// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::Result;
use async_trait::async_trait;
use diem_json_rpc_types::{
    stream::{
        request::{
            StreamJsonRpcRequest, StreamMethodRequest, SubscribeToEventsParams,
            SubscribeToTransactionsParams,
        },
        response::StreamJsonRpcResponse,
    },
    Id,
};
use diem_types::event::EventKey;

/// Interface for various transports
#[async_trait]
pub trait StreamingClientTransport: std::marker::Send + Sync {
    async fn send(&mut self, request_json: String) -> Result<()>;

    fn get_next_id(&self) -> Id;

    async fn get_message(&mut self) -> Result<StreamJsonRpcResponse>;

    async fn send_method_request(
        &mut self,
        request: StreamMethodRequest,
        id: Option<Id>,
    ) -> Result<Id> {
        let id = id.unwrap_or_else(|| self.get_next_id());
        let request = StreamJsonRpcRequest::new(request, id);
        self.send_request(&request).await
    }

    async fn send_request(&mut self, request: &StreamJsonRpcRequest) -> Result<Id> {
        let json = serde_json::to_string(&request)?;
        self.send(json).await?;
        Ok(request.id.clone())
    }

    async fn subscribe_transactions(
        &mut self,
        starting_version: u64,
        include_events: Option<bool>,
        id: Option<Id>,
    ) -> Result<Id> {
        let request = StreamMethodRequest::SubscribeToTransactions(SubscribeToTransactionsParams {
            starting_version,
            include_events,
        });
        self.send_method_request(request, id).await
    }

    async fn subscribe_events(
        &mut self,
        event_key: EventKey,
        event_seq_num: u64,
        id: Option<Id>,
    ) -> Result<Id> {
        let request = StreamMethodRequest::SubscribeToEvents(SubscribeToEventsParams {
            event_key,
            event_seq_num,
        });
        self.send_method_request(request, id).await
    }
}

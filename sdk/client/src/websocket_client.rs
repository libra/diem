// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use super::USER_AGENT;
use crate::{
    views::{EventView, TransactionView},
    Error, Result,
};
use diem_types::event::EventKey;
use std::{
    sync::{atomic::AtomicU64, Arc},
    time::Duration,
};

use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{self, handshake::client::Request, protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

use diem_json_rpc_types::{
    stream::request::{
        StreamJsonRpcRequest, StreamMethodRequest, SubscribeToEventsParams,
        SubscribeToTransactionsParams,
    },
    Id,
};

pub struct WebsocketClient {
    stream: Arc<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>,
    next_id: AtomicU64,
}

impl WebsocketClient {
    pub async fn new<T: Into<String>>(
        url: T,
        websocket_config: Option<WebSocketConfig>,
    ) -> Result<Self, tungstenite::Error> {
        let request = Request::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
            .get(url.into())
            .header(reqwest::header::USER_AGENT, USER_AGENT);

        let (stream, _) = connect_async_with_config(request, websocket_config).await?;

        Ok(Self {
            stream: Arc::new(stream),
            next_id: AtomicU64::new(0),
        })
    }

    pub async fn subscribe_transactions(
        &mut self,
        starting_version: u64,
        include_events: Option<bool>,
    ) -> Result<()> {
        //Result<Response<Vec<TransactionView>>>
        let request = StreamMethodRequest::SubscribeToTransactions(SubscribeToTransactionsParams {
            starting_version,
            include_events,
        });
        self.send_method_request(request).await
    }

    pub async fn subscribe_events(
        &mut self,
        event_key: EventKey,
        event_seq_num: u64,
    ) -> Result<()> {
        // Result<Response<Vec<EventView>>>
        let request = StreamMethodRequest::SubscribeToEvents(SubscribeToEventsParams {
            event_key,
            event_seq_num,
        });
        self.send_method_request(request).await
    }

    pub async fn send_method_request(&mut self, request: StreamMethodRequest) -> Result<()> {
        let id = Id::Number(self.get_next_id());
        let request = StreamJsonRpcRequest::new(request, id);
        self.send_request(&request).await
    }

    pub async fn send_request(&mut self, request: &StreamJsonRpcRequest) -> Result<()> {
        self.send(serde_json::to_string(&request)?).await
    }

    pub async fn send(&mut self, request_json: String) -> Result<()> {
        self.stream.send(Message::text(request_json)).await?;
        Ok(())
    }

    fn get_next_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /*
    async fn send_without_retry<T: DeserializeOwned>(
        &self,
        request: &JsonRpcRequest,
        ignore_stale: bool,
    ) -> Result<Response<T>> {
        let resp: diem_json_rpc_types::response::JsonRpcResponse = self.send_impl(&request).await?;

        let (id, state, result) = validate(&self.state, req_state.as_ref(), &resp, ignore_stale)?;

        if request.id() != id {
            return Err(Error::rpc_response("invalid response id"));
        }

        let inner = serde_json::from_value(result).map_err(Error::decode)?;
        Ok(Response::new(inner, state))
    }
     */
}

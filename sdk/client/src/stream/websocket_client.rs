// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    Error, Result, USER_AGENT,
};
use diem_types::event::EventKey;
use futures::{SinkExt, StreamExt};
use std::{
    str::FromStr,
    sync::{atomic::AtomicU64},
};

use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        handshake::client::Request, protocol::WebSocketConfig, Message,
    },
    MaybeTlsStream, WebSocketStream,
};

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

pub struct WebsocketClient {
    stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    next_id: AtomicU64,
}

impl WebsocketClient {
    pub async fn new<T: Into<String>>(
        url: T,
        websocket_config: Option<WebSocketConfig>,
    ) -> Result<Self, Error> {
        let request = Request::builder()
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::CONTENT_LENGTH, 1_000)
            .uri(url.into())
            .method("GET")
            .body(())
            .map_err(|e| Error::from_http_error(e))?;

        let (stream, _) = connect_async_with_config(request, websocket_config)
            .await
            .map_err(|e| Error::from_tungstenite_error(e))?;

        Ok(Self {
            stream,
            next_id: AtomicU64::new(0),
        })
    }

    pub async fn subscribe_transactions(
        &mut self,
        starting_version: u64,
        include_events: Option<bool>,
    ) -> Result<()> {
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
        let json = serde_json::to_string(&request)?;
        self.send(json).await
    }

    pub async fn send(&mut self, request_json: String) -> Result<()> {
        self.stream
            .send(Message::text(request_json))
            .await
            .map_err(Error::encode)?;
        Ok(())
    }

    pub async fn get_message(mut self) -> Result<StreamJsonRpcResponse, Error> {
        loop {
            let msg = self.stream.next().await;
            let msg = match msg {
                None => return Err(Error::connection_closed(None::<Error>)),
                Some(msg) => msg,
            }
                .map_err(|e| Error::from_tungstenite_error(e))?;

            if msg.is_text() {
                let msg = msg
                    .to_text()
                    .map_err(|e| Error::from_tungstenite_error(e))?;
                return Ok(StreamJsonRpcResponse::from_str(msg)?);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use diem_json_rpc_types::stream::request::StreamMethod;
    use diem_json_rpc_types::stream::response::StreamJsonRpcResponseView;

    #[tokio::test]
    async fn test_connecting() {
        let input = serde_json::json!({"jsonrpc": "2.0", "id": "my-id", "result": {"status": "OK", "transaction_version": 77}}).to_string();
        let mut client = WebsocketClient::new("ws://127.0.0.1:8080/v1/stream/ws", None).await
            .unwrap_or_else(|e| panic!("Error connecting to WS endpoint: {}", e));

        let params = SubscribeToTransactionsParams { starting_version: 0, include_events: None };
        let method_request = StreamMethodRequest::SubscribeToTransactions(params);

        client
            .send_method_request(method_request)
            .await
            .unwrap_or_else(|e| panic!("Error sending message: {}", e));


        let message = client.get_message()
            .await
            .unwrap_or_else(|e| panic!("1: Error parsing response: {}", e));

        println!("resp: {:?}", message);

        let result = message.parse_result(&StreamMethod::SubscribeToTransactions)
            .unwrap_or_else(|e| panic!("1: Error parsing result: {}", e))
            .unwrap_or_else(|e| panic!("1: Result was None"));

        let res = match result {
            StreamJsonRpcResponseView::SubscribeResult(res) => res,
            _ => panic!("1. StreamJsonRpcResponseView was not a SubscribeResult")
        };
    }
}

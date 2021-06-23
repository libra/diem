// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{stream::traits::StreamingClientTransport, Error, Result, USER_AGENT};
use futures::{SinkExt, StreamExt};
use std::{str::FromStr, sync::atomic::AtomicU64};

use async_trait::async_trait;
use diem_json_rpc_types::{stream::response::StreamJsonRpcResponse, Id};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{handshake::client::Request, protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
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
            .map_err(Error::from_http_error)?;

        let (stream, _) = connect_async_with_config(request, websocket_config)
            .await
            .map_err(Error::from_tungstenite_error)?;

        Ok(Self {
            stream,
            next_id: AtomicU64::new(0),
        })
    }
}

#[async_trait]
impl StreamingClientTransport for WebsocketClient {
    async fn send(&mut self, request_json: String) -> Result<()> {
        self.stream
            .send(Message::text(request_json))
            .await
            .map_err(Error::encode)?;
        Ok(())
    }

    fn get_next_id(&self) -> Id {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Id::Number(id)
    }

    /// Loops until one of the following happens:
    /// 1. `self.stream.next().await` returns `None`: this happens if a stream is closed
    /// 2. `self.stream.next().await` returns `Err`: this is generally a transport error, and for
    ///     most intents and purposes the stream can be considered closed
    /// 3. `self.stream.next().await` returns `Message`: this may or may not be valid JSON from the
    ///     server, so we need to do a bit of validation:
    ///     1. `msg.is_text()`: we ignore `Ping`/`Pong`, `Binary`, etc message types
    ///     2. `msg.to_text()`: Ensures the message is valid UTF8
    ///     3. `StreamJsonRpcResponse::from_str(msg)`: this is serde deserialization, which validates
    ///         that the JSON is well formed, and matches the given struct
    ///
    async fn get_message(&mut self) -> Result<StreamJsonRpcResponse> {
        loop {
            let msg = self.stream.next().await;
            let msg = match msg {
                None => return Err(Error::connection_closed(None::<Error>)),
                Some(msg) => msg,
            }
            .map_err(Error::from_tungstenite_error)?;

            if msg.is_text() {
                let msg = msg
                    .to_text()
                    .map_err(Error::from_tungstenite_error)?;
                return Ok(StreamJsonRpcResponse::from_str(msg)?);
            }
        }
    }
}

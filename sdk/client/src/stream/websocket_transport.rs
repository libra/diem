// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    stream::{streaming_client::StreamingClientReceiver},
    StreamError, StreamResult, USER_AGENT,
};
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::{
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};

use diem_json_rpc_types::{stream::{request::{StreamJsonRpcRequest, StreamMethodRequest}, response::StreamJsonRpcResponse}, Id};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{handshake::client::Request, protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};


pub struct WebsocketTransport {
    stream: Option<SplitStream<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>>,
    sink: SplitSink<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, Message>,
    next_id: AtomicU64,
    channel_task: Option<JoinHandle<()>>,
}

impl WebsocketTransport {
    pub async fn new<T: Into<String>>(
        url: T,
        websocket_config: Option<WebSocketConfig>,
    ) -> StreamResult<Self> {
        let request = Request::builder()
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::CONTENT_LENGTH, 1_000)
            .uri(url.into())
            .method("GET")
            .body(())
            .map_err(StreamError::from_http_error)?;

        let (stream, _) = connect_async_with_config(request, websocket_config)
            .await
            .map_err(StreamError::from_tungstenite_error)?;

        let (sink, stream) = stream.split();

        Ok(Self {
            stream: Some(stream),
            sink,
            next_id: AtomicU64::new(0),
            channel_task: None,
        })
    }

    pub async fn send(&mut self, request_json: String) -> StreamResult<()> {
        self.sink
            .send(Message::text(request_json))
            .await
            .map_err(StreamError::encode)?;
        Ok(())
    }

    pub async fn send_method_request(
        &mut self,
        request: StreamMethodRequest,
        id: Option<Id>,
    ) -> StreamResult<Id> {
        let id = id.unwrap_or_else(|| self.get_next_id());
        let request = StreamJsonRpcRequest::new(request, id.clone());
        self.send_request(&request).await
    }

    pub async fn send_request(&mut self, request: &StreamJsonRpcRequest) -> StreamResult<Id> {
        let json = serde_json::to_string(&request)?;
        self.send(json).await?;
        Ok(request.id.clone())
    }

    pub fn get_next_id(&self) -> Id {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        Id::Number(id)
    }

    /// This consumes the websocket stream (but not the sink)
    /// Possibilities:
    /// 1. `stream.next().await` returns `None`: this happens if a stream is closed
    /// 2. `stream.next().await` returns `Some(Err)`: this is generally a transport error, and the
    ///     stream can be considered closed
    /// 3. `stream.next().await` returns `Some(Ok(Message))`: this may or may not be valid JSON from
    ///     the server, so we need to do a bit of validation:
    ///     1. `msg.is_text()` must be `true`: we ignore `Ping`/`Pong`, `Binary`, etc message types
    ///     2. `msg.to_text()` must be `Ok(String)`: Ensures the message is valid UTF8
    ///     3. `StreamJsonRpcResponse::from_str(msg)` must be `Ok(StreamJsonRpcResponse)`: this is
    ///         serde deserialization, which validates that the JSON is well formed, and matches the
    ///         `StreamJsonRpcResponse` struct
    ///
    pub fn get_stream(mut self) -> (StreamingClientReceiver, Self) {
        let (sender, receiver) = mpsc::channel(100);

        let mut stream = self
            .stream
            .expect("Stream is `None`: it has already been consumed");
        self.stream = None;

        self.channel_task = Some(tokio::task::spawn(async move {
            loop {
                match stream.next().await {
                    None => {
                        sender
                            .send(Err(StreamError::connection_closed(None::<StreamError>)))
                            .await
                            .ok();
                        continue;
                    }
                    Some(msg) => match msg {
                        Ok(msg) => {
                            if msg.is_text() {
                                let msg = match msg
                                    .to_text()
                                    .map_err(StreamError::from_tungstenite_error)
                                {
                                    Ok(msg) => msg,
                                    Err(e) => {
                                        sender.send(Err(e)).await.ok();
                                        continue;
                                    }
                                };
                                match StreamJsonRpcResponse::from_str(msg) {
                                    Ok(msg) => sender.send(Ok(msg)).await.ok(),
                                    Err(e) => sender.send(Err(StreamError::from(e))).await.ok(),
                                };
                            }
                        }
                        Err(e) => {
                            sender
                                .send(Err(StreamError::from_tungstenite_error(e)))
                                .await
                                .ok();
                            continue;
                        }
                    },
                };
            }
        }));

        (receiver, self)
    }
}

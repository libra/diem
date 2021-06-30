// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{StreamError, StreamResult, stream::websocket_transport::WebsocketTransport};
use diem_json_rpc_types::{stream::{request::{StreamMethodRequest, SubscribeToEventsParams, SubscribeToTransactionsParams}, response::StreamJsonRpcResponse}, Id};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    sync::{mpsc, RwLock},
    task::JoinHandle,
};
use diem_types::event::EventKey;
use futures::Stream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;


pub type StreamingClientReceiver = mpsc::Receiver<StreamResult<StreamJsonRpcResponse>>;
pub type StreamingClientSender = mpsc::Sender<StreamResult<StreamJsonRpcResponse>>;
pub type SubscriptionStreamResult = StreamResult<SubscriptionStream>;


pub struct SubscriptionStream {
    pub id: Id,
    stream: StreamingClientReceiver,
}

impl SubscriptionStream {
    fn new(id: Id, stream: StreamingClientReceiver) -> Self {
        Self { id, stream }
    }
}

impl Stream for SubscriptionStream {
    type Item = StreamResult<StreamJsonRpcResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.poll_recv(cx)
    }
}

pub struct SubscriptionChannelManager {
    stream: StreamingClientReceiver,
    subscriptions: Arc<RwLock<HashMap<Id, StreamingClientSender>>>,
    error_sender: StreamingClientSender,
}

impl SubscriptionChannelManager {
    pub fn new(
        stream: StreamingClientReceiver,
        subscriptions: Arc<RwLock<HashMap<Id, StreamingClientSender>>>,
        error_sender: StreamingClientSender,
    ) -> Self {
        Self {
            stream,
            subscriptions,
            error_sender,
        }
    }

    /// Returning an actual `Err` from here signals some kind of connection problem
    pub async fn handle_next_message(&mut self) -> StreamResult<()> {
        // make sure we time out if there is no message immediately available, so the lock on `client` doesn't get held indefinitely
        let msg = match self.stream.recv().await {
            None => return Err(StreamError::connection_closed(None::<StreamError>)),
            Some(msg) => msg,
        };

        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                self.error_sender.send(Err(e)).await.ok();
                return Ok(());
            }
        };

        // If there is no ID, we can't route this to anywhere but the catchall '_error' channel
        let id = match &msg.id {
            Some(id) => id,
            None => {
                self.error_sender.send(Ok(msg)).await.ok();
                return Ok(());
            }
        };
        // Send the message to the respective channel
        let id = id.clone();
        match self.subscriptions.read().await.get(&id) {
            // If we could not send the subscription, or the channel is closed, make sure to clean up the subscription
            Some(sender) => match sender.send(Ok(msg)).await {
                Err(e) => {
                    self.unsubscribe(&id).await;
                    Err(StreamError::connection_closed(Some(e)))
                }
                Ok(_) => Ok(()),
            },
            // No such subscription exists
            None => {
                self.error_sender
                    .send(Err(StreamError::subscription_id_not_found(
                        Some(msg),
                        None::<StreamError>,
                    )))
                    .await
                    .ok();
                self.unsubscribe(&id).await;
                Ok(())
            }
        }
    }

    pub async fn unsubscribe(&self, id: &Id) {
        self.subscriptions.write().await.remove(id);
    }
}

/// This API is experimental and subject to change
/// Documentation is in /json-rpc/src/stream_rpc/README.md
pub struct StreamingClient {
    client: Arc<RwLock<WebsocketTransport>>,
    subscriptions: Arc<RwLock<HashMap<Id, StreamingClientSender>>>,
    channel_size: usize,
    error_sender: StreamingClientSender,
    channel_task: Option<JoinHandle<StreamResult<()>>>,
}

impl StreamingClient {
    pub async fn new<T: Into<String>>(
        url: T, channel_size: usize, websocket_config: Option<WebSocketConfig>) -> StreamResult<(Self, StreamingClientReceiver)> {
        let client = WebsocketTransport::new(url, websocket_config).await?;
        let subscriptions = Arc::new(RwLock::new(HashMap::new()));
        let (error_sender, error_receiver) =
            mpsc::channel::<StreamResult<StreamJsonRpcResponse>>(channel_size);

        subscriptions
            .write()
            .await
            .insert(Id::String(Box::from("_errors")), error_sender.clone());

        let (stream, client) = client.get_stream();

        let mut sct = Self {
            client: Arc::new(RwLock::new(client)),
            subscriptions,
            channel_size,
            error_sender,
            channel_task: None,
        };

        sct.start_channel_task(stream);

        Ok((sct, error_receiver))
    }

    #[allow(unused)]
    pub async fn subscribe_transactions(
        &mut self,
        starting_version: u64,
        include_events: Option<bool>,
    ) -> SubscriptionStreamResult {
        let request = StreamMethodRequest::SubscribeToTransactions(SubscribeToTransactionsParams {
            starting_version,
            include_events,
        });
        self.send_subscription(request).await
    }

    #[allow(unused)]
    pub async fn subscribe_events(
        &mut self,
        event_key: EventKey,
        event_seq_num: u64,
    ) -> SubscriptionStreamResult {
        let request = StreamMethodRequest::SubscribeToEvents(SubscribeToEventsParams {
            event_key,
            event_seq_num,
        });
        self.send_subscription(request).await
    }

    pub async fn register_subscription(&self, id: Id) -> StreamResult<StreamingClientReceiver> {
        let (sender, receiver) = mpsc::channel(self.channel_size);
        if self.subscriptions.read().await.get(&id).is_some() {
            return Err(StreamError::subscription_id_already_used(
                None::<StreamError>,
            ));
        }
        self.subscriptions.write().await.insert(id, sender);
        Ok(receiver)
    }

    pub async fn send_subscription(&mut self, request: StreamMethodRequest) -> SubscriptionStreamResult {
        let subscription_stream = self.get_and_register_id().await?;
        let res = self
            .client
            .write()
            .await
            .send_method_request(request, Some(subscription_stream.id.clone()))
            .await;

        self
            .maybe_clear_subscription(&subscription_stream.id, res)
            .await?;
        Ok(subscription_stream)
    }

    fn start_channel_task(&mut self, stream: StreamingClientReceiver) {
        let mut scm = SubscriptionChannelManager::new(
            stream,
            self.subscriptions.clone(),
            self.error_sender.clone(),
        );
        self.channel_task = Some(tokio::task::spawn(async move {
            loop {
                scm.handle_next_message().await?;
            }
        }));
    }

    async fn maybe_clear_subscription(&self, id: &Id, res: StreamResult<Id>) -> StreamResult<Id> {
        match res {
            Ok(id) => Ok(id),
            Err(e) => {
                self.clear_subscription(&id).await;
                Err(e)
            }
        }
    }

    async fn clear_subscription(&self, id: &Id) {
        self.subscriptions.write().await.remove(id);
    }

    async fn get_and_register_id(&self) -> SubscriptionStreamResult {
        let id = self.client.read().await.get_next_id();
        let receiver = self.register_subscription(id.clone()).await?;
        Ok(SubscriptionStream::new(id, receiver))
    }
}

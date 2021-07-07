// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{stream::websocket_transport::WebsocketTransport, StreamError, StreamResult};
use diem_json_rpc_types::{
    stream::{
        request::{StreamMethodRequest, SubscribeToEventsParams, SubscribeToTransactionsParams},
        response::StreamJsonRpcResponse,
    },
    Id,
};
use diem_types::event::EventKey;
use futures::Stream;
use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    sync::{mpsc, RwLock},
    time::timeout,
};

use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tracing::{debug, warn, trace};

pub(crate) type StreamingClientReceiver = mpsc::Receiver<StreamResult<StreamJsonRpcResponse>>;
pub(crate) type StreamingClientSender = mpsc::Sender<StreamResult<StreamJsonRpcResponse>>;

struct SubscriptionSender {
    pub id: Id,
    pub sender: StreamingClientSender,
}

impl SubscriptionSender {
    pub fn new(id: Id, sender: StreamingClientSender) -> Self {
        Self {
            id,
            sender,
        }
    }
}

pub struct SubscriptionStream {
    id: Id,
    stream: StreamingClientReceiver,
}

impl SubscriptionStream {
    fn new(id: Id, stream: StreamingClientReceiver) -> Self {
        Self { id, stream }
    }

    pub fn id(&self) -> &Id {
        &self.id
    }

    pub async fn wait_for_msg(&mut self) -> StreamResult<StreamResult<StreamJsonRpcResponse>> {
        match self.stream.recv().await {
            None => Err(StreamError::connection_closed(None::<StreamError>)),
            Some(msg) => Ok(msg),
        }
    }
}

impl Stream for SubscriptionStream {
    type Item = StreamResult<StreamJsonRpcResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.poll_recv(cx)
    }
}

/// Configuration options for the `Streaming Client`
pub struct StreamingClientConfig {
    /// The buffer of incoming messages per subscription
    pub channel_size: usize,
    /// How long to wait for an incoming message before considering a subscription 'timed out'
    pub ok_timeout_millis: u64,
}

impl Default for StreamingClientConfig {
    fn default() -> Self {
        Self {
            channel_size: 10,
            ok_timeout_millis: 1_000,
        }
    }
}

/// This API is experimental and subject to change
/// Documentation is in /json-rpc/src/stream_rpc/README.md
#[derive(Clone)]
pub struct StreamingClient {
    client: Arc<RwLock<WebsocketTransport>>,
    subscriptions: Arc<RwLock<HashMap<Id, SubscriptionSender>>>,
    stream: Arc<RwLock<StreamingClientReceiver>>,
    config: Arc<StreamingClientConfig>,
}

impl StreamingClient {
    pub async fn new<T: Into<String>>(
        url: T,
        config: StreamingClientConfig,
        websocket_config: Option<WebSocketConfig>,
    ) -> StreamResult<Self> {
        let client = WebsocketTransport::new(url, websocket_config).await?;
        let subscriptions = Arc::new(RwLock::new(HashMap::new()));

        let (stream, client) = client.get_stream();

        let mut sct = Self {
            client: Arc::new(RwLock::new(client)),
            subscriptions,
            stream: Arc::new(RwLock::new(stream)),
            config: Arc::new(config),
        };

        sct.start_channel_task();

        Ok(sct)
    }

    #[allow(unused)]
    pub async fn subscribe_transactions(
        &mut self,
        starting_version: u64,
        include_events: Option<bool>,
    ) -> StreamResult<SubscriptionStream> {
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
    ) -> StreamResult<SubscriptionStream> {
        let request = StreamMethodRequest::SubscribeToEvents(SubscribeToEventsParams {
            event_key,
            event_seq_num,
        });
        self.send_subscription(request).await
    }

    pub async fn send_subscription(
        &mut self,
        request: StreamMethodRequest,
    ) -> StreamResult<SubscriptionStream> {
        let mut subscription_stream = self.get_and_register_id().await?;
        let res = self
            .client
            .write()
            .await
            .send_method_request(request, Some(subscription_stream.id().clone()))
            .await;

        let id = match res {
            Ok(id) => id,
            Err(e) => {
                self.clear_subscription(&subscription_stream.id()).await;
                return Err(e);
            }
        };

        debug!("StreamingClient starting OkTimeout task for id: {:?}", &id);
        let duration = Duration::from_millis(self.config.ok_timeout_millis);
        // The `res??` handles the channel being closed before we get our first message
        let msg = match timeout(duration, subscription_stream.wait_for_msg()).await {
            Ok(res) => res??,
            Err(_) => {
                debug!("StreamingClient OkTimeout for id: {:?}", &id);
                self.clear_subscription(&id).await;
                return Err(StreamError::subscription_ok_timeout());
            }
        };

        if let Some(err) = msg.error {
            self.clear_subscription(&id).await;
            return Err(StreamError::subscription_json_rpc_error(err));
        }

        Ok(subscription_stream)
    }

    /// Returning an actual `Err` from here signals some kind of connection problem
    async fn handle_next_message(&mut self) -> StreamResult<()> {
        let msg = self.stream.write().await.recv().await;

        trace!("StreamingClient got message: {:?}", &msg);

        let msg = match msg {
            None => return Err(StreamError::connection_closed(None::<StreamError>)),
            Some(msg) => msg,
        };

        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                warn!("StreamingClient received error on channel: {:?}", e);
                return Ok(());
            }
        };

        // If there is no ID, we can't route this to anywhere but the catchall '_error' channel
        let id = match &msg.id {
            Some(id) => id,
            None => {
                warn!("StreamingClient got message without an ID: {:?}", &msg);
                return Ok(());
            }
        };
        // Send the message to the respective channel
        let id = id.clone();
        match self.subscriptions.read().await.get(&id) {
            // If we could not send the subscription, or the channel is closed, make sure to clean up the subscription
            Some(sender) => match sender.sender.send(Ok(msg.clone())).await {
                Err(e) => {
                    warn!(error=?&e, "StreamingClient could not forward message: {:?}", &msg);
                    self.clear_subscription(&id).await;
                    Err(StreamError::connection_closed(Some(e)))
                }
                Ok(_) => {
                    debug!("StreamingClient forwarded message: {:?}", &msg);
                    Ok(())
                }
            },
            // No such subscription exists
            None => {
                warn!(
                    "StreamingClient got message without matching subscription: {:?}",
                    &msg
                );
                Ok(())
            }
        }
    }

    async fn register_subscription(&self, id: Id) -> StreamResult<StreamingClientReceiver> {
        if self.subscriptions.read().await.get(&id).is_some() {
            return Err(StreamError::subscription_id_already_used(
                None::<StreamError>,
            ));
        }
        let (sender, receiver) = mpsc::channel(self.config.channel_size);

        self.subscriptions
            .write()
            .await
            .insert(id.clone(), SubscriptionSender::new(id, sender));
        Ok(receiver)
    }

    fn start_channel_task(&mut self) {
        debug!("StreamingClient starting channel task");
        let mut clone = self.clone();
        tokio::task::spawn(async move { while clone.handle_next_message().await.is_ok() {} });
    }

    async fn clear_subscription(&self, id: &Id) {
        debug!("StreamingClient clearing subscription: {:?}", &id);
        self.subscriptions.write().await.remove(id);
    }

    async fn get_and_register_id(&self) -> StreamResult<SubscriptionStream> {
        let id = self.client.read().await.get_next_id();
        let receiver = self.register_subscription(id.clone()).await?;
        Ok(SubscriptionStream::new(id, receiver))
    }
}

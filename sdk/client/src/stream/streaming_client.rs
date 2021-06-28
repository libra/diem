// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{stream::traits::StreamingClientTransport, Error, Result};
use diem_json_rpc_types::{stream::response::StreamJsonRpcResponse, Id};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{mpsc, RwLock},
    task::JoinHandle,
};

use diem_types::event::EventKey;

pub type StreamingClientReceiver = mpsc::Receiver<Result<StreamJsonRpcResponse>>;
pub type StreamingClientSender = mpsc::Sender<Result<StreamJsonRpcResponse>>;
pub type SubscriptionStreamResult = Result<(Id, StreamingClientReceiver)>;

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
    pub async fn handle_next_message(&mut self) -> Result<()> {
        // make sure we time out if there is no message immediately available, so the lock on `client` doesn't get held indefinitely
        let msg = match self.stream.recv().await {
            None => return Err(Error::connection_closed(None::<Error>)),
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
                    Err(Error::connection_closed(Some(e)))
                }
                Ok(_) => Ok(()),
            },
            // No such subscription exists
            None => {
                self.error_sender
                    .send(Err(Error::subscription_id_not_found(
                        Some(msg),
                        None::<Error>,
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

pub struct StreamingClient<C: StreamingClientTransport + 'static> {
    client: Arc<RwLock<C>>,
    subscriptions: Arc<RwLock<HashMap<Id, StreamingClientSender>>>,
    channel_size: usize,
    error_sender: StreamingClientSender,
    channel_task: Option<JoinHandle<Result<()>>>,
}

impl<C: StreamingClientTransport + 'static> StreamingClient<C> {
    pub async fn new(client: C, channel_size: usize) -> (Self, StreamingClientReceiver) {
        let subscriptions = Arc::new(RwLock::new(HashMap::new()));
        let (error_sender, error_receiver) =
            mpsc::channel::<Result<StreamJsonRpcResponse>>(channel_size);

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

        (sct, error_receiver)
    }

    #[allow(unused)]
    async fn subscribe_transactions(
        &mut self,
        starting_version: u64,
        include_events: Option<bool>,
    ) -> SubscriptionStreamResult {
        let (id, receiver) = self.get_and_register_id().await?;
        let res = self
            .client
            .write()
            .await
            .subscribe_transactions(starting_version, include_events, Some(id.clone()))
            .await;

        let id = self.maybe_clear_subscription(&id, res).await?;
        Ok((id, receiver))
    }

    #[allow(unused)]
    pub async fn subscribe_events(
        &mut self,
        event_key: EventKey,
        event_seq_num: u64,
    ) -> SubscriptionStreamResult {
        let (id, receiver) = self.get_and_register_id().await?;

        let mut res = self
            .client
            .write()
            .await
            .subscribe_events(event_key, event_seq_num, Some(id.clone()))
            .await;

        let id = self.maybe_clear_subscription(&id, res).await?;
        Ok((id, receiver))
    }

    pub async fn register_subscription(&self, id: Id) -> Result<StreamingClientReceiver> {
        let (sender, receiver) = mpsc::channel(self.channel_size);
        if self.subscriptions.read().await.get(&id).is_some() {
            return Err(Error::subscription_id_already_used(None::<Error>));
        }
        self.subscriptions.write().await.insert(id, sender);
        Ok(receiver)
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

    async fn maybe_clear_subscription(&self, id: &Id, res: Result<Id>) -> Result<Id> {
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
        Ok((id, receiver))
    }
}

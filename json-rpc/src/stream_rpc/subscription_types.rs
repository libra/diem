// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! # Subscription Handlers
//!
//! ## Implementing the Subscription Trait
//! In order to turn a struct containing subscription state into something capable of launching a
//! subscription, you must implement the `Subscription<ParamType, ReturnType>` trait.
//! `ParamType` is what will be used to deserialize JSONRPC Requests
//! `ReturnType` is what will be serialized and sent to the client
//!
//! To implement `Subscription` there are three methods you must define:
//! 1. `init(&mut self, helper: &SubscriptionHelper, params: &ParamType) -> Result<(), JsonRpcError>;`
//!     - This is where initial state is set up to keep track of what has been sent to the client.
//!       This could be any incremental id: a sequence number for events, the transaction version for transactions, etc.
//!     - Returning `Err(JsonRpcRequest)` is the way to handle an issue with a parameter value, or
//!       any other such case where a subscription may not be started or requested data may not be
//!       returned. Otherwise if the parameters are valid and a subscription could be created, simply
//!       return `Ok(())`
//!
//! 2. `next(&self, helper: &SubscriptionHelper, params: &ParamType) -> Vec<ParamType>;`
//!     Within this method, use existing state set up in the `init` function to fetch the next batch of data.
//!     The size of the batch should be determined by `helper.client.config.fetch_size`.
//!     If the function returns an array of data, each item will be serialized one by one and sent to the client.
//!     The `on_send` method (below) will be called for each item in the array.
//!     If the function returns an empty array, the thread will sleep for `client.config.poll_interval_ms`,
//!     increasing exponentially up to a ceiling, and will reset any time the function returns data.
//!
//! 3. `on_send(&mut self, item: &ParamType);`
//!     If the `next` function returns an array of items, after each the item is serialized and sent to the client
//!     it will be passed to this `on_send` method. This is the perfect place to update state fields
//!     which keep track of what has/hasn't been sent to the client.
//!

use crate::{
    errors::JsonRpcError,
    stream_rpc::{connection::ClientConnection, counters},
};
use diem_json_rpc_types::{
    stream::{request::StreamMethod, response::SubscribeResult},
    Id,
};
use diem_logger::debug;
use serde::Serialize;
use std::{iter::Map, sync::Arc, time::Duration};
use storage_interface::DbReader;
use tokio::task::JoinHandle;
use tokio_retry::strategy::ExponentialBackoff;

#[derive(Clone, Debug)]
pub struct SubscriptionConfig {
    pub fetch_size: u64,
    pub poll_interval_ms: u64,
    pub max_poll_interval_ms: u64,
    pub queue_size: usize,
}

type JitterBackoff = Map<ExponentialBackoff, fn(Duration) -> Duration>;

/// Jitter up to 80% of the actual value
pub fn jitter(duration: Duration) -> Duration {
    // Generate a float from 0.8<->1.0
    let jitter = 0.8 + rand::random::<f64>() / 5.0;
    duration.mul_f64(jitter)
}

pub fn create_backoff(poll_interval_ms: u64, max_poll_interval_ms: u64) -> JitterBackoff {
    ExponentialBackoff::from_millis(poll_interval_ms)
        .max_delay(tokio::time::Duration::from_millis(max_poll_interval_ms))
        .map(jitter)
}

#[derive(Clone)]
pub struct SubscriptionHelper {
    pub db: Arc<dyn DbReader>,
    pub client: ClientConnection,
    pub jsonrpc_id: Id,
    pub method: StreamMethod,
    pub backoff: JitterBackoff,
}

impl SubscriptionHelper {
    pub fn new(
        db: Arc<dyn DbReader>,
        client: ClientConnection,
        jsonrpc_id: Id,
        method: StreamMethod,
    ) -> Self {
        let poll_interval_ms = client.config.poll_interval_ms;
        let max_poll_interval_ms = client.config.max_poll_interval_ms;
        Self {
            db,
            client,
            jsonrpc_id,
            method,
            backoff: create_backoff(poll_interval_ms, max_poll_interval_ms),
        }
    }

    /// This is to be one of the first things that a new subscription task must do, to let the client
    /// know that the subscription has been created (some streams may be very sparse, or start with 0 items)
    pub async fn send_ok(&self) {
        counters::SUBSCRIPTION_OKS
            .with_label_values(&[
                self.client.connection_context.transport.as_str(),
                &self.method.as_str(),
                counters::RpcResult::Success.as_str(),
                self.client.connection_context.sdk_info.language.as_str(),
                &self.client.connection_context.sdk_info.version.to_string(),
            ])
            .inc();
        self.client
            .send_success(
                self.jsonrpc_id.clone(),
                &SubscribeResult::ok(self.db.get_latest_version().unwrap_or(0)),
            )
            .await
            .ok();
    }

    pub fn reset_backoff(&mut self) {
        let config = &self.client.config;
        self.backoff = create_backoff(config.poll_interval_ms, config.max_poll_interval_ms);
    }

    pub async fn sleep_wiggled(&mut self) {
        let sleepy_time = self.backoff.next().unwrap();
        debug!(
            "Client#{} Sleeping for {:?}ms as no results for {}",
            self.client.id,
            sleepy_time,
            self.method.as_str()
        );
        tokio::time::sleep(sleepy_time).await;
    }
}

pub trait Subscription<ParamType, ReturnType>: Send + Sync + Clone + Default + 'static
where
    ParamType: Send + Sync + Clone + 'static,
    ReturnType: Send + Sync + Serialize + Clone,
{
    fn init(&mut self, helper: &SubscriptionHelper, params: &ParamType)
        -> Result<(), JsonRpcError>;
    fn next(&self, helper: &SubscriptionHelper, params: &ParamType) -> Vec<ReturnType>;
    fn on_send(&mut self, item: Option<&ReturnType>);

    fn run(
        mut self,
        mut helper: SubscriptionHelper,
        params: ParamType,
    ) -> Result<JoinHandle<()>, JsonRpcError> {
        self.init(&helper, &params)?;

        Ok(tokio::spawn(async move {
            helper.send_ok().await;

            loop {
                let items = self.next(&helper, &params);

                debug!(
                    "Client#{}: fetched {} items for {}",
                    helper.client.id,
                    items.len(),
                    helper.method.as_str()
                );

                if items.is_empty() {
                    self.on_send(None);
                    helper.sleep_wiggled().await;
                    continue;
                }

                helper.reset_backoff();

                counters::SUBSCRIPTION_RPC_SENT
                    .with_label_values(&[
                        helper.client.connection_context.transport.as_str(),
                        helper.method.as_str(),
                        counters::RpcResult::Success.as_str(),
                        helper.client.connection_context.sdk_info.language.as_str(),
                        &helper
                            .client
                            .connection_context
                            .sdk_info
                            .version
                            .to_string(),
                    ])
                    .inc_by(items.len() as u64);

                for item in items {
                    match helper
                        .client
                        .send_success(helper.jsonrpc_id.clone(), &item)
                        .await
                    {
                        Err(e) => {
                            // client has disconnected
                            debug!("Client#{}: Send error: {:?}", &helper.client.id, e);
                            return;
                        }
                        Ok(_) => self.on_send(Some(&item)),
                    };
                }
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        stream_rpc::{
            errors::StreamError,
            tests::util::{create_client_connection, timeout},
        },
        tests::utils::MockDiemDB,
    };
    use serde::{Deserialize, Serialize};
    use std::time::Instant;
    use tokio::sync::mpsc::Receiver;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct SubscribeTestParams {
        pub is_valid: bool,
        pub items_to_send: Vec<Vec<TestView>>,
    }

    #[derive(Clone, Copy, Debug, Default)]
    pub struct TestSubscription {
        pub item_index: usize,
    }

    #[derive(Clone, Copy, Debug, Deserialize, Serialize)]
    pub struct TestView {
        pub value: u64,
    }

    impl Subscription<SubscribeTestParams, TestView> for TestSubscription {
        fn init(
            &mut self,
            _helper: &SubscriptionHelper,
            params: &SubscribeTestParams,
        ) -> Result<(), JsonRpcError> {
            // Pretend there's a problem with one of the params
            if params.is_valid {
                Ok(())
            } else {
                Err(JsonRpcError::invalid_param("not is_valid"))
            }
        }

        fn next(
            &self,
            _helper: &SubscriptionHelper,
            params: &SubscribeTestParams,
        ) -> Vec<TestView> {
            let items: Option<&Vec<TestView>> = params.items_to_send.get(self.item_index);
            match items {
                None => vec![],
                Some(items) => items.clone(),
            }
        }

        fn on_send(&mut self, _tx: Option<&TestView>) {
            self.item_index += 1;
        }
    }

    fn create_subscription_helper() -> (
        MockDiemDB,
        ClientConnection,
        Receiver<Result<String, StreamError>>,
        SubscriptionHelper,
    ) {
        let (mock_db, client_connection, receiver) = create_client_connection();

        // The 'method' does not matter for this test (it's used for logging)
        let subscription_helper = SubscriptionHelper::new(
            Arc::new(mock_db.clone()),
            client_connection.clone(),
            Id::Number(1010101),
            StreamMethod::SubscribeToTransactions,
        );

        (mock_db, client_connection, receiver, subscription_helper)
    }

    #[tokio::test]
    async fn test_subscription() {
        let (mock_db, _, mut receiver, subscription_helper) = create_subscription_helper();

        let test_items = vec![
            // Send two items
            vec![TestView { value: 0 }, TestView { value: 1 }],
            // Then don't send anything, and increase the exponential delay
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            // Send some values: the exponential delay should reset
            vec![TestView { value: 2 }],
            vec![TestView { value: 3 }],
        ];

        let params = SubscribeTestParams {
            is_valid: true,
            items_to_send: test_items,
        };

        let handle = TestSubscription::default()
            .run(subscription_helper, params)
            .unwrap();
        let ok_msg = receiver.recv().await;

        let result = ok_msg.unwrap().unwrap();
        let expected = serde_json::json!({"jsonrpc": "2.0", "id": 1010101, "result": {"status": "OK", "transaction_version": mock_db.version}}).to_string();
        assert_eq!(expected, result);

        // These should have no delay
        let expected =
            serde_json::json!({"jsonrpc": "2.0", "id": 1010101, "result":{"value": 0}}).to_string();
        let next = timeout(1_000, receiver.recv(), "message 0")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expected, next);

        let expected =
            serde_json::json!({"jsonrpc": "2.0", "id": 1010101, "result":{"value": 1}}).to_string();
        let next = timeout(1_000, receiver.recv(), "message 1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expected, next);

        // Here we have many iterations which return nothing, so the exponential delay should kick in
        let start = Instant::now();
        let expected =
            serde_json::json!({"jsonrpc": "2.0", "id": 1010101, "result":{"value": 2}}).to_string();
        let next = timeout(3_000, receiver.recv(), "message 2")
            .await
            .unwrap()
            .unwrap();
        let elapsed = start.elapsed().as_millis();
        assert_eq!(expected, next);
        assert!(elapsed > 500);

        // This one should have no delay again, due to being reset by the previous message
        let expected =
            serde_json::json!({"jsonrpc": "2.0", "id": 1010101, "result":{"value": 3}}).to_string();
        let next = timeout(1_000, receiver.recv(), "message 3")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expected, next);
        handle.abort();
    }

    #[tokio::test]
    async fn test_init_rejects_subscription() {
        let (_, _, mut receiver, subscription_helper) = create_subscription_helper();

        let params = SubscribeTestParams {
            is_valid: false,
            items_to_send: vec![],
        };

        let handle = TestSubscription::default().run(subscription_helper, params);

        assert!(handle.is_err());
        let expected = serde_json::json!({"code":-32602, "message": "Invalid param not is_valid", "data": null}).to_string();
        assert_eq!(
            expected,
            serde_json::to_string(&handle.err().unwrap()).unwrap()
        );

        // This is expected to be `None` as the client_connection is dropped
        let ok_msg = receiver.recv().await;
        assert!(ok_msg.is_none());
    }

    #[tokio::test]
    async fn test_sleep_jitter_await() {
        let (_, _, _, mut subscription_helper) = create_subscription_helper();

        let mut results = vec![];

        let start = Instant::now();
        let mut last: u64 = 0;

        for _ in 0..10 {
            subscription_helper.sleep_wiggled().await;
            let elapsed = start.elapsed().as_millis() as u64;
            results.push(elapsed - last);
            last = elapsed;
        }

        // In the event the test fails, this println makes it much easier to debug
        println!("jittered sleeps: {:?}", results);

        // 80 Jitter makes exact comparison hard, but exponentially increasing values here should be > 1.8x the previous one
        for i in 5..9 {
            let prev = *results.get(i).unwrap() as f64;
            let curr = *results.get(i + 1).unwrap() as f64 * 1.8;
            assert!(prev < curr);
        }

        subscription_helper.reset_backoff();
        timeout(
            50,
            subscription_helper.sleep_wiggled(),
            "sleep_wiggled after reset_backoff",
        )
        .await;
    }
}

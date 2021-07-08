// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::errors::JsonRpcError;
use std::sync::Arc;
use storage_interface::DbReader;
use tokio::task::JoinHandle;

use crate::stream_rpc::{
    connection::ClientConnection,
    subscription_types::{Subscription, SubscriptionHelper},
    subscriptions::{EventsSubscription, TransactionsSubscription},
};
use diem_json_rpc_types::{stream::request::StreamMethodRequest, Id};

pub struct CallableStreamMethod(pub StreamMethodRequest);

impl CallableStreamMethod {
    pub fn call_method(
        self,
        db: Arc<dyn DbReader>,
        client: ClientConnection,
        jsonrpc_id: Id,
    ) -> Result<JoinHandle<()>, JsonRpcError> {
        let method = self.0.method();
        let helper = SubscriptionHelper::new(db, client, jsonrpc_id, method);
        match self.0 {
            StreamMethodRequest::SubscribeToTransactions(params) => {
                TransactionsSubscription::default().run(helper, params)
            }
            StreamMethodRequest::SubscribeToEvents(params) => {
                EventsSubscription::default().run(helper, params)
            }
            // This is handled in the `handle_rpc_request` function, as we don't spawn a task
            StreamMethodRequest::Unsubscribe => unreachable!(),
        }
    }
}

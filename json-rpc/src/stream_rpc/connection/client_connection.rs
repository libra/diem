// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    errors::JsonRpcError,
    stream_rpc::{
        connection::{ConnectionContext, StreamSender},
        counters,
        errors::StreamError,
        json_rpc::CallableStreamMethod,
        logging,
        subscription_types::SubscriptionConfig,
    },
};
use diem_infallible::Mutex;
use diem_json_rpc_types::{
    stream::{
        request::{StreamJsonRpcRequest, StreamMethod, StreamMethodRequest},
        response::{StreamJsonRpcResponse, UnsubscribeResult},
    },
    Id,
};
use diem_logger::debug;
use std::{collections::HashMap, str::FromStr, sync::Arc};
use storage_interface::DbReader;

const UNKNOWN: &str = "unknown";

#[derive(Debug)]
pub struct Task(tokio::task::JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The `ClientConnection` is the interface between a transport, and subscriptions
/// This will get cloned for every subscription, so lets keep it light :-)
#[derive(Debug, Clone)]
pub struct ClientConnection {
    pub id: u64,
    pub sender: StreamSender,
    pub tasks: Arc<Mutex<HashMap<Id, Task>>>,
    pub connection_context: Arc<ConnectionContext>,
    pub config: Arc<SubscriptionConfig>,
}

impl ClientConnection {
    pub fn new(
        id: u64,
        sender: StreamSender,
        connection_context: ConnectionContext,
        config: Arc<SubscriptionConfig>,
    ) -> Self {
        Self {
            id,
            sender,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            connection_context: Arc::new(connection_context),
            config,
        }
    }

    pub async fn send_raw(&self, message: String) -> Result<(), StreamError> {
        if self.sender.is_closed() {
            return Err(StreamError::ClientAlreadyClosed(self.id));
        }
        match self.sender.send(Ok(message)).await {
            Ok(_) => Ok(()),
            Err(e) => match e.0 {
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            },
        }
    }

    pub async fn send_response(&self, message: StreamJsonRpcResponse) -> Result<(), StreamError> {
        if let Ok(response) = serde_json::to_string(&message) {
            return self.send_raw(response).await;
        }
        Err(StreamError::CouldNotStringifyMessage(format!(
            "{:?}",
            message
        )))
    }

    pub async fn send_success<T: serde::Serialize>(
        &self,
        id: Id,
        message: &T,
    ) -> Result<(), StreamError> {
        let message = serde_json::to_value(message).unwrap();
        self.send_response(StreamJsonRpcResponse::result(Some(id), Some(message)))
            .await
    }

    pub async fn send_error(
        &self,
        method: Option<StreamMethod>,
        id: Option<Id>,
        error: JsonRpcError,
    ) -> Result<(), StreamError> {
        counters::INVALID_REQUESTS
            .with_label_values(&[
                self.connection_context.transport.as_str(),
                method.map_or(UNKNOWN, |m| m.as_str()),
                error.code_as_str(),
                self.connection_context.sdk_info.language.as_str(),
                &self.connection_context.sdk_info.version.to_string(),
            ])
            .inc();
        self.send_response(StreamJsonRpcResponse::error(id, error))
            .await
    }

    /// Unsubscribe a subscription for a client
    pub fn unsubscribe(&self, id: &Id) -> Option<()> {
        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.get(id) {
            debug!(
                "Unsubscribing: terminating task '{}' for Client#{}",
                &id, &self.id
            );

            // Send off a response to the client
            let sender = self.sender.clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                let response = StreamJsonRpcResponse::result(
                    Some(id2),
                    Some(serde_json::to_value(UnsubscribeResult::ok()).unwrap()),
                );
                // If we can't send a message, connection is closed and we're going down
                let _ = sender
                    .send(Ok(serde_json::to_string(&response).unwrap()))
                    .await;
            });

            task.0.abort();
            tasks.remove(&id);
            Some(())
        } else {
            debug!(
                "Unsubscribing: No such task '{}' for Client#{}",
                &id, &self.id
            );
            None
        }
    }

    pub async fn received_message(&self, db: Arc<dyn DbReader>, message: String) {
        match StreamJsonRpcRequest::from_str(&message) {
            Ok(mut request) => {
                debug!(
                    logging::StreamRpcLog {
                        transport: self.connection_context.transport.as_str(),
                        remote_addr: self.connection_context.remote_addr.as_deref(),
                        user_agent: None,
                        action: logging::StreamRpcAction::ClientConnectionLog(
                            logging::ClientConnectionLog {
                                client_id: Some(self.id),
                                forwarded: None,
                                rpc_method: Some(request.method_name()),
                            }
                        ),
                    },
                    "subscription request"
                );
                if let Err(err) = self.handle_rpc_request(db, &mut request) {
                    self.send_error(Some(request.method_request.method()), Some(request.id), err)
                        .await
                        .ok();
                }
            }
            Err((err, method, id)) => {
                // We couldn't parse the request- it's not valid json or an unknown structure
                debug!(
                    logging::StreamRpcLog {
                        transport: self.connection_context.transport.as_str(),
                        remote_addr: self.connection_context.remote_addr.as_deref(),
                        user_agent: None,
                        action: logging::StreamRpcAction::ClientConnectionLog(
                            logging::ClientConnectionLog {
                                client_id: Some(self.id),
                                forwarded: None,
                                rpc_method: method.map(|v| v.as_str()),
                            }
                        ),
                    },
                    "failed to parse subscription request ({})", &err
                );
                metric_subscription_rpc_received(
                    &self,
                    method.map_or(UNKNOWN, |m| m.as_str()),
                    counters::RpcResult::Error,
                );
                self.send_error(method, id, err).await.ok();
            }
        }
    }

    /// - The `tasks` lock is within the scope of one client (subscribing, unsubscribing, disconnecting, or some combination therein)
    /// - The `call_method` is responsible for doing validation on the parameters, and returning a `Result<JoinHandle<()>>` (tokio task) if
    ///     a subscription task was spawned
    /// - The lock must be held until we can determine whether or not we have a subscription because otherwise there is a race condition:
    ///     if a user submits the same rpc id (`RequestIdentifier`) again after we’ve verified it’s not used, but before we insert it,
    ///     which would result in losing track of that subscription task (and leaking green threads)
    ///
    /// When calling `request.method_request.call_method`, communication to the client and
    /// subscription behavior is determined by the `Result<JoinHandle<()>, JsonRpcError>` returned.
    ///
    /// 1. Returning `Err(JsonRpcRequest)` is the way to handle an issue with a parameter value, or
    ///     any other such case where a subscription may not be started or requested data may not be
    ///     returned.
    /// 2. Returning `Ok(JoinHandle<()>)` indicates that the subscription has been successfully created.
    fn handle_rpc_request(
        &self,
        db: Arc<dyn DbReader>,
        request: &mut StreamJsonRpcRequest,
    ) -> Result<(), JsonRpcError> {
        // No task needs to spawn for an unsubscribe
        if matches!(request.method_request, StreamMethodRequest::Unsubscribe) {
            self.unsubscribe(&request.id);
            return Ok(());
        }

        let mut tasks = self.tasks.lock();
        if tasks.contains_key(&request.id) {
            debug!(
                "Client#{} already has a subscription for '{}'",
                self.id, &request.id
            );
            let err = JsonRpcError::invalid_request_with_msg(format!(
                "Subscription for '{}' already exists",
                &request.id
            ));

            return Err(err);
        }

        match CallableStreamMethod(request.method_request).call_method(
            db.clone(),
            self.clone(),
            request.id.clone(),
        ) {
            Ok(task) => {
                tasks.insert(request.id.clone(), Task(task));
                metric_subscription_rpc_received(
                    &self,
                    request.method_name(),
                    counters::RpcResult::Success,
                );
                Ok(())
            }
            Err(err) => {
                // This error comes from within the subscription itself, before the task is started: it's most likely parameter validation.
                metric_subscription_rpc_received(
                    &self,
                    request.method_name(),
                    counters::RpcResult::Error,
                );
                Err(err)
            }
        }
    }
}

fn metric_subscription_rpc_received(
    client: &ClientConnection,
    method: &str,
    result: counters::RpcResult,
) {
    counters::SUBSCRIPTION_RPC_RECEIVED
        .with_label_values(&[
            client.connection_context.transport.as_str(),
            method,
            result.as_str(),
            client.connection_context.sdk_info.language.as_str(),
            &client.connection_context.sdk_info.version.to_string(),
        ])
        .inc();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_rpc::tests::util::{create_client_connection, timeout};

    #[tokio::test]
    async fn test_send_raw() {
        let (_, client_connection, mut receiver) = create_client_connection();
        let expected = "success".to_string();
        client_connection.send_raw(expected.clone()).await.unwrap();
        let result = timeout(50, receiver.recv(), "get message")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, expected)
    }

    #[tokio::test]
    async fn test_client_connection_success() {
        let (mock_db, client_connection, mut receiver) = create_client_connection();

        let request = serde_json::json!({
          "id": "client-generated-id",
          "method": "subscribe_to_transactions",
          "params": {
            "starting_version": 0
          },
          "jsonrpc": "2.0"
        })
        .to_string();
        client_connection
            .received_message(Arc::new(mock_db.clone()), request)
            .await;

        let result = timeout(50, receiver.recv(), "message 1")
            .await
            .unwrap()
            .unwrap();
        let expected = serde_json::json!({
          "jsonrpc": "2.0",
          "id": "client-generated-id",
          "result": {
            "status": "OK",
            "transaction_version": mock_db.version
          }
        })
        .to_string();
        assert_eq!(result, expected);

        let result = timeout(50, receiver.recv(), "message 1")
            .await
            .unwrap()
            .unwrap();

        let result = serde_json::Value::from_str(&result)
            .expect("could not parse json")
            .get("result")
            .expect("no result")
            .get("version")
            .expect("no version")
            .to_string();

        assert_eq!(result, "0");
    }

    #[tokio::test]
    async fn test_client_connection_unsubscribe() {
        let (mock_db, client_connection, mut receiver) = create_client_connection();

        let request = serde_json::json!({
          "id": "client-generated-id",
          "method": "subscribe_to_transactions",
          "params": {
            "starting_version": 0
          },
          "jsonrpc": "2.0"
        })
        .to_string();
        client_connection
            .received_message(Arc::new(mock_db.clone()), request)
            .await;

        let result = timeout(50, receiver.recv(), "message 1")
            .await
            .unwrap()
            .unwrap();
        let expected = serde_json::json!({
          "jsonrpc": "2.0",
          "id": "client-generated-id",
          "result": {
            "status": "OK",
            "transaction_version": mock_db.version
          }
        })
        .to_string();
        assert_eq!(result, expected);

        let request = serde_json::json!({
          "id": "client-generated-id",
          "method": "unsubscribe",
          "params": {},
          "jsonrpc": "2.0"
        })
        .to_string();

        client_connection
            .received_message(Arc::new(mock_db.clone()), request)
            .await;

        let result = timeout(50, receiver.recv(), "message 1")
            .await
            .unwrap()
            .unwrap();

        let result = serde_json::Value::from_str(&result);
        println!("{:?}", &result);
        let result = result
            .expect("could not parse json")
            .get("result")
            .expect("no result")
            .get("unsubscribe")
            .expect("no unsubscribe")
            .to_string();

        assert_eq!(result, "\"OK\"");
    }

    #[tokio::test]
    async fn test_client_connection_error() {
        let (mock_db, client_connection, mut receiver) = create_client_connection();

        let request = "{\"bad_json".to_string();
        client_connection
            .received_message(Arc::new(mock_db.clone()), request)
            .await;

        let result = timeout(50, receiver.recv(), "message 1")
            .await
            .unwrap()
            .unwrap();
        let expected = serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32604, "message": "Invalid request format", "data": null}}).to_string();
        assert_eq!(result, expected);
    }
}

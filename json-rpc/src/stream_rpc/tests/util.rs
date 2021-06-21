// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use std::{future::Future, sync::Arc};

// use proptest::prelude::*;
use warp::{test::WsClient, ws::Message};

use diem_config::config::StreamConfig;

use crate::{
    stream_rpc::{
        connection::{ClientConnection, ConnectionContext, ConnectionManager},
        errors::StreamError,
        subscription_types::SubscriptionConfig,
        transport::{util::Transport, websocket::get_websocket_routes},
    },
    tests::utils::{mock_db, MockDiemDB},
};

use diem_json_rpc_types::stream::response::StreamJsonRpcResponse;
use tokio::sync::{mpsc, mpsc::Receiver};

pub fn make_ws_config(
    fetch_size: u64,
    queue_size: usize,
    poll_interval_ms: u64,
    max_poll_interval_ms: u64,
) -> StreamConfig {
    StreamConfig {
        enabled: true,
        subscription_fetch_size: fetch_size,
        send_queue_size: queue_size,
        poll_interval_ms,
        max_poll_interval_ms,
    }
}

pub async fn connect_to_ws(
    db: Arc<MockDiemDB>,
    config: &StreamConfig,
    cm: Option<ConnectionManager>,
) -> (WsClient, ConnectionManager) {
    let (routes, cm) = get_websocket_routes(config, 1024 * 10, db.clone(), cm);
    let ws_client = warp::test::ws()
        .path("/v1/stream/ws")
        .header("user-agent", "diem-client-sdk-python / 0.1.22")
        .header("Content-Length", "1500")
        .handshake(routes)
        .await
        .expect("handshake");

    (ws_client, cm)
}

pub async fn ws_test_setup(
    fetch_size: u64,
    queue_size: usize,
    poll_interval_ms: u64,
    max_poll_interval_ms: u64,
) -> (Arc<MockDiemDB>, StreamConfig) {
    diem_logger::DiemLogger::init_for_testing();
    let db = Arc::new(mock_db());
    let config = make_ws_config(
        fetch_size,
        queue_size,
        poll_interval_ms,
        max_poll_interval_ms,
    );
    (db, config)
}

/// Verifies the subscription "OK" response is correct
pub fn verify_ok(msg: Message, name: &str) -> u64 {
    let msg_str = msg
        .to_str()
        .unwrap_or_else(|_| panic!("{}: ok msg response", &name));

    let resp: StreamJsonRpcResponse = serde_json::from_str(msg_str)
        .unwrap_or_else(|_| panic!("{}: could not parse response. {:?}", &name, msg));

    let res = resp.result;
    assert!(res.is_some(), "{}: result is None. {:?}", &name, msg);

    let res = res.unwrap();

    let status = res.get("status");
    assert!(status.is_some(), "{}: status is None. {:?}", &name, msg);

    assert_eq!(
        status.unwrap().as_str().unwrap(),
        "OK",
        "{}: status is not 'OK'. {:?}",
        &name,
        msg
    );

    res.get("transaction_version")
        .expect("OK transaction version")
        .as_u64()
        .expect("ok transaction version to u64")
}

pub fn num_clients(cm: &ConnectionManager) -> usize {
    cm.clients.read().len()
}

pub fn get_latest_client(cm: &ConnectionManager) -> ClientConnection {
    cm.get_client((num_clients(cm) - 1) as u64)
        .expect("no clients exist")
}

pub fn num_tasks(client: &ClientConnection) -> usize {
    client.tasks.lock().len()
}

/// This is primarily to ensure that the websocket transport responds to `close` correctly
/// And passes the appropriate signal along to the `ConnectionManager`
/// Dropping the ws_client is enough to actually close the connection
pub async fn close_ws(mut ws_client: WsClient, name: &str) {
    ws_client.send(warp::ws::Message::close()).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    ws_client
        .recv_closed()
        .await
        .unwrap_or_else(|err| panic!("{}: connection not closed. {:?}", name, err));

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
}

pub async fn next_message(ws_client: &mut WsClient, name: &str) -> warp::filters::ws::Message {
    timeout(100, ws_client.recv(), name)
        .await
        .unwrap_or_else(|e| panic!("{}: message not ok. {:?}", name, e))
}

pub async fn timeout<F: Future<Output = T>, T>(duration_millis: u64, future: F, name: &str) -> T {
    tokio::time::timeout(std::time::Duration::from_millis(duration_millis), future)
        .await
        .unwrap_or_else(|_| panic!("{}: Timed out", name))
}

pub fn create_client_connection() -> (
    MockDiemDB,
    ClientConnection,
    Receiver<Result<String, StreamError>>,
) {
    let mock_db = mock_db();

    let (sender, receiver) = mpsc::channel::<Result<String, StreamError>>(1);

    let config = SubscriptionConfig {
        fetch_size: 1,
        poll_interval_ms: 2,
        max_poll_interval_ms: 1000,
        queue_size: 1,
    };

    let connection_context = ConnectionContext {
        transport: Transport::Websocket,
        sdk_info: Default::default(),
        remote_addr: None,
    };
    let client_connection =
        ClientConnection::new(1337, sender, connection_context, Arc::new(config));

    (mock_db, client_connection, receiver)
}

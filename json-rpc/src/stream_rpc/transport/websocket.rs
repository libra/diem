// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use warp::{filters::BoxedFilter, ws::Message, Filter, Rejection, Reply};

use diem_config::config::StreamConfig;
use diem_logger::debug;
use storage_interface::DbReader;

use crate::stream_rpc::{
    connection::{ConnectionContext, ConnectionManager},
    counters,
    errors::StreamError,
    logging,
    subscription_types::SubscriptionConfig,
    transport::util::{get_remote_addr, Transport},
};

pub fn get_websocket_routes(
    config: &StreamConfig,
    content_length_limit: u64,
    diem_db: Arc<dyn DbReader>,
    connection_manager: Option<ConnectionManager>,
) -> (BoxedFilter<(impl Reply,)>, ConnectionManager) {
    let sub_config = Arc::new(SubscriptionConfig {
        fetch_size: config.subscription_fetch_size,
        poll_interval_ms: config.poll_interval_ms,
        max_poll_interval_ms: config.max_poll_interval_ms,
        queue_size: config.send_queue_size,
    });

    let connection_manager = match connection_manager {
        None => ConnectionManager::new(diem_db.clone(), sub_config),
        Some(cm) => cm,
    };

    let cm2 = connection_manager.clone();
    let ws_route = warp::path("v1")
        .and(warp::path("stream"))
        .and(warp::path("ws"))
        .and(warp::path::end())
        .and(warp::body::content_length_limit(content_length_limit))
        .and(warp::filters::header::header::<String>("user-agent"))
        .and(warp::ws())
        .and(warp::any().map(move || cm2.clone()))
        .and(warp::any().map(move || content_length_limit as usize))
        .and(warp::header::headers_cloned())
        .and(warp::filters::addr::remote())
        .and_then(handle_websocket_stream)
        .with(warp::cors().allow_any_origin())
        .with(warp::log::custom(|info| {
            debug!(
                logging::StreamRpcLog {
                    transport: Transport::Websocket.as_str(),
                    remote_addr: Some(&format!("{:?}", info.remote_addr())),
                    user_agent: Some(info.user_agent().unwrap_or("")),
                    action: logging::StreamRpcAction::HttpRequestLog(logging::HttpRequestLog {
                        path: &info.path().to_string(),
                        status: info.status().as_u16(),
                        referer: info.referer(),
                        forwarded: info
                            .request_headers()
                            .get(warp::http::header::FORWARDED)
                            .and_then(|v| v.to_str().ok()),
                    }),
                },
                "http request"
            )
        }))
        .boxed();

    (ws_route, connection_manager)
}

/// The `ContextWrapperMapper` exists so we can pass websocket specific connection context along
/// It's responsible for mapping between the message types expected by the warp websocket implementation,
/// and the internal `ClientConnectionManager` responsible for handling subscriptions
#[derive(Clone)]
pub struct ContextWrapperMapper {
    pub content_length_limit: usize,
    pub context: ConnectionContext,
}

impl ContextWrapperMapper {
    // Incoming messages
    pub fn message_result_to_string(
        &self,
        result: Result<Message, warp::Error>,
    ) -> Result<Option<String>, StreamError> {
        counters::MESSAGES_RECEIVED
            .with_label_values(&[
                self.context.transport.as_str(),
                self.context.sdk_info.language.as_str(),
                &self.context.sdk_info.version.to_string(),
            ])
            .inc();

        match result {
            Ok(msg) => {
                // Silently drop messages that are too big
                if msg.as_bytes().len() > self.content_length_limit {
                    debug!(
                        "Received websocket message that was too big from {:?}",
                        self.context.remote_addr
                    );
                    return Ok(None);
                }

                // tungstenite already auto-responds to `Ping`s with a `Pong`,
                // so this just prevents accidentally sending extra pings.
                // We ignore pong messages here
                if msg.is_ping() || msg.is_pong() {
                    return Ok(None);
                }

                // Returning an error allows the `ConnectionManager` to process the disconnect
                if msg.is_close() {
                    return Err(StreamError::ClientWantsToDisconnect);
                }

                match msg.to_str() {
                    Ok(s) => Ok(Some(s.to_string())),
                    // Silently drop messages that aren't text
                    Err(_) => {
                        debug!(
                            "Received unhandled '{}' websocket message from {:?}",
                            message_type_to_string(&msg),
                            self.context.remote_addr
                        );
                        Ok(None)
                    }
                }
            }
            Err(e) => Err(StreamError::TransportError(e.to_string())),
        }
    }

    // Outgoing messages
    fn string_result_to_message(&self, result: Result<String, StreamError>) -> Message {
        counters::MESSAGES_SENT
            .with_label_values(&[
                self.context.transport.as_str(),
                self.context.sdk_info.language.as_str(),
                &self.context.sdk_info.version.to_string(),
            ])
            .inc();
        match result {
            Ok(item) => Message::text(item),
            Err(_e) => Message::close(),
        }
    }
}

pub async fn handle_websocket_stream(
    user_agent: String,
    socket: warp::ws::Ws,
    cm: ConnectionManager,
    content_length_limit: usize,
    headers: warp::http::HeaderMap,
    remote_socket: Option<std::net::SocketAddr>,
) -> Result<impl Reply, Rejection> {
    let sdk_info = crate::util::sdk_info_from_user_agent(Some(&user_agent));
    counters::HTTP_WEBSOCKET_REQUESTS
        .with_label_values(&[sdk_info.language.as_str(), &sdk_info.version.to_string()])
        .inc();

    Ok(socket
        .max_send_queue(cm.config.queue_size)
        .max_message_size(content_length_limit)
        .on_upgrade(move |socket| {
            async move {
                counters::HTTP_WEBSOCKET_REQUEST_UPGRADES
                    .with_label_values(&[sdk_info.language.as_str(), &sdk_info.version.to_string()])
                    .inc();
                let context = ConnectionContext {
                    transport: Transport::Websocket,
                    sdk_info,
                    remote_addr: get_remote_addr(&headers, remote_socket.as_ref()),
                };

                let (to_client_ws, from_client_ws) = socket.split();
                let (to_client, to_client_rcv) =
                    mpsc::channel::<Result<String, StreamError>>(cm.config.queue_size);

                let cwm = Arc::new(ContextWrapperMapper {
                    content_length_limit,
                    context: context.clone(),
                });

                // Incoming
                let cwm_i = cwm.clone();
                let mapped_from_client_ws =
                    from_client_ws.map(move |result| cwm_i.message_result_to_string(result));

                // Outgoing
                tokio::task::spawn(
                    ReceiverStream::new(to_client_rcv)
                        .map(move |result| Ok(cwm.string_result_to_message(result)))
                        .forward(to_client_ws)
                        .map(move |result: Result<(), warp::Error>| {
                            debug!(
                                logging::StreamRpcLog {
                                    transport: Transport::Websocket.as_str(),
                                    remote_addr: remote_socket
                                        .map(|remote_socket| remote_socket.to_string())
                                        .as_deref(),
                                    user_agent: Some(&user_agent),
                                    action: logging::StreamRpcAction::ClientConnectionLog(
                                        logging::ClientConnectionLog {
                                            client_id: None,
                                            forwarded: headers
                                                .get(warp::http::header::FORWARDED)
                                                .and_then(|v| v.to_str().ok()),
                                            rpc_method: None,
                                        }
                                    ),
                                },
                                "websocket disconnected ({:?})", result
                            )
                        }),
                );

                // This will keep running until the connection is closed
                cm.client_connection(to_client, Box::new(mapped_from_client_ws), context)
                    .await;
            }
        }))
}

pub fn message_type_to_string(msg: &Message) -> &str {
    if msg.is_ping() {
        return "ping";
    }
    if msg.is_pong() {
        return "pong";
    }
    if msg.is_close() {
        return "close";
    }
    if msg.is_binary() {
        return "binary";
    }
    if msg.is_text() {
        return "text";
    }
    // This should never happen
    "unknown"
}

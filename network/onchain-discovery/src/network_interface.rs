// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Protobuf based interface between OnchainDiscovery and Network layers.
use crate::types::{OnchainDiscoveryMsg,QueryDiscoverySetRequest, QueryDiscoverySetResponse};
use channel::{ message_queues::QueueStyle, libra_channel};
use futures::{channel::mpsc, sink::SinkExt};
use network::{
    peer_manager::{
        ConnectionRequestSender,
        PeerManagerRequestSender,
        PeerManagerNotification,
        ConnectionNotification
    },
    connectivity_manager::ConnectivityRequest,
    protocols::{
        network::{ NetworkSender},
        protocols::{network::NetworkSender, rpc::error::RpcError},
    },
    ProtocolId,
};
use std::time::Duration;

/// The interface from Network to OnchainDiscovery layer.
///
/// `OnchainDiscoveryNetworkEvents` is a `Stream` of `PeerManagerNotification` where the
/// raw `Bytes` rpc messages are deserialized into
/// `OnchainDiscoveryMsg` types. `OnchainDiscoveryNetworkEvents` is a thin wrapper
/// around an `channel::Receiver<PeerManagerNotification>`.
pub struct OnchainDiscoveryNetworkEvents {
    pub peer_mgr_notifs_rx: libra_channel::Receiver<(PeerId, ProtocolId), PeerManagerNotification>,
    pub connection_notifs_rx: libra_channel::Receiver<PeerId, ConnectionNotification>
}

impl OnchainDiscoveryNetworkEvents {
    pub fn new (peer_mgr_notifs_rx: libra_channel::Receiver<(PeerId, ProtocolId), PeerManagerNotification>,
                connection_notifs_rx: libra_channel::Receiver<PeerId, ConnectionNotification>) -> Self {
        Self {
            peer_mgr_notifs_rx,
            connection_notifs_rx
        }
    }
}

/// The interface from OnchainDiscovery to Networking layer.
#[derive(Clone)]
pub struct OnchainDiscoveryNetworkSender {
    inner: NetworkSender<OnchainDiscoveryMsg>,
}

impl OnchainDiscoveryNetworkSender {
    pub fn new(
        peer_mgr_reqs_tx: PeerManagerRequestSender,
        conn_reqs_tx: ConnectionRequestSender,
    ) -> Self {
        Self {
            inner: NetworkSender::new(peer_mgr_reqs_tx, conn_reqs_tx),
        }
    }

    pub async fn query_discovery_set(
        &mut self,
        recipient: PeerId,
        req_msg: QueryDiscoverySetRequest,
        timeout: Duration,
    ) -> Result<Box<QueryDiscoverySetResponse>, RpcError> {
        let protocol = ProtocolId::OnchainDiscoveryRpc;
        let req_msg_enum = OnchainDiscoveryMsg::QueryDiscoverySetRequest(req_msg);

        let res_msg_enum = self
            .inner
            .send_rpc(recipient, protocol, req_msg_enum, timeout)
            .await?;

        let res_msg = match res_msg_enum {
            OnchainDiscoveryMsg::QueryDiscoverySetResponse(res_msg) => res_msg,
            OnchainDiscoveryMsg::QueryDiscoverySetRequest(_) => {
                return Err(RpcError::InvalidRpcResponse);
            }
        };
        Ok(res_msg)
    }
}

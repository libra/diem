// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Remotely authenticated vs. unauthenticated network end-points:
//! ---------------------------------------------------
//! A network end-point operates with remote authentication if it only accepts connections
//! from a known set of peers (`trusted_peers`) identified by their network identity keys.
//! This does not mean that the other end-point of a connection also needs to operate with
//! authentication -- a network end-point running with remote authentication enabled will
//! connect to or accept connections from an end-point running in authenticated mode as
//! long as the latter is in its trusted peers set.
use channel::{self, message_queues::QueueStyle};
use libra_config::{
    chain_id::ChainId,
    config::{DiscoveryMethod, NetworkConfig, RoleType, HANDSHAKE_VERSION},
    network_id::{NetworkContext, NetworkId},
};
use libra_crypto::x25519;
use libra_logger::prelude::*;
use libra_metrics::IntCounterVec;
use libra_network_address::NetworkAddress;
use libra_types::{waypoint::Waypoint, PeerId};
use network::{
    connectivity_manager::{builder::ConnectivityManagerBuilder, ConnectivityRequest},
    constants,
    peer_manager::{builder::PeerManagerBuilder, conn_notifs_channel, ConnectionRequestSender},
    protocols::{
        discovery::{self, builder::DiscoveryBuilder},
        health_checker::{self, builder::HealthCheckerBuilder},
        network::{NewNetworkEvents, NewNetworkSender},
    },
    ProtocolId,
};
use network_simple_onchain_discovery::ConfigurationChangeListenerBuilder;
use onchain_discovery::builder::OnchainDiscoveryBuilder;
use std::{
    clone::Clone,
    collections::HashMap,
    sync::{Arc, RwLock},
};
use storage_interface::DbReader;
use subscription_service::ReconfigSubscription;
use tokio::runtime::{Builder, Handle, Runtime};

pub use network::peer_manager::builder::AuthenticationMode;

/// Build Network module with custom configuration values.
/// Methods can be chained in order to set the configuration values.
/// MempoolNetworkHandler and ConsensusNetworkHandler are constructed by calling
/// [`NetworkBuilder::build`].  New instances of `NetworkBuilder` are obtained
/// via [`NetworkBuilder::new`].
// TODO(philiphayes): refactor NetworkBuilder and libra-node; current config is
// pretty tangled.
pub struct NetworkBuilder {
    network_context: Arc<NetworkContext>,
    trusted_peers: Arc<RwLock<HashMap<PeerId, x25519::PublicKey>>>,

    connectivity_manager_builder: Option<ConnectivityManagerBuilder>,
    configuration_change_listener_builder: Option<ConfigurationChangeListenerBuilder>,
    discovery_builder: Option<DiscoveryBuilder>,
    health_checker_builder: Option<HealthCheckerBuilder>,
    onchain_discovery_builder: Option<OnchainDiscoveryBuilder>,
    peer_manager_builder: Option<PeerManagerBuilder>,

    reconfig_subscriptions: Vec<ReconfigSubscription>,
}

impl NetworkBuilder {
    /// Return a new NetworkBuilder initialized with default configuration values.
    pub fn new(
        chain_id: ChainId,
        network_id: NetworkId,
        role: RoleType,
        peer_id: PeerId,
        listen_address: NetworkAddress,
        authentication_mode: AuthenticationMode,
    ) -> NetworkBuilder {
        let network_context = Arc::new(NetworkContext::new(network_id, role, peer_id));
        let trusted_peers = Arc::new(RwLock::new(HashMap::new()));

        let peer_manager_builder = PeerManagerBuilder::create(
            chain_id,
            network_context.clone(),
            listen_address,
            trusted_peers.clone(),
            authentication_mode,
            constants::NETWORK_CHANNEL_SIZE,
            constants::MAX_CONCURRENT_NETWORK_REQS,
            constants::MAX_CONCURRENT_NETWORK_NOTIFS,
        );
        NetworkBuilder {
            network_context,
            trusted_peers,
            peer_manager_builder: Some(peer_manager_builder),
            connectivity_manager_builder: None,
            configuration_change_listener_builder: None,
            discovery_builder: None,
            health_checker_builder: None,
            onchain_discovery_builder: None,
            reconfig_subscriptions: Vec::new(),
        }
    }

    pub fn create(
        chain_id: &ChainId,
        role: RoleType,
        config: &mut NetworkConfig,
        libra_db: Arc<dyn DbReader>,
        waypoint: Waypoint,
    ) -> (Runtime, NetworkBuilder) {
        let runtime = Builder::new()
            .thread_name("network-")
            .threaded_scheduler()
            .enable_all()
            .build()
            .expect("Failed to start runtime. Won't be able to start networking.");

        let peer_id = config.peer_id();
        let identity_key = config.identity_key();
        let public_key = identity_key.public_key();

        let authentication_mode = if config.mutual_authentication {
            AuthenticationMode::Mutual(identity_key)
        } else {
            AuthenticationMode::ServerOnly(identity_key)
        };

        let mut network_builder = NetworkBuilder::new(
            chain_id.clone(),
            config.network_id.clone(),
            role,
            peer_id,
            config.listen_address.clone(),
            authentication_mode,
        );
        network_builder.add_connection_monitoring(
            // TODO:  move into NetworkConfig
            constants::PING_INTERVAL_MS,
            // TODO:  move into NetworkConfig
            constants::PING_TIMEOUT_MS,
            // TODO:  move into NetworkConfig
            constants::PING_FAILURES_TOLERATED,
        );

        // Sanity check seed peer addresses.
        config
            .verify_seed_peer_addrs()
            .expect("Seed peer addresses must be well-formed");
        let seed_peers = config.seed_peers.clone();

        if config.mutual_authentication {
            let network_peers = config.network_peers.clone();
            let trusted_peers = if role == RoleType::Validator {
                // for validators, trusted_peers is empty will be populated from consensus
                HashMap::new()
            } else {
                network_peers
            };

            info!(
                "network setup: role: {}, seed_peers: {:?}, trusted_peers: {:?}",
                role, seed_peers, trusted_peers,
            );

            network_builder
                .trusted_peers(trusted_peers)
                // TODO place channel_size in network_config
                .add_connectivity_manager(
                    seed_peers,
                    config.connectivity_check_interval_ms,
                    constants::MAX_FULLNODE_CONNECTIONS,
                    constants::NETWORK_CHANNEL_SIZE,
                );
        } else {
            // TODO:  Why does ServerOnly and no seed_peers mean that a ConnectivityManager is unnecessary?
            // My understanding is that ConnectivityManager is responsible for keeping connections to neighbors alive.
            if !seed_peers.is_empty() {
                network_builder.add_connectivity_manager(
                    seed_peers,
                    config.connectivity_check_interval_ms,
                    constants::MAX_FULLNODE_CONNECTIONS,
                    constants::NETWORK_CHANNEL_SIZE,
                );
            }
        }

        match &config.discovery_method {
            DiscoveryMethod::Gossip(gossip_config) => {
                network_builder.add_gossip_discovery(
                    gossip_config.advertised_address.clone(),
                    gossip_config.discovery_interval_ms,
                    public_key,
                );
            }
            DiscoveryMethod::Onchain => {
                network_builder.add_onchain_discovery(libra_db, waypoint);
            }
            DiscoveryMethod::None => {}
        }

        (runtime, network_builder)
    }

    pub fn build(&mut self, executor: &Handle) -> &mut Self {
        self.build_connection_monitoring(&executor)
            .build_connectivity_manager(&executor)
            .build_gossip_discovery(&executor)
            .build_onchain_discovery(&executor)
            .build_peer_manager(&executor);
        self
    }

    pub fn start(&mut self, executor: &Handle) -> &mut Self {
        self.start_connection_monitoring(executor)
            .start_connectivity_manager(executor)
            .start_gossip_discovery(executor)
            .start_onchain_discovery(executor)
            .start_peer_manager(executor);

        self
    }

    pub fn network_context(&self) -> Arc<NetworkContext> {
        self.network_context.clone()
    }

    pub fn peer_id(&self) -> PeerId {
        self.network_context.peer_id()
    }

    pub fn reconfig_subscriptions(&mut self) -> &mut Vec<ReconfigSubscription> {
        &mut self.reconfig_subscriptions
    }

    pub fn listen_address(&self) -> NetworkAddress {
        self.peer_manager_builder
            .as_ref()
            .expect("PeerManagerBuilder must exist")
            .listen_address()
    }

    /// Set trusted peers.
    pub fn trusted_peers(
        &mut self,
        trusted_peers: HashMap<PeerId, x25519::PublicKey>,
    ) -> &mut Self {
        *self.trusted_peers.write().unwrap() = trusted_peers;
        self
    }

    pub fn conn_mgr_reqs_tx(&self) -> Option<channel::Sender<ConnectivityRequest>> {
        match self.connectivity_manager_builder {
            Some(ref conn_mgr) => Some(conn_mgr.conn_mgr_reqs_tx()),
            None => None,
        }
    }

    pub fn add_connection_event_listener(&mut self) -> conn_notifs_channel::Receiver {
        match &mut self.peer_manager_builder {
            Some(builder) => builder.add_connection_event_listener(),
            None => panic!(
                "Cannot add a conneciton event listener if PeerManagerBuilder does not exist"
            ),
        }
    }

    /// Add a [`ConnectivityManager`] to the network.
    ///
    /// [`ConnectivityManager`] is responsible for ensuring that we are connected
    /// to a node iff. it is an eligible node and maintaining persistent
    /// connections with all eligible nodes. A list of eligible nodes is received
    /// at initialization, and updates are received on changes to system membership.
    ///
    /// Note: a connectivity manager should only be added if the network is
    /// permissioned.
    pub fn add_connectivity_manager(
        &mut self,
        seed_peers: HashMap<PeerId, Vec<NetworkAddress>>,
        connectivity_check_interval_ms: u64,
        max_fullnode_connections: usize,
        channel_size: usize,
    ) -> &mut Self {
        let pm_conn_mgr_notifs_rx = self.add_connection_event_listener();
        let connection_reqs_tx = match &self.peer_manager_builder {
            Some(builder) => builder.connection_reqs_tx(),
            None => {
                panic!("Cannot add a ConnectivityManager if the PeerManagerBuilder is not present")
            }
        };

        let connection_limit = if let RoleType::FullNode = self.network_context.role() {
            Some(max_fullnode_connections)
        } else {
            None
        };
        let builder = ConnectivityManagerBuilder::create(
            self.network_context(),
            self.trusted_peers.clone(),
            seed_peers,
            connectivity_check_interval_ms,
            // TODO:  Move this value to NetworkConfig
            2, // Hard-coded constant
            // TODO:  Move this value to NetworkConfig
            constants::MAX_CONNECTION_DELAY_MS,
            channel_size,
            ConnectionRequestSender::new(connection_reqs_tx),
            pm_conn_mgr_notifs_rx,
            connection_limit,
        );
        self.connectivity_manager_builder = Some(builder);

        if self.network_context.role() == RoleType::Validator {
            // Set up to listen for network configuration changes
            let (simple_discovery_reconfig_subscription, simple_discovery_reconfig_rx) =
                network_simple_onchain_discovery::gen_simple_discovery_reconfig_subscription();
            self.reconfig_subscriptions
                .push(simple_discovery_reconfig_subscription);
            self.configuration_change_listener_builder =
                Some(ConfigurationChangeListenerBuilder::create(
                    self.network_context.role(),
                    self.conn_mgr_reqs_tx().expect("We just set this value"),
                    simple_discovery_reconfig_rx,
                ));
        }
        self
    }

    /// Build the ConnectivityManager from the provided configuration.
    fn build_connectivity_manager(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut connectivity_builder) = self.connectivity_manager_builder {
            connectivity_builder.build(executor);
            if let Some(ref mut change_listener_builder) =
                self.configuration_change_listener_builder
            {
                change_listener_builder.build();
            }
        }
        self.start_connectivity_manager(executor)
    }

    /// Start the ConnectivityManager
    fn start_connectivity_manager(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut connectivity_builder) = self.connectivity_manager_builder {
            connectivity_builder.start(executor);
            debug!("Started ConnectivityManager");
            if let Some(ref mut change_listener_builder) =
                self.configuration_change_listener_builder
            {
                change_listener_builder.start(executor);
            }
        }
        self
    }

    /// Add the (gossip) [`Discovery`] protocol to the network.
    ///
    /// (gossip) [`Discovery`] discovers other eligible peers' network addresses
    /// by exchanging the full set of known peer network addresses with connected
    /// peers as a network protocol.
    ///
    /// This is for testing purposes only and should not be used in production networks.
    pub fn add_gossip_discovery(
        &mut self,
        advertised_address: NetworkAddress,
        discovery_interval_ms: u64,
        pubkey: x25519::PublicKey,
    ) -> &mut Self {
        let conn_mgr_reqs_tx = self
            .conn_mgr_reqs_tx()
            .expect("connectivityManager msut be enabled");
        // Get handles for network events and sender.
        let (discovery_network_tx, discovery_network_rx) =
            self.add_protocol_handler(discovery::network_endpoint_config());

        // TODO(philiphayes): the current setup for gossip discovery doesn't work
        // when we don't have an `advertised_address` set, since it uses the
        // `listen_address`, which might not be bound to a port yet. For example,
        // if our `listen_address` is "/ip6/::1/tcp/0" and `advertised_address` is
        // `None`, then this will set our `advertised_address` to something like
        // "/ip6/::1/tcp/0/ln-noise-ik/<pubkey>/ln-handshake/0", which is wrong
        // since the actual bound port will be something > 0.

        // TODO(philiphayes): in network_builder setup, only bind the channels.
        // wait until PeerManager is running to actual setup gossip discovery.

        let advertised_address = advertised_address.append_prod_protos(pubkey, HANDSHAKE_VERSION);

        let addrs = vec![advertised_address];
        let discovery_builder = DiscoveryBuilder::create(
            self.network_context.clone(),
            addrs,
            discovery_interval_ms,
            discovery_network_tx,
            discovery_network_rx,
            conn_mgr_reqs_tx,
        );

        self.discovery_builder = Some(discovery_builder);

        self
    }

    fn build_gossip_discovery(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut discovery_builder) = self.discovery_builder {
            discovery_builder.build(executor);
        }
        self.start_gossip_discovery(executor)
    }

    fn start_gossip_discovery(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut discovery) = self.discovery_builder {
            discovery.start(executor);
            debug!("{} Started discovery protocol actor", self.network_context);
        }
        self
    }

    fn add_onchain_discovery(
        &mut self,
        libra_db: Arc<dyn DbReader>,
        waypoint: Waypoint,
    ) -> &mut Self {
        let (network_tx, discovery_events) = self
            .add_protocol_handler(onchain_discovery::network_interface::network_endpoint_config());

        let onchain_discovery_builder = OnchainDiscoveryBuilder::create(
            self.conn_mgr_reqs_tx()
                .expect("ConnectivityManager must be installed"),
            network_tx,
            discovery_events,
            self.network_context(),
            libra_db,
            waypoint,
            // TODO: Move into NetworkConfig
            30, // Legacy hard-coded value.
            // TODO: move into NetworkConfig
            8, // Legacy hard-coded value.
        );

        self.onchain_discovery_builder = Some(onchain_discovery_builder);

        self
    }

    fn build_onchain_discovery(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut onchain_discovery_builder) = self.onchain_discovery_builder {
            onchain_discovery_builder.build(executor);
        }
        self.start_onchain_discovery(executor)
    }

    fn start_onchain_discovery(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut onchain_discovery_builder) = self.onchain_discovery_builder {
            onchain_discovery_builder.start(executor);
            debug!("Started Onchain Discovery");
        }
        self
    }
    pub fn add_connection_monitoring(
        &mut self,
        ping_interval_ms: u64,
        ping_timeout_ms: u64,
        ping_failures_tolerated: u64,
    ) -> &mut Self {
        // Initialize and start HealthChecker.
        let (hc_network_tx, hc_network_rx) =
            self.add_protocol_handler(health_checker::network_endpoint_config());
        let health_checker_builder = HealthCheckerBuilder::create(
            self.network_context.clone(),
            ping_interval_ms,
            ping_timeout_ms,
            ping_failures_tolerated,
            hc_network_tx,
            hc_network_rx,
        );

        self.health_checker_builder = Some(health_checker_builder);

        self
    }

    fn build_connection_monitoring(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut health_checker_builder) = self.health_checker_builder {
            health_checker_builder.build(executor);
        }
        self.start_connection_monitoring(executor)
    }

    fn start_connection_monitoring(&mut self, executor: &Handle) -> &mut Self {
        if let Some(ref mut health_checker_builder) = self.health_checker_builder {
            health_checker_builder.start(executor);
            debug!("{} Started health checker", self.network_context);
        }
        self
    }

    fn build_peer_manager(&mut self, executor: &Handle) -> &mut Self {
        self.peer_manager_builder
            .as_mut()
            .expect("PeerManagerBuilder must exist")
            .build(executor);
        self
    }

    fn start_peer_manager(&mut self, executor: &Handle) -> &mut Self {
        self.peer_manager_builder
            .as_mut()
            .expect("PeerManagerBuilder must exist")
            .start(executor);
        self
    }

    /// Adds a endpoints for the provided configuration.  Returns NetworkSender and NetworkEvent which
    /// can be attached to other components.
    pub fn add_protocol_handler<SenderT, EventT>(
        &mut self,
        (
                rpc_protocols,
                direct_send_protocols,
                queue_preference,
                max_queue_size_per_peer,
                counter,
            ): (
                Vec<ProtocolId>,
                Vec<ProtocolId>,
                QueueStyle,
                usize,
                Option<&'static IntCounterVec>,
            ),
    ) -> (SenderT, EventT)
    where
        EventT: NewNetworkEvents,
        SenderT: NewNetworkSender,
    {
        let (peer_mgr_reqs_tx, peer_mgr_reqs_rx, connection_reqs_tx, connection_notifs_rx) =
            match &mut self.peer_manager_builder {
                Some(builder) => builder.add_protocol_handler(
                    rpc_protocols,
                    direct_send_protocols,
                    queue_preference,
                    max_queue_size_per_peer,
                    counter,
                ),
                None => panic!("Cannot add protocol handler if PeerManagerBuilder does not exist"),
            };
        (
            SenderT::new(peer_mgr_reqs_tx, connection_reqs_tx),
            EventT::new(peer_mgr_reqs_rx, connection_notifs_rx),
        )
    }
}

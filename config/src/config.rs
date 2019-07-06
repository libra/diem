// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    config::ConsensusProposerType::{FixedProposer, RotatingProposer},
    seed_peers::{SeedPeersConfig, SeedPeersConfigHelpers},
    trusted_peers::{
        deserialize_key, serialize_key, TrustedPeerPrivateKeys, TrustedPeersConfig,
        TrustedPeersConfigHelpers,
    },
    utils::{deserialize_whitelist, get_available_port, get_local_ip, serialize_whitelist},
};
use crypto::{
    signing,
    x25519::{self, X25519PrivateKey, X25519PublicKey},
};
use failure::prelude::*;
use logger::LoggerType;
use parity_multiaddr::{Multiaddr, Protocol};
use proto_conv::FromProtoBytes;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    string::ToString,
};
use tempfile::TempDir;
use toml;
use types::transaction::{SignedTransaction, SCRIPT_HASH_LENGTH};

#[cfg(test)]
#[path = "unit_tests/config_test.rs"]
mod config_test;

pub const DISPOSABLE_DIR_MARKER: &str = "<USE_TEMP_DIR>";

// path is relative to this file location
static CONFIG_TEMPLATE: &[u8] = include_bytes!("../data/configs/node.config.toml");

/// Config pulls in configuration information from the config file.
/// This is used to set up the nodes and configure various parameters.
/// The config file is broken up into sections for each module
/// so that only that module can be passed around
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NodeConfig {
    //TODO Add configuration for multiple chain's in a future diff
    pub base: BaseConfig,
    pub metrics: MetricsConfig,
    pub execution: ExecutionConfig,
    pub admission_control: AdmissionControlConfig,
    pub debug_interface: DebugInterfaceConfig,

    pub storage: StorageConfig,
    pub network: NetworkConfig,
    pub consensus: ConsensusConfig,
    pub mempool: MempoolConfig,
    pub log_collector: LoggerConfig,
    pub vm_config: VMConfig,

    pub secret_service: SecretServiceConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BaseConfig {
    pub peer_id: String,
    // peer_keypairs contains all the node's private keys,
    // it is filled later on from a different file
    #[serde(skip)]
    pub peer_keypairs: KeyPairs,
    // peer_keypairs_file contains the configuration file containing all the node's private keys.
    pub peer_keypairs_file: PathBuf,
    pub data_dir_path: PathBuf,
    #[serde(skip)]
    temp_data_dir: Option<TempDir>,
    //TODO move set of trusted peers into genesis file
    pub trusted_peers_file: String,
    #[serde(skip)]
    pub trusted_peers: TrustedPeersConfig,

    // Size of chunks to request when performing restart sync to catchup
    pub node_sync_batch_size: u64,

    // Number of retries per chunk download
    pub node_sync_retries: usize,

    // Buffer size for sync_channel used for node syncing (number of elements that it can
    // hold before it blocks on sends)
    pub node_sync_channel_buffer_size: u64,

    // chan_size of slog async drain for node logging.
    pub node_async_log_chan_size: usize,
}

// KeyPairs is used to store all of a node's private keys.
// It is filled via a config file at the moment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyPairs {
    #[serde(serialize_with = "serialize_key")]
    #[serde(deserialize_with = "deserialize_key")]
    network_signing_private_key: signing::PrivateKey,
    #[serde(serialize_with = "serialize_key")]
    #[serde(deserialize_with = "deserialize_key")]
    network_signing_public_key: signing::PublicKey,

    #[serde(serialize_with = "serialize_key")]
    #[serde(deserialize_with = "deserialize_key")]
    network_identity_private_key: X25519PrivateKey,
    #[serde(serialize_with = "serialize_key")]
    #[serde(deserialize_with = "deserialize_key")]
    network_identity_public_key: X25519PublicKey,

    #[serde(serialize_with = "serialize_key")]
    #[serde(deserialize_with = "deserialize_key")]
    consensus_private_key: signing::PrivateKey,
    #[serde(serialize_with = "serialize_key")]
    #[serde(deserialize_with = "deserialize_key")]
    consensus_public_key: signing::PublicKey,
}

// required for serialization
impl Default for KeyPairs {
    fn default() -> Self {
        let (private_sig, public_sig) = signing::generate_keypair();
        let (private_kex, public_kex) = x25519::generate_keypair();
        Self {
            network_signing_private_key: private_sig.clone(),
            network_signing_public_key: public_sig,
            network_identity_private_key: private_kex.clone(),
            network_identity_public_key: public_kex,
            consensus_private_key: private_sig.clone(),
            consensus_public_key: public_sig,
        }
    }
}

impl KeyPairs {
    // used to deserialize keypairs from a configuration file
    pub fn load_config<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        let mut file = File::open(path)
            .unwrap_or_else(|_| panic!("Cannot open KeyPair Config file {:?}", path));
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .unwrap_or_else(|_| panic!("Error reading KeyPair Config file {:?}", path));

        Self::parse(&contents)
    }
    fn parse(config_string: &str) -> Self {
        toml::from_str(config_string).expect("Unable to parse Config")
    }
    // used to serialize keypairs to a configuration file
    pub fn save_config<P: AsRef<Path>>(&self, output_file: P) {
        let contents = toml::to_vec(&self).expect("Error serializing");

        let mut file = File::create(output_file).expect("Error opening file");

        file.write_all(&contents).expect("Error writing file");
    }
    // used in testing to fill the structure with test keypairs
    pub fn load(private_keys: &TrustedPeerPrivateKeys) -> Self {
        let network_signing_private_key = private_keys.get_network_signing_private();
        let network_signing_public_key = (&network_signing_private_key).into();
        let network_identity_private_key = private_keys.get_network_identity_private();
        let network_identity_public_key = (&network_identity_private_key).into();
        let consensus_private_key = private_keys.get_consensus_private();
        let consensus_public_key = (&consensus_private_key).into();
        Self {
            network_signing_private_key,
            network_signing_public_key,
            network_identity_private_key,
            network_identity_public_key,
            consensus_private_key,
            consensus_public_key,
        }
    }
    // getters for private keys
    pub fn get_network_signing_private(&self) -> signing::PrivateKey {
        self.network_signing_private_key.clone()
    }
    pub fn get_network_identity_private(&self) -> X25519PrivateKey {
        self.network_identity_private_key.clone()
    }
    pub fn get_consensus_private(&self) -> signing::PrivateKey {
        self.consensus_private_key.clone()
    }
    // getters for public keys
    pub fn get_network_signing_public(&self) -> signing::PublicKey {
        self.network_signing_public_key
    }
    pub fn get_network_identity_public(&self) -> X25519PublicKey {
        self.network_identity_public_key
    }
    pub fn get_consensus_public(&self) -> signing::PublicKey {
        self.consensus_public_key
    }
    // getters for keypairs
    pub fn get_network_signing_keypair(&self) -> (signing::PrivateKey, signing::PublicKey) {
        (
            self.get_network_signing_private(),
            self.get_network_signing_public(),
        )
    }
    pub fn get_network_identity_keypair(&self) -> (X25519PrivateKey, X25519PublicKey) {
        (
            self.get_network_identity_private(),
            self.get_network_identity_public(),
        )
    }
    pub fn get_consensus_keypair(&self) -> (signing::PrivateKey, signing::PublicKey) {
        (self.get_consensus_private(), self.get_consensus_public())
    }
}

impl Clone for BaseConfig {
    fn clone(&self) -> Self {
        Self {
            peer_id: self.peer_id.clone(),
            peer_keypairs: self.peer_keypairs.clone(),
            peer_keypairs_file: self.peer_keypairs_file.clone(),
            data_dir_path: self.data_dir_path.clone(),
            temp_data_dir: None,
            trusted_peers_file: self.trusted_peers_file.clone(),
            trusted_peers: self.trusted_peers.clone(),
            node_sync_batch_size: self.node_sync_batch_size,
            node_sync_retries: self.node_sync_retries,
            node_sync_channel_buffer_size: self.node_sync_channel_buffer_size,
            node_async_log_chan_size: self.node_async_log_chan_size,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MetricsConfig {
    pub dir: PathBuf,
    pub collection_interval_ms: u64,
    pub push_server_addr: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ExecutionConfig {
    pub address: String,
    pub port: u16,
    // directive to load the testnet genesis block or the default genesis block.
    // There are semantic differences between the 2 genesis related to minting and
    // account creation
    pub testnet_genesis: bool,
    pub genesis_file_location: String,
}

impl ExecutionConfig {
    pub fn get_genesis_transaction(&self) -> Result<SignedTransaction> {
        let mut file = File::open(self.genesis_file_location.clone())?;
        let mut buffer = vec![];
        file.read_to_end(&mut buffer)?;
        SignedTransaction::from_proto_bytes(&buffer)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoggerConfig {
    pub http_endpoint: Option<String>,
    pub is_async: bool,
    pub chan_size: Option<usize>,
    pub use_std_output: bool,
}

impl LoggerConfig {
    pub fn get_log_collector_type(&self) -> Option<LoggerType> {
        // There is priority between different logger. If multiple ones are specified, only
        // the higher one will be returned.
        if self.http_endpoint.is_some() {
            return Some(LoggerType::Http(
                self.http_endpoint
                    .clone()
                    .expect("Http endpoint not available for logger"),
            ));
        } else if self.use_std_output {
            return Some(LoggerType::StdOutput);
        }
        None
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SecretServiceConfig {
    pub address: String,
    pub secret_service_port: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AdmissionControlConfig {
    pub address: String,
    pub admission_control_service_port: u16,
    pub need_to_check_mempool_before_validation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DebugInterfaceConfig {
    pub admission_control_node_debug_port: u16,
    pub secret_service_node_debug_port: u16,
    pub storage_node_debug_port: u16,
    // This has similar use to the core-node-debug-server itself
    pub metrics_server_port: u16,
    pub address: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StorageConfig {
    pub address: String,
    pub port: u16,
    pub dir: PathBuf,
}

impl StorageConfig {
    pub fn get_dir(&self) -> &Path {
        &self.dir
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NetworkConfig {
    pub seed_peers_file: String,
    #[serde(skip)]
    pub seed_peers: SeedPeersConfig,
    // TODO: Add support for multiple listen/advertised addresses in config.
    // The address that this node is listening on for new connections.
    pub listen_address: Multiaddr,
    // The address that this node advertises to other nodes for the discovery protocol.
    pub advertised_address: Multiaddr,
    pub discovery_interval_ms: u64,
    pub connectivity_check_interval_ms: u64,
    pub enable_encryption_and_authentication: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConsensusConfig {
    max_block_size: u64,
    proposer_type: String,
    contiguous_rounds: u32,
    max_pruned_blocks_in_mem: Option<u64>,
    pacemaker_initial_timeout_ms: Option<u64>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ConsensusProposerType {
    // Choose the smallest PeerId as the proposer
    FixedProposer,
    // Round robin rotation of proposers
    RotatingProposer,
}

impl ConsensusConfig {
    pub fn get_proposer_type(&self) -> ConsensusProposerType {
        match self.proposer_type.as_str() {
            "fixed_proposer" => FixedProposer,
            "rotating_proposer" => RotatingProposer,
            &_ => unimplemented!("Invalid proposer type: {}", self.proposer_type),
        }
    }

    pub fn contiguous_rounds(&self) -> u32 {
        self.contiguous_rounds
    }

    pub fn max_block_size(&self) -> u64 {
        self.max_block_size
    }

    pub fn max_pruned_blocks_in_mem(&self) -> &Option<u64> {
        &self.max_pruned_blocks_in_mem
    }

    pub fn pacemaker_initial_timeout_ms(&self) -> &Option<u64> {
        &self.pacemaker_initial_timeout_ms
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MempoolConfig {
    pub broadcast_transactions: bool,
    pub shared_mempool_tick_interval_ms: u64,
    pub shared_mempool_batch_size: usize,
    pub shared_mempool_max_concurrent_inbound_syncs: usize,
    pub capacity: usize,
    // max number of transactions per user in Mempool
    pub capacity_per_user: usize,
    pub sequence_cache_capacity: usize,
    pub system_transaction_timeout_secs: u64,
    pub system_transaction_gc_interval_ms: u64,
    pub mempool_service_port: u16,
    pub address: String,
}

impl NodeConfig {
    /// Reads the config file and returns the configuration object
    pub fn load_template<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let mut file =
            File::open(path).with_context(|_| format!("Cannot open NodeConfig file {:?}", path))?;
        let mut config_string = String::new();
        file.read_to_string(&mut config_string)
            .with_context(|_| format!("Cannot read NodeConfig file {:?}", path))?;

        let config = Self::parse(&config_string)
            .with_context(|_| format!("Cannot parse NodeConfig file {:?}", path))?;

        Ok(config)
    }

    /// Reads the config file and returns the configuration object in addition to doing some
    /// post-processing of the config
    /// Paths used in the config are either absolute or relative to the config location
    pub fn load_config<P: AsRef<Path>>(peer_id: Option<String>, path: P) -> Result<Self> {
        let mut config = Self::load_template(&path)?;
        // Allow peer_id override if set
        if let Some(peer_id) = peer_id {
            config.base.peer_id = peer_id;
        }
        if !config.base.trusted_peers_file.is_empty() {
            config.base.trusted_peers = TrustedPeersConfig::load_config(
                path.as_ref()
                    .with_file_name(&config.base.trusted_peers_file),
            );
        }
        if !config.base.peer_keypairs_file.as_os_str().is_empty() {
            config.base.peer_keypairs = KeyPairs::load_config(
                path.as_ref()
                    .with_file_name(&config.base.peer_keypairs_file),
            );
        }
        if !config.network.seed_peers_file.is_empty() {
            config.network.seed_peers = SeedPeersConfig::load_config(
                path.as_ref()
                    .with_file_name(&config.network.seed_peers_file),
            );
        }
        if config.network.advertised_address.to_string().is_empty() {
            config.network.advertised_address =
                get_local_ip().ok_or_else(|| ::failure::err_msg("No local IP"))?;
        }
        if config.network.listen_address.to_string().is_empty() {
            config.network.listen_address =
                get_local_ip().ok_or_else(|| ::failure::err_msg("No local IP"))?;
        }
        NodeConfigHelpers::update_data_dir_path_if_needed(&mut config, &path)?;
        Ok(config)
    }

    pub fn save_config<P: AsRef<Path>>(&self, output_file: P) {
        let contents = toml::to_vec(&self).expect("Error serializing");
        let mut file = File::create(output_file).expect("Error opening file");

        file.write_all(&contents).expect("Error writing file");
    }

    /// Parses the config file into a Config object
    pub fn parse(config_string: &str) -> Result<Self> {
        assert!(!config_string.is_empty());
        Ok(toml::from_str(config_string)?)
    }

    /// Returns the peer info for this node
    pub fn own_addrs(&self) -> (String, Vec<Multiaddr>) {
        let own_peer_id = self.base.peer_id.clone();
        let own_addrs = vec![self.network.advertised_address.clone()];
        (own_peer_id, own_addrs)
    }
}

// Given a multiaddr, randomizes its Tcp port if present.
fn randomize_tcp_port(addr: &Multiaddr) -> Multiaddr {
    let mut new_addr = Multiaddr::empty();
    for p in addr.iter() {
        if let Protocol::Tcp(_) = p {
            new_addr.push(Protocol::Tcp(get_available_port()));
        } else {
            new_addr.push(p);
        }
    }
    new_addr
}

fn get_tcp_port(addr: &Multiaddr) -> Option<u16> {
    for p in addr.iter() {
        if let Protocol::Tcp(port) = p {
            return Some(port);
        }
    }
    None
}

pub struct NodeConfigHelpers {}

impl NodeConfigHelpers {
    /// Returns a simple test config for single node. It does not have correct trusted_peers_file,
    /// peer_keypairs_file, and seed_peers_file set and expected that callee will provide these
    pub fn get_single_node_test_config(random_ports: bool) -> NodeConfig {
        Self::get_single_node_test_config_publish_options(random_ports, None)
    }

    /// Returns a simple test config for single node. It does not have correct trusted_peers_file,
    /// peer_keypairs_file, and seed_peers_file set and expected that callee will provide these
    /// `publishing_options` is either one of either `Open` or `CustomScripts` only.
    pub fn get_single_node_test_config_publish_options(
        random_ports: bool,
        publishing_options: Option<VMPublishingOption>,
    ) -> NodeConfig {
        let config_string = String::from_utf8_lossy(CONFIG_TEMPLATE);
        let mut config =
            NodeConfig::parse(&config_string).expect("Error parsing single node test config");
        if random_ports {
            NodeConfigHelpers::randomize_config_ports(&mut config);
        }

        if let Some(vm_publishing_option) = publishing_options {
            config.vm_config.publishing_options = vm_publishing_option;
        }

        let (peers_private_keys, trusted_peers_test) =
            TrustedPeersConfigHelpers::get_test_config(1, None);
        let peer_id = trusted_peers_test.peers.keys().collect::<Vec<_>>()[0];
        config.base.peer_id = peer_id.clone();
        // load node's keypairs
        let private_keys = peers_private_keys.get(peer_id.as_str()).unwrap();
        config.base.peer_keypairs = KeyPairs::load(private_keys);
        config.base.trusted_peers = trusted_peers_test;
        config.network.seed_peers = SeedPeersConfigHelpers::get_test_config(
            &config.base.trusted_peers,
            get_tcp_port(&config.network.advertised_address),
        );
        NodeConfigHelpers::update_data_dir_path_if_needed(&mut config, ".")
            .expect("creating tempdir");
        config
    }

    /// Replaces temp marker with the actual path and returns holder to the temp dir.
    fn update_data_dir_path_if_needed<P: AsRef<Path>>(
        config: &mut NodeConfig,
        base_path: P,
    ) -> Result<()> {
        if config.base.data_dir_path == Path::new(DISPOSABLE_DIR_MARKER) {
            let dir = tempfile::tempdir().context("error creating tempdir")?;
            config.base.data_dir_path = dir.path().to_owned();
            config.base.temp_data_dir = Some(dir);
        }

        if !config.metrics.dir.as_os_str().is_empty() {
            // do not set the directory if metrics.dir is empty to honor the check done
            // in setup_metrics
            config.metrics.dir = config.base.data_dir_path.join(&config.metrics.dir);
        }
        config.storage.dir = config.base.data_dir_path.join(config.storage.get_dir());
        if config.execution.genesis_file_location == DISPOSABLE_DIR_MARKER {
            config.execution.genesis_file_location = config
                .base
                .data_dir_path
                .join("genesis.blob")
                .to_str()
                .unwrap()
                .to_string();
        }
        config.execution.genesis_file_location = base_path
            .as_ref()
            .with_file_name(&config.execution.genesis_file_location)
            .to_str()
            .unwrap()
            .to_string();

        Ok(())
    }

    pub fn randomize_config_ports(config: &mut NodeConfig) {
        config.admission_control.admission_control_service_port = get_available_port();
        config.debug_interface.admission_control_node_debug_port = get_available_port();
        config.debug_interface.metrics_server_port = get_available_port();
        config.debug_interface.secret_service_node_debug_port = get_available_port();
        config.debug_interface.storage_node_debug_port = get_available_port();
        config.execution.port = get_available_port();
        config.mempool.mempool_service_port = get_available_port();
        config.network.advertised_address = randomize_tcp_port(&config.network.advertised_address);
        config.network.listen_address = randomize_tcp_port(&config.network.listen_address);
        config.secret_service.secret_service_port = get_available_port();
        config.storage.port = get_available_port();
    }
}

/// Holds the VM configuration, currently this is only the publishing options for scripts and
/// modules, but in the future this may need to be expanded to hold more information.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VMConfig {
    pub publishing_options: VMPublishingOption,
}

/// Defines and holds the publishing policies for the VM. There are three possible configurations:
/// 1. No module publishing, only whitelisted scripts are allowed.
/// 2. No module publishing, custom scripts are allowed.
/// 3. Both module publishing and custom scripts are allowed.
/// We represent these as an enum instead of a struct since whitelisting and module/script
/// publishing are mutually exclusive options.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "whitelist")]
pub enum VMPublishingOption {
    /// Only allow scripts on a whitelist to be run
    #[serde(deserialize_with = "deserialize_whitelist")]
    #[serde(serialize_with = "serialize_whitelist")]
    Locked(HashSet<[u8; SCRIPT_HASH_LENGTH]>),
    /// Allow custom scripts, but _not_ custom module publishing
    CustomScripts,
    /// Allow both custom scripts and custom module publishing
    Open,
}

impl VMPublishingOption {
    pub fn custom_scripts_only(&self) -> bool {
        !self.is_open() && !self.is_locked()
    }

    pub fn is_open(&self) -> bool {
        match self {
            VMPublishingOption::Open => true,
            _ => false,
        }
    }

    pub fn is_locked(&self) -> bool {
        match self {
            VMPublishingOption::Locked { .. } => true,
            _ => false,
        }
    }

    pub fn get_whitelist_set(&self) -> Option<&HashSet<[u8; SCRIPT_HASH_LENGTH]>> {
        match self {
            VMPublishingOption::Locked(whitelist) => Some(&whitelist),
            _ => None,
        }
    }
}

impl VMConfig {
    /// Creates a new `VMConfig` where the whitelist is empty. This should only be used for testing.
    #[allow(non_snake_case)]
    #[doc(hidden)]
    pub fn empty_whitelist_FOR_TESTING() -> Self {
        VMConfig {
            publishing_options: VMPublishingOption::Locked(HashSet::new()),
        }
    }

    pub fn save_config<P: AsRef<Path>>(&self, output_file: P) {
        let contents = toml::to_vec(&self).expect("Error serializing");

        let mut file = File::create(output_file).expect("Error opening file");

        file.write_all(&contents).expect("Error writing file");
    }
}

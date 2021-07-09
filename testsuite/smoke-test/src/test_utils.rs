// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::smoke_test_environment::SmokeTestEnvironment;
use cli::client_proxy::ClientProxy;
use diem_config::config::{Identity, NodeConfig, SecureBackend};
use diem_crypto::ed25519::Ed25519PublicKey;
use diem_types::account_address::AccountAddress;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use std::{collections::BTreeMap, fs::File, io::Write, path::PathBuf, str::FromStr};

// TODO(joshlind): Refactor all of these so that they can be contained within the calling
// test files and not shared across all tests.
pub fn compare_balances(
    expected_balances: Vec<(f64, String)>,
    extracted_balances: Vec<String>,
) -> bool {
    if extracted_balances.len() != extracted_balances.len() {
        return false;
    }

    let extracted_balances_dec: BTreeMap<_, _> = extracted_balances
        .into_iter()
        .map(|balance_str| {
            let (currency_code, stripped_str) = if balance_str.ends_with("XUS") {
                ("XUS", balance_str.trim_end_matches("XUS"))
            } else if balance_str.ends_with("XDX") {
                ("XDX", balance_str.trim_end_matches("XDX"))
            } else {
                panic!("Unexpected currency type returned for balance")
            };
            (currency_code, Decimal::from_str(stripped_str).ok())
        })
        .collect();

    expected_balances
        .into_iter()
        .all(|(balance, currency_code)| {
            if let Some(extracted_balance) = extracted_balances_dec.get(currency_code.as_str()) {
                Decimal::from_f64(balance) == *extracted_balance
            } else {
                false
            }
        })
}

/// Sets up a SmokeTestEnvironment with specified size and connects a client
/// proxy to the node_index.
pub fn setup_swarm_and_client_proxy(
    num_nodes: usize,
    node_index: usize,
) -> (SmokeTestEnvironment, ClientProxy) {
    let mut env = SmokeTestEnvironment::new(num_nodes);
    env.validator_swarm.launch();

    let client = env.get_validator_client(node_index, None);
    (env, client)
}

/// Waits for a transaction to be processed by all validator nodes in the smoke
/// test environment.
pub fn wait_for_transaction_on_all_nodes(
    env: &SmokeTestEnvironment,
    num_nodes: usize,
    account: AccountAddress,
    sequence_number: u64,
) {
    for i in 0..num_nodes {
        let client = env.get_validator_client(i, None);
        client
            .wait_for_transaction(account, sequence_number)
            .unwrap();
    }
}

/// This module provides useful functions for operating, handling and managing
/// DiemSwarm instances. It is particularly useful for working with tests that
/// require a SmokeTestEnvironment, as it provides a generic interface across
/// DiemSwarms, regardless of if the swarm is a validator swarm, validator full
/// node swarm, or a public full node swarm.
pub mod diem_swarm_utils {
    use crate::test_utils::fetch_backend_storage;
    use cli::client_proxy::ClientProxy;
    use diem_config::config::{NodeConfig, SecureBackend, WaypointConfig};
    use diem_key_manager::diem_interface::JsonRpcDiemInterface;
    use diem_operational_tool::test_helper::OperationalTool;
    use diem_secure_storage::{KVStorage, Storage};
    use diem_swarm::swarm::DiemSwarm;
    use diem_transaction_replay::DiemDebugger;
    use diem_types::{chain_id::ChainId, waypoint::Waypoint};
    use std::path::PathBuf;

    /// Returns a new client proxy connected to the given swarm at the specified
    /// node index.
    pub fn get_client_proxy(
        swarm: &DiemSwarm,
        node_index: usize,
        diem_root_key_path: &str,
        mnemonic_file_path: PathBuf,
        waypoint: Option<Waypoint>,
    ) -> ClientProxy {
        let port = swarm.get_client_port(node_index);

        let mnemonic_file_path = mnemonic_file_path
            .canonicalize()
            .expect("Unable to get canonical path of mnemonic_file_path")
            .to_str()
            .unwrap()
            .to_string();

        ClientProxy::new(
            ChainId::test(),
            &format!("http://localhost:{}/v1", port),
            &diem_root_key_path,
            &diem_root_key_path,
            &diem_root_key_path,
            false,
            /* faucet server */ None,
            Some(mnemonic_file_path),
            waypoint.unwrap_or(swarm.config.waypoint),
            true,
        )
        .unwrap()
    }

    /// Returns the JSON RPC url pointing to a node at the given
    /// node index.
    pub fn get_json_rpc_url(swarm: &DiemSwarm, node_index: usize) -> String {
        format!("http://127.0.0.1:{}", swarm.get_client_port(node_index))
    }

    /// Returns a JSON RPC based Diem Interface pointing to a node at the given
    /// node index.
    pub fn get_json_rpc_diem_interface(
        swarm: &DiemSwarm,
        node_index: usize,
    ) -> JsonRpcDiemInterface {
        let json_rpc_endpoint = format!("http://127.0.0.1:{}", swarm.get_client_port(node_index));
        JsonRpcDiemInterface::new(json_rpc_endpoint)
    }

    /// Returns a Diem Debugger pointing to a node at the given node index.
    pub fn get_diem_debugger(swarm: &DiemSwarm, node_index: usize) -> DiemDebugger {
        let (node_config, _) = load_node_config(swarm, node_index);
        let swarm_rpc_endpoint =
            format!("http://localhost:{}", node_config.json_rpc.address.port());
        DiemDebugger::json_rpc(swarm_rpc_endpoint.as_str()).unwrap()
    }

    /// Returns an operational tool pointing to a validator node at the given node index.
    pub fn get_op_tool(swarm: &DiemSwarm, node_index: usize) -> OperationalTool {
        OperationalTool::new(
            format!("http://127.0.0.1:{}", swarm.get_client_port(node_index)),
            ChainId::test(),
        )
    }

    /// Loads the nodes's storage backend identified by the node index in the given swarm.
    pub fn load_backend_storage(swarm: &DiemSwarm, node_index: usize) -> SecureBackend {
        let (node_config, _) = load_node_config(swarm, node_index);
        fetch_backend_storage(&node_config, None)
    }

    /// Loads the diem root's storage backend identified by the node index in the given swarm.
    pub fn load_diem_root_storage(swarm: &DiemSwarm, _node_index: usize) -> SecureBackend {
        SecureBackend::OnDiskStorage(swarm.config.root_storage.clone())
    }

    /// Loads the node config for the validator at the specified index. Also returns the node
    /// config path.
    pub fn load_node_config(swarm: &DiemSwarm, node_index: usize) -> (NodeConfig, PathBuf) {
        let node_config_path = swarm.config.config_files.get(node_index).unwrap();
        let node_config = NodeConfig::load(&node_config_path).unwrap();
        (node_config, node_config_path.clone())
    }

    /// Saves the node config for the node at the specified index in the given swarm.
    pub fn save_node_config(node_config: &mut NodeConfig, swarm: &DiemSwarm, node_index: usize) {
        let node_config_path = swarm.config.config_files.get(node_index).unwrap();
        node_config.save(node_config_path).unwrap();
    }

    pub fn insert_waypoint(node_config: &mut NodeConfig, waypoint: Waypoint) {
        let f = |backend: &SecureBackend| {
            let mut storage: Storage = backend.into();
            storage
                .set(diem_global_constants::WAYPOINT, waypoint)
                .expect("Unable to write waypoint");
            storage
                .set(diem_global_constants::GENESIS_WAYPOINT, waypoint)
                .expect("Unable to write waypoint");
        };
        let backend = &node_config.consensus.safety_rules.backend;
        f(backend);
        match &node_config.base.waypoint {
            WaypointConfig::FromStorage(backend) => {
                f(backend);
            }
            _ => panic!("unexpected waypoint from node config"),
        }
    }
}

/// Loads the node's storage backend from the given node config. If a namespace
/// is specified, the storage namespace will be overridden.
fn fetch_backend_storage(
    node_config: &NodeConfig,
    overriding_namespace: Option<String>,
) -> SecureBackend {
    if let Identity::FromStorage(storage_identity) =
        &node_config.validator_network.as_ref().unwrap().identity
    {
        match storage_identity.backend.clone() {
            SecureBackend::OnDiskStorage(mut config) => {
                if let Some(namespace) = overriding_namespace {
                    config.namespace = Some(namespace);
                }
                SecureBackend::OnDiskStorage(config)
            }
            _ => unimplemented!("On-disk storage is the only backend supported in smoke tests"),
        }
    } else {
        panic!("Couldn't load identity from storage");
    }
}

/// Writes a given public key to a file specified by the given path using hex encoding.
/// Contents are written using utf-8 encoding and a newline is appended to ensure that
/// whitespace can be handled by tests.
pub fn write_key_to_file_hex_format(key: &Ed25519PublicKey, key_file_path: PathBuf) {
    let hex_encoded_key = hex::encode(key.to_bytes());
    let key_and_newline = hex_encoded_key + "\n";
    let mut file = File::create(key_file_path).unwrap();
    file.write_all(&key_and_newline.as_bytes()).unwrap();
}

/// Writes a given public key to a file specified by the given path using bcs encoding.
pub fn write_key_to_file_bcs_format(key: &Ed25519PublicKey, key_file_path: PathBuf) {
    let bcs_encoded_key = bcs::to_bytes(&key).unwrap();
    let mut file = File::create(key_file_path).unwrap();
    file.write_all(&bcs_encoded_key).unwrap();
}

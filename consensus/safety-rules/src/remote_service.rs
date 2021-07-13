// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    persistent_safety_storage::PersistentSafetyStorage,
    serializer::{SafetyRulesInput, SerializerClient, SerializerService, TSerializerClient},
    Error, SafetyRules, TSafetyRules,
};
use diem_logger::warn;
use diem_secure_net::{NetworkClient, NetworkServer};
use std::net::SocketAddr;

pub trait RemoteService {
    fn client(&self) -> SerializerClient {
        let network_client = NetworkClient::new(
            "safety-rules",
            self.server_address(),
            self.network_timeout_ms(),
        );
        let service = Box::new(RemoteClient::new(network_client));
        SerializerClient::new_client(service)
    }

    fn server_address(&self) -> SocketAddr;

    /// Network Timeout in milliseconds.
    fn network_timeout_ms(&self) -> u64;
}

pub fn execute(
    storage: PersistentSafetyStorage,
    listen_addr: SocketAddr,
    verify_vote_proposal_signature: bool,
    export_consensus_key: bool,
    network_timeout_ms: u64,
    decoupled_execution: bool,
) {
    let mut safety_rules = SafetyRules::new(
        storage,
        verify_vote_proposal_signature,
        export_consensus_key,
        decoupled_execution,
    );
    if let Err(e) = safety_rules.consensus_state() {
        warn!("Unable to print consensus state: {}", e);
    }

    let mut serializer_service = SerializerService::new(safety_rules);
    let mut network_server = NetworkServer::new("safety-rules", listen_addr, network_timeout_ms);

    loop {
        if let Err(e) = process_one_message(&mut network_server, &mut serializer_service) {
            warn!("Failed to process message: {}", e);
        }
    }
}

fn process_one_message(
    network_server: &mut NetworkServer,
    serializer_service: &mut SerializerService,
) -> Result<(), Error> {
    let request = network_server.read()?;
    let response = serializer_service.handle_message(request)?;
    network_server.write(&response)?;
    Ok(())
}

struct RemoteClient {
    network_client: NetworkClient,
}

impl RemoteClient {
    pub fn new(network_client: NetworkClient) -> Self {
        Self { network_client }
    }

    fn process_one_message(&mut self, input: &[u8]) -> Result<Vec<u8>, Error> {
        self.network_client.write(&input)?;
        self.network_client.read().map_err(|e| e.into())
    }
}

impl TSerializerClient for RemoteClient {
    fn request(&mut self, input: SafetyRulesInput) -> Result<Vec<u8>, Error> {
        let input_message = bcs::to_bytes(&input)?;
        loop {
            match self.process_one_message(&input_message) {
                Err(err) => warn!("Failed to communicate with SafetyRules service: {}", err),
                Ok(value) => return Ok(value),
            }
        }
    }
}

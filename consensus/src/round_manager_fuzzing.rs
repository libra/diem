// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    block_storage::BlockStore,
    liveness::{
        proposal_generator::ProposalGenerator,
        rotating_proposer_election::RotatingProposer,
        round_state::{ExponentialTimeInterval, NewRoundEvent, NewRoundReason, RoundState},
    },
    metrics_safety_rules::MetricsSafetyRules,
    network::NetworkSender,
    network_interface::ConsensusNetworkSender,
    persistent_liveness_storage::{PersistentLivenessStorage, RecoveryData},
    round_manager::RoundManager,
    test_utils::{EmptyStateComputer, MockStorage, MockTransactionManager},
    util::{mock_time_service::SimulatedTimeService, time_service::TimeService},
};
use channel::{self, diem_channel, message_queues::QueueStyle};
use consensus_types::proposal_msg::ProposalMsg;
use diem_infallible::Mutex;
use diem_types::{
    epoch_change::EpochChangeProof,
    epoch_state::EpochState,
    ledger_info::{LedgerInfo, LedgerInfoWithSignatures},
    on_chain_config::ValidatorSet,
    validator_info::ValidatorInfo,
    validator_signer::ValidatorSigner,
    validator_verifier::ValidatorVerifier,
};
use futures::{channel::mpsc, executor::block_on};
use network::{
    peer_manager::{ConnectionRequestSender, PeerManagerRequestSender},
    protocols::network::NewNetworkSender,
};
use once_cell::sync::Lazy;
use safety_rules::{test_utils, SafetyRules, TSafetyRules};
use std::{
    collections::BTreeMap,
    sync::{atomic::AtomicU64, Arc},
    time::Duration,
};
use tokio::runtime::Runtime;

// This generates a proposal for round 1
pub fn generate_corpus_proposal() -> Vec<u8> {
    let mut round_manager = create_node_for_fuzzing();
    block_on(async {
        let proposal = round_manager
            .generate_proposal(NewRoundEvent {
                round: 1,
                reason: NewRoundReason::QCReady,
                timeout: std::time::Duration::new(5, 0),
            })
            .await;
        // serialize and return proposal
        bcs::to_bytes(&proposal.unwrap()).unwrap()
    })
}

// optimization for the fuzzer
static STATIC_RUNTIME: Lazy<Runtime> = Lazy::new(|| Runtime::new().unwrap());
static FUZZING_SIGNER: Lazy<ValidatorSigner> = Lazy::new(|| ValidatorSigner::from_int(1));

// helpers
fn build_empty_store(
    storage: Arc<dyn PersistentLivenessStorage>,
    initial_data: RecoveryData,
) -> Arc<BlockStore> {
    let (_commit_cb_sender, _commit_cb_receiver) = mpsc::unbounded::<LedgerInfoWithSignatures>();

    Arc::new(BlockStore::new(
        storage,
        initial_data,
        Arc::new(EmptyStateComputer),
        10, // max pruned blocks in mem
        Arc::new(SimulatedTimeService::new()),
    ))
}

// helpers for safety rule initialization
fn make_initial_epoch_change_proof(signer: &ValidatorSigner) -> EpochChangeProof {
    let validator_info =
        ValidatorInfo::new_with_test_network_keys(signer.author(), signer.public_key(), 1);
    let validator_set = ValidatorSet::new(vec![validator_info]);
    let li = LedgerInfo::mock_genesis(Some(validator_set));
    let lis = LedgerInfoWithSignatures::new(li, BTreeMap::new());
    EpochChangeProof::new(vec![lis], false)
}

// TODO: MockStorage -> EmptyStorage
fn create_round_state() -> RoundState {
    let base_timeout = std::time::Duration::new(60, 0);
    let time_interval = Box::new(ExponentialTimeInterval::fixed(base_timeout));
    let (round_timeout_sender, _) = channel::new_test(1_024);
    let time_service = Arc::new(SimulatedTimeService::new());
    RoundState::new(time_interval, time_service, round_timeout_sender)
}

// Creates an RoundManager for fuzzing
fn create_node_for_fuzzing() -> RoundManager {
    // signer is re-used accross fuzzing runs
    let signer = FUZZING_SIGNER.clone();

    // TODO: remove
    let validator = ValidatorVerifier::new_single(signer.author(), signer.public_key());
    let validator_set = (&validator).into();

    // TODO: EmptyStorage
    let (initial_data, storage) = MockStorage::start_for_testing(validator_set);

    // TODO: remove
    let proof = make_initial_epoch_change_proof(&signer);
    let mut safety_rules = SafetyRules::new(test_utils::test_storage(&signer), false, false, false);
    safety_rules.initialize(&proof).unwrap();

    // TODO: mock channels
    let (network_reqs_tx, _network_reqs_rx) = diem_channel::new(QueueStyle::FIFO, 8, None);
    let (connection_reqs_tx, _) = diem_channel::new(QueueStyle::FIFO, 8, None);
    let network_sender = ConsensusNetworkSender::new(
        PeerManagerRequestSender::new(network_reqs_tx),
        ConnectionRequestSender::new(connection_reqs_tx),
    );
    let (self_sender, _self_receiver) = channel::new_test(8);

    let epoch_state = EpochState {
        epoch: 1,
        verifier: storage.get_validator_set().into(),
    };
    let network = NetworkSender::new(
        signer.author(),
        network_sender,
        self_sender,
        epoch_state.verifier.clone(),
    );

    // TODO: mock
    let block_store = build_empty_store(storage.clone(), initial_data);

    // TODO: remove
    let time_service = Arc::new(SimulatedTimeService::new());
    time_service.sleep(Duration::from_millis(1));

    // TODO: remove
    let proposal_generator = ProposalGenerator::new(
        signer.author(),
        block_store.clone(),
        Arc::new(MockTransactionManager::new(None)),
        time_service,
        1,
    );

    //
    let round_state = create_round_state();

    // TODO: have two different nodes, one for proposing, one for accepting a proposal
    let proposer_election = Box::new(RotatingProposer::new(vec![signer.author()], 1));

    // event processor
    RoundManager::new(
        epoch_state,
        Arc::clone(&block_store),
        round_state,
        proposer_election,
        proposal_generator,
        Arc::new(Mutex::new(MetricsSafetyRules::new(
            Box::new(safety_rules),
            storage.clone(),
        ))),
        network,
        Arc::new(MockTransactionManager::new(None)),
        storage,
        false,
        Arc::new(AtomicU64::new(0)),
        false,
        10,
    )
}

// This functions fuzzes a Proposal protobuffer (not a ConsensusMsg)
pub fn fuzz_proposal(data: &[u8]) {
    // create node
    let mut round_manager = create_node_for_fuzzing();

    let proposal: ProposalMsg = match bcs::from_bytes(data) {
        Ok(xx) => xx,
        Err(_) => {
            if cfg!(test) {
                panic!();
            }
            return;
        }
    };

    let proposal = match proposal.verify_well_formed() {
        Ok(_) => proposal,
        Err(e) => {
            println!("{:?}", e);
            if cfg!(test) {
                panic!();
            }
            return;
        }
    };

    block_on(async move {
        // TODO: make sure this obtains a vote when testing
        // TODO: make sure that if this obtains a vote, it's for round 1, etc.
        let _ = round_manager.process_proposal_msg(proposal).await;
    });
}

// This test is here so that the fuzzer can be maintained
#[test]
fn test_consensus_proposal_fuzzer() {
    // generate a proposal
    let proposal = generate_corpus_proposal();
    // successfully parse it
    fuzz_proposal(&proposal);
}

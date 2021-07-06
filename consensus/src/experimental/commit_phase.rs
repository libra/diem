// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    counters, metrics_safety_rules::MetricsSafetyRules, network_interface::ConsensusMsg,
    state_replication::StateComputer,
};
use channel::Receiver;
use consensus_types::{
    common::Author,
    executed_block::ExecutedBlock,
    experimental::{commit_decision::CommitDecision, commit_vote::CommitVote},
};
use core::sync::atomic::Ordering;
use diem_crypto::{ed25519::Ed25519Signature, HashValue};
use diem_infallible::Mutex;
use diem_logger::prelude::*;
use diem_metrics::monitor;
use diem_types::{
    account_address::AccountAddress,
    block_info::Round,
    ledger_info::{LedgerInfo, LedgerInfoWithSignatures},
    validator_verifier::ValidatorVerifier,
};
use executor_types::Error as ExecutionError;
use futures::{select, SinkExt, StreamExt};
use safety_rules::TSafetyRules;
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap, VecDeque},
    sync::{atomic::AtomicU64, Arc},
};
use crate::network::NetworkSender;

/*
┌───────────┬──────────────────────────────────────┐
│           │                                      │
│ Message   ├─────────────────┐                    │
│ Channels  │                 │                    │
│           │                 ▼                    ▼
└─▲──┬──────┘  ┌───────► Commit Vote       Commit Decision ◄────────────────┐
  │  │         │              │                    │                        │
  │  │         │              │ Add sig            │ Replace sig tree       │
  │  │         │              │                    │                        │
  │  │         │       ┌──────▼───────┐            │                        │
  │  │         │       │              │            │                        │
  │  │         │       │ Local Cache  │◄───────────┘                        │
  │  │         │       │ (HashMap)    │                                     │
  │  │         │       │              │                                     │ Send
  │  │         │       └──────────────┴──────────────┐                      │
  │  │         │ Send                                │                      │
  │  │         │       ┌──────────────┐              │                      │
  │  └────► Commit     │              │              │                      │
  │            │       │ Local Queue  │◄─────────┐   │                      │
  │            └──────►│              │          │   │                      │
  │         Enqueue    └──────────┬───┘          │   ▼                      │
  │                               │              │ Check if committable:    │
  └─────────────── Check Channels │              │    If so, commit and dequeue
                                  │              │
                                  └────────► Main Loop
                                                 ▲
                                                 │
                                             fn start

 Commit phase takes in the executed blocks from the execution
 phase and commit them. Specifically, commit phase signs a commit
 vote message containing the execution result and broadcast it.
 Upon collecting a quorum of agreeing votes to a execution result,
 the commit phase commits the blocks as well as broadcasts a commit
 decision message together with the quorum of signatures. The commit
 decision message helps the slower nodes to quickly catch up without
 having to collect the signatures.
 */

#[derive(Clone)]
struct PendingBlocks {
    vecblocks: Vec<ExecutedBlock>,
}

impl PendingBlocks {
    pub fn new(vecblocks: Vec<ExecutedBlock>) -> Self {
        Self { vecblocks }
    }
}

#[derive(Debug)]
pub enum CommitPhaseMessage {
    CommitVote(Author, LedgerInfo, Ed25519Signature),
    CommitDecision(LedgerInfoWithSignatures),
}

pub struct CommitPhase {
    commit_channel_recv: Receiver<(Vec<ExecutedBlock>, LedgerInfoWithSignatures)>,
    execution_proxy: Arc<dyn StateComputer>,
    local_cache: HashMap<HashValue, LedgerInfoWithSignatures>,
    local_queue: VecDeque<PendingBlocks>,
    commit_msg_receiver: channel::Receiver<ConsensusMsg>,
    verifier: ValidatorVerifier,
    safety_rules: Arc<Mutex<MetricsSafetyRules>>,
    author: Author,
    committed_round: Round,
    back_pressure: Arc<AtomicU64>,
    network_sender: NetworkSender,
}

pub async fn commit(
    execution_proxy: &Arc<dyn StateComputer>,
    vecblock: &[ExecutedBlock],
    ledger_info: &LedgerInfoWithSignatures,
) -> Result<(), ExecutionError> {
    debug!("New commit: {} {}", vecblock.len(), ledger_info);
    // have to maintain the order.
    execution_proxy
        .commit(
            &vecblock
                .iter()
                .map(|eb| Arc::new(eb.clone()))
                .collect::<Vec<Arc<ExecutedBlock>>>(),
            ledger_info.clone(),
        )
        .await
}

macro_rules! report_err {
    ($result:expr, $error_string:literal) => {
        if let Err(err) = $result {
            counters::ERROR_COUNT.inc();
            error!(error = err.to_string(), $error_string,)
        }
    };
}

impl CommitPhase {
    pub fn new(
        commit_channel_recv: Receiver<(Vec<ExecutedBlock>, LedgerInfoWithSignatures)>,
        execution_proxy: Arc<dyn StateComputer>,
        commit_msg_receiver: channel::Receiver<ConsensusMsg>,
        verifier: ValidatorVerifier,
        safety_rules: Arc<Mutex<MetricsSafetyRules>>,
        author: Author,
        back_pressure: Arc<AtomicU64>,
        network_sender: NetworkSender,
    ) -> Self {
        Self {
            commit_channel_recv,
            execution_proxy,
            local_cache: HashMap::<HashValue, LedgerInfoWithSignatures>::new(),
            local_queue: VecDeque::<PendingBlocks>::new(),
            commit_msg_receiver,
            verifier,
            safety_rules,
            author,
            committed_round: 0,
            back_pressure,
            network_sender,
        }
    }

    /// Notified when receiving a commit vote message
    pub async fn process_commit_vote(&mut self, commit_vote: &CommitVote) -> anyhow::Result<()> {
        debug!("process_commit_vote {}", commit_vote);

        let li = commit_vote.ledger_info();

        if li.commit_info().round() < self.committed_round {
            return Ok(()); // we ignore the message
        }

        // verify the signature
        commit_vote.verify(&self.verifier)?;

        let executed_state_hash = li.commit_info().executed_state_id();

        // add the signature to local_cache
        match self.local_cache.entry(executed_state_hash) {
            Entry::Occupied(mut ledger_info_entry) => {
                let mut_ledger_info_entry = ledger_info_entry.get_mut();
                mut_ledger_info_entry
                    .add_signature(commit_vote.author(), commit_vote.signature().clone());
            }
            Entry::Vacant(_) => {
                let mut li_sig = LedgerInfoWithSignatures::new(
                    li.clone(),
                    BTreeMap::<AccountAddress, Ed25519Signature>::new(),
                );
                li_sig.add_signature(commit_vote.author(), commit_vote.signature().clone());
                self.local_cache.insert(executed_state_hash, li_sig);
                debug!(
                    "inserted local cache: key: {}, len: {}",
                    executed_state_hash,
                    self.local_cache.len()
                );
            }
        };

        Ok(())
    }

    pub async fn process_commit_decision(
        &mut self,
        commit_decision: CommitDecision,
    ) -> anyhow::Result<()> {
        debug!("process_commit_decision {}", commit_decision);

        let li = commit_decision.ledger_info();

        if li.ledger_info().commit_info().round() < self.committed_round {
            return Ok(()); // we ignore the message
        }

        commit_decision.verify(&self.verifier)?;

        let executed_state_hash = li.ledger_info().commit_info().executed_state_id();

        // TODO: optimization1: probe local_cache first to see if the existing already verifies,
        // TODO: otherwise we do not make changes.
        // TODO: optimization2: we can set a bit to indicate this tree of signatures are already verified,
        // TODO: we do not have to verify it again in the main loop.

        // replace the signature tree directly if it does not verify
        self.local_cache.insert(executed_state_hash, li.clone());

        Ok(())
    }

    pub async fn process_local_queue(&mut self) -> anyhow::Result<()> {
        let mut_local_queue = &mut self.local_queue;
        let mut_local_cache = &mut self.local_cache;
        while let Some(front) = mut_local_queue.front() {
            // TODO: what is vecblocks is empty?
            let front_executed_state_hash = front
                .vecblocks
                .last()
                .unwrap()
                .block_info()
                .executed_state_id();
            match mut_local_cache.entry(front_executed_state_hash) {
                Entry::Occupied(ledger_info_occupied_entry) => {
                    // cancel out an item from local_cache and an item from local_queue
                    debug!(
                        "Found {}, checking voting power: {}",
                        front_executed_state_hash,
                        ledger_info_occupied_entry.get()
                    );
                    if ledger_info_occupied_entry
                        .get()
                        .verify_signatures(&self.verifier)
                        .is_ok()
                    {
                        commit(
                            &self.execution_proxy,
                            &front.vecblocks,
                            ledger_info_occupied_entry.get(),
                        )
                        .await
                        .expect("Failed to commit the executed blocks.");

                        let commit_round = front.vecblocks.last().unwrap().block_info().round();
                        self.back_pressure.store(commit_round, Ordering::SeqCst);

                        debug!("Commit End");

                        assert!(
                            self.committed_round
                                < front.vecblocks.last().unwrap().block_info().round()
                        );
                        self.committed_round = front.vecblocks.last().unwrap().block_info().round();

                        let mut commit_sender = self.network_sender.clone();
                        let ledger_info_clone = ledger_info_occupied_entry.get().clone();
                        tokio::spawn(async move {
                            commit_sender.broadcast(ConsensusMsg::CommitDecisionMsg(Box::new(
                                CommitDecision::new(ledger_info_clone),
                            ))).await;
                        });

                        ledger_info_occupied_entry.remove_entry();
                        mut_local_queue.pop_front();
                    } else {
                        break;
                    }
                }
                Entry::Vacant(_) => {
                    debug!("Not found in the cache: {}", front_executed_state_hash);
                    break;
                }
            }
        }
        Ok(())
    }

    pub async fn process_executed_blocks(
        &mut self,
        vecblock: Vec<ExecutedBlock>,
        ledger_info: LedgerInfoWithSignatures,
    ) -> anyhow::Result<()> {
        debug!(
            "process_executed_blocks: vecblock {} ledger_info {}",
            vecblock.len(),
            ledger_info
        );
        let new_ledger_info = LedgerInfo::new(
            vecblock.last().unwrap().block_info(),
            ledger_info.ledger_info().consensus_data_hash(),
        );
        debug!("build new ledger info: {}", new_ledger_info);

        let signature = self
            .safety_rules
            .lock()
            .sign_commit_vote(ledger_info, new_ledger_info.clone())?;

        debug!("signed");
        // if fails, it needs to resend, otherwise the liveness might compromise.

        let mut commit_sender = self.network_sender.clone();
        let author = self.author;
        let msg = ConsensusMsg::CommitVoteMsg(Box::new(CommitVote::new_with_signature(
            author,
            new_ledger_info,
            signature,
        )));

        debug!("sent commit vote");
        // note that this message will also reach the node itself
        self.local_queue.push_back(PendingBlocks::new(vecblock));

        tokio::spawn(async move {
            commit_sender.broadcast(msg).await;
        });

        Ok(())
    }

    pub async fn start(mut self) {
        loop {
            if self.local_queue.is_empty() {
                select! {
                    (vecblock, ledger_info) = self.commit_channel_recv.select_next_some() => {
                        report_err!(self.process_executed_blocks(vecblock, ledger_info).await, "Error in processing executed blocks");
                    }
                    msg = self.commit_msg_receiver.select_next_some() => {
                        match msg {
                            ConsensusMsg::CommitVoteMsg(request) => {
                                monitor!(
                                    "process_commit_vote",
                                    report_err!(self.process_commit_vote(&*request).await, "Error in processing commit vote.")
                                );
                            }
                            ConsensusMsg::CommitDecisionMsg(request) => {
                                monitor!(
                                    "process_commit_decision",
                                    report_err!(self.process_commit_decision(*request).await, "Error in processing commit decision.")
                                );
                            }
                            _ => {}
                        };
                    }
                    complete => break,
                }
            } else {
                select! {
                    msg = self.commit_msg_receiver.select_next_some() => {
                        match msg {
                            ConsensusMsg::CommitVoteMsg(request) => {
                                monitor!(
                                    "process_commit_vote",
                                    report_err!(self.process_commit_vote(&*request).await, "Error in processing commit vote.")
                                );
                            }
                            ConsensusMsg::CommitDecisionMsg(request) => {
                                monitor!(
                                    "process_commit_decision",
                                    report_err!(self.process_commit_decision(*request).await, "Error in processing commit decision.")
                                );
                            }
                            _ => {}
                        };
                    }
                    complete => break,
                }
            }
            report_err!(
                self.process_local_queue().await,
                "Error in processing local queue"
            );
        }
    }
}

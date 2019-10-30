// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use consensus_types::{
    block::Block, executed_block::ExecutedBlock, quorum_cert::QuorumCert,
    timeout_certificate::TimeoutCertificate,
};
use libra_crypto::HashValue;
use std::sync::Arc;

mod block_store;
mod block_tree;
mod pending_votes;

pub use block_store::{sync_manager::BlockRetriever, BlockStore};

/// Result of the vote processing. The failure case (Verification error) is returned
/// as the Error part of the result.
#[derive(Debug, PartialEq)]
pub enum VoteReceptionResult {
    /// The vote has been added but QC has not been formed yet. Return the amount of voting power
    /// the given (proposal, execution) pair.
    VoteAdded(u64),
    /// The very same vote message has been processed in past.
    DuplicateVote,
    /// The very same author has already voted for another proposal in this round (equivocation).
    EquivocateVote,
    /// This block has been already certified.
    OldQuorumCertificate(Arc<QuorumCert>),
    /// This block has just been certified after adding the vote.
    NewQuorumCertificate(Arc<QuorumCert>),
    /// The vote completes a new TimeoutCertificate
    NewTimeoutCertificate(Arc<TimeoutCertificate>),
}

pub trait BlockReader: Send + Sync {
    type Payload;

    /// Check if a block with the block_id exist in the BlockTree.
    fn block_exists(&self, block_id: HashValue) -> bool;

    /// Try to get a block with the block_id, return an Arc of it if found.
    fn get_block(&self, block_id: HashValue) -> Option<Arc<ExecutedBlock<Self::Payload>>>;

    /// Get the current root block of the BlockTree.
    fn root(&self) -> Arc<ExecutedBlock<Self::Payload>>;

    fn get_quorum_cert_for_block(&self, block_id: HashValue) -> Option<Arc<QuorumCert>>;

    /// Returns all the blocks between the root and the given block, including the given block
    /// but excluding the root.
    /// In case a given block is not the successor of the root, return None.
    /// For example if a tree is b0 <- b1 <- b2 <- b3, then
    /// path_from_root(b2) -> Some([b2, b1])
    /// path_from_root(b0) -> Some([])
    /// path_from_root(a) -> None
    fn path_from_root(&self, block_id: HashValue)
        -> Option<Vec<Arc<ExecutedBlock<Self::Payload>>>>;

    /// Return the certified block with the highest round.
    fn highest_certified_block(&self) -> Arc<ExecutedBlock<Self::Payload>>;

    /// Return the quorum certificate with the highest round
    fn highest_quorum_cert(&self) -> Arc<QuorumCert>;

    /// Return the quorum certificate that carries ledger info with the highest round
    fn highest_ledger_info(&self) -> Arc<QuorumCert>;

    /// Return the highest timeout certificate if available.
    fn highest_timeout_cert(&self) -> Option<Arc<TimeoutCertificate>>;

    /// Retrieve the chain of ancestors starting with the given `block_id`.
    /// Return the max possible chain up to `num_blocks` (including the starting block).
    /// The returned vector is empty if the given block id is not found.
    /// The vector is ordered by round in descending order (starts with the `block_id`).
    fn get_ancestors(&self, block_id: HashValue, num_blocks: u64) -> Vec<Block<Self::Payload>>;

    /// Retrieve the chain of descendants of a committed block.
    /// The chain ends with the given block id, and includes the LedgerInfo certifying the commit of
    /// the target block (max size of chain is constrained by the max message size constraints).
    /// In case no such chain can be found, an empty vector is returned.
    fn get_descendants_for_committed_id(&self, block_id: HashValue) -> Vec<Block<Self::Payload>>;
}

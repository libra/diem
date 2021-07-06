// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{test_utils, test_utils::make_timeout_cert, Error, SafetyRules, TSafetyRules};
use consensus_types::{
    block::block_test_utils::random_payload,
    common::Round,
    quorum_cert::QuorumCert,
    timeout::Timeout,
    timeout_2chain::{TwoChainTimeout, TwoChainTimeoutCertificate},
    vote_proposal::MaybeSignedVoteProposal,
};
use diem_crypto::{
    ed25519::{Ed25519PrivateKey, Ed25519Signature},
    hash::{CryptoHash, HashValue, ACCUMULATOR_PLACEHOLDER_HASH},
};
use diem_global_constants::CONSENSUS_KEY;
use diem_secure_storage::CryptoStorage;
use diem_types::{
    account_address::AccountAddress,
    block_info::BlockInfo,
    epoch_state::EpochState,
    ledger_info::{LedgerInfo, LedgerInfoWithSignatures},
    validator_signer::ValidatorSigner,
    validator_verifier::ValidatorVerifier,
};
use std::collections::BTreeMap;

type Proof = test_utils::Proof;

fn make_proposal_with_qc_and_proof(
    round: Round,
    proof: Proof,
    qc: QuorumCert,
    signer: &ValidatorSigner,
    exec_key: Option<&Ed25519PrivateKey>,
) -> MaybeSignedVoteProposal {
    test_utils::make_proposal_with_qc_and_proof(vec![], round, proof, qc, signer, exec_key)
}

fn make_proposal_with_parent(
    round: Round,
    parent: &MaybeSignedVoteProposal,
    committed: Option<&MaybeSignedVoteProposal>,
    signer: &ValidatorSigner,
    exec_key: Option<&Ed25519PrivateKey>,
) -> MaybeSignedVoteProposal {
    test_utils::make_proposal_with_parent(vec![], round, parent, committed, signer, exec_key)
}

pub type Callback = Box<
    dyn Fn() -> (
        Box<dyn TSafetyRules + Send + Sync>,
        ValidatorSigner,
        Option<Ed25519PrivateKey>,
    ),
>;

pub fn run_test_suite(safety_rules: &Callback) {
    test_bad_execution_output(safety_rules);
    test_commit_rule_consecutive_rounds(safety_rules);
    test_end_to_end(safety_rules);
    test_initialize(safety_rules);
    test_preferred_block_rule(safety_rules);
    test_sign_timeout(safety_rules);
    test_voting(safety_rules);
    test_voting_potential_commit_id(safety_rules);
    test_voting_bad_epoch(safety_rules);
    test_sign_old_proposal(safety_rules);
    test_sign_proposal_with_bad_signer(safety_rules);
    test_sign_proposal_with_invalid_qc(safety_rules);
    test_sign_proposal_with_early_preferred_round(safety_rules);
    test_uninitialized_signer(safety_rules);
    test_reconcile_key(safety_rules);
    test_validator_not_in_set(safety_rules);
    test_key_not_in_store(safety_rules);
    test_2chain_rules(safety_rules);
    test_2chain_timeout(safety_rules);
    test_sign_commit_vote(safety_rules);
}

fn test_bad_execution_output(safety_rules: &Callback) {
    // build a tree of the following form:
    //                 _____
    //                /     \
    // genesis---a1--a2--a3  evil_a3
    //
    // evil_a3 attempts to append to a1 but fails append only check
    // a3 works as it properly extends a2
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 2, &a1, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 3, &a2, None, &signer, key.as_ref());

    safety_rules.initialize(&proof).unwrap();
    let a1_output = a1
        .accumulator_extension_proof()
        .verify(
            a1.block()
                .quorum_cert()
                .certified_block()
                .executed_state_id(),
        )
        .unwrap();

    let evil_proof = Proof::new(
        a1_output.frozen_subtree_roots().clone(),
        a1_output.num_leaves(),
        vec![Timeout::new(0, a3.block().round()).hash()],
    );

    let evil_a3 = make_proposal_with_qc_and_proof(
        round,
        evil_proof,
        a3.block().quorum_cert().clone(),
        &signer,
        key.as_ref(),
    );

    let evil_a3_block = safety_rules.construct_and_sign_vote(&evil_a3);
    assert!(evil_a3_block.is_err());

    let a3_block = safety_rules.construct_and_sign_vote(&a3);
    a3_block.unwrap();
}

fn test_commit_rule_consecutive_rounds(safety_rules: &Callback) {
    // build a tree of the following form:
    //             ___________
    //            /           \
    // genesis---a1  b1---b2   a2---a3---a4
    //         \_____/
    //
    // a1 cannot be committed after a3 gathers QC because a1 and a2 are not consecutive
    // a2 can be committed after a4 gathers QC
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    let a1 =
        test_utils::make_proposal_with_qc(round + 1, genesis_qc.clone(), &signer, key.as_ref());
    let b1 = test_utils::make_proposal_with_qc(round + 2, genesis_qc, &signer, key.as_ref());
    let b2 = make_proposal_with_parent(round + 3, &b1, None, &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 4, &a1, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 5, &a2, None, &signer, key.as_ref());
    let a4 = make_proposal_with_parent(round + 6, &a3, Some(&a2), &signer, key.as_ref());

    safety_rules.initialize(&proof).unwrap();
    safety_rules.construct_and_sign_vote(&a1).unwrap();
    safety_rules.construct_and_sign_vote(&b1).unwrap();
    safety_rules.construct_and_sign_vote(&b2).unwrap();
    safety_rules.construct_and_sign_vote(&a2).unwrap();
    safety_rules.construct_and_sign_vote(&a3).unwrap();
    safety_rules.construct_and_sign_vote(&a4).unwrap();
}

fn test_end_to_end(safety_rules: &Callback) {
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    let data = random_payload(2048);

    let p0 =
        test_utils::make_proposal_with_qc(round + 1, genesis_qc.clone(), &signer, key.as_ref());
    let p1 = test_utils::make_proposal_with_parent(
        data.clone(),
        round + 2,
        &p0,
        None,
        &signer,
        key.as_ref(),
    );
    let p2 = test_utils::make_proposal_with_parent(
        data.clone(),
        round + 3,
        &p1,
        None,
        &signer,
        key.as_ref(),
    );
    let p3 = test_utils::make_proposal_with_parent(
        data,
        round + 4,
        &p2,
        Some(&p0),
        &signer,
        key.as_ref(),
    );

    let state = safety_rules.consensus_state().unwrap();
    assert_eq!(
        state.last_voted_round(),
        genesis_qc.certified_block().round()
    );
    assert_eq!(
        state.preferred_round(),
        genesis_qc.certified_block().round()
    );

    safety_rules.initialize(&proof).unwrap();
    safety_rules.construct_and_sign_vote(&p0).unwrap();
    safety_rules.construct_and_sign_vote(&p1).unwrap();
    safety_rules.construct_and_sign_vote(&p2).unwrap();
    safety_rules.construct_and_sign_vote(&p3).unwrap();

    let state = safety_rules.consensus_state().unwrap();
    assert_eq!(state.last_voted_round(), round + 4);
    assert_eq!(state.preferred_round(), round + 2);
}

/// Initialize from scratch, ensure that SafetyRules can properly initialize from a Waypoint and
/// that it rejects invalid LedgerInfos or those that do not match.
fn test_initialize(safety_rules: &Callback) {
    let (mut safety_rules, signer, _key) = safety_rules();

    let state = safety_rules.consensus_state().unwrap();
    assert_eq!(state.last_voted_round(), 0);
    assert_eq!(state.preferred_round(), 0);
    assert_eq!(state.epoch(), 1);

    let (proof, _genesis_qc) = test_utils::make_genesis(&signer);
    safety_rules.initialize(&proof).unwrap();

    let signer1 = ValidatorSigner::from_int(1);
    let (bad_proof, _bad_genesis_qc) = test_utils::make_genesis(&signer1);

    match safety_rules.initialize(&bad_proof) {
        Err(Error::InvalidEpochChangeProof(_)) => (),
        _ => panic!("Unexpected output"),
    };
}

fn test_preferred_block_rule(safety_rules: &Callback) {
    // Preferred block is the highest 2-chain head.
    //
    // build a tree of the following form:
    //             _____    _____
    //            /     \  /     \
    // genesis---a1  b1  b2  a2  b3  a3---a4
    //         \_____/ \_____/ \_____/
    //
    // PB should change from genesis to b1 and then a2.
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let genesis_round = genesis_qc.certified_block().round();
    let round = genesis_round;

    let a1 =
        test_utils::make_proposal_with_qc(round + 1, genesis_qc.clone(), &signer, key.as_ref());
    let b1 = test_utils::make_proposal_with_qc(round + 2, genesis_qc, &signer, key.as_ref());
    let b2 = make_proposal_with_parent(round + 3, &a1, None, &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 4, &b1, None, &signer, key.as_ref());
    let b3 = make_proposal_with_parent(round + 5, &b2, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 6, &a2, None, &signer, key.as_ref());
    let a4 = make_proposal_with_parent(round + 7, &a3, None, &signer, key.as_ref());

    safety_rules.initialize(&proof).unwrap();

    safety_rules.construct_and_sign_vote(&a1).unwrap();
    assert_eq!(
        safety_rules.consensus_state().unwrap().preferred_round(),
        genesis_round
    );

    safety_rules.construct_and_sign_vote(&b1).unwrap();
    assert_eq!(
        safety_rules.consensus_state().unwrap().preferred_round(),
        genesis_round
    );

    safety_rules.construct_and_sign_vote(&a2).unwrap();
    assert_eq!(
        safety_rules.consensus_state().unwrap().preferred_round(),
        genesis_round
    );

    safety_rules.construct_and_sign_vote(&b2).unwrap_err();
    assert_eq!(
        safety_rules.consensus_state().unwrap().preferred_round(),
        genesis_round
    );

    safety_rules.construct_and_sign_vote(&a3).unwrap();
    assert_eq!(
        safety_rules.consensus_state().unwrap().preferred_round(),
        b1.block().round()
    );

    safety_rules.construct_and_sign_vote(&b3).unwrap_err();
    assert_eq!(
        safety_rules.consensus_state().unwrap().preferred_round(),
        b1.block().round()
    );

    safety_rules.construct_and_sign_vote(&a4).unwrap();
    assert_eq!(
        safety_rules.consensus_state().unwrap().preferred_round(),
        a2.block().round()
    );
}

/// Verify first that we can successfully sign a timeout on the correct conditions, then ensure
/// that poorly set last_voted_rounds both historical and in the future fail as well as
/// synchronization issues on preferred round are correct. Effectivelly ensure that equivocation is
/// impossible for signing timeouts.
fn test_sign_timeout(safety_rules: &Callback) {
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();
    let epoch = genesis_qc.certified_block().epoch();

    let p0 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    let p1 = make_proposal_with_parent(round + 2, &p0, None, &signer, key.as_ref());
    let p2 = make_proposal_with_parent(round + 3, &p1, None, &signer, key.as_ref());
    let p3 = make_proposal_with_parent(round + 4, &p2, None, &signer, key.as_ref());
    let p4 = make_proposal_with_parent(round + 5, &p3, None, &signer, key.as_ref());

    safety_rules.initialize(&proof).unwrap();
    safety_rules.construct_and_sign_vote(&p0).unwrap();

    // Verify multiple signings are the same
    let timeout = Timeout::new(epoch, p0.block().round());
    let sign1 = safety_rules.sign_timeout(&timeout).unwrap();
    let sign2 = safety_rules.sign_timeout(&timeout).unwrap();
    assert_eq!(sign1, sign2);

    // Verify can sign last_voted_round + 1
    let timeout_plus_1 = Timeout::new(timeout.epoch(), timeout.round() + 1);
    safety_rules.sign_timeout(&timeout_plus_1).unwrap();

    // Verify cannot sign round older rounds now
    let actual_err = safety_rules.sign_timeout(&timeout).unwrap_err();
    let expected_err = Error::IncorrectLastVotedRound(timeout.round(), timeout.round() + 1);
    assert_eq!(actual_err, expected_err);

    // Verify cannot sign last_voted_round < vote < preferred_round
    safety_rules.construct_and_sign_vote(&p4).unwrap();
    let preferred_round = p4.block().quorum_cert().parent_block().round();
    let ptimeout = Timeout::new(timeout.epoch(), preferred_round - 1);
    let actual_err = safety_rules.sign_timeout(&ptimeout).unwrap_err();
    let expected_err = Error::IncorrectPreferredRound(ptimeout.round(), preferred_round);
    assert_eq!(actual_err, expected_err);

    // Verify cannot sign for different epoch
    let etimeout = Timeout::new(timeout.epoch() + 1, round + 1);
    let actual_err = safety_rules.sign_timeout(&etimeout).unwrap_err();
    let expected_err = Error::IncorrectEpoch(etimeout.epoch(), timeout.epoch());
    assert_eq!(actual_err, expected_err);
}

fn test_voting(safety_rules: &Callback) {
    // build a tree of the following form:
    //             _____    __________
    //            /     \  /          \
    // genesis---a1  b1  b2  a2---a3  b3  a4  b4
    //         \_____/ \_____/     \______/   /
    //                    \__________________/
    //
    //
    // We'll introduce the votes in the following order:
    // a1 (ok), potential_commit is None
    // b1 (ok), potential commit is None
    // a2 (ok), potential_commit is None
    // b2 (old proposal)
    // a3 (ok), potential commit is None
    // b3 (ok), potential commit is None
    // a4 (ok), potential commit is None
    // a4 (old proposal)
    // b4 (round lower then round of pb. PB: a2, parent(b4)=b2)
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    let a1 =
        test_utils::make_proposal_with_qc(round + 1, genesis_qc.clone(), &signer, key.as_ref());
    let b1 = test_utils::make_proposal_with_qc(round + 2, genesis_qc, &signer, key.as_ref());
    let b2 = make_proposal_with_parent(round + 3, &a1, None, &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 4, &b1, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 5, &a2, None, &signer, key.as_ref());
    let b3 = make_proposal_with_parent(round + 6, &b2, None, &signer, key.as_ref());
    let a4 = make_proposal_with_parent(round + 7, &a3, None, &signer, key.as_ref());
    let a4_prime = make_proposal_with_parent(round + 7, &a2, None, &signer, key.as_ref());
    let b4 = make_proposal_with_parent(round + 8, &b2, None, &signer, key.as_ref());

    safety_rules.initialize(&proof).unwrap();

    let mut vote = safety_rules.construct_and_sign_vote(&a1).unwrap();
    assert_eq!(vote.ledger_info().consensus_block_id(), HashValue::zero());

    vote = safety_rules.construct_and_sign_vote(&b1).unwrap();
    assert_eq!(vote.ledger_info().consensus_block_id(), HashValue::zero());

    vote = safety_rules.construct_and_sign_vote(&a2).unwrap();
    assert_eq!(vote.ledger_info().consensus_block_id(), HashValue::zero());

    assert_eq!(
        safety_rules.construct_and_sign_vote(&b2),
        Err(Error::IncorrectLastVotedRound(3, 4))
    );

    vote = safety_rules.construct_and_sign_vote(&a3).unwrap();
    assert_eq!(vote.ledger_info().consensus_block_id(), HashValue::zero());

    vote = safety_rules.construct_and_sign_vote(&b3).unwrap();
    assert_eq!(vote.ledger_info().consensus_block_id(), HashValue::zero());

    vote = safety_rules.construct_and_sign_vote(&a4).unwrap();
    assert_eq!(vote.ledger_info().consensus_block_id(), HashValue::zero());

    assert_eq!(
        safety_rules.construct_and_sign_vote(&a3),
        Err(Error::IncorrectLastVotedRound(5, 7))
    );

    // return the last vote for the same round
    assert_eq!(
        safety_rules.construct_and_sign_vote(&a4_prime),
        Ok(vote.clone())
    );
    assert_eq!(safety_rules.construct_and_sign_vote(&a4), Ok(vote));

    assert_eq!(
        safety_rules.construct_and_sign_vote(&b4),
        Err(Error::IncorrectPreferredRound(3, 4))
    );
}

fn test_voting_bad_epoch(safety_rules: &Callback) {
    // Test to verify epoch is the same between parent and proposed in a vote proposal
    // genesis--a1 -> a2 fails due to jumping to a different epoch
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    let a2 = test_utils::make_proposal_with_parent_and_overrides(
        vec![],
        round + 3,
        &a1,
        None,
        &signer,
        Some(21),
        None,
        key.as_ref(),
    );
    safety_rules.initialize(&proof).unwrap();
    safety_rules.construct_and_sign_vote(&a1).unwrap();

    assert_eq!(
        safety_rules.construct_and_sign_vote(&a2),
        Err(Error::IncorrectEpoch(21, 1))
    );
}

fn test_voting_potential_commit_id(safety_rules: &Callback) {
    // Test the potential ledger info that we're going to use in case of voting
    // build a tree of the following form:
    //            _____
    //           /     \
    // genesis--a1  b1  a2--a3--a4--a5
    //        \_____/
    //
    // All the votes before a4 cannot produce any potential commits.
    // A potential commit for proposal a4 is a2, a potential commit for proposal a5 is a3.
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    let a1 =
        test_utils::make_proposal_with_qc(round + 1, genesis_qc.clone(), &signer, key.as_ref());
    let b1 = test_utils::make_proposal_with_qc(round + 2, genesis_qc, &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 3, &a1, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 4, &a2, None, &signer, key.as_ref());
    let a4 = make_proposal_with_parent(round + 5, &a3, Some(&a2), &signer, key.as_ref());
    let a5 = make_proposal_with_parent(round + 6, &a4, Some(&a3), &signer, key.as_ref());

    safety_rules.initialize(&proof).unwrap();

    for b in &[&a1, &b1, &a2, &a3] {
        let vote = safety_rules.construct_and_sign_vote(b).unwrap();
        assert_eq!(vote.ledger_info().consensus_block_id(), HashValue::zero());
    }

    assert_eq!(
        safety_rules
            .construct_and_sign_vote(&a4)
            .unwrap()
            .ledger_info()
            .consensus_block_id(),
        a2.block().id(),
    );

    assert_eq!(
        safety_rules
            .construct_and_sign_vote(&a5)
            .unwrap()
            .ledger_info()
            .consensus_block_id(),
        a3.block().id(),
    );
}

fn test_sign_old_proposal(safety_rules: &Callback) {
    // Test to sign a proposal which makes no progress, compared with last voted round

    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();
    safety_rules.initialize(&proof).unwrap();

    let a1 = test_utils::make_proposal_with_qc(round, genesis_qc, &signer, key.as_ref());
    let err = safety_rules
        .sign_proposal(a1.block().block_data())
        .unwrap_err();
    assert!(matches!(err, Error::InvalidProposal(_)));
}

fn test_sign_proposal_with_bad_signer(safety_rules: &Callback) {
    // Test to sign a proposal signed by an unrecognizable signer

    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();
    safety_rules.initialize(&proof).unwrap();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    safety_rules.sign_proposal(a1.block().block_data()).unwrap();

    let bad_signer = ValidatorSigner::from_int(0xef);
    let a2 = make_proposal_with_parent(round + 2, &a1, None, &bad_signer, key.as_ref());
    let err = safety_rules
        .sign_proposal(a2.block().block_data())
        .unwrap_err();
    assert_eq!(
        err,
        Error::InvalidProposal("Proposal author is not validator signer!".into())
    );
}

fn test_sign_proposal_with_invalid_qc(safety_rules: &Callback) {
    // Test to sign a proposal with an invalid qc inherited from proposal a2, which
    // is signed by a bad_signer.

    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();
    safety_rules.initialize(&proof).unwrap();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    safety_rules.sign_proposal(a1.block().block_data()).unwrap();

    let bad_signer = ValidatorSigner::from_int(0xef);
    let a2 = make_proposal_with_parent(round + 2, &a1, Some(&a1), &bad_signer, key.as_ref());
    let a3 = test_utils::make_proposal_with_qc(
        round + 3,
        a2.block().quorum_cert().clone(),
        &signer,
        key.as_ref(),
    );
    let err = safety_rules
        .sign_proposal(a3.block().block_data())
        .unwrap_err();
    assert_eq!(
        err,
        Error::InvalidQuorumCertificate("Fail to verify QuorumCert".into())
    );
}

fn test_sign_proposal_with_early_preferred_round(safety_rules: &Callback) {
    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();
    safety_rules.initialize(&proof).unwrap();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    safety_rules.sign_proposal(a1.block().block_data()).unwrap();

    // Update preferred round with a few legal proposals
    let a2 = make_proposal_with_parent(round + 2, &a1, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 3, &a2, None, &signer, key.as_ref());
    let a4 = make_proposal_with_parent(round + 4, &a3, Some(&a2), &signer, key.as_ref());
    safety_rules.construct_and_sign_vote(&a2).unwrap();
    safety_rules.construct_and_sign_vote(&a3).unwrap();
    safety_rules.construct_and_sign_vote(&a4).unwrap();

    let a5 = make_proposal_with_qc_and_proof(
        round + 5,
        test_utils::empty_proof(),
        a1.block().quorum_cert().clone(),
        &signer,
        key.as_ref(),
    );
    let err = safety_rules
        .sign_proposal(a5.block().block_data())
        .unwrap_err();
    assert_eq!(err, Error::IncorrectPreferredRound(0, 2));
}

fn test_uninitialized_signer(safety_rules: &Callback) {
    // Testing for an uninitialized Option<ValidatorSigner>

    let (mut safety_rules, signer, key) = safety_rules();

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    let err = safety_rules.construct_and_sign_vote(&a1).unwrap_err();
    assert_eq!(err, Error::NotInitialized("validator_signer".into()));
    let err = safety_rules
        .sign_proposal(a1.block().block_data())
        .unwrap_err();
    assert_eq!(err, Error::NotInitialized("validator_signer".into()));

    safety_rules.initialize(&proof).unwrap();
    safety_rules.construct_and_sign_vote(&a1).unwrap();
}

fn test_validator_not_in_set(safety_rules: &Callback) {
    // Testing for a validator missing from the validator set
    // It does so by updating the safey rule to an epoch state, which does not contain the
    // current validator and check the consensus state

    let (mut safety_rules, signer, key) = safety_rules();

    let (mut proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    safety_rules.initialize(&proof).unwrap();

    // validator_signer is set during initialization
    let state = safety_rules.consensus_state().unwrap();
    assert_eq!(state.in_validator_set(), true);

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());

    // remove the validator_signer in next epoch
    let mut next_epoch_state = EpochState::empty();
    next_epoch_state.epoch = 1;
    let rand_signer = ValidatorSigner::random([0xfu8; 32]);
    next_epoch_state.verifier =
        ValidatorVerifier::new_single(rand_signer.author(), rand_signer.public_key());
    let a2 = test_utils::make_proposal_with_parent_and_overrides(
        vec![],
        round + 2,
        &a1,
        Some(&a1),
        &signer,
        Some(1),
        Some(next_epoch_state),
        key.as_ref(),
    );
    proof
        .ledger_info_with_sigs
        .push(a2.block().quorum_cert().ledger_info().clone());
    assert!(matches!(
        safety_rules.initialize(&proof),
        Err(Error::ValidatorNotInSet(_))
    ));

    let state = safety_rules.consensus_state().unwrap();
    assert_eq!(state.in_validator_set(), false);
}

fn test_reconcile_key(_safety_rules: &Callback) {
    // Test to verify desired consensus key can be retrieved according to validator set.
    // It does so by updating the safey rule to a desired epoch state, reconciling old signer key
    // with the new one. Later when it tries to verify the QC signed by the old signer key, safety
    // rules fails the check.

    // Initialize the storage with two versions of signer keys
    let signer = ValidatorSigner::from_int(0);
    let mut storage = test_utils::test_storage(&signer);

    let new_pub_key = storage.internal_store().rotate_key(CONSENSUS_KEY).unwrap();
    let mut safety_rules = Box::new(SafetyRules::new(storage, false, false, false));

    let (mut proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    safety_rules.initialize(&proof).unwrap();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, None);
    safety_rules.construct_and_sign_vote(&a1).unwrap();

    // Update validator epoch state, reconciling the old key with the new pub key
    let mut next_epoch_state = EpochState::empty();
    next_epoch_state.epoch = 2;
    next_epoch_state.verifier = ValidatorVerifier::new_single(signer.author(), new_pub_key);
    let a2 = test_utils::make_proposal_with_parent_and_overrides(
        vec![],
        round + 2,
        &a1,
        Some(&a1),
        &signer,
        Some(1),
        Some(next_epoch_state),
        None,
    );
    proof
        .ledger_info_with_sigs
        .push(a2.block().quorum_cert().ledger_info().clone());
    safety_rules.initialize(&proof).unwrap();

    // Verification fails for proposal signed by the outdated key
    let outdated_signer = &signer;
    let a3 = test_utils::make_proposal_with_parent_and_overrides(
        vec![],
        round + 3,
        &a2,
        Some(&a2),
        outdated_signer,
        Some(2),
        None,
        None,
    );
    let err = safety_rules.construct_and_sign_vote(&a3).unwrap_err();
    assert_eq!(
        err,
        Error::InvalidQuorumCertificate("Fail to verify QuorumCert".into())
    );
}

// Tests for fetching a missing validator key from persistent storage.
fn test_key_not_in_store(safety_rules: &Callback) {
    let (mut safety_rules, signer, key) = safety_rules();
    let (mut proof, genesis_qc) = test_utils::make_genesis(&signer);
    let round = genesis_qc.certified_block().round();

    safety_rules.initialize(&proof).unwrap();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());

    // Update to an epoch where the validator fails to retrive the respective key
    // from persistent storage
    let mut next_epoch_state = EpochState::empty();
    next_epoch_state.epoch = 1;
    let rand_signer = ValidatorSigner::random([0xfu8; 32]);
    next_epoch_state.verifier =
        ValidatorVerifier::new_single(signer.author(), rand_signer.public_key());
    let a2 = test_utils::make_proposal_with_parent_and_overrides(
        vec![],
        round + 2,
        &a1,
        Some(&a1),
        &signer,
        Some(1),
        Some(next_epoch_state),
        key.as_ref(),
    );
    proof
        .ledger_info_with_sigs
        .push(a2.block().quorum_cert().ledger_info().clone());

    // Expected failure due to validator key not being found.
    safety_rules.initialize(&proof).unwrap_err();

    let state = safety_rules.consensus_state().unwrap();
    assert_eq!(state.in_validator_set(), false);
}

fn test_2chain_rules(constructor: &Callback) {
    // One chain round is the highest quorum cert round.
    //
    // build a tree of the following form:
    //             _____    _____   _________
    //            /     \  /     \ /         \
    // genesis---a1  b1  b2  a2  b3  a3---a4  b4 a5---a6
    //         \_____/ \_____/ \_____/ \_________/
    //
    let (mut safety_rules, signer, key) = constructor();
    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let genesis_round = genesis_qc.certified_block().round();
    let round = genesis_round;
    safety_rules.initialize(&proof).unwrap();
    let a1 =
        test_utils::make_proposal_with_qc(round + 1, genesis_qc.clone(), &signer, key.as_ref());
    let b1 = test_utils::make_proposal_with_qc(round + 2, genesis_qc, &signer, key.as_ref());
    let b2 = make_proposal_with_parent(round + 3, &a1, None, &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 4, &b1, None, &signer, key.as_ref());
    let b3 = make_proposal_with_parent(round + 5, &b2, None, &signer, key.as_ref());
    let b4 = make_proposal_with_parent(round + 6, &b3, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 6, &a2, None, &signer, key.as_ref());
    let a4 = make_proposal_with_parent(round + 7, &a3, None, &signer, key.as_ref());
    let a5 = make_proposal_with_parent(round + 8, &a3, None, &signer, key.as_ref());
    let a6 = make_proposal_with_parent(round + 9, &a5, None, &signer, key.as_ref());

    safety_rules.initialize(&proof).unwrap();

    let mut expect = |p, maybe_tc: Option<TwoChainTimeoutCertificate>, vote, commit| {
        let result = safety_rules.construct_and_sign_vote_two_chain(p, maybe_tc.as_ref());
        let qc = p.vote_proposal.block().quorum_cert();
        if vote {
            let vote = result.unwrap();
            let id = if commit {
                qc.certified_block().id()
            } else {
                HashValue::zero()
            };
            assert_eq!(vote.ledger_info().consensus_block_id(), id);
            assert!(
                safety_rules.consensus_state().unwrap().one_chain_round()
                    >= qc.certified_block().round()
            );
        } else {
            result.unwrap_err();
        }
    };
    // block == qc + 1, commit
    expect(&a1, None, true, true);
    // block != qc + 1 && block != tc + 1
    expect(
        &b1,
        Some(make_timeout_cert(
            3,
            b1.vote_proposal.block().quorum_cert(),
            &signer,
        )),
        false,
        false,
    );
    // block != qc + 1, no TC
    expect(&b2, None, false, false);
    // block = tc + 1, qc == tc.hqc
    expect(
        &a2,
        Some(make_timeout_cert(
            3,
            a2.vote_proposal.block().quorum_cert(),
            &signer,
        )),
        true,
        false,
    );
    // block = tc + 1, qc < tc.hqc
    expect(
        &b3,
        Some(make_timeout_cert(
            4,
            a3.vote_proposal.block().quorum_cert(),
            &signer,
        )),
        false,
        false,
    );
    // block != qc + 1, no TC
    expect(&a3, None, false, false);
    // block = qc + 1, with TC, commit
    expect(
        &a4,
        Some(make_timeout_cert(
            7,
            a3.vote_proposal.block().quorum_cert(),
            &signer,
        )),
        true,
        true,
    );
    // block = tc + 1, qc > tc.hqc
    expect(
        &a5,
        Some(make_timeout_cert(
            7,
            b4.vote_proposal.block().quorum_cert(),
            &signer,
        )),
        true,
        false,
    );
    // block = qc + 1, block != tc + 1 (tc is ignored)
    expect(
        &a6,
        Some(make_timeout_cert(
            7,
            b4.vote_proposal.block().quorum_cert(),
            &signer,
        )),
        true,
        true,
    );
}

fn test_2chain_timeout(constructor: &Callback) {
    let (mut safety_rules, signer, key) = constructor();
    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    let genesis_round = genesis_qc.certified_block().round();
    let round = genesis_round;
    safety_rules.initialize(&proof).unwrap();
    let a1 =
        test_utils::make_proposal_with_qc(round + 1, genesis_qc.clone(), &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 2, &a1, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 3, &a2, None, &signer, key.as_ref());

    safety_rules
        .sign_timeout_with_qc(&TwoChainTimeout::new(1, 1, genesis_qc.clone()), None)
        .unwrap();
    assert_eq!(
        safety_rules
            .sign_timeout_with_qc(&TwoChainTimeout::new(1, 2, genesis_qc.clone()), None)
            .unwrap_err(),
        Error::NotSafeToTimeout(2, 0, 0, 0),
    );

    assert_eq!(
        safety_rules
            .sign_timeout_with_qc(&TwoChainTimeout::new(2, 2, genesis_qc.clone()), None)
            .unwrap_err(),
        Error::IncorrectEpoch(2, 1)
    );
    safety_rules
        .sign_timeout_with_qc(
            &TwoChainTimeout::new(1, 2, genesis_qc.clone()),
            Some(make_timeout_cert(1, &genesis_qc, &signer)).as_ref(),
        )
        .unwrap();
    assert_eq!(
        safety_rules
            .sign_timeout_with_qc(&TwoChainTimeout::new(1, 1, genesis_qc.clone()), None)
            .unwrap_err(),
        Error::IncorrectLastVotedRound(1, 2)
    );
    // update one-chain to 2
    safety_rules
        .construct_and_sign_vote_two_chain(&a3, None)
        .unwrap();
    assert_eq!(
        safety_rules
            .sign_timeout_with_qc(
                &TwoChainTimeout::new(1, 4, a3.vote_proposal.block().quorum_cert().clone(),),
                Some(make_timeout_cert(2, &genesis_qc, &signer)).as_ref()
            )
            .unwrap_err(),
        Error::NotSafeToTimeout(4, 2, 2, 2)
    );
    assert_eq!(
        safety_rules
            .sign_timeout_with_qc(
                &TwoChainTimeout::new(1, 4, a2.vote_proposal.block().quorum_cert().clone(),),
                Some(make_timeout_cert(3, &genesis_qc, &signer)).as_ref()
            )
            .unwrap_err(),
        Error::NotSafeToTimeout(4, 1, 3, 2)
    );
}

/// Test that we can succesfully sign a valid commit vote
fn test_sign_commit_vote(constructor: &Callback) {
    // we construct a chain of proposals
    // genesis -- a1 -- a2 -- a3

    let (mut safety_rules, signer, key) = constructor();
    let (proof, genesis_qc) = test_utils::make_genesis(&signer);

    let round = genesis_qc.certified_block().round();
    safety_rules.initialize(&proof).unwrap();

    let a1 = test_utils::make_proposal_with_qc(round + 1, genesis_qc, &signer, key.as_ref());
    let a2 = make_proposal_with_parent(round + 2, &a1, None, &signer, key.as_ref());
    let a3 = make_proposal_with_parent(round + 3, &a2, Some(&a1), &signer, key.as_ref());

    // now we try to agree on a1's execution result
    let ledger_info_with_sigs = a3.block().quorum_cert().ledger_info();
    // make sure this is for a1
    assert!(ledger_info_with_sigs
        .ledger_info()
        .commit_info()
        .match_ordered_only(
            &a1.block()
                .gen_block_info(*ACCUMULATOR_PLACEHOLDER_HASH, 0, None,)
        ));

    assert!(safety_rules
        .sign_commit_vote(
            ledger_info_with_sigs.clone(),
            ledger_info_with_sigs.ledger_info().clone()
        )
        .is_ok());

    // check empty ledger info
    assert!(matches!(
        safety_rules
            .sign_commit_vote(
                a2.block().quorum_cert().ledger_info().clone(),
                a3.block().quorum_cert().ledger_info().ledger_info().clone()
            )
            .unwrap_err(),
        Error::InvalidOrderedLedgerInfo(_)
    ));

    // non-dummy blockinfo test
    assert!(matches!(
        safety_rules
            .sign_commit_vote(
                LedgerInfoWithSignatures::new(
                    LedgerInfo::new(
                        a1.block().gen_block_info(
                            *ACCUMULATOR_PLACEHOLDER_HASH,
                            100, // non-dummy value
                            None
                        ),
                        ledger_info_with_sigs.ledger_info().consensus_data_hash()
                    ),
                    BTreeMap::<AccountAddress, Ed25519Signature>::new()
                ),
                ledger_info_with_sigs.ledger_info().clone()
            )
            .unwrap_err(),
        Error::InvalidOrderedLedgerInfo(_)
    ));

    // empty signature test
    assert!(matches!(
        safety_rules
            .sign_commit_vote(
                LedgerInfoWithSignatures::new(
                    ledger_info_with_sigs.ledger_info().clone(),
                    BTreeMap::<AccountAddress, Ed25519Signature>::new()
                ),
                ledger_info_with_sigs.ledger_info().clone()
            )
            .unwrap_err(),
        Error::InvalidQuorumCertificate(_)
    ));

    // inconsistent ledger_info test
    let bad_ledger_info = LedgerInfo::new(
        BlockInfo::random(ledger_info_with_sigs.ledger_info().round()),
        ledger_info_with_sigs.ledger_info().consensus_data_hash(),
    );

    assert!(matches!(
        safety_rules
            .sign_commit_vote(ledger_info_with_sigs.clone(), bad_ledger_info,)
            .unwrap_err(),
        Error::InconsistentExecutionResult(_, _)
    ));
}

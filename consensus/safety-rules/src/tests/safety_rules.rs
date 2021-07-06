// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{test_utils, tests::suite, SafetyRules};
use diem_crypto::{ed25519::Ed25519PrivateKey, Uniform};
use diem_types::validator_signer::ValidatorSigner;

#[test]
fn test() {
    let boolean_values = [false, true];
    for verify_vote_proposal_signature in &boolean_values {
        for export_consensus_key in &boolean_values {
            suite::run_test_suite(&safety_rules(
                *verify_vote_proposal_signature,
                *export_consensus_key,
                false, // todo: test on both cases
            ));
        }
    }
}

fn safety_rules(
    verify_vote_proposal_signature: bool,
    export_consensus_key: bool,
    decoupled_execution: bool,
) -> suite::Callback {
    Box::new(move || {
        let signer = ValidatorSigner::from_int(0);
        let storage = test_utils::test_storage(&signer);
        let safety_rules = Box::new(SafetyRules::new(
            storage,
            verify_vote_proposal_signature,
            export_consensus_key,
            decoupled_execution,
        ));
        (
            safety_rules,
            signer,
            if verify_vote_proposal_signature {
                Some(Ed25519PrivateKey::generate_for_testing())
            } else {
                None
            },
        )
    })
}

// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{test_utils, tests::suite, SafetyRulesManager};
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
            ));
        }
    }
}

fn safety_rules(
    verify_vote_proposal_signature: bool,
    export_consensus_key: bool,
) -> suite::Callback {
    Box::new(move || {
        let signer = ValidatorSigner::from_int(0);
        let storage = test_utils::test_storage(&signer);
        let safety_rules_manager = SafetyRulesManager::new_serializer(
            storage,
            verify_vote_proposal_signature,
            export_consensus_key,
            false,
        );
        let safety_rules = safety_rules_manager.client();
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

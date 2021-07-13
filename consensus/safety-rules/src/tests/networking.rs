// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{test_utils, SafetyRulesManager};
use diem_types::validator_signer::ValidatorSigner;

#[test]
fn test_reconnect() {
    let signer = ValidatorSigner::from_int(0);
    let storage = test_utils::test_storage(&signer);
    // test value for network timeout, in milliseconds.
    let network_timeout = 5_000;
    let safety_rules_manager =
        SafetyRulesManager::new_thread(storage, false, false, network_timeout, false);

    // Verify that after a client has disconnected a new client will connect and resume operations
    let state0 = safety_rules_manager.client().consensus_state().unwrap();
    let state1 = safety_rules_manager.client().consensus_state().unwrap();
    assert_eq!(state0, state1);
}

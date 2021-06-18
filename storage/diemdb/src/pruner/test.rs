// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::{change_set::ChangeSet, state_store::StateStore, DiemDB};
use diem_crypto::HashValue;
use diem_temppath::TempPath;
use diem_types::{account_address::AccountAddress, account_state_blob::AccountStateBlob};
use std::collections::HashMap;

fn put_account_state_set(
    db: &DB,
    state_store: &StateStore,
    account_state_set: Vec<(AccountAddress, AccountStateBlob)>,
    version: Version,
) -> HashValue {
    let mut cs = ChangeSet::new();
    let root = state_store
        .put_account_state_sets(
            vec![account_state_set.into_iter().collect::<HashMap<_, _>>()],
            None,
            version,
            &mut cs,
        )
        .unwrap()[0];
    db.write_schemas(cs.batch).unwrap();

    root
}

fn verify_state_in_store(
    state_store: &StateStore,
    address: AccountAddress,
    expected_value: Option<&AccountStateBlob>,
    version: Version,
) {
    let (value, _proof) = state_store
        .get_account_state_with_proof_by_version(address, version)
        .unwrap();
    assert_eq!(value.as_ref(), expected_value);
}

#[test]
fn test_pruner() {
    let address = AccountAddress::new([1u8; AccountAddress::LENGTH]);
    let value0 = AccountStateBlob::from(vec![0x01]);
    let value1 = AccountStateBlob::from(vec![0x02]);
    let value2 = AccountStateBlob::from(vec![0x03]);

    let tmp_dir = TempPath::new();
    let db = DiemDB::new_for_test(&tmp_dir).db;
    let state_store = &StateStore::new(Arc::clone(&db));
    let pruner = Pruner::new(Arc::clone(&db), 0 /* historical_versions_to_keep */);

    let _root0 = put_account_state_set(
        &db,
        state_store,
        vec![(address, value0.clone())],
        0, /* version */
    );
    let _root1 = put_account_state_set(
        &db,
        state_store,
        vec![(address, value1.clone())],
        1, /* version */
    );
    let _root2 = put_account_state_set(
        &db,
        state_store,
        vec![(address, value2.clone())],
        2, /* version */
    );

    // Prune till version=0.
    {
        pruner.wake_and_wait(0 /* latest_version */).unwrap();
        verify_state_in_store(state_store, address, Some(&value0), 0);
        verify_state_in_store(state_store, address, Some(&value1), 1);
        verify_state_in_store(state_store, address, Some(&value2), 2);
    }
    // Prune till version=1.
    {
        pruner.wake_and_wait(1 /* latest_version */).unwrap();
        // root0 is gone.
        assert!(state_store
            .get_account_state_with_proof_by_version(address, 0)
            .is_err());
        // root1 is still there.
        verify_state_in_store(state_store, address, Some(&value1), 1);
        verify_state_in_store(state_store, address, Some(&value2), 2);
    }
    // Prune till version=2.
    {
        pruner.wake_and_wait(2 /* latest_version */).unwrap();
        // root1 is gone.
        assert!(state_store
            .get_account_state_with_proof_by_version(address, 1)
            .is_err());
        // root2 is still there.
        verify_state_in_store(state_store, address, Some(&value2), 2);
    }
}

#[test]
fn test_worker_quit_eagerly() {
    let address = AccountAddress::new([1u8; AccountAddress::LENGTH]);
    let value0 = AccountStateBlob::from(vec![0x01]);
    let value1 = AccountStateBlob::from(vec![0x02]);
    let value2 = AccountStateBlob::from(vec![0x03]);

    let tmp_dir = TempPath::new();
    let db = DiemDB::new_for_test(&tmp_dir).db;
    let state_store = &StateStore::new(Arc::clone(&db));

    let _root0 = put_account_state_set(
        &db,
        state_store,
        vec![(address, value0.clone())],
        0, /* version */
    );
    let _root1 = put_account_state_set(
        &db,
        state_store,
        vec![(address, value1.clone())],
        1, /* version */
    );
    let _root2 = put_account_state_set(
        &db,
        state_store,
        vec![(address, value2.clone())],
        2, /* version */
    );

    {
        let (command_sender, command_receiver) = channel();
        let worker = Worker::new(
            Arc::clone(&db),
            command_receiver,
            Arc::new(AtomicU64::new(0)), /* progress */
        );
        command_sender
            .send(Command::Prune {
                least_readable_version: 1,
            })
            .unwrap();
        command_sender
            .send(Command::Prune {
                least_readable_version: 2,
            })
            .unwrap();
        command_sender.send(Command::Quit).unwrap();
        // Worker quits immediately although `Command::Quit` is not the first command sent.
        worker.work();
        verify_state_in_store(state_store, address, Some(&value0), 0);
        verify_state_in_store(state_store, address, Some(&value1), 1);
        verify_state_in_store(state_store, address, Some(&value2), 2);
    }
}

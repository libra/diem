// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::LibraDB;
use libra_tools::tempdir::TempPath;
use libra_types::proof::verify_transaction_accumulator_element;
use proptest::{collection::vec, prelude::*};

fn verify(
    store: &LedgerStore,
    txn_infos: &[TransactionInfo],
    first_version: Version,
    ledger_version: Version,
    root_hash: HashValue,
) {
    txn_infos
        .iter()
        .enumerate()
        .for_each(|(idx, expected_txn_info)| {
            let version = first_version + idx as u64;

            let (txn_info, proof) = store
                .get_transaction_info_with_proof(version, ledger_version)
                .unwrap();

            assert_eq!(&txn_info, expected_txn_info);
            verify_transaction_accumulator_element(root_hash, txn_info.hash(), version, &proof)
                .unwrap();
        })
}

fn save(store: &LedgerStore, first_version: Version, txn_infos: &[TransactionInfo]) -> HashValue {
    let mut cs = ChangeSet::new();
    let root_hash = store
        .put_transaction_infos(first_version, &txn_infos, &mut cs)
        .unwrap();
    store.db.write_schemas(cs.batch).unwrap();
    root_hash
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]

    #[test]
    fn test_transaction_info_put_get_verify(
        batch1 in vec(any::<TransactionInfo>(), 1..100),
        batch2 in vec(any::<TransactionInfo>(), 1..100),
    ) {

        let tmp_dir = TempPath::new();
        let db = LibraDB::new(&tmp_dir);
        let store = &db.ledger_store;

        // insert two batches of transaction infos
        let root_hash1 = save(store, 0, &batch1);
        let ledger_version1 = batch1.len() as u64 - 1;
        let root_hash2 = save(store, batch1.len() as u64, &batch2);
        let ledger_version2 = batch1.len() as u64 + batch2.len() as u64 - 1;

        // retrieve all leaves and verify against latest root hash
        verify(store, &batch1, 0, ledger_version2, root_hash2);
        verify(store, &batch2, batch1.len() as u64, ledger_version2, root_hash2);

        // retrieve batch1 and verify against root_hash after batch1 was interted
        verify(store, &batch1, 0, ledger_version1, root_hash1);
    }
}

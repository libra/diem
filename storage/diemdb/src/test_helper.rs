// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module provides reusable helpers in tests.

use super::*;
use diem_crypto::hash::CryptoHash;
use diem_temppath::TempPath;
use diem_types::{
    block_info::BlockInfo,
    ledger_info::{LedgerInfo, LedgerInfoWithSignatures},
    proptest_types::{AccountInfoUniverse, LedgerInfoGen, TransactionToCommitGen},
    validator_signer::ValidatorSigner,
};
use proptest::{collection::vec, prelude::*};

fn to_blocks_to_commit(
    partial_blocks: Vec<(Vec<TransactionToCommit>, LedgerInfo, Vec<ValidatorSigner>)>,
) -> Result<Vec<(Vec<TransactionToCommit>, LedgerInfoWithSignatures)>> {
    // Use temporary DiemDB and STORE LEVEL APIs to calculate hashes on a per transaction basis.
    // Result is used to test the batch PUBLIC API for saving everything, i.e. `save_transactions()`
    let tmp_dir = TempPath::new();
    let db = DiemDB::new_for_test(&tmp_dir);

    let mut cur_ver = 0;
    let mut cur_txn_accu_hash = HashValue::zero();
    let blocks_to_commit = partial_blocks
        .into_iter()
        .map(|(txns_to_commit, partial_ledger_info, validator_set)| {
            for txn_to_commit in txns_to_commit.iter() {
                let mut cs = ChangeSet::new();

                let txn_hash = txn_to_commit.transaction().hash();
                let state_root_hash = db.state_store.put_account_state_sets(
                    vec![txn_to_commit.account_states().clone()],
                    None,
                    cur_ver,
                    &mut cs,
                )?[0];
                let event_root_hash =
                    db.event_store
                        .put_events(cur_ver, txn_to_commit.events(), &mut cs)?;

                let txn_info = TransactionInfo::new(
                    txn_hash,
                    state_root_hash,
                    event_root_hash,
                    txn_to_commit.gas_used(),
                    txn_to_commit.status().clone(),
                );
                let txn_accu_hash =
                    db.ledger_store
                        .put_transaction_infos(cur_ver, &[txn_info], &mut cs)?;
                db.db.write_schemas(cs.batch)?;

                cur_ver += 1;
                cur_txn_accu_hash = txn_accu_hash;
            }

            assert_eq!(cur_ver, partial_ledger_info.version() + 1);

            let block_info = BlockInfo::new(
                partial_ledger_info.epoch(),
                partial_ledger_info.round(),
                partial_ledger_info.consensus_block_id(),
                cur_txn_accu_hash,
                partial_ledger_info.version(),
                partial_ledger_info.timestamp_usecs(),
                partial_ledger_info.next_epoch_state().cloned(),
            );
            let ledger_info =
                LedgerInfo::new(block_info, partial_ledger_info.consensus_data_hash());
            let signatures = validator_set
                .iter()
                .map(|signer| (signer.author(), signer.sign(&ledger_info)))
                .collect();
            let ledger_info_with_sigs = LedgerInfoWithSignatures::new(ledger_info, signatures);
            Ok((txns_to_commit, ledger_info_with_sigs))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(blocks_to_commit)
}

prop_compose! {
    /// This returns a [`proptest`](https://altsysrq.github.io/proptest-book/intro.html)
    /// [`Strategy`](https://docs.rs/proptest/0/proptest/strategy/trait.Strategy.html) that yields an
    /// arbitrary number of arbitrary batches of transactions to commit.
    ///
    /// It is used in tests for both transaction block committing during normal running and
    /// transaction syncing during start up.
    pub fn arb_blocks_to_commit_impl(
        num_accounts: usize,
        max_txn_per_block: usize,
        max_blocks: usize,
    )(
        mut universe in any_with::<AccountInfoUniverse>(num_accounts).no_shrink(),
        batches in vec(
            (
                vec(any::<TransactionToCommitGen>(), 1..=max_txn_per_block),
                any::<LedgerInfoGen>(),
            ),
            1..=max_blocks,
        ),
    ) ->
        Vec<(
            Vec<TransactionToCommit>,
            LedgerInfoWithSignatures,
        )>
    {
        let partial_blocks = batches
            .into_iter()
            .map(|(txn_gens, ledger_info_gen)| {
                let block_size = txn_gens.len();
                let txns_to_commit = txn_gens
                        .into_iter()
                        .map(|gen| gen.materialize(&mut universe))
                        .collect();
                let ledger_info = ledger_info_gen.materialize(&mut universe, block_size);
                let validator_set = universe.get_validator_set(ledger_info.epoch()).to_vec();
                (
                    txns_to_commit,
                    ledger_info,
                    validator_set,
                )
            })
            .collect();

        to_blocks_to_commit(partial_blocks).unwrap()
    }
}

pub fn arb_blocks_to_commit(
) -> impl Strategy<Value = Vec<(Vec<TransactionToCommit>, LedgerInfoWithSignatures)>> {
    arb_blocks_to_commit_impl(
        5,  /* num_accounts */
        2,  /* max_txn_per_block */
        10, /* max_blocks */
    )
}

pub fn arb_mock_genesis() -> impl Strategy<Value = (TransactionToCommit, LedgerInfoWithSignatures)>
{
    arb_blocks_to_commit_impl(
        1, /* num_accounts */
        1, /* max_txn_per_block */
        1, /* max_blocks */
    )
    .prop_map(|blocks| {
        let (block, ledger_info_with_sigs) = &blocks[0];

        (block[0].clone(), ledger_info_with_sigs.clone())
    })
}

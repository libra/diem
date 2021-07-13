// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use byteorder::{BigEndian, WriteBytesExt};
use diem_config::config::RocksdbConfig;
use diem_crypto::hash::HashValue;
use diem_jellyfish_merkle::metrics::{
    DIEM_JELLYFISH_INTERNAL_ENCODED_BYTES, DIEM_JELLYFISH_LEAF_ENCODED_BYTES,
    DIEM_JELLYFISH_STORAGE_READS,
};
use diem_types::{
    account_address::AccountAddress,
    account_state_blob::AccountStateBlob,
    block_info::BlockInfo,
    ledger_info::{LedgerInfo, LedgerInfoWithSignatures},
    transaction::{ChangeSet, Transaction, TransactionToCommit, WriteSetPayload},
    vm_status::KeptVMStatus,
    write_set::WriteSetMut,
};
use diemdb::{
    metrics::DIEM_STORAGE_ROCKSDB_PROPERTIES, schema::JELLYFISH_MERKLE_NODE_CF_NAME, DiemDB,
};
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use rand::Rng;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::PathBuf,
};
use storage_interface::{DbReader, DbWriter};

pub fn gen_account_from_index(account_index: u64) -> AccountAddress {
    let mut array = [0u8; AccountAddress::LENGTH];
    array
        .as_mut()
        .write_u64::<BigEndian>(account_index)
        .expect("Unable to write u64 to array");
    AccountAddress::new(array)
}

pub fn gen_random_blob<R: Rng>(size: usize, rng: &mut R) -> AccountStateBlob {
    let mut v = vec![0u8; size];
    rng.fill(v.as_mut_slice());
    AccountStateBlob::from(v)
}

fn gen_txn_to_commit<R: Rng>(
    max_accounts: u64,
    blob_size: usize,
    rng: &mut R,
) -> TransactionToCommit {
    let txn = Transaction::GenesisTransaction(WriteSetPayload::Direct(ChangeSet::new(
        WriteSetMut::new(vec![])
            .freeze()
            .expect("freeze cannot fail"),
        vec![],
    )));
    let account1 = gen_account_from_index(rng.gen_range(0..max_accounts));
    let account2 = gen_account_from_index(rng.gen_range(0..max_accounts));
    let mut states = HashMap::new();
    let blob1 = gen_random_blob(blob_size, rng);
    let blob2 = gen_random_blob(blob_size, rng);
    states.insert(account1, blob1);
    states.insert(account2, blob2);
    TransactionToCommit::new(
        txn,
        states,
        HashMap::new(), /* new node hashes */
        vec![],         /* events */
        0,              /* gas_used */
        KeptVMStatus::Executed,
    )
}

pub fn run_benchmark(
    num_accounts: usize,
    total_version: u64,
    blob_size: usize,
    db_dir: PathBuf,
    prune_window: Option<u64>,
) {
    if db_dir.exists() {
        fs::remove_dir_all(db_dir.join("diemdb")).unwrap();
    }
    // create if not exists
    fs::create_dir_all(db_dir.clone()).unwrap();

    let db = DiemDB::open(
        &db_dir,
        false,        /* readonly */
        prune_window, /* pruner */
        RocksdbConfig::default(),
    )
    .expect("DB should open.");

    let mut rng = ::rand::thread_rng();
    let mut version = 0;

    // Set a progressing bar
    let bar = ProgressBar::new(total_version);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed}] {bar:100.cyan/blue} {pos:>7}/{len:7} {msg}"),
    );

    for chunk in &(0..total_version).chunks(1000 /* split by 1000 */) {
        let txns_to_commit = chunk
            .map(|_| gen_txn_to_commit(num_accounts as u64, blob_size, &mut rng))
            .collect::<Vec<_>>();
        let version_bump = txns_to_commit.len() as u64;
        db.save_transactions(
            &txns_to_commit,
            version,
            None, /* ledger_info_with_sigs */
        )
        .expect("commit cannot fail");
        version = version.checked_add(version_bump).expect("Cannot overflow");
        bar.inc(version_bump);
    }
    let accu_root_hash = db.get_accumulator_root_hash(total_version - 1).unwrap();
    // Last txn
    let li = LedgerInfo::new(
        BlockInfo::new(
            /* current_epoch = */ 0,
            /* round = */ 0,
            /* block_id */ HashValue::random_with_rng(&mut rng),
            accu_root_hash,
            total_version - 1,
            /* timestamp = */ 0,
            None,
        ),
        HashValue::random_with_rng(&mut rng),
    );
    let li_with_sigs = LedgerInfoWithSignatures::new(li, BTreeMap::new());
    db.save_transactions(&[], total_version, Some(&li_with_sigs))
        .unwrap();
    bar.finish();

    db.update_rocksdb_properties().unwrap();
    let db_size = DIEM_STORAGE_ROCKSDB_PROPERTIES
        .with_label_values(&[
            JELLYFISH_MERKLE_NODE_CF_NAME,
            "diem_rocksdb_live_sst_files_size_bytes",
        ])
        .get();
    let data_size = DIEM_STORAGE_ROCKSDB_PROPERTIES
        .with_label_values(&[JELLYFISH_MERKLE_NODE_CF_NAME, "diem_rocksdb_cf_size_bytes"])
        .get();
    let reads = DIEM_JELLYFISH_STORAGE_READS.get();
    let leaf_bytes = DIEM_JELLYFISH_LEAF_ENCODED_BYTES.get();
    let internal_bytes = DIEM_JELLYFISH_INTERNAL_ENCODED_BYTES.get();
    println!(
        "created a DiemDB til version {}, where {} accounts with avg blob size {} bytes exist.",
        total_version, num_accounts, blob_size
    );
    println!("DB dir: {}", db_dir.as_path().display());
    println!("Jellyfish Merkle physical size: {}", db_size);
    println!("Jellyfish Merkle logical size: {}", data_size);
    println!("Total reads from storage: {}", reads);
    println!(
        "Total written internal nodes value size: {} bytes",
        internal_bytes
    );
    println!("Total written leaf nodes value size: {} bytes", leaf_bytes);
}

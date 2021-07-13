// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use consensus_types::block::block_test_utils;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use diem_crypto::{ed25519::Ed25519PrivateKey, Uniform};
use diem_secure_storage::{InMemoryStorage, KVStorage, OnDiskStorage, Storage, VaultStorage};
use diem_types::validator_signer::ValidatorSigner;
use safety_rules::{test_utils, PersistentSafetyStorage, SafetyRulesManager, TSafetyRules};
use tempfile::NamedTempFile;

const VAULT_HOST: &str = "http://localhost:8200";
const VAULT_TOKEN: &str = "root_token";

/// Execute an in order series of blocks (0 <- 1 <- 2 <- 3 and commit 0 and continue to rotate
/// left, appending new blocks on the right, committing the left most block
fn lsr(mut safety_rules: Box<dyn TSafetyRules>, signer: ValidatorSigner, n: u64) {
    let data = block_test_utils::random_payload(1);

    let (proof, genesis_qc) = test_utils::make_genesis(&signer);
    safety_rules.initialize(&proof).unwrap();

    let mut round = genesis_qc.certified_block().round();

    round += 1;
    let mut b0 = test_utils::make_proposal_with_qc(round, genesis_qc, &signer, None);
    safety_rules.construct_and_sign_vote(&b0).unwrap();

    round += 1;
    let mut b1 =
        test_utils::make_proposal_with_parent(data.clone(), round, &b0, None, &signer, None);
    safety_rules.construct_and_sign_vote(&b1).unwrap();

    round += 1;
    let mut b2 =
        test_utils::make_proposal_with_parent(data.clone(), round, &b1, None, &signer, None);
    safety_rules.construct_and_sign_vote(&b2).unwrap();

    for _i in 0..n {
        round += 1;
        let b3 = test_utils::make_proposal_with_parent(
            data.clone(),
            round,
            &b2,
            Some(&b0),
            &signer,
            None,
        );

        safety_rules.construct_and_sign_vote(&b3).unwrap();

        b0 = b1;
        b1 = b2;
        b2 = b3;
    }
}

fn in_memory(n: u64) {
    let signer = ValidatorSigner::from_int(0);
    let waypoint = test_utils::validator_signers_to_waypoint(&[&signer]);
    let storage = PersistentSafetyStorage::initialize(
        Storage::from(InMemoryStorage::new()),
        signer.author(),
        signer.private_key().clone(),
        Ed25519PrivateKey::generate_for_testing(),
        waypoint,
        true,
    );
    let safety_rules_manager = SafetyRulesManager::new_local(storage, false, false, false);
    lsr(safety_rules_manager.client(), signer, n);
}

fn on_disk(n: u64) {
    let signer = ValidatorSigner::from_int(0);
    let file_path = NamedTempFile::new().unwrap().into_temp_path().to_path_buf();
    let waypoint = test_utils::validator_signers_to_waypoint(&[&signer]);
    let storage = PersistentSafetyStorage::initialize(
        Storage::from(OnDiskStorage::new(file_path)),
        signer.author(),
        signer.private_key().clone(),
        Ed25519PrivateKey::generate_for_testing(),
        waypoint,
        true,
    );
    let safety_rules_manager = SafetyRulesManager::new_local(storage, false, false, false);
    lsr(safety_rules_manager.client(), signer, n);
}

fn serializer(n: u64) {
    let signer = ValidatorSigner::from_int(0);
    let file_path = NamedTempFile::new().unwrap().into_temp_path().to_path_buf();
    let waypoint = test_utils::validator_signers_to_waypoint(&[&signer]);
    let storage = PersistentSafetyStorage::initialize(
        Storage::from(OnDiskStorage::new(file_path)),
        signer.author(),
        signer.private_key().clone(),
        Ed25519PrivateKey::generate_for_testing(),
        waypoint,
        true,
    );
    let safety_rules_manager = SafetyRulesManager::new_serializer(storage, false, false, false);
    lsr(safety_rules_manager.client(), signer, n);
}

fn thread(n: u64) {
    let signer = ValidatorSigner::from_int(0);
    let file_path = NamedTempFile::new().unwrap().into_temp_path().to_path_buf();
    let waypoint = test_utils::validator_signers_to_waypoint(&[&signer]);
    let storage = PersistentSafetyStorage::initialize(
        Storage::from(OnDiskStorage::new(file_path)),
        signer.author(),
        signer.private_key().clone(),
        Ed25519PrivateKey::generate_for_testing(),
        waypoint,
        true,
    );
    // Test value, in milliseconds
    let timeout_ms = 5_000;
    let safety_rules_manager =
        SafetyRulesManager::new_thread(storage, false, false, timeout_ms, false);
    lsr(safety_rules_manager.client(), signer, n);
}

fn vault(n: u64) {
    let signer = ValidatorSigner::from_int(0);
    let waypoint = test_utils::validator_signers_to_waypoint(&[&signer]);

    let mut storage = create_vault_storage();
    storage.reset_and_clear().unwrap();

    let storage = PersistentSafetyStorage::initialize(
        Storage::from(storage),
        signer.author(),
        signer.private_key().clone(),
        Ed25519PrivateKey::generate_for_testing(),
        waypoint,
        true,
    );
    // Test value in milliseconds.
    let timeout_ms = 5_000;
    let safety_rules_manager =
        SafetyRulesManager::new_thread(storage, false, false, timeout_ms, false);
    lsr(safety_rules_manager.client(), signer, n);
}

pub fn benchmark(c: &mut Criterion) {
    let count = 100;
    let duration_secs = 5;
    let samples = 10;

    let storage = create_vault_storage();

    let enable_vault = if storage.available().is_err() {
        println!(
            "Vault ({}) is not availble, experiment will not be launched",
            VAULT_HOST
        );
        false
    } else {
        true
    };

    let mut group = c.benchmark_group("SafetyRules");
    group
        .measurement_time(std::time::Duration::from_secs(duration_secs))
        .sample_size(samples);
    group.bench_function("InMemory", |b| b.iter(|| in_memory(black_box(count))));
    group.bench_function("OnDisk", |b| b.iter(|| on_disk(black_box(count))));
    group.bench_function("Serializer", |b| b.iter(|| serializer(black_box(count))));
    group.bench_function("Thread", |b| b.iter(|| thread(black_box(count))));

    if enable_vault {
        group.bench_function("Vault", |b| b.iter(|| vault(black_box(count))));
    }
}

fn create_vault_storage() -> VaultStorage {
    VaultStorage::new(
        VAULT_HOST.to_string(),
        VAULT_TOKEN.to_string(),
        None,
        None,
        true,
        None,
        None,
    )
}

criterion_group!(benches, benchmark);
criterion_main!(benches);

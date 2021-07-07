// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! A suite of client/sdk compatibility tests

#![cfg(test)]

use anyhow::Result;
use diem_sdk::{
    crypto::HashValue,
    transaction_builder::{
        stdlib::{self, ScriptCall},
        Currency, DualAttestationMessage,
    },
    types::{
        account_address::AccountAddress,
        ledger_info::LedgerInfoWithSignatures,
        proof::{AccumulatorConsistencyProof, TransactionAccumulatorSummary},
        transaction::{AccountTransactionsWithProof, Script},
        AccountKey,
    },
};
use std::convert::TryFrom;

mod env;
pub use env::{Coffer, Environment};

#[test]
#[ignore]
fn metadata() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let metadata = client.get_metadata()?.into_inner();

    // get_metadata documentation states that the following fields will be present when no version
    // argument is provided
    metadata.script_hash_allow_list.unwrap();
    metadata.diem_version.unwrap();
    metadata.module_publishing_allowed.unwrap();
    metadata.dual_attestation_limit.unwrap();

    Ok(())
}

#[test]
#[ignore]
fn metadata_by_version() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let current_version = client.get_metadata()?.into_inner().version;
    let metadata = client
        .get_metadata_by_version(current_version.saturating_sub(1))?
        .into_inner();

    // get_metadata documentation states that the following fields should not be present when a version
    // argument is provided
    assert!(metadata.script_hash_allow_list.is_none());
    assert!(metadata.diem_version.is_none());
    assert!(metadata.module_publishing_allowed.is_none());
    assert!(metadata.dual_attestation_limit.is_none());

    Ok(())
}

#[test]
#[ignore]
fn get_accumulator_consistency_proof() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let metadata = client.get_metadata()?.into_inner();

    // build a complete accumulator up to the version in the previous response.
    let proof_view = client
        .get_accumulator_consistency_proof(None, Some(metadata.version))?
        .into_inner();
    let proof = AccumulatorConsistencyProof::try_from(&proof_view)?;
    let accumulator =
        TransactionAccumulatorSummary::try_from_genesis_proof(proof, metadata.version)?;

    // the root hashes should match up
    assert_eq!(metadata.accumulator_root_hash, accumulator.root_hash());

    Ok(())
}

#[test]
#[ignore]
fn script_hash_allow_list() {
    let env = Environment::from_env();
    let client = env.client();

    // List of scripts that we are not committing to being backward compatable with:
    //     stdlib::encode_rotate_authentication_key_with_nonce_admin_script
    //     stdlib::encode_set_validator_operator_with_nonce_admin_script
    //     stdlib::encode_burn_script
    //     stdlib::encode_cancel_burn_script

    // Construct the full list of scripts that we expect to always be valid for BC purposes
    let allow_list: Vec<Script> = [
        // Scripts that can be submitted by anyone or by VASPs
        stdlib::encode_add_currency_to_account_script(Currency::XDX.type_tag()),
        stdlib::encode_add_recovery_rotation_capability_script(AccountAddress::ZERO),
        stdlib::encode_peer_to_peer_with_metadata_script(
            Currency::XDX.type_tag(),
            AccountAddress::ZERO,
            0,
            Vec::new(),
            Vec::new(),
        ),
        stdlib::encode_create_child_vasp_account_script(
            Currency::XDX.type_tag(),
            AccountAddress::ZERO,
            Vec::new(),
            false,
            0,
        ),
        stdlib::encode_create_recovery_address_script(),
        stdlib::encode_publish_shared_ed25519_public_key_script(Vec::new()),
        stdlib::encode_rotate_authentication_key_script(Vec::new()),
        stdlib::encode_rotate_authentication_key_with_recovery_address_script(
            AccountAddress::ZERO,
            AccountAddress::ZERO,
            Vec::new(),
        ),
        stdlib::encode_rotate_dual_attestation_info_script(Vec::new(), Vec::new()),
        stdlib::encode_rotate_shared_ed25519_public_key_script(Vec::new()),
        // Root Account Scripts
        stdlib::encode_add_validator_and_reconfigure_script(0, Vec::new(), AccountAddress::ZERO),
        stdlib::encode_create_validator_account_script(
            0,
            AccountAddress::ZERO,
            Vec::new(),
            Vec::new(),
        ),
        stdlib::encode_create_validator_operator_account_script(
            0,
            AccountAddress::ZERO,
            Vec::new(),
            Vec::new(),
        ),
        stdlib::encode_remove_validator_and_reconfigure_script(0, Vec::new(), AccountAddress::ZERO),
        stdlib::encode_rotate_authentication_key_with_nonce_script(0, Vec::new()),
        stdlib::encode_update_diem_version_script(0, 0),
        // TreasuryCompliance Scripts
        stdlib::encode_burn_txn_fees_script(Currency::XDX.type_tag()),
        stdlib::encode_create_designated_dealer_script(
            Currency::XDX.type_tag(),
            0,
            AccountAddress::ZERO,
            Vec::new(),
            Vec::new(),
            false,
        ),
        stdlib::encode_create_parent_vasp_account_script(
            Currency::XDX.type_tag(),
            0,
            AccountAddress::ZERO,
            Vec::new(),
            Vec::new(),
            false,
        ),
        stdlib::encode_freeze_account_script(0, AccountAddress::ZERO),
        stdlib::encode_preburn_script(Currency::XDX.type_tag(), 0),
        stdlib::encode_tiered_mint_script(Currency::XDX.type_tag(), 0, AccountAddress::ZERO, 0, 0),
        stdlib::encode_unfreeze_account_script(0, AccountAddress::ZERO),
        stdlib::encode_update_dual_attestation_limit_script(0, 0),
        stdlib::encode_update_exchange_rate_script(Currency::XDX.type_tag(), 0, 0, 0),
        stdlib::encode_update_minting_ability_script(Currency::XDX.type_tag(), false),
        // Validator Operator Scripts
        stdlib::encode_register_validator_config_script(
            AccountAddress::ZERO,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        stdlib::encode_set_validator_config_and_reconfigure_script(
            AccountAddress::ZERO,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        stdlib::encode_set_validator_operator_script(Vec::new(), AccountAddress::ZERO),
    ]
    .to_vec();

    let metadata = client.get_metadata().unwrap().into_inner();
    let published_allow_list = metadata.script_hash_allow_list.unwrap();

    println!("{:#?}", published_allow_list);

    for script in allow_list {
        let hash = HashValue::sha3_256_of(script.code());
        let script_call = ScriptCall::decode(&script).unwrap();
        let script_name = format!("{:?}", script_call)
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned();
        assert!(
            published_allow_list.contains(&hash),
            "Network doesn't have '{}' in script allow list",
            script_name
        );
    }
}

#[test]
#[ignore]
fn currency_info() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let currencies = client.get_currencies()?.into_inner();

    // Check that XDX and XUS currencies exist
    assert!(currencies.iter().any(|c| c.code == Currency::XDX));
    assert!(currencies.iter().any(|c| c.code == Currency::XUS));

    Ok(())
}

#[test]
#[ignore]
fn get_events() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let currencies = client.get_currencies()?.into_inner();

    for currency in currencies {
        client.get_events(currency.mint_events_key, 0, 10)?;
    }

    Ok(())
}

#[test]
#[ignore]
fn network_status() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    client.get_network_status()?;

    Ok(())
}

#[test]
#[ignore]
fn fund_account() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let account = env.random_account();
    let amount = 1000;
    let currency = Currency::XUS;
    env.coffer()
        .fund(currency, account.authentication_key(), amount)?;

    let account_view = client.get_account(account.address())?.into_inner().unwrap();
    let balance = account_view
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, amount);

    Ok(())
}

#[test]
#[ignore]
fn get_account_by_version() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let account = env.random_account();

    // get_account(_) is like get_account_by_version(_, latest version). If we
    // call get_account_by_version at the version get_account was fulfilled at,
    // we should always get the same response.

    let (account_view_1, state_1) = client.get_account(account.address())?.into_parts();
    let account_view_2 = client
        .get_account_by_version(account.address(), state_1.version)?
        .into_inner();
    assert_eq!(account_view_1, account_view_2);

    env.coffer()
        .fund(Currency::XUS, account.authentication_key(), 1000)?;

    // The responses should still be the same after the account is created and funded.

    let (account_view_3, state_3) = client.get_account(account.address())?.into_parts();
    let account_view_4 = client
        .get_account_by_version(account.address(), state_3.version)?
        .into_inner();
    assert_eq!(account_view_3, account_view_4);

    // We should be able to look up the historical account state with get_account_by_version.

    let account_view_5 = client
        .get_account_by_version(account.address(), state_1.version)?
        .into_inner();
    assert_eq!(account_view_1, account_view_5);

    Ok(())
}

#[test]
#[ignore]
fn get_account_transactions_with_proofs() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    // create and fund some accounts
    let mut account_1 = env.random_account();
    let account_2 = env.random_account();

    env.coffer()
        .fund(Currency::XUS, account_1.authentication_key(), 1000)?;
    env.coffer()
        .fund(Currency::XUS, account_2.authentication_key(), 1000)?;

    // generate some transaction activity
    let num_txns = 3;
    for _ in 0..num_txns {
        let txn = account_1.sign_with_transaction_builder(env.transaction_factory().peer_to_peer(
            Currency::XUS,
            account_2.address(),
            100,
        ));

        client.submit(&txn)?;
        client.wait_for_signed_transaction(&txn, None, None)?;
    }

    // get a ledger info so we can verify proofs
    let li_view = client
        .get_state_proof(0)?
        .into_inner()
        .ledger_info_with_signatures;
    let latest_li = bcs::from_bytes::<LedgerInfoWithSignatures>(li_view.as_ref())?;
    let ledger_version = latest_li.ledger_info().version();

    // request the sending account's transactions and verify the proofs
    let txns_view = client
        .get_account_transactions_with_proofs(
            account_1.address(),
            0,
            100,
            true,
            Some(ledger_version),
        )?
        .into_inner();
    let txns = AccountTransactionsWithProof::try_from(&txns_view)?;

    assert_eq!(num_txns, txns.len());
    txns.verify(
        latest_li.ledger_info(),
        account_1.address(),
        0,
        100,
        true,
        ledger_version,
    )?;

    Ok(())
}

#[test]
#[ignore]
fn create_child_account() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let mut account = env.random_account();
    let amount = 1000;
    let currency = Currency::XUS;
    env.coffer()
        .fund(currency, account.authentication_key(), amount)?;

    let child_account = env.random_account();

    // create a child account
    let txn =
        account.sign_with_transaction_builder(env.transaction_factory().create_child_vasp_account(
            currency,
            child_account.authentication_key(),
            false,
            0,
        ));

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    let account_view = client
        .get_account(child_account.address())?
        .into_inner()
        .unwrap();
    let balance = account_view
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, 0);

    Ok(())
}

#[test]
#[ignore]
fn add_currency_to_account() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let amount = 1000;
    let currency = Currency::XUS;

    let mut account = env.random_account();
    env.coffer()
        .fund(currency, account.authentication_key(), amount)?;

    // Ensure that the account doesn't carry a balance of XDX
    let account_view = client.get_account(account.address())?.into_inner().unwrap();
    assert!(account_view
        .balances
        .iter()
        .find(|b| b.currency == Currency::XDX)
        .is_none());

    // Submit txn to add XDX to the account
    let txn = account.sign_with_transaction_builder(
        env.transaction_factory()
            .add_currency_to_account(Currency::XDX),
    );

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    // Verify that the account has a 0 balance of XDX
    let account_view = client.get_account(account.address())?.into_inner().unwrap();
    let balance = account_view
        .balances
        .iter()
        .find(|b| b.currency == Currency::XDX)
        .unwrap();
    assert_eq!(balance.amount, 0);

    Ok(())
}

#[test]
#[ignore]
fn peer_to_peer() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let start_amount = 1000;
    let transfer_amount = 100;
    let currency = Currency::XUS;

    let mut account = env.random_account();
    env.coffer()
        .fund(currency, account.authentication_key(), start_amount)?;

    let account_2 = env.random_account();
    env.coffer()
        .fund(currency, account_2.authentication_key(), start_amount)?;

    let txn = account.sign_with_transaction_builder(env.transaction_factory().peer_to_peer(
        currency,
        account_2.address(),
        transfer_amount,
    ));

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    let account_view_1 = client.get_account(account.address())?.into_inner().unwrap();
    let balance = account_view_1
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, start_amount - transfer_amount);

    let account_view_2 = client
        .get_account(account_2.address())?
        .into_inner()
        .unwrap();
    let balance = account_view_2
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, start_amount + transfer_amount);

    Ok(())
}

#[test]
#[ignore]
fn rotate_authentication_key() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let amount = 1000;
    let currency = Currency::XUS;

    let mut account = env.random_account();
    env.coffer()
        .fund(currency, account.authentication_key(), amount)?;

    let rotated_key = AccountKey::generate(&mut rand::rngs::OsRng);

    // Submit txn to rotate the auth key
    let txn = account.sign_with_transaction_builder(
        env.transaction_factory()
            .rotate_authentication_key(rotated_key.authentication_key()),
    );

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    // Verify that the account has the new authentication key
    let account_view = client.get_account(account.address())?.into_inner().unwrap();
    assert_eq!(
        account_view.authentication_key.inner(),
        rotated_key.authentication_key().as_ref(),
    );

    let old_key = account.rotate_key(rotated_key);

    // Verify that we can rotate it back
    let txn = account.sign_with_transaction_builder(
        env.transaction_factory()
            .rotate_authentication_key(old_key.authentication_key()),
    );

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    // Verify that the account has the old auth key again
    let account_view = client.get_account(account.address())?.into_inner().unwrap();
    assert_eq!(
        account_view.authentication_key.inner(),
        old_key.authentication_key().as_ref(),
    );

    Ok(())
}

#[test]
#[ignore]
fn dual_attestation() -> Result<()> {
    use diem_sdk::crypto::ed25519::ed25519_dalek;

    let env = Environment::from_env();
    let client = env.client();

    let dual_attestation_limit = client
        .get_metadata()?
        .into_inner()
        .dual_attestation_limit
        .unwrap();
    let start_amount = dual_attestation_limit + 1000;
    let transfer_amount = dual_attestation_limit + 100;
    let currency = Currency::XUS;

    let mut account = env.random_account();
    env.coffer()
        .fund(currency, account.authentication_key(), start_amount)?;

    let mut account_2 = env.random_account();
    env.coffer()
        .fund(currency, account_2.authentication_key(), start_amount)?;

    // Sending a txn that's over the dual_attestation_limit without a signature from the reciever
    let txn = account.sign_with_transaction_builder(env.transaction_factory().peer_to_peer(
        currency,
        account_2.address(),
        transfer_amount,
    ));

    client.submit(&txn)?;
    assert!(matches!(
        client
            .wait_for_signed_transaction(&txn, None, None)
            .unwrap_err(),
        diem_sdk::client::WaitForTransactionError::TransactionExecutionFailed(_),
    ));

    // Add a dual_attestation creds for account_2
    let dual_attestation_secret_key = ed25519_dalek::SecretKey::generate(&mut rand::rngs::OsRng);
    let dual_attestation_extended_secret_key =
        ed25519_dalek::ExpandedSecretKey::from(&dual_attestation_secret_key);
    let dual_attestation_public_key = ed25519_dalek::PublicKey::from(&dual_attestation_secret_key);

    let txn = account_2.sign_with_transaction_builder(
        env.transaction_factory().rotate_dual_attestation_info(
            "https://example.com".as_bytes().to_vec(),
            dual_attestation_public_key.as_bytes().to_vec(),
        ),
    );

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    let metadata = "metadata";
    let sig = {
        let message =
            DualAttestationMessage::new(metadata.as_bytes(), account.address(), transfer_amount);
        dual_attestation_extended_secret_key
            .sign(message.message(), &dual_attestation_public_key)
            .to_bytes()
            .to_vec()
    };
    // Send a p2p txn with the signed metadata
    let txn = account.sign_with_transaction_builder(
        env.transaction_factory().peer_to_peer_with_metadata(
            currency,
            account_2.address(),
            transfer_amount,
            metadata.as_bytes().to_vec(),
            sig,
        ),
    );

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    let account_view_1 = client.get_account(account.address())?.into_inner().unwrap();
    let balance = account_view_1
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, start_amount - transfer_amount);

    let account_view_2 = client
        .get_account(account_2.address())?
        .into_inner()
        .unwrap();
    let balance = account_view_2
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, start_amount + transfer_amount);

    Ok(())
}

#[test]
#[ignore]
fn no_dual_attestation_needed_between_parent_vasp_and_its_child() -> Result<()> {
    let env = Environment::from_env();
    let client = env.client();

    let dual_attestation_limit = client
        .get_metadata()?
        .into_inner()
        .dual_attestation_limit
        .unwrap();
    let start_amount = dual_attestation_limit + 1000;
    let transfer_amount = dual_attestation_limit + 100;
    let currency = Currency::XUS;

    let mut account = env.random_account();
    env.coffer()
        .fund(currency, account.authentication_key(), start_amount)?;

    let child_account = env.random_account();

    // create a child account
    let txn =
        account.sign_with_transaction_builder(env.transaction_factory().create_child_vasp_account(
            currency,
            child_account.authentication_key(),
            false,
            0,
        ));

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    // Send a p2p txn over the dual_attestation_limit
    let txn = account.sign_with_transaction_builder(env.transaction_factory().peer_to_peer(
        currency,
        child_account.address(),
        transfer_amount,
    ));

    client.submit(&txn)?;
    client.wait_for_signed_transaction(&txn, None, None)?;

    let account_view_1 = client.get_account(account.address())?.into_inner().unwrap();
    let balance = account_view_1
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, start_amount - transfer_amount);

    let account_view_2 = client
        .get_account(child_account.address())?
        .into_inner()
        .unwrap();
    let balance = account_view_2
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, transfer_amount);

    Ok(())
}

// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use diem_sdk::{
    client::{views::TransactionDataView, SignedTransaction},
    crypto::{ed25519::Ed25519PrivateKey, PrivateKey, SigningKey, Uniform},
    transaction_builder::Currency,
    types::{account_config::XUS_NAME, transaction::authenticator::AuthenticationKey},
};
use forge::{PublicUsageContext, PublicUsageTest, Result, Test};

pub struct ExternalTransactionSigner;

impl Test for ExternalTransactionSigner {
    fn name(&self) -> &'static str {
        "smoke-test::external-transaction-signer"
    }
}

impl PublicUsageTest for ExternalTransactionSigner {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let client = ctx.client();

        // generate key pair
        let private_key = Ed25519PrivateKey::generate(ctx.rng());
        let public_key = private_key.public_key();

        // create transfer parameters
        let sender_auth_key = AuthenticationKey::ed25519(&public_key);
        let sender_address = sender_auth_key.derived_address();
        ctx.create_parent_vasp_account(sender_auth_key)?;
        ctx.fund(sender_address, 10_000_000)?;

        let receiver = ctx.random_account();
        ctx.create_parent_vasp_account(receiver.authentication_key())?;
        ctx.fund(receiver.address(), 1_000_000)?;

        let amount = 1_000_000;
        let test_gas_unit_price = 1;
        let test_max_gas_amount = 1_000_000;

        // prepare transfer transaction
        let test_sequence_number = client
            .get_account(sender_address)?
            .into_inner()
            .unwrap()
            .sequence_number;

        let currency_code = XUS_NAME;

        let unsigned_txn = ctx
            .transaction_factory()
            .peer_to_peer(Currency::XUS, receiver.address(), amount)
            .sender(sender_address)
            .sequence_number(test_sequence_number)
            .max_gas_amount(test_max_gas_amount)
            .gas_unit_price(test_gas_unit_price)
            .build();

        assert_eq!(unsigned_txn.sender(), sender_address);

        // sign the transaction with the private key
        let signature = private_key.sign(&unsigned_txn);

        // submit the transaction
        let txn = SignedTransaction::new(unsigned_txn, public_key, signature);
        client.submit(&txn)?;
        client.wait_for_signed_transaction(&txn, None, None)?;

        // query the transaction and check it contains the same values as requested
        let txn = client
            .get_account_transaction(sender_address, test_sequence_number, false)?
            .into_inner()
            .unwrap();

        match txn.transaction {
            TransactionDataView::UserTransaction {
                sender,
                sequence_number,
                max_gas_amount,
                gas_unit_price,
                gas_currency,
                script,
                ..
            } => {
                assert_eq!(sender, sender_address);
                assert_eq!(sequence_number, test_sequence_number);
                assert_eq!(gas_unit_price, test_gas_unit_price);
                assert_eq!(gas_currency, currency_code.to_string());
                assert_eq!(max_gas_amount, test_max_gas_amount);

                assert_eq!(script.r#type, "peer_to_peer_with_metadata");
                assert_eq!(script.type_arguments.unwrap(), vec!["XUS"]);
                assert_eq!(
                    script.arguments.unwrap(),
                    vec![
                        format!("{{ADDRESS: {:?}}}", receiver.address()),
                        format!("{{U64: {}}}", amount),
                        "{U8Vector: 0x}".to_string(),
                        "{U8Vector: 0x}".to_string()
                    ]
                );
                // legacy fields
                assert_eq!(script.receiver.unwrap(), receiver.address());
                assert_eq!(script.amount.unwrap(), amount);
                assert_eq!(script.currency.unwrap(), currency_code.to_string());
                assert!(script.metadata.unwrap().inner().is_empty());
                assert!(script.metadata_signature.unwrap().inner().is_empty());
            }
            _ => panic!("Query should get user transaction"),
        }
        Ok(())
    }
}

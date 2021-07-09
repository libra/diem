// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{Coffer, PublicInfo, Result};
use diem_sdk::{
    client::BlockingClient,
    transaction_builder::{Currency, TransactionFactory},
    types::{
        account_address::AccountAddress, chain_id::ChainId,
        transaction::authenticator::AuthenticationKey, LocalAccount,
    },
};

#[derive(Debug)]
pub struct ChainInfo<'t> {
    pub root_account: &'t mut LocalAccount,
    pub treasury_compliance_account: &'t mut LocalAccount,
    pub designated_dealer_account: &'t mut LocalAccount,
    pub json_rpc_url: String,
    pub chain_id: ChainId,
}

impl<'t> ChainInfo<'t> {
    pub fn new(
        root_account: &'t mut LocalAccount,
        treasury_compliance_account: &'t mut LocalAccount,
        designated_dealer_account: &'t mut LocalAccount,
        json_rpc_url: String,
        chain_id: ChainId,
    ) -> Self {
        Self {
            root_account,
            treasury_compliance_account,
            designated_dealer_account,
            json_rpc_url,
            chain_id,
        }
    }

    pub fn designated_dealer_account(&mut self) -> &mut LocalAccount {
        self.designated_dealer_account
    }

    pub fn root_account(&mut self) -> &mut LocalAccount {
        self.root_account
    }

    pub fn treasury_compliance_account(&mut self) -> &mut LocalAccount {
        self.treasury_compliance_account
    }

    pub fn json_rpc(&self) -> &str {
        &self.json_rpc_url
    }

    pub fn json_rpc_client(&self) -> BlockingClient {
        BlockingClient::new(&self.json_rpc_url)
    }

    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    pub fn transaction_factory(&self) -> TransactionFactory {
        TransactionFactory::new(self.chain_id())
    }

    pub fn create_parent_vasp_account(
        &mut self,
        currency: Currency,
        authentication_key: AuthenticationKey,
    ) -> Result<()> {
        let factory = self.transaction_factory();
        let client = self.json_rpc_client();
        let treasury_compliance_account = self.treasury_compliance_account();

        let create_account_txn = treasury_compliance_account.sign_with_transaction_builder(
            factory.create_parent_vasp_account(
                currency,
                0,
                authentication_key,
                &format!("No. {} VASP", treasury_compliance_account.sequence_number()),
                false,
            ),
        );
        client.submit(&create_account_txn)?;
        client.wait_for_signed_transaction(&create_account_txn, None, None)?;
        Ok(())
    }

    pub fn create_designated_dealer_account(
        &mut self,
        currency: Currency,
        authentication_key: AuthenticationKey,
    ) -> Result<()> {
        let factory = self.transaction_factory();
        let client = self.json_rpc_client();
        let treasury_compliance_account = self.treasury_compliance_account();

        let create_account_txn = treasury_compliance_account.sign_with_transaction_builder(
            factory.create_designated_dealer(
                currency,
                0, // sliding_nonce
                authentication_key,
                &format!("No. {} DD", treasury_compliance_account.sequence_number()),
                false, // add all currencies
            ),
        );
        client.submit(&create_account_txn)?;
        client.wait_for_signed_transaction(&create_account_txn, None, None)?;
        Ok(())
    }

    pub fn fund(&mut self, currency: Currency, address: AccountAddress, amount: u64) -> Result<()> {
        let factory = self.transaction_factory();
        let client = self.json_rpc_client();
        let designated_dealer_account = self.designated_dealer_account();
        let fund_account_txn = designated_dealer_account
            .sign_with_transaction_builder(factory.peer_to_peer(currency, address, amount));
        client.submit(&fund_account_txn)?;
        client.wait_for_signed_transaction(&fund_account_txn, None, None)?;
        Ok(())
    }

    pub fn into_public_info(self) -> PublicInfo<'t> {
        PublicInfo::new(
            self.json_rpc_url.clone(),
            self.chain_id,
            Coffer::TreasuryCompliance {
                transaction_factory: TransactionFactory::new(self.chain_id),
                json_rpc_client: BlockingClient::new(self.json_rpc_url),
                treasury_compliance_account: self.treasury_compliance_account,
                designated_dealer_account: self.designated_dealer_account,
            },
        )
    }
}

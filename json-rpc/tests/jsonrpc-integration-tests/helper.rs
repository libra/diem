// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::{format_err, Result};
use diem_crypto::hash::CryptoHash;
use diem_json_rpc_types::response::JsonRpcResponse;
use diem_sdk::{transaction_builder::Currency, types::LocalAccount};
use diem_types::{
    account_address::AccountAddress, account_config::XUS_NAME, chain_id::ChainId,
    transaction::SignedTransaction,
};
use forge::PublicUsageContext;
use serde_json::{json, Value};

pub struct JsonRpcTestHelper {
    url: String,
    client: reqwest::blocking::Client,
    allow_execution_failures: bool,
}

impl JsonRpcTestHelper {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::blocking::Client::new(),
            allow_execution_failures: false,
        }
    }

    pub fn get_metadata(&self) -> Value {
        self.send("get_metadata", json!([])).result.unwrap()
    }

    pub fn get_balance(&self, address: AccountAddress, currency: &str) -> u64 {
        let resp = self.send("get_account", json!([address]));
        let account = resp.result.unwrap();
        let balances = account["balances"].as_array().unwrap();
        for balance in balances.iter() {
            if balance["currency"].as_str().unwrap() == currency {
                return balance["amount"].as_u64().unwrap();
            }
        }
        0
    }

    pub fn allow_execution_failures<F, T>(&mut self, mut f: F) -> T
    where
        F: FnMut(&mut JsonRpcTestHelper) -> T,
    {
        self.allow_execution_failures = true;
        let ret = f(self);
        self.allow_execution_failures = false;
        ret
    }

    pub fn send(&self, method: &'static str, params: Value) -> JsonRpcResponse {
        let request = json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1});
        let resp = self
            .client
            .post(self.url.as_str())
            .json(&request)
            .send()
            .expect("request success");
        assert_eq!(resp.status(), 200);
        let headers = resp.headers().clone();
        let json: serde_json::Value = resp.json().unwrap();
        if !self.allow_execution_failures {
            assert_eq!(json.get("error"), None);
        }
        let rpc_resp: JsonRpcResponse =
            serde_json::from_value(json).expect("should be valid JsonRpcResponse");
        assert_eq!(
            headers.get("X-Diem-Chain-Id").unwrap().to_str().unwrap(),
            rpc_resp.diem_chain_id.to_string()
        );
        assert_eq!(
            headers
                .get("X-Diem-Ledger-Version")
                .unwrap()
                .to_str()
                .unwrap(),
            rpc_resp.diem_ledger_version.to_string()
        );
        assert_eq!(
            headers
                .get("X-Diem-Ledger-TimestampUsec")
                .unwrap()
                .to_str()
                .unwrap(),
            rpc_resp.diem_ledger_timestampusec.to_string()
        );
        rpc_resp
    }

    pub fn send_request(&self, request: Value) -> Value {
        let resp = self
            .client
            .post(self.url.as_str())
            .json(&request)
            .send()
            .expect("request success");
        assert_eq!(resp.status(), 200);

        resp.json().unwrap()
    }

    pub fn submit_and_wait(&self, txn: &SignedTransaction) -> Value {
        self.submit(&txn);
        self.wait_for_txn(&txn)
    }

    pub fn submit(&self, txn: &SignedTransaction) -> JsonRpcResponse {
        let txn_hex = hex::encode(bcs::to_bytes(txn).expect("bcs txn failed"));
        self.send("submit", json!([txn_hex]))
    }

    pub fn wait_for_txn(&self, txn: &SignedTransaction) -> Value {
        let txn_hash = diem_types::transaction::Transaction::UserTransaction(txn.clone())
            .hash()
            .to_hex();
        for _i in 0..60 {
            let resp = self.get_account_transaction(&txn.sender(), txn.sequence_number(), true);
            if let Some(result) = resp.result {
                if result.is_object() {
                    if !self.allow_execution_failures {
                        assert_eq!(result["vm_status"]["type"], "executed", "{:#}", result);
                    }
                    assert_eq!(result["hash"], txn_hash, "{:#}", result);
                    return result;
                }
            }
            ::std::thread::sleep(::std::time::Duration::from_millis(500));
        }
        panic!("transaction not executed?");
    }

    pub fn get_account_transaction(
        &self,
        address: &AccountAddress,
        seq_num: u64,
        include_events: bool,
    ) -> JsonRpcResponse {
        self.send(
            "get_account_transaction",
            json!([hex::encode(address), seq_num, include_events]),
        )
    }

    pub fn create_parent_and_child_accounts(
        &self,
        ctx: &mut PublicUsageContext<'_>,
        amount: u64,
    ) -> Result<(LocalAccount, LocalAccount, LocalAccount)> {
        let factory = ctx.transaction_factory();
        let mut parent = ctx.random_account();
        let child1 = ctx.random_account();
        let child2 = ctx.random_account();
        ctx.create_parent_vasp_account(parent.authentication_key())?;
        self.submit_and_wait(&parent.sign_with_transaction_builder(
            factory.create_child_vasp_account(Currency::XUS, child1.authentication_key(), false, 0),
        ));
        self.submit_and_wait(&parent.sign_with_transaction_builder(
            factory.create_child_vasp_account(Currency::XUS, child2.authentication_key(), false, 0),
        ));

        ctx.fund(parent.address(), amount)?;
        ctx.fund(child1.address(), amount)?;
        ctx.fund(child2.address(), amount)?;

        Ok((parent, child1, child2))
    }

    pub fn get_account_sequence(&self, address: AccountAddress) -> Result<u64> {
        let resp = self.send("get_account", json!([address]));
        if let Some(result) = resp.result {
            if result.is_object() {
                return Ok(result["sequence_number"].as_u64().unwrap());
            }
        }
        Err(format_err!("account not found: {}", address))
    }

    pub fn create_multi_agent_txn(
        &self,
        sender: &mut LocalAccount,
        secondary_signers: &[&mut LocalAccount],
        payload: diem_types::transaction::TransactionPayload,
    ) -> SignedTransaction {
        let seq_onchain = self
            .get_account_sequence(sender.address())
            .expect("account should exist onchain for create transaction");
        let seq = sender.sequence_number();
        assert_eq!(seq, seq_onchain);
        *sender.sequence_number_mut() += 1;
        let raw_txn = diem_types::transaction::helpers::create_unsigned_txn(
            payload,
            sender.address(),
            seq,
            1_000_000,
            0,
            XUS_NAME.to_owned(),
            30,
            ChainId::test(),
        );
        raw_txn
            .sign_multi_agent(
                sender.private_key(),
                secondary_signers
                    .iter()
                    .map(|signer| signer.address())
                    .collect(),
                secondary_signers
                    .iter()
                    .map(|signer| signer.private_key())
                    .collect(),
            )
            .unwrap()
            .into_inner()
    }
}

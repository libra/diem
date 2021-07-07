// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use diem_sdk::{
    client::{BlockingClient, FaucetClient},
    transaction_builder::{Currency, TransactionFactory},
    types::{chain_id::ChainId, transaction::authenticator::AuthenticationKey, LocalAccount},
};
use once_cell::sync::OnceCell;
use std::env;

const JSON_RPC_URL: &str = "JSON_RPC_URL";
const FAUCET_URL: &str = "FAUCET_URL";

pub struct Environment {
    json_rpc_url: String,
    coffer: Coffer,
    chain_id: ChainId,
}

impl Environment {
    pub fn from_env() -> &'static Self {
        static ENV: OnceCell<Environment> = OnceCell::new();

        ENV.get_or_init(|| Self::read_env().unwrap())
    }

    fn read_env() -> Result<Self> {
        let json_rpc_url = env::var(JSON_RPC_URL)?;
        let coffer = env::var(FAUCET_URL)
            .map(|f| FaucetClient::new(f, json_rpc_url.clone()))
            .map(Coffer::Faucet)?;

        let chain_id = ChainId::new(
            BlockingClient::new(&json_rpc_url)
                .get_metadata()?
                .into_inner()
                .chain_id,
        );

        Ok(Self::new(json_rpc_url, coffer, chain_id))
    }

    fn new(json_rpc_url: String, coffer: Coffer, chain_id: ChainId) -> Self {
        Self {
            json_rpc_url,
            coffer,
            chain_id,
        }
    }

    pub fn json_rpc_url(&self) -> &str {
        &self.json_rpc_url
    }

    pub fn client(&self) -> BlockingClient {
        BlockingClient::new(self.json_rpc_url())
    }

    pub fn coffer(&self) -> &Coffer {
        &self.coffer
    }

    pub fn chain_id(&self) -> ChainId {
        self.chain_id
    }

    pub fn transaction_factory(&self) -> TransactionFactory {
        TransactionFactory::new(self.chain_id())
    }

    pub fn random_account(&self) -> LocalAccount {
        LocalAccount::generate(&mut rand::rngs::OsRng)
    }
}

pub enum Coffer {
    Faucet(FaucetClient),
}

impl Coffer {
    pub fn fund(&self, currency: Currency, auth_key: AuthenticationKey, amount: u64) -> Result<()> {
        match self {
            Coffer::Faucet(faucet) => faucet
                .fund(currency.as_str(), auth_key, amount)
                .map_err(Into::into),
        }
    }
}

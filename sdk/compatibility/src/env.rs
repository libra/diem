// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use diem_sdk::{
    client::{
        stream::{StreamingClient, StreamingClientConfig},
        BlockingClient, FaucetClient, StreamResult,
    },
    transaction_builder::{Currency, TransactionFactory},
    types::{chain_id::ChainId, transaction::authenticator::AuthenticationKey, LocalAccount},
};
use once_cell::sync::OnceCell;
use std::env;
use url::Url;

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

    pub fn websocket_rpc_url(&self) -> String {
        let mut url = Url::parse(&self.json_rpc_url).expect("Invalid json_rpc_url");
        url.set_scheme("ws").expect("Could not set scheme");
        url.set_path("/v1/stream/ws");
        url.to_string()
    }

    pub fn client(&self) -> BlockingClient {
        BlockingClient::new(self.json_rpc_url())
    }

    pub async fn streaming_client(
        &self,
        config: Option<StreamingClientConfig>,
    ) -> StreamResult<StreamingClient> {
        StreamingClient::new(self.websocket_rpc_url(), config.unwrap_or_default(), None).await
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

///////////
// Tests //
///////////

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_websocket_url() {
        let json_rpc_url = "http://localhost:1337/v1".to_string();
        let coffer = FaucetClient::new(json_rpc_url.clone(), json_rpc_url.clone());
        let env = Environment::new(json_rpc_url, Coffer::Faucet(coffer), ChainId::test());

        let expected = "ws://localhost:1337/v1/stream/ws";
        assert_eq!(env.websocket_rpc_url(), expected)
    }
}

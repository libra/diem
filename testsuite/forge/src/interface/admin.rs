// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use super::{ChainInfo, CoreContext, Test};
use crate::Result;
use diem_sdk::{client::BlockingClient, types::LocalAccount};

/// The testing interface which defines a test written from the perspective of the Admin of the
/// network. This means that the test will have access to the Root account but do not control any
/// of the validators or full nodes running on the network.
pub trait AdminTest: Test {
    /// Executes the test against the given context.
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()>;
}

#[derive(Debug)]
pub struct AdminContext<'t> {
    core: CoreContext,

    chain_info: ChainInfo<'t>,
}

impl<'t> AdminContext<'t> {
    pub fn new(core: CoreContext, chain_info: ChainInfo<'t>) -> Self {
        Self { core, chain_info }
    }

    pub fn core(&self) -> &CoreContext {
        &self.core
    }

    pub fn rng(&mut self) -> &mut ::rand::rngs::StdRng {
        self.core.rng()
    }

    pub fn client(&self) -> BlockingClient {
        BlockingClient::new(&self.chain_info.json_rpc_url)
    }

    pub fn chain_info(&mut self) -> &mut ChainInfo<'t> {
        &mut self.chain_info
    }

    pub fn random_account(&mut self) -> LocalAccount {
        LocalAccount::generate(self.core.rng())
    }
}

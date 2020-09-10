// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    data_cache::{RemoteCache, TransactionDataCache},
    runtime::VMRuntime,
};
use move_core_types::{
    account_address::AccountAddress,
    effects::{ChangeSet, Event},
    identifier::IdentStr,
    language_storage::{ModuleId, TypeTag},
    vm_status::VMStatus,
};
use move_vm_types::{gas_schedule::CostStrategy, values::Value};
use vm::errors::*;

pub struct Session<'r, 'l, R> {
    pub(crate) runtime: &'l VMRuntime,
    pub(crate) data_cache: TransactionDataCache<'r, 'l, R>,
}

impl<'r, 'l, R: RemoteCache> Session<'r, 'l, R> {
    pub fn execute_function<F: FnOnce(VMStatus) -> VMStatus>(
        &mut self,
        module: &ModuleId,
        function_name: &IdentStr,
        ty_args: Vec<TypeTag>,
        args: Vec<Value>,
        _sender: AccountAddress,
        cost_strategy: &mut CostStrategy,
        error_specializer: F,
    ) -> Result<(), VMStatus> {
        self.runtime
            .execute_function(
                module,
                function_name,
                ty_args,
                args,
                &mut self.data_cache,
                cost_strategy,
            )
            .map_err(|e| error_specializer(e.into_vm_status()))
    }

    pub fn execute_script(
        &mut self,
        script: Vec<u8>,
        ty_args: Vec<TypeTag>,
        args: Vec<Value>,
        senders: Vec<AccountAddress>,
        cost_strategy: &mut CostStrategy,
    ) -> VMResult<()> {
        self.runtime.execute_script(
            script,
            ty_args,
            args,
            senders,
            &mut self.data_cache,
            cost_strategy,
        )
    }

    pub fn publish_module(
        &mut self,
        module: Vec<u8>,
        sender: AccountAddress,
        cost_strategy: &mut CostStrategy,
    ) -> VMResult<()> {
        self.runtime
            .publish_module(module, sender, &mut self.data_cache, cost_strategy)
    }

    pub fn num_mutated_accounts(&self) -> u64 {
        self.data_cache.num_mutated_accounts()
    }

    pub fn finish(self) -> VMResult<(ChangeSet, Vec<Event>)> {
        self.data_cache
            .into_effects()
            .map_err(|e| e.finish(Location::Undefined))
    }
}

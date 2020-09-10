// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{data_cache::RemoteCache, move_vm::MoveVM};
use move_core_types::{
    account_address::AccountAddress,
    gas_schedule::{GasAlgebra, GasUnits},
    identifier::{IdentStr, Identifier},
    language_storage::{ModuleId, StructTag},
};
use move_lang::{compiled_unit::CompiledUnit, shared::Address};
use move_vm_types::gas_schedule::{zero_cost_schedule, CostStrategy};
use std::{collections::HashMap, path::PathBuf, sync::Arc, thread};
use vm::{
    errors::{PartialVMResult, VMResult},
    CompiledModule,
};

const WORKING_ACCOUNT: AccountAddress =
    AccountAddress::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);

struct Adapter {
    store: DataStore,
    vm: Arc<MoveVM>,
    functions: Vec<(ModuleId, Identifier)>,
}

impl Adapter {
    fn new(store: DataStore) -> Self {
        let mut functions = vec![];
        functions.push((
            ModuleId::new(WORKING_ACCOUNT, Identifier::new("A").unwrap()),
            Identifier::new("entry_a").unwrap(),
        ));
        functions.push((
            ModuleId::new(WORKING_ACCOUNT, Identifier::new("D").unwrap()),
            Identifier::new("entry_d").unwrap(),
        ));
        functions.push((
            ModuleId::new(WORKING_ACCOUNT, Identifier::new("E").unwrap()),
            Identifier::new("entry_e").unwrap(),
        ));
        functions.push((
            ModuleId::new(WORKING_ACCOUNT, Identifier::new("F").unwrap()),
            Identifier::new("entry_f").unwrap(),
        ));
        functions.push((
            ModuleId::new(WORKING_ACCOUNT, Identifier::new("C").unwrap()),
            Identifier::new("just_c").unwrap(),
        ));
        Self {
            store,
            vm: Arc::new(MoveVM::new()),
            functions,
        }
    }

    fn publish_modules(&mut self, modules: Vec<CompiledModule>) {
        let mut session = self.vm.new_session(&self.store);
        let cost_table = zero_cost_schedule();
        let mut cost_strategy = CostStrategy::system(&cost_table, GasUnits::new(0));
        for module in modules {
            let mut binary = vec![];
            module
                .serialize(&mut binary)
                .unwrap_or_else(|_| panic!("failure in module serialization: {:#?}", module));
            session
                .publish_module(binary, WORKING_ACCOUNT, &mut cost_strategy)
                .unwrap_or_else(|_| panic!("failure publishing module: {:#?}", module));
        }
        let (changeset, _events) = session.finish().expect("failure getting write set");
        for (addr, account_changeset) in changeset.accounts {
            for (name, module) in account_changeset.modules {
                self.store
                    .add_module(ModuleId::new(addr, name), module.unwrap());
            }
        }
    }

    fn call_functions(&self) {
        for (module_id, name) in &self.functions {
            self.call_function(module_id, name);
        }
    }

    fn call_functions_async(&self, reps: usize) {
        let mut children = vec![];
        for _ in 0..reps {
            for (module_id, name) in self.functions.clone() {
                let vm = self.vm.clone();
                let data_store = self.store.clone();
                children.push(thread::spawn(move || {
                    let cost_table = zero_cost_schedule();
                    let mut cost_strategy = CostStrategy::system(&cost_table, GasUnits::new(0));
                    let mut session = vm.new_session(&data_store);
                    session
                        .execute_function(
                            &module_id,
                            &name,
                            vec![],
                            vec![],
                            WORKING_ACCOUNT,
                            &mut cost_strategy,
                            |e| e,
                        )
                        .unwrap_or_else(|_| {
                            panic!("Failure executing {:?}::{:?}", module_id, name)
                        });
                }));
            }
        }
        for child in children {
            let _ = child.join();
        }
    }

    fn call_function(&self, module: &ModuleId, name: &IdentStr) {
        let cost_table = zero_cost_schedule();
        let mut cost_strategy = CostStrategy::system(&cost_table, GasUnits::new(0));
        let mut session = self.vm.new_session(&self.store);
        session
            .execute_function(
                module,
                name,
                vec![],
                vec![],
                WORKING_ACCOUNT,
                &mut cost_strategy,
                |e| e,
            )
            .unwrap_or_else(|_| panic!("Failure executing {:?}::{:?}", module, name));
    }
}

#[derive(Clone, Debug)]
struct DataStore {
    modules: HashMap<ModuleId, Vec<u8>>,
}

impl DataStore {
    fn empty() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    fn add_module(&mut self, module_id: ModuleId, binary: Vec<u8>) {
        self.modules.insert(module_id, binary);
    }
}

impl RemoteCache for DataStore {
    fn get_module(&self, module_id: &ModuleId) -> VMResult<Option<Vec<u8>>> {
        match self.modules.get(module_id) {
            None => Ok(None),
            Some(binary) => Ok(Some(binary.clone())),
        }
    }

    fn get_resource(
        &self,
        _address: &AccountAddress,
        _tag: &StructTag,
    ) -> PartialVMResult<Option<Vec<u8>>> {
        Ok(None)
    }
}

fn compile_file(addr: &[u8; AccountAddress::LENGTH]) -> Vec<CompiledModule> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("src/unit_tests/modules.move");
    let s = path.to_str().expect("no path specified").to_owned();
    let (_, modules) =
        move_lang::move_compile(&[s], &[], Some(Address::new(*addr))).expect("Error compiling...");

    let mut compiled_modules = vec![];
    for module in modules {
        match module {
            CompiledUnit::Module { module, .. } => compiled_modules.push(module),
            CompiledUnit::Script { .. } => (),
        }
    }
    compiled_modules
}

#[test]
fn load() {
    let data_store = DataStore::empty();
    let mut adapter = Adapter::new(data_store);
    let modules = compile_file(&[0; 16]);
    adapter.publish_modules(modules);
    // calls all functions sequentially
    adapter.call_functions();
}

#[test]
fn load_concurrent() {
    let data_store = DataStore::empty();
    let mut adapter = Adapter::new(data_store);
    let modules = compile_file(&[0; 16]);
    adapter.publish_modules(modules);
    // makes 15 threads
    adapter.call_functions_async(3);
}

#[test]
fn load_concurrent_many() {
    let data_store = DataStore::empty();
    let mut adapter = Adapter::new(data_store);
    let modules = compile_file(&[0; 16]);
    adapter.publish_modules(modules);
    // makes 150 threads
    adapter.call_functions_async(30);
}

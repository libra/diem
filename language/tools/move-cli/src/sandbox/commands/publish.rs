// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::sandbox::utils::{
    explain_publish_changeset, explain_publish_error, get_gas_status,
    on_disk_state_view::OnDiskStateView,
};
use move_core_types::identifier::IdentStr;
use move_lang::{self, compiled_unit::CompiledUnit, Compiler, Flags};
use move_vm_runtime::{logging::NoContextLog, move_vm::MoveVM};

use anyhow::{bail, Result};
use std::collections::BTreeMap;

pub fn publish(
    state: &OnDiskStateView,
    files: &[String],
    republish: bool,
    ignore_breaking_changes: bool,
    override_ordering: Option<&[String]>,
    verbose: bool,
) -> Result<()> {
    if verbose {
        println!("Compiling Move modules...")
    }

    let (_, compiled_units) = Compiler::new(files, &[state.interface_files_dir()?])
        .set_flags(Flags::empty().set_sources_shadow_deps(republish))
        .build_and_report()?;

    let num_modules = compiled_units
        .iter()
        .filter(|u| matches!(u, CompiledUnit::Module { .. }))
        .count();
    if verbose {
        println!("Found and compiled {} modules", num_modules)
    }

    let mut modules = vec![];
    for c in compiled_units {
        match c {
            CompiledUnit::Script { loc, .. } => {
                if verbose {
                    println!(
                        "Warning: Found script in specified files for publishing. But scripts \
                         cannot be published. Script found in: {}",
                        loc.file()
                    )
                }
            }
            CompiledUnit::Module { module, .. } => modules.push(module),
        }
    }

    // use the the publish_module API frm the VM if we do not allow breaking changes
    if !ignore_breaking_changes {
        let vm = MoveVM::new(diem_vm::natives::diem_natives()).unwrap();
        let mut gas_status = get_gas_status(None)?;
        let log_context = NoContextLog::new();
        let mut session = vm.new_session(state);

        let mut has_error = false;
        match override_ordering {
            None => {
                for module in &modules {
                    let mut module_bytes = vec![];
                    module.serialize(&mut module_bytes)?;

                    let id = module.self_id();
                    let sender = *id.address();

                    let res =
                        session.publish_module(module_bytes, sender, &mut gas_status, &log_context);
                    if let Err(err) = res {
                        explain_publish_error(err, &state, module)?;
                        has_error = true;
                        break;
                    }
                }
            }
            Some(ordering) => {
                let module_map: BTreeMap<_, _> = modules
                    .into_iter()
                    .map(|m| (m.self_id().name().to_owned(), m))
                    .collect();

                let mut sender_opt = None;
                let mut module_bytes_vec = vec![];
                for name_str in ordering {
                    let name = IdentStr::new(name_str)?;
                    match module_map.get(name) {
                        None => bail!("Invalid module name in publish ordering: {}", name),
                        Some(module) => {
                            let mut module_bytes = vec![];
                            module.serialize(&mut module_bytes)?;
                            module_bytes_vec.push(module_bytes);
                            if sender_opt.is_none() {
                                sender_opt = Some(*module.self_id().address());
                            }
                        }
                    }
                }

                match sender_opt {
                    None => bail!("No modules to publish"),
                    Some(sender) => {
                        let res = session.publish_module_bundle(
                            module_bytes_vec,
                            sender,
                            &mut gas_status,
                            &log_context,
                        );
                        if let Err(err) = res {
                            // TODO (mengxu): explain publish errors in multi-module publishing
                            println!("Invalid multi-module publishing: {}", err);
                            has_error = true;
                        }
                    }
                }
            }
        }

        if !has_error {
            let (changeset, events) = session.finish().map_err(|e| e.into_vm_status())?;
            assert!(events.is_empty());
            if verbose {
                explain_publish_changeset(&changeset, &state);
            }
            let modules: Vec<_> = changeset
                .into_modules()
                .map(|(module_id, blob_opt)| (module_id, blob_opt.expect("must be non-deletion")))
                .collect();
            state.save_modules(&modules)?;
        }
    } else {
        // NOTE: the VM enforces the most strict way of module republishing and does not allow
        // backward incompatible changes, as as result, if this flag is set, we skip the VM process
        // and force the CLI to override the on-disk state directly
        let mut serialized_modules = vec![];
        for module in modules {
            let mut module_bytes = vec![];
            module.serialize(&mut module_bytes)?;
            serialized_modules.push((module.self_id(), module_bytes));
        }
        state.save_modules(&serialized_modules)?;
    }

    Ok(())
}

// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, format_err, Result};
use diem_types::{
    account_address::AccountAddress,
    chain_id::ChainId,
    transaction::{Transaction, WriteSetPayload},
};

use compiled_stdlib::stdlib_modules as stdlib;
use diem_writeset_generator::{
    create_release, encode_custom_script, encode_halt_network_payload,
    encode_remove_validators_payload, verify_release,
};
use std::path::PathBuf;
use structopt::StructOpt;
use vm::CompiledModule;

const GENESIS_MODULE_NAME: &str = "Genesis";

#[derive(Debug, StructOpt)]
struct Opt {
    /// Path to the output serialized bytes
    #[structopt(long, short, parse(from_os_str))]
    output: Option<PathBuf>,
    /// Output as serialized WriteSet payload. Set this flag if this payload is submitted to AOS portal.
    #[structopt(long)]
    output_payload: bool,
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    /// List of addresses to remove from validator set
    #[structopt(name = "remove-validators")]
    RemoveValidators { addresses: Vec<AccountAddress> },
    /// Block the execution of any transaction in the network
    #[structopt(name = "halt-network")]
    HaltNetwork,
    /// Build a custom file in templates into admin script
    #[structopt(name = "build-custom-script")]
    BuildCustomScript {
        script_name: String,
        args: String,
        execute_as: Option<AccountAddress>,
    },
    /// Create a release writeset by comparing local Diem Framework against a remote blockchain state.
    #[structopt(name = "create-release")]
    CreateDiemFrameworkRelease {
        /// ChainID to distinguish the diem network. e.g: PREMAINNET
        chain_id: ChainId,
        /// Public JSON-rpc endpoint URL.
        // TODO: Get rid of this URL argument once we have a stable mapping from ChainId to its url.
        url: String,
        /// Blockchain height
        version: u64,
        /// Set the flag to true in the first release. This will manually create the first release artifact on disk.
        #[structopt(long)]
        first_release: bool,
    },
    /// Verify if a blob is generated by the checked-in artifact.
    #[structopt(name = "verify-release")]
    VerifyDiemFrameworkRelease {
        /// ChainID to distinguish the diem network. e.g: PREMAINNET
        chain_id: ChainId,
        /// Public JSON-rpc endpoint URL.
        url: String,
        /// Path to the serialized bytes of WriteSet.
        #[structopt(parse(from_os_str))]
        writeset_path: PathBuf,
    },
}

fn save_bytes(bytes: Vec<u8>, path: PathBuf) -> Result<()> {
    std::fs::write(path.as_path(), bytes.as_slice())
        .map_err(|_| format_err!("Unable to write to path"))
}

fn stdlib_modules() -> Vec<(Vec<u8>, CompiledModule)> {
    // Need to filter out Genesis module similiar to what is done in vmgenesis to make sure Genesis
    // module isn't published on-chain.
    stdlib()
        .bytes_and_modules()
        .iter()
        .filter(|(_bytes, module)| module.self_id().name().as_str() != GENESIS_MODULE_NAME)
        .map(|(bytes, module)| (bytes.clone(), module.clone()))
        .collect()
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    let payload = match opt.cmd {
        Command::RemoveValidators { addresses } => encode_remove_validators_payload(addresses),
        Command::HaltNetwork => encode_halt_network_payload(),
        Command::BuildCustomScript {
            script_name,
            args,
            execute_as,
        } => encode_custom_script(
            &script_name,
            &serde_json::from_str::<serde_json::Value>(args.as_str())?,
            execute_as,
        ),
        Command::CreateDiemFrameworkRelease {
            chain_id,
            url,
            version,
            first_release,
        } => {
            let release_modules = stdlib_modules();
            create_release(chain_id, url, version, first_release, &release_modules)?
        }
        Command::VerifyDiemFrameworkRelease {
            url,
            chain_id,
            writeset_path,
        } => {
            let writeset_payload = {
                let raw_bytes = std::fs::read(writeset_path.as_path()).unwrap();
                bcs::from_bytes::<WriteSetPayload>(raw_bytes.as_slice()).or_else(|_| {
                    let txn: Transaction = bcs::from_bytes(raw_bytes.as_slice())?;
                    match txn {
                        Transaction::GenesisTransaction(ws) => Ok(ws),
                        _ => bail!("Unexpected transacton type"),
                    }
                })?
            };
            let release_modules = stdlib_modules();
            verify_release(chain_id, url, &writeset_payload, &release_modules)?;
            return Ok(());
        }
    };
    let output_path = if let Some(p) = opt.output {
        p
    } else {
        bail!("Empty output path provided");
    };
    if opt.output_payload {
        save_bytes(
            bcs::to_bytes(&payload).map_err(|_| format_err!("Transaction Serialize Error"))?,
            output_path,
        )
    } else {
        save_bytes(
            bcs::to_bytes(&Transaction::GenesisTransaction(payload))
                .map_err(|_| format_err!("Transaction Serialize Error"))?,
            output_path,
        )
    }
}

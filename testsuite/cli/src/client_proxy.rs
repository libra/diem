// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    commands::{is_address, is_authentication_key},
    diem_client::DiemClient,
    AccountData, AccountStatus,
};
use anyhow::{bail, ensure, format_err, Error, Result};
use compiler::Compiler;
use diem_client::{stream::WebsocketClient, views, WaitForTransactionError};
use diem_crypto::{
    ed25519::{Ed25519PrivateKey, Ed25519PublicKey, Ed25519Signature},
    test_utils::KeyPair,
};
use diem_logger::prelude::{error, info};
use diem_resource_viewer::{AnnotatedAccountStateBlob, DiemValueAnnotator};
use diem_temppath::TempPath;
use diem_transaction_builder::stdlib as transaction_builder;
use diem_types::{
    access_path::AccessPath,
    account_address::AccountAddress,
    account_config::{
        diem_root_address, from_currency_code_string, testnet_dd_account_address,
        treasury_compliance_account_address, type_tag_for_currency_code, XDX_NAME, XUS_NAME,
    },
    account_state::AccountState,
    chain_id::ChainId,
    ledger_info::LedgerInfoWithSignatures,
    transaction::{
        authenticator::AuthenticationKey,
        helpers::{create_unsigned_txn, create_user_txn, TransactionSigner},
        parse_transaction_argument, ChangeSet, Module, RawTransaction, Script, SignedTransaction,
        TransactionArgument, TransactionPayload, Version, WriteSetPayload,
    },
    waypoint::Waypoint,
    write_set::{WriteOp, WriteSetMut},
};
use diem_wallet::{io_utils, WalletLibrary};
use move_vm_test_utils::InMemoryStorage;
use num_traits::{
    cast::{FromPrimitive, ToPrimitive},
    identities::Zero,
};
use reqwest::Url;
use rust_decimal::Decimal;
use std::{
    collections::HashMap,
    convert::TryFrom,
    fmt, fs,
    io::{stdout, Write},
    path::{Path, PathBuf},
    process::Command,
    str::{self, FromStr},
    time,
};

const CLIENT_WALLET_MNEMONIC_FILE: &str = "client.mnemonic";
const GAS_UNIT_PRICE: u64 = 0;
const MAX_GAS_AMOUNT: u64 = 1_000_000;
const TX_EXPIRATION: i64 = 100;
const DEFAULT_WAIT_TIMEOUT: time::Duration = time::Duration::from_secs(60);

/// Enum used for error formatting.
#[derive(Debug)]
enum InputType {
    Bool,
    UnsignedInt,
    Usize,
}

/// Account data is stored in a map and referenced by an index.
#[derive(Debug)]
pub struct AddressAndIndex {
    /// Address of the account.
    pub address: AccountAddress,
    /// The account_ref_id of this account in client.
    pub index: usize,
}

/// Account is represented either as an entry into accounts vector or as an address.
pub enum AccountEntry {
    /// Index into client.accounts
    Index(usize),
    /// Address of the account
    Address(AccountAddress),
}

/// Used to return the sequence and sender account index submitted for a transfer
pub struct IndexAndSequence {
    /// Index/key of the account in TestClient::accounts vector.
    pub account_index: AccountEntry,
    /// Sequence number of the account.
    pub sequence_number: u64,
}

/// Proxy handling CLI commands/inputs.
pub struct ClientProxy {
    /// chain ID of the Diem network this client is interacting with
    pub chain_id: ChainId,
    /// client for admission control interface.
    pub client: DiemClient,
    /// Created accounts.
    pub accounts: Vec<AccountData>,
    /// Address to account_ref_id map.
    address_to_ref_id: HashMap<AccountAddress, usize>,
    /// Host that operates a faucet service
    faucet_url: Url,
    /// Account used for Diem Root operations (e.g., adding a new transaction script)
    pub diem_root_account: Option<AccountData>,
    /// Account used for Treasury Compliance operations
    pub tc_account: Option<AccountData>,
    /// Account used for "minting" operations
    pub testnet_designated_dealer_account: Option<AccountData>,
    /// do not print '.' when waiting for signed transaction
    pub quiet_wait: bool,
    /// Wallet library managing user accounts.
    pub wallet: WalletLibrary,
    /// Whether to sync with validator on wallet recovery.
    sync_on_wallet_recovery: bool,
    /// temp files (alive for duration of program)
    temp_files: Vec<PathBuf>,
    /// Host of the node that client connects to
    url: Url,
}

impl ClientProxy {
    /// Construct a new TestClient.
    pub fn new(
        chain_id: ChainId,
        url: &str,
        diem_root_account_file: &str,
        tc_account_file: &str,
        testnet_designated_dealer_account_file: &str,
        sync_on_wallet_recovery: bool,
        faucet_url: Option<String>,
        mnemonic_file: Option<String>,
        waypoint: Waypoint,
        quiet_wait: bool,
    ) -> Result<Self> {
        // fail fast if url is not valid
        let url = Url::parse(url)?;
        let client = DiemClient::new(url.clone(), waypoint)?;

        let accounts = vec![];

        let diem_root_account = if diem_root_account_file.is_empty() {
            None
        } else {
            let diem_root_account_key = generate_key::load_key(diem_root_account_file);
            let diem_root_account_data = Self::get_account_data_from_address(
                &client,
                diem_root_address(),
                true,
                Some(KeyPair::from(diem_root_account_key)),
                None,
            )?;
            Some(diem_root_account_data)
        };

        let tc_account = if tc_account_file.is_empty() {
            None
        } else {
            let tc_account_key = generate_key::load_key(tc_account_file);
            let tc_account_data = Self::get_account_data_from_address(
                &client,
                treasury_compliance_account_address(),
                true,
                Some(KeyPair::from(tc_account_key)),
                None,
            )?;
            Some(tc_account_data)
        };

        let dd_account = if testnet_designated_dealer_account_file.is_empty() {
            None
        } else {
            let dd_account_key = generate_key::load_key(testnet_designated_dealer_account_file);
            let dd_account_data = Self::get_account_data_from_address(
                &client,
                testnet_dd_account_address(),
                true,
                Some(KeyPair::from(dd_account_key)),
                None,
            )?;
            Some(dd_account_data)
        };

        let faucet_url = if let Some(faucet_url) = &faucet_url {
            Url::parse(faucet_url).expect("Invalid faucet URL specified")
        } else {
            url.join("/mint")
                .expect("Failed to construct faucet URL from JSON-RPC URL")
        };

        let address_to_ref_id = accounts
            .iter()
            .enumerate()
            .map(|(ref_id, acc_data): (usize, &AccountData)| (acc_data.address, ref_id))
            .collect::<HashMap<AccountAddress, usize>>();

        Ok(ClientProxy {
            chain_id,
            client,
            accounts,
            address_to_ref_id,
            faucet_url,
            diem_root_account,
            tc_account,
            testnet_designated_dealer_account: dd_account,
            wallet: Self::get_diem_wallet(mnemonic_file)?,
            sync_on_wallet_recovery,
            temp_files: vec![],
            quiet_wait,
            url,
        })
    }

    /// Gets a websocket client for the same node `DiemClient` connects to
    pub async fn websocket_client(&self) -> Result<WebsocketClient, diem_client::Error> {
        let ws_url = format!(
            "ws://{}:{}/v1/stream/ws",
            self.url.host().unwrap(),
            self.url.port().unwrap()
        );
        println!("ws_url: {}", &ws_url);
        WebsocketClient::new(&ws_url, None).await
    }

    /// Gets account data for the indexed address
    pub fn get_account(&self, address_num: usize) -> Option<&AccountData> {
        self.accounts.get(address_num)
    }

    fn get_account_data_and_id(&self, address: &AccountAddress) -> Result<(usize, &AccountData)> {
        for (index, acc) in self.accounts.iter().enumerate() {
            if &acc.address == address {
                return Ok((index, acc));
            }
        }
        bail!(
            "Unable to find existing managing account by address: {}, to see all existing \
                     accounts, run: 'account list'",
            address
        )
    }

    fn get_account_data(&self, address: &AccountAddress) -> Result<&AccountData> {
        if let Some(account) = &self.diem_root_account {
            if &account.address == address {
                return Ok(account);
            }
        }

        if let Some(account) = &self.tc_account {
            if &account.address == address {
                return Ok(account);
            }
        }

        self.get_account_data_and_id(address).map(|(_, data)| data)
    }

    /// Returns the account index that should be used by user to reference this account
    pub fn create_next_account(&mut self, sync_with_validator: bool) -> Result<AddressAndIndex> {
        let (auth_key, _) = self.wallet.new_address()?;
        let account_data = Self::get_account_data_from_address(
            &self.client,
            auth_key.derived_address(),
            sync_with_validator,
            None,
            Some(auth_key.to_vec()),
        )?;

        Ok(self.insert_account_data(account_data))
    }

    /// Returns the ledger info corresonding to the latest epoch change
    /// (could further be used for e.g., generating a waypoint)
    pub fn latest_epoch_change_li(&mut self) -> Option<&LedgerInfoWithSignatures> {
        self.client.update_and_verify_state_proof().unwrap();
        self.client.latest_epoch_change_li()
    }

    /// Print index and address of all accounts.
    pub fn print_all_accounts(&self) {
        if self.accounts.is_empty() {
            println!("No user accounts");
        } else {
            for (ref index, ref account) in self.accounts.iter().enumerate() {
                println!(
                    "User account index: {}, address: {}, private_key: {:?}, sequence number: {}, status: {:?}",
                    index,
                    hex::encode(&account.address),
                    hex::encode(&self.wallet.get_private_key(&account.address).unwrap().to_bytes()),
                    account.sequence_number,
                    account.status,
                );
            }
        }

        if let Some(diem_root_account) = &self.diem_root_account {
            println!(
                "AssocRoot account address: {}, sequence_number: {}, status: {:?}",
                hex::encode(&diem_root_account.address),
                diem_root_account.sequence_number,
                diem_root_account.status,
            );
        }
        if let Some(tc_account) = &self.tc_account {
            println!(
                "TC account address: {}, sequence_number: {}, status: {:?}",
                hex::encode(&tc_account.address),
                tc_account.sequence_number,
                tc_account.status,
            );
        }
        if let Some(testnet_dd_account) = &self.testnet_designated_dealer_account {
            println!(
                "Testnet DD account address: {}, sequence_number: {}, status: {:?}",
                hex::encode(&testnet_dd_account.address),
                testnet_dd_account.sequence_number,
                testnet_dd_account.status,
            );
        }
    }

    /// Clone all accounts held in the client.
    #[cfg(any(test, feature = "fuzzing"))]
    pub fn copy_all_accounts(&self) -> Vec<AccountData> {
        self.accounts.clone()
    }

    /// Set the account of this client instance.
    pub fn set_accounts(&mut self, accounts: Vec<AccountData>) -> Vec<AddressAndIndex> {
        self.accounts.clear();
        self.address_to_ref_id.clear();
        let mut ret = vec![];
        for data in accounts {
            ret.push(self.insert_account_data(data));
        }
        ret
    }

    /// Get balance from validator for the account specified.
    pub fn get_balances(&mut self, space_delim_strings: &[&str]) -> Result<Vec<String>> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments for getting balances"
        );
        let (address, _) = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let currency_info: HashMap<_, _> = self
            .client
            .get_currency_info()?
            .into_iter()
            .map(|view| (view.code.clone(), view))
            .collect();
        let account = self.get_account_resource_and_update(&address)?;
        account
            .balances
            .iter()
            .map(|amt_view| {
                let info = currency_info.get(&amt_view.currency).ok_or_else(|| {
                    format_err!(
                        "Unable to get currency info for balance {}",
                        amt_view.currency
                    )
                })?;

                let whole_num = amt_view
                    .amount
                    .checked_div(info.scaling_factor)
                    .ok_or_else(|| {
                        format_err!(
                            "checked_div failed, amount {}, scaling_factor: {}",
                            amt_view.amount,
                            info.scaling_factor
                        )
                    })?;
                let remainder = amt_view
                    .amount
                    .checked_rem(info.scaling_factor)
                    .ok_or_else(|| {
                        format_err!(
                            "checked_rem failed, amount {}, scaling_factor: {}",
                            amt_view.amount,
                            info.scaling_factor
                        )
                    })?;

                Ok(format!(
                    "{}.{:0>6}{}",
                    whole_num.to_string(),
                    remainder.to_string(),
                    amt_view.currency
                ))
            })
            .collect()
    }

    /// Get the latest sequence number from validator for the account specified.
    pub fn get_sequence_number(&mut self, space_delim_strings: &[&str]) -> Result<u64> {
        ensure!(
            space_delim_strings.len() == 2 || space_delim_strings.len() == 3,
            "Invalid number of arguments for getting sequence number"
        );
        let (address, _) = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let sequence_number = self
            .get_account_resource_and_update(&address)?
            .sequence_number;

        let reset_sequence_number = if space_delim_strings.len() == 3 {
            parse_bool(space_delim_strings[2]).map_err(|error| {
                format_parse_data_error(
                    "reset_sequence_number",
                    InputType::Bool,
                    space_delim_strings[2],
                    error,
                )
            })?
        } else {
            false
        };
        if reset_sequence_number {
            self.update_account_seq(&address, sequence_number);
        }

        Ok(sequence_number)
    }

    /// Adds a currency to the sending account. Fails if that currency already exists.
    pub fn add_currency(&mut self, space_delim_strings: &[&str], is_blocking: bool) -> Result<()> {
        ensure!(
            space_delim_strings.len() >= 3 && space_delim_strings.len() <= 6,
            "Invalid number of arguments for adding currency"
        );

        let (sender_address, _) =
            self.get_account_address_from_parameter(space_delim_strings[1])?;

        let sender = self.get_account_data(&sender_address)?;

        let currency_to_add = space_delim_strings[2];
        let currency_code = from_currency_code_string(currency_to_add).map_err(|_| {
            format_err!(
                "Invalid currency code {} provided to add currency",
                currency_to_add
            )
        })?;

        let gas_unit_price = if space_delim_strings.len() > 3 {
            Some(space_delim_strings[3].parse::<u64>().map_err(|error| {
                format_parse_data_error(
                    "gas_unit_price",
                    InputType::UnsignedInt,
                    space_delim_strings[3],
                    error,
                )
            })?)
        } else {
            None
        };

        let max_gas_amount = if space_delim_strings.len() > 4 {
            Some(space_delim_strings[4].parse::<u64>().map_err(|error| {
                format_parse_data_error(
                    "max_gas_amount",
                    InputType::UnsignedInt,
                    space_delim_strings[4],
                    error,
                )
            })?)
        } else {
            None
        };

        let gas_currency_code = if space_delim_strings.len() > 5 {
            Some(space_delim_strings[5].to_owned())
        } else {
            None
        };

        let program = transaction_builder::encode_add_currency_to_account_script(
            type_tag_for_currency_code(currency_code),
        );

        let txn = self.create_txn_to_submit(
            TransactionPayload::Script(program),
            &sender,
            max_gas_amount,    /* max_gas_amount */
            gas_unit_price,    /* gas_unit_price */
            gas_currency_code, /* gas_currency_code */
        )?;

        self.submit_and_wait(&txn, is_blocking)?;

        Ok(())
    }

    /// Mints coins for the receiver specified.
    pub fn mint_coins(&mut self, space_delim_strings: &[&str], is_blocking: bool) -> Result<()> {
        ensure!(
            space_delim_strings.len() >= 4 && space_delim_strings.len() <= 5,
            "Invalid number of arguments for mint"
        );
        let (receiver, receiver_auth_key_opt) =
            self.get_account_address_from_parameter(space_delim_strings[1])?;
        let receiver_auth_key = receiver_auth_key_opt.ok_or_else(|| {
            format_err!("Need authentication key to create new account via minting from facuet")
        })?;
        let mint_currency = space_delim_strings[3];
        let use_base_units = space_delim_strings
            .get(4)
            .map(|s| s == &"use_base_units")
            .unwrap_or(false);
        let num_coins = if !use_base_units {
            self.convert_to_on_chain_representation(space_delim_strings[2], mint_currency)?
        } else {
            Self::convert_to_scaled_representation(space_delim_strings[2], 1, 1)?
        };
        let currency_code = from_currency_code_string(mint_currency)
            .map_err(|_| format_err!("Invalid currency code {} provided to mint", mint_currency))?;

        ensure!(
            num_coins > 0,
            "Invalid number of coins to transfer from faucet."
        );

        if self.tc_account.is_some() {
            let script = transaction_builder::encode_create_parent_vasp_account_script(
                type_tag_for_currency_code(currency_code.clone()),
                0,
                receiver,
                receiver_auth_key.prefix().to_vec(),
                b"testnet".to_vec(),
                false, /* add all currencies */
            );
            // If the receiver is local, create it now.
            if let Some(pos) = self
                .accounts
                .iter()
                .position(|account_data| account_data.address == receiver)
            {
                let status = &self.accounts.get(pos).unwrap().status;
                if &AccountStatus::Local == status {
                    println!(">> Creating recipient account before minting from faucet");
                    // This needs to be blocking since the mint can't happen until it completes
                    self.association_transaction_with_local_tc_account(
                        TransactionPayload::Script(script),
                        true,
                    )?;
                    self.accounts.get_mut(pos).unwrap().status = AccountStatus::Persisted;
                }
            } else {
                // We can't determine the account state. So try and create the account, but
                // if it already exists don't error.
                let _result = self.association_transaction_with_local_tc_account(
                    TransactionPayload::Script(script),
                    true,
                );
            } // else, the account has already been created -- do nothing
        }

        println!(">> Sending coins from faucet");
        match self.testnet_designated_dealer_account {
            Some(_) => {
                let script = transaction_builder::encode_peer_to_peer_with_metadata_script(
                    type_tag_for_currency_code(currency_code),
                    receiver,
                    num_coins,
                    vec![],
                    vec![],
                );
                self.association_transaction_with_local_testnet_dd_account(
                    TransactionPayload::Script(script),
                    is_blocking,
                )
            }
            None => self.mint_coins_with_faucet_service(
                receiver_auth_key,
                num_coins,
                mint_currency.to_owned(),
            ),
        }
    }

    /// Allow executing arbitrary script in the network.
    pub fn enable_custom_script(
        &mut self,
        space_delim_strings: &[&str],
        open_module: bool,
        is_blocking: bool,
    ) -> Result<()> {
        ensure!(
            space_delim_strings[0] == "enable_custom_script" || space_delim_strings[0] == "s",
            "inconsistent command '{}' for enable_custom_script",
            space_delim_strings[0]
        );
        ensure!(
            space_delim_strings.len() == 1,
            "Invalid number of arguments for setting publishing option"
        );
        let script_body = {
            let code = format!(
                "
                import 0x1.DiemTransactionPublishingOption;

                main(account: signer) {{
                    DiemTransactionPublishingOption.set_open_script(&account);
                    {}

                    return;
                }}
            ",
                if open_module {
                    "DiemTransactionPublishingOption.set_open_module(&account, true);"
                } else {
                    ""
                }
            );

            let compiler = Compiler {
                address: diem_types::account_config::CORE_CODE_ADDRESS,
                deps: diem_framework_releases::current_modules().iter().collect(),
            };
            compiler
                .into_script_blob("file_name", &code)
                .expect("Failed to compile")
        };
        match self.diem_root_account {
            Some(_) => self.association_transaction_with_local_diem_root_account(
                TransactionPayload::Script(Script::new(script_body, vec![], vec![])),
                is_blocking,
            ),
            None => unimplemented!(),
        }
    }

    /// Modify the stored DiemVersion on chain.
    pub fn change_diem_version(
        &mut self,
        space_delim_strings: &[&str],
        is_blocking: bool,
    ) -> Result<()> {
        ensure!(
            space_delim_strings[0] == "change_diem_version" || space_delim_strings[0] == "v",
            "inconsistent command '{}' for change_diem_version",
            space_delim_strings[0]
        );
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments for changing diem_version"
        );
        match self.diem_root_account {
            Some(_) => self.association_transaction_with_local_diem_root_account(
                TransactionPayload::Script(transaction_builder::encode_update_diem_version_script(
                    self.diem_root_account.as_ref().unwrap().sequence_number,
                    space_delim_strings[1].parse::<u64>().unwrap(),
                )),
                is_blocking,
            ),
            None => unimplemented!(),
        }
    }

    /// Only allow executing predefined script in the Move standard library in the network.
    pub fn upgrade_stdlib(
        &mut self,
        space_delim_strings: &[&str],
        is_blocking: bool,
    ) -> Result<()> {
        ensure!(
            space_delim_strings[0] == "upgrade_stdlib" || space_delim_strings[0] == "u",
            "inconsistent command '{}' for upgrade_stdlib",
            space_delim_strings[0]
        );
        ensure!(
            space_delim_strings.len() == 1,
            "Invalid number of arguments for upgrading_stdlib_transaction"
        );

        match self.diem_root_account {
            Some(_) => self.association_transaction_with_local_diem_root_account(
                TransactionPayload::WriteSet(WriteSetPayload::Direct(
                    encode_stdlib_upgrade_transaction(),
                )),
                is_blocking,
            ),
            None => unimplemented!(),
        }
    }

    /// Wait for transaction, this function is not safe for waiting for a specific transaction,
    /// should use wait_for_signed_transaction instead.
    /// TODO: rename to wait_for_account_seq or remove
    pub fn wait_for_transaction(&self, address: AccountAddress, seq: u64) -> Result<()> {
        let start = time::Instant::now();
        while start.elapsed() < DEFAULT_WAIT_TIMEOUT {
            let account_txn = self.client.get_txn_by_acc_seq(&address, seq, false)?;
            if let Some(txn) = account_txn {
                if let views::TransactionDataView::UserTransaction {
                    sequence_number, ..
                } = txn.transaction
                {
                    if sequence_number >= seq {
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(time::Duration::from_millis(10));
        }
        bail!(
            "wait for account(address={}) transaction(seq={}) timeout",
            address,
            seq
        )
    }

    /// Submit transaction and waits for the transaction executed
    pub fn submit_and_wait(&mut self, txn: &SignedTransaction, is_blocking: bool) -> Result<()> {
        self.client.submit_transaction(&txn)?;
        if is_blocking {
            self.wait_for_signed_transaction(txn)?;
        } else {
            let seq = txn
                .sequence_number()
                .checked_add(1)
                .ok_or_else(|| format_err!("seqnum can't reach u64::max"))?;
            self.update_account_seq(&txn.sender(), seq);
        }
        Ok(())
    }

    /// Waits for the transaction
    pub fn wait_for_signed_transaction(
        &mut self,
        txn: &SignedTransaction,
    ) -> Result<views::TransactionView> {
        let (tx, rx) = std::sync::mpsc::channel();
        if !self.quiet_wait {
            let _handler = std::thread::spawn(move || loop {
                if rx.try_recv().is_ok() {
                    break;
                }
                print!(".");
                stdout().flush().unwrap();
                std::thread::sleep(time::Duration::from_millis(10));
            });
        }

        let ret = self.client.wait_for_transaction(txn, DEFAULT_WAIT_TIMEOUT);
        let ac_update = self.get_account_and_update(&txn.sender());

        if !self.quiet_wait {
            tx.send(()).expect("stop waiting thread");
            println!();
        }

        if let Err(err) = ac_update {
            println!("account update failed: {}", err);
        }
        match ret {
            Ok(t) => Ok(t),
            Err(WaitForTransactionError::TransactionExecutionFailed(txn)) => Err(format_err!(
                "transaction failed to execute; status: {:?}!",
                txn.vm_status
            )),
            Err(e) => Err(anyhow::Error::new(e)),
        }
    }

    /// Transfer num_coins from sender account to receiver. If is_blocking = true,
    /// it will keep querying validator till the sequence number is bumped up in validator.
    pub fn transfer_coins_int(
        &mut self,
        sender_address: &AccountAddress,
        receiver_address: &AccountAddress,
        num_coins: u64,
        coin_currency: String,
        gas_unit_price: Option<u64>,
        gas_currency_code: Option<String>,
        max_gas_amount: Option<u64>,
        is_blocking: bool,
    ) -> Result<IndexAndSequence> {
        let currency_code = from_currency_code_string(&coin_currency)
            .map_err(|_| format_err!("Invalid currency code {} specified", coin_currency))?;
        let gas_currency_code = gas_currency_code.or(Some(coin_currency));

        let (sender_account_ref_id, sender) = self.get_account_data_and_id(sender_address)?;
        let program = transaction_builder::encode_peer_to_peer_with_metadata_script(
            type_tag_for_currency_code(currency_code),
            *receiver_address,
            num_coins,
            vec![],
            vec![],
        );
        let txn = self.create_txn_to_submit(
            TransactionPayload::Script(program),
            sender,
            max_gas_amount,    /* max_gas_amount */
            gas_unit_price,    /* gas_unit_price */
            gas_currency_code, /* gas_currency_code */
        )?;
        self.submit_and_wait(&txn, is_blocking)?;

        Ok(IndexAndSequence {
            account_index: AccountEntry::Index(sender_account_ref_id),
            sequence_number: txn.sequence_number(),
        })
    }

    /// Prepare a transfer transaction: return the unsigned raw transaction
    pub fn prepare_transfer_coins(
        &mut self,
        sender_address: AccountAddress,
        sender_sequence_number: u64,
        receiver_address: AccountAddress,
        num_coins: u64,
        coin_currency: String,
        gas_unit_price: Option<u64>,
        max_gas_amount: Option<u64>,
        gas_currency_code: Option<String>,
    ) -> Result<RawTransaction> {
        let currency_code = from_currency_code_string(&coin_currency)
            .map_err(|_| format_err!("Invalid currency code {} specified", coin_currency))?;
        let program = transaction_builder::encode_peer_to_peer_with_metadata_script(
            type_tag_for_currency_code(currency_code),
            receiver_address,
            num_coins,
            vec![],
            vec![],
        );

        Ok(create_unsigned_txn(
            TransactionPayload::Script(program),
            sender_address,
            sender_sequence_number,
            max_gas_amount.unwrap_or(MAX_GAS_AMOUNT),
            gas_unit_price.unwrap_or(GAS_UNIT_PRICE),
            gas_currency_code.unwrap_or_else(|| XUS_NAME.to_owned()),
            TX_EXPIRATION,
            self.chain_id,
        ))
    }

    /// Transfers coins from sender to receiver.
    pub fn transfer_coins(
        &mut self,
        space_delim_strings: &[&str],
        is_blocking: bool,
    ) -> Result<IndexAndSequence> {
        ensure!(
            space_delim_strings.len() >= 5 && space_delim_strings.len() <= 7,
            "Invalid number of arguments for transfer"
        );

        let (sender_account_address, _) =
            self.get_account_address_from_parameter(space_delim_strings[1])?;
        let (receiver_address, _) =
            self.get_account_address_from_parameter(space_delim_strings[2])?;

        let transfer_currency = space_delim_strings[4];
        let num_coins =
            self.convert_to_on_chain_representation(space_delim_strings[3], transfer_currency)?;

        let gas_unit_price = if space_delim_strings.len() > 5 {
            Some(space_delim_strings[5].parse::<u64>().map_err(|error| {
                format_parse_data_error(
                    "gas_unit_price",
                    InputType::UnsignedInt,
                    space_delim_strings[5],
                    error,
                )
            })?)
        } else {
            None
        };

        let max_gas_amount = if space_delim_strings.len() > 6 {
            Some(space_delim_strings[6].parse::<u64>().map_err(|error| {
                format_parse_data_error(
                    "max_gas_amount",
                    InputType::UnsignedInt,
                    space_delim_strings[6],
                    error,
                )
            })?)
        } else {
            None
        };

        let gas_currency = if space_delim_strings.len() > 7 {
            space_delim_strings[7].to_owned()
        } else {
            transfer_currency.to_owned()
        };

        self.transfer_coins_int(
            &sender_account_address,
            &receiver_address,
            num_coins,
            transfer_currency.to_owned(),
            gas_unit_price,
            Some(gas_currency),
            max_gas_amount,
            is_blocking,
        )
    }

    /// Compile Move program
    pub fn compile_program(&mut self, space_delim_strings: &[&str]) -> Result<Vec<String>> {
        ensure!(
            space_delim_strings[0] == "compile" || space_delim_strings[0] == "c",
            "inconsistent command '{}' for compile_program",
            space_delim_strings[0]
        );
        let file_path = space_delim_strings[1];
        let mut tmp_output_dir = TempPath::new();
        tmp_output_dir.persist();
        tmp_output_dir
            .create_as_dir()
            .expect("error creating temporary output directory");
        let tmp_output_path = tmp_output_dir.as_ref();
        self.temp_files.push(tmp_output_path.to_path_buf());

        let mut args = format!(
            "run -p move-lang --bin move-build -- {} -o {}",
            file_path,
            tmp_output_path.display(),
        );
        for dep in &space_delim_strings[2..] {
            args.push_str(&format!(" -d {}", dep));
        }

        let status = Command::new("cargo")
            .args(args.split(' '))
            .spawn()?
            .wait()?;
        if !status.success() {
            return Err(format_err!("compilation failed"));
        }

        let output_files = walkdir::WalkDir::new(tmp_output_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let path = e.path();
                e.file_type().is_file()
                    && path
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|ext| ext == "mv")
                        .unwrap_or(false)
            })
            .filter_map(|e| e.path().to_str().map(|s| s.to_string()))
            .collect::<Vec<_>>();
        if output_files.is_empty() {
            bail!("compiler failed to produce an output file")
        }
        Ok(output_files)
    }

    /// Submit a transaction to the network given the unsigned raw transaction, sender public key
    /// and signature
    pub fn submit_signed_transaction(
        &mut self,
        raw_txn: RawTransaction,
        public_key: Ed25519PublicKey,
        signature: Ed25519Signature,
    ) -> Result<()> {
        let txn = SignedTransaction::new(raw_txn, public_key, signature);
        self.submit_and_wait(&txn, true)?;
        Ok(())
    }

    fn submit_program(
        &mut self,
        space_delim_strings: &[&str],
        program: TransactionPayload,
    ) -> Result<()> {
        let (sender_address, _) =
            self.get_account_address_from_parameter(space_delim_strings[1])?;
        let sender = self.get_account_data(&sender_address)?;
        let txn = self.create_txn_to_submit(program, &sender, None, None, None)?;

        self.submit_and_wait(&txn, true)?;
        Ok(())
    }

    /// Publish Move module
    pub fn publish_module(&mut self, space_delim_strings: &[&str]) -> Result<()> {
        ensure!(
            space_delim_strings[0] == "publish" || space_delim_strings[0] == "p",
            "inconsistent command '{}' for publish_module",
            space_delim_strings[0]
        );
        let module_bytes = fs::read(space_delim_strings[2])?;
        self.submit_program(
            space_delim_strings,
            TransactionPayload::Module(Module::new(module_bytes)),
        )
    }

    /// Execute custom script
    pub fn execute_script(&mut self, space_delim_strings: &[&str]) -> Result<()> {
        ensure!(
            space_delim_strings[0] == "execute" || space_delim_strings[0] == "e",
            "inconsistent command '{}' for execute_script",
            space_delim_strings[0]
        );
        let script_bytes = fs::read(space_delim_strings[2])?;
        let arguments: Vec<_> = space_delim_strings[3..]
            .iter()
            .filter_map(|arg| parse_transaction_argument_for_client(arg).ok())
            .collect();
        // TODO: support type arguments in the client.
        self.submit_program(
            space_delim_strings,
            TransactionPayload::Script(Script::new(script_bytes, vec![], arguments)),
        )
    }

    /// Submit a writeset transaction signed by local diem root account.
    pub fn submit_writeset(&mut self, space_delim_strings: &[&str]) -> Result<()> {
        ensure!(
            space_delim_strings[0] == "submit_payload" || space_delim_strings[0] == "ws",
            "inconsistent command '{}' for submit_payload",
            space_delim_strings[0]
        );
        let payload = bcs::from_bytes(fs::read(space_delim_strings[1])?.as_slice())?;
        self.association_transaction_with_local_diem_root_account(payload, true)
    }

    /// Get the latest account information from validator.
    pub fn get_latest_account(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<Option<views::AccountView>> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments to get latest account"
        );
        let (account, _) = self.get_account_address_from_parameter(space_delim_strings[1])?;
        self.get_account_and_update(&account)
    }

    /// Get the latest version
    pub fn get_latest_version(&mut self) -> Version {
        self.client.update_and_verify_state_proof().unwrap();
        self.client.trusted_state().version()
    }

    /// Get the latest annotated account resources from validator.
    pub fn get_latest_account_resources(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<(Option<AnnotatedAccountStateBlob>, Version)> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments to get latest account state"
        );
        let (account, _) = self.get_account_address_from_parameter(space_delim_strings[1])?;
        self.get_annotate_account_blob(account)
    }

    /// Get committed txn by account and sequence number.
    pub fn get_committed_txn_by_acc_seq(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<Option<views::TransactionView>> {
        ensure!(
            space_delim_strings.len() == 4,
            "Invalid number of arguments to get transaction by account and sequence number"
        );
        let (account, _) = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let sequence_number = space_delim_strings[2].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "account_sequence_number",
                InputType::UnsignedInt,
                space_delim_strings[2],
                error,
            )
        })?;

        let fetch_events = parse_bool(space_delim_strings[3]).map_err(|error| {
            format_parse_data_error(
                "fetch_events",
                InputType::Bool,
                space_delim_strings[3],
                error,
            )
        })?;

        self.client
            .get_txn_by_acc_seq(&account, sequence_number, fetch_events)
    }

    /// Get committed txn by account and sequence number
    pub fn get_committed_txn_by_range(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<Vec<views::TransactionView>> {
        ensure!(
            space_delim_strings.len() == 4,
            "Invalid number of arguments to get transaction by range"
        );
        let start_version = space_delim_strings[1].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "start_version",
                InputType::UnsignedInt,
                space_delim_strings[1],
                error,
            )
        })?;
        let limit = space_delim_strings[2].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "limit",
                InputType::UnsignedInt,
                space_delim_strings[2],
                error,
            )
        })?;
        let fetch_events = parse_bool(space_delim_strings[3]).map_err(|error| {
            format_parse_data_error(
                "fetch_events",
                InputType::Bool,
                space_delim_strings[3],
                error,
            )
        })?;

        self.client
            .get_txn_by_range(start_version, limit, fetch_events)
    }

    /// Get account address and (if applicable) authentication key from parameter. If the parameter
    /// is string of address, try to convert it to address, otherwise, try to convert to u64 and
    /// looking at TestClient::accounts.
    pub fn get_account_address_from_parameter(
        &self,
        para: &str,
    ) -> Result<(AccountAddress, Option<AuthenticationKey>)> {
        if is_authentication_key(para) {
            let auth_key = ClientProxy::authentication_key_from_string(para)?;
            Ok((auth_key.derived_address(), Some(auth_key)))
        } else if is_address(para) {
            Ok((ClientProxy::address_from_strings(para)?, None))
        } else {
            let account_ref_id = para.parse::<usize>().map_err(|error| {
                format_parse_data_error(
                    "account_reference_id/account_address",
                    InputType::Usize,
                    para,
                    error,
                )
            })?;
            let account_data = self.accounts.get(account_ref_id).ok_or_else(|| {
                format_err!(
                    "Unable to find account by account reference id: {}, to see all existing \
                     accounts, run: 'account list'",
                    account_ref_id
                )
            })?;
            Ok((
                account_data.address,
                account_data
                    .authentication_key
                    .clone()
                    .and_then(|bytes| AuthenticationKey::try_from(bytes).ok()),
            ))
        }
    }

    /// Get events by account and event type with start sequence number and limit.
    pub fn get_events_by_account_and_type(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<(Vec<views::EventView>, views::AccountView)> {
        ensure!(
            space_delim_strings.len() == 5,
            "Invalid number of arguments, required 5, given {}",
            space_delim_strings.len()
        );
        let (account, _) = self.get_account_address_from_parameter(space_delim_strings[1])?;
        let account_view = match self.client.get_account(&account)? {
            None => bail!("No account found for address {:?}", account),
            Some(account) => account,
        };

        let path = match space_delim_strings[2] {
            "sent" => account_view.sent_events_key,
            "received" => account_view.received_events_key,
            _ => bail!(
                "Unknown event type: {:?}, only sent and received are supported",
                space_delim_strings[2]
            ),
        };
        let start_seq_number = space_delim_strings[3].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "start_seq_number",
                InputType::UnsignedInt,
                space_delim_strings[3],
                error,
            )
        })?;
        let limit = space_delim_strings[4].parse::<u64>().map_err(|error| {
            format_parse_data_error(
                "start_seq_number",
                InputType::UnsignedInt,
                space_delim_strings[4],
                error,
            )
        })?;
        Ok((
            self.client.get_events(path, start_seq_number, limit)?,
            account_view,
        ))
    }

    /// Write mnemonic recover to the file specified.
    pub fn write_recovery(&self, space_delim_strings: &[&str]) -> Result<()> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments for writing recovery"
        );

        self.wallet
            .write_recovery(&Path::new(space_delim_strings[1]))?;
        Ok(())
    }

    /// Recover wallet accounts from command 'recover <file>' and return vec<(account_address, index)>.
    pub fn recover_wallet_accounts(
        &mut self,
        space_delim_strings: &[&str],
    ) -> Result<Vec<AddressAndIndex>> {
        ensure!(
            space_delim_strings.len() == 2,
            "Invalid number of arguments for recovering wallets"
        );
        let wallet = WalletLibrary::recover(&Path::new(space_delim_strings[1]))?;
        self.set_wallet(wallet);
        self.recover_accounts_in_wallet()
    }

    /// Recover accounts in wallets and sync state if sync_on_wallet_recovery is true.
    pub fn recover_accounts_in_wallet(&mut self) -> Result<Vec<AddressAndIndex>> {
        let wallet_addresses = self.wallet.get_addresses()?;
        let mut account_data = Vec::new();
        for address in wallet_addresses {
            let auth_key = self.wallet.get_authentication_key(&address)?;

            account_data.push(Self::get_account_data_from_address(
                &self.client,
                address,
                self.sync_on_wallet_recovery,
                None,
                Some(auth_key.to_vec()),
            )?);
        }
        // Clear current cached AccountData as we always swap the entire wallet completely.
        Ok(self.set_accounts(account_data))
    }

    /// Insert the account data to Client::accounts and return its address and index.s
    pub fn insert_account_data(&mut self, account_data: AccountData) -> AddressAndIndex {
        let address = account_data.address;

        self.accounts.push(account_data);
        self.address_to_ref_id
            .insert(address, self.accounts.len() - 1);

        AddressAndIndex {
            address,
            index: self.accounts.len() - 1,
        }
    }

    /// Test JSON RPC client connection with validator.
    pub fn test_validator_connection(&mut self) -> Result<views::MetadataView> {
        self.client.update_and_verify_state_proof()?;
        self.client.get_metadata()
    }

    /// Test client's connection to validator with proof.
    pub fn test_trusted_connection(&mut self) -> Result<()> {
        self.client.update_and_verify_state_proof()
    }

    fn get_annotate_account_blob(
        &mut self,
        address: AccountAddress,
    ) -> Result<(Option<AnnotatedAccountStateBlob>, Version)> {
        let (blob, ver) = self.client.get_account_state_blob(&address)?;
        if let Some(account_blob) = blob {
            let mut storage = InMemoryStorage::new();
            for (blob, module) in diem_framework_releases::current_modules_with_blobs() {
                storage.publish_or_overwrite_module(module.self_id(), blob.clone())
            }
            let annotator = DiemValueAnnotator::new(&storage);
            let annotate_blob =
                annotator.view_account_state(&AccountState::try_from(&account_blob)?)?;
            Ok((Some(annotate_blob), ver))
        } else {
            Ok((None, ver))
        }
    }

    /// Get account from validator and update status of account if it is cached locally.
    fn get_account_and_update(
        &mut self,
        address: &AccountAddress,
    ) -> Result<Option<views::AccountView>> {
        let account = self.client.get_account(address)?;
        // This isn't used by anything except to keep track of the current version and to simulate
        // some potential verifiable clients, which is yet to be implemented. It also has some
        // challenges in handling retries if the upstream hasn't yet arrived at the expected
        // version and breaks with our testnet deployment, so disabling this for now.
        // self.client.update_and_verify_state_proof()?;

        if let Some(ref ac) = account.as_ref() {
            self.update_account_seq(address, ac.sequence_number)
        }
        Ok(account)
    }

    /// Update account seq
    fn update_account_seq(&mut self, address: &AccountAddress, seq: u64) {
        if let Some(diem_root_account) = &mut self.diem_root_account {
            if &diem_root_account.address == address {
                diem_root_account.sequence_number = seq;
            }
        }
        if let Some(tc_account) = &mut self.tc_account {
            if &tc_account.address == address {
                tc_account.sequence_number = seq;
            }
        }
        if let Some(testnet_dd_account) = &mut self.testnet_designated_dealer_account {
            if &testnet_dd_account.address == address {
                testnet_dd_account.sequence_number = seq;
            }
        }
        if let Ok((ref_id, _)) = self.get_account_data_and_id(address) {
            // assumption follows from invariant
            let mut account_data: &mut AccountData = self.accounts.get_mut(ref_id).unwrap();
            account_data.status = AccountStatus::Persisted;
            account_data.sequence_number = seq;
        };
    }

    /// Get account resource from validator and update status of account if it is cached locally.
    fn get_account_resource_and_update(
        &mut self,
        address: &AccountAddress,
    ) -> Result<views::AccountView> {
        self.get_account_and_update(address)?
            .ok_or_else(|| format_err!("No account exists at {:?}", address))
    }

    /// Get account using specific address.
    /// Sync with validator for account sequence number in case it is already created on chain.
    /// This assumes we have a very low probability of mnemonic word conflict.
    #[allow(clippy::unnecessary_wraps)]
    fn get_account_data_from_address(
        client: &DiemClient,
        address: AccountAddress,
        sync_with_validator: bool,
        key_pair: Option<KeyPair<Ed25519PrivateKey, Ed25519PublicKey>>,
        authentication_key_opt: Option<Vec<u8>>,
    ) -> Result<AccountData> {
        let (sequence_number, authentication_key, status) = if sync_with_validator {
            let ret = client.get_account(&address);
            match ret {
                Ok(resp) => match resp {
                    Some(account_view) => (
                        account_view.sequence_number,
                        Some(account_view.authentication_key.into_inner().into()),
                        AccountStatus::Persisted,
                    ),
                    None => (0, authentication_key_opt, AccountStatus::Local),
                },
                Err(e) => {
                    error!("Failed to get account from validator, error: {:?}", e);
                    (0, authentication_key_opt, AccountStatus::Unknown)
                }
            }
        } else {
            (0, authentication_key_opt, AccountStatus::Local)
        };
        Ok(AccountData {
            address,
            authentication_key,
            key_pair,
            sequence_number,
            status,
        })
    }

    fn get_diem_wallet(mnemonic_file: Option<String>) -> Result<WalletLibrary> {
        let wallet_recovery_file_path = if let Some(input_mnemonic_word) = mnemonic_file {
            Path::new(&input_mnemonic_word).to_path_buf()
        } else {
            let mut file_path = std::env::current_dir()?;
            file_path.push(CLIENT_WALLET_MNEMONIC_FILE);
            file_path
        };

        let wallet = if let Ok(recovered_wallet) = io_utils::recover(&wallet_recovery_file_path) {
            recovered_wallet
        } else {
            let new_wallet = WalletLibrary::new();
            new_wallet.write_recovery(&wallet_recovery_file_path)?;
            new_wallet
        };
        Ok(wallet)
    }

    /// Set wallet instance used by this client.
    fn set_wallet(&mut self, wallet: WalletLibrary) {
        self.wallet = wallet;
    }

    fn address_from_strings(data: &str) -> Result<AccountAddress> {
        let account_vec: Vec<u8> = hex::decode(data.parse::<String>()?)?;
        ensure!(
            account_vec.len() == AccountAddress::LENGTH,
            "The address {:?} is of invalid length. Addresses must be 16-bytes long"
        );
        let account = AccountAddress::try_from(&account_vec[..]).map_err(|error| {
            format_err!(
                "The address {:?} is invalid, error: {:?}",
                &account_vec,
                error,
            )
        })?;
        Ok(account)
    }

    fn authentication_key_from_string(data: &str) -> Result<AuthenticationKey> {
        let bytes_vec: Vec<u8> = hex::decode(data.parse::<String>()?)?;
        ensure!(
            bytes_vec.len() == AuthenticationKey::LENGTH,
            "The authentication key string {:?} is of invalid length. Authentication keys must be 32-bytes long"
        );

        let auth_key = AuthenticationKey::try_from(&bytes_vec[..]).map_err(|error| {
            format_err!(
                "The authentication key {:?} is invalid, error: {:?}",
                &bytes_vec,
                error,
            )
        })?;
        Ok(auth_key)
    }

    /// Send a transaction signed by the local diem_root credential
    pub fn association_transaction_with_local_diem_root_account(
        &mut self,
        payload: TransactionPayload,
        is_blocking: bool,
    ) -> Result<()> {
        ensure!(
            self.diem_root_account.is_some(),
            "No assoc root account loaded"
        );
        let sender = self.diem_root_account.as_ref().unwrap();
        let txn = self.create_txn_to_submit(payload, sender, None, None, None)?;

        self.submit_and_wait(&txn, is_blocking)?;

        Ok(())
    }

    fn association_transaction_with_local_tc_account(
        &mut self,
        payload: TransactionPayload,
        is_blocking: bool,
    ) -> Result<()> {
        ensure!(
            self.tc_account.is_some(),
            "No treasury compliance account loaded"
        );
        let sender = self.tc_account.as_ref().unwrap();
        let txn = self.create_txn_to_submit(payload, sender, None, None, None)?;

        self.submit_and_wait(&txn, is_blocking)?;

        Ok(())
    }

    fn association_transaction_with_local_testnet_dd_account(
        &mut self,
        payload: TransactionPayload,
        is_blocking: bool,
    ) -> Result<()> {
        ensure!(
            self.testnet_designated_dealer_account.is_some(),
            "No testnet Designated Dealer account loaded"
        );
        let sender = self.testnet_designated_dealer_account.as_ref().unwrap();
        let txn = self.create_txn_to_submit(payload, sender, None, None, None)?;

        self.submit_and_wait(&txn, is_blocking)?;
        Ok(())
    }

    fn mint_coins_with_faucet_service(
        &mut self,
        receiver: AuthenticationKey,
        num_coins: u64,
        coin_currency: String,
    ) -> Result<()> {
        let client = reqwest::blocking::Client::new();

        let url = Url::parse_with_params(
            self.faucet_url.as_str(),
            &[
                ("amount", num_coins.to_string().as_str()),
                ("auth_key", &hex::encode(receiver)),
                ("currency_code", coin_currency.as_str()),
                ("return_txns", "true"),
            ],
        )?;

        let response = client.post(url).send()?;
        let status_code = response.status();
        let body = response.text()?;
        if !status_code.is_success() {
            return Err(format_err!(
                "Failed to query remote faucet server[status={}]: {:?}",
                status_code.as_str(),
                body,
            ));
        }
        let bytes = hex::decode(body)?;
        let txns: Vec<SignedTransaction> = bcs::from_bytes(&bytes).unwrap();
        for txn in &txns {
            self.wait_for_signed_transaction(txn).map_err(|e| {
                info!("minting transaction error: {}", e);
                format_err!("transaction execution failed, please retry")
            })?;
        }

        Ok(())
    }

    /// Scale the number in `input` based on `scaling_factor` and ensure the fractional part is no
    /// less than `fractional_part` amount.
    pub fn convert_to_scaled_representation(
        input: &str,
        scaling_factor: i64,
        fractional_part: i64,
    ) -> Result<u64> {
        ensure!(!input.is_empty(), "Empty input not allowed for diem unit");
        let max_value = Decimal::from_u64(std::u64::MAX).unwrap() / Decimal::new(scaling_factor, 0);
        let scale = input.find('.').unwrap_or(input.len() - 1);
        let digits_after_decimal = input
            .find('.')
            .map(|num_digits| input.len() - num_digits - 1)
            .unwrap_or(0) as u32;
        ensure!(
            digits_after_decimal <= 14,
            "Input value is too small: {}",
            input
        );
        let input_fractional_part = 10u64.pow(digits_after_decimal);
        ensure!(
            input_fractional_part <= fractional_part as u64,
            "Input value has too small of a fractional part 1/{}, but smallest allowed is 1/{}",
            input_fractional_part,
            fractional_part
        );
        ensure!(
            scale <= 14,
            "Input value is too big: {:?}, max: {:?}",
            input,
            max_value
        );
        let original = Decimal::from_str(input)?;
        ensure!(
            original <= max_value,
            "Input value is too big: {:?}, max: {:?}",
            input,
            max_value
        );
        let value = original * Decimal::new(scaling_factor, 0);
        ensure!(value.fract().is_zero(), "invalid value");
        value.to_u64().ok_or_else(|| format_err!("invalid value"))
    }

    /// convert number of coins (main unit) given as string to its on-chain representation
    pub fn convert_to_on_chain_representation(
        &mut self,
        input: &str,
        currency: &str,
    ) -> Result<u64> {
        ensure!(!input.is_empty(), "Empty input not allowed for diem unit");
        ensure!(
            currency != XDX_NAME,
            "XDX not allowed to be minted or transferred. Use XUS instead"
        );
        // This is not supposed to panic as it is used as constant here.
        let currencies_info = self.client.get_currency_info()?;
        let currency_info = currencies_info
            .iter()
            .find(|info| info.code == currency)
            .ok_or_else(|| {
                format_err!(
                    "Unable to get currency info for {} when converting to on-chain units",
                    currency
                )
            })?;
        Self::convert_to_scaled_representation(
            input,
            currency_info.scaling_factor as i64,
            currency_info.fractional_part as i64,
        )
    }

    /// Craft a transaction to be submitted.
    fn create_txn_to_submit(
        &self,
        program: TransactionPayload,
        sender_account: &AccountData,
        max_gas_amount: Option<u64>,
        gas_unit_price: Option<u64>,
        gas_currency_code: Option<String>,
    ) -> Result<SignedTransaction> {
        let signer: Box<&dyn TransactionSigner> = match &sender_account.key_pair {
            Some(key_pair) => Box::new(key_pair),
            None => Box::new(&self.wallet),
        };
        create_user_txn(
            *signer,
            program,
            sender_account.address,
            sender_account.sequence_number,
            max_gas_amount.unwrap_or(MAX_GAS_AMOUNT),
            gas_unit_price.unwrap_or(GAS_UNIT_PRICE),
            gas_currency_code.unwrap_or_else(|| XUS_NAME.to_owned()),
            TX_EXPIRATION,
            self.chain_id,
        )
    }
}

// Update WriteSet
fn encode_stdlib_upgrade_transaction() -> ChangeSet {
    let mut write_set = WriteSetMut::new(vec![]);
    for module in diem_framework::modules() {
        let mut bytes = vec![];
        module.serialize(&mut bytes).unwrap();
        write_set.push((
            AccessPath::code_access_path(module.self_id()),
            WriteOp::Value(bytes),
        ));
    }
    ChangeSet::new(
        write_set.freeze().expect("Failed to create writeset"),
        vec![],
    )
}

fn parse_transaction_argument_for_client(s: &str) -> Result<TransactionArgument> {
    if is_address(s) {
        let account_address = ClientProxy::address_from_strings(s)?;
        return Ok(TransactionArgument::Address(account_address));
    }
    parse_transaction_argument(s)
}

fn format_parse_data_error<T: std::fmt::Debug>(
    field: &str,
    input_type: InputType,
    value: &str,
    error: T,
) -> Error {
    format_err!(
        "Unable to parse input for {} - \
         please enter an {:?}.  Input was: {}, error: {:?}",
        field,
        input_type,
        value,
        error
    )
}

fn parse_bool(para: &str) -> Result<bool> {
    Ok(para.to_lowercase().parse::<bool>()?)
}

impl fmt::Display for AccountEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AccountEntry::Index(i) => write!(f, "{}", i),
            AccountEntry::Address(addr) => write!(f, "{}", addr),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::client_proxy::{parse_bool, AddressAndIndex, ClientProxy};
    use diem_temppath::TempPath;
    use diem_types::{
        chain_id::ChainId, ledger_info::LedgerInfo, on_chain_config::ValidatorSet,
        waypoint::Waypoint,
    };
    use diem_wallet::io_utils;
    use proptest::prelude::*;

    fn generate_accounts_from_wallet(count: usize) -> (ClientProxy, Vec<AddressAndIndex>) {
        let mut accounts = Vec::new();
        accounts.reserve(count);
        let file = TempPath::new();
        let mnemonic_path = file.path().to_str().unwrap().to_string();
        let waypoint =
            Waypoint::new_epoch_boundary(&LedgerInfo::mock_genesis(Some(ValidatorSet::empty())))
                .unwrap();

        // Note: `client_proxy` won't actually connect to URL - it will be used only to
        // generate random accounts
        let mut client_proxy = ClientProxy::new(
            ChainId::test(),
            "http://localhost:8080/v1",
            &"",
            &"",
            &"",
            false,
            None,
            Some(mnemonic_path),
            waypoint,
            true,
        )
        .unwrap();
        for _ in 0..count {
            accounts.push(client_proxy.create_next_account(false).unwrap());
        }

        (client_proxy, accounts)
    }

    #[test]
    fn test_parse_bool() {
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("True").unwrap());
        assert!(parse_bool("TRue").unwrap());
        assert!(parse_bool("TRUE").unwrap());
        assert!(!parse_bool("false").unwrap());
        assert!(!parse_bool("False").unwrap());
        assert!(!parse_bool("FaLSe").unwrap());
        assert!(!parse_bool("FALSE").unwrap());
        assert!(parse_bool("1").is_err());
        assert!(parse_bool("0").is_err());
        assert!(parse_bool("2").is_err());
        assert!(parse_bool("1adf").is_err());
        assert!(parse_bool("ad13").is_err());
        assert!(parse_bool("ad1f").is_err());
    }

    #[test]
    fn test_micro_diem_conversion() {
        assert!(ClientProxy::convert_to_scaled_representation("", 1_000_000, 1_000_000).is_err());
        assert!(
            ClientProxy::convert_to_scaled_representation("-11", 1_000_000, 1_000_000).is_err()
        );
        assert!(
            ClientProxy::convert_to_scaled_representation("abc", 1_000_000, 1_000_000).is_err()
        );
        assert!(ClientProxy::convert_to_scaled_representation(
            "11111112312321312321321321",
            1_000_000,
            1_000_000
        )
        .is_err());
        assert!(ClientProxy::convert_to_scaled_representation("100000.0", 1, 1).is_err());
        assert!(ClientProxy::convert_to_scaled_representation("0", 1_000_000, 1_000_000).is_ok());
        assert!(ClientProxy::convert_to_scaled_representation("0", 1_000_000, 1_000_000).is_ok());
        assert!(ClientProxy::convert_to_scaled_representation("1", 1_000_000, 1_000_000).is_ok());
        assert!(ClientProxy::convert_to_scaled_representation("0.1", 1_000_000, 1_000_000).is_ok());
        assert!(ClientProxy::convert_to_scaled_representation("1.1", 1_000_000, 1_000_000).is_ok());
        // Max of micro diem is u64::MAX (18446744073709551615).
        assert!(ClientProxy::convert_to_scaled_representation(
            "18446744073709.551615",
            1_000_000,
            1_000_000
        )
        .is_ok());
        assert!(ClientProxy::convert_to_scaled_representation(
            "184467440737095.51615",
            1_000_000,
            1_000_000
        )
        .is_err());
        assert!(ClientProxy::convert_to_scaled_representation(
            "18446744073709.551616",
            1_000_000,
            1_000_000
        )
        .is_err());
    }

    #[test]
    fn test_scaled_represenation() {
        assert_eq!(
            ClientProxy::convert_to_scaled_representation("10", 1_000_000, 100).unwrap(),
            10 * 1_000_000
        );
        assert_eq!(
            ClientProxy::convert_to_scaled_representation("10.", 1_000_000, 100).unwrap(),
            10 * 1_000_000
        );
        assert_eq!(
            ClientProxy::convert_to_scaled_representation("10.20", 1_000_000, 100).unwrap(),
            (10.20 * 1_000_000f64) as u64
        );
        assert!(ClientProxy::convert_to_scaled_representation("10.201", 1_000_000, 100).is_err());
        assert_eq!(
            ClientProxy::convert_to_scaled_representation("10.991", 1_000_000, 1000).unwrap(),
            (10.991 * 1_000_000f64) as u64
        );
        assert_eq!(
            ClientProxy::convert_to_scaled_representation("100.99", 1000, 100).unwrap(),
            (100.99 * 1000f64) as u64
        );
        assert_eq!(
            ClientProxy::convert_to_scaled_representation("100000", 1, 1).unwrap(),
            100_000
        );
    }

    #[test]
    fn test_generate() {
        let num = 1;
        let (_, accounts) = generate_accounts_from_wallet(num);
        assert_eq!(accounts.len(), num);
    }

    #[test]
    fn test_write_recover() {
        let num = 100;
        let (client, accounts) = generate_accounts_from_wallet(num);
        assert_eq!(accounts.len(), num);

        let file = TempPath::new();
        let path = file.path();
        io_utils::write_recovery(&client.wallet, &path).expect("failed to write to file");

        let wallet = io_utils::recover(&path).expect("failed to load from file");

        assert_eq!(client.wallet.mnemonic(), wallet.mnemonic());
    }

    proptest! {
        // Proptest is used to verify that the conversion will not panic with random input.
        #[test]
        fn test_micro_diem_conversion_random_string(req in any::<String>()) {
            let _res = ClientProxy::convert_to_scaled_representation(&req, 1_000_000, 1_000_000);
        }
        #[test]
        fn test_micro_diem_conversion_random_f64(req in any::<f64>()) {
            let req_str = req.to_string();
            let _res = ClientProxy::convert_to_scaled_representation(&req_str, 1_000_000, 1_000_000);
        }
        #[test]
        fn test_micro_diem_conversion_random_u64(req in any::<u64>()) {
            let req_str = req.to_string();
            let _res = ClientProxy::convert_to_scaled_representation(&req_str, 1_000_000, 1_000_000);
        }
    }
}

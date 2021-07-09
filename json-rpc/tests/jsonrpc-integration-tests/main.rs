// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use diem_crypto::{hash::CryptoHash, HashValue};
use diem_json_rpc::views::{
    AccountTransactionsWithProofView, AccumulatorConsistencyProofView, EventView,
};
use diem_sdk::{transaction_builder::Currency, types::AccountKey};
use diem_transaction_builder::stdlib;
use diem_types::{
    access_path::AccessPath,
    account_address::AccountAddress,
    account_config::{treasury_compliance_account_address, xus_tag},
    contract_event::EventWithProof,
    diem_id_identifier::DiemIdVaspDomainIdentifier,
    event::EventKey,
    ledger_info::LedgerInfoWithSignatures,
    on_chain_config::DIEM_MAX_KNOWN_VERSION,
    proof::{AccumulatorConsistencyProof, TransactionAccumulatorSummary},
    transaction::{
        AccountTransactionsWithProof, ChangeSet, Transaction, TransactionPayload, WriteSetPayload,
    },
    write_set::{WriteOp, WriteSet, WriteSetMut},
};
use forge::{
    forge_main, AdminContext, AdminTest, ForgeConfig, LocalFactory, Options, PublicUsageContext,
    PublicUsageTest, Result, Test,
};
use serde_json::json;
use std::{
    convert::{TryFrom, TryInto},
    ops::Deref,
    str::FromStr,
};

mod helper;
use helper::JsonRpcTestHelper;

fn main() -> Result<()> {
    let tests = ForgeConfig {
        public_usage_tests: &[
            &CurrencyInfo,
            &BlockMetadata,
            &OldMetadata,
            &AccoutNotFound,
            &UnknownAccountRoleType,
            &DesignatedDealerPreburns,
            &ParentVaspAccountRole,
            &GetAccountByVersion,
            &ChildVaspAccountRole,
            &PeerToPeerWithEvents,
            &PeerToPeerErrorExplination,
            &ReSubmittingTransactionWontFail,
            &MempoolValidationError,
            &ExpiredTransaction,
            &RotateComplianceKeyEvent,
            &CreateAccountEvent,
            &GetTransactionsWithoutEvents,
            &GetAccountTransactionsWithoutEvents,
            &GetAccountTransactionsWithProofs,
            &GetTransactionsWithProofs,
            &GetTreasuryComplianceAccount,
            &GetEventsWithProofs,
            &MultiAgentPaymentOverDualAttestationLimit,
            &GetAccumulatorConsistencyProof,
            &NoUnknownEvents,
        ],
        admin_tests: &[
            &PreburnAndBurnEvents,
            &CancleBurnEvent,
            &UpdateExchangeRateEvent,
            &MintAndReceivedMintEvents,
            &AddAndRemoveVaspDomain,
            &MultiAgentRotateAuthenticationKeyAdminScript,
            &MultiAgentRotateAuthenticationKeyAdminScriptFunction,
            &UpgradeEventAndNewEpoch,
            &UpgradeDiemVersion,
        ],
        network_tests: &[],
    };

    let options = Options::from_args();
    forge_main(tests, LocalFactory::from_workspace()?, &options)
}

struct CurrencyInfo;

impl Test for CurrencyInfo {
    fn name(&self) -> &'static str {
        "jsonrpc::currency-info"
    }
}

impl PublicUsageTest for CurrencyInfo {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let resp = env.send("get_currencies", json!([]));
        assert_eq!(
            resp.result.unwrap(),
            json!([
                {
                    "burn_events_key": "06000000000000000000000000000000000000000a550c18",
                    "cancel_burn_events_key": "08000000000000000000000000000000000000000a550c18",
                    "code": "XUS",
                    "exchange_rate_update_events_key": "09000000000000000000000000000000000000000a550c18",
                    "fractional_part": 100,
                    "mint_events_key": "05000000000000000000000000000000000000000a550c18",
                    "preburn_events_key": "07000000000000000000000000000000000000000a550c18",
                    "scaling_factor": 1000000,
                    "to_xdx_exchange_rate": 1.0,
                },
                {
                    "burn_events_key": "0b000000000000000000000000000000000000000a550c18",
                    "cancel_burn_events_key": "0d000000000000000000000000000000000000000a550c18",
                    "code": "XDX",
                    "exchange_rate_update_events_key": "0e000000000000000000000000000000000000000a550c18",
                    "fractional_part": 1000,
                    "mint_events_key": "0a000000000000000000000000000000000000000a550c18",
                    "preburn_events_key": "0c000000000000000000000000000000000000000a550c18",
                    "scaling_factor": 1000000,
                    "to_xdx_exchange_rate": 1.0
                }
            ])
        );

        Ok(())
    }
}

struct BlockMetadata;

impl Test for BlockMetadata {
    fn name(&self) -> &'static str {
        "jsonrpc::block-metadata"
    }
}

impl PublicUsageTest for BlockMetadata {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());

        // Fund and account to ensure there are some txns on the chain
        let account = ctx.random_account();
        ctx.create_parent_vasp_account(account.authentication_key())?;
        ctx.fund(account.address(), 10)?;

        // batch request
        let resp = env.send_request(json!([
            {"jsonrpc": "2.0", "method": "get_metadata", "params": [], "id": 1},
            {"jsonrpc": "2.0", "method": "get_state_proof", "params": [0], "id": 2}
        ]));

        // extract both responses
        let resps: Vec<serde_json::Value> =
            serde_json::from_value(resp).expect("should be valid serde_json::Value");
        let metadata = &resps.iter().find(|g| g["id"] == 1).unwrap()["result"];
        let state_proof = &resps.iter().find(|g| g["id"] == 2).unwrap()["result"];

        // extract header and ensure they match in both responses
        let diem_chain_id = &resps[0]["diem_chain_id"];
        let diem_ledger_timestampusec = &resps[0]["diem_ledger_timestampusec"];
        let diem_ledger_version = &resps[0]["diem_ledger_version"];

        assert_eq!(diem_chain_id, &resps[1]["diem_chain_id"]);
        assert_eq!(
            diem_ledger_timestampusec,
            &resps[1]["diem_ledger_timestampusec"]
        );
        assert_eq!(diem_ledger_version, &resps[1]["diem_ledger_version"]);

        // parse metadata
        assert_eq!(&metadata["chain_id"], diem_chain_id);
        assert_eq!(&metadata["timestamp"], diem_ledger_timestampusec);
        assert_eq!(&metadata["version"], diem_ledger_version);
        assert_eq!(metadata["chain_id"], ctx.chain_id().id());
        // All genesis's start with closed publishing so this should be populated with a
        // list of allowed scripts and publishing off
        assert_ne!(metadata["script_hash_allow_list"], json!([]));
        assert_eq!(metadata["module_publishing_allowed"], false);
        assert_eq!(metadata["diem_version"], 3);
        assert_eq!(metadata["dual_attestation_limit"], 1000000000);
        assert_ne!(diem_ledger_timestampusec, 0);
        assert_ne!(diem_ledger_version, 0);

        // prove the accumulator_root_hash
        let info_hex = state_proof["ledger_info_with_signatures"].as_str().unwrap();
        let info: LedgerInfoWithSignatures =
            bcs::from_bytes(&hex::decode(&info_hex).unwrap()).unwrap();
        let expected_hash = info
            .deref()
            .ledger_info()
            .transaction_accumulator_hash()
            .to_hex();
        assert_eq!(
            expected_hash,
            metadata["accumulator_root_hash"].as_str().unwrap()
        );

        Ok(())
    }
}

/// Get Metadata with older version parameter should not return version information
struct OldMetadata;

impl Test for OldMetadata {
    fn name(&self) -> &'static str {
        "jsonrpc::old-metadata"
    }
}

impl PublicUsageTest for OldMetadata {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        // create a random accound in order to force the version hieght to be greater than 1
        let account = ctx.random_account();
        ctx.create_parent_vasp_account(account.authentication_key())?;

        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let resp = env.send("get_metadata", json!([1]));
        let metadata = resp.result.unwrap();
        // no data provided for the following fields when requesting older version
        assert_eq!(metadata["script_hash_allow_list"], json!(null));
        assert_eq!(metadata["module_publishing_allowed"], json!(null));
        assert_eq!(metadata["diem_version"], json!(null));
        Ok(())
    }
}

struct AccoutNotFound;

impl Test for AccoutNotFound {
    fn name(&self) -> &'static str {
        "jsonrpc::account-not-found"
    }
}

impl PublicUsageTest for AccoutNotFound {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let resp = env.send("get_account", json!(["d738a0b9851305dfe1d17707f0841dbc"]));
        assert!(resp.result.is_none());
        Ok(())
    }
}

struct UnknownAccountRoleType;

impl Test for UnknownAccountRoleType {
    fn name(&self) -> &'static str {
        "jsonrpc::unknown-account-role-type"
    }
}

impl PublicUsageTest for UnknownAccountRoleType {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let address = format!("{:x}", diem_types::account_config::diem_root_address());
        let resp = env.send("get_account", json!([address]));
        let mut result = resp.result.unwrap();
        // as we generate account auth key, ignore it in assertion
        assert_ne!(result["authentication_key"].as_str().unwrap(), "");
        result["authentication_key"] = json!(null);
        let sequence_number = result["sequence_number"].as_u64().unwrap();
        assert_eq!(
            result,
            json!({
                "address": address,
                "authentication_key": null,
                "balances": [],
                "delegated_key_rotation_capability": false,
                "delegated_withdrawal_capability": false,
                "is_frozen": false,
                "received_events_key": "02000000000000000000000000000000000000000a550c18",
                "role": { "type": "unknown" },
                "sent_events_key": "03000000000000000000000000000000000000000a550c18",
                "sequence_number": sequence_number,
                "version": resp.diem_ledger_version,
            }),
        );
        Ok(())
    }
}

struct DesignatedDealerPreburns;

impl Test for DesignatedDealerPreburns {
    fn name(&self) -> &'static str {
        "jsonrpc::designated-dealer-preburns"
    }
}

impl PublicUsageTest for DesignatedDealerPreburns {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let factory = ctx.transaction_factory();
        let mut dd = ctx.random_account();
        ctx.create_designated_dealer_account(dd.authentication_key())?;

        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let address = format!("{:x}", dd.address());
        let resp = env.send("get_account", json!([address]));
        let result = resp.result.unwrap();
        let human_name = result["role"]["human_name"].as_str().unwrap();

        // Without Preburns
        assert_eq!(
            result,
            json!({
                "address": address,
                "authentication_key": dd.authentication_key(),
                "balances": [
                    {
                        "amount": 0_u64,
                        "currency": "XUS"
                    },
                ],
                "delegated_key_rotation_capability": false,
                "delegated_withdrawal_capability": false,
                "is_frozen": false,
                "received_events_key": EventKey::new_from_address(&dd.address(), 3),
                "role": {
                    "type": "designated_dealer",
                    "base_url": "",
                    "compliance_key": "",
                    "expiration_time": 18446744073709551615_u64,
                    "human_name": human_name,
                    "preburn_balances": [
                        {
                            "amount": 0_u64,
                            "currency": "XUS"
                        }
                    ],
                    "preburn_queues": [
                        {
                            "preburns": [],
                            "currency": "XUS",
                        }
                    ],
                    "received_mint_events_key": EventKey::new_from_address(&dd.address(), 0),
                    "compliance_key_rotation_events_key": EventKey::new_from_address(&dd.address(), 1),
                    "base_url_rotation_events_key": EventKey::new_from_address(&dd.address(), 2),
                },
                "sent_events_key": EventKey::new_from_address(&dd.address(), 4),
                "sequence_number": dd.sequence_number(),
                "version": resp.diem_ledger_version,
            }),
        );

        // Fund the DD account and create some Pre-burns
        ctx.fund(dd.address(), 400)?;

        env.submit_and_wait(&dd.sign_with_transaction_builder(
            factory.script(stdlib::encode_preburn_script(xus_tag(), 100)),
        ));
        env.submit_and_wait(&dd.sign_with_transaction_builder(
            factory.script(stdlib::encode_preburn_script(xus_tag(), 40)),
        ));
        env.submit_and_wait(&dd.sign_with_transaction_builder(
            factory.script(stdlib::encode_preburn_script(xus_tag(), 60)),
        ));

        let resp = env.send("get_account", json!([address]));
        let result = resp.result.unwrap();

        // With Preburns
        assert_eq!(
            result,
            json!({
                "address": address,
                "authentication_key": dd.authentication_key(),
                "balances": [
                    {
                        "amount": 200_u64,
                        "currency": "XUS"
                    },
                ],
                "delegated_key_rotation_capability": false,
                "delegated_withdrawal_capability": false,
                "is_frozen": false,
                "received_events_key": EventKey::new_from_address(&dd.address(), 3),
                "role": {
                    "type": "designated_dealer",
                    "base_url": "",
                    "compliance_key": "",
                    "expiration_time": 18446744073709551615_u64,
                    "human_name": human_name,
                    "preburn_balances": [
                        {
                            "amount": 200_u64,
                            "currency": "XUS"
                        }
                    ],
                    "preburn_queues": [
                        {
                            "currency": "XUS",
                            "preburns": [
                                {
                                    "preburn": {
                                        "amount": 100_u64,
                                        "currency": "XUS",
                                    },
                                    "metadata": "",
                                },
                                {
                                    "preburn": {
                                        "amount": 40_u64,
                                        "currency": "XUS"
                                    },
                                    "metadata": "",
                                },
                                {
                                    "preburn": {
                                        "amount": 60_u64,
                                        "currency": "XUS"
                                    },
                                    "metadata": "",
                                },
                            ],
                        }
                    ],
                    "received_mint_events_key": EventKey::new_from_address(&dd.address(), 0),
                    "compliance_key_rotation_events_key": EventKey::new_from_address(&dd.address(), 1),
                    "base_url_rotation_events_key": EventKey::new_from_address(&dd.address(), 2),
                },
                "sent_events_key": EventKey::new_from_address(&dd.address(), 4),
                "sequence_number": dd.sequence_number(),
                "version": resp.diem_ledger_version,
            }),
        );

        Ok(())
    }
}

struct ParentVaspAccountRole;

impl Test for ParentVaspAccountRole {
    fn name(&self) -> &'static str {
        "jsonrpc::parent-vasp-account-role"
    }
}

impl PublicUsageTest for ParentVaspAccountRole {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let vasp = ctx.random_account();
        ctx.create_parent_vasp_account(vasp.authentication_key())?;

        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let address = format!("{:x}", vasp.address());
        let resp = env.send("get_account", json!([address]));
        let result = resp.result.unwrap();
        let human_name = result["role"]["human_name"].as_str().unwrap();

        assert_eq!(
            result,
            json!({
                "address": address,
                "authentication_key": vasp.authentication_key().to_string(),
                "balances": [{"amount": 0_u64, "currency": "XUS"}],
                "delegated_key_rotation_capability": false,
                "delegated_withdrawal_capability": false,
                "is_frozen": false,
                "received_events_key": EventKey::new_from_address(&vasp.address(), 2),
                "role": {
                    "base_url": "",
                    "base_url_rotation_events_key": EventKey::new_from_address(&vasp.address(), 1),
                    "compliance_key": "",
                    "compliance_key_rotation_events_key": EventKey::new_from_address(&vasp.address(), 0),
                    "vasp_domains": [],
                    "expiration_time": 18446744073709551615_u64,
                    "human_name": human_name,
                    "num_children": 0,
                    "type": "parent_vasp",
                },
                "sent_events_key": EventKey::new_from_address(&vasp.address(), 3),
                "sequence_number": 0,
                "version": resp.diem_ledger_version,
            }),
        );

        Ok(())
    }
}

struct GetAccountByVersion;

impl Test for GetAccountByVersion {
    fn name(&self) -> &'static str {
        "jsonrpc::get-account-by-version"
    }
}

impl PublicUsageTest for GetAccountByVersion {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();

        let mut vasp = ctx.random_account();
        let child = ctx.random_account();
        ctx.create_parent_vasp_account(vasp.authentication_key())?;
        env.submit_and_wait(&vasp.sign_with_transaction_builder(
            factory.create_child_vasp_account(Currency::XUS, child.authentication_key(), false, 0),
        ));

        let address = format!("{:x}", vasp.address());
        let resp = env.send("get_account_transaction", json!([address, 0, false]));
        let result = resp.result.unwrap();
        let prev_version: u64 = result["version"].as_u64().unwrap() - 1;
        let resp = env.send("get_account", json!([address, prev_version]));
        let result = resp.result.unwrap();
        let human_name = result["role"]["human_name"].as_str().unwrap();

        assert_eq!(
            result,
            json!({
                "address": address,
                "authentication_key": vasp.authentication_key().to_string(),
                "balances": [{"amount": 0_u64, "currency": "XUS"}],
                "delegated_key_rotation_capability": false,
                "delegated_withdrawal_capability": false,
                "is_frozen": false,
                "received_events_key": EventKey::new_from_address(&vasp.address(), 2),
                "role": {
                    "base_url": "",
                    "base_url_rotation_events_key": EventKey::new_from_address(&vasp.address(), 1),
                    "compliance_key": "",
                    "compliance_key_rotation_events_key": EventKey::new_from_address(&vasp.address(), 0),
                    "vasp_domains": [],
                    "expiration_time": 18446744073709551615_u64,
                    "human_name": human_name,
                    "num_children": 0,
                    "type": "parent_vasp",
                },
                "sent_events_key": EventKey::new_from_address(&vasp.address(), 3),
                "sequence_number": 0,
                "version": prev_version,
            }),
        );

        Ok(())
    }
}

struct ChildVaspAccountRole;

impl Test for ChildVaspAccountRole {
    fn name(&self) -> &'static str {
        "jsonrpc::child-vasp-account-role"
    }
}

impl PublicUsageTest for ChildVaspAccountRole {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();

        let mut parent = ctx.random_account();
        let child = ctx.random_account();
        ctx.create_parent_vasp_account(parent.authentication_key())?;
        env.submit_and_wait(&parent.sign_with_transaction_builder(
            factory.create_child_vasp_account(Currency::XUS, child.authentication_key(), false, 0),
        ));

        let address = format!("{:x}", child.address());
        let resp = env.send("get_account", json!([address]));
        let result = resp.result.unwrap();

        assert_eq!(
            result,
            json!({
                "address": address,
                "authentication_key": child.authentication_key(),
                "balances": [{"amount": 0_u64, "currency": "XUS"}],
                "delegated_key_rotation_capability": false,
                "delegated_withdrawal_capability": false,
                "is_frozen": false,
                "received_events_key": EventKey::new_from_address(&child.address(), 0),
                "role": {
                    "type": "child_vasp",
                    "parent_vasp_address": parent.address(),
                },
                "sent_events_key": EventKey::new_from_address(&child.address(), 1),
                "sequence_number": 0,
                "version": resp.diem_ledger_version,
            }),
        );

        Ok(())
    }
}

struct PeerToPeerWithEvents;

impl Test for PeerToPeerWithEvents {
    fn name(&self) -> &'static str {
        "jsonrpc::peer-to-peer-with-events"
    }
}

impl PublicUsageTest for PeerToPeerWithEvents {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();

        let prev_ledger_version = env.send("get_metadata", json!([])).diem_ledger_version;

        let (_parent1, mut child1_1, _child1_2) =
            env.create_parent_and_child_accounts(ctx, 3_000_000_000)?;
        let (_parent2, child2_1, _child2_2) =
            env.create_parent_and_child_accounts(ctx, 3_000_000_000)?;

        let txn = child1_1.sign_with_transaction_builder(factory.peer_to_peer(
            Currency::XUS,
            child2_1.address(),
            200_000,
        ));

        env.submit_and_wait(&txn);
        let txn_hex = hex::encode(bcs::to_bytes(&txn).expect("bcs txn failed"));

        let sender = &child1_1;
        let receiver = &child2_1;

        let resp = env.send(
            "get_account_transaction",
            json!([sender.address(), 0, true]),
        );
        let result = resp.result.unwrap();
        let version = result["version"].as_u64().unwrap();
        assert_eq!(
            true,
            version > prev_ledger_version && version <= resp.diem_ledger_version
        );

        let gas_used = result["gas_used"].as_u64().expect("exist as u64");
        let txn_hash = Transaction::UserTransaction(txn.clone()).hash().to_hex();

        let script = match txn.payload() {
            TransactionPayload::Script(s) => s,
            _ => unreachable!(),
        };
        let script_hash = diem_crypto::HashValue::sha3_256_of(script.code()).to_hex();
        let script_bytes = hex::encode(bcs::to_bytes(script).unwrap());

        assert_eq!(
            result,
            json!({
                "bytes": format!("00{}", txn_hex),
                "events": [
                    {
                        "data": {
                            "amount": {"amount": 200000_u64, "currency": "XUS"},
                            "metadata": "",
                            "receiver": format!("{:x}", receiver.address()),
                            "sender": format!("{:x}", sender.address()),
                            "type": "sentpayment"
                        },
                        "key": format!("0100000000000000{:x}", sender.address()),
                        "sequence_number": 0,
                        "transaction_version": version,
                    },
                    {
                        "data": {
                            "amount": {"amount": 200000_u64, "currency": "XUS"},
                            "metadata": "",
                            "receiver": format!("{:x}", receiver.address()),
                            "sender": format!("{:x}", sender.address()),
                            "type": "receivedpayment"
                        },
                        "key": format!("0000000000000000{:x}", receiver.address()),
                        "sequence_number": 1,
                        "transaction_version": version
                    }
                ],
                "gas_used": gas_used,
                "hash": txn_hash,
                "transaction": {
                    "chain_id": 4,
                    "expiration_timestamp_secs": txn.expiration_timestamp_secs(),
                    "gas_currency": "XUS",
                    "gas_unit_price": 0,
                    "max_gas_amount": 1000000,
                    "public_key": sender.public_key().to_string(),
                    "secondary_signers": [],
                    "secondary_signature_schemes": [],
                    "secondary_signatures": [],
                    "secondary_public_keys": [],
                    "script": {
                        "type": "peer_to_peer_with_metadata",
                        "type_arguments": [
                            "XUS"
                        ],
                        "arguments": [
                            format!("{{ADDRESS: {:?}}}", receiver.address()),
                            "{U64: 200000}",
                            "{U8Vector: 0x}",
                            "{U8Vector: 0x}"
                        ],
                        "code": hex::encode(script.code()),
                        "amount": 200000,
                        "currency": "XUS",
                        "metadata": "",
                        "metadata_signature": "",
                        "receiver": format!("{:x}", receiver.address()),
                    },
                    "script_bytes": script_bytes,
                    "script_hash": script_hash,
                    "sender": format!("{:x}", sender.address()),
                    "sequence_number": 0,
                    "signature": hex::encode(txn.authenticator().sender().signature_bytes()),
                    "signature_scheme": "Scheme::Ed25519",
                    "type": "user"
                },
                "version": version,
                "vm_status": {"type": "executed"}
            }),
            "{:#}",
            result,
        );

        Ok(())
    }
}

struct PeerToPeerErrorExplination;

impl Test for PeerToPeerErrorExplination {
    fn name(&self) -> &'static str {
        "jsonrpc::peer-to-peer-error-explination"
    }
}

impl PublicUsageTest for PeerToPeerErrorExplination {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let mut env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();
        let (_parent, mut child1, child2) =
            env.create_parent_and_child_accounts(ctx, 1_000_000_000)?;
        let txn = child1.sign_with_transaction_builder(factory.peer_to_peer(
            Currency::XUS,
            child2.address(),
            200000000000000,
        ));

        env.allow_execution_failures(|env| {
            env.submit_and_wait(&txn);
        });

        let sender = &child1;

        let resp = env.send(
            "get_account_transaction",
            json!([sender.address(), 0, true]),
        );
        let result = resp.result.unwrap();
        let vm_status = result["vm_status"].clone();
        assert_eq!(
            vm_status,
            json!({
                "abort_code": 1288,
                "explanation": {
                    "category": "LIMIT_EXCEEDED",
                    "category_description": " A limit on an amount, e.g. a currency, is exceeded. Example: withdrawal of money after account limits window\n is exhausted.",
                    "reason": "EINSUFFICIENT_BALANCE",
                    "reason_description": " The account does not hold a large enough balance in the specified currency"
                },
                "location": "00000000000000000000000000000001::DiemAccount",
                "type": "move_abort"
            })
        );

        Ok(())
    }
}

struct ReSubmittingTransactionWontFail;

impl Test for ReSubmittingTransactionWontFail {
    fn name(&self) -> &'static str {
        "jsonrpc::re-submitting-transaction-wont-fail"
    }
}

impl PublicUsageTest for ReSubmittingTransactionWontFail {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();
        let (_parent, mut child1, child2) =
            env.create_parent_and_child_accounts(ctx, 1_000_000_000)?;
        let txn = child1.sign_with_transaction_builder(factory.peer_to_peer(
            Currency::XUS,
            child2.address(),
            200,
        ));

        env.submit(&txn);
        env.submit(&txn);
        env.wait_for_txn(&txn);

        Ok(())
    }
}

struct MempoolValidationError;

impl Test for MempoolValidationError {
    fn name(&self) -> &'static str {
        "jsonrpc::mempool-validator-error"
    }
}

impl PublicUsageTest for MempoolValidationError {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let mut env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();
        let (_parent, child1, child2) = env.create_parent_and_child_accounts(ctx, 1_000_000_000)?;
        let txn1 = child1.sign_transaction(
            factory
                .peer_to_peer(Currency::XUS, child2.address(), 200)
                .sender(child1.address())
                .sequence_number(child1.sequence_number())
                .build(),
        );
        let txn2 = child1.sign_transaction(
            factory
                .peer_to_peer(Currency::XUS, child2.address(), 300)
                .sender(child1.address())
                .sequence_number(child1.sequence_number())
                .build(),
        );

        env.submit(&txn1);
        env.allow_execution_failures(|env| {
            let resp = env.submit(&txn2);
            assert_eq!(
                resp.error.expect("error").message,
                "Server error: Mempool submission error: \"Failed to update gas price to 0\""
                    .to_string(),
            );
        });
        env.wait_for_txn(&txn1);

        Ok(())
    }
}

struct ExpiredTransaction;

impl Test for ExpiredTransaction {
    fn name(&self) -> &'static str {
        "jsonrpc::expired-transaction"
    }
}

impl PublicUsageTest for ExpiredTransaction {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let mut env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();
        let (_parent, child1, child2) = env.create_parent_and_child_accounts(ctx, 1_000_000_000)?;

        env.allow_execution_failures(|env| {
            let txn = child1.sign_transaction(
                factory
                    .peer_to_peer(Currency::XUS, child2.address(), 200)
                    .sender(child1.address())
                    .sequence_number(child1.sequence_number() + 100)
                    .expiration_timestamp_secs(0)
                    .build(),
            );
            let resp = env.submit(&txn);
            assert_eq!(
                resp.error.expect("error").message,
                "Server error: VM Validation error: TRANSACTION_EXPIRED".to_string(),
            );
        });
        Ok(())
    }
}

struct PreburnAndBurnEvents;

impl Test for PreburnAndBurnEvents {
    fn name(&self) -> &'static str {
        "jsonrpc::preburn-and-burn-events"
    }
}

impl AdminTest for PreburnAndBurnEvents {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());
        let factory = ctx.chain_info().transaction_factory();
        let mut dd = ctx.random_account();
        ctx.chain_info()
            .create_designated_dealer_account(Currency::XUS, dd.authentication_key())?;
        ctx.chain_info().fund(Currency::XUS, dd.address(), 1000)?;

        let script = stdlib::encode_preburn_script(Currency::XUS.type_tag(), 100);
        let txn = dd.sign_with_transaction_builder(factory.script(script.clone()));
        let result = env.submit_and_wait(&txn);
        let version = result["version"].as_u64().unwrap();

        assert_eq!(
            result["events"],
            json!([
                {
                    "data": {
                        "amount": {"amount": 100, "currency": "XUS"},
                        "metadata": "",
                        "receiver": dd.address(),
                        "sender": dd.address(),
                        "type": "sentpayment"
                    },
                    "key": EventKey::new_from_address(&dd.address(), 4),
                    "sequence_number": result["events"][0]["sequence_number"].as_u64().unwrap(),
                    "transaction_version": version
                },
                {
                    "data": {
                        "amount": {"amount": 100, "currency": "XUS"},
                        "preburn_address": dd.address(),
                        "type": "preburn"
                    },
                    "key": "07000000000000000000000000000000000000000a550c18",
                    "sequence_number": result["events"][1]["sequence_number"].as_u64().unwrap(),
                    "transaction_version": version
                }
            ]),
            "{:#?}",
            result["events"]
        );
        assert_eq!(
            result["transaction"]["script"],
            json!({
                "type_arguments": [
                    "XUS"
                ],
                "arguments": [
                    "{U64: 100}",
                ],
                "code": hex::encode(script.code()),
                "type": "preburn"
            }),
            "{}",
            result["transaction"]
        );

        let script = stdlib::encode_burn_with_amount_script_function(
            Currency::XUS.type_tag(),
            0,
            dd.address(),
            100,
        );
        let burn_txn = ctx
            .chain_info()
            .treasury_compliance_account()
            .sign_with_transaction_builder(factory.payload(script));
        let result = env.submit_and_wait(&burn_txn);
        let version = result["version"].as_u64().unwrap();
        assert_eq!(
            result["events"],
            json!([{
                "data":{
                    "amount":{"amount":100,"currency":"XUS"},
                    "preburn_address": dd.address(),
                    "type":"burn"
                },
                "key":"06000000000000000000000000000000000000000a550c18",
                "sequence_number":0,
                "transaction_version":version
            }]),
            "{:#?}",
            result["events"]
        );
        assert_eq!(
            result["transaction"]["script"],
            json!({
                "type_arguments": [
                    "XUS"
                ],
                "arguments_bcs": [
                    "0000000000000000",
                    dd.address(),
                    "6400000000000000",
                ],
                "type": "script_function",
                "module_address":"00000000000000000000000000000001",
                "module_name":"TreasuryComplianceScripts",
                "function_name":"burn_with_amount",
            }),
            "{:#?}",
            result["transaction"]
        );

        Ok(())
    }
}

struct CancleBurnEvent;

impl Test for CancleBurnEvent {
    fn name(&self) -> &'static str {
        "jsonrpc::cancel-burn-event"
    }
}

impl AdminTest for CancleBurnEvent {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());
        let factory = ctx.chain_info().transaction_factory();
        let mut dd = ctx.random_account();
        ctx.chain_info()
            .create_designated_dealer_account(Currency::XUS, dd.authentication_key())?;
        ctx.chain_info().fund(Currency::XUS, dd.address(), 1000)?;

        let txn = dd.sign_with_transaction_builder(factory.preburn(Currency::XUS, 100));
        env.submit_and_wait(&txn);

        let cancel_burn_txn = ctx
            .chain_info()
            .treasury_compliance_account()
            .sign_with_transaction_builder(factory.cancel_burn_with_amount(
                Currency::XUS,
                dd.address(),
                100,
            ));

        let result = env.submit_and_wait(&cancel_burn_txn);
        let version = result["version"].as_u64().unwrap();
        assert_eq!(
            result["events"],
            json!([
                {
                    "data":{
                        "amount":{"amount":100,"currency":"XUS"},
                        "preburn_address": dd.address(),
                        "type":"cancelburn"
                    },
                    "key":"08000000000000000000000000000000000000000a550c18",
                    "sequence_number":0,
                    "transaction_version":version
                },
                {
                    "data":{
                        "amount":{"amount":100,"currency":"XUS"},
                        "metadata":"",
                        "receiver": dd.address(),
                        "sender": dd.address(),
                        "type":"receivedpayment"
                    },
                    "key": EventKey::new_from_address(&dd.address(), 3),
                    "sequence_number":1,
                    "transaction_version":version
                }
            ]),
            "{}",
            result["events"]
        );
        assert_eq!(
            result["transaction"]["script"],
            json!({
                "type_arguments": [
                    "XUS"
                ],
                "arguments_bcs": [
                    dd.address(),
                    "6400000000000000",
                ],
                "type": "script_function",
                "module_address":"00000000000000000000000000000001",
                "module_name":"TreasuryComplianceScripts",
                "function_name":"cancel_burn_with_amount",
            }),
            "{}",
            result["transaction"]
        );

        Ok(())
    }
}

struct UpdateExchangeRateEvent;

impl Test for UpdateExchangeRateEvent {
    fn name(&self) -> &'static str {
        "jsonrpc::update-exchange-rate-event"
    }
}

impl AdminTest for UpdateExchangeRateEvent {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());
        let factory = ctx.chain_info().transaction_factory();
        let script = stdlib::encode_update_exchange_rate_script(xus_tag(), 0, 1, 4);
        let txn = ctx
            .chain_info()
            .treasury_compliance_account()
            .sign_with_transaction_builder(factory.script(script.clone()));

        let result = env.submit_and_wait(&txn);
        let version = result["version"].as_u64().unwrap();
        assert_eq!(
            result["events"],
            json!([{
                "data":{
                    "currency_code":"XUS",
                    "new_to_xdx_exchange_rate":0.25,
                    "type":"to_xdx_exchange_rate_update"
                },
                "key":"09000000000000000000000000000000000000000a550c18",
                "sequence_number":0,
                "transaction_version":version
            }]),
            "{}",
            result["events"]
        );
        assert_eq!(
            result["transaction"]["script"],
            json!({
                "type_arguments": [
                    "XUS"
                ],
                "arguments": [
                    "{U64: 0}",
                    "{U64: 1}",
                    "{U64: 4}"
                ],
                "code": hex::encode(script.code()),
                "type": "update_exchange_rate"
            }),
            "{}",
            result["transaction"]
        );

        // Reset exchange rate
        let txn = ctx
            .chain_info()
            .treasury_compliance_account()
            .sign_with_transaction_builder(factory.update_exchange_rate(Currency::XUS, 0, 1, 1));

        env.submit_and_wait(&txn);

        Ok(())
    }
}

struct MintAndReceivedMintEvents;

impl Test for MintAndReceivedMintEvents {
    fn name(&self) -> &'static str {
        "jsonrpc::mint-and-received-mint-events"
    }
}

impl AdminTest for MintAndReceivedMintEvents {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());
        let factory = ctx.chain_info().transaction_factory();
        let script = stdlib::encode_tiered_mint_script(
            xus_tag(),
            0,
            ctx.chain_info().designated_dealer_account().address(),
            1_000_000,
            1,
        );
        let txn = ctx
            .chain_info()
            .treasury_compliance_account()
            .sign_with_transaction_builder(factory.script(script.clone()));

        let result = env.submit_and_wait(&txn);
        let version = result["version"].as_u64().unwrap();
        assert_eq!(
            result["events"],
            json!([
                {
                    "data":{
                        "amount":{"amount":1000000,"currency":"XUS"},
                        "destination_address":"000000000000000000000000000000dd",
                        "type":"receivedmint"
                    },
                    "key":"0000000000000000000000000000000000000000000000dd",
                    "sequence_number":1,
                    "transaction_version":version
                },
                {
                    "data":{
                        "amount":{"amount":1000000,"currency":"XUS"},
                        "type":"mint"
                    },
                    "key":"05000000000000000000000000000000000000000a550c18",
                    "sequence_number":1,
                    "transaction_version":version
                },
                {
                    "data":{
                        "amount":{"amount":1000000,"currency":"XUS"},
                        "metadata":"",
                        "receiver":"000000000000000000000000000000dd",
                        "sender":"00000000000000000000000000000000",
                        "type":"receivedpayment"
                    },
                    "key":"0300000000000000000000000000000000000000000000dd",
                    "sequence_number":1,
                    "transaction_version":version
                }
            ]),
            "{:#?}",
            result["events"]
        );
        assert_eq!(
            result["transaction"]["script"],
            json!({
                "type_arguments": [
                    "XUS"
                ],
                "arguments": [
                    "{U64: 0}",
                    "{ADDRESS: 000000000000000000000000000000DD}",
                    "{U64: 1000000}",
                    "{U64: 1}",
                ],
                "code": hex::encode(script.code()),
                "type": "tiered_mint"
            }),
            "{}",
            result["transaction"]
        );
        Ok(())
    }
}

struct RotateComplianceKeyEvent;

impl Test for RotateComplianceKeyEvent {
    fn name(&self) -> &'static str {
        "jsonrpc::rotate-compliance-key-event"
    }
}

impl PublicUsageTest for RotateComplianceKeyEvent {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let factory = ctx.transaction_factory();
        let (mut parent, _child1, _child2) =
            env.create_parent_and_child_accounts(ctx, 1_000_000_000)?;
        let compliance_key = AccountKey::generate(ctx.rng());

        let txn = parent.sign_with_transaction_builder(factory.rotate_dual_attestation_info(
            b"http://hello.com".to_vec(),
            compliance_key.public_key().to_bytes().to_vec(),
        ));

        let result = env.submit_and_wait(&txn);
        let version = result["version"].as_u64().unwrap();
        let rotated_seconds = result["events"][0]["data"]["time_rotated_seconds"]
            .as_u64()
            .unwrap();
        assert_eq!(
            result["events"],
            json!([
                {
                    "data":{
                        "new_base_url":"http://hello.com",
                        "time_rotated_seconds": rotated_seconds,
                        "type":"baseurlrotation"
                    },
                    "key": format!("0100000000000000{:x}", parent.address()),
                    "sequence_number":0,
                    "transaction_version":version
                },
                {
                    "data":{
                        "new_compliance_public_key": hex::encode(compliance_key.public_key().to_bytes()),
                        "time_rotated_seconds": rotated_seconds,
                        "type":"compliancekeyrotation"
                    },
                    "key": format!("0000000000000000{:x}", parent.address()),
                    "sequence_number":0,
                    "transaction_version":version
                }
            ]),
            "{}",
            result["events"]
        );

        Ok(())
    }
}

struct CreateAccountEvent;

impl Test for CreateAccountEvent {
    fn name(&self) -> &'static str {
        "jsonrpc::create-account-event"
    }
}

impl PublicUsageTest for CreateAccountEvent {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let response = env.send(
            "get_events",
            json!(["00000000000000000000000000000000000000000a550c18", 0, 2]),
        );
        let events = response.result.unwrap();
        assert_eq!(
            events,
            json!([
                {
                    "data":{
                        "created_address":"0000000000000000000000000a550c18",
                        "role_id":0,
                        "type":"createaccount"
                    },
                    "key":"00000000000000000000000000000000000000000a550c18",
                    "sequence_number":0,
                    "transaction_version":0
                },
                {
                    "data":{
                        "created_address":"0000000000000000000000000b1e55ed",
                        "role_id":1,
                        "type":"createaccount"
                    },
                    "key":"00000000000000000000000000000000000000000a550c18",
                    "sequence_number":1,
                    "transaction_version":0
                },
            ]),
            "{}",
            events
        );
        Ok(())
    }
}

struct GetTransactionsWithoutEvents;

impl Test for GetTransactionsWithoutEvents {
    fn name(&self) -> &'static str {
        "jsonrpc::get-transactions-without-events"
    }
}

impl PublicUsageTest for GetTransactionsWithoutEvents {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let response = env.send("get_transactions", json!([0, 1000, false]));
        let txns = response.result.unwrap();
        assert!(!txns.as_array().unwrap().is_empty());

        for (index, txn) in txns.as_array().unwrap().iter().enumerate() {
            assert_eq!(txn["version"], index);
            assert_eq!(txn["events"], json!([]));
        }
        Ok(())
    }
}

struct GetAccountTransactionsWithoutEvents;

impl Test for GetAccountTransactionsWithoutEvents {
    fn name(&self) -> &'static str {
        "jsonrpc::get-account-transactions-without-events"
    }
}

impl PublicUsageTest for GetAccountTransactionsWithoutEvents {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let account = ctx.random_account();
        ctx.create_parent_vasp_account(account.authentication_key())?;
        ctx.fund(account.address(), 10)?;
        let response = env.send(
            "get_account_transactions",
            json!([treasury_compliance_account_address(), 0, 1000, false]),
        );
        let txns = response.result.unwrap();
        assert!(!txns.as_array().unwrap().is_empty());

        for txn in txns.as_array().unwrap() {
            assert_eq!(txn["events"], json!([]));
        }
        Ok(())
    }
}

struct GetAccountTransactionsWithProofs;

impl Test for GetAccountTransactionsWithProofs {
    fn name(&self) -> &'static str {
        "jsonrpc::get-account-transactions-with-proofs"
    }
}

impl PublicUsageTest for GetAccountTransactionsWithProofs {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let response = env.send(
            "get_account_transactions_with_proofs",
            json!([treasury_compliance_account_address(), 0, 1000, false]),
        );
        // Just check that the responses deserialize correctly, we'll let
        // the verifying client tests handle the proof checking.
        let value = response.result.unwrap();
        let view = serde_json::from_value::<AccountTransactionsWithProofView>(value).unwrap();
        let _txns = AccountTransactionsWithProof::try_from(&view).unwrap();
        Ok(())
    }
}

struct GetTransactionsWithProofs;

impl Test for GetTransactionsWithProofs {
    fn name(&self) -> &'static str {
        "jsonrpc::get-transactions-with-proofs"
    }
}

impl PublicUsageTest for GetTransactionsWithProofs {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());

        // Fund and account to ensure there are some txns on the chain
        let account = ctx.random_account();
        ctx.create_parent_vasp_account(account.authentication_key())?;
        ctx.fund(account.address(), 10)?;

        let resp = env.send("get_metadata", json!([]));

        let limit = 10;
        let include_events = false;
        // We test 2 cases:
        //      1. base_version + limit > resp.diem_ledger_version
        //      2. base_version + limit < resp.diem_ledger_version
        for base_version in &[resp.diem_ledger_version, 0] {
            // We use a batched call to ensure we get an answer using the same latest server ledger_info for both
            let responses = env.send_request(json!([
                        {"jsonrpc": "2.0", "method": "get_state_proof", "params": json!([0]), "id": 1},
                        {"jsonrpc": "2.0", "method": "get_transactions_with_proofs", "params": json!([*base_version, limit, include_events]), "id": 2}
                    ]));

            let f: Vec<serde_json::Value> =
                serde_json::from_value(responses).expect("should be valid serde_json::Value");
            let data = &f.iter().find(|g| g["id"] == 2).unwrap()["result"];
            let proofs = data["proofs"].as_object().unwrap();

            // We want to verify the signatures of the LedgerInfo that will be returned by the
            // get_transactions_with_proofs call to be sure it's valid, but
            // since we don't have a local state with the set of validators unlike an actual client,
            // we need to get the validator set from the batched get_state_proof call.
            let ledger_info_view = &f.iter().find(|g| g["id"] == 1).unwrap()["result"];
            let ep_cp = ledger_info_view["epoch_change_proof"].as_str().unwrap();
            let epoch_proofs: diem_types::epoch_change::EpochChangeProof =
                bcs::from_bytes(&hex::decode(&ep_cp).unwrap()).unwrap();
            let some_li: Vec<_> = epoch_proofs.ledger_info_with_sigs;
            assert!(!some_li.is_empty());
            // We use the last one (but the validator set does not change in the tests and
            // in practice the epoch change proofs should be verified).
            let validator_set = &some_li
                .last()
                .unwrap()
                .ledger_info()
                .next_epoch_state()
                .unwrap()
                .verifier;

            // The actual proofs
            let raw_hex_li = proofs["ledger_info_to_transaction_infos_proof"]
                .as_str()
                .unwrap();
            let li_to_tip: diem_types::proof::TransactionAccumulatorRangeProof =
                bcs::from_bytes(&hex::decode(&raw_hex_li).unwrap()).unwrap();
            // The txs for which we got the proofs
            let raw_hex_txs = proofs["transaction_infos"].as_str().unwrap();
            let txs_infos: Vec<diem_types::transaction::TransactionInfo> =
                bcs::from_bytes(&hex::decode(&raw_hex_txs).unwrap()).unwrap();
            let hashes: Vec<_> = txs_infos.iter().map(CryptoHash::hash).collect();

            // We make sure we have between 1 and 10 txs
            if hashes.len() > 10 || hashes.is_empty() {
                panic!(
                    "Unexpected hash len returned at {} by get_transactions_with_proofs: {}",
                    base_version,
                    hashes.len()
                );
            }

            // We must check the transactions we got correspond to the hashes in the proofs
            let raw_blobs = data["serialized_transactions"].as_array().unwrap();
            assert!(!raw_blobs.is_empty());
            let actual_txs: Vec<Transaction> = raw_blobs
                .iter()
                .map(|tx| bcs::from_bytes(&hex::decode(&tx.as_str().unwrap()).unwrap()).unwrap())
                .collect();
            assert!(!actual_txs.is_empty());
            assert_eq!(txs_infos.len(), actual_txs.len());
            for (index, tx) in actual_txs.iter().enumerate() {
                // Notice we need to actually hash the transaction to be sure its hash is correct
                assert_eq!(tx.hash(), txs_infos[index].transaction_hash());
            }

            // We compare our results with the non-veryfing API for the test
            let resp_tx = env.send(
                "get_transactions",
                json!([*base_version, txs_infos.len(), false]),
            );
            let no_proof_txns = resp_tx.result.unwrap();
            assert!(!no_proof_txns.as_array().unwrap().is_empty());
            assert_eq!(no_proof_txns.as_array().unwrap().len(), actual_txs.len());
            for (index, tx) in no_proof_txns.as_array().unwrap().iter().enumerate() {
                assert_eq!(
                    tx["hash"].as_str().unwrap(),
                    actual_txs[index].hash().to_hex()
                );
            }

            // We need to get the details required to verify the proof from the batched get_state_proof call
            let li_raw = ledger_info_view["ledger_info_with_signatures"]
                .as_str()
                .unwrap();
            let li: LedgerInfoWithSignatures =
                bcs::from_bytes(&hex::decode(&li_raw).unwrap()).unwrap();
            let expected_hash = li.ledger_info().transaction_accumulator_hash();

            // and we verify the signature of the provided ledger info that provided the accumulator hash
            assert!(li.verify_signatures(&validator_set).is_ok());

            // and we eventually verify the proofs for the transactions
            assert!(li_to_tip
                .verify(expected_hash, Some(*base_version), &hashes)
                .is_ok());
        }
        Ok(())
    }
}

struct AddAndRemoveVaspDomain;

impl Test for AddAndRemoveVaspDomain {
    fn name(&self) -> &'static str {
        "jsonrpc::add-and-remove-vasp-domain"
    }
}

impl AdminTest for AddAndRemoveVaspDomain {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());
        let factory = ctx.chain_info().transaction_factory();

        let vasp = ctx.random_account();
        ctx.chain_info()
            .create_parent_vasp_account(Currency::XUS, vasp.authentication_key())?;

        // add domain
        let domain = DiemIdVaspDomainIdentifier::new(&"diem")
            .unwrap()
            .as_str()
            .as_bytes()
            .to_vec();

        let txn = ctx
            .chain_info()
            .treasury_compliance_account()
            .sign_with_transaction_builder(factory.add_vasp_domain(vasp.address(), domain.clone()));

        let result = env.submit_and_wait(&txn);
        let version1 = result["version"].as_u64().unwrap();

        // get account
        let address = format!("{:x}", vasp.address());
        let resp = env.send("get_account", json!([address]));
        let result = resp.result.unwrap();
        assert_eq!(result["role"]["vasp_domains"], json!(["diem"]),);

        // remove domain
        let txn = ctx
            .chain_info()
            .treasury_compliance_account()
            .sign_with_transaction_builder(factory.remove_vasp_domain(vasp.address(), domain));

        let result = env.submit_and_wait(&txn);
        let version2 = result["version"].as_u64().unwrap();

        // get account
        let resp = env.send("get_account", json!([address]));
        let result = resp.result.unwrap();
        assert_eq!(result["role"]["vasp_domains"], json!([]),);

        // get event
        let tc_address = format!("{:x}", treasury_compliance_account_address());
        let resp = env.send("get_account", json!([tc_address]));
        let result = resp.result.unwrap();
        let vasp_domain_events_key = result["role"]["vasp_domain_events_key"].clone();
        let response = env.send("get_events", json!([vasp_domain_events_key, 0, 3]));
        let events = response.result.unwrap();
        assert_eq!(
            events,
            json!([
                {
                    "data":{
                        "domain": "diem",
                        "removed": false,
                        "address": address,
                        "type":"vaspdomain"
                    },
                    "key": format!("0000000000000000{}", tc_address),
                    "sequence_number": 0,
                    "transaction_version": version1,
                },
                {
                    "data":{
                        "domain": "diem",
                        "removed": true,
                        "address": address,
                        "type":"vaspdomain"
                    },
                    "key": format!("0000000000000000{}", tc_address),
                    "sequence_number": 1,
                    "transaction_version": version2,
                },
            ]),
        );
        Ok(())
    }
}

struct GetTreasuryComplianceAccount;

impl Test for GetTreasuryComplianceAccount {
    fn name(&self) -> &'static str {
        "jsonrpc::get-treasury-compliance-account"
    }
}

impl PublicUsageTest for GetTreasuryComplianceAccount {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());

        let address = format!("{:x}", treasury_compliance_account_address());
        let resp = env.send("get_account", json!([address]));
        let result = resp.result.unwrap();
        let authentication_key = result["authentication_key"].as_str().unwrap();
        let sequence_number = result["sequence_number"].as_u64().unwrap();
        assert_eq!(
            result,
            json!({
                "address": address,
                "authentication_key": authentication_key,
                "balances": [],
                "delegated_key_rotation_capability": false,
                "delegated_withdrawal_capability": false,
                "is_frozen": false,
                "received_events_key": format!("0100000000000000{}", address),
                "role": {
                    "vasp_domain_events_key": format!("0000000000000000{}", address),
                    "type": "treasury_compliance",
                },
                "sent_events_key": format!("0200000000000000{}", address),
                "sequence_number": sequence_number,
                "version": resp.diem_ledger_version,
            }),
        );
        Ok(())
    }
}

struct GetEventsWithProofs;

impl Test for GetEventsWithProofs {
    fn name(&self) -> &'static str {
        "jsonrpc::get-events-with-proofs"
    }
}

impl PublicUsageTest for GetEventsWithProofs {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());

        // Fund and account to ensure there are some txns on the chain
        let account = ctx.random_account();
        ctx.create_parent_vasp_account(account.authentication_key())?;
        ctx.fund(account.address(), 10)?;

        let responses = env.send_request(json!([
                    {"jsonrpc": "2.0", "method": "get_state_proof", "params": json!([0]), "id": 1},
                    {"jsonrpc": "2.0", "method": "get_events_with_proofs", "params": json!(["00000000000000000000000000000000000000000a550c18", 0, 3]), "id": 2}
                ]));

        let resps: Vec<serde_json::Value> =
            serde_json::from_value(responses).expect("should be valid serde_json::Value");

        // we need te get the current ledger_info in order to verify the events
        let ledger_info_view = &resps.iter().find(|g| g["id"] == 1).unwrap()["result"];
        let li_raw = ledger_info_view["ledger_info_with_signatures"]
            .as_str()
            .unwrap();
        let li: LedgerInfoWithSignatures = bcs::from_bytes(&hex::decode(&li_raw).unwrap()).unwrap();
        // We want to verify the signatures of the LedgerInfo to be sure it's valid, but
        // since we don't have a local state with the set of validators unlike an actual client,
        // we need to get the validator set from the batched get_state_proof call.
        let ep_cp = ledger_info_view["epoch_change_proof"].as_str().unwrap();
        let epoch_proofs: diem_types::epoch_change::EpochChangeProof =
            bcs::from_bytes(&hex::decode(&ep_cp).unwrap()).unwrap();
        let some_li: Vec<_> = epoch_proofs.ledger_info_with_sigs;
        assert!(!some_li.is_empty());
        // We use the last one (but the validator set does not change in the tests and
        // in practice the epoch change proofs should be verified).
        let validator_set = &some_li
            .last()
            .unwrap()
            .ledger_info()
            .next_epoch_state()
            .unwrap()
            .verifier;
        // And we verify the signature
        assert!(li.verify_signatures(&validator_set).is_ok());

        // We now need to verify the events using this verified ledger_info:
        let ledger_info = li.ledger_info();
        let data = &resps.iter().find(|g| g["id"] == 2).unwrap()["result"]
            .as_array()
            .unwrap();
        let mut events: Vec<EventView> = vec![];
        for d in data.iter() {
            let bcs_data = d["event_with_proof"].as_str().unwrap();
            let event: EventWithProof = bcs::from_bytes(&hex::decode(&bcs_data).unwrap()).unwrap();
            let hash = event.event.hash();

            // We verify the proof of the event
            assert!(event
                .proof
                .verify(
                    ledger_info,
                    hash,
                    event.transaction_version,
                    event.event_index
                )
                .is_ok());

            // We can now use our verified events
            events.push((event.transaction_version, event.event).try_into().unwrap());
        }

        assert_eq!(events.len(), 3);

        Ok(())
    }
}

struct MultiAgentPaymentOverDualAttestationLimit;

impl Test for MultiAgentPaymentOverDualAttestationLimit {
    fn name(&self) -> &'static str {
        "jsonrpc::multi-agent-payment-over-dual-attestation-limit"
    }
}

impl PublicUsageTest for MultiAgentPaymentOverDualAttestationLimit {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());

        let limit = env.get_metadata()["dual_attestation_limit"]
            .as_u64()
            .unwrap();
        let amount = limit + 1;

        let (_parent1, mut sender, _child1_2) =
            env.create_parent_and_child_accounts(ctx, limit + 3_000_000_000)?;
        let (_parent2, mut receiver, _child2_2) =
            env.create_parent_and_child_accounts(ctx, limit + 3_000_000_000)?;

        let sender_balance = env.get_balance(sender.address(), "XUS");
        let receiver_balance = env.get_balance(receiver.address(), "XUS");

        let script =
            diem_transaction_builder::stdlib::encode_peer_to_peer_by_signers_script_function(
                xus_tag(),
                amount,
                vec![],
            );
        let txn = env.create_multi_agent_txn(&mut sender, &[&mut receiver], script);

        let txn_view = env.submit_and_wait(&txn);

        let events = txn_view["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["data"]["type"], "sentpayment");
        assert_eq!(events[1]["data"]["type"], "receivedpayment");
        for event in events.iter() {
            assert_eq!(
                event["data"]["amount"],
                json!({"amount": amount, "currency": "XUS"})
            );
            assert_eq!(event["data"]["sender"], format!("{:x}", sender.address()));
            assert_eq!(
                event["data"]["receiver"],
                format!("{:x}", receiver.address())
            );
        }

        assert_eq!(
            sender_balance - amount,
            env.get_balance(sender.address(), "XUS")
        );
        assert_eq!(
            receiver_balance + amount,
            env.get_balance(receiver.address(), "XUS")
        );

        Ok(())
    }
}

struct MultiAgentRotateAuthenticationKeyAdminScriptFunction;

impl Test for MultiAgentRotateAuthenticationKeyAdminScriptFunction {
    fn name(&self) -> &'static str {
        "jsonrpc::multi-agent-rotate-authentication-key-admin-script-function"
    }
}

impl AdminTest for MultiAgentRotateAuthenticationKeyAdminScriptFunction {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());

        // ScriptFunction
        let mut vasp = ctx.random_account();

        ctx.chain_info()
            .create_parent_vasp_account(Currency::XUS, vasp.authentication_key())?;

        let new_key = AccountKey::generate(ctx.rng());
        let txn = env.create_multi_agent_txn(
            ctx.chain_info().root_account(),
            &[&mut vasp],
            stdlib::encode_rotate_authentication_key_with_nonce_admin_script_function(
                0,
                new_key.public_key().to_bytes().to_vec(),
            ),
        );
        let result = env.submit_and_wait(&txn);
        let script = match txn.payload() {
            TransactionPayload::ScriptFunction(s) => s,
            _ => unreachable!(),
        };
        let script_hash = diem_crypto::HashValue::zero().to_hex();
        let script_bytes = hex::encode(bcs::to_bytes(script).unwrap());
        assert_eq!(result["vm_status"], json!({"type": "executed"}));
        assert_eq!(
            result["transaction"],
            json!({
                "type": "user",
                "sender": format!("{:x}", ctx.chain_info().root_account().address()),
                "signature_scheme": "Scheme::Ed25519",
                "signature": hex::encode(txn.authenticator().sender().signature_bytes()),
                "public_key": ctx.chain_info().root_account().public_key(),
                "secondary_signers": [ format!("{:x}", vasp.address()) ],
                "secondary_signature_schemes": [ "Scheme::Ed25519" ],
                "secondary_signatures": [ hex::encode(txn.authenticator().secondary_signers()[0].signature_bytes())],
                "secondary_public_keys": [ vasp.public_key().to_string() ],
                "sequence_number": ctx.chain_info().root_account().sequence_number() - 1,
                "chain_id": ctx.chain_info().chain_id(),
                "max_gas_amount": 1000000,
                "gas_unit_price": 0,
                "gas_currency": "XUS",
                "expiration_timestamp_secs": txn.expiration_timestamp_secs(),
                "script_hash": script_hash,
                "script_bytes": script_bytes,
                "script": {
                    "type": "script_function",
                    "arguments_bcs": vec![ "0000000000000000", &hex::encode(bcs::to_bytes(&new_key.public_key()).unwrap())],
                    "type_arguments": [],
                    "module_address": "00000000000000000000000000000001",
                    "module_name": "AccountAdministrationScripts",
                    "function_name": "rotate_authentication_key_with_nonce_admin"
                },
            }),
        );

        Ok(())
    }
}

struct MultiAgentRotateAuthenticationKeyAdminScript;

impl Test for MultiAgentRotateAuthenticationKeyAdminScript {
    fn name(&self) -> &'static str {
        "jsonrpc::multi-agent-rotate-authentication-key-admin-script"
    }
}

impl AdminTest for MultiAgentRotateAuthenticationKeyAdminScript {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());

        // Script
        let mut vasp = ctx.random_account();

        ctx.chain_info()
            .create_parent_vasp_account(Currency::XUS, vasp.authentication_key())?;
        let new_key = AccountKey::generate(ctx.rng());
        let txn = env.create_multi_agent_txn(
            ctx.chain_info().root_account(),
            &[&mut vasp],
            TransactionPayload::Script(
                stdlib::encode_rotate_authentication_key_with_nonce_admin_script(
                    0,
                    new_key.public_key().to_bytes().to_vec(),
                ),
            ),
        );

        let result = env.submit_and_wait(&txn);
        let script = match txn.payload() {
            TransactionPayload::Script(s) => s,
            _ => unreachable!(),
        };
        let script_hash = diem_crypto::HashValue::sha3_256_of(script.code()).to_hex();
        let script_bytes = hex::encode(bcs::to_bytes(script).unwrap());
        assert_eq!(result["vm_status"], json!({"type": "executed"}));
        assert_eq!(
            result["transaction"],
            json!({
                "type": "user",
                "sender": format!("{:x}", ctx.chain_info().root_account().address()),
                "signature_scheme": "Scheme::Ed25519",
                "signature": hex::encode(txn.authenticator().sender().signature_bytes()),
                "public_key": ctx.chain_info().root_account().public_key(),
                "secondary_signers": [ format!("{:x}", vasp.address()) ],
                "secondary_signature_schemes": [ "Scheme::Ed25519" ],
                "secondary_signatures": [ hex::encode(txn.authenticator().secondary_signers()[0].signature_bytes())],
                "secondary_public_keys": [ vasp.public_key().to_string() ],
                "sequence_number": ctx.chain_info().root_account().sequence_number() - 1,
                "chain_id": ctx.chain_info().chain_id(),
                "max_gas_amount": 1000000,
                "gas_unit_price": 0,
                "gas_currency": "XUS",
                "expiration_timestamp_secs": txn.expiration_timestamp_secs(),
                "script_hash": script_hash,
                "script_bytes": script_bytes,
                "script": {
                    "type_arguments": [],
                    "arguments": [
                        "{U64: 0}",
                        format!("{{U8Vector: 0x{}}}", new_key.public_key().to_string()),
                    ],
                    "code": hex::encode(script.code()),
                    "type": "rotate_authentication_key_with_nonce_admin"
                },
            }),
        );

        Ok(())
    }
}

struct GetAccumulatorConsistencyProof;

impl Test for GetAccumulatorConsistencyProof {
    fn name(&self) -> &'static str {
        "jsonrpc::get-accumulator-consistency-proof"
    }
}

impl PublicUsageTest for GetAccumulatorConsistencyProof {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        // batch request
        let resp = env.send_request(json!([
            {"jsonrpc": "2.0", "method": "get_metadata", "params": [], "id": 1},
            // leave both params empty to get the full accumulator summary
            {"jsonrpc": "2.0", "method": "get_accumulator_consistency_proof", "params": [], "id": 2},
        ]));

        // extract both responses
        let resps: Vec<serde_json::Value> =
            serde_json::from_value(resp).expect("should be valid serde_json::Value");
        let metadata = &resps.iter().find(|g| g["id"] == 1).unwrap()["result"];
        let proof_view = &resps.iter().find(|g| g["id"] == 2).unwrap()["result"];

        // get the root hash and version from the metadata response
        let metadata_root_hash =
            HashValue::from_str(metadata["accumulator_root_hash"].as_str().unwrap()).unwrap();
        let version = metadata["version"].as_u64().unwrap();

        // parse the consistency proof and build the accumulator
        let proof_view =
            serde_json::from_value::<AccumulatorConsistencyProofView>(proof_view.clone()).unwrap();
        let proof = AccumulatorConsistencyProof::try_from(&proof_view).unwrap();
        let accumulator =
            TransactionAccumulatorSummary::try_from_genesis_proof(proof, version).unwrap();

        // root hash from metadata and the computed root hash from the
        // accumulator summary should match
        assert_eq!(metadata_root_hash, accumulator.root_hash());
        Ok(())
    }
}

struct NoUnknownEvents;

impl Test for NoUnknownEvents {
    fn name(&self) -> &'static str {
        "jsonrpc::no-unknwon-events"
    }
}

impl PublicUsageTest for NoUnknownEvents {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.url().to_owned());
        let response = env.send("get_transactions", json!([0, 1000, true]));
        let txns = response.result.unwrap();
        for txn in txns.as_array().unwrap() {
            for event in txn["events"].as_array().unwrap() {
                let event_type = event["data"]["type"].as_str().unwrap();
                assert_ne!(event_type, "unknown", "{}", event);
            }
        }
        Ok(())
    }
}

struct UpgradeEventAndNewEpoch;

impl Test for UpgradeEventAndNewEpoch {
    fn name(&self) -> &'static str {
        "jsonrpc::upgrade-event-and-new-epoch"
    }
}

impl AdminTest for UpgradeEventAndNewEpoch {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());
        let factory = ctx.chain_info().transaction_factory();

        let write_set = ChangeSet::new(create_common_write_set(), vec![]);
        let txn = ctx
            .chain_info()
            .root_account()
            .sign_with_transaction_builder(factory.payload(TransactionPayload::WriteSet(
                WriteSetPayload::Direct(write_set),
            )));
        let result = env.submit_and_wait(&txn);
        let version = result["version"].as_u64().unwrap();
        let committed_time = result["events"][0]["data"]["committed_timestamp_secs"]
            .as_u64()
            .unwrap();
        let epoch = result["events"][1]["data"]["epoch"].as_u64().unwrap();
        assert!(committed_time != 0);
        assert_eq!(
            result["events"],
            json!([
                {
                    "data":{
                        "type": "admintransaction",
                        "committed_timestamp_secs": committed_time,
                    },
                    "key": "01000000000000000000000000000000000000000a550c18",
                    "sequence_number": result["events"][0]["sequence_number"].as_u64().unwrap(),
                    "transaction_version": version
                },
                {
                    "data":{
                        "epoch": epoch,
                        "type": "newepoch"
                    },
                    "key": "04000000000000000000000000000000000000000a550c18",
                    "sequence_number": result["events"][1]["sequence_number"].as_u64().unwrap(),
                    "transaction_version": version
                }
            ]),
            "{:#?}",
            result["events"]
        );
        Ok(())
    }
}

fn create_common_write_set() -> WriteSet {
    WriteSetMut::new(vec![(
        AccessPath::new(
            AccountAddress::new([
                0xc4, 0xc6, 0x3f, 0x80, 0xc7, 0x4b, 0x11, 0x26, 0x3e, 0x42, 0x1e, 0xbf, 0x84, 0x86,
                0xa4, 0xe3,
            ]),
            vec![0x01, 0x21, 0x7d, 0xa6, 0xc6, 0xb3, 0xe1, 0x9f, 0x18],
        ),
        WriteOp::Value(vec![0xca, 0xfe, 0xd0, 0x0d]),
    )])
    .freeze()
    .unwrap()
}

struct UpgradeDiemVersion;

impl Test for UpgradeDiemVersion {
    fn name(&self) -> &'static str {
        "jsonrpc::upgrade-diem-version"
    }
}

impl AdminTest for UpgradeDiemVersion {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let env = JsonRpcTestHelper::new(ctx.chain_info().json_rpc().to_owned());
        let factory = ctx.chain_info().transaction_factory();
        let txn = ctx
            .chain_info()
            .root_account()
            .sign_with_transaction_builder(
                factory.update_diem_version(0, DIEM_MAX_KNOWN_VERSION.major + 1),
            );
        env.submit_and_wait(&txn);
        Ok(())
    }
}

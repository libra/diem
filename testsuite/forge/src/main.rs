// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use diem_sdk::{
    client::{BlockingClient, MethodRequest},
    move_types::account_address::AccountAddress,
    transaction_builder::Currency,
};
use forge::{forge_main, ForgeConfig, Options, Result, *};
use itertools::Itertools;
use rand_core::SeedableRng;
use std::time::Duration;
use structopt::StructOpt;
use tokio::runtime::Runtime;

#[derive(StructOpt, Debug)]
struct Args {
    #[structopt(
        long,
        help = "If set, tries to connect to a local swarm instead of testnet"
    )]
    local_swarm: bool,

    // emit_tx options
    #[structopt(long, default_value = "15")]
    accounts_per_client: usize,
    #[structopt(long)]
    workers_per_ac: Option<usize>,
    #[structopt(
        long,
        help = "Time to run --emit-tx for in seconds",
        default_value = "60"
    )]
    duration: u64,

    #[structopt(flatten)]
    options: Options,
}

fn main() -> Result<()> {
    let args = Args::from_args();

    if args.local_swarm {
        forge_main(
            local_test_suite(),
            LocalFactory::from_workspace()?,
            &args.options,
        )
    } else {
        forge_main(k8s_test_suite(), K8sFactory::new().unwrap(), &args.options)
    }
}

fn local_test_suite() -> ForgeConfig<'static> {
    ForgeConfig {
        public_usage_tests: &[&FundAccount, &TransferCoins],
        admin_tests: &[&GetMetadata],
        network_tests: &[&RestartValidator, &EmitTransaction],
    }
}

fn k8s_test_suite() -> ForgeConfig<'static> {
    ForgeConfig {
        public_usage_tests: &[&FundAccount, &TransferCoins],
        admin_tests: &[&GetMetadata],
        network_tests: &[&EmitTransaction],
    }
}

//TODO Make public test later
#[derive(Debug)]
struct GetMetadata;

impl Test for GetMetadata {
    fn name(&self) -> &'static str {
        "get_metadata"
    }
}

impl AdminTest for GetMetadata {
    fn run<'t>(&self, ctx: &mut AdminContext<'t>) -> Result<()> {
        let client = ctx.client();

        let metadata = client.get_metadata()?.into_inner();

        // get_metadata documentation states that the following fields will be present when no version
        // argument is provided
        metadata.script_hash_allow_list.unwrap();
        metadata.diem_version.unwrap();
        metadata.module_publishing_allowed.unwrap();
        metadata.dual_attestation_limit.unwrap();

        Ok(())
    }
}

pub fn check_account_balance(
    client: &BlockingClient,
    currency: Currency,
    account_address: AccountAddress,
    expected: u64,
) -> Result<()> {
    let account_view = client.get_account(account_address)?.into_inner().unwrap();
    let balance = account_view
        .balances
        .iter()
        .find(|b| b.currency == currency)
        .unwrap();
    assert_eq!(balance.amount, expected);

    Ok(())
}

#[derive(Debug)]
struct FundAccount;

impl Test for FundAccount {
    fn name(&self) -> &'static str {
        "fund_account"
    }
}

impl PublicUsageTest for FundAccount {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let client = ctx.client();

        let account = ctx.random_account();
        let amount = 1000;
        let currency = Currency::XUS;
        ctx.create_parent_vasp_account(account.authentication_key())?;
        ctx.fund(account.address(), amount)?;
        check_account_balance(&client, currency, account.address(), amount)?;

        Ok(())
    }
}

#[derive(Debug)]
struct TransferCoins;

impl Test for TransferCoins {
    fn name(&self) -> &'static str {
        "transfer_coins"
    }
}

impl PublicUsageTest for TransferCoins {
    fn run<'t>(&self, ctx: &mut PublicUsageContext<'t>) -> Result<()> {
        let mut account = ctx.random_account();
        let amount = 1000;
        let currency = Currency::XUS;
        let client = ctx.client();
        ctx.create_parent_vasp_account(account.authentication_key())?;
        ctx.fund(account.address(), amount)?;

        let mut payer = ctx.random_account();
        let payee = ctx.random_account();
        let create_payer = account.sign_with_transaction_builder(
            ctx.transaction_factory().create_child_vasp_account(
                currency,
                payer.authentication_key(),
                false,
                100,
            ),
        );
        let create_payee = account.sign_with_transaction_builder(
            ctx.transaction_factory().create_child_vasp_account(
                currency,
                payee.authentication_key(),
                false,
                0,
            ),
        );
        let batch = vec![
            MethodRequest::submit(&create_payer)?,
            MethodRequest::submit(&create_payee)?,
        ];
        client.batch(batch)?;
        client.wait_for_signed_transaction(&create_payer, None, None)?;
        client.wait_for_signed_transaction(&create_payee, None, None)?;
        check_account_balance(&client, currency, payer.address(), 100)?;

        ctx.transfer_coins(currency, &mut payer, payee.address(), 10)?;
        check_account_balance(&client, currency, payer.address(), 90)?;
        check_account_balance(&client, currency, payee.address(), 10)?;
        let account_view = client.get_account(payee.address())?.into_inner().unwrap();
        let balance = account_view
            .balances
            .iter()
            .find(|b| b.currency == currency)
            .unwrap();
        assert_eq!(balance.amount, 10);

        Ok(())
    }
}

#[derive(Debug)]
struct RestartValidator;

impl Test for RestartValidator {
    fn name(&self) -> &'static str {
        "restart_validator"
    }
}

impl NetworkTest for RestartValidator {
    fn run<'t>(&self, ctx: &mut NetworkContext<'t>) -> Result<()> {
        let node = ctx.swarm().validators_mut().next().unwrap();
        node.health_check().expect("node health check failed");
        node.stop()?;
        println!("Restarting node {}", node.peer_id());
        node.start()?;
        // wait node to recovery
        std::thread::sleep(Duration::from_millis(1000));
        node.health_check().expect("node health check failed");

        Ok(())
    }
}

#[derive(Debug)]
struct EmitTransaction;

impl Test for EmitTransaction {
    fn name(&self) -> &'static str {
        "emit_transaction"
    }
}

impl NetworkTest for EmitTransaction {
    fn run<'t>(&self, ctx: &mut NetworkContext<'t>) -> Result<()> {
        let duration = Duration::from_secs(10);
        let rng = SeedableRng::from_rng(ctx.core().rng()).unwrap();
        let validator_clients = ctx
            .swarm()
            .validators()
            .into_iter()
            .map(|n| n.json_rpc_client())
            .collect_vec();
        let mut emitter = TxnEmitter::new(ctx.swarm().chain_info(), rng);
        let rt = Runtime::new().unwrap();
        let stats = rt
            .block_on(emitter.emit_txn_for(duration, EmitJobRequest::default(validator_clients)))
            .unwrap();
        ctx.report
            .report_txn_stats(self.name().to_string(), stats, duration);
        ctx.report.print_report();

        Ok(())
    }
}

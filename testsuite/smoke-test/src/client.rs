// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::test_utils::{
    compare_balances,
    diem_swarm_utils::{get_op_tool, load_diem_root_storage},
    setup_swarm_and_client_proxy,
};
use cli::client_proxy::ClientProxy;

use diem_client::stream::{
    request::StreamMethod, response::StreamJsonRpcResponseView, StreamingClientConfig,
};
use diem_json_rpc_types::Id;
use diem_types::{ledger_info::LedgerInfo, waypoint::Waypoint};
use futures::StreamExt;
use std::time::Duration;
use tokio::{runtime::Runtime, time::timeout};

#[test]
fn test_create_mint_transfer_block_metadata() {
    let (env, client) = setup_swarm_and_client_proxy(1, 0);

    // This script does 4 transactions
    check_create_mint_transfer(client);

    // Test if we commit not only user transactions but also block metadata transactions,
    // assert committed version > # of user transactions
    let mut vclient = env.get_validator_client(0, None);
    vclient.test_trusted_connection().expect("success");
    let version = vclient.get_latest_version();
    assert!(
        version > 4,
        "BlockMetadata txn not produced, current version: {}",
        version
    );
}

#[test]
fn test_get_events_via_websocket_stream() {
    let (_env, client) = setup_swarm_and_client_proxy(2, 1);

    let currencies = client
        .client
        .get_currency_info()
        .expect("Could not get currency info");

    let rt = Runtime::new().unwrap();
    let _guard = rt.enter();

    let ms_500 = Duration::from_millis(500);

    let config = Some(StreamingClientConfig {
        channel_size: 1,
        ok_timeout_millis: 1_000,
    });
    let mut s_client = rt
        .block_on(timeout(ms_500, client.streaming_client(config)))
        .unwrap_or_else(|e| panic!("Timeout creating StreamingClient: {}", e))
        .unwrap_or_else(|e| panic!("Error connecting to WS endpoint: {}", e));

    for (i, currency) in currencies.iter().enumerate() {
        println!("Subscribing to events for {}", &currency.code);

        let mut subscription_stream = rt
            .block_on(timeout(
                ms_500,
                s_client.subscribe_events(currency.mint_events_key, 0),
            ))
            .unwrap_or_else(|e| panic!("Timeout subscribing to {}: {}", &currency.code, e))
            .unwrap_or_else(|e| {
                panic!("Error subscribing to currency '{}': {}", &currency.code, e)
            });

        assert_eq!(subscription_stream.id(), &Id::Number(i as u64));

        // If we're here, then the subscription has already sent the 'OK' message
        if &currency.code != "XUS" {
            continue;
        }

        println!("Getting msg 1 for {}", &currency.code);

        let response = rt
            .block_on(timeout(ms_500, subscription_stream.next()))
            .unwrap_or_else(|e| panic!("Timeout getting message 1: {}", e))
            .unwrap_or_else(|| panic!("Currency '{}' response 1 is None", &currency.code))
            .unwrap_or_else(|e| panic!("Currency '{}' response 1 is Err: {}", &currency.code, e));

        println!("Got msg 1 for {}: {:?}", &currency.code, &response);

        let response_view = response
            .parse_result(&StreamMethod::SubscribeToEvents)
            .unwrap_or_else(|e| {
                panic!(
                    "Currency '{}' response 1 view is err: {}",
                    &currency.code, e
                )
            })
            .unwrap_or_else(|| panic!("Currency '{}' response 1 view is None", &currency.code));

        match response_view {
            StreamJsonRpcResponseView::Event(_) => {}
            _ => panic!("Expected 'Event', but got: {:?}", response_view),
        }

    }
}

#[test]
fn test_basic_fault_tolerance() {
    // A configuration with 4 validators should tolerate single node failure.
    let (mut env, client) = setup_swarm_and_client_proxy(4, 1);
    env.validator_swarm.kill_node(0);
    check_create_mint_transfer(client);
}

#[test]
fn test_basic_restartability() {
    let (mut env, mut client) = setup_swarm_and_client_proxy(4, 0);

    client.create_next_account(false).unwrap();
    client.create_next_account(false).unwrap();
    client.mint_coins(&["mb", "0", "100", "XUS"], true).unwrap();
    client.mint_coins(&["mb", "1", "10", "XUS"], true).unwrap();
    client
        .transfer_coins(&["tb", "0", "1", "10", "XUS"], true)
        .unwrap();
    assert!(compare_balances(
        vec![(90.0, "XUS".to_string())],
        client.get_balances(&["b", "0"]).unwrap(),
    ));
    assert!(compare_balances(
        vec![(20.0, "XUS".to_string())],
        client.get_balances(&["b", "1"]).unwrap(),
    ));

    let peer_to_restart = 0;
    env.validator_swarm.kill_node(peer_to_restart);

    assert!(env.validator_swarm.start_node(peer_to_restart).is_ok());
    assert!(compare_balances(
        vec![(90.0, "XUS".to_string())],
        client.get_balances(&["b", "0"]).unwrap(),
    ));
    assert!(compare_balances(
        vec![(20.0, "XUS".to_string())],
        client.get_balances(&["b", "1"]).unwrap(),
    ));

    client
        .transfer_coins(&["tb", "0", "1", "10", "XUS"], true)
        .unwrap();
    assert!(compare_balances(
        vec![(80.0, "XUS".to_string())],
        client.get_balances(&["b", "0"]).unwrap(),
    ));
    assert!(compare_balances(
        vec![(30.0, "XUS".to_string())],
        client.get_balances(&["b", "1"]).unwrap(),
    ));
}

#[test]
fn test_client_waypoints() {
    let (env, mut client) = setup_swarm_and_client_proxy(4, 1);

    client.create_next_account(false).unwrap();
    client
        .mint_coins(&["mintb", "0", "10", "XUS"], true)
        .unwrap();

    // Create the waypoint for the initial epoch
    let genesis_li = client.latest_epoch_change_li().unwrap();
    assert_eq!(genesis_li.ledger_info().epoch(), 0);
    let genesis_waypoint = Waypoint::new_epoch_boundary(genesis_li.ledger_info()).unwrap();

    // Start another client with the genesis waypoint and make sure it successfully connects
    let mut client_with_waypoint = env.get_validator_client(0, Some(genesis_waypoint));
    client_with_waypoint.test_trusted_connection().unwrap();
    assert_eq!(
        client_with_waypoint.latest_epoch_change_li().unwrap(),
        genesis_li
    );

    // Start next epoch
    let peer_id = env.validator_swarm.get_node(0).unwrap().peer_id();
    let op_tool = get_op_tool(&env.validator_swarm, 1);
    let diem_root = load_diem_root_storage(&env.validator_swarm, 0);
    let _ = op_tool
        .remove_validator(peer_id, &diem_root, false)
        .unwrap();

    client
        .mint_coins(&["mintb", "0", "10", "XUS"], true)
        .unwrap();
    assert!(compare_balances(
        vec![(20.0, "XUS".to_string())],
        client.get_balances(&["b", "0"]).unwrap(),
    ));
    let epoch_1_li = client
        .latest_epoch_change_li()
        .expect("Failed to retrieve end of epoch 1 LedgerInfo");

    assert_eq!(epoch_1_li.ledger_info().epoch(), 1);
    let epoch_1_waypoint = Waypoint::new_epoch_boundary(epoch_1_li.ledger_info())
        .expect("Failed to generate waypoint from end of epoch 1");

    // Start a client with the waypoint for end of epoch 1 and make sure it successfully connects
    client_with_waypoint = env.get_validator_client(1, Some(epoch_1_waypoint));
    client_with_waypoint.test_trusted_connection().unwrap();
    assert_eq!(
        client_with_waypoint.latest_epoch_change_li().unwrap(),
        epoch_1_li
    );

    // Verify that a client with the wrong waypoint is not going to be able to connect to the chain.
    let bad_li = LedgerInfo::mock_genesis(None);
    let bad_waypoint = Waypoint::new_epoch_boundary(&bad_li).unwrap();
    let mut client_with_bad_waypoint = env.get_validator_client(1, Some(bad_waypoint));
    assert!(client_with_bad_waypoint.test_trusted_connection().is_err());
}

#[test]
fn test_concurrent_transfers_single_node() {
    let (_env, mut client) = setup_swarm_and_client_proxy(1, 0);

    client.create_next_account(false).unwrap();
    client
        .mint_coins(&["mintb", "0", "100", "XUS"], true)
        .unwrap();

    client.create_next_account(false).unwrap();
    client
        .mint_coins(&["mintb", "1", "10", "XUS"], true)
        .unwrap();

    for _ in 0..20 {
        client
            .transfer_coins(&["t", "0", "1", "1", "XUS"], false)
            .unwrap();
    }
    client
        .transfer_coins(&["tb", "0", "1", "1", "XUS"], true)
        .unwrap();

    assert!(compare_balances(
        vec![(79.0, "XUS".to_string())],
        client.get_balances(&["b", "0"]).unwrap(),
    ));
    assert!(compare_balances(
        vec![(31.0, "XUS".to_string())],
        client.get_balances(&["b", "1"]).unwrap(),
    ));
}

/// This helper function creates 3 new accounts, mints funds, transfers funds
/// between the accounts and verifies that these operations succeed.
fn check_create_mint_transfer(mut client: ClientProxy) {
    // Create account 0, mint 10 coins and check balance
    client.create_next_account(false).unwrap();
    client
        .mint_coins(&["mintb", "0", "10", "XUS"], true)
        .unwrap();
    assert!(compare_balances(
        vec![(10.0, "XUS".to_string())],
        client.get_balances(&["b", "0"]).unwrap(),
    ));

    // Create account 1, mint 1 coin, transfer 3 coins from account 0 to 1, check balances
    client.create_next_account(false).unwrap();
    client
        .mint_coins(&["mintb", "1", "1", "XUS"], true)
        .unwrap();
    client
        .transfer_coins(&["tb", "0", "1", "3", "XUS"], true)
        .unwrap();
    assert!(compare_balances(
        vec![(7.0, "XUS".to_string())],
        client.get_balances(&["b", "0"]).unwrap(),
    ));
    assert!(compare_balances(
        vec![(4.0, "XUS".to_string())],
        client.get_balances(&["b", "1"]).unwrap(),
    ));

    // Create account 2, mint 15 coins and check balance
    client.create_next_account(false).unwrap();
    client
        .mint_coins(&["mintb", "2", "15", "XUS"], true)
        .unwrap();
    assert!(compare_balances(
        vec![(15.0, "XUS".to_string())],
        client.get_balances(&["b", "2"]).unwrap(),
    ));
}

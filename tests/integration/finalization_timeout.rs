//! Regression: the route heartbeat must cover finalization for both protocols.
//!
//! The first two makers complete normally. The taker then waits longer than
//! the maker idle timeout before handing the key to the final maker. Without a
//! heartbeat spanning finalization, that maker drains its live state into
//! recovery and rejects the later private-key handover.

use bitcoin::Amount;
use openswap::{
    maker::{start_server, MakerBehavior},
    protocol::common_messages::ProtocolVersion,
    taker::{SwapParams, TakerBehavior},
};

use super::test_framework::*;

use std::thread;

#[test]
fn taproot_last_maker_survives_finalization_idle_window() {
    let maker_count = 3;
    let taker_behaviors = vec![TakerBehavior::StallBeforeLastHandover];
    let maker_behaviors = vec![
        MakerBehavior::Normal,
        MakerBehavior::Normal,
        MakerBehavior::DropHandoverResponse,
    ];

    let (test_framework, mut takers, makers, block_generation_handle) =
        TestFramework::init::<BitcoindBackend>(maker_count, taker_behaviors, maker_behaviors);

    let bitcoind = &test_framework.bitcoind;
    let taker = takers.first_mut().unwrap();

    fund_taker_default(taker, bitcoind, 4);
    fund_makers_default(&makers, bitcoind);

    let maker_threads = makers
        .iter()
        .map(|maker| {
            let maker = maker.clone();
            thread::spawn(move || start_server(maker).unwrap())
        })
        .collect::<Vec<_>>();

    wait_for_makers_setup(&makers, 120);
    sync_maker_wallets(&makers);
    generate_blocks(bitcoind, 1);

    let params = SwapParams::new(ProtocolVersion::Taproot, Amount::from_sat(500_000), 3)
        .with_tx_count(1)
        .with_required_confirms(1);
    let summary = taker.prepare_swap(params).expect("prepare should succeed");

    taker
        .start_swap(&summary.swap_id)
        .expect("the route heartbeat must keep the last maker live through finalization");

    generate_blocks(bitcoind, 1);
    sync_maker_wallets(&makers);

    for (index, maker) in makers.iter().enumerate() {
        let balances = maker.wallet.read().unwrap().get_balances().unwrap();
        assert_eq!(
            balances.contract,
            Amount::ZERO,
            "maker {index} retained contract balance after finalization"
        );
    }

    let log = std::fs::read_to_string(test_framework.taker_log_path()).unwrap();
    assert!(log.contains("Test behavior: stalling"));
    assert!(log.contains("Test behavior: dropping completed handover response"));
    assert!(
        !log.contains(&format!("Swap {} idle", summary.swap_id)),
        "the last maker timed out while the taker was still finalizing"
    );
    assert!(
        !log.contains(
            "UnexpectedMessage { expected: \"Legacy protocol message\", got: \"Taproot protocol message\" }"
        ),
        "a completed/missing Taproot swap fell back to the connection's Legacy default"
    );
    assert_eq!(
        log.matches("Processing Taproot private key handover")
            .count(),
        3,
        "each maker should process exactly one private-key handover"
    );

    shutdown_makers(&makers, maker_threads);
    test_framework.stop();
    block_generation_handle.join().unwrap();
}

#[test]
fn legacy_last_maker_survives_finalization_idle_window() {
    let maker_count = 3;
    let taker_behaviors = vec![TakerBehavior::StallBeforeLastHandover];
    let maker_behaviors = vec![
        MakerBehavior::Normal,
        MakerBehavior::Normal,
        MakerBehavior::DropHandoverResponse,
    ];

    let (test_framework, mut takers, makers, block_generation_handle) =
        TestFramework::init::<BitcoindBackend>(maker_count, taker_behaviors, maker_behaviors);

    let bitcoind = &test_framework.bitcoind;
    let taker = takers.first_mut().unwrap();

    fund_taker_default(taker, bitcoind, 4);
    fund_makers_default(&makers, bitcoind);

    let maker_threads = makers
        .iter()
        .map(|maker| {
            let maker = maker.clone();
            thread::spawn(move || start_server(maker).unwrap())
        })
        .collect::<Vec<_>>();

    wait_for_makers_setup(&makers, 120);
    sync_maker_wallets(&makers);
    generate_blocks(bitcoind, 1);

    let params = SwapParams::new(ProtocolVersion::Legacy, Amount::from_sat(500_000), 3)
        .with_tx_count(1)
        .with_required_confirms(1);
    let summary = taker.prepare_swap(params).expect("prepare should succeed");

    taker
        .start_swap(&summary.swap_id)
        .expect("the route heartbeat must keep the last Legacy maker live through finalization");

    generate_blocks(bitcoind, 1);
    sync_maker_wallets(&makers);

    for (index, maker) in makers.iter().enumerate() {
        let balances = maker.wallet.read().unwrap().get_balances().unwrap();
        assert_eq!(
            balances.contract,
            Amount::ZERO,
            "maker {index} retained contract balance after finalization"
        );
    }

    let log = std::fs::read_to_string(test_framework.taker_log_path()).unwrap();
    assert!(log.contains("Test behavior: stalling"));
    assert!(log.contains("Test behavior: dropping completed handover response"));
    assert!(
        !log.contains(&format!("Swap {} idle", summary.swap_id)),
        "the last Legacy maker timed out while the taker was still finalizing"
    );
    assert_eq!(
        log.matches("Processing Legacy private key handover")
            .count(),
        3,
        "each Legacy maker should process exactly one private-key handover"
    );

    shutdown_makers(&makers, maker_threads);
    test_framework.stop();
    block_generation_handle.join().unwrap();
}

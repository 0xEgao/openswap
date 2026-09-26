//! Successful swaps are terminal across maker restarts.
//!
//! A completed maker has already swept every incoming contract and handed its
//! outgoing keys to the taker. Persisting those outgoing swapcoins makes the
//! next startup misclassify the successful swap as abandoned and launch
//! recovery. Exercise the complete protocol, restart both makers from the same
//! wallets, and prove that neither protocol leaves recovery material behind.

use bitcoin::Amount;
use openswap::{
    maker::{start_server, MakerBehavior, MakerServer},
    protocol::common_messages::ProtocolVersion,
    taker::{SwapParams, TakerBehavior},
};

use super::test_framework::*;

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn run_successful_swap_restart(protocol: ProtocolVersion) {
    let (test_framework, mut takers, makers, block_generation_handle) =
        TestFramework::init::<BitcoindBackend>(
            2,
            vec![TakerBehavior::Normal],
            vec![MakerBehavior::Normal, MakerBehavior::Normal],
        );

    let bitcoind = &test_framework.bitcoind;
    fund_taker_default(&takers[0], bitcoind, 3);
    fund_makers_default(&makers, bitcoind);

    let maker_threads = makers
        .iter()
        .map(|maker| {
            let maker = Arc::clone(maker);
            thread::spawn(move || start_server(maker).unwrap())
        })
        .collect();
    wait_for_makers_setup(&makers, 120);
    sync_maker_wallets(&makers);

    generate_blocks(bitcoind, 1);
    let summary = takers[0]
        .prepare_swap(
            SwapParams::new(protocol, Amount::from_sat(500_000), 2)
                .with_tx_count(2)
                .with_required_confirms(1),
        )
        .expect("prepare successful swap");
    takers[0]
        .start_swap(&summary.swap_id)
        .expect("swap must complete successfully");

    // The taker can return just before the maker handler finishes its durable
    // cleanup. Wait for that cleanup, not for an arbitrary sleep.
    let deadline = Instant::now() + Duration::from_secs(120);
    while makers.iter().any(|maker| {
        let wallet = maker.wallet.read().unwrap();
        wallet.get_incoming_swapcoins_count() != 0 || wallet.get_outgoing_swapcoins_count() != 0
    }) {
        assert!(
            Instant::now() < deadline,
            "successful {:?} swap left maker swapcoins on disk",
            protocol
        );
        thread::sleep(Duration::from_millis(250));
    }

    let configs = makers
        .iter()
        .map(|maker| {
            let mut config = maker.config.clone();
            // Initialization consumes the configured passphrase.
            config.password = Some("integration-test".to_string());
            config
        })
        .collect::<Vec<_>>();
    drop(takers);
    shutdown_makers(&makers, maker_threads);
    drop(makers);

    let restarted = configs
        .into_iter()
        .map(|config| Arc::new(MakerServer::init(config).unwrap()))
        .collect::<Vec<_>>();
    let restarted_threads = restarted
        .iter()
        .map(|maker| {
            let maker = Arc::clone(maker);
            thread::spawn(move || start_server(maker).unwrap())
        })
        .collect();
    wait_for_makers_setup(&restarted, 120);

    for maker in &restarted {
        let wallet = maker.wallet.read().unwrap();
        assert_eq!(
            wallet.get_incoming_swapcoins_count(),
            0,
            "restarted maker retained incoming swapcoins for a successful {protocol:?} swap"
        );
        assert_eq!(
            wallet.get_outgoing_swapcoins_count(),
            0,
            "restarted maker retained outgoing swapcoins for a successful {protocol:?} swap"
        );
    }

    shutdown_makers(&restarted, restarted_threads);
    test_framework.stop();
    block_generation_handle.join().unwrap();
}

#[test]
fn successful_legacy_swap_stays_complete_after_maker_restart() {
    run_successful_swap_restart(ProtocolVersion::Legacy);
}

#[test]
fn successful_taproot_swap_stays_complete_after_maker_restart() {
    run_successful_swap_restart(ProtocolVersion::Taproot);
}

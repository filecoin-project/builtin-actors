use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::Method as AccountMethod;
use fil_actor_market::{
    ClientDealProposal, DealProposal, Label, Method as MarketMethod, PublishStorageDealsParams,
};
use fil_actor_miner::max_prove_commit_duration;
use fil_actor_verifreg::{AddVerifiedClientParams, Method as VerifregMethod};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    test_utils::*, STORAGE_MARKET_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ID, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};
use vm_api::trace::ExpectInvocation;
use vm_api::util::{apply_ok, serialize_ok};
use vm_api::VM;

use crate::deals::{DealBatcher, DealOptions};
use crate::expects::Expect;

use crate::util::{
    assert_invariants, bf_all, create_accounts, create_accounts_seeded, create_miner,
    verifreg_add_verifier,
};
use crate::TEST_FAUCET_ADDR;
use export_macro::vm_test;

struct Addrs {
    worker: Address,
    client1: Address,
    client2: Address,
    not_miner: Address,
    cheap_client: Address,
    maddr: Address,
    verified_client: Address,
}

const DEAL_LIFETIME: ChainEpoch = 181 * EPOCHS_IN_DAY;

// create miner and client and add collateral
fn setup(v: &dyn VM) -> (Addrs, ChainEpoch) {
    let addrs = create_accounts(v, 7, &TokenAmount::from_whole(10_000));
    let (worker, client1, client2, not_miner, cheap_client, verifier, verified_client) =
        (addrs[0], addrs[1], addrs[2], addrs[3], addrs[4], addrs[5], addrs[6]);
    let owner = worker;

    // setup provider
    let miner_balance = TokenAmount::from_whole(100);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;

    let maddr = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &miner_balance,
    )
    .0;

    // setup verified client
    verifreg_add_verifier(v, &verifier, StoragePower::from((32_u64 << 40) as u128));
    let add_client_params = AddVerifiedClientParams {
        address: verified_client,
        allowance: StoragePower::from(1_u64 << 32),
    };
    apply_ok(
        v,
        &verifier,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        Some(add_client_params),
    );

    let client_collateral = TokenAmount::from_whole(100);
    apply_ok(
        v,
        &client1,
        &STORAGE_MARKET_ACTOR_ADDR,
        &client_collateral,
        MarketMethod::AddBalance as u64,
        Some(client1),
    );
    apply_ok(
        v,
        &client2,
        &STORAGE_MARKET_ACTOR_ADDR,
        &client_collateral,
        MarketMethod::AddBalance as u64,
        Some(client2),
    );
    apply_ok(
        v,
        &verified_client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &client_collateral,
        MarketMethod::AddBalance as u64,
        Some(verified_client),
    );

    let miner_collateral = TokenAmount::from_whole(100);
    apply_ok(
        v,
        &worker,
        &STORAGE_MARKET_ACTOR_ADDR,
        &miner_collateral,
        MarketMethod::AddBalance as u64,
        Some(maddr),
    );

    let deal_start = v.epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    (
        Addrs { worker, client1, client2, not_miner, cheap_client, maddr, verified_client },
        deal_start,
    )
}

#[vm_test]
pub fn psd_mismatched_provider_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts);

    // good deal
    batcher.stage(a.client1, a.maddr);
    // bad deal, provider doesn't match worker
    batcher.stage(a.client1, a.not_miner);
    //good deal
    batcher.stage(a.client1, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 2], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_bad_piece_size_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    // bad deal piece size too small
    batcher.stage_with_opts(
        a.client1,
        a.maddr,
        DealOptions { piece_size: PaddedPieceSize(0), ..opts },
    );
    // good deal
    batcher.stage(a.client1, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![1], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_start_time_in_past_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    let bad_deal_start = v.epoch() - 1;
    batcher.stage_with_opts(a.client1, a.maddr, DealOptions { deal_start: bad_deal_start, ..opts });
    batcher.stage(a.client1, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![1], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_client_address_cannot_be_resolved_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts);
    let bad_client = Address::new_id(5_000_000);
    batcher.stage(a.client1, a.maddr);
    batcher.stage(bad_client, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_no_client_lockup_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts);
    batcher.stage(a.cheap_client, a.maddr);
    batcher.stage(a.client1, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![1], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_not_enough_client_lockup_for_batch_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    // Add one lifetime cost to cheap_client's market balance but attempt to make 3 deals
    let one_lifetime_cost = opts.client_collateral + DEAL_LIFETIME * opts.price_per_epoch;
    apply_ok(
        v,
        &a.cheap_client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &one_lifetime_cost,
        MarketMethod::AddBalance as u64,
        Some(a.cheap_client),
    );

    // good
    batcher.stage(a.cheap_client, a.maddr);
    // bad -- insufficient funds
    batcher.stage(a.cheap_client, a.maddr);
    batcher.stage(a.cheap_client, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_not_enough_provider_lockup_for_batch_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    // note different seed, different address
    let cheap_worker =
        create_accounts_seeded(v, 1, &TokenAmount::from_whole(10_000), 444, &TEST_FAUCET_ADDR)[0];
    let cheap_maddr = create_miner(
        v,
        &cheap_worker,
        &cheap_worker,
        fvm_shared::sector::RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
        &TokenAmount::from_whole(100),
    )
    .0;
    // add one deal of collateral to provider's market account
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    apply_ok(
        v,
        &cheap_worker,
        &STORAGE_MARKET_ACTOR_ADDR,
        &opts.provider_collateral,
        MarketMethod::AddBalance as u64,
        Some(cheap_maddr),
    );
    // good deal
    batcher.stage(a.client1, cheap_maddr);
    // bad deal insufficient funds on provider
    batcher.stage(a.client2, cheap_maddr);
    let deal_ret = batcher.publish_ok(cheap_worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_duplicate_deal_in_batch_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts);

    // good deals
    batcher.stage_with_label(a.client1, a.maddr, "deal0".to_string());
    batcher.stage_with_label(a.client1, a.maddr, "deal1".to_string());

    // bad duplicates
    batcher.stage_with_label(a.client1, a.maddr, "deal0".to_string());
    batcher.stage_with_label(a.client1, a.maddr, "deal0".to_string());

    // good
    batcher.stage_with_label(a.client1, a.maddr, "deal2".to_string());

    // bad
    batcher.stage_with_label(a.client1, a.maddr, "deal1".to_string());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1, 4], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_duplicate_deal_in_state_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    batcher.stage(a.client2, a.maddr);
    let deal_ret1 = batcher.publish_ok(a.worker);
    let good_inputs1 = bf_all(deal_ret1.valid_deals);
    assert_eq!(vec![0], good_inputs1);

    let mut batcher = DealBatcher::new(v, opts);
    // duplicate in state from previous dealer
    batcher.stage(a.client2, a.maddr);
    // duplicate in batch
    batcher.stage_with_label(a.client2, a.maddr, "deal1".to_string());
    batcher.stage_with_label(a.client2, a.maddr, "deal1".to_string());

    let deal_ret2 = batcher.publish_ok(a.worker);
    let good_inputs2 = bf_all(deal_ret2.valid_deals);
    assert_eq!(vec![1], good_inputs2);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_verified_deal_fails_getting_datacap_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    batcher.stage(a.verified_client, a.maddr);
    // good verified deal that uses up all data cap
    batcher.stage_with_opts(
        a.verified_client,
        a.maddr,
        DealOptions { piece_size: PaddedPieceSize(1 << 32), verified: true, ..opts.clone() },
    );
    // bad verified deal, no data cap left
    batcher.stage_with_opts(
        a.verified_client,
        a.maddr,
        DealOptions { piece_size: PaddedPieceSize(1 << 32), verified: true, ..opts },
    );

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_random_assortment_of_failures_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());
    // Add one lifetime cost to cheap_client's market balance but attempt to make 3 deals
    let one_lifetime_cost = &opts.client_collateral + DEAL_LIFETIME * &opts.price_per_epoch;
    apply_ok(
        v,
        &a.cheap_client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &one_lifetime_cost,
        MarketMethod::AddBalance as u64,
        Some(a.cheap_client),
    );
    let broke_client =
        create_accounts_seeded(v, 1, &TokenAmount::zero(), 555, &TEST_FAUCET_ADDR)[0];

    batcher.stage_with_opts_label(
        a.verified_client,
        a.maddr,
        "foo".to_string(),
        DealOptions { piece_size: PaddedPieceSize(1 << 32), verified: true, ..opts.clone() },
    );
    // duplicate
    batcher.stage_with_opts_label(
        a.verified_client,
        a.maddr,
        "foo".to_string(),
        DealOptions { piece_size: PaddedPieceSize(1 << 32), verified: true, ..opts.clone() },
    );
    batcher.stage(a.cheap_client, a.maddr);
    // no client funds
    batcher.stage(broke_client, a.maddr);
    // provider addr does not match
    batcher.stage(a.client1, a.client2);
    // insufficient data cap
    batcher.stage_with_opts(
        a.verified_client,
        a.maddr,
        DealOptions { verified: true, ..opts.clone() },
    );
    // cheap client out of funds
    batcher.stage(a.cheap_client, a.maddr);
    // provider collateral too low
    batcher.stage_with_opts(
        a.client2,
        a.maddr,
        DealOptions { provider_collateral: TokenAmount::zero(), ..opts },
    );
    batcher.stage(a.client1, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 2, 8], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_all_deals_are_bad_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());
    let bad_client = Address::new_id(1000);

    batcher.stage_with_opts(
        a.client1,
        a.maddr,
        DealOptions { provider_collateral: TokenAmount::zero(), ..opts.clone() },
    );
    batcher.stage(a.client1, a.client2);
    batcher.stage_with_opts(a.client1, a.maddr, DealOptions { verified: true, ..opts.clone() });
    batcher.stage(bad_client, a.maddr);
    batcher.stage_with_opts(
        a.client1,
        a.maddr,
        DealOptions { piece_size: PaddedPieceSize(0), ..opts },
    );

    batcher.publish_fail(a.worker);
    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_bad_sig_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let worker_id = a.worker.id().unwrap();
    let DealOptions { price_per_epoch, provider_collateral, client_collateral, .. } =
        DealOptions::default();
    let deal_label = "deal0".to_string();
    let proposal = DealProposal {
        piece_cid: make_piece_cid(deal_label.as_bytes()),
        piece_size: PaddedPieceSize(1 << 30),
        verified_deal: false,
        client: a.client1,
        provider: a.maddr,
        label: Label::String(deal_label),
        start_epoch: deal_start,
        end_epoch: deal_start + DEAL_LIFETIME,
        storage_price_per_epoch: price_per_epoch,
        provider_collateral,
        client_collateral,
    };

    let invalid_sig_bytes = "very_invalid_sig".as_bytes().to_vec();

    let publish_params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal {
            proposal: proposal.clone(),
            client_signature: Signature {
                sig_type: SignatureType::BLS,
                bytes: invalid_sig_bytes.clone(),
            },
        }],
    };
    let ret = v
        .execute_message(
            &a.worker,
            &STORAGE_MARKET_ACTOR_ADDR,
            &TokenAmount::zero(),
            MarketMethod::PublishStorageDeals as u64,
            Some(serialize_ok(&publish_params)),
        )
        .unwrap();
    assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, ret.code);

    ExpectInvocation {
        from: worker_id,
        to: STORAGE_MARKET_ACTOR_ADDR,
        method: MarketMethod::PublishStorageDeals as u64,
        subinvocs: Some(vec![
            Expect::miner_is_controlling_address(STORAGE_MARKET_ACTOR_ID, a.maddr, a.worker),
            Expect::reward_this_epoch(STORAGE_MARKET_ACTOR_ID),
            Expect::power_current_total(STORAGE_MARKET_ACTOR_ID),
            ExpectInvocation {
                from: STORAGE_MARKET_ACTOR_ID,
                to: a.client1,
                method: AccountMethod::AuthenticateMessageExported as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&AuthenticateMessageParams {
                        signature: invalid_sig_bytes,
                        message: serialize(&proposal, "deal proposal").unwrap().to_vec(),
                    })
                    .unwrap(),
                ),
                return_value: None,
                exit_code: ExitCode::USR_ILLEGAL_ARGUMENT,
                ..Default::default()
            },
        ]),
        return_value: None,
        exit_code: ExitCode::USR_ILLEGAL_ARGUMENT,
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn all_deals_are_good_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts);

    // good deals
    batcher.stage(a.client1, a.maddr);
    batcher.stage(a.client1, a.maddr);
    batcher.stage(a.client1, a.maddr);
    batcher.stage(a.client1, a.maddr);
    batcher.stage(a.client1, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1, 2, 3, 4], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_valid_deals_with_ones_longer_than_540_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    // good deals
    batcher.stage_with_opts(
        a.client1,
        a.maddr,
        DealOptions { deal_lifetime: 541 * EPOCHS_IN_DAY, ..opts.clone() },
    );
    batcher.stage_with_opts(
        a.client1,
        a.maddr,
        DealOptions { deal_lifetime: 1278 * EPOCHS_IN_DAY, ..opts },
    );
    batcher.stage(a.client1, a.maddr);

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1, 2], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn psd_deal_duration_too_long_test(v: &dyn VM) {
    let (a, deal_start) = setup(v);
    let opts = DealOptions { deal_start, ..DealOptions::default() };
    let mut batcher = DealBatcher::new(v, opts.clone());

    // good deals
    batcher.stage_with_opts(
        a.client1,
        a.maddr,
        DealOptions { deal_lifetime: 541 * EPOCHS_IN_DAY, ..opts.clone() },
    );
    batcher.stage(a.client1, a.maddr);

    //bad deal - duration > max deal
    batcher.stage_with_opts(
        a.client1,
        a.maddr,
        DealOptions { deal_lifetime: 1279 * EPOCHS_IN_DAY, ..opts },
    );

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1], good_inputs);

    assert_invariants(v, &Policy::default(), None)
}

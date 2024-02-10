use std::ops::Neg;

use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};
use num_traits::cast::FromPrimitive;

use export_macro::vm_test;
use fil_actor_market::{
    DealMetaArray, Method as MarketMethod, State as MarketState, WithdrawBalanceParams,
};
use fil_actor_miner::{
    power_for_sector, Method as MinerMethod, ProveCommitSectorParams, State as MinerState,
    TerminateSectorsParams, TerminationDeclaration,
};
use fil_actor_power::State as PowerState;
use fil_actor_verifreg::{Method as VerifregMethod, VerifierParams};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    STORAGE_MARKET_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ID, STORAGE_POWER_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_shared::deal::DealID;
use fvm_shared::ActorID;
use vm_api::trace::ExpectInvocation;
use vm_api::util::{apply_ok, get_state, DynBlockstore};
use vm_api::VM;

use crate::expects::Expect;
use crate::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_to_proving_deadline, assert_invariants, create_accounts, create_miner, cron_tick,
    deal_cid_for_testing, make_bitfield, market_publish_deal, miner_balance,
    miner_precommit_one_sector_v2, precommit_meta_data_from_deals, submit_windowed_post,
    verifreg_add_verifier,
};

#[vm_test]
pub fn terminate_sectors_test(v: &dyn VM) {
    let addrs = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let (owner, verifier, unverified_client, verified_client) =
        (addrs[0], addrs[1], addrs[2], addrs[3]);
    let worker = owner;
    let worker_id = worker.id().unwrap();
    let verified_client_id = verified_client.id().unwrap();

    let m_balance = TokenAmount::from_whole(1_000);
    let sector_number = 100;
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;

    let (miner_id_addr, miner_robust_addr) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &m_balance,
    );
    let miner_id = miner_id_addr.id().unwrap();

    // publish verified and unverified deals
    verifreg_add_verifier(v, &verifier, StoragePower::from_i64(32 << 40_i64).unwrap());

    let add_client_params = VerifierParams {
        address: verified_client,
        allowance: StoragePower::from_i64(32 << 40_i64).unwrap(),
    };
    apply_ok(
        v,
        &verifier,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        Some(add_client_params),
    );

    // add market collateral
    let collateral = TokenAmount::from_whole(3);
    apply_ok(
        v,
        &unverified_client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &collateral,
        MarketMethod::AddBalance as u64,
        Some(unverified_client),
    );
    apply_ok(
        v,
        &verified_client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &collateral,
        MarketMethod::AddBalance as u64,
        Some(verified_client),
    );

    let miner_collateral = TokenAmount::from_whole(64);
    apply_ok(
        v,
        &worker,
        &STORAGE_MARKET_ACTOR_ADDR,
        &miner_collateral,
        MarketMethod::AddBalance as u64,
        Some(miner_id_addr),
    );

    // create 3 deals, some verified and some not
    let mut deal_ids = vec![];
    let deal_start = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;
    let deals = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id_addr,
        "deal1".to_string(),
        PaddedPieceSize(1 << 30),
        true,
        deal_start,
        181 * EPOCHS_IN_DAY,
    );
    for id in deals.ids.iter() {
        deal_ids.push(*id);
    }
    let deals = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id_addr,
        "deal2".to_string(),
        PaddedPieceSize(1 << 32),
        true,
        deal_start,
        200 * EPOCHS_IN_DAY,
    );
    for id in deals.ids.iter() {
        deal_ids.push(*id);
    }
    let deals = market_publish_deal(
        v,
        &worker,
        &unverified_client,
        &miner_id_addr,
        "deal3".to_string(),
        PaddedPieceSize(1 << 34),
        false,
        deal_start,
        210 * EPOCHS_IN_DAY,
    );
    for id in deals.ids.iter() {
        deal_ids.push(*id);
    }

    cron_tick(v);

    let st: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let deal_states = DealMetaArray::load(&st.states, &store).unwrap();
    for id in deal_ids.iter() {
        // deals are pending and don't yet have deal states
        let state = deal_states.get(*id).unwrap();
        assert_eq!(None, state);
    }

    miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_robust_addr,
        seal_proof,
        sector_number,
        precommit_meta_data_from_deals(v, &deal_ids, seal_proof),
        true,
        v.epoch() + 220 * EPOCHS_IN_DAY,
    );
    let prove_time = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &miner_id_addr, prove_time);

    // prove commit, cron, advance to post time
    let prove_params = ProveCommitSectorParams { sector_number, proof: vec![].into() };
    apply_ok(
        v,
        &worker,
        &miner_robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSector as u64,
        Some(prove_params),
    );

    cron_tick(v);

    let (dline_info, p_idx) = advance_to_proving_deadline(v, &miner_id_addr, sector_number);
    let d_idx = dline_info.index;
    let st: MinerState = get_state(v, &miner_id_addr).unwrap();
    let sector =
        st.get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number).unwrap().unwrap();
    let sector_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    submit_windowed_post(v, &worker, &miner_id_addr, dline_info, p_idx, Some(sector_power.clone()));
    v.set_epoch(dline_info.last());

    cron_tick(v);

    // advance cron delay epochs so deals are active
    let start = dline_info.close;
    v.set_epoch(start);
    // get out of proving deadline so we don't post twice
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id_addr,
        &worker,
        sector_number,
        start + Policy::default().deal_updates_interval,
    );

    // deals are no longer pending, though they've never been processed
    let st: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    for id in deal_ids.iter() {
        let proposal = st.get_proposal(&store, *id).unwrap();
        let dcid = deal_cid_for_testing(&proposal);
        assert!(!st.has_pending_deal(&store, &dcid).unwrap());
    }
    let epoch = v.epoch();

    let expect_event = Expect::build_miner_event("sector-terminated", miner_id, sector_number);
    let deal_clients: Vec<(DealID, ActorID)> = vec![
        (deal_ids[0], verified_client_id),
        (deal_ids[1], verified_client_id),
        (deal_ids[2], unverified_client.id().unwrap()),
    ];

    // Terminate Sector
    apply_ok(
        v,
        &worker,
        &miner_robust_addr,
        &TokenAmount::zero(),
        MinerMethod::TerminateSectors as u64,
        Some(TerminateSectorsParams {
            terminations: vec![TerminationDeclaration {
                deadline: d_idx,
                partition: p_idx,
                sectors: make_bitfield(&[sector_number]),
            }],
        }),
    );
    ExpectInvocation {
        from: worker_id,
        to: miner_id_addr,
        method: MinerMethod::TerminateSectors as u64,
        subinvocs: Some(vec![
            Expect::reward_this_epoch(miner_id),
            Expect::power_current_total(miner_id),
            Expect::burn(miner_id, None),
            Expect::power_update_pledge(miner_id, None),
            Expect::market_sectors_terminate(
                miner_id,
                epoch,
                [sector_number].to_vec(),
                deal_clients,
            ),
            Expect::power_update_claim(miner_id, sector_power.neg()),
        ]),
        events: vec![expect_event],
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    let miner_balances = miner_balance(v, &miner_id_addr);
    assert!(miner_balances.initial_pledge.is_zero());
    assert!(miner_balances.pre_commit_deposit.is_zero());

    let pow_st: PowerState = get_state(v, &STORAGE_POWER_ACTOR_ADDR).unwrap();
    assert_eq!(0, pow_st.miner_above_min_power_count);
    assert!(pow_st.total_raw_byte_power.is_zero());
    assert!(pow_st.total_quality_adj_power.is_zero());
    assert!(pow_st.total_bytes_committed.is_zero());
    assert!(pow_st.total_qa_bytes_committed.is_zero());
    assert!(pow_st.total_pledge_collateral.is_zero());

    // termination synchronously deletes deal state
    let st: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let deal_states = DealMetaArray::load(&st.states, &store).unwrap();
    for &id in deal_ids.iter() {
        let state = deal_states.get(id).unwrap();
        assert!(state.is_none());
        assert!(st.find_proposal(&store, id).unwrap().is_none());
    }

    // because of rounding error it's annoying to compute exact withdrawable balance which is 2.9999.. FIL
    // withdrawing 2 FIL proves out that the claim to 1 FIL per deal (2 deals for this client) is removed at termination
    let withdrawal = TokenAmount::from_whole(2);
    apply_ok(
        v,
        &verified_client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &TokenAmount::zero(),
        MarketMethod::WithdrawBalance as u64,
        Some(WithdrawBalanceParams {
            provider_or_client: verified_client,
            amount: withdrawal.clone(),
        }),
    );
    ExpectInvocation {
        from: verified_client_id,
        to: STORAGE_MARKET_ACTOR_ADDR,
        method: MarketMethod::WithdrawBalance as u64,
        subinvocs: Some(vec![Expect::send(
            STORAGE_MARKET_ACTOR_ID,
            verified_client,
            Some(withdrawal),
        )]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    apply_ok(
        v,
        &worker,
        &STORAGE_MARKET_ACTOR_ADDR,
        &TokenAmount::zero(),
        MarketMethod::WithdrawBalance as u64,
        Some(WithdrawBalanceParams { provider_or_client: miner_id_addr, amount: miner_collateral }),
    );

    let value_withdrawn = v.take_invocations().last().unwrap().subinvocations[1].value.clone();
    // miner add 64 balance. Each of 3 deals required 2 FIL collateral, so provider collateral should have been
    // slashed by 6 FIL. Miner's remaining market balance should be 64 - 6 + payment, where payment is for storage
    // before the slash and should be << 1 FIL. Actual amount withdrawn should be between 58 and 59 FIL.
    assert!(TokenAmount::from_whole(58) < value_withdrawn);
    assert!(TokenAmount::from_whole(59) > value_withdrawn);

    assert_invariants(v, &Policy::default(), None);
}

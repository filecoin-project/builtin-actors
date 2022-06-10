use fil_actor_cron::Method as MethodsCron;
use fil_actor_market::{
    DealMetaArray, Method as MethodsMarket, State as MarketState, WithdrawBalanceParams,
};
use fil_actor_miner::{
    power_for_sector, Method as MethodsMiner, PreCommitSectorParams, ProveCommitSectorParams,
    State as MinerState, TerminateSectorsParams, TerminationDeclaration,
};
use fil_actor_power::{Method as MethodsPower, State as PowerState};
use fil_actor_reward::Method as MethodsReward;
use fil_actor_verifreg::{Method as MethodsVerifreg, VerifierParams};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    test_utils::*, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, REWARD_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};
use fvm_shared::METHOD_SEND;
use num_traits::cast::FromPrimitive;
use test_vm::util::{
    add_verifier, advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_to_proving_deadline, apply_ok, create_accounts, create_miner, make_bitfield,
    publish_deal, submit_windowed_post,
};
use test_vm::{ExpectInvocation, VM};

#[test]
fn terminate_sectors() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 4, TokenAmount::from(10_000e18 as i128));
    let (owner, verifier, unverified_client, verified_client) =
        (addrs[0], addrs[1], addrs[2], addrs[3]);
    let worker = owner;

    let miner_balance = TokenAmount::from(1_000e18 as i128);
    let sector_number = 100;
    let sealed_cid = make_sealed_cid(b"s100");
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;

    let (id_addr, robust_addr) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        miner_balance,
    );

    // publish verified and unverified deals
    add_verifier(&v, verifier, StoragePower::from_i64(32 << 40_i64).unwrap());

    let add_client_params = VerifierParams {
        address: verified_client,
        allowance: StoragePower::from_i64(32 << 40_i64).unwrap(),
    };
    apply_ok(
        &v,
        verifier,
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        MethodsVerifreg::AddVerifiedClient as u64,
        add_client_params,
    );

    // add market collateral
    let collateral = TokenAmount::from(3e18 as u64);
    apply_ok(
        &v,
        unverified_client,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral.clone(),
        MethodsMarket::AddBalance as u64,
        unverified_client,
    );
    apply_ok(
        &v,
        verified_client,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral,
        MethodsMarket::AddBalance as u64,
        verified_client,
    );

    let miner_collateral = TokenAmount::from(64e18 as u128);
    apply_ok(
        &v,
        worker,
        *STORAGE_MARKET_ACTOR_ADDR,
        miner_collateral.clone(),
        MethodsMarket::AddBalance as u64,
        id_addr,
    );

    // create 3 deals, some verified and some not
    let mut deal_ids = vec![];
    let deal_start = v.get_epoch() + Policy::default().pre_commit_challenge_delay + 1;
    let deals = publish_deal(
        &v,
        worker,
        verified_client,
        id_addr,
        "deal1".to_string(),
        PaddedPieceSize(1 << 30),
        true,
        deal_start,
        181 * EPOCHS_IN_DAY,
    );
    for id in deals.ids.iter() {
        deal_ids.push(*id);
    }
    let deals = publish_deal(
        &v,
        worker,
        verified_client,
        id_addr,
        "deal2".to_string(),
        PaddedPieceSize(1 << 32),
        true,
        deal_start,
        200 * EPOCHS_IN_DAY,
    );
    for id in deals.ids.iter() {
        deal_ids.push(*id);
    }
    let deals = publish_deal(
        &v,
        worker,
        unverified_client,
        id_addr,
        "deal3".to_string(),
        PaddedPieceSize(1 << 34),
        false,
        deal_start,
        210 * EPOCHS_IN_DAY,
    );
    for id in deals.ids.iter() {
        deal_ids.push(*id);
    }

    let res = v
        .apply_message(
            *SYSTEM_ACTOR_ADDR,
            *CRON_ACTOR_ADDR,
            TokenAmount::zero(),
            MethodsCron::EpochTick as u64,
            RawBytes::default(),
        )
        .unwrap();
    assert_eq!(ExitCode::OK, res.code);
    let st = v.get_state::<MarketState>(*STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let deal_states = DealMetaArray::load(&st.states, v.store).unwrap();
    for id in deal_ids.iter() {
        // deals are pending and don't yet have deal states
        let state = deal_states.get(*id).unwrap();
        assert_eq!(None, state);
    }
    //    precommit_sectors(&mut v, 1, 1, worker, robust_addr, seal_proof, sector_number, true, None);
    apply_ok(
        &v,
        worker,
        robust_addr,
        TokenAmount::zero(),
        MethodsMiner::PreCommitSector as u64,
        PreCommitSectorParams {
            seal_proof,
            sector_number,
            sealed_cid,
            seal_rand_epoch: v.get_epoch() - 1,
            deal_ids: deal_ids.clone(),
            expiration: v.get_epoch() + 220 * EPOCHS_IN_DAY,
            ..Default::default()
        },
    );
    let prove_time = v.get_epoch() + Policy::default().pre_commit_challenge_delay + 1;
    let v = advance_by_deadline_to_epoch(v, id_addr, prove_time).0;

    // prove commit, cron, advance to post time
    let prove_params = ProveCommitSectorParams { sector_number, proof: vec![] };
    apply_ok(
        &v,
        worker,
        robust_addr,
        TokenAmount::zero(),
        MethodsMiner::ProveCommitSector as u64,
        prove_params,
    );
    let res = v
        .apply_message(
            *SYSTEM_ACTOR_ADDR,
            *CRON_ACTOR_ADDR,
            TokenAmount::zero(),
            MethodsCron::EpochTick as u64,
            RawBytes::default(),
        )
        .unwrap();
    assert_eq!(ExitCode::OK, res.code);
    let (dline_info, p_idx, v) = advance_to_proving_deadline(v, id_addr, sector_number);
    let d_idx = dline_info.index;
    let st = v.get_state::<MinerState>(id_addr).unwrap();
    let sector = st.get_sector(v.store, sector_number).unwrap().unwrap();
    let sector_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    submit_windowed_post(&v, worker, id_addr, dline_info, p_idx, sector_power);
    let v = v.with_epoch(dline_info.last());

    v.apply_message(
        *SYSTEM_ACTOR_ADDR,
        *CRON_ACTOR_ADDR,
        TokenAmount::zero(),
        MethodsCron::EpochTick as u64,
        RawBytes::default(),
    )
    .unwrap();
    assert_eq!(ExitCode::OK, res.code);

    // advance cron delay epochs so deals are active
    let start = dline_info.close;
    let v = v.with_epoch(start); // get out of proving deadline so we don't post twice
    let v = advance_by_deadline_to_epoch_while_proving(
        v,
        id_addr,
        worker,
        sector_number,
        start + Policy::default().deal_updates_interval,
    );

    // market cron updates deal states indication deals are no longer pending
    let st = v.get_state::<MarketState>(*STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let deal_states = DealMetaArray::load(&st.states, v.store).unwrap();
    for id in deal_ids.iter() {
        let state = deal_states.get(*id).unwrap().unwrap();
        assert!(state.last_updated_epoch > 0);
        assert_eq!(-1, state.slash_epoch);
    }

    // Terminate Sector
    apply_ok(
        &v,
        worker,
        robust_addr,
        TokenAmount::zero(),
        MethodsMiner::TerminateSectors as u64,
        TerminateSectorsParams {
            terminations: vec![TerminationDeclaration {
                deadline: d_idx,
                partition: p_idx,
                sectors: make_bitfield(&[sector_number]),
            }],
        },
    );
    ExpectInvocation {
        to: id_addr,
        method: MethodsMiner::TerminateSectors as u64,
        subinvocs: Some(vec![
            ExpectInvocation {
                to: *REWARD_ACTOR_ADDR,
                method: MethodsReward::ThisEpochReward as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: *STORAGE_POWER_ACTOR_ADDR,
                method: MethodsPower::CurrentTotalPower as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: *BURNT_FUNDS_ACTOR_ADDR,
                method: METHOD_SEND,
                ..Default::default()
            },
            ExpectInvocation {
                to: *STORAGE_POWER_ACTOR_ADDR,
                method: MethodsPower::UpdatePledgeTotal as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: *STORAGE_MARKET_ACTOR_ADDR,
                method: MethodsMarket::OnMinerSectorsTerminate as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: *STORAGE_POWER_ACTOR_ADDR,
                method: MethodsPower::UpdateClaimedPower as u64,
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    let miner_balances = v.get_miner_balance(id_addr);
    assert!(miner_balances.initial_pledge.is_zero());
    assert!(miner_balances.pre_commit_deposit.is_zero());

    let pow_st = v.get_state::<PowerState>(*STORAGE_POWER_ACTOR_ADDR).unwrap();
    assert_eq!(0, pow_st.miner_above_min_power_count);
    assert!(pow_st.total_raw_byte_power.is_zero());
    assert!(pow_st.total_quality_adj_power.is_zero());
    assert!(pow_st.total_bytes_committed.is_zero());
    assert!(pow_st.total_qa_bytes_committed.is_zero());
    assert!(pow_st.total_pledge_collateral.is_zero());

    // termination slashes deals in market state
    let termination_epoch = v.get_epoch();
    let st = v.get_state::<MarketState>(*STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let deal_states = DealMetaArray::load(&st.states, v.store).unwrap();
    for id in deal_ids.iter() {
        let state = deal_states.get(*id).unwrap().unwrap();
        assert!(state.last_updated_epoch > 0);
        assert_eq!(termination_epoch, state.slash_epoch);
    }

    // advance a market cron processing period to process terminations fully
    let (v, _) = advance_by_deadline_to_epoch(
        v,
        id_addr,
        termination_epoch + Policy::default().deal_updates_interval,
    );
    // because of rounding error it's annoying to compute exact withdrawable balance which is 2.9999.. FIL
    // withdrawing 2 FIL proves out that the claim to 1 FIL per deal (2 deals for this client) is removed at termination
    let withdrawal = TokenAmount::from(2e18 as u64);
    apply_ok(
        &v,
        verified_client,
        *STORAGE_MARKET_ACTOR_ADDR,
        TokenAmount::zero(),
        MethodsMarket::WithdrawBalance as u64,
        WithdrawBalanceParams { provider_or_client: verified_client, amount: withdrawal.clone() },
    );
    ExpectInvocation {
        to: *STORAGE_MARKET_ACTOR_ADDR,
        method: MethodsMarket::WithdrawBalance as u64,
        subinvocs: Some(vec![ExpectInvocation {
            to: verified_client,
            method: METHOD_SEND,
            value: Some(withdrawal),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    apply_ok(
        &v,
        worker,
        *STORAGE_MARKET_ACTOR_ADDR,
        TokenAmount::zero(),
        MethodsMarket::WithdrawBalance as u64,
        WithdrawBalanceParams { provider_or_client: id_addr, amount: miner_collateral },
    );

    let value_withdrawn = v.take_invocations().last().unwrap().subinvocations[1].msg.value();
    // miner add 64 balance. Each of 3 deals required 2 FIL collateral, so provider collateral should have been
    // slashed by 6 FIL. Miner's remaining market balance should be 64 - 6 + payment, where payment is for storage
    // before the slash and should be << 1 FIL. Actual amount withdrawn should be between 58 and 59 FIL.
    assert!(TokenAmount::from(58e18 as u128) < value_withdrawn);
    assert!(TokenAmount::from(59e18 as u128) > value_withdrawn);
}

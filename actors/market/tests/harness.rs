use cid::Cid;
use num_traits::FromPrimitive;
use std::collections::HashMap;

use fil_actor_market::{
    balance_table::BalanceTable, ext, ext::miner::GetControlAddressesReturnParams,
    gen_rand_next_epoch, ActivateDealsParams, Actor as MarketActor, ClientDealProposal, DealArray,
    DealMetaArray, DealProposal, DealState, Method, OnMinerSectorsTerminateParams,
    PublishStorageDealsParams, PublishStorageDealsReturn, State, WithdrawBalanceParams,
    WithdrawBalanceReturn, PROPOSALS_AMT_BITWIDTH,
};
use fil_actor_power::{CurrentTotalPowerReturn, Method as PowerMethod};
use fil_actor_reward::Method as RewardMethod;
use fil_actor_verifreg::UseBytesParams;
use fil_actors_runtime::runtime::{Policy, Runtime};
use fil_actors_runtime::{
    test_utils::*, ActorError, SetMultimap, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR,
    REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::crypto::signature::Signature;
use fvm_shared::deal::DealID;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::reward::ThisEpochRewardReturn;
use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::{
    address::Address, econ::TokenAmount, error::ExitCode, METHOD_CONSTRUCTOR, METHOD_SEND,
};

pub const OWNER_ID: u64 = 101;
pub const PROVIDER_ID: u64 = 102;
pub const WORKER_ID: u64 = 103;
pub const CLIENT_ID: u64 = 104;
pub const CONTROL_ID: u64 = 200;

pub const OWNER_ADDR: Address = Address::new_id(OWNER_ID);
pub const PROVIDER_ADDR: Address = Address::new_id(PROVIDER_ID);
pub const WORKER_ADDR: Address = Address::new_id(WORKER_ID);
pub const CLIENT_ADDR: Address = Address::new_id(CLIENT_ID);
pub const CONTROL_ADDR: Address = Address::new_id(CONTROL_ID);

pub fn setup() -> MockRuntime {
    let mut actor_code_cids = HashMap::default();
    actor_code_cids.insert(Address::new_id(OWNER_ID), *ACCOUNT_ACTOR_CODE_ID);
    actor_code_cids.insert(Address::new_id(WORKER_ID), *ACCOUNT_ACTOR_CODE_ID);
    actor_code_cids.insert(Address::new_id(PROVIDER_ID), *MINER_ACTOR_CODE_ID);
    actor_code_cids.insert(Address::new_id(CLIENT_ID), *ACCOUNT_ACTOR_CODE_ID);

    let mut rt = MockRuntime {
        receiver: *STORAGE_MARKET_ACTOR_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *INIT_ACTOR_CODE_ID,
        actor_code_cids,
        ..Default::default()
    };
    construct_and_verify(&mut rt);

    rt
}

pub fn check_state(_rt: &mut MockRuntime) {
    // TODO
}

pub fn construct_and_verify(rt: &mut MockRuntime) {
    rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(METHOD_CONSTRUCTOR, &RawBytes::default(),).unwrap()
    );
    rt.verify();
}

pub fn get_escrow_balance(rt: &MockRuntime, addr: &Address) -> Result<TokenAmount, ActorError> {
    let st: State = rt.get_state();

    let et = BalanceTable::from_root(rt.store(), &st.escrow_table).unwrap();

    Ok(et.get(addr).unwrap())
}

pub fn expect_get_control_addresses(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
    controls: Vec<Address>,
) {
    let result = GetControlAddressesReturnParams { owner, worker, control_addresses: controls };

    rt.expect_send(
        provider,
        ext::miner::CONTROL_ADDRESSES_METHOD,
        RawBytes::default(),
        BigInt::from(0u8),
        RawBytes::serialize(result).unwrap(),
        ExitCode::OK,
    )
}

pub fn expect_provider_control_address(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    expect_get_control_addresses(rt, provider, owner, worker, vec![])
}

pub fn add_provider_funds(
    rt: &mut MockRuntime,
    amount: TokenAmount,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    rt.set_value(amount.clone());
    rt.set_address_actor_type(provider, *MINER_ACTOR_CODE_ID);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, owner);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

    expect_provider_control_address(rt, provider, owner, worker);

    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(Method::AddBalance as u64, &RawBytes::serialize(provider).unwrap(),)
            .unwrap()
    );
    rt.verify();
    rt.add_balance(amount);
}

pub fn add_participant_funds(rt: &mut MockRuntime, addr: Address, amount: TokenAmount) {
    rt.set_value(amount.clone());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addr);

    rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

    assert!(rt
        .call::<MarketActor>(Method::AddBalance as u64, &RawBytes::serialize(addr).unwrap(),)
        .is_ok());

    rt.verify();

    rt.add_balance(amount);
}

pub fn withdraw_provider_balance(
    rt: &mut MockRuntime,
    withdraw_amount: TokenAmount,
    expected_send: TokenAmount,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    rt.expect_validate_caller_addr(vec![owner, worker]);
    expect_provider_control_address(rt, provider, owner, worker);

    let params = WithdrawBalanceParams { provider_or_client: provider, amount: withdraw_amount };

    rt.expect_send(
        owner,
        METHOD_SEND,
        RawBytes::default(),
        expected_send.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    let ret: WithdrawBalanceReturn = rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap())
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(
        expected_send, ret.amount_withdrawn,
        "return value indicates {} withdrawn but expected {}",
        ret.amount_withdrawn, expected_send
    );
}

pub fn withdraw_client_balance(
    rt: &mut MockRuntime,
    withdraw_amount: TokenAmount,
    expected_send: TokenAmount,
    client: Address,
) {
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, client);
    rt.expect_send(
        client,
        METHOD_SEND,
        RawBytes::default(),
        expected_send.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    rt.expect_validate_caller_addr(vec![client]);

    let params = WithdrawBalanceParams { provider_or_client: client, amount: withdraw_amount };

    let ret: WithdrawBalanceReturn = rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap())
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(
        expected_send, ret.amount_withdrawn,
        "return value indicates {} withdrawn but expected {}",
        ret.amount_withdrawn, expected_send
    );
}

pub fn activate_deals(
    rt: &mut MockRuntime,
    sector_expiry: ChainEpoch,
    provider: Address,
    current_epoch: ChainEpoch,
    deal_ids: &[DealID],
) {
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

    let params = ActivateDealsParams { deal_ids: deal_ids.to_vec(), sector_expiry };

    let ret = rt
        .call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap())
        .unwrap();
    assert_eq!(ret, RawBytes::default());
    rt.verify();

    for d in deal_ids {
        let s = get_deal_state(rt, *d);
        assert_eq!(current_epoch, s.sector_start_epoch);
    }
}

pub fn get_deal_proposal(rt: &mut MockRuntime, deal_id: DealID) -> DealProposal {
    let st: State = rt.get_state();

    let deals = DealArray::load(&st.proposals, &rt.store).unwrap();

    let d = deals.get(deal_id).unwrap();
    d.unwrap().clone()
}

pub fn get_locked_balance(rt: &mut MockRuntime, addr: Address) -> TokenAmount {
    let st: State = rt.get_state();

    let lt = BalanceTable::from_root(&rt.store, &st.locked_table).unwrap();

    lt.get(&addr).unwrap()
}

pub fn get_deal_state(rt: &mut MockRuntime, deal_id: DealID) -> DealState {
    let st: State = rt.get_state();

    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();

    let s = states.get(deal_id).unwrap();
    *s.unwrap()
}

pub fn update_last_updated(rt: &mut MockRuntime, deal_id: DealID, new_last_updated: ChainEpoch) {
    let st: State = rt.get_state();

    let mut states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = *states.get(deal_id).unwrap().unwrap();

    states.set(deal_id, DealState { last_updated_epoch: new_last_updated, ..s }).unwrap();
    let root = states.flush().unwrap();
    rt.replace_state(&State { states: root, ..st })
}

pub fn delete_deal_proposal(rt: &mut MockRuntime, deal_id: DealID) {
    let mut st: State = rt.get_state();

    let mut deals = DealArray::load(&st.proposals, &rt.store).unwrap();
    deals.delete(deal_id).unwrap();

    let root = deals.flush().unwrap();
    st.proposals = root;
    rt.replace_state(&st)
}

// if this is the first crontick for the deal, it's next tick will be scheduled at `desiredNextEpoch`
// if this is not the first crontick, the `desiredNextEpoch` param is ignored.
pub fn cron_tick_and_assert_balances(
    rt: &mut MockRuntime,
    client_addr: Address,
    provider_addr: Address,
    current_epoch: ChainEpoch,
    deal_id: DealID,
) -> (TokenAmount, TokenAmount) {
    // fetch current client and provider escrow balances
    let c_locked = get_locked_balance(rt, client_addr);
    let c_escrow = get_escrow_balance(rt, &client_addr).unwrap();
    let p_locked = get_locked_balance(rt, provider_addr);
    let p_escrow = get_escrow_balance(rt, &provider_addr).unwrap();
    let mut amount_slashed = TokenAmount::from(0u8);

    let s = get_deal_state(rt, deal_id);
    let d = get_deal_proposal(rt, deal_id);

    // end epoch for payment calc
    let mut payment_end = d.end_epoch;
    if s.slash_epoch != EPOCH_UNDEFINED {
        rt.expect_send(
            *BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            RawBytes::default(),
            d.provider_collateral.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        amount_slashed = d.provider_collateral;

        if s.slash_epoch < d.start_epoch {
            payment_end = d.start_epoch;
        } else {
            payment_end = s.slash_epoch;
        }
    } else if current_epoch < payment_end {
        payment_end = current_epoch;
    }

    // start epoch for payment calc
    let mut payment_start = d.start_epoch;
    if s.last_updated_epoch != EPOCH_UNDEFINED {
        payment_start = s.last_updated_epoch;
    }
    let duration = payment_end - payment_start;
    let payment = duration * d.storage_price_per_epoch;

    // expected updated amounts
    let updated_client_escrow = c_escrow - payment.clone();
    let updated_provider_escrow = (p_escrow + payment.clone()) - amount_slashed.clone();
    let mut updated_client_locked = c_locked - payment.clone();
    let mut updated_provider_locked = p_locked;
    // if the deal has expired or been slashed, locked amount will be zero for provider and client.
    let is_deal_expired = payment_end == d.end_epoch;
    if is_deal_expired || s.slash_epoch != EPOCH_UNDEFINED {
        updated_client_locked = TokenAmount::from(0u8);
        updated_provider_locked = TokenAmount::from(0u8);
    }

    cron_tick(rt);

    assert_eq!(updated_client_escrow, get_escrow_balance(rt, &client_addr).unwrap());
    assert_eq!(updated_client_locked, get_locked_balance(rt, client_addr));
    assert_eq!(updated_provider_escrow, get_escrow_balance(rt, &provider_addr).unwrap());
    assert_eq!(updated_provider_locked, get_locked_balance(rt, provider_addr));
    (payment, amount_slashed)
}

pub fn publish_deals(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    publish_deals: &[DealProposal],
) -> Vec<DealID> {
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

    let return_value = ext::miner::GetControlAddressesReturnParams {
        owner,
        worker,
        control_addresses: vec![control],
    };
    rt.expect_send(
        provider,
        ext::miner::CONTROL_ADDRESSES_METHOD,
        RawBytes::default(),
        TokenAmount::from(0u8),
        RawBytes::serialize(return_value).unwrap(),
        ExitCode::OK,
    );

    expect_query_network_info(rt);

    let mut params: PublishStorageDealsParams = PublishStorageDealsParams { deals: vec![] };

    for deal in publish_deals {
        // create a client proposal with a valid signature
        let buf = RawBytes::serialize(deal.clone()).expect("failed to marshal deal proposal");
        let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
        let client_proposal =
            ClientDealProposal { proposal: deal.clone(), client_signature: sig.clone() };
        params.deals.push(client_proposal);

        // expect a call to verify the above signature
        rt.expect_verify_signature(ExpectedVerifySig {
            sig,
            signer: deal.client,
            plaintext: buf.to_vec(),
            result: Ok(()),
        });
        if deal.verified_deal {
            let param = RawBytes::serialize(UseBytesParams {
                address: deal.client,
                deal_size: BigInt::from(deal.piece_size.0),
            })
            .unwrap();

            rt.expect_send(
                *VERIFIED_REGISTRY_ACTOR_ADDR,
                ext::verifreg::USE_BYTES_METHOD as u64,
                param,
                TokenAmount::from(0u8),
                RawBytes::default(),
                ExitCode::OK,
            );
        }
    }

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(ret.ids.len(), publish_deals.len());

    // assert state after publishing the deals
    for (i, deal_id) in ret.ids.iter().enumerate() {
        let expected = &publish_deals[i];
        let p = get_deal_proposal(rt, *deal_id);

        assert_eq!(expected, &p);
    }

    ret.ids
}

pub fn assert_deals_not_activated(rt: &mut MockRuntime, _epoch: ChainEpoch, deal_ids: &[DealID]) {
    let st: State = rt.get_state();

    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();

    for d in deal_ids {
        let opt = states.get(*d).unwrap();
        assert!(opt.is_none());
    }
}

pub fn cron_tick(rt: &mut MockRuntime) {
    assert_eq!(RawBytes::default(), cron_tick_raw(rt).unwrap());
    rt.verify()
}

pub fn cron_tick_raw(rt: &mut MockRuntime) -> Result<RawBytes, ActorError> {
    rt.expect_validate_caller_addr(vec![*CRON_ACTOR_ADDR]);
    rt.set_caller(*CRON_ACTOR_CODE_ID, *CRON_ACTOR_ADDR);

    rt.call::<MarketActor>(Method::CronTick as u64, &RawBytes::default())
}

pub fn expect_query_network_info(rt: &mut MockRuntime) {
    //networkQAPower
    //networkBaselinePower
    let rwd = TokenAmount::from(10u8) * TokenAmount::from(10_i128.pow(18));
    let power = StoragePower::from_i128(1 << 50).unwrap();
    let epoch_reward_smooth = FilterEstimate::new(rwd.clone(), BigInt::from(0u8));

    let current_power = CurrentTotalPowerReturn {
        raw_byte_power: StoragePower::default(),
        quality_adj_power: power.clone(),
        pledge_collateral: TokenAmount::default(),
        quality_adj_power_smoothed: FilterEstimate::new(rwd, TokenAmount::default()),
    };
    let current_reward = ThisEpochRewardReturn {
        this_epoch_baseline_power: power,
        this_epoch_reward_smoothed: epoch_reward_smooth,
    };
    rt.expect_send(
        *REWARD_ACTOR_ADDR,
        RewardMethod::ThisEpochReward as u64,
        RawBytes::default(),
        TokenAmount::from(0u8),
        RawBytes::serialize(current_reward).unwrap(),
        ExitCode::OK,
    );
    rt.expect_send(
        *STORAGE_POWER_ACTOR_ADDR,
        PowerMethod::CurrentTotalPower as u64,
        RawBytes::default(),
        TokenAmount::from(0u8),
        RawBytes::serialize(current_power).unwrap(),
        ExitCode::OK,
    );
}

pub fn assert_n_good_deals<BS>(dobe: &SetMultimap<BS>, epoch: ChainEpoch, n: isize)
where
    BS: fvm_ipld_blockstore::Blockstore,
{
    let deal_updates_interval = Policy::default().deal_updates_interval;
    let mut count = 0;
    dobe.for_each(epoch, |id| {
        assert_eq!(epoch % deal_updates_interval, (id as i64) % deal_updates_interval);
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(n, count, "unexpected deal count at epoch {}", epoch);
}

pub fn assert_deals_terminated(rt: &mut MockRuntime, epoch: ChainEpoch, deal_ids: &[DealID]) {
    for &deal_id in deal_ids {
        let s = get_deal_state(rt, deal_id);
        assert_eq!(s.slash_epoch, epoch);
    }
}

pub fn assert_deals_not_terminated(rt: &mut MockRuntime, deal_ids: &[DealID]) {
    for &deal_id in deal_ids {
        let s = get_deal_state(rt, deal_id);
        assert_eq!(s.slash_epoch, EPOCH_UNDEFINED);
    }
}

pub fn assert_deal_deleted(rt: &mut MockRuntime, deal_id: DealID, p: DealProposal) {
    use cid::multihash::Code;
    use cid::multihash::MultihashDigest;
    use fvm_ipld_hamt::{BytesKey, Hamt};

    let st: State = rt.get_state();

    // Check that the deal_id is not in st.proposals.
    let deals = DealArray::load(&st.proposals, &rt.store).unwrap();
    let d = deals.get(deal_id).unwrap();
    assert!(d.is_none());

    // Check that the deal_id is not in st.states
    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = states.get(deal_id).unwrap();
    assert!(s.is_none());

    let mh_code = Code::Blake2b256;
    let p_cid = Cid::new_v1(fvm_ipld_encoding::DAG_CBOR, mh_code.digest(&to_vec(&p).unwrap()));
    // Check that the deal_id is not in st.pending_proposals.
    let pending_deals: Hamt<&fvm_ipld_blockstore::MemoryBlockstore, DealProposal> =
        fil_actors_runtime::make_map_with_root_and_bitwidth(
            &st.pending_proposals,
            &rt.store,
            PROPOSALS_AMT_BITWIDTH,
        )
        .unwrap();
    assert!(!pending_deals.contains_key(&BytesKey(p_cid.to_bytes())).unwrap());
}

pub fn process_epoch(start_epoch: ChainEpoch, deal_id: DealID) -> ChainEpoch {
    let policy = Policy::default();
    gen_rand_next_epoch(&policy, start_epoch, deal_id)
}

#[allow(clippy::too_many_arguments)]
pub fn publish_and_activate_deal(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    current_epoch: ChainEpoch,
    sector_expiry: ChainEpoch,
) -> DealID {
    let deal =
        generate_deal_and_add_funds(rt, client, provider, owner, worker, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    let deal_ids = publish_deals(rt, provider, owner, worker, control, &[deal]);
    activate_deals(rt, sector_expiry, provider, current_epoch, &deal_ids);
    deal_ids[0]
}

#[allow(clippy::too_many_arguments)]
pub fn generate_and_publish_deal(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealID {
    let deal =
        generate_deal_and_add_funds(rt, client, provider, owner, worker, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    let deal_ids = publish_deals(rt, provider, owner, worker, control, &[deal]);
    deal_ids[0]
}

#[allow(clippy::too_many_arguments)]
pub fn generate_and_publish_deal_for_piece(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    piece_cid: Cid,
    piece_size: PaddedPieceSize,
) -> DealID {
    // generate deal
    let storage_per_epoch = BigInt::from(10u8);
    let client_collateral = TokenAmount::from(10u8);
    let provider_collateral = TokenAmount::from(10u8);

    let deal = DealProposal {
        piece_cid,
        piece_size,
        verified_deal: true,
        client,
        provider,
        label: "label".to_string(),
        start_epoch,
        end_epoch,
        storage_price_per_epoch: storage_per_epoch,
        provider_collateral,
        client_collateral,
    };

    // add funds
    add_provider_funds(rt, deal.provider_collateral.clone(), provider, owner, worker);
    add_participant_funds(rt, client, deal.client_balance_requirement());

    // publish
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    let deal_ids = publish_deals(rt, provider, owner, worker, control, &[deal]);
    deal_ids[0]
}

pub fn generate_deal_and_add_funds(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let deal = generate_deal_proposal(client, provider, start_epoch, end_epoch);
    add_provider_funds(rt, deal.provider_collateral.clone(), provider, owner, worker);
    add_participant_funds(rt, client, deal.client_balance_requirement());
    deal
}

#[allow(clippy::too_many_arguments)]
pub fn generate_deal_with_collateral_and_add_funds(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    provider_collateral: BigInt,
    client_collateral: BigInt,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let deal = generate_deal_proposal_with_collateral(
        client,
        provider,
        client_collateral,
        provider_collateral,
        start_epoch,
        end_epoch,
    );
    add_provider_funds(rt, deal.provider_collateral.clone(), provider, owner, worker);
    add_participant_funds(rt, client, deal.client_balance_requirement());
    deal
}

fn generate_deal_proposal_with_collateral(
    client: Address,
    provider: Address,
    client_collateral: TokenAmount,
    provider_collateral: TokenAmount,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let piece_cid = make_piece_cid("1".as_bytes());
    let piece_size = PaddedPieceSize(2048u64);
    let storage_per_epoch = BigInt::from(10u8);
    DealProposal {
        piece_cid,
        piece_size,
        verified_deal: true,
        client,
        provider,
        label: "label".to_string(),
        start_epoch,
        end_epoch,
        storage_price_per_epoch: storage_per_epoch,
        provider_collateral,
        client_collateral,
    }
}

pub fn generate_deal_proposal(
    client: Address,
    provider: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let client_collateral = TokenAmount::from(10u8);
    let provider_collateral = TokenAmount::from(10u8);
    generate_deal_proposal_with_collateral(
        client,
        provider,
        client_collateral,
        provider_collateral,
        start_epoch,
        end_epoch,
    )
}

pub fn terminate_deals(rt: &mut MockRuntime, miner_addr: Address, deal_ids: &[DealID]) {
    let ret = terminate_deals_raw(rt, miner_addr, deal_ids).unwrap();
    assert_eq!(ret, RawBytes::default());
    rt.verify();
}

pub fn terminate_deals_raw(
    rt: &mut MockRuntime,
    miner_addr: Address,
    deal_ids: &[DealID],
) -> Result<RawBytes, ActorError> {
    rt.set_caller(*MINER_ACTOR_CODE_ID, miner_addr);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

    let params = OnMinerSectorsTerminateParams { epoch: rt.epoch, deal_ids: deal_ids.to_vec() };

    rt.call::<MarketActor>(
        Method::OnMinerSectorsTerminate as u64,
        &RawBytes::serialize(params).unwrap(),
    )
}

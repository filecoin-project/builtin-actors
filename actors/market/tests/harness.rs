#![allow(dead_code)]

use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::{cell::RefCell, collections::HashMap, collections::HashSet};

use cid::Cid;
use fil_actors_runtime::reward::{FilterEstimate, ThisEpochRewardReturn};
use frc46_token::token::types::{TransferFromParams, TransferFromReturn};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::crypto::signature::Signature;
use fvm_shared::deal::DealID;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use fvm_shared::sys::SendFlags;
use fvm_shared::{
    address::Address, econ::TokenAmount, error::ExitCode, ActorID, METHOD_CONSTRUCTOR, METHOD_SEND,
};
use num_traits::{FromPrimitive, Zero};
use regex::Regex;

use fil_actor_market::ext::account::{AuthenticateMessageParams, AUTHENTICATE_MESSAGE_METHOD};
use fil_actor_market::ext::miner::{
    PieceChange, SectorChanges, SectorContentChangedParams, SectorContentChangedReturn,
};
use fil_actor_market::ext::verifreg::{AllocationID, AllocationRequest, AllocationsResponse};
use fil_actor_market::{
    deal_cid, deal_get_payment_remaining, BatchActivateDealsParams, BatchActivateDealsResult,
    DealOpsByEpoch, PendingDealAllocationsMap, PendingProposalsSet, ProviderSectorsMap,
    SectorDealsMap, SettleDealPaymentsParams, SettleDealPaymentsReturn, PENDING_ALLOCATIONS_CONFIG,
    PENDING_PROPOSALS_CONFIG, PROVIDER_SECTORS_CONFIG, SECTOR_DEALS_CONFIG,
};
use fil_actor_market::{
    ext, ext::miner::GetControlAddressesReturnParams, next_update_epoch,
    testing::check_state_invariants, Actor as MarketActor, ClientDealProposal, DealArray,
    DealMetaArray, DealProposal, DealState, GetBalanceReturn, Label, MarketNotifyDealParams,
    Method, OnMinerSectorsTerminateParams, PublishStorageDealsParams, PublishStorageDealsReturn,
    SectorDeals, State, VerifyDealsForActivationParams, VerifyDealsForActivationReturn,
    WithdrawBalanceParams, WithdrawBalanceReturn, MARKET_NOTIFY_DEAL_METHOD, NO_ALLOCATION_ID,
};
use fil_actor_power::{CurrentTotalPowerReturn, Method as PowerMethod};
use fil_actor_reward::Method as RewardMethod;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::{
    network::EPOCHS_IN_DAY,
    runtime::{builtins::Type, Policy, Runtime},
    test_utils::*,
    ActorError, BatchReturn, EventBuilder, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR,
    DATACAP_TOKEN_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};

// Define common set of actor ids that will be used across all tests.
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

lazy_static::lazy_static! {
    pub static ref AUTHENTICATE_MESSAGE_RESPONSE: Option<IpldBlock> =
        IpldBlock::serialize_cbor(&true).unwrap();
}

pub struct MinerAddresses {
    pub owner: Address,
    pub worker: Address,
    pub provider: Address,
    pub control: Vec<Address>,
}

// Use the predefined actor addresses by default
impl Default for MinerAddresses {
    fn default() -> Self {
        MinerAddresses {
            owner: OWNER_ADDR,
            worker: WORKER_ADDR,
            provider: PROVIDER_ADDR,
            control: vec![CONTROL_ADDR],
        }
    }
}

pub fn setup() -> MockRuntime {
    let actor_code_cids = HashMap::from([
        (OWNER_ADDR, *ACCOUNT_ACTOR_CODE_ID),
        (WORKER_ADDR, *ACCOUNT_ACTOR_CODE_ID),
        (PROVIDER_ADDR, *MINER_ACTOR_CODE_ID),
        (CLIENT_ADDR, *ACCOUNT_ACTOR_CODE_ID),
    ]);

    let rt = MockRuntime {
        receiver: STORAGE_MARKET_ACTOR_ADDR,
        caller: RefCell::new(SYSTEM_ACTOR_ADDR),
        caller_type: RefCell::new(*INIT_ACTOR_CODE_ID),
        actor_code_cids: RefCell::new(actor_code_cids),
        balance: RefCell::new(TokenAmount::from_whole(10)),
        ..Default::default()
    };

    construct_and_verify(&rt);

    rt
}

/// Checks that there are no dangling deal ops in the queue waiting to be cleaned up
/// Dangling deal ops are a valid transient state, but the deal ids should eventually be removed
/// from the queue when attepting to process them in cron.
// NOTE: this is only a concern during the transition period from cron-serviced deals and this
// check can likely be removed with https://github.com/filecoin-project/builtin-actors/issues/1389
// TODO: when this check is removed, add back the check in market state invariants as at that point
// there should be no active deals in the queue
pub fn assert_deal_ops_clean(rt: &MockRuntime) {
    let st: State = rt.get_state();

    let mut proposal_set = HashSet::<DealID>::new();
    let proposals = DealArray::load(&st.proposals, rt.store()).unwrap();
    proposals
        .for_each(|deal_id, _| {
            proposal_set.insert(deal_id);
            Ok(())
        })
        .unwrap();

    let deal_ops = st.load_deal_ops(rt.store()).unwrap();
    deal_ops.for_each(|epoch, _| {
        deal_ops
            .for_each_in(&epoch, |deal_id| {
                assert!(proposal_set.contains(&deal_id), "deal op found for deal id {deal_id} with missing proposal at epoch {epoch}");
                Ok(())
            })
            .unwrap();
        Ok(())
    }).unwrap();
}

/// Checks internal invariants of market state asserting none of them are broken.
pub fn check_state(rt: &MockRuntime) {
    let (_, acc) = check_state_invariants(
        &rt.get_state::<State>(),
        rt.store(),
        &rt.get_balance(),
        *rt.epoch.borrow(),
    );
    acc.assert_empty();
}

/// Checks state, allowing expected invariants to fail. The invariants *must* fail in the
/// provided order.
pub fn check_state_with_expected(rt: &MockRuntime, expected_patterns: &[Regex]) {
    let (_, acc) = check_state_invariants(
        &rt.get_state::<State>(),
        rt.store(),
        &rt.get_balance(),
        *rt.epoch.borrow(),
    );
    acc.assert_expected(expected_patterns);
}

pub fn construct_and_verify(rt: &MockRuntime) {
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    assert!(rt.call::<MarketActor>(METHOD_CONSTRUCTOR, None).unwrap().is_none());
    rt.verify();
}

pub fn get_balance(rt: &MockRuntime, addr: &Address) -> GetBalanceReturn {
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(1234));
    rt.expect_validate_caller_any();
    let ret: GetBalanceReturn = rt
        .call::<MarketActor>(
            Method::GetBalanceExported as u64,
            IpldBlock::serialize_cbor(addr).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();
    ret
}

pub fn expect_get_control_addresses(
    rt: &MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
    controls: Vec<Address>,
) {
    let result = GetControlAddressesReturnParams { owner, worker, control_addresses: controls };

    rt.expect_send_simple(
        provider,
        ext::miner::CONTROL_ADDRESSES_METHOD,
        None,
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&result).unwrap(),
        ExitCode::OK,
    )
}

pub fn expect_provider_control_address(
    rt: &MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    expect_get_control_addresses(rt, provider, owner, worker, vec![])
}

pub fn expect_provider_is_control_address(
    rt: &MockRuntime,
    provider: Address,
    caller: Address,
    is_controlling: bool,
) {
    let result = ext::miner::IsControllingAddressReturn { is_controlling };

    rt.expect_send_simple(
        provider,
        ext::miner::IS_CONTROLLING_ADDRESS_EXPORTED,
        IpldBlock::serialize_cbor(&ext::miner::IsControllingAddressParam { address: caller })
            .unwrap(),
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&result).unwrap(),
        ExitCode::OK,
    )
}

pub fn add_provider_funds(rt: &MockRuntime, amount: TokenAmount, addrs: &MinerAddresses) {
    rt.set_received(amount.clone());
    rt.set_address_actor_type(addrs.provider, *MINER_ACTOR_CODE_ID);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.owner);
    rt.expect_validate_caller_any();

    expect_provider_control_address(rt, addrs.provider, addrs.owner, addrs.worker);

    assert!(rt
        .call::<MarketActor>(
            Method::AddBalance as u64,
            IpldBlock::serialize_cbor(&addrs.provider).unwrap(),
        )
        .unwrap()
        .is_none(),);
    rt.verify();
    rt.add_balance(amount);
}

pub fn add_participant_funds(rt: &MockRuntime, addr: Address, amount: TokenAmount) {
    rt.set_received(amount.clone());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addr);

    rt.expect_validate_caller_any();
    assert!(rt
        .call::<MarketActor>(Method::AddBalance as u64, IpldBlock::serialize_cbor(&addr).unwrap())
        .is_ok());

    rt.verify();

    rt.add_balance(amount);
}

pub fn withdraw_provider_balance(
    rt: &MockRuntime,
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

    rt.expect_send_simple(owner, METHOD_SEND, None, expected_send.clone(), None, ExitCode::OK);
    let ret: WithdrawBalanceReturn = rt
        .call::<MarketActor>(
            Method::WithdrawBalance as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
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
    rt: &MockRuntime,
    withdraw_amount: TokenAmount,
    expected_send: TokenAmount,
    client: Address,
) {
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, client);
    rt.expect_send_simple(client, METHOD_SEND, None, expected_send.clone(), None, ExitCode::OK);
    rt.expect_validate_caller_addr(vec![client]);

    let params = WithdrawBalanceParams { provider_or_client: client, amount: withdraw_amount };

    let ret: WithdrawBalanceReturn = rt
        .call::<MarketActor>(
            Method::WithdrawBalance as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
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

pub fn create_deal(
    rt: &MockRuntime,
    client_addr: Address,
    miner_addrs: &MinerAddresses,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    verified: bool,
) -> DealProposal {
    let mut deal =
        generate_deal_and_add_funds(rt, client_addr, miner_addrs, start_epoch, end_epoch);
    deal.verified_deal = verified;
    deal
}

/// Activate a single sector of deals
pub fn activate_deals_legacy(
    rt: &MockRuntime,
    sector_expiry: ChainEpoch,
    provider: Address,
    current_epoch: ChainEpoch,
    sector_number: SectorNumber,
    deal_ids: &[DealID],
) -> BatchActivateDealsResult {
    let ret = activate_deals(rt, sector_expiry, provider, current_epoch, sector_number, deal_ids);

    for deal_id in deal_ids {
        simulate_legacy_deal(rt, *deal_id, current_epoch);
    }

    ret
}

/// Activate a single sector of deals
pub fn activate_deals(
    rt: &MockRuntime,
    sector_expiry: ChainEpoch,
    provider: Address,
    current_epoch: ChainEpoch,
    sector_number: SectorNumber,
    deal_ids: &[DealID],
) -> BatchActivateDealsResult {
    activate_deals_for(
        rt,
        sector_expiry,
        provider,
        current_epoch,
        sector_number,
        deal_ids,
        deal_ids,
    )
}

/// Activate a single sector of deals
pub fn activate_deals_for(
    rt: &MockRuntime,
    sector_expiry: ChainEpoch,
    provider: Address,
    current_epoch: ChainEpoch,
    sector_number: SectorNumber,
    deal_ids: &[DealID],
    expected_deal_activations: &[DealID],
) -> BatchActivateDealsResult {
    rt.set_epoch(current_epoch);
    let compute_cid = false;
    let ret = batch_activate_deals_raw(
        rt,
        provider,
        vec![SectorDeals {
            sector_number,
            deal_ids: deal_ids.into(),
            sector_expiry,
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
        }],
        compute_cid,
        expected_deal_activations,
    )
    .unwrap();

    let ret: BatchActivateDealsResult =
        ret.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    // batch size was 1
    if ret.activation_results.all_ok() {
        for deal in deal_ids {
            let s = get_deal_state(rt, *deal);
            assert_eq!(current_epoch, s.sector_start_epoch);
        }
    }

    ret
}

/// Batch activate deals across multiple sectors
/// For each sector, provide its expiry  list of unique, valid deal ids contained within
pub fn batch_activate_deals(
    rt: &MockRuntime,
    provider: Address,
    sectors: &[(SectorNumber, ChainEpoch, Vec<DealID>)],
    compute_cid: bool,
) -> BatchActivateDealsResult {
    let sectors_deals: Vec<SectorDeals> = sectors
        .iter()
        .map(|(sector_number, sector_expiry, deal_ids)| SectorDeals {
            sector_number: *sector_number,
            deal_ids: deal_ids.clone(),
            sector_expiry: *sector_expiry,
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
        })
        .collect();

    let deal_ids =
        sectors.iter().flat_map(|(_, _, deal_ids)| deal_ids).cloned().collect::<Vec<_>>();

    let ret =
        batch_activate_deals_raw(rt, provider, sectors_deals, compute_cid, &deal_ids).unwrap();

    let ret: BatchActivateDealsResult =
        ret.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    // check all deals were activated correctly
    assert!(ret.activation_results.all_ok());

    ret
}

pub fn batch_activate_deals_raw(
    rt: &MockRuntime,
    provider: Address,
    sectors_deals: Vec<SectorDeals>,
    compute_cid: bool,
    expected_activated_deals: &[DealID],
) -> Result<Option<IpldBlock>, ActorError> {
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = BatchActivateDealsParams { sectors: sectors_deals, compute_cid };

    for deal_id in expected_activated_deals {
        let dp = get_deal_proposal(rt, *deal_id);

        expect_emitted(
            rt,
            "deal-activated",
            *deal_id,
            dp.client.id().unwrap(),
            dp.provider.id().unwrap(),
        );
    }
    let ret = rt.call::<MarketActor>(
        Method::BatchActivateDeals as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    )?;
    rt.verify();

    Ok(ret)
}

pub fn sector_content_changed(
    rt: &MockRuntime,
    provider: Address,
    sectors: Vec<SectorChanges>,
) -> Result<SectorContentChangedReturn, ActorError> {
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider);
    rt.expect_validate_caller_type(vec![Type::Miner]);
    let params = SectorContentChangedParams { sectors };

    let ret = rt.call::<MarketActor>(
        Method::SectorContentChangedExported as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    )?;
    rt.verify();
    Ok(ret.unwrap().deserialize().expect("SectorContentChanged failed"))
}

pub fn get_deal_proposal(rt: &MockRuntime, deal_id: DealID) -> DealProposal {
    find_deal_proposal(rt, deal_id).unwrap()
}

pub fn find_deal_proposal(rt: &MockRuntime, deal_id: DealID) -> Option<DealProposal> {
    let st: State = rt.get_state();
    let deals = DealArray::load(&st.proposals, &rt.store).unwrap();
    let d = deals.get(deal_id).unwrap();
    d.cloned()
}

pub fn get_pending_deal_allocation(rt: &MockRuntime, deal_id: DealID) -> AllocationID {
    let st: State = rt.get_state();
    let pending_allocations = PendingDealAllocationsMap::load(
        &rt.store,
        &st.pending_deal_allocation_ids,
        PENDING_ALLOCATIONS_CONFIG,
        "pending deal allocations",
    )
    .unwrap();
    *pending_allocations.get(&deal_id).unwrap().unwrap_or(&NO_ALLOCATION_ID)
}

pub fn get_deal_state(rt: &MockRuntime, deal_id: DealID) -> DealState {
    let st: State = rt.get_state();
    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = states.get(deal_id).unwrap();
    *s.unwrap()
}

// Returns the deal IDs associated with a provider address and sectors from state
pub fn get_sector_deal_ids(
    rt: &MockRuntime,
    provider: ActorID,
    sector_numbers: &[SectorNumber],
) -> Vec<DealID> {
    let st: State = rt.get_state();
    let provider_sectors = ProviderSectorsMap::load(
        &rt.store,
        &st.provider_sectors,
        PROVIDER_SECTORS_CONFIG,
        "provider sectors",
    )
    .unwrap();
    let sectors_root: Option<&Cid> = provider_sectors.get(&provider).unwrap();
    if let Some(sectors_root) = sectors_root {
        let sector_deals =
            SectorDealsMap::load(&rt.store, sectors_root, SECTOR_DEALS_CONFIG, "sector deals")
                .unwrap();
        sector_numbers
            .iter()
            .flat_map(|sector_number| {
                let deals: Option<&Vec<DealID>> = sector_deals.get(sector_number).unwrap();
                match deals {
                    Some(deals) => deals.clone(),
                    None => vec![],
                }
            })
            .collect()
    } else {
        vec![]
    }
}

pub fn update_last_updated(rt: &MockRuntime, deal_id: DealID, new_last_updated: ChainEpoch) {
    let st: State = rt.get_state();
    let mut states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = *states.get(deal_id).unwrap().unwrap();

    states.set(deal_id, DealState { last_updated_epoch: new_last_updated, ..s }).unwrap();
    let root = states.flush().unwrap();
    rt.replace_state(&State { states: root, ..st })
}

pub fn delete_deal_proposal(rt: &MockRuntime, deal_id: DealID) {
    let mut st: State = rt.get_state();
    let mut deals = DealArray::load(&st.proposals, &rt.store).unwrap();
    deals.delete(deal_id).unwrap();

    let root = deals.flush().unwrap();
    st.proposals = root;
    rt.replace_state(&st)
}

pub fn cron_tick_and_assert_balances(
    rt: &MockRuntime,
    client_addr: Address,
    provider_addr: Address,
    current_epoch: ChainEpoch,
    deal_id: DealID,
) -> (TokenAmount, TokenAmount) {
    // fetch current client escrow balances
    // NOTE(alexytsu): this code could be factored out and shared it with settle_deal_payments_and_assert_balances
    // except that this path will probably be deleted when https://github.com/filecoin-project/builtin-actors/issues/1389
    // is actioned
    let c_acct = get_balance(rt, &client_addr);
    let p_acct = get_balance(rt, &provider_addr);
    let mut amount_slashed = TokenAmount::zero();

    let s = get_deal_state(rt, deal_id);
    let d = get_deal_proposal(rt, deal_id);

    // end epoch for payment calc
    let mut payment_end = d.end_epoch;
    if s.slash_epoch != EPOCH_UNDEFINED {
        rt.expect_send_simple(
            BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            d.provider_collateral.clone(),
            None,
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
    let updated_client_escrow = c_acct.balance - &payment;
    let updated_provider_escrow = (p_acct.balance + &payment) - &amount_slashed;
    let mut updated_client_locked = c_acct.locked - &payment;
    let mut updated_provider_locked = p_acct.locked;
    // if the deal has expired or been slashed, locked amount will be zero for provider .
    let is_deal_expired = payment_end == d.end_epoch;
    if is_deal_expired || s.slash_epoch != EPOCH_UNDEFINED {
        updated_client_locked = TokenAmount::zero();
        updated_provider_locked = TokenAmount::zero();
    }

    if is_deal_expired {
        expect_emitted(
            rt,
            "deal-completed",
            deal_id,
            client_addr.id().unwrap(),
            provider_addr.id().unwrap(),
        );
    }

    cron_tick(rt);

    let client_acct = get_balance(rt, &client_addr);
    let provider_acct = get_balance(rt, &provider_addr);
    assert_eq!(updated_client_escrow, client_acct.balance);
    assert_eq!(updated_client_locked, client_acct.locked);
    assert_eq!(updated_provider_escrow, provider_acct.balance);
    assert_eq!(updated_provider_locked, provider_acct.locked);
    (payment, amount_slashed)
}

pub fn cron_tick_no_change(rt: &MockRuntime, client_addr: Address, provider_addr: Address) {
    let st: State = rt.get_state();
    let epoch_cid = st.deal_ops_by_epoch;

    // fetch current client  escrow balances
    let client_acct = get_balance(rt, &client_addr);
    let provider_acct = get_balance(rt, &provider_addr);

    cron_tick(rt);

    let st: State = rt.get_state();
    let new_client_acct = get_balance(rt, &client_addr);
    let new_provider_acct = get_balance(rt, &provider_addr);
    assert_eq!(epoch_cid, st.deal_ops_by_epoch);
    assert_eq!(client_acct, new_client_acct);
    assert_eq!(provider_acct, new_provider_acct);
}

pub fn publish_deals(
    rt: &MockRuntime,
    addrs: &MinerAddresses,
    publish_deals: &[DealProposal],
    clients_datacap_balance: TokenAmount,
    next_allocation_id: AllocationID,
) -> Vec<DealID> {
    let st: State = rt.get_state();
    let next_deal_id = st.next_id;
    rt.expect_validate_caller_any();
    let return_value = ext::miner::IsControllingAddressReturn { is_controlling: true };
    rt.expect_send_simple(
        addrs.provider,
        ext::miner::IS_CONTROLLING_ADDRESS_EXPORTED,
        IpldBlock::serialize_cbor(&ext::miner::IsControllingAddressParam {
            address: *rt.caller.borrow(),
        })
        .unwrap(),
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&return_value).unwrap(),
        ExitCode::OK,
    );

    expect_query_network_info(rt);

    let mut params: PublishStorageDealsParams = PublishStorageDealsParams { deals: vec![] };

    // Accumulate proposals by client, so we can set expectations for the per-client calls
    //  per-deal calls. This matches flow in the market actor.
    // Note the shortcut of not normalising the client/provider addresses in the proposal.
    struct ClientVerifiedDeals {
        deals: Vec<DealProposal>,
        datacap_consumed: TokenAmount,
    }
    let mut client_verified_deals: BTreeMap<ActorID, ClientVerifiedDeals> = BTreeMap::new();
    let mut alloc_id = next_allocation_id;
    let mut valid_deals = vec![];
    for deal in publish_deals {
        // create a client proposal with a valid signature
        let buf = RawBytes::serialize(deal.clone()).expect("failed to marshal deal proposal");
        let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
        let client_proposal =
            ClientDealProposal { proposal: deal.clone(), client_signature: sig.clone() };
        params.deals.push(client_proposal);

        // Expect an invocation of authenticate_message to verify the signature.
        rt.expect_send(
            deal.client,
            ext::account::AUTHENTICATE_MESSAGE_METHOD,
            IpldBlock::serialize_cbor(&AuthenticateMessageParams {
                signature: "does not matter".as_bytes().to_vec(),
                message: buf.to_vec(),
            })
            .unwrap(),
            TokenAmount::zero(),
            None,
            SendFlags::READ_ONLY,
            AUTHENTICATE_MESSAGE_RESPONSE.clone(),
            ExitCode::OK,
            None,
        );
    }

    for deal in publish_deals {
        if deal.verified_deal {
            // Expect query for the client's datacap balance, just once per client.
            let client_id = deal.client.id().unwrap();
            if client_verified_deals.get(&client_id).is_none() {
                rt.expect_send_simple(
                    DATACAP_TOKEN_ACTOR_ADDR,
                    ext::datacap::BALANCE_OF_METHOD,
                    IpldBlock::serialize_cbor(&deal.client).unwrap(),
                    TokenAmount::zero(),
                    IpldBlock::serialize_cbor(&clients_datacap_balance).unwrap(),
                    ExitCode::OK,
                );
            }

            let cvd = client_verified_deals.entry(client_id).or_insert(ClientVerifiedDeals {
                deals: vec![],
                datacap_consumed: TokenAmount::zero(),
            });
            let piece_datacap = TokenAmount::from_whole(deal.piece_size.0);
            if piece_datacap > &clients_datacap_balance - &cvd.datacap_consumed {
                continue; // Drop deal
            }
            cvd.deals.push(deal.clone());
            cvd.datacap_consumed += piece_datacap;
        }
        valid_deals.push(deal);
    }

    let curr_epoch = *rt.epoch.borrow();
    let policy = Policy::default();
    for (client, cvd) in client_verified_deals {
        if cvd.deals.is_empty() {
            continue;
        }
        // Expect transfer of data cap to the verified registry, with spec for the allocation.
        let mut allocations = vec![];
        for deal in cvd.deals {
            let term_min = deal.end_epoch - deal.start_epoch;
            let term_max = min(
                term_min + policy.market_default_allocation_term_buffer,
                policy.maximum_verified_allocation_term,
            );
            let expiration =
                min(deal.start_epoch, curr_epoch + policy.maximum_verified_allocation_expiration);
            allocations.push(AllocationRequest {
                provider: deal.provider.id().unwrap(),
                data: deal.piece_cid,
                size: deal.piece_size,
                term_min,
                term_max,
                expiration,
            });
        }

        let alloc_req = ext::verifreg::AllocationRequests { allocations, extensions: vec![] };
        let params = TransferFromParams {
            from: Address::new_id(client),
            to: VERIFIED_REGISTRY_ACTOR_ADDR,
            amount: cvd.datacap_consumed.clone(),
            operator_data: serialize(&alloc_req, "allocation requests").unwrap(),
        };
        let alloc_ids = AllocationsResponse {
            allocation_results: BatchReturn::ok(alloc_req.allocations.len() as u32),
            extension_results: BatchReturn::empty(),
            new_allocations: (alloc_id..alloc_id + alloc_req.allocations.len() as u64).collect(),
        };
        rt.expect_send_simple(
            DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::TRANSFER_FROM_METHOD,
            IpldBlock::serialize_cbor(&params).unwrap(),
            TokenAmount::zero(),
            IpldBlock::serialize_cbor(&TransferFromReturn {
                from_balance: TokenAmount::zero(),
                to_balance: cvd.datacap_consumed,
                allowance: TokenAmount::zero(),
                recipient_data: serialize(&alloc_ids, "allocation response").unwrap(),
            })
            .unwrap(),
            ExitCode::OK,
        );
        alloc_id += alloc_req.allocations.len() as AllocationID;
    }

    let mut deal_id = next_deal_id;
    for deal in valid_deals {
        let buf = RawBytes::serialize(deal.clone()).expect("failed to marshal deal proposal");
        let params =
            IpldBlock::serialize_cbor(&MarketNotifyDealParams { proposal: buf.to_vec(), deal_id })
                .unwrap();
        rt.expect_send_simple(
            deal.client,
            MARKET_NOTIFY_DEAL_METHOD,
            params,
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );
        expect_emitted(
            rt,
            "deal-published",
            deal_id,
            deal.client.id().unwrap(),
            deal.provider.id().unwrap(),
        );
        deal_id += 1;
    }

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    // assert state after publishing the deals
    alloc_id = next_allocation_id;
    for (i, deal_id) in ret.ids.iter().enumerate() {
        let expected = &publish_deals[i];
        let p = get_deal_proposal(rt, *deal_id);
        assert_eq!(expected, &p);
        if p.verified_deal {
            assert_eq!(get_pending_deal_allocation(rt, *deal_id), alloc_id);
            alloc_id += 1;
        }
    }

    ret.ids
}

pub fn publish_deals_expect_abort(
    rt: &MockRuntime,
    miner_addresses: &MinerAddresses,
    proposal: DealProposal,
    expected_exit_code: ExitCode,
) {
    rt.expect_validate_caller_any();
    expect_provider_is_control_address(rt, miner_addresses.provider, WORKER_ADDR, true);

    let deal_serialized =
        RawBytes::serialize(proposal.clone()).expect("Failed to marshal deal proposal");
    let client_signature = Signature::new_bls(deal_serialized.to_vec());

    expect_query_network_info(rt);
    let auth_param = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: deal_serialized.to_vec(),
        message: deal_serialized.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        proposal.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal, client_signature }],
    };
    expect_abort(
        expected_exit_code,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&deal_params).unwrap(),
        ),
    );

    rt.verify();
}

pub fn settle_deal_payments(
    rt: &MockRuntime,
    caller: Address,
    deal_ids: &[DealID],
    completed_deals: &[DealID],
    terminated_deals: &[DealID],
) -> SettleDealPaymentsReturn {
    let mut deal_id_bitfield = BitField::new();
    for deal_id in deal_ids {
        deal_id_bitfield.set(*deal_id);
    }
    let params = SettleDealPaymentsParams { deal_ids: deal_id_bitfield };
    let params = IpldBlock::serialize_cbor(&params).unwrap();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
    rt.expect_validate_caller_any();

    for deal_id in completed_deals {
        let deal = get_deal_proposal(rt, *deal_id);
        expect_emitted(
            rt,
            "deal-completed",
            *deal_id,
            deal.client.id().unwrap(),
            deal.provider.id().unwrap(),
        );
    }

    for deal_id in terminated_deals {
        let deal = get_deal_proposal(rt, *deal_id);
        expect_emitted(
            rt,
            "deal-terminated",
            *deal_id,
            deal.client.id().unwrap(),
            deal.provider.id().unwrap(),
        );
    }

    let res =
        rt.call::<MarketActor>(Method::SettleDealPaymentsExported as u64, params).unwrap().unwrap();
    let res: SettleDealPaymentsReturn = res.deserialize().unwrap();

    rt.verify();
    res
}

pub fn settle_deal_payments_no_change(
    rt: &MockRuntime,
    caller: Address,
    client_addr: Address,
    provider_addr: Address,
    deal_ids: &[DealID],
) {
    let st: State = rt.get_state();
    let epoch_cid = st.deal_ops_by_epoch;

    // fetch current client  escrow balances
    let client_acct = get_balance(rt, &client_addr);
    let provider_acct = get_balance(rt, &provider_addr);

    settle_deal_payments(rt, caller, deal_ids, &[], &[]);

    let st: State = rt.get_state();
    let new_client_acct = get_balance(rt, &client_addr);
    let new_provider_acct = get_balance(rt, &provider_addr);
    assert_eq!(epoch_cid, st.deal_ops_by_epoch);
    assert_eq!(client_acct, new_client_acct);
    assert_eq!(provider_acct, new_provider_acct);
}

pub fn settle_deal_payments_and_assert_balances(
    rt: &MockRuntime,
    client_addr: Address,
    provider_addr: Address,
    current_epoch: ChainEpoch,
    deal_id: DealID,
    already_terminated: bool,
) -> (TokenAmount, TokenAmount) {
    // fetch current client escrow balances
    let c_acct = get_balance(rt, &client_addr);
    let p_acct = get_balance(rt, &provider_addr);
    let mut amount_slashed = TokenAmount::zero();

    let s = get_deal_state(rt, deal_id);
    let d = get_deal_proposal(rt, deal_id);

    // end epoch for payment calc
    let mut payment_end = d.end_epoch;
    if s.slash_epoch != EPOCH_UNDEFINED {
        rt.expect_send_simple(
            BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            d.provider_collateral.clone(),
            None,
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
    let updated_client_escrow = c_acct.balance - &payment;
    let updated_provider_escrow = (p_acct.balance + &payment) - &amount_slashed;
    let mut updated_client_locked = c_acct.locked - &payment;
    let mut updated_provider_locked = p_acct.locked;
    // if the deal has expired or been slashed, locked amount will be zero for provider .
    let is_deal_expired = payment_end == d.end_epoch;
    if is_deal_expired || s.slash_epoch != EPOCH_UNDEFINED {
        updated_client_locked = TokenAmount::zero();
        updated_provider_locked = TokenAmount::zero();
    }

    let mut completed: Vec<DealID> = vec![];
    let mut terminated: Vec<DealID> = vec![];
    if is_deal_expired {
        completed.push(deal_id);
    } else if s.slash_epoch != EPOCH_UNDEFINED && !already_terminated {
        terminated.push(deal_id);
    }

    settle_deal_payments(rt, provider_addr, &[deal_id], &completed, &terminated);

    let client_acct = get_balance(rt, &client_addr);
    let provider_acct = get_balance(rt, &provider_addr);
    assert_eq!(updated_client_escrow, client_acct.balance);
    assert_eq!(updated_client_locked, client_acct.locked);
    assert_eq!(updated_provider_escrow, provider_acct.balance);
    assert_eq!(updated_provider_locked, provider_acct.locked);
    (payment, amount_slashed)
}

pub fn settle_deal_payments_expect_abort(
    rt: &MockRuntime,
    caller: Address,
    deal_ids: &[DealID],
    expected_exit_code: ExitCode,
) {
    let mut deal_id_bitfield = BitField::new();
    for deal_id in deal_ids {
        deal_id_bitfield.set(*deal_id);
    }
    let params = SettleDealPaymentsParams { deal_ids: deal_id_bitfield };
    let params = IpldBlock::serialize_cbor(&params).unwrap();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
    rt.expect_validate_caller_any();
    expect_abort(
        expected_exit_code,
        rt.call::<MarketActor>(Method::SettleDealPaymentsExported as u64, params),
    );

    rt.verify();
}

pub fn assert_deals_not_activated(rt: &MockRuntime, _epoch: ChainEpoch, deal_ids: &[DealID]) {
    let st: State = rt.get_state();

    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();

    for d in deal_ids {
        let opt = states.get(*d).unwrap();
        assert!(opt.is_none());
    }
}

pub fn cron_tick(rt: &MockRuntime) {
    assert!(cron_tick_raw(rt).unwrap().is_none());
    rt.verify()
}

pub fn cron_tick_raw(rt: &MockRuntime) -> Result<Option<IpldBlock>, ActorError> {
    rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);
    rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);

    rt.call::<MarketActor>(Method::CronTick as u64, None)
}

pub fn expect_query_network_info(rt: &MockRuntime) {
    //networkQAPower
    //networkBaselinePower
    let reward = TokenAmount::from_whole(10);
    let power = StoragePower::from_i128(1 << 50).unwrap();
    let epoch_reward_smooth = FilterEstimate::new(reward.atto().clone(), BigInt::from(0u8));

    let current_power = CurrentTotalPowerReturn {
        raw_byte_power: StoragePower::default(),
        quality_adj_power: power.clone(),
        pledge_collateral: TokenAmount::default(),
        quality_adj_power_smoothed: FilterEstimate::new(reward.atto().clone(), BigInt::zero()),
    };
    let current_reward = ThisEpochRewardReturn {
        this_epoch_baseline_power: power,
        this_epoch_reward_smoothed: epoch_reward_smooth,
    };
    rt.expect_send_simple(
        REWARD_ACTOR_ADDR,
        RewardMethod::ThisEpochReward as u64,
        None,
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&current_reward).unwrap(),
        ExitCode::OK,
    );
    rt.expect_send_simple(
        STORAGE_POWER_ACTOR_ADDR,
        PowerMethod::CurrentTotalPower as u64,
        None,
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&current_power).unwrap(),
        ExitCode::OK,
    );
}

pub fn assert_n_good_deals<BS>(
    dobe: &DealOpsByEpoch<BS>,
    updates_interval: ChainEpoch,
    epoch: ChainEpoch,
    n: isize,
) where
    BS: fvm_ipld_blockstore::Blockstore,
{
    let mut count = 0;
    dobe.for_each_in(&epoch, |id| {
        assert_eq!(epoch % updates_interval, (id as i64) % updates_interval);
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(n, count, "unexpected deal count at epoch {}", epoch);
}

pub fn assert_deals_not_marked_terminated(rt: &MockRuntime, deal_ids: &[DealID]) {
    for &deal_id in deal_ids {
        let s = get_deal_state(rt, deal_id);
        assert_eq!(s.slash_epoch, EPOCH_UNDEFINED);
    }
}

pub fn assert_deal_deleted(
    rt: &MockRuntime,
    deal_id: DealID,
    p: &DealProposal,
    sector_number: SectorNumber,
) {
    use cid::multihash::Code;
    use cid::multihash::MultihashDigest;

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
    let p_cid = Cid::new_v1(fvm_ipld_encoding::DAG_CBOR, mh_code.digest(&to_vec(p).unwrap()));
    // Check that the deal_id is not in st.pending_proposals.
    let pending_deals = PendingProposalsSet::load(
        rt.store(),
        &st.pending_proposals,
        PENDING_PROPOSALS_CONFIG,
        "pending proposals",
    )
    .unwrap();
    assert!(!pending_deals.has(&p_cid).unwrap());

    // Check deal is no longer associated with sector
    let sector_deals = get_sector_deal_ids(rt, p.provider.id().unwrap(), &[sector_number]);
    assert!(!sector_deals.contains(&deal_id));
}

pub fn assert_deal_failure<F>(add_funds: bool, post_setup: F, exit_code: ExitCode, sig_valid: bool)
where
    F: FnOnce(&MockRuntime, &mut DealProposal),
{
    let current_epoch = ChainEpoch::from(5);
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let rt = setup();
    let mut deal_proposal = if add_funds {
        generate_deal_and_add_funds(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        )
    } else {
        generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, start_epoch, end_epoch)
    };
    rt.set_epoch(current_epoch);
    post_setup(&rt, &mut deal_proposal);

    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    let buf = RawBytes::serialize(deal_proposal.clone()).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls(buf.to_vec());
    let auth_param = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal_proposal.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        match sig_valid {
            true => AUTHENTICATE_MESSAGE_RESPONSE.clone(),
            false => None,
        },
        match sig_valid {
            true => ExitCode::OK,
            false => ExitCode::USR_ILLEGAL_ARGUMENT,
        },
        None,
    );

    let params: PublishStorageDealsParams = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: deal_proposal, client_signature: sig }],
    };

    assert_eq!(
        exit_code,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap_err()
        .exit_code()
    );
    rt.verify();
    check_state(&rt);
}

pub fn process_epoch(start_epoch: ChainEpoch, deal_id: DealID) -> ChainEpoch {
    next_update_epoch(deal_id, Policy::default().deal_updates_interval, start_epoch)
}

#[allow(clippy::too_many_arguments)]
pub fn publish_and_activate_deal(
    rt: &MockRuntime,
    client: Address,
    addrs: &MinerAddresses,
    sector_number: SectorNumber,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    current_epoch: ChainEpoch,
    sector_expiry: ChainEpoch,
) -> (DealID, DealProposal) {
    let deal = generate_deal_and_add_funds(rt, client, addrs, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.worker);
    let deal_ids = publish_deals(rt, addrs, &[deal.clone()], TokenAmount::zero(), NO_ALLOCATION_ID); // unverified deal
    activate_deals(rt, sector_expiry, addrs.provider, current_epoch, sector_number, &deal_ids);
    (deal_ids[0], deal)
}

#[allow(clippy::too_many_arguments)]
pub fn publish_and_activate_deal_legacy(
    rt: &MockRuntime,
    client: Address,
    addrs: &MinerAddresses,
    sector_number: SectorNumber,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    current_epoch: ChainEpoch,
    sector_expiry: ChainEpoch,
) -> (DealID, DealProposal) {
    let (deal_id, proposal) = publish_and_activate_deal(
        rt,
        client,
        addrs,
        sector_number,
        start_epoch,
        end_epoch,
        current_epoch,
        sector_expiry,
    );
    simulate_legacy_deal(rt, deal_id, start_epoch);
    (deal_id, proposal)
}

pub fn generate_and_publish_deal(
    rt: &MockRuntime,
    client: Address,
    addrs: &MinerAddresses,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> (DealID, DealProposal) {
    let deal = generate_deal_and_add_funds(rt, client, addrs, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.worker);
    let deal_ids = publish_deals(rt, addrs, &[deal.clone()], TokenAmount::zero(), NO_ALLOCATION_ID); // unverified deal
    (deal_ids[0], deal)
}

pub fn generate_and_publish_verified_deal(
    rt: &MockRuntime,
    client: Address,
    addrs: &MinerAddresses,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    next_allocation_id: AllocationID,
) -> DealID {
    let mut deal = generate_deal_and_add_funds(rt, client, addrs, start_epoch, end_epoch);
    deal.verified_deal = true;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.worker);
    let deal_ids = publish_deals(
        rt,
        addrs,
        &[deal.clone()],
        TokenAmount::from_whole(deal.piece_size.0),
        next_allocation_id,
    );
    deal_ids[0]
}

pub fn generate_and_publish_deal_for_piece(
    rt: &MockRuntime,
    client: Address,
    addrs: &MinerAddresses,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    piece_cid: Cid,
    piece_size: PaddedPieceSize,
) -> DealID {
    // generate deal
    let storage_price_per_epoch = TokenAmount::from_atto(10u8);
    let client_collateral = TokenAmount::from_atto(10u8);
    let provider_collateral = TokenAmount::from_atto(10u8);

    let deal = DealProposal {
        piece_cid,
        piece_size,
        verified_deal: false,
        client,
        provider: addrs.provider,
        label: Label::String("label".to_string()),
        start_epoch,
        end_epoch,
        storage_price_per_epoch,
        provider_collateral,
        client_collateral,
    };

    // add funds
    add_provider_funds(rt, deal.provider_collateral.clone(), addrs);
    add_participant_funds(rt, client, deal.client_balance_requirement());

    // publish
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.worker);
    let deal_ids = publish_deals(rt, addrs, &[deal], TokenAmount::zero(), NO_ALLOCATION_ID); // unverified deal
    deal_ids[0]
}

pub fn generate_deal_and_add_funds(
    rt: &MockRuntime,
    client: Address,
    addrs: &MinerAddresses,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let deal = generate_deal_proposal(client, addrs.provider, start_epoch, end_epoch);
    add_provider_funds(rt, deal.provider_collateral.clone(), addrs);
    add_participant_funds(rt, client, deal.client_balance_requirement());
    deal
}

pub fn generate_deal_with_collateral_and_add_funds(
    rt: &MockRuntime,
    client: Address,
    addrs: &MinerAddresses,
    provider_collateral: TokenAmount,
    client_collateral: TokenAmount,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let deal = generate_deal_proposal_with_collateral(
        client,
        addrs.provider,
        client_collateral,
        provider_collateral,
        start_epoch,
        end_epoch,
    );
    add_provider_funds(rt, deal.provider_collateral.clone(), addrs);
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
    let storage_price_per_epoch = TokenAmount::from_atto(10u8);
    DealProposal {
        piece_cid,
        piece_size,
        verified_deal: false,
        client,
        provider,
        label: Label::String("label".to_string()),
        start_epoch,
        end_epoch,
        storage_price_per_epoch,
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
    let client_collateral = TokenAmount::from_atto(10u8);
    let provider_collateral = TokenAmount::from_atto(10u8);
    generate_deal_proposal_with_collateral(
        client,
        provider,
        client_collateral,
        provider_collateral,
        start_epoch,
        end_epoch,
    )
}

/// NOTE: assumes all deals are made between the same client
pub fn terminate_deals_and_assert_balances(
    rt: &MockRuntime,
    client_addr: Address,
    provider_addr: Address,
    sectors: &[SectorNumber],
    expected_terminations: &[DealID],
) -> (/*total_paid*/ TokenAmount, /*total_slashed*/ TokenAmount) {
    // Accumulate deal IDs for all sectors
    let deal_ids = get_sector_deal_ids(rt, provider_addr.id().unwrap(), sectors);
    // get deal state  before the are cleaned up in terminate deals
    let deal_infos: Vec<(DealState, DealProposal)> = deal_ids
        .iter()
        .filter_map(|id| {
            let proposal = find_deal_proposal(rt, *id);
            proposal.map(|proposal| {
                let state = get_deal_state(rt, *id);
                (state, proposal)
            })
        })
        .collect();

    // outstanding payment to be made
    let mut total_payment = TokenAmount::zero();
    // provider penalty
    let mut total_slashed = TokenAmount::zero();
    // payment to be refunded
    let mut payment_remaining = TokenAmount::zero();
    let mut client_unlocked = TokenAmount::zero();

    let curr_epoch = *rt.epoch.borrow();
    for (s, d) in &deal_infos {
        // terminate is a no-op if deal is already expired/expiring
        if curr_epoch < d.end_epoch {
            let mut payment_start = d.start_epoch;
            if s.last_updated_epoch != EPOCH_UNDEFINED {
                payment_start = max(s.last_updated_epoch, d.start_epoch);
            }
            let duration = max(0, curr_epoch - payment_start);
            let payment = duration * &d.storage_price_per_epoch;
            total_payment += payment;
            payment_remaining += deal_get_payment_remaining(d, curr_epoch).unwrap();
            client_unlocked += &d.client_collateral;
            total_slashed += &d.provider_collateral;
        }
    }

    let client_before = get_balance(rt, &client_addr);
    let provider_before = get_balance(rt, &provider_addr);

    // expected updated amounts
    let updated_client_escrow = &client_before.balance - &total_payment;
    let updated_provider_escrow = &provider_before.balance + &total_payment - &total_slashed;
    let updated_client_locked =
        &client_before.locked - &total_payment - &payment_remaining - &client_unlocked;
    let updated_provider_locked = &provider_before.locked - &total_slashed;

    terminate_deals(rt, provider_addr, sectors, expected_terminations);

    let client_acct = get_balance(rt, &client_addr);
    let provider_acct = get_balance(rt, &provider_addr);
    assert_eq!(&updated_client_escrow, &client_acct.balance);
    assert_eq!(&updated_client_locked, &client_acct.locked);
    assert_eq!(updated_provider_escrow, provider_acct.balance);
    assert_eq!(updated_provider_locked, provider_acct.locked);

    (total_payment, total_slashed)
}

pub fn terminate_deals_no_change(
    rt: &MockRuntime,
    client_addr: Address,
    provider_addr: Address,
    sectors: &[SectorNumber],
    expected_terminations: &[DealID],
) {
    let st: State = rt.get_state();
    let epoch_cid = st.deal_ops_by_epoch;

    // fetch current client  escrow balances
    let client_acct = get_balance(rt, &client_addr);
    let provider_acct = get_balance(rt, &provider_addr);

    terminate_deals(rt, provider_addr, sectors, expected_terminations);

    let st: State = rt.get_state();
    let new_client_acct = get_balance(rt, &client_addr);
    let new_provider_acct = get_balance(rt, &provider_addr);
    assert_eq!(epoch_cid, st.deal_ops_by_epoch);
    assert_eq!(client_acct, new_client_acct);
    assert_eq!(provider_acct, new_provider_acct);
}

pub fn terminate_deals(
    rt: &MockRuntime,
    miner_addr: Address,
    sectors: &[SectorNumber],
    expected_terminations: &[DealID],
) {
    let deal_ids = get_sector_deal_ids(rt, miner_addr.id().unwrap(), sectors);
    // calculate the expected amount to be slashed for the provider that it is burnt
    let curr_epoch = *rt.epoch.borrow();
    let mut total_slashed = TokenAmount::zero();
    for deal_id in deal_ids {
        let d = find_deal_proposal(rt, deal_id);
        if let Some(d) = d {
            if curr_epoch < d.end_epoch {
                total_slashed += d.provider_collateral.clone();
            }
        }
    }

    if total_slashed.is_positive() {
        rt.expect_send_simple(
            BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            total_slashed,
            None,
            ExitCode::OK,
        );
    }

    let ret = terminate_deals_raw(rt, miner_addr, sectors, expected_terminations).unwrap();
    assert!(ret.is_none());
    rt.verify();
}

pub fn terminate_deals_expect_abort(
    rt: &MockRuntime,
    miner_addr: Address,
    sectors: &[SectorNumber],
    expected_exit_code: ExitCode,
) {
    rt.set_caller(*MINER_ACTOR_CODE_ID, miner_addr);
    rt.expect_validate_caller_type(vec![Type::Miner]);
    let params = OnMinerSectorsTerminateParams {
        epoch: *rt.epoch.borrow(),
        sectors: BitField::try_from_bits(sectors.iter().copied()).unwrap(),
    };

    expect_abort(
        expected_exit_code,
        rt.call::<MarketActor>(
            Method::OnMinerSectorsTerminate as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
}

pub fn terminate_deals_raw(
    rt: &MockRuntime,
    miner_addr: Address,
    sector_numbers: &[SectorNumber],
    terminated_deals: &[DealID],
) -> Result<Option<IpldBlock>, ActorError> {
    rt.set_caller(*MINER_ACTOR_CODE_ID, miner_addr);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let bf = BitField::try_from_bits(sector_numbers.iter().copied()).unwrap();
    let params = OnMinerSectorsTerminateParams { epoch: *rt.epoch.borrow(), sectors: bf };

    for deal_id in terminated_deals {
        let d = get_deal_proposal(rt, *deal_id);
        expect_emitted(
            rt,
            "deal-terminated",
            *deal_id,
            d.client.id().unwrap(),
            d.provider.id().unwrap(),
        )
    }

    rt.call::<MarketActor>(
        Method::OnMinerSectorsTerminate as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    )
}

pub fn assert_account_zero(rt: &MockRuntime, addr: Address) {
    let account = get_balance(rt, &addr);
    assert!(account.balance.is_zero());
    assert!(account.locked.is_zero());
}

pub fn verify_deals_for_activation<F>(
    rt: &MockRuntime,
    provider: Address,
    sector_deals: Vec<SectorDeals>,
    piece_info_override: F,
) -> VerifyDealsForActivationReturn
where
    F: Fn(usize) -> Option<Vec<fvm_shared::piece::PieceInfo>>,
{
    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider);

    for (i, sd) in sector_deals.iter().enumerate() {
        let pi = piece_info_override(i).unwrap_or_else(|| {
            vec![fvm_shared::piece::PieceInfo {
                cid: make_piece_cid("1".as_bytes()),
                size: PaddedPieceSize(2048),
            }]
        });
        rt.expect_compute_unsealed_sector_cid(
            sd.sector_type,
            pi,
            make_piece_cid("1".as_bytes()),
            ExitCode::OK,
        )
    }

    let param = VerifyDealsForActivationParams { sectors: sector_deals };
    let ret: VerifyDealsForActivationReturn = rt
        .call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            IpldBlock::serialize_cbor(&param).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .expect("VerifyDealsForActivation failed!");
    rt.verify();
    ret
}

// market cron tick uses last_updated_epoch == EPOCH_UNDEFINED to determine if a deal is new
// it will not process such deals
// however, for testing we need to simulate deals that are already in the system that should be
// continued to be processed by cron
fn simulate_legacy_deal(
    rt: &fil_actors_runtime::test_utils::MockRuntime,
    deal_id: u64,
    start_epoch: i64,
) {
    let mut state = rt.get_state::<State>();
    let mut deal_state = state.remove_deal_state(rt.store(), deal_id).unwrap().unwrap();

    // set last_updated_epoch to the beginning of the deal (if cron had run here, it would have been a no-op)
    deal_state.last_updated_epoch = start_epoch;
    state.put_deal_states(rt.store(), &[(deal_id, deal_state)]).unwrap();

    // the first cron_tick would have removed the proposal from the pending queue
    let proposal = state.find_proposal(rt.store(), deal_id).unwrap().unwrap();
    state.remove_pending_deal(rt.store(), deal_cid(rt, &proposal).unwrap()).unwrap();

    rt.replace_state(&state);
}

pub fn piece_info_from_deal(id: DealID, deal: &DealProposal) -> PieceChange {
    PieceChange {
        data: deal.piece_cid,
        size: deal.piece_size,
        payload: serialize(&id, "deal id").unwrap(),
    }
}

pub fn expect_emitted(rt: &MockRuntime, typ: &str, id: DealID, client: ActorID, provider: ActorID) {
    rt.expect_emitted_event(
        EventBuilder::new()
            .typ(typ)
            .field_indexed("id", &id)
            .field_indexed("client", &client)
            .field_indexed("provider", &provider)
            .build()
            .unwrap(),
    );
}

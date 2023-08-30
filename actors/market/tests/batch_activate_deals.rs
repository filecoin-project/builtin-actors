use fil_actor_market::{
    BatchActivateDealsResult, DealMetaArray, SectorDeals, State, NO_ALLOCATION_ID,
};
use fil_actors_runtime::test_utils::ACCOUNT_ACTOR_CODE_ID;
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredSealProof;
use num_traits::Zero;

mod harness;
use harness::*;

const START_EPOCH: ChainEpoch = 10;
const END_EPOCH: ChainEpoch = 200 * EPOCHS_IN_DAY;
const MINER_ADDRESSES: MinerAddresses = MinerAddresses {
    owner: OWNER_ADDR,
    worker: WORKER_ADDR,
    provider: PROVIDER_ADDR,
    control: vec![],
};

#[test]
fn activate_deals_across_multiple_sectors() {
    let rt = setup();
    let create_deal = |end_epoch, verified| {
        create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, end_epoch, verified)
    };
    let verified_deal_1 = create_deal(END_EPOCH, true);
    let unverified_deal_1 = create_deal(END_EPOCH, false);
    let verified_deal_2 = create_deal(END_EPOCH + 1, true);
    let unverified_deal_2 = create_deal(END_EPOCH + 2, false);

    let deals =
        [verified_deal_1.clone(), unverified_deal_1, verified_deal_2.clone(), unverified_deal_2];

    let next_allocation_id = 1;
    let datacap_required =
        TokenAmount::from_whole(verified_deal_1.piece_size.0 + verified_deal_2.piece_size.0);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, datacap_required, next_allocation_id);
    assert_eq!(4, deal_ids.len());

    let verified_deal_1_id = deal_ids[0];
    let unverified_deal_1_id = deal_ids[1];
    let verified_deal_2_id = deal_ids[2];
    let unverified_deal_2_id = deal_ids[3];

    // group into sectors
    let sectors = [
        (END_EPOCH, vec![verified_deal_1_id, unverified_deal_1_id]), // contains both verified and unverified deals
        (END_EPOCH + 1, vec![verified_deal_2_id]),                   // contains verified deal only
        (END_EPOCH + 2, vec![unverified_deal_2_id]), // contains unverified deal only
    ];

    let res = batch_activate_deals(&rt, PROVIDER_ADDR, &sectors, false);

    // three sectors activated successfully
    assert!(res.activation_results.all_ok());
    assert_eq!(vec![ExitCode::OK, ExitCode::OK, ExitCode::OK], res.activation_results.codes());

    // all four deals were activated
    let verified_deal_1 = get_deal_state(&rt, verified_deal_1_id);
    let unverified_deal_1 = get_deal_state(&rt, unverified_deal_1_id);
    let verified_deal_2 = get_deal_state(&rt, verified_deal_2_id);
    let unverified_deal_2 = get_deal_state(&rt, unverified_deal_2_id);

    // allocations were claimed successfully
    assert_eq!(next_allocation_id, verified_deal_1.verified_claim);
    assert_eq!(next_allocation_id + 1, verified_deal_2.verified_claim);
    assert_eq!(NO_ALLOCATION_ID, unverified_deal_1.verified_claim);
    assert_eq!(NO_ALLOCATION_ID, unverified_deal_2.verified_claim);

    // all activated during same epoch
    assert_eq!(0, verified_deal_1.sector_start_epoch);
    assert_eq!(0, verified_deal_2.sector_start_epoch);
    assert_eq!(0, unverified_deal_1.sector_start_epoch);
    assert_eq!(0, unverified_deal_2.sector_start_epoch);

    check_state(&rt);
}

#[test]
fn sectors_fail_and_succeed_independently_during_batch_activation() {
    let rt = setup();
    let deal_1 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH, false);
    let deal_2 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH, true);
    let deal_3 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH + 1, false);
    let deal_4 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH + 2, false);

    let deals = [deal_1, deal_2.clone(), deal_3, deal_4];

    let next_allocation_id = 1;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(
        &rt,
        &MINER_ADDRESSES,
        &deals,
        TokenAmount::from_whole(deal_2.piece_size.0),
        next_allocation_id,
    );
    assert_eq!(4, deal_ids.len());

    let id_1 = deal_ids[0];
    let id_2 = deal_ids[1];
    let id_3 = deal_ids[2];
    let id_4 = deal_ids[3];

    // activate the first deal so it will fail later
    activate_deals(&rt, END_EPOCH, PROVIDER_ADDR, 0, &[id_1]);
    // activate the third deal so it will fail later
    activate_deals(&rt, END_EPOCH + 1, PROVIDER_ADDR, 0, &[id_3]);

    let sector_type = RegisteredSealProof::StackedDRG8MiBV1;
    // group into sectors
    let sectors_deals = vec![
        SectorDeals { deal_ids: vec![id_1, id_2], sector_type, sector_expiry: END_EPOCH }, // 1 bad deal causes whole sector to fail
        SectorDeals { deal_ids: vec![id_3], sector_type, sector_expiry: END_EPOCH + 1 }, // bad deal causes whole sector to fail
        SectorDeals { deal_ids: vec![id_4], sector_type, sector_expiry: END_EPOCH + 2 }, // sector succeeds
    ];

    let res = batch_activate_deals_raw(&rt, PROVIDER_ADDR, sectors_deals, false).unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    // first two sectors should fail
    assert_eq!(1, res.activation_results.success_count);
    assert_eq!(
        vec![ExitCode::USR_ILLEGAL_ARGUMENT, ExitCode::USR_ILLEGAL_ARGUMENT, ExitCode::OK],
        res.activation_results.codes()
    );

    // originally activated deals should still be active
    let deal_1 = get_deal_state(&rt, id_1);
    assert_eq!(0, deal_1.sector_start_epoch);
    let deal_3 = get_deal_state(&rt, id_3);
    assert_eq!(0, deal_3.sector_start_epoch);

    // newly activated deal should be active
    let deal_4 = get_deal_state(&rt, id_4);
    assert_eq!(0, deal_4.sector_start_epoch);

    // no state for deal2 means deal2 was not activated
    let st: State = rt.get_state();
    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = states.get(id_2).unwrap();
    assert!(s.is_none());

    check_state(&rt);
}

#[test]
fn handles_sectors_empty_of_deals_gracefully() {
    let rt = setup();
    let deal_1 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH, false);

    let next_allocation_id = 1;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &[deal_1], TokenAmount::zero(), next_allocation_id);
    assert_eq!(1, deal_ids.len());

    let id_1 = deal_ids[0];

    let sector_type = RegisteredSealProof::StackedDRG8MiBV1;
    // group into sectors
    let sectors_deals = vec![
        SectorDeals { deal_ids: vec![], sector_type, sector_expiry: END_EPOCH }, // empty sector
        SectorDeals { deal_ids: vec![id_1], sector_type, sector_expiry: END_EPOCH + 1 }, // sector with one valid deal
        SectorDeals { deal_ids: vec![], sector_type, sector_expiry: END_EPOCH + 2 }, // empty sector
    ];

    let res = batch_activate_deals_raw(&rt, PROVIDER_ADDR, sectors_deals, false).unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    // all sectors should succeed
    assert!(res.activation_results.all_ok());
    // should treat empty sectors as success
    assert_eq!(3, res.activation_results.success_count);

    // deal should have activated
    let deal_1 = get_deal_state(&rt, id_1);
    assert_eq!(0, deal_1.sector_start_epoch);

    check_state(&rt);
}

#[test]
fn fails_to_activate_sectors_containing_duplicate_deals() {
    let rt = setup();
    let deal_1 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH, false);
    let deal_2 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH + 1, END_EPOCH, false);
    let deal_3 = create_deal(&rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH + 2, END_EPOCH, false);

    let next_allocation_id = 1;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(
        &rt,
        &MINER_ADDRESSES,
        &[deal_1, deal_2, deal_3],
        TokenAmount::zero(),
        next_allocation_id,
    );
    assert_eq!(3, deal_ids.len());

    let id_1 = deal_ids[0];
    let id_2 = deal_ids[1];
    let id_3 = deal_ids[2];

    let sector_type = RegisteredSealProof::StackedDRG8MiBV1;
    // group into sectors
    let sectors_deals = vec![
        // activate deal 1
        SectorDeals { deal_ids: vec![id_1], sector_type, sector_expiry: END_EPOCH },
        // duplicate id_1 so no deals activated here
        SectorDeals { deal_ids: vec![id_3, id_1, id_2], sector_type, sector_expiry: END_EPOCH }, // duplicate with sector 1 so all fail
        // since id_3 wasn't activated earlier this is a valid request
        SectorDeals { deal_ids: vec![id_3], sector_type, sector_expiry: END_EPOCH },
    ];

    let res = batch_activate_deals_raw(&rt, PROVIDER_ADDR, sectors_deals, false).unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    // all sectors should succeed
    assert_eq!(
        vec![ExitCode::OK, ExitCode::USR_ILLEGAL_ARGUMENT, ExitCode::OK],
        res.activation_results.codes()
    );
    // should treat empty sectors as success
    assert_eq!(2, res.activation_results.success_count);

    // deal should have activated
    let deal_1 = get_deal_state(&rt, id_1);
    assert_eq!(0, deal_1.sector_start_epoch);

    let deal_3 = get_deal_state(&rt, id_3);
    assert_eq!(0, deal_3.sector_start_epoch);

    // no state for deal2 means deal2 was not activated
    let st: State = rt.get_state();
    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = states.get(id_2).unwrap();
    assert!(s.is_none());

    check_state(&rt);
}

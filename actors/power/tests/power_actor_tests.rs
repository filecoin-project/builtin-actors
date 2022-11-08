use fil_actor_power::ext::init::{ExecParams, EXEC_METHOD};
use fil_actor_power::ext::miner::MinerConstructorParams;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::test_utils::{
    expect_abort, expect_abort_contains_message, make_identity_cid, ACCOUNT_ACTOR_CODE_ID,
    MINER_ACTOR_CODE_ID, SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::{runtime::Policy, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredPoStProof, StoragePower};
use fvm_shared::MethodNum;
use num_traits::Zero;
use std::ops::Neg;

use fil_actor_power::{
    consensus_miner_min_power, Actor as PowerActor, Actor, CreateMinerParams, CreateMinerReturn,
    EnrollCronEventParams, Method, MinerRawPowerParams, MinerRawPowerReturn, NetworkRawPowerReturn,
    State, UpdateClaimedPowerParams, CONSENSUS_MINER_MIN_MINERS,
};
use fil_actors_runtime::cbor::serialize;

use crate::harness::*;

mod harness;

#[test]
fn construct() {
    let mut rt = new_runtime();
    let h = new_harness();
    h.construct_and_verify(&mut rt);
    h.check_state(&rt);
}

#[test]
fn create_miner() {
    let (h, mut rt) = setup();

    let peer = "miner".as_bytes().to_vec();
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];

    h.create_miner(
        &mut rt,
        &OWNER,
        &OWNER,
        &MINER,
        &ACTOR,
        peer,
        multiaddrs,
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
        &TokenAmount::from_atto(10),
    )
    .unwrap();

    let st: State = rt.get_state();
    // Verify the miner's claim.
    let claim = h.get_claim(&rt, &MINER).unwrap();
    assert_eq!(RegisteredPoStProof::StackedDRGWindow32GiBV1, claim.window_post_proof_type);
    assert_eq!(StoragePower::zero(), claim.raw_byte_power);
    assert_eq!(StoragePower::zero(), claim.quality_adj_power);

    // Verify aggregates.
    let miners = h.list_miners(&rt);
    assert_eq!(1, miners.len());
    assert_eq!(1, st.miner_count);
    assert_eq!(StoragePower::zero(), st.total_quality_adj_power);
    assert_eq!(StoragePower::zero(), st.total_raw_byte_power);
    assert_eq!(StoragePower::zero(), st.total_bytes_committed);
    assert_eq!(StoragePower::zero(), st.total_qa_bytes_committed);
    assert_eq!(TokenAmount::zero(), st.total_pledge_collateral);
    assert_eq!(0, st.miner_above_min_power_count);

    verify_empty_map(&rt, st.cron_event_queue);
    h.check_state(&rt);
}

#[test]
fn create_miner_given_send_to_init_actor_fails_should_fail() {
    let (h, mut rt) = setup();

    let peer = "miner".as_bytes().to_vec();
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];

    let create_miner_params = CreateMinerParams {
        owner: *OWNER,
        worker: *OWNER,
        window_post_proof_type: RegisteredPoStProof::StackedDRGWindow32GiBV1,
        peer: peer.clone(),
        multiaddrs: multiaddrs.clone(),
    };

    // owner send CreateMiner to Actor
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *OWNER);
    rt.value_received = TokenAmount::from_atto(10);
    rt.set_balance(TokenAmount::from_atto(10));
    rt.expect_validate_caller_any();

    let message_params = ExecParams {
        code_cid: *MINER_ACTOR_CODE_ID,
        constructor_params: RawBytes::serialize(MinerConstructorParams {
            owner: *OWNER,
            worker: *OWNER,
            window_post_proof_type: RegisteredPoStProof::StackedDRGWindow32GiBV1,
            peer_id: peer,
            multi_addresses: multiaddrs,
            control_addresses: Default::default(),
        })
        .unwrap(),
    };

    rt.expect_send(
        INIT_ACTOR_ADDR,
        EXEC_METHOD,
        RawBytes::serialize(message_params).unwrap(),
        TokenAmount::from_atto(10),
        RawBytes::default(),
        ExitCode::SYS_INSUFFICIENT_FUNDS,
    );

    expect_abort(
        ExitCode::SYS_INSUFFICIENT_FUNDS,
        rt.call::<PowerActor>(
            Method::CreateMiner as u64,
            &RawBytes::serialize(&create_miner_params).unwrap(),
        ),
    );
    rt.verify();
    h.check_state(&rt);
}

#[test]
fn claimed_power_given_caller_is_not_storage_miner_should_fail() {
    let (h, mut rt) = setup();

    let params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::from(100),
        quality_adjusted_delta: StoragePower::from(200),
    };

    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<PowerActor>(
            Method::UpdateClaimedPower as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );

    rt.verify();
    h.check_state(&rt);
}

#[test]
fn claimed_power_given_claim_does_not_exist_should_fail() {
    let (h, mut rt) = setup();

    let params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::from(100),
        quality_adjusted_delta: StoragePower::from(200),
    };

    rt.set_caller(*MINER_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    expect_abort(
        ExitCode::USR_NOT_FOUND,
        rt.call::<PowerActor>(
            Method::UpdateClaimedPower as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );

    rt.verify();
    h.check_state(&rt);
}

const MINER1: Address = Address::new_id(111);
const MINER2: Address = Address::new_id(112);
const MINER3: Address = Address::new_id(113);
const MINER4: Address = Address::new_id(114);
const MINER5: Address = Address::new_id(115);

#[test]
fn power_and_pledge_accounted_below_threshold() {
    assert_eq!(CONSENSUS_MINER_MIN_MINERS, 4);

    let small_power_unit = &StoragePower::from(1_000_000);
    let small_power_unit_x2 = &(small_power_unit * 2);
    let small_power_unit_x3 = &(small_power_unit * 3);

    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER2).unwrap();

    let ret = h.current_power_total(&mut rt);
    assert_eq!(StoragePower::zero(), ret.raw_byte_power);
    assert_eq!(StoragePower::zero(), ret.quality_adj_power);
    assert_eq!(TokenAmount::zero(), ret.pledge_collateral);

    // Add power for miner1
    h.update_claimed_power(&mut rt, MINER1, small_power_unit, small_power_unit_x2);
    h.expect_total_power_eager(&mut rt, small_power_unit, small_power_unit_x2);

    // Add power and pledge for miner2
    h.update_claimed_power(&mut rt, MINER2, small_power_unit, small_power_unit);
    h.update_pledge_total(&mut rt, MINER1, &TokenAmount::from_atto(1_000_000));
    h.expect_total_power_eager(&mut rt, small_power_unit_x2, small_power_unit_x3);
    h.expect_total_pledge_eager(&mut rt, &TokenAmount::from_atto(1_000_000));

    rt.verify();

    // Verify claims in state.
    let claim1 = h.get_claim(&rt, &MINER1).unwrap();
    assert_eq!(small_power_unit, &claim1.raw_byte_power);
    assert_eq!(small_power_unit_x2, &claim1.quality_adj_power);

    let claim2 = h.get_claim(&rt, &MINER2).unwrap();
    assert_eq!(small_power_unit, &claim2.raw_byte_power);
    assert_eq!(small_power_unit, &claim2.quality_adj_power);

    // Subtract power and some pledge for miner2
    h.update_claimed_power(&mut rt, MINER2, &small_power_unit.neg(), &small_power_unit.neg());
    h.update_pledge_total(&mut rt, MINER2, &TokenAmount::from_atto(100_000).neg());
    h.expect_total_power_eager(&mut rt, small_power_unit, small_power_unit_x2);
    h.expect_total_pledge_eager(&mut rt, &TokenAmount::from_atto(900_000));

    let claim2 = h.get_claim(&rt, &MINER2).unwrap();
    assert!(claim2.raw_byte_power.is_zero());
    assert!(claim2.quality_adj_power.is_zero());
    h.check_state(&rt);
}

#[test]
fn enroll_cron_epoch_multiple_events() {
    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, *MINER).unwrap();
    let miner2_address = Address::new_id(501);
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, miner2_address).unwrap();

    let mut enroll_and_check_cron_event = |epoch, miner_address, payload| {
        let pre_existing_event_count = h.get_enrolled_cron_ticks(&rt, epoch).len();

        h.enroll_cron_event(&mut rt, epoch, miner_address, payload).unwrap();

        let events = h.get_enrolled_cron_ticks(&rt, epoch);
        assert_eq!(events.len(), pre_existing_event_count + 1);
        assert_eq!(&events.last().unwrap().callback_payload, payload);
        assert_eq!(&events.last().unwrap().miner_addr, miner_address);
    };

    // enroll event with miner 1
    let payload = RawBytes::serialize(b"Cthulhu").unwrap();
    enroll_and_check_cron_event(1, &MINER, &payload);

    // enroll another event with the same miner
    let payload = RawBytes::serialize(b"Nyarlathotep").unwrap();
    enroll_and_check_cron_event(1, &MINER, &payload);

    // enroll another event with a different miner for a different epoch
    let payload = RawBytes::serialize(b"Azathoth").unwrap();
    enroll_and_check_cron_event(2, &miner2_address, &payload);

    h.check_state(&rt);
}

#[test]
fn enroll_cron_epoch_before_current_epoch() {
    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, *MINER).unwrap();

    let current_epoch: ChainEpoch = 5;
    rt.set_epoch(current_epoch);

    // enroll event with miner at epoch=2
    let miner_epoch = 2;
    let payload = RawBytes::serialize(b"Cthulhu").unwrap();
    h.enroll_cron_event(&mut rt, miner_epoch, &MINER, &payload).unwrap();

    let events = h.get_enrolled_cron_ticks(&rt, miner_epoch);
    assert_eq!(events.len(), 1);
    assert_eq!(&events.last().unwrap().callback_payload, &payload);
    assert_eq!(events.last().unwrap().miner_addr, *MINER);

    let state: State = rt.get_state();
    assert_eq!(state.first_cron_epoch, 0);

    // enroll event with miner at epoch=1
    let miner_epoch = 1;
    let payload = RawBytes::serialize(b"Azathoth").unwrap();
    h.enroll_cron_event(&mut rt, miner_epoch, &MINER, &payload).unwrap();

    let events = h.get_enrolled_cron_ticks(&rt, miner_epoch);
    assert_eq!(events.len(), 1);
    assert_eq!(&events.last().unwrap().callback_payload, &payload);
    assert_eq!(events.last().unwrap().miner_addr, *MINER);

    let state: State = rt.get_state();
    assert_eq!(state.first_cron_epoch, 0);

    rt.verify();
    h.check_state(&rt);
}

#[test]
fn new_miner_updates_miner_above_min_power_count() {
    struct TestCase {
        proof: RegisteredPoStProof,
        expected_miners: i64,
    }

    let test_cases = [
        TestCase { proof: RegisteredPoStProof::StackedDRGWindow2KiBV1, expected_miners: 0 },
        TestCase { proof: RegisteredPoStProof::StackedDRGWindow32GiBV1, expected_miners: 0 },
    ];

    for test in test_cases {
        let (mut h, mut rt) = setup();
        h.window_post_proof = test.proof;
        h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();

        h.expect_miners_above_min_power(&mut rt, test.expected_miners);
        h.check_state(&rt);
    }
}

#[test]
fn power_accounting_crossing_threshold() {
    let small_power_unit = &StoragePower::from(1_000_000);
    let small_power_unit_x10 = &(small_power_unit * 10);

    let power_unit = &consensus_miner_min_power(
        &Policy::default(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
    )
    .unwrap();
    let power_unit_x10 = &(power_unit * 10);

    assert!(small_power_unit < power_unit);

    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER2).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER3).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER4).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER5).unwrap();

    // Use qa power 10x raw power to show it's not being used for threshold calculations.
    h.update_claimed_power(&mut rt, MINER1, small_power_unit, small_power_unit_x10);
    h.update_claimed_power(&mut rt, MINER2, small_power_unit, small_power_unit_x10);

    h.update_claimed_power(&mut rt, MINER3, power_unit, power_unit_x10);
    h.update_claimed_power(&mut rt, MINER4, power_unit, power_unit_x10);
    h.update_claimed_power(&mut rt, MINER5, power_unit, power_unit_x10);

    // Below threshold small miner power is counted
    let expected_total_below = small_power_unit * 2 + power_unit * 3;
    h.expect_total_power_eager(&mut rt, &expected_total_below, &(&expected_total_below * 10));

    // Above threshold (power.ConsensusMinerMinMiners = 4) small miner power is ignored
    let delta = &(power_unit - small_power_unit);
    h.update_claimed_power(&mut rt, MINER2, delta, &(delta * 10));
    let expected_total_above = &(power_unit * 4);
    h.expect_total_power_eager(&mut rt, expected_total_above, &(expected_total_above * 10));

    h.expect_miners_above_min_power(&mut rt, 4);

    // Less than 4 miners above threshold again small miner power is counted again
    h.update_claimed_power(&mut rt, MINER4, &delta.neg(), &(delta.neg() * 10));
    h.expect_total_power_eager(&mut rt, &expected_total_below, &(&expected_total_below * 10));
    h.check_state(&rt);
}

#[test]
fn all_of_one_miners_power_disappears_when_that_miner_dips_below_min_power_threshold() {
    let small_power_unit = &StoragePower::from(1_000_000);
    let power_unit = &consensus_miner_min_power(
        &Policy::default(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
    )
    .unwrap();

    assert!(small_power_unit < power_unit);

    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER2).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER3).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER4).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER5).unwrap();

    h.update_claimed_power(&mut rt, MINER1, power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER2, power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER3, power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER4, power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER5, power_unit, power_unit);

    let expected_total = &(power_unit * 5);
    h.expect_total_power_eager(&mut rt, expected_total, expected_total);

    // miner4 dips just below threshold
    h.update_claimed_power(&mut rt, MINER4, &small_power_unit.neg(), &small_power_unit.neg());

    let expected_total = &(power_unit * 4);
    h.expect_total_power_eager(&mut rt, expected_total, expected_total);
    h.check_state(&rt);
}

#[test]
fn enroll_cron_epoch_given_negative_epoch_should_fail() {
    let (h, mut rt) = setup();

    rt.set_caller(*MINER_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = EnrollCronEventParams {
        event_epoch: -1,
        payload: RawBytes::serialize(b"Cthulhu").unwrap(),
    };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<PowerActor>(
            Method::EnrollCronEvent as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );

    rt.verify();
    h.check_state(&rt);
}

#[test]
fn power_gets_added_when_miner_crosses_min_power_but_not_before() {
    let power_unit = &consensus_miner_min_power(
        &Policy::default(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
    )
    .unwrap();

    // Setup four miners above threshold
    let (mut h, mut rt) = setup();

    // create 4 miners that meet minimum
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER2).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER3).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER4).unwrap();

    h.update_claimed_power(&mut rt, MINER1, power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER2, power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER3, power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER4, power_unit, power_unit);

    h.expect_miners_above_min_power(&mut rt, 4);
    let expected_total = &(power_unit * 4);
    h.expect_total_power_eager(&mut rt, expected_total, expected_total);

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER5).unwrap();
    let below_limit_unit = power_unit / 2;

    // below limit actors power is not added
    h.update_claimed_power(&mut rt, MINER5, &below_limit_unit, &below_limit_unit);
    h.expect_miners_above_min_power(&mut rt, 4);
    h.expect_total_power_eager(&mut rt, expected_total, expected_total);

    // just below limit
    let delta = power_unit - below_limit_unit - 1;
    h.update_claimed_power(&mut rt, MINER5, &delta, &delta);
    h.expect_miners_above_min_power(&mut rt, 4);
    h.expect_total_power_eager(&mut rt, expected_total, expected_total);

    // at limit power is added
    h.update_claimed_power(&mut rt, MINER5, &StoragePower::from(1), &StoragePower::from(1));
    h.expect_miners_above_min_power(&mut rt, 5);
    let new_expected_total = expected_total + power_unit;
    h.expect_total_power_eager(&mut rt, &new_expected_total, &new_expected_total);
    h.check_state(&rt);
}

#[test]
fn threshold_only_depends_on_raw_power_not_qa_power() {
    let power_unit = &consensus_miner_min_power(
        &Policy::default(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
    )
    .unwrap();
    let half_power_unit = &(power_unit / 2);

    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER2).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER3).unwrap();
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER4).unwrap();

    h.update_claimed_power(&mut rt, MINER1, half_power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER2, half_power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER3, half_power_unit, power_unit);
    h.expect_miners_above_min_power(&mut rt, 0);

    h.update_claimed_power(&mut rt, MINER1, half_power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER2, half_power_unit, power_unit);
    h.update_claimed_power(&mut rt, MINER3, half_power_unit, power_unit);
    h.expect_miners_above_min_power(&mut rt, 3);
    h.check_state(&rt);
}

#[test]
fn qa_power_is_above_threshold_before_and_after_update() {
    let power_unit = &consensus_miner_min_power(
        &Policy::default(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
    )
    .unwrap();
    let power_unit_x3 = &(power_unit * 3);
    let power_unit_x4 = &(power_unit * 4);

    let (mut h, mut rt) = setup();

    // update claim so qa is above threshold
    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.update_claimed_power(&mut rt, MINER1, power_unit_x3, power_unit_x3);
    let st: State = rt.get_state();
    assert_eq!(power_unit_x3, &st.total_quality_adj_power);
    assert_eq!(power_unit_x3, &st.total_raw_byte_power);

    // update such that it's above threshold again
    h.update_claimed_power(&mut rt, MINER1, power_unit, power_unit);
    let st: State = rt.get_state();
    assert_eq!(power_unit_x4, &st.total_quality_adj_power);
    assert_eq!(power_unit_x4, &st.total_raw_byte_power);
    h.check_state(&rt);
}

#[test]
fn claimed_power_is_externally_available() {
    let power_unit = &consensus_miner_min_power(
        &Policy::default(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
    )
    .unwrap();

    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.update_claimed_power(&mut rt, MINER1, power_unit, power_unit);

    let claim = h.get_claim(&rt, &MINER1).unwrap();

    assert_eq!(power_unit, &claim.raw_byte_power);
    assert_eq!(power_unit, &claim.quality_adj_power);
    h.check_state(&rt);
}

#[test]
fn get_network_and_miner_power() {
    let power_unit = &consensus_miner_min_power(
        &Policy::default(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
    )
    .unwrap();

    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, MINER1).unwrap();
    h.update_claimed_power(&mut rt, MINER1, power_unit, power_unit);

    // manually update state in lieu of cron running
    let mut state: State = rt.get_state();
    state.this_epoch_raw_byte_power = power_unit.clone();
    rt.replace_state(&state);

    // set caller to not-builtin
    rt.set_caller(make_identity_cid(b"1234"), Address::new_id(1234));

    rt.expect_validate_caller_any();
    let network_power: NetworkRawPowerReturn = rt
        .call::<Actor>(Method::NetworkRawPowerExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    assert_eq!(power_unit, &network_power.raw_byte_power);

    rt.expect_validate_caller_any();
    let miner_power: MinerRawPowerReturn = rt
        .call::<Actor>(
            Method::MinerRawPowerExported as u64,
            &serialize(
                &MinerRawPowerParams { miner: MINER1.id().unwrap() },
                "serializing MinerRawPowerParams",
            )
            .unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();

    assert_eq!(power_unit, &miner_power.raw_byte_power);

    h.check_state(&rt);
}

#[test]
fn given_no_miner_claim_update_pledge_total_should_abort() {
    let (mut h, mut rt) = setup();

    h.create_miner_basic(&mut rt, *OWNER, *OWNER, *MINER).unwrap();

    // explicitly delete miner claim
    h.delete_claim(&mut rt, &*MINER);

    rt.set_caller(*MINER_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![Type::Miner]);
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "unknown miner",
        rt.call::<PowerActor>(
            Method::UpdatePledgeTotal as u64,
            &RawBytes::serialize(&TokenAmount::from_atto(1_000_000)).unwrap(),
        ),
    );

    rt.verify();
    h.check_state(&rt);
}

#[cfg(test)]
mod cron_tests {
    use super::*;

    use fil_actor_power::ext::reward::Method as RewardMethod;
    use fil_actor_power::ext::{
        miner::{DeferredCronEventParams, ON_DEFERRED_CRON_EVENT_METHOD},
        reward::UPDATE_NETWORK_KPI,
    };
    use fil_actors_runtime::{test_utils::CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR, REWARD_ACTOR_ADDR};
    use fvm_shared::bigint::BigInt;

    const OWNER: Address = Address::new_id(103);

    #[test]
    fn call_reward_actor() {
        let (h, mut rt) = setup();

        let expected_power = BigInt::zero();
        rt.set_epoch(1);

        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);

        h.expect_query_network_info(&mut rt);
        rt.expect_send(
            REWARD_ACTOR_ADDR,
            RewardMethod::UpdateNetworkKPI as u64,
            RawBytes::serialize(BigIntSer(&expected_power)).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);
        rt.expect_batch_verify_seals(Vec::new(), Ok(Vec::new()));

        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();

        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn amount_sent_to_reward_actor_and_state_change() {
        let (mut h, mut rt) = setup();
        let power_unit = consensus_miner_min_power(
            &Policy::default(),
            RegisteredPoStProof::StackedDRGWindow2KiBV1,
        )
        .unwrap();

        let miner1 = Address::new_id(101);
        let miner2 = Address::new_id(102);
        let miner3 = Address::new_id(103);
        let miner4 = Address::new_id(104);

        h.create_miner_basic(&mut rt, OWNER, OWNER, miner1).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner2).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner3).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner4).unwrap();

        h.update_claimed_power(&mut rt, miner1, &power_unit, &power_unit);
        h.update_claimed_power(&mut rt, miner2, &power_unit, &power_unit);
        h.update_claimed_power(&mut rt, miner3, &power_unit, &power_unit);
        h.update_claimed_power(&mut rt, miner4, &power_unit, &power_unit);

        let expected_power: BigInt = power_unit * 4u8;

        let delta = TokenAmount::from_atto(1u8);
        h.update_pledge_total(&mut rt, miner1, &delta);
        h.on_epoch_tick_end(&mut rt, 0, &expected_power, Vec::new(), Vec::new());

        let state: State = rt.get_state();

        assert_eq!(delta, state.this_epoch_pledge_collateral);
        assert_eq!(expected_power, state.this_epoch_quality_adj_power);
        assert_eq!(expected_power, state.this_epoch_raw_byte_power);

        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn event_scheduled_in_null_round_called_next_round() {
        let (mut h, mut rt) = setup();

        let miner1 = Address::new_id(101);
        let miner2 = Address::new_id(102);

        h.create_miner_basic(&mut rt, OWNER, OWNER, miner1).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner2).unwrap();

        //  0 - genesis
        //  1 - block - registers events
        //  2 - null  - has event
        //  3 - null
        //  4 - block - has event

        rt.set_epoch(1);
        h.enroll_cron_event(&mut rt, 2, &miner1, &RawBytes::from(vec![0x01, 0x03])).unwrap();
        h.enroll_cron_event(&mut rt, 4, &miner2, &RawBytes::from(vec![0x02, 0x03])).unwrap();

        let expected_raw_byte_power = BigInt::zero();
        rt.set_epoch(4);
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);
        h.expect_query_network_info(&mut rt);
        let state: State = rt.get_state();

        let params1 = DeferredCronEventParams {
            event_payload: vec![0x01, 0x03],
            reward_smoothed: h.this_epoch_reward_smoothed.clone(),
            quality_adj_power_smoothed: state.this_epoch_qa_power_smoothed.clone(),
        };
        rt.expect_send(
            miner1,
            ON_DEFERRED_CRON_EVENT_METHOD,
            RawBytes::serialize(params1).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        let params2 = DeferredCronEventParams {
            event_payload: vec![0x02, 0x03],
            reward_smoothed: h.this_epoch_reward_smoothed.clone(),
            quality_adj_power_smoothed: state.this_epoch_qa_power_smoothed,
        };
        rt.expect_send(
            miner2,
            ON_DEFERRED_CRON_EVENT_METHOD,
            RawBytes::serialize(params2).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&expected_raw_byte_power)).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);
        rt.expect_batch_verify_seals(Vec::new(), Ok(Vec::new()));
        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();

        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn event_scheduled_in_past_called_next_round() {
        let (mut h, mut rt) = setup();

        let miner_addr = Address::new_id(101);
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner_addr).unwrap();

        // run cron once to put it in a clean state at epoch 4
        let expected_raw_byte_power = BigInt::zero();
        rt.set_epoch(4);
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);
        h.expect_query_network_info(&mut rt);
        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&expected_raw_byte_power)).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);

        rt.expect_batch_verify_seals(Vec::new(), Ok(Vec::new()));

        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();
        rt.verify();

        // enroll a cron task at epoch 2 (which is in the past)
        let payload = vec![0x01, 0x03];
        h.enroll_cron_event(&mut rt, 2, &miner_addr, &RawBytes::from(payload.clone())).unwrap();

        // run cron again in the future
        rt.set_epoch(6);
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);
        h.expect_query_network_info(&mut rt);

        let state: State = rt.get_state();

        let input = DeferredCronEventParams {
            event_payload: payload,
            reward_smoothed: h.this_epoch_reward_smoothed.clone(),
            quality_adj_power_smoothed: state.this_epoch_qa_power_smoothed,
        };
        rt.expect_send(
            miner_addr,
            ON_DEFERRED_CRON_EVENT_METHOD,
            RawBytes::serialize(input).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&expected_raw_byte_power)).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);
        rt.expect_batch_verify_seals(Vec::new(), Ok(Vec::new()));

        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();
        rt.verify();

        // assert used cron events are cleaned up
        let state: State = rt.get_state();

        verify_empty_map(&rt, state.cron_event_queue);
        h.check_state(&rt);
    }

    #[test]
    fn fails_to_enroll_if_epoch_negative() {
        let (mut h, mut rt) = setup();
        let miner_addr = Address::new_id(101);
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner_addr).unwrap();

        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "epoch -2 cannot be less than zero",
            h.enroll_cron_event(&mut rt, -2, &miner_addr, &RawBytes::from(vec![0x01, 0x03])),
        );
        h.check_state(&rt);
    }

    #[test]
    fn skips_invocation_if_miner_has_no_claim() {
        let (mut h, mut rt) = setup();
        rt.set_epoch(1);

        let miner1 = Address::new_id(101);
        let miner2 = Address::new_id(102);

        h.create_miner_basic(&mut rt, OWNER, OWNER, miner1).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner2).unwrap();

        h.enroll_cron_event(&mut rt, 2, &miner1, &RawBytes::default()).unwrap();
        h.enroll_cron_event(&mut rt, 2, &miner2, &RawBytes::default()).unwrap();

        // explicitly delete miner 1's claim
        h.delete_claim(&mut rt, &miner1);

        rt.set_epoch(2);
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);

        // process batch verifies first
        rt.expect_batch_verify_seals(Vec::new(), Ok(Vec::new()));
        h.expect_query_network_info(&mut rt);

        let state: State = rt.get_state();
        let input = DeferredCronEventParams {
            event_payload: Vec::new(),
            reward_smoothed: h.this_epoch_reward_smoothed.clone(),
            quality_adj_power_smoothed: state.this_epoch_qa_power_smoothed,
        };

        // only expect second deferred cron event call
        rt.expect_send(
            miner2,
            ON_DEFERRED_CRON_EVENT_METHOD,
            RawBytes::serialize(input).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        // reward actor is still invoked
        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&BigInt::zero())).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);
        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();
        rt.verify();

        h.check_state(&rt);
    }

    #[test]
    fn handles_failed_call() {
        let (mut h, mut rt) = setup();
        rt.set_epoch(1);

        let miner1 = Address::new_id(101);
        let miner2 = Address::new_id(102);

        h.create_miner_basic(&mut rt, OWNER, OWNER, miner1).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner2).unwrap();

        h.enroll_cron_event(&mut rt, 2, &miner1, &RawBytes::default()).unwrap();
        h.enroll_cron_event(&mut rt, 2, &miner2, &RawBytes::default()).unwrap();

        let raw_power = consensus_miner_min_power(
            &Policy::default(),
            RegisteredPoStProof::StackedDRGWindow32GiBV1,
        )
        .unwrap();

        let qa_power = &raw_power;
        h.update_claimed_power(&mut rt, miner1, &raw_power, qa_power);
        h.expect_total_power_eager(&mut rt, &raw_power, qa_power);
        h.expect_miners_above_min_power(&mut rt, 1);

        rt.set_epoch(2);
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);

        // process batch verifies first
        rt.expect_batch_verify_seals(Vec::new(), Ok(Vec::new()));

        h.expect_query_network_info(&mut rt);

        let state: State = rt.get_state();
        let input = RawBytes::serialize(DeferredCronEventParams {
            event_payload: Vec::new(),
            reward_smoothed: h.this_epoch_reward_smoothed.clone(),
            quality_adj_power_smoothed: state.this_epoch_qa_power_smoothed,
        })
        .unwrap();

        // first send fails
        rt.expect_send(
            miner1,
            ON_DEFERRED_CRON_EVENT_METHOD,
            RawBytes::from(input.bytes().to_vec()),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::USR_ILLEGAL_STATE,
        );

        // subsequent one still invoked
        rt.expect_send(
            miner2,
            ON_DEFERRED_CRON_EVENT_METHOD,
            input,
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        // reward actor is still invoked
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);
        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&BigInt::zero())).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();
        rt.verify();

        // expect power stats to be decremented due to claim deletion
        h.expect_total_power_eager(&mut rt, &BigInt::zero(), &BigInt::zero());
        h.expect_miners_above_min_power(&mut rt, 0);

        // miner's claim is removed
        assert!(h.get_claim(&rt, &miner1).is_none());

        // miner count has been reduced to 1
        assert_eq!(h.miner_count(&mut rt), 1);

        // next epoch, only the reward actor is invoked
        rt.set_epoch(3);
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);

        h.expect_query_network_info(&mut rt);

        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&BigInt::zero())).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);
        rt.expect_batch_verify_seals(Vec::new(), Ok(Vec::new()));

        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();
        rt.verify();
        h.check_state(&rt);
    }
}

#[cfg(test)]
mod cron_batch_proof_verifies_tests {
    use super::*;
    use fil_actor_power::ext::{
        miner::{ConfirmSectorProofsParams, CONFIRM_SECTOR_PROOFS_VALID_METHOD},
        reward::UPDATE_NETWORK_KPI,
    };
    use fil_actors_runtime::{
        test_utils::{make_piece_cid, make_sealed_cid, CRON_ACTOR_CODE_ID},
        CRON_ACTOR_ADDR, REWARD_ACTOR_ADDR,
    };
    use fvm_shared::{
        bigint::BigInt,
        sector::{InteractiveSealRandomness, SealRandomness, SealVerifyInfo, SectorID},
    };

    fn create_basic_seal_info(id: u64) -> SealVerifyInfo {
        SealVerifyInfo {
            registered_proof: fvm_shared::sector::RegisteredSealProof::StackedDRG32GiBV1,
            deal_ids: Vec::new(),
            randomness: SealRandomness::default(),
            interactive_randomness: InteractiveSealRandomness::default(),
            proof: Vec::new(),
            sealed_cid: make_sealed_cid(format!("CommR-{id}").as_bytes()),
            unsealed_cid: make_piece_cid(format!("CommD-{id}").as_bytes()),
            sector_id: SectorID { number: id, ..Default::default() },
        }
    }

    const MINER_1: Address = Address::new_id(101);
    const OWNER: Address = Address::new_id(102);

    #[test]
    fn success_with_one_miner_and_one_confirmed_sector() {
        let (mut h, mut rt) = setup();

        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER_1).unwrap();

        let info = create_basic_seal_info(0);
        h.submit_porep_for_bulk_verify(&mut rt, MINER_1, info.clone(), true).unwrap();

        let confirmed_sectors =
            vec![ConfirmedSectorSend { miner: MINER_1, sector_nums: vec![info.sector_id.number] }];
        h.on_epoch_tick_end(&mut rt, 0, &BigInt::zero(), confirmed_sectors, vec![info]);

        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn success_with_one_miner_and_multiple_confirmed_sectors() {
        let (mut h, mut rt) = setup();

        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER_1).unwrap();

        let infos: Vec<_> = (1..=3).map(create_basic_seal_info).collect();
        infos.iter().for_each(|info| {
            h.submit_porep_for_bulk_verify(&mut rt, MINER_1, info.clone(), true).unwrap()
        });

        let sector_id_nums = infos.iter().map(|info| info.sector_id.number).collect();
        let confirmed_sectors =
            vec![ConfirmedSectorSend { miner: MINER_1, sector_nums: sector_id_nums }];
        h.on_epoch_tick_end(&mut rt, 0, &BigInt::zero(), confirmed_sectors, infos);

        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn duplicate_sector_numbers_are_ignored_for_a_miner() {
        let (mut h, mut rt) = setup();

        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER_1).unwrap();

        // duplicates will be sent to the batch verify call
        let infos =
            vec![create_basic_seal_info(1), create_basic_seal_info(1), create_basic_seal_info(2)];

        infos.iter().for_each(|info| {
            h.submit_porep_for_bulk_verify(&mut rt, MINER_1, info.clone(), true).unwrap()
        });

        // however, duplicates will not be sent to the miner as confirmed
        let sector_id_nums = vec![infos[0].sector_id.number, infos[2].sector_id.number];
        let confirmed_sectors =
            vec![ConfirmedSectorSend { miner: MINER_1, sector_nums: sector_id_nums }];
        h.on_epoch_tick_end(&mut rt, 0, &BigInt::zero(), confirmed_sectors, infos);

        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn skips_verify_if_miner_has_no_claim() {
        let (mut h, mut rt) = setup();
        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER_1).unwrap();

        let info = create_basic_seal_info(1);

        h.submit_porep_for_bulk_verify(&mut rt, MINER_1, info, true).unwrap();

        h.delete_claim(&mut rt, &MINER_1);

        let infos = vec![];

        let confirmed_sectors = vec![];

        h.on_epoch_tick_end(&mut rt, 0, &BigInt::zero(), confirmed_sectors, infos);

        h.check_state(&rt);
    }

    #[test]
    fn success_with_multiple_miners_and_multiple_confirmed_sectors_and_assert_expected_power() {
        let miner1 = Address::new_id(101);

        // TODO: shares an id with constant `OWNER`
        // this is a known issue however the ordering of the values
        // are vital for this test and have been left as such
        let miner2 = Address::new_id(102);
        let miner3 = Address::new_id(103);
        let miner4 = Address::new_id(104);

        let info1 = create_basic_seal_info(1);
        let info2 = create_basic_seal_info(2);
        let info3 = create_basic_seal_info(3);
        let info4 = create_basic_seal_info(101);
        let info5 = create_basic_seal_info(200);
        let info6 = create_basic_seal_info(201);
        let info7 = create_basic_seal_info(300);
        let info8 = create_basic_seal_info(301);

        let (mut h, mut rt) = setup();

        h.create_miner_basic(&mut rt, OWNER, OWNER, miner1).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner2).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner3).unwrap();
        h.create_miner_basic(&mut rt, OWNER, OWNER, miner4).unwrap();

        h.submit_porep_for_bulk_verify(&mut rt, miner1, info1.clone(), true).unwrap();
        h.submit_porep_for_bulk_verify(&mut rt, miner1, info2.clone(), true).unwrap();

        h.submit_porep_for_bulk_verify(&mut rt, miner2, info3.clone(), true).unwrap();
        h.submit_porep_for_bulk_verify(&mut rt, miner2, info4.clone(), true).unwrap();

        h.submit_porep_for_bulk_verify(&mut rt, miner3, info5.clone(), true).unwrap();
        h.submit_porep_for_bulk_verify(&mut rt, miner3, info6.clone(), true).unwrap();

        h.submit_porep_for_bulk_verify(&mut rt, miner4, info7.clone(), true).unwrap();
        h.submit_porep_for_bulk_verify(&mut rt, miner4, info8.clone(), true).unwrap();

        // TODO Because read order of keys in a multi-map is not as per insertion order,
        // we have to move around the expected sends
        let confirmed_sectors = vec![
            ConfirmedSectorSend {
                miner: MINER_1,
                sector_nums: vec![info1.sector_id.number, info2.sector_id.number],
            },
            ConfirmedSectorSend {
                miner: miner3,
                sector_nums: vec![info5.sector_id.number, info6.sector_id.number],
            },
            ConfirmedSectorSend {
                miner: miner4,
                sector_nums: vec![info7.sector_id.number, info8.sector_id.number],
            },
            ConfirmedSectorSend {
                miner: miner2,
                sector_nums: vec![info3.sector_id.number, info4.sector_id.number],
            },
        ];

        let infos = vec![info1, info2, info5, info6, info7, info8, info3, info4];

        h.on_epoch_tick_end(&mut rt, 0, &BigInt::zero(), confirmed_sectors, infos);
        h.check_state(&rt);
    }

    #[test]
    fn success_when_no_confirmed_sector() {
        let (h, mut rt) = setup();
        h.on_epoch_tick_end(&mut rt, 0, &BigInt::zero(), vec![], vec![]);

        h.check_state(&rt);
    }

    #[test]
    fn verification_for_one_sector_fails_but_others_succeeds_for_a_miner() {
        let (mut h, mut rt) = setup();
        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER_1).unwrap();

        let infos: Vec<_> = (1..=3).map(create_basic_seal_info).collect();
        infos.iter().for_each(|info| {
            h.submit_porep_for_bulk_verify(&mut rt, MINER_1, info.clone(), true).unwrap()
        });

        let res = Ok(vec![true, false, true]);

        // send will only be for the first and third sector as the middle sector will fail verification
        let cs = ConfirmedSectorSend {
            miner: MINER_1,
            sector_nums: vec![infos[0].sector_id.number, infos[2].sector_id.number],
        };

        h.expect_query_network_info(&mut rt);

        let state: State = rt.get_state();

        // expect sends for confirmed sectors
        let params = ConfirmSectorProofsParams {
            sectors: cs.sector_nums,
            reward_smoothed: h.this_epoch_reward_smoothed.clone(),
            reward_baseline_power: h.this_epoch_baseline_power().clone(),
            quality_adj_power_smoothed: state.this_epoch_qa_power_smoothed,
        };

        rt.expect_send(
            cs.miner,
            CONFIRM_SECTOR_PROOFS_VALID_METHOD,
            RawBytes::serialize(params).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        rt.expect_batch_verify_seals(infos, res);

        // expect power sends to reward actor
        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&BigInt::zero())).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);

        rt.set_epoch(0);
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);

        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();

        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn cron_tick_does_not_fail_if_batch_verify_seals_fails() {
        let (mut h, mut rt) = setup();
        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER_1).unwrap();

        let infos: Vec<_> = (1..=3).map(create_basic_seal_info).collect();
        infos.iter().for_each(|info| {
            h.submit_porep_for_bulk_verify(&mut rt, MINER_1, info.clone(), true).unwrap()
        });

        h.expect_query_network_info(&mut rt);

        rt.expect_batch_verify_seals(infos, Err(anyhow::Error::msg("fail")));
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);

        // expect power sends to reward actor
        rt.expect_send(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            RawBytes::serialize(BigIntSer(&BigInt::zero())).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_epoch(0);
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);

        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, &RawBytes::default()).unwrap();
        rt.verify();
        h.check_state(&rt);
    }
}

#[cfg(test)]
mod submit_porep_for_bulk_verify_tests {
    use super::*;

    use fil_actor_power::{
        ERR_TOO_MANY_PROVE_COMMITS, MAX_MINER_PROVE_COMMITS_PER_EPOCH,
        PROOF_VALIDATION_BATCH_AMT_BITWIDTH,
    };
    use fil_actors_runtime::shared::HAMT_BIT_WIDTH;
    use fil_actors_runtime::test_utils::{make_piece_cid, make_sealed_cid};
    use fil_actors_runtime::Multimap;
    use fvm_shared::sector::{InteractiveSealRandomness, SealRandomness, SealVerifyInfo, SectorID};

    const MINER: Address = Address::new_id(101);
    const OWNER: Address = Address::new_id(101);

    #[test]
    fn registers_porep_and_charges_gas() {
        let (mut h, mut rt) = setup();

        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER).unwrap();

        let comm_r = make_sealed_cid("commR".as_bytes());
        let comm_d = make_piece_cid("commD".as_bytes());

        let info = SealVerifyInfo {
            registered_proof: fvm_shared::sector::RegisteredSealProof::StackedDRG32GiBV1,
            deal_ids: Vec::new(),
            randomness: SealRandomness::default(),
            interactive_randomness: InteractiveSealRandomness::default(),
            proof: Vec::new(),
            sealed_cid: comm_r,
            unsealed_cid: comm_d,
            sector_id: SectorID { number: 0, ..Default::default() },
        };

        h.submit_porep_for_bulk_verify(&mut rt, MINER, info, true).unwrap();
        let st: State = rt.get_state();
        let store = &rt.store;
        assert!(st.proof_validation_batch.is_some());
        let mmap = Multimap::from_root(
            store,
            st.proof_validation_batch.as_ref().unwrap(),
            HAMT_BIT_WIDTH,
            PROOF_VALIDATION_BATCH_AMT_BITWIDTH,
        )
        .unwrap();
        let arr = mmap.get::<SealVerifyInfo>(&MINER.to_bytes()).unwrap();
        let found = arr.unwrap();
        assert_eq!(1_u64, found.count());
        let sealed_cid = found.get(0).unwrap().unwrap().sealed_cid;
        assert_eq!(comm_r, sealed_cid);
        h.check_state(&rt);
    }

    #[test]
    fn aborts_when_too_many_poreps() {
        let (mut h, mut rt) = setup();

        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER).unwrap();

        fn create_basic_seal_info(id: u64) -> SealVerifyInfo {
            SealVerifyInfo {
                registered_proof: fvm_shared::sector::RegisteredSealProof::StackedDRG32GiBV1,
                deal_ids: Vec::new(),
                randomness: SealRandomness::default(),
                interactive_randomness: InteractiveSealRandomness::default(),
                proof: Vec::new(),
                sealed_cid: make_sealed_cid(format!("CommR-{id}").as_bytes()),
                unsealed_cid: make_piece_cid(format!("CommD-{id}").as_bytes()),
                sector_id: SectorID { number: id, ..Default::default() },
            }
        }

        // Adding MAX_MINER_PROVE_COMMITS_PER_EPOCH works without error
        for i in 0..MAX_MINER_PROVE_COMMITS_PER_EPOCH {
            h.submit_porep_for_bulk_verify(&mut rt, MINER, create_basic_seal_info(i), true)
                .unwrap();
        }

        expect_abort(
            ERR_TOO_MANY_PROVE_COMMITS,
            h.submit_porep_for_bulk_verify(
                &mut rt,
                MINER,
                create_basic_seal_info(MAX_MINER_PROVE_COMMITS_PER_EPOCH),
                false,
            ),
        );

        h.check_state(&rt);
    }

    #[test]
    fn aborts_when_miner_has_no_claim() {
        let (mut h, mut rt) = setup();

        h.create_miner_basic(&mut rt, OWNER, OWNER, MINER).unwrap();

        let comm_r = make_sealed_cid("commR".as_bytes());
        let comm_d = make_piece_cid("commD".as_bytes());

        let info = SealVerifyInfo {
            registered_proof: fvm_shared::sector::RegisteredSealProof::StackedDRG32GiBV1,
            deal_ids: Vec::new(),
            randomness: SealRandomness::default(),
            interactive_randomness: InteractiveSealRandomness::default(),
            proof: Vec::new(),
            sealed_cid: comm_r,
            unsealed_cid: comm_d,
            sector_id: SectorID { number: 0, ..Default::default() },
        };

        // delete miner
        h.delete_claim(&mut rt, &MINER);

        expect_abort(
            ExitCode::USR_FORBIDDEN,
            h.submit_porep_for_bulk_verify(&mut rt, MINER, info, false),
        );
        h.check_state(&rt);
    }
}

#[test]
fn create_miner_restricted_correctly() {
    let (h, mut rt) = setup();

    let peer = "miner".as_bytes().to_vec();
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];

    let params = serialize(
        &CreateMinerParams {
            owner: *OWNER,
            worker: *OWNER,
            window_post_proof_type: RegisteredPoStProof::StackedDRGWinning2KiBV1,
            peer: peer.clone(),
            multiaddrs: multiaddrs.clone(),
        },
        "create miner params",
    )
    .unwrap();

    rt.set_caller(make_identity_cid(b"1234"), *OWNER);

    // cannot call the unexported method
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "must be built-in",
        rt.call::<PowerActor>(Method::CreateMiner as MethodNum, &params),
    );

    // can call the exported method

    rt.expect_validate_caller_any();
    let expected_init_params = ExecParams {
        code_cid: *MINER_ACTOR_CODE_ID,
        constructor_params: serialize(
            &MinerConstructorParams {
                owner: *OWNER,
                worker: *OWNER,
                control_addresses: vec![],
                window_post_proof_type: RegisteredPoStProof::StackedDRGWinning2KiBV1,
                peer_id: peer,
                multi_addresses: multiaddrs,
            },
            "minerctor params",
        )
        .unwrap(),
    };
    let create_miner_ret = CreateMinerReturn { id_address: *MINER, robust_address: *ACTOR };
    rt.expect_send(
        INIT_ACTOR_ADDR,
        EXEC_METHOD,
        RawBytes::serialize(expected_init_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::serialize(create_miner_ret).unwrap(),
        ExitCode::OK,
    );

    let ret: CreateMinerReturn = rt
        .call::<PowerActor>(Method::CreateMinerExported as MethodNum, &params)
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(ret.id_address, *MINER);
    assert_eq!(ret.robust_address, *ACTOR);

    h.check_state(&rt);
}

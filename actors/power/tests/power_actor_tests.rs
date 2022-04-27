use fil_actor_power::ext::init::{ExecParams, EXEC_METHOD};
use fil_actor_power::ext::miner::MinerConstructorParams;
use fil_actors_runtime::test_utils::{
    expect_abort, expect_abort_contains_message, ACCOUNT_ACTOR_CODE_ID, CALLER_TYPES_SIGNABLE,
    MINER_ACTOR_CODE_ID, SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredPoStProof, StoragePower};
use num_traits::Zero;

use fil_actor_power::{
    Actor as PowerActor, CreateMinerParams, EnrollCronEventParams, Method, State,
    UpdateClaimedPowerParams,
};

use crate::harness::*;

mod harness;

#[test]
fn construct() {
    let mut rt = new_runtime();
    let h = new_harness();
    h.construct_and_verify(&mut rt);
    h.check_state();
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
        &TokenAmount::from(10),
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
    h.check_state();
}

#[test]
fn create_miner_given_caller_is_not_of_signable_type_should_fail() {
    let (h, mut rt) = setup();

    let peer = "miner".as_bytes().to_vec();
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];

    let create_miner_params = CreateMinerParams {
        owner: *OWNER,
        worker: *OWNER,
        window_post_proof_type: RegisteredPoStProof::StackedDRGWindow32GiBV1,
        peer,
        multiaddrs,
    };

    rt.set_caller(*MINER_ACTOR_CODE_ID, *OWNER);
    rt.expect_validate_caller_type(CALLER_TYPES_SIGNABLE.to_vec());
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<PowerActor>(
            Method::CreateMiner as u64,
            &RawBytes::serialize(&create_miner_params).unwrap(),
        ),
    );
    rt.verify();
    h.check_state();
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
    rt.value_received = TokenAmount::from(10);
    rt.set_balance(TokenAmount::from(10));
    rt.expect_validate_caller_type(CALLER_TYPES_SIGNABLE.to_vec());

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
        *INIT_ACTOR_ADDR,
        EXEC_METHOD,
        RawBytes::serialize(message_params).unwrap(),
        TokenAmount::from(10),
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
    h.check_state();
}

#[test]
fn claimed_power_given_caller_is_not_storage_miner_should_fail() {
    let (h, mut rt) = setup();

    let params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::from(100),
        quality_adjusted_delta: StoragePower::from(200),
    };

    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<PowerActor>(
            Method::UpdateClaimedPower as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );

    rt.verify();
    h.check_state();
}

#[test]
fn claimed_power_given_claim_does_not_exist_should_fail() {
    let (h, mut rt) = setup();

    let params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::from(100),
        quality_adjusted_delta: StoragePower::from(200),
    };

    rt.set_caller(*MINER_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

    expect_abort(
        ExitCode::USR_NOT_FOUND,
        rt.call::<PowerActor>(
            Method::UpdateClaimedPower as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );

    rt.verify();
    h.check_state();
}

#[test]
fn enroll_cron_epoch_multiple_events() {
    let (h, mut rt) = setup();

    let peer = "miner".as_bytes().to_vec();
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];

    h.create_miner(
        &mut rt,
        &OWNER,
        &OWNER,
        &MINER,
        &ACTOR,
        peer.clone(),
        multiaddrs.clone(),
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
        &TokenAmount::zero(),
    )
    .unwrap();

    let miner2_address = Address::new_id(501);
    h.create_miner(
        &mut rt,
        &OWNER,
        &OWNER,
        &miner2_address,
        &ACTOR,
        peer,
        multiaddrs,
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
        &TokenAmount::zero(),
    )
    .unwrap();

    let mut enroll_and_check_cron_event = |epoch, miner_address, payload| {
        let pre_existing_event_count = h.get_enrolled_cron_ticks(&rt, epoch).len();

        h.enroll_cron_event(&mut rt, epoch, miner_address, payload);

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

    h.check_state();
}

#[test]
fn enroll_cron_epoch_before_current_epoch() {
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
        &TokenAmount::zero(),
    )
    .unwrap();

    let current_epoch: ChainEpoch = 5;
    rt.set_epoch(current_epoch);

    // enroll event with miner at epoch=2
    let miner_epoch = 2;
    let payload = RawBytes::serialize(b"Cthulhu").unwrap();
    h.enroll_cron_event(&mut rt, miner_epoch, &MINER, &payload);

    let events = h.get_enrolled_cron_ticks(&rt, miner_epoch);
    assert_eq!(events.len(), 1);
    assert_eq!(&events.last().unwrap().callback_payload, &payload);
    assert_eq!(events.last().unwrap().miner_addr, *MINER);

    let state: State = rt.get_state();
    assert_eq!(state.first_cron_epoch, 0);

    // enroll event with miner at epoch=1
    let miner_epoch = 1;
    let payload = RawBytes::serialize(b"Azathoth").unwrap();
    h.enroll_cron_event(&mut rt, miner_epoch, &MINER, &payload);

    let events = h.get_enrolled_cron_ticks(&rt, miner_epoch);
    assert_eq!(events.len(), 1);
    assert_eq!(&events.last().unwrap().callback_payload, &payload);
    assert_eq!(events.last().unwrap().miner_addr, *MINER);

    let state: State = rt.get_state();
    assert_eq!(state.first_cron_epoch, 0);

    rt.verify();
    h.check_state();
}

#[test]
fn enroll_cron_epoch_given_negative_epoch_should_fail() {
    let (h, mut rt) = setup();

    rt.set_caller(*MINER_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

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
    h.check_state();
}

#[test]
fn given_no_miner_claim_update_pledge_total_should_abort() {
    let (mut h, mut rt) = setup();

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
        &TokenAmount::zero(),
    )
    .unwrap();

    // explicitly delete miner claim
    h.delete_claim(&mut rt, &*MINER);

    rt.set_caller(*MINER_ACTOR_CODE_ID, *MINER);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "unknown miner",
        rt.call::<PowerActor>(
            Method::UpdatePledgeTotal as u64,
            &RawBytes::serialize(BigIntSer(&TokenAmount::from(1_000_000))).unwrap(),
        ),
    );

    rt.verify();
    h.check_state();
}

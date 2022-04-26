use fil_actor_power::ext::init::{ExecParams, EXEC_METHOD};
use fil_actor_power::ext::miner::MinerConstructorParams;
use fil_actors_runtime::test_utils::{
    expect_abort, ACCOUNT_ACTOR_CODE_ID, CALLER_TYPES_SIGNABLE, MINER_ACTOR_CODE_ID,
    SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredPoStProof, StoragePower};
use num_traits::Zero;

use fil_actor_power::{
    Actor as PowerActor, CreateMinerParams, Method, State, UpdateClaimedPowerParams,
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

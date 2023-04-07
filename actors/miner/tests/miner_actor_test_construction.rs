use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::INIT_ACTOR_ADDR;

use fil_actor_account::Method as AccountMethod;
use fil_actor_miner::{
    Actor, Deadline, Deadlines, Method, MinerConstructorParams as ConstructorParams, State,
};

use fvm_ipld_encoding::{BytesDe, CborStore};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredPoStProof, SectorSize};

use cid::Cid;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use num_traits::Zero;

mod util;

#[allow(dead_code)]
struct TestEnv {
    receiver: Address,
    owner: Address,
    worker: Address,
    worker_key: Address,
    control_addrs: Vec<Address>,
    peer_id: Vec<u8>,
    multiaddrs: Vec<BytesDe>,
    rt: MockRuntime,
}

fn prepare_env() -> TestEnv {
    let mut env = TestEnv {
        receiver: Address::new_id(1000),
        owner: Address::new_id(100),
        worker: Address::new_id(101),
        worker_key: new_bls_addr(0),
        control_addrs: vec![Address::new_id(999), Address::new_id(998)],
        peer_id: vec![1, 2, 3],
        multiaddrs: vec![BytesDe(vec![1, 2, 3])],
        rt: MockRuntime::default(),
    };

    env.rt.receiver = env.receiver;
    env.rt.actor_code_cids.borrow_mut().insert(env.owner, *ACCOUNT_ACTOR_CODE_ID);
    env.rt.actor_code_cids.borrow_mut().insert(env.worker, *ACCOUNT_ACTOR_CODE_ID);
    env.rt.actor_code_cids.borrow_mut().insert(env.control_addrs[0], *ACCOUNT_ACTOR_CODE_ID);
    env.rt.actor_code_cids.borrow_mut().insert(env.control_addrs[1], *ACCOUNT_ACTOR_CODE_ID);
    env.rt.hash_func = Box::new(hash);
    env.rt.caller.replace(INIT_ACTOR_ADDR);
    env.rt.caller_type.replace(*INIT_ACTOR_CODE_ID);
    env
}

fn constructor_params(env: &TestEnv) -> ConstructorParams {
    ConstructorParams {
        owner: env.owner,
        worker: env.worker,
        control_addresses: env.control_addrs.clone(),
        window_post_proof_type: RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
        peer_id: env.peer_id.clone(),
        multi_addresses: env.multiaddrs.clone(),
    }
}

#[test]
fn simple_construction() {
    let env = prepare_env();
    let params = constructor_params(&env);

    env.rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    env.rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);
    env.rt.expect_send_simple(
        env.worker,
        AccountMethod::PubkeyAddress as u64,
        None,
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&env.worker_key).unwrap(),
        ExitCode::OK,
    );

    let result = env
        .rt
        .call::<Actor>(Method::Constructor as u64, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap();
    expect_empty(result);
    env.rt.verify();

    let state = env.rt.get_state::<State>();

    let info = state.get_info(&env.rt.store).unwrap();
    assert_eq!(env.owner, info.owner);
    assert_eq!(env.worker, info.worker);
    assert_eq!(env.control_addrs, info.control_addresses);
    assert_eq!(env.peer_id, info.peer_id);
    assert_eq!(env.multiaddrs, info.multi_address);
    assert_eq!(RegisteredPoStProof::StackedDRGWindow32GiBV1P1, info.window_post_proof_type);
    assert_eq!(SectorSize::_32GiB, info.sector_size);
    assert_eq!(2349, info.window_post_partition_sectors);

    assert_eq!(TokenAmount::zero(), state.pre_commit_deposits);
    assert_eq!(TokenAmount::zero(), state.locked_funds);
    assert_ne!(Cid::default(), state.pre_committed_sectors);
    assert_ne!(Cid::default(), state.sectors);

    // according to original specs-actors test, this is set by running the code; magic...
    let proving_period_start = -2222;
    assert_eq!(proving_period_start, state.proving_period_start);
    // this is supposed to be the proving period cron
    let dl_idx =
        (*env.rt.epoch.borrow() - proving_period_start) / env.rt.policy.wpost_challenge_window;
    assert_eq!(dl_idx as u64, state.current_deadline);

    let deadlines = env.rt.store.get_cbor::<Deadlines>(&state.deadlines).unwrap().unwrap();
    for i in 0..env.rt.policy.wpost_period_deadlines {
        let c = deadlines.due[i as usize];
        let deadline = env.rt.store.get_cbor::<Deadline>(&c).unwrap().unwrap();
        assert_ne!(Cid::default(), deadline.partitions);
        assert_ne!(Cid::default(), deadline.expirations_epochs);
        assert!(deadline.partitions_posted.is_empty());
        assert!(deadline.early_terminations.is_empty());
        assert_eq!(0, deadline.live_sectors);
    }

    assert!(state.early_terminations.is_empty());

    util::check_state_invariants_from_mock_runtime(&env.rt);
}

#[test]
fn control_addresses_are_resolved_during_construction() {
    let mut env = prepare_env();

    let control1 = new_bls_addr(1);
    let control1id = Address::new_id(555);
    let control2 = new_bls_addr(2);
    let control2id = Address::new_id(655);

    env.control_addrs = vec![control1, control2];
    env.rt.actor_code_cids.borrow_mut().insert(control1id, *ACCOUNT_ACTOR_CODE_ID);
    env.rt.actor_code_cids.borrow_mut().insert(control2id, *ACCOUNT_ACTOR_CODE_ID);
    env.rt.id_addresses.borrow_mut().insert(control1, control1id);
    env.rt.id_addresses.borrow_mut().insert(control2, control2id);

    let params = constructor_params(&env);
    env.rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    env.rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);
    env.rt.expect_send_simple(
        env.worker,
        AccountMethod::PubkeyAddress as u64,
        None,
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&env.worker_key).unwrap(),
        ExitCode::OK,
    );

    let result = env
        .rt
        .call::<Actor>(Method::Constructor as u64, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap();
    expect_empty(result);
    env.rt.verify();

    let state: State = env.rt.get_state();
    let info = state.get_info(&env.rt.store).unwrap();

    assert_eq!(2, info.control_addresses.len());
    assert_eq!(control1id, info.control_addresses[0]);
    assert_eq!(control2id, info.control_addresses[1]);
}

#[test]
fn test_construct_with_invalid_peer_id() {
    let mut env = prepare_env();
    env.peer_id = vec![0; env.rt.policy.max_peer_id_length + 1];

    let params = constructor_params(&env);
    env.rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    env.rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);

    let result = env
        .rt
        .call::<Actor>(Method::Constructor as u64, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap_err();
    assert_eq!(result.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
    env.rt.verify();
}

#[test]
fn fails_if_control_addresses_exceeds_maximum_length() {
    let mut env = prepare_env();
    env.control_addrs = Vec::new();
    for i in 0..env.rt.policy.max_control_addresses + 1 {
        env.control_addrs.push(Address::new_id(i as u64));
    }

    let params = constructor_params(&env);
    env.rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    env.rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);

    let result = env
        .rt
        .call::<Actor>(Method::Constructor as u64, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap_err();
    assert_eq!(result.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
    env.rt.verify();
}

#[test]
fn test_construct_with_large_multiaddr() {
    let mut env = prepare_env();
    env.multiaddrs = Vec::new();
    for _ in 0..100 {
        env.multiaddrs.push(BytesDe(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]));
    }

    let params = constructor_params(&env);
    env.rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    env.rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);

    let result = env
        .rt
        .call::<Actor>(Method::Constructor as u64, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap_err();
    assert_eq!(result.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
    env.rt.verify();
}

#[test]
fn test_construct_with_empty_multiaddr() {
    let mut env = prepare_env();
    env.multiaddrs = Vec::new();
    env.multiaddrs.push(BytesDe(vec![]));
    env.multiaddrs.push(BytesDe(vec![1]));

    let params = constructor_params(&env);
    env.rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    env.rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);

    let result = env
        .rt
        .call::<Actor>(Method::Constructor as u64, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap_err();
    assert_eq!(result.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
    env.rt.verify();
}

use fil_actors_runtime::runtime::Runtime;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use hierarchical_sca::{subnet, Actor as SCAActor, Checkpoint, State};

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
fn register_subnet() {
    let (h, mut rt) = setup();

    // Register a subnet with 1FIL collateral
    let mut value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = subnet::new_id(&h.net_name, *SUBNET_ONE);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();

    // Registering an already existing subnet should fail
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::USR_ILLEGAL_ARGUMENT).unwrap();
    h.check_state();
    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);

    // Registering without enough collateral.
    value = TokenAmount::from(10_u64.pow(17));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::USR_ILLEGAL_ARGUMENT).unwrap();
    h.check_state();
    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);

    // Register additional subnet
    value = TokenAmount::from(12_i128.pow(18));
    h.register(&mut rt, &SUBNET_TWO, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 2);
    let shid = subnet::new_id(&h.net_name, *SUBNET_TWO);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();
}

#[test]
fn add_stake() {
    let (h, mut rt) = setup();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = subnet::new_id(&h.net_name, *SUBNET_ONE);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();

    // Add some stake
    h.add_stake(&mut rt, &shid, &value, ExitCode::OK).unwrap();
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.stake, TokenAmount::from(2_i16) * value.clone());

    // Add to unregistered subnet
    h.add_stake(
        &mut rt,
        &subnet::new_id(&h.net_name, *SUBNET_TWO),
        &value,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    )
    .unwrap();

    // Add some more stake
    h.add_stake(&mut rt, &shid, &value, ExitCode::OK).unwrap();
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.stake, TokenAmount::from(3_i16) * value.clone());

    // Add with zero value
    h.add_stake(&mut rt, &shid, &TokenAmount::zero(), ExitCode::USR_ILLEGAL_ARGUMENT).unwrap();
}

#[test]
fn release_stake() {
    let (h, mut rt) = setup();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = subnet::new_id(&h.net_name, *SUBNET_ONE);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();

    // Add some stake
    h.add_stake(&mut rt, &shid, &value, ExitCode::OK).unwrap();
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.stake, TokenAmount::from(2_i16) * value.clone());

    // Release some stake
    h.release_stake(&mut rt, &shid, &value, ExitCode::OK).unwrap();
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.stake, value.clone());
    assert_eq!(subnet.status, subnet::Status::Active);

    // Release from unregistered subnet
    h.release_stake(
        &mut rt,
        &subnet::new_id(&h.net_name, *SUBNET_TWO),
        &value,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    )
    .unwrap();

    // Release with zero value
    h.release_stake(&mut rt, &shid, &TokenAmount::zero(), ExitCode::USR_ILLEGAL_ARGUMENT).unwrap();

    // Release enough to inactivate
    rt.set_balance(TokenAmount::from(2_i16) * value.clone());
    h.release_stake(&mut rt, &shid, &TokenAmount::from(5u64.pow(17)), ExitCode::OK).unwrap();
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.stake, value.clone() - TokenAmount::from(5u64.pow(17)));
    assert_eq!(subnet.status, subnet::Status::Inactive);

    // Not enough funds to release
    h.release_stake(&mut rt, &shid, &value, ExitCode::USR_ILLEGAL_STATE).unwrap();

    // Balance is not enough to release
    //, ExitCode::OK).unwrap();
    rt.set_balance(TokenAmount::zero());
    h.release_stake(&mut rt, &shid, &TokenAmount::from(5u64.pow(17)), ExitCode::USR_ILLEGAL_STATE)
        .unwrap();
}

#[test]
fn kill() {
    let (h, mut rt) = setup();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = subnet::new_id(&h.net_name, *SUBNET_ONE);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();

    // Add some stake
    h.kill(&mut rt, &shid, &value, ExitCode::OK).unwrap();
    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 0);
    assert!(h.get_subnet(&rt, &shid).is_none());
}

#[test]
fn checkpoint_commit() {
    let (h, mut rt) = setup();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = subnet::new_id(&h.net_name, *SUBNET_ONE);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();

    // Commit first checkpoint for first window in first subnet
    let epoch: ChainEpoch = 10;
    rt.set_epoch(epoch);
    let ch = Checkpoint::new(shid.clone(), epoch + 9);

    h.commit_child_check(&mut rt, &shid, ch, ExitCode::OK, TokenAmount::zero()).unwrap();
    let st: State = rt.get_state();
    let commit = st.get_window_checkpoint(rt.store(), epoch).unwrap();
    assert_eq!(commit.epoch(), 10); // TODO: Here we use the default, what about change it when we register the subnet?
    assert_eq!(commit.data.children[0].checks.len(), 1);

    // Register second subnet
    h.register(&mut rt, &SUBNET_TWO, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 2);
    let shid = subnet::new_id(&h.net_name, *SUBNET_TWO);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    h.check_state();
}

use cid::Cid;
use fil_actors_runtime::runtime::Runtime;
use fvm_shared::address::subnet::ROOTNET_ID;
use fvm_shared::address::{Address, SubnetID};
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use hierarchical_sca::{
    get_bottomup_msg, subnet, Actor as SCAActor, Checkpoint, CrossMsgMetaArray, State,
    DEFAULT_CHECKPOINT_PERIOD,
};
use std::str::FromStr;

use crate::harness::*;

mod harness;

#[test]
fn construct() {
    let mut rt = new_runtime();
    let h = new_harness(ROOTNET_ID.clone());
    h.construct_and_verify(&mut rt);
    h.check_state();
}

#[test]
fn register_subnet() {
    let (h, mut rt) = setup_root();

    // Register a subnet with 1FIL collateral
    let mut value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = SubnetID::new(&h.net_name, *SUBNET_ONE);
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
    let shid = SubnetID::new(&h.net_name, *SUBNET_TWO);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();
}

#[test]
fn add_stake() {
    let (h, mut rt) = setup_root();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = SubnetID::new(&h.net_name, *SUBNET_ONE);
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
        &SubnetID::new(&h.net_name, *SUBNET_TWO),
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
    let (h, mut rt) = setup_root();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = SubnetID::new(&h.net_name, *SUBNET_ONE);
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
        &SubnetID::new(&h.net_name, *SUBNET_TWO),
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
fn test_kill() {
    let (h, mut rt) = setup_root();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = SubnetID::new(&h.net_name, *SUBNET_ONE);
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
    let (h, mut rt) = setup_root();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = SubnetID::new(&h.net_name, *SUBNET_ONE);
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

    h.commit_child_check(&mut rt, &shid, &ch, ExitCode::OK, TokenAmount::zero()).unwrap();
    let st: State = rt.get_state();
    let commit = st.get_window_checkpoint(rt.store(), epoch).unwrap();
    assert_eq!(commit.epoch(), DEFAULT_CHECKPOINT_PERIOD);
    let child_check = has_childcheck_source(&commit.data.children, &shid).unwrap();
    assert_eq!(&child_check.checks.len(), &1);
    assert_eq!(has_cid(&child_check.checks, &ch.cid()), true);

    // Commit a checkpoint for subnet twice
    h.commit_child_check(&mut rt, &shid, &ch, ExitCode::USR_ILLEGAL_ARGUMENT, TokenAmount::zero())
        .unwrap();
    let prev_cid = ch.cid();

    // Append a new checkpoint for the same subnet
    let mut ch = Checkpoint::new(shid.clone(), epoch + 11);
    ch.data.prev_check = prev_cid;
    h.commit_child_check(&mut rt, &shid, &ch, ExitCode::OK, TokenAmount::zero()).unwrap();
    let st: State = rt.get_state();
    let commit = st.get_window_checkpoint(rt.store(), epoch).unwrap();
    assert_eq!(commit.epoch(), DEFAULT_CHECKPOINT_PERIOD);
    let child_check = has_childcheck_source(&commit.data.children, &shid).unwrap();
    assert_eq!(&child_check.checks.len(), &2);
    assert_eq!(has_cid(&child_check.checks, &ch.cid()), true);

    // Register second subnet
    h.register(&mut rt, &SUBNET_TWO, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 2);
    let shid_two = SubnetID::new(&h.net_name, *SUBNET_TWO);
    let subnet = h.get_subnet(&rt, &shid_two).unwrap();
    assert_eq!(subnet.id, shid_two);
    h.check_state();

    // Trying to commit from the wrong subnet
    let ch = Checkpoint::new(shid.clone(), epoch + 9);
    h.commit_child_check(
        &mut rt,
        &shid_two,
        &ch,
        ExitCode::USR_ILLEGAL_ARGUMENT,
        TokenAmount::zero(),
    )
    .unwrap();

    // Commit first checkpoint for first window in second subnet
    let epoch: ChainEpoch = 10;
    rt.set_epoch(epoch);
    let ch = Checkpoint::new(shid_two.clone(), epoch + 9);

    h.commit_child_check(&mut rt, &shid_two, &ch, ExitCode::OK, TokenAmount::zero()).unwrap();
    let st: State = rt.get_state();
    let commit = st.get_window_checkpoint(rt.store(), epoch).unwrap();
    assert_eq!(commit.epoch(), DEFAULT_CHECKPOINT_PERIOD);
    let child_check = has_childcheck_source(&commit.data.children, &shid_two).unwrap();
    assert_eq!(&child_check.checks.len(), &1);
    assert_eq!(has_cid(&child_check.checks, &ch.cid()), true);
}

#[test]
fn checkpoint_crossmsgs() {
    let (h, mut rt) = setup_root();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = SubnetID::new(&h.net_name, *SUBNET_ONE);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();

    // Commit first checkpoint for first window in first subnet
    let epoch: ChainEpoch = 10;
    rt.set_epoch(epoch);
    let mut ch = Checkpoint::new(shid.clone(), epoch + 9);
    // Directed to other subnets
    add_msg_meta(
        &mut ch,
        &shid,
        &SubnetID::from_str("/root/f0102/f0101").unwrap(),
        "rand1".as_bytes().to_vec(),
        TokenAmount::zero(),
    );
    add_msg_meta(
        &mut ch,
        &shid,
        &SubnetID::from_str("/root/f0102/f0102").unwrap(),
        "rand2".as_bytes().to_vec(),
        TokenAmount::zero(),
    );
    // And to this subnet
    add_msg_meta(&mut ch, &shid, &h.net_name, "rand1".as_bytes().to_vec(), TokenAmount::zero());
    add_msg_meta(&mut ch, &shid, &h.net_name, "rand2".as_bytes().to_vec(), TokenAmount::zero());
    add_msg_meta(&mut ch, &shid, &h.net_name, "rand3".as_bytes().to_vec(), TokenAmount::zero());
    // And to other child from the subnet
    add_msg_meta(
        &mut ch,
        &shid,
        &SubnetID::new(&h.net_name, Address::new_id(100)),
        "rand1".as_bytes().to_vec(),
        TokenAmount::zero(),
    );

    h.commit_child_check(&mut rt, &shid, &ch, ExitCode::OK, TokenAmount::zero()).unwrap();
    let st: State = rt.get_state();
    let commit = st.get_window_checkpoint(rt.store(), epoch).unwrap();
    assert_eq!(commit.epoch(), DEFAULT_CHECKPOINT_PERIOD);
    let child_check = has_childcheck_source(&commit.data.children, &shid).unwrap();
    assert_eq!(&child_check.checks.len(), &1);
    assert_eq!(has_cid(&child_check.checks, &ch.cid()), true);

    let crossmsgs = CrossMsgMetaArray::load(&st.bottomup_msg_meta, rt.store()).unwrap();
    for item in 0..=2 {
        get_bottomup_msg(&crossmsgs, item).unwrap().unwrap();
    }
    // Check that the ones directed to other subnets are aggregated in message-meta
    for to in vec![
        SubnetID::from_str("/root/f0102/f0101").unwrap(),
        SubnetID::from_str("/root/f0102/f0102").unwrap(),
    ] {
        commit.crossmsg_meta(&h.net_name, &to).unwrap();
    }

    // TODO: Add another checkpoint with cross-messages and include some
    // values to test that the circulating supply is updated correctly.
    // (deferring these tests for when cross-message support is fully implemented).
}

#[test]
fn test_fund() {
    let (h, mut rt) = setup_root();

    // Register a subnet with 1FIL collateral
    let value = TokenAmount::from(10_u64.pow(18));
    h.register(&mut rt, &SUBNET_ONE, &value, ExitCode::OK).unwrap();

    let st: State = rt.get_state();
    assert_eq!(st.total_subnets, 1);
    let shid = SubnetID::new(&h.net_name, *SUBNET_ONE);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    assert_eq!(subnet.stake, value);
    assert_eq!(subnet.circ_supply, TokenAmount::zero());
    assert_eq!(subnet.status, subnet::Status::Active);
    h.check_state();

    let funder = Address::new_id(1001);
    let amount = TokenAmount::from(10_u64.pow(18));
    h.fund(&mut rt, &funder, &shid, ExitCode::OK, amount.clone(), 1, &amount).unwrap();
    let funder = Address::new_id(1002);
    let mut exp_cs = amount.clone() * 2;
    h.fund(&mut rt, &funder, &shid, ExitCode::OK, amount.clone(), 2, &exp_cs).unwrap();
    exp_cs += amount.clone();
    h.fund(&mut rt, &funder, &shid, ExitCode::OK, amount.clone(), 3, &exp_cs).unwrap();
    // No funds sent
    h.fund(
        &mut rt,
        &funder,
        &shid,
        ExitCode::USR_ILLEGAL_ARGUMENT,
        TokenAmount::zero(),
        3,
        &exp_cs,
    )
    .unwrap();

    // Subnet doesn't exist
    h.fund(
        &mut rt,
        &funder,
        &SubnetID::new(&h.net_name, *SUBNET_TWO),
        ExitCode::USR_ILLEGAL_ARGUMENT,
        TokenAmount::zero(),
        3,
        &exp_cs,
    )
    .unwrap();
}

#[test]
fn test_release() {
    let shid = SubnetID::new(&ROOTNET_ID, *SUBNET_ONE);
    let (h, mut rt) = setup(shid.clone());

    // Include some funds
    let releaser = Address::new_id(1001);

    // Release funds
    let r_amount = TokenAmount::from(5_u64.pow(18));
    rt.set_balance(2 * r_amount.clone());
    let prev_cid =
        h.release(&mut rt, &releaser, ExitCode::OK, r_amount.clone(), 0, &Cid::default()).unwrap();
    h.release(&mut rt, &releaser, ExitCode::OK, r_amount, 1, &prev_cid).unwrap();
}

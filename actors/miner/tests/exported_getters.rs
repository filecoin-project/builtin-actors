use fil_actor_miner::{
    locked_reward_from_reward, Actor, GetFeeDebtReturn, GetInitialPledgeReturn,
    GetLockedFundsReturn, GetOwnerReturn, GetPreCommitDepositReturn, GetSectorSizeReturn,
    IsControllingAddressParam, IsControllingAddressReturn, Method,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::test_utils::make_identity_cid;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount, sector::MAX_SECTOR_NUMBER};
use num_traits::Zero;

mod util;

use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

#[test]
fn info_getters() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    // set caller to not-builtin
    rt.set_caller(make_identity_cid(b"1234"), Address::new_id(1234));

    // owner is good
    rt.expect_validate_caller_any();
    let owner_ret: GetOwnerReturn = rt
        .call::<Actor>(Method::GetOwnerExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    assert_eq!(h.owner, owner_ret.owner);

    // check that the controlling addresses all return true
    for control in h.control_addrs.iter().chain(&[h.worker, h.owner]) {
        rt.expect_validate_caller_any();
        let is_control_ret: IsControllingAddressReturn = rt
            .call::<Actor>(
                Method::IsControllingAddressExported as u64,
                &serialize(&IsControllingAddressParam { address: *control }, "serializing control")
                    .unwrap(),
            )
            .unwrap()
            .deserialize()
            .unwrap();
        assert!(is_control_ret.is_controlling);

        rt.verify();
    }

    // check that a non-controlling address doesn't return true

    rt.expect_validate_caller_any();
    let is_control_ret: IsControllingAddressReturn = rt
        .call::<Actor>(
            Method::IsControllingAddressExported as u64,
            &serialize(
                &IsControllingAddressParam { address: INIT_ACTOR_ADDR },
                "serializing control",
            )
            .unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();
    assert!(!is_control_ret.is_controlling);

    rt.verify();

    // sector size is good
    rt.expect_validate_caller_any();
    let sector_size_ret: GetSectorSizeReturn = rt
        .call::<Actor>(Method::GetSectorSizeExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    assert_eq!(h.sector_size, sector_size_ret.sector_size);

    h.check_state(&rt);
}

#[test]
fn collateral_getters() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&mut rt);
    let dl_info = h.deadline(&rt);

    // Precommit a sector
    // Use the max sector number to make sure everything works.
    let sector_no = MAX_SECTOR_NUMBER;
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // something on deadline boundary but > 180 days

    let precommit_params =
        h.make_pre_commit_params(sector_no, precommit_epoch - 1, expiration, vec![]);
    let precommit =
        h.pre_commit_sector_and_get(&mut rt, precommit_params, PreCommitConfig::empty(), true);

    // Query the total precommit deposit from a non-builtin actor

    // set caller to not-builtin
    rt.set_caller(make_identity_cid(b"1234"), Address::new_id(1234));

    rt.expect_validate_caller_any();
    let pre_commit_deposit_ret: GetPreCommitDepositReturn = rt
        .call::<Actor>(Method::GetPreCommitDepositExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    // let's be sure we're not vacuously testing this method
    assert!(!precommit.pre_commit_deposit.is_zero());
    assert_eq!(precommit.pre_commit_deposit, pre_commit_deposit_ret.pre_commit_deposit);

    // run prove commit logic
    rt.set_epoch(prove_commit_epoch);
    rt.balance.replace(TokenAmount::from_whole(1000));
    let pcc = ProveCommitConfig::empty();

    let sector = h
        .prove_commit_sector_and_confirm(
            &mut rt,
            &precommit,
            h.make_prove_commit_params(sector_no),
            pcc,
        )
        .unwrap();

    // Query the total precommit deposits and initial pledge from a non-builtin actor

    // set caller to not-builtin
    rt.set_caller(make_identity_cid(b"1234"), Address::new_id(1234));

    // query PCD
    rt.expect_validate_caller_any();
    let pre_commit_deposit_ret: GetPreCommitDepositReturn = rt
        .call::<Actor>(Method::GetPreCommitDepositExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    assert!(pre_commit_deposit_ret.pre_commit_deposit.is_zero());

    // query IP

    rt.expect_validate_caller_any();
    let initial_pledge_ret: GetInitialPledgeReturn = rt
        .call::<Actor>(Method::GetInitialPledgeExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    // let's be sure we're not vacuously testing this method
    assert!(!sector.initial_pledge.is_zero());
    assert_eq!(sector.initial_pledge, initial_pledge_ret.initial_pledge);

    h.check_state(&rt);
}

#[test]
fn debt_and_vesting_getters() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    let reward_amount: TokenAmount = 4 * &*BIG_BALANCE;
    let (amount_locked, _) = locked_reward_from_reward(reward_amount.clone());
    rt.set_balance(amount_locked.clone());
    h.apply_rewards(&mut rt, reward_amount, TokenAmount::zero());

    // introduce fee debt
    let mut st = h.get_state(&rt);
    st.fee_debt = 4 * &*BIG_BALANCE;
    rt.replace_state(&st);

    // Query the total locked funds & debt from a non-builtin actor

    // set caller to not-builtin
    rt.set_caller(make_identity_cid(b"1234"), Address::new_id(1234));

    // locked funds
    rt.expect_validate_caller_any();
    let locked_funds_ret: GetLockedFundsReturn = rt
        .call::<Actor>(Method::GetLockedFundsExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    // let's be sure we're not vacuously testing this method
    assert!(!amount_locked.is_zero());
    assert_eq!(amount_locked, locked_funds_ret.locked_funds);

    // debt
    rt.expect_validate_caller_any();
    let debt_ret: GetFeeDebtReturn = rt
        .call::<Actor>(Method::GetFeeDebtExported as u64, &RawBytes::default())
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    assert_eq!(st.fee_debt, debt_ret.fee_debt);
}

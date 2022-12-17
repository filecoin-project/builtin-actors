use fil_actor_miner::{
    Actor, GetAvailableBalanceReturn, GetOwnerReturn, GetSectorSizeReturn,
    IsControllingAddressParam, IsControllingAddressReturn, Method,
};
use fil_actors_runtime::test_utils::make_identity_cid;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount, sector::MAX_SECTOR_NUMBER};
use std::ops::Sub;

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
    let owner_ret: GetOwnerReturn =
        rt.call::<Actor>(Method::GetOwnerExported as u64, None).unwrap().deserialize().unwrap();

    rt.verify();

    assert_eq!(h.owner, owner_ret.owner);

    // check that the controlling addresses all return true
    for control in h.control_addrs.iter().chain(&[h.worker, h.owner]) {
        rt.expect_validate_caller_any();
        let is_control_ret: IsControllingAddressReturn = rt
            .call::<Actor>(
                Method::IsControllingAddressExported as u64,
                IpldBlock::serialize_cbor(&IsControllingAddressParam { address: *control })
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
            IpldBlock::serialize_cbor(&IsControllingAddressParam { address: INIT_ACTOR_ADDR })
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
        .call::<Actor>(Method::GetSectorSizeExported as u64, None)
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

    // run prove commit logic
    rt.set_epoch(prove_commit_epoch);
    let actor_balance = TokenAmount::from_whole(1000);
    rt.balance.replace(actor_balance.clone());
    let pcc = ProveCommitConfig::empty();

    let sector = h
        .prove_commit_sector_and_confirm(
            &mut rt,
            &precommit,
            h.make_prove_commit_params(sector_no),
            pcc,
        )
        .unwrap();

    // query available balance

    rt.expect_validate_caller_any();
    let available_balance_ret: GetAvailableBalanceReturn = rt
        .call::<Actor>(Method::GetAvailableBalanceExported as u64, None)
        .unwrap()
        .deserialize()
        .unwrap();

    rt.verify();

    // let's be sure we're not vacuously testing this method
    assert_eq!(actor_balance.sub(sector.initial_pledge), available_balance_ret.available_balance);

    h.check_state(&rt);
}

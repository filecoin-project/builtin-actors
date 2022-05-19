use fil_actor_miner::{
    pledge_penalty_for_continued_fault, power_for_sectors, Deadline, PowerPair, SectorOnChainInfo,
};
use fil_actors_runtime::test_utils::MessageAccumulator;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_bitfield::BitField;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::{ChainEpoch, QuantSpec};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorSize;
use std::ops::Neg;

mod util;
use crate::util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const PERIOD_OFFSET: ChainEpoch = 100;
const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;
const BIG_REWARDS: u128 = 1_000 * 1e18 as u128;

#[test]
fn cron_enrolls_on_precommit_prove_commits_and_continues_enrolling() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let mut cron_ctrl = CronControl { rt, h, pre_commit_num: 0 };
    let long_expiration = 500;

    cron_ctrl.require_cron_inactive();
    let sectors =
        cron_ctrl.h.commit_and_prove_sectors(&mut cron_ctrl.rt, 1, long_expiration, vec![], true);
}
// func TestDeadlineCronDefersStopsRestarts(t *testing.T) {
// 	periodOffset := abi.ChainEpoch(100)
// 	actor := newHarness(t, periodOffset)
// 	builder := builderForHarness(actor).
// 		WithBalance(bigBalance, big.Zero())

// 	t.Run("cron enrolls on precommit, prove commits and continues enrolling", func(t *testing.T) {
// 		rt := builder.Build(t)
// 		actor.constructAndVerify(rt)
// 		cronCtrl := newCronControl(rt, actor)
// 		longExpiration := uint64(500)

// 		cronCtrl.requireCronInactive(t)
// 		sectors := actor.commitAndProveSectors(rt, 1, longExpiration, nil, true)
// 		cronCtrl.requireCronActive(t)

// 		// advance cron to activate power.
// 		advanceAndSubmitPoSts(rt, actor, sectors...)
// 		// advance 499 days of deadline (1 before expiration occurrs)
// 		// this asserts that cron continues to enroll within advanceAndSubmitPoSt
// 		for i := 0; i < 499; i++ {
// 			advanceAndSubmitPoSts(rt, actor, sectors...)
// 		}
// 		actor.checkState(rt)
// 		st := getState(rt)
// 		assert.True(t, st.DeadlineCronActive)
// 	})

// 	t.Run("cron enrolls on precommit, expires on pcd expiration, re-enrolls on new precommit immediately", func(t *testing.T) {
// 		rt := builder.Build(t)
// 		epoch := periodOffset + 1
// 		rt.SetEpoch(epoch)
// 		actor.constructAndVerify(rt)
// 		cronCtrl := newCronControl(rt, actor)

// 		epoch = cronCtrl.preCommitStartCronExpireStopCron(t, epoch)
// 		cronCtrl.preCommitToStartCron(t, epoch)
// 	})

// 	t.Run("cron enrolls on precommit, expires on pcd expiration, re-enrolls on new precommit after falling out of date", func(t *testing.T) {
// 		rt := builder.Build(t)
// 		epoch := periodOffset + 1
// 		rt.SetEpoch(epoch)
// 		actor.constructAndVerify(rt)
// 		cronCtrl := newCronControl(rt, actor)

// 		epoch = cronCtrl.preCommitStartCronExpireStopCron(t, epoch)
// 		// Advance some epochs to fall several pp out of date, then precommit again reenrolling cron
// 		epoch = epoch + 200*miner.WPoStProvingPeriod
// 		epoch = cronCtrl.preCommitStartCronExpireStopCron(t, epoch)
// 		// Stay within the same deadline but advance an epoch
// 		epoch = epoch + 1
// 		cronCtrl.preCommitToStartCron(t, epoch)
// 	})

// 	t.Run("enroll, pcd expire, re-enroll x 3", func(t *testing.T) {
// 		rt := builder.Build(t)
// 		epoch := periodOffset + 1
// 		rt.SetEpoch(epoch)
// 		actor.constructAndVerify(rt)
// 		cronCtrl := newCronControl(rt, actor)
// 		for i := 0; i < 3; i++ {
// 			epoch = cronCtrl.preCommitStartCronExpireStopCron(t, epoch) + 42
// 		}
// 	})
// }

use fil_actor_miner::power_for_sectors;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use std::ops::Neg;

mod util;
use crate::util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const PERIOD_OFFSET: ChainEpoch = 100;
const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;

#[test]
fn cron_on_inactive_state() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let st = h.get_state(&rt);
    assert_eq!(PERIOD_OFFSET - rt.policy.wpost_proving_period, st.proving_period_start);
    assert!(!st.continue_deadline_cron());

    // cron does nothing and does not enroll another cron
    let deadline = h.deadline(&rt);
    rt.set_epoch(deadline.last());
    h.on_deadline_cron(&mut rt, CronConfig { no_enrollment: true, ..CronConfig::default() });

    check_state_invariants(&rt);
}

#[test]
fn sector_expires() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let sectors =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&mut rt, &sectors);
    let active_power = power_for_sectors(h.sector_size, &sectors);

    let mut st = h.get_state(&rt);
    let initial_pledge = &st.initial_pledge;
    let expiration_raw = sectors[0].expiration;
    assert!(st.deadline_cron_active);

    // setup state to simulate moving forward all the way to expiry

    let (dl_idx, _) = st.find_sector(&rt.policy, &rt.store, sectors[0].sector_number).unwrap();
    let exp_quant_spec = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let expiration = exp_quant_spec.quantize_up(expiration_raw);
    let remaining_epoch = expiration - st.proving_period_start;
    let remaining_period = remaining_epoch / rt.policy.wpost_proving_period + 1;
    st.proving_period_start += remaining_period * rt.policy.wpost_proving_period;
    st.current_deadline = dl_idx;
    rt.replace_state(&st);

    // Advance to expiration epoch and expect expiration during cron
    rt.set_epoch(expiration);
    let power_delta = active_power.neg();
    // because we skip forward in state the sector is detected faulty, no penalty
    h.advance_deadline(
        &mut rt,
        CronConfig {
            no_enrollment: true,
            expired_sectors_power_delta: Some(power_delta),
            expired_sectors_pledge_delta: initial_pledge.neg(),
            ..CronConfig::default()
        },
    );
    let st = h.get_state(&rt);
    assert!(!st.deadline_cron_active);
    check_state_invariants(&rt);
}

// 	t.Run("sector expires and repays fee debt", func(t *testing.T) {
// 		rt := builder.Build(t)
// 		actor.constructAndVerify(rt)

// 		sectors := actor.commitAndProveSectors(rt, 1, defaultSectorExpiration, nil, true)
// 		// advance cron to activate power.
// 		advanceAndSubmitPoSts(rt, actor, sectors...)
// 		activePower := miner.PowerForSectors(actor.sectorSize, sectors)

// 		st := getState(rt)
// 		initialPledge := st.InitialPledge
// 		expirationRaw := sectors[0].Expiration
// 		assert.True(t, st.DeadlineCronActive)

// 		// setup state to simulate moving forward all the way to expiry
// 		dlIdx, _, err := st.FindSector(rt.AdtStore(), sectors[0].SectorNumber)
// 		require.NoError(t, err)
// 		expQuantSpec := st.QuantSpecForDeadline(dlIdx)
// 		expiration := expQuantSpec.QuantizeUp(expirationRaw)
// 		remainingEpochs := expiration - st.ProvingPeriodStart
// 		remainingPeriods := remainingEpochs/miner.WPoStProvingPeriod + 1
// 		st.ProvingPeriodStart += remainingPeriods * miner.WPoStProvingPeriod
// 		st.CurrentDeadline = dlIdx
// 		rt.ReplaceState(st)

// 		// Advance to expiration epoch and expect expiration during cron
// 		rt.SetEpoch(expiration)
// 		powerDelta := activePower.Neg()

// 		// introduce lots of fee debt
// 		st = getState(rt)
// 		feeDebt := big.Mul(big.NewInt(400), big.NewInt(1e18))
// 		st.FeeDebt = feeDebt
// 		rt.ReplaceState(st)
// 		// Miner balance = IP, debt repayment covered by unlocked funds
// 		rt.SetBalance(st.InitialPledge)

// 		// because we skip forward in state and don't check post, there's no penalty.
// 		// this is the first time the sector is detected faulty
// 		advanceDeadline(rt, actor, &cronConfig{
// 			noEnrollment:              true,
// 			expiredSectorsPowerDelta:  &powerDelta,
// 			expiredSectorsPledgeDelta: initialPledge.Neg(),
// 			repaidFeeDebt:             initialPledge, // We repay unlocked IP as fees
// 		})
// 		st = getState(rt)
// 		assert.False(t, st.DeadlineCronActive)
// 		actor.checkState(rt)
// 	})

// 	t.Run("detects and penalizes faults", func(t *testing.T) {
// 		rt := builder.Build(t)
// 		actor.constructAndVerify(rt)

// 		activeSectors := actor.commitAndProveSectors(rt, 2, defaultSectorExpiration, nil, true)
// 		// advance cron to activate power.
// 		advanceAndSubmitPoSts(rt, actor, activeSectors...)
// 		activePower := miner.PowerForSectors(actor.sectorSize, activeSectors)

// 		unprovenSectors := actor.commitAndProveSectors(rt, 1, defaultSectorExpiration, nil, false)
// 		unprovenPower := miner.PowerForSectors(actor.sectorSize, unprovenSectors)

// 		totalPower := unprovenPower.Add(activePower)
// 		allSectors := append(activeSectors, unprovenSectors...)

// 		// add lots of funds so penalties come from vesting funds
// 		actor.applyRewards(rt, bigRewards, big.Zero())

// 		st := getState(rt)
// 		dlIdx, pIdx, err := st.FindSector(rt.AdtStore(), activeSectors[0].SectorNumber)
// 		require.NoError(t, err)

// 		// advance to next deadline where we expect the first sectors to appear
// 		dlinfo := actor.deadline(rt)
// 		for dlinfo.Index != dlIdx {
// 			dlinfo = advanceDeadline(rt, actor, &cronConfig{})
// 		}

// 		// Skip to end of the deadline, cron detects sectors as faulty
// 		activePowerDelta := activePower.Neg()
// 		advanceDeadline(rt, actor, &cronConfig{
// 			detectedFaultsPowerDelta: &activePowerDelta,
// 		})

// 		// expect faulty power to be added to state
// 		deadline := actor.getDeadline(rt, dlIdx)
// 		assert.True(t, totalPower.Equals(deadline.FaultyPower))

// 		// advance 3 deadlines
// 		advanceDeadline(rt, actor, &cronConfig{})
// 		advanceDeadline(rt, actor, &cronConfig{})
// 		dlinfo = advanceDeadline(rt, actor, &cronConfig{})

// 		actor.declareRecoveries(rt, dlIdx, pIdx, sectorInfoAsBitfield(allSectors[1:]), big.Zero())

// 		// Skip to end of proving period for sectors, cron detects all sectors as faulty
// 		for dlinfo.Index != dlIdx {
// 			dlinfo = advanceDeadline(rt, actor, &cronConfig{})
// 		}

// 		// Un-recovered faults (incl failed recovery) are charged as ongoing faults
// 		ongoingPwr := miner.PowerForSectors(actor.sectorSize, allSectors)
// 		ongoingPenalty := miner.PledgePenaltyForContinuedFault(actor.epochRewardSmooth, actor.epochQAPowerSmooth, ongoingPwr.QA)

// 		advanceDeadline(rt, actor, &cronConfig{
// 			continuedFaultsPenalty: ongoingPenalty,
// 		})

// 		// recorded faulty power is unchanged
// 		deadline = actor.getDeadline(rt, dlIdx)
// 		assert.True(t, totalPower.Equals(deadline.FaultyPower))
// 		checkDeadlineInvariants(t, rt.AdtStore(), deadline, st.QuantSpecForDeadline(dlIdx), actor.sectorSize, allSectors)
// 		actor.checkState(rt)
// 	})

// 	t.Run("test cron run trigger faults", func(t *testing.T) {
// 		rt := builder.Build(t)
// 		actor.constructAndVerify(rt)

// 		// add lots of funds so we can pay penalties without going into debt
// 		actor.applyRewards(rt, bigRewards, big.Zero())

// 		// create enough sectors that one will be in a different partition
// 		allSectors := actor.commitAndProveSectors(rt, 1, defaultSectorExpiration, nil, true)

// 		// advance cron to activate power.
// 		advanceAndSubmitPoSts(rt, actor, allSectors...)

// 		st := getState(rt)
// 		dlIdx, _, err := st.FindSector(rt.AdtStore(), allSectors[0].SectorNumber)
// 		require.NoError(t, err)

// 		// advance to deadline prior to first
// 		dlinfo := actor.deadline(rt)
// 		for dlinfo.Index != dlIdx {
// 			dlinfo = advanceDeadline(rt, actor, &cronConfig{})
// 		}

// 		rt.SetEpoch(dlinfo.Last())

// 		// run cron and expect all sectors to be detected as faults (no penalty)
// 		pwr := miner.PowerForSectors(actor.sectorSize, allSectors)

// 		// power for sectors is removed
// 		powerDeltaClaim := miner.NewPowerPair(pwr.Raw.Neg(), pwr.QA.Neg())

// 		// expect next cron to be one deadline period after expected cron for this deadline
// 		nextCron := dlinfo.Last() + miner.WPoStChallengeWindow

// 		actor.onDeadlineCron(rt, &cronConfig{
// 			expectedEnrollment:       nextCron,
// 			detectedFaultsPowerDelta: &powerDeltaClaim,
// 		})
// 		actor.checkState(rt)
// 	})
// }

use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

mod util;
use crate::util::*;

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

// 	t.Run("sector expires", func(t *testing.T) {
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
// 		// because we skip forward in state the sector is detected faulty, no penalty
// 		advanceDeadline(rt, actor, &cronConfig{
// 			noEnrollment:              true, // the last power has expired so we expect cron to go inactive
// 			expiredSectorsPowerDelta:  &powerDelta,
// 			expiredSectorsPledgeDelta: initialPledge.Neg(),
// 		})
// 		st = getState(rt)
// 		assert.False(t, st.DeadlineCronActive)
// 		actor.checkState(rt)
// 	})

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

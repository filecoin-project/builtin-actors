use fil_actor_miner::{
    initial_pledge_for_power, pre_commit_deposit_for_power, qa_power_for_weight, PowerPair,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::DealWeight;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use fvm_shared::sector::MAX_SECTOR_NUMBER;
use std::collections::HashMap;

mod util;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: u64 = 220;

const VERIFIED_DEAL_WEIGHT_MULTIPLIER: u64 = 100;
const QUALITY_BASE_MULTIPLIER: u64 = 10;
const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;
const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn prove_single_sector() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.balance.replace(TokenAmount::from(BIG_BALANCE));

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    // actor.constructAndVerify(rt)
    h.construct_and_verify(&mut rt);
    let dl_info = h.deadline(&rt);

    // Make a good commitment for the proof to target.
    // Use the max sector number to make sure everything works.
    let sector_no = MAX_SECTOR_NUMBER;
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION as i64 * rt.policy.wpost_proving_period; // something on deadline boundary but > 180 days
                                                                                                  // Fill the sector with verified deals
    let sector_weight =
        DealWeight::from(h.sector_size as u64) * DealWeight::from(expiration - prove_commit_epoch);
    let deal_weight = DealWeight::zero();
    let verified_deal_weight = sector_weight;

    // Pre-commit with a deal in order to exercise non-zero deal weights.
    let precommit_params =
        h.make_pre_commit_params(sector_no, precommit_epoch - 1, expiration, vec![1]);
    let precommit = h.pre_commit_sector(
        &mut rt,
        precommit_params,
        PreCommitConfig {
            deal_weight: deal_weight.clone(),
            verified_deal_weight: verified_deal_weight.clone(),
            deal_space: None,
        },
        true,
    );

    // Check precommit
    // deal weights must be set in precommit onchain info
    assert_eq!(&deal_weight, &precommit.deal_weight);
    assert_eq!(&verified_deal_weight, &precommit.verified_deal_weight);

    let pwr_estimate = qa_power_for_weight(
        h.sector_size,
        precommit.info.expiration - precommit_epoch,
        &precommit.deal_weight,
        &precommit.verified_deal_weight,
    );
    let expected_deposit = pre_commit_deposit_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &pwr_estimate,
    );
    assert_eq!(expected_deposit, precommit.pre_commit_deposit);

    // expect total precommit deposit to equal our new deposit
    let st = h.get_state(&rt);
    assert_eq!(expected_deposit, st.pre_commit_deposits);

    // run prove commit logic
    rt.set_epoch(prove_commit_epoch);
    rt.balance.replace(TokenAmount::from(1000) * 1e18 as u64);
    let sector = h.prove_commit_sector_and_confirm(
        &mut rt,
        &precommit,
        h.make_prove_commit_params(sector_no),
        ProveCommitConfig::default(),
    );

    assert_eq!(precommit.info.seal_proof, sector.seal_proof);
    assert_eq!(precommit.info.sealed_cid, sector.sealed_cid);
    assert_eq!(precommit.info.deal_ids, sector.deal_ids);
    assert_eq!(rt.epoch, sector.activation);
    assert_eq!(precommit.info.expiration, sector.expiration);
    assert_eq!(precommit.deal_weight, sector.deal_weight);
    assert_eq!(precommit.verified_deal_weight, sector.verified_deal_weight);

    // expect precommit to have been removed
    let st = h.get_state(&rt);
    let found = st.get_precommitted_sector(&rt.store, sector_no).unwrap();
    assert!(found.is_none());

    // expect deposit to have been transferred to initial pledges
    assert!(st.pre_commit_deposits.is_zero());

    // The sector is exactly full with verified deals, so expect fully verified power.
    let expected_power = StoragePower::from(h.sector_size as u64)
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER / QUALITY_BASE_MULTIPLIER);
    let qa_power = qa_power_for_weight(
        h.sector_size,
        precommit.info.expiration - rt.epoch,
        &precommit.deal_weight,
        &precommit.verified_deal_weight,
    );
    assert_eq!(expected_power, qa_power);
    let sector_power =
        PowerPair { raw: StoragePower::from(h.sector_size as u64), qa: qa_power.clone() };

    // expect deal weights to be transferred to on chain info
    assert_eq!(precommit.deal_weight, sector.deal_weight);
    assert_eq!(precommit.verified_deal_weight, sector.verified_deal_weight);

    // expect initial plege of sector to be set, and be total pledge requirement
    let expected_initial_pledge = initial_pledge_for_power(
        &qa_power,
        &h.baseline_power,
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );
    assert_eq!(expected_initial_pledge, sector.initial_pledge);
    assert_eq!(expected_initial_pledge, st.initial_pledge);

    // expect sector to be assigned a deadline/partition
    let (dl_idx, p_idx) = st.find_sector(&rt.policy, rt.store(), sector_no).unwrap();
    let (deadline, partition) = h.get_deadline_and_partition(&rt, dl_idx, p_idx);
    assert_eq!(1, deadline.live_sectors);
    assert!(deadline.partitions_posted.is_empty());
    assert!(deadline.early_terminations.is_empty());

    let quant = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let quantized_expiration = quant.quantize_up(precommit.info.expiration);

    let d_queue = h.collect_deadline_expirations(&rt, &deadline);
    assert_eq!(d_queue, HashMap::from([(quantized_expiration, vec![p_idx])]));

    assert_bitfield_equals(&partition.sectors, &[sector_no]);
    assert!(partition.faults.is_empty());
    assert!(partition.recoveries.is_empty());
    assert!(partition.terminated.is_empty());
    assert_eq!(sector_power, partition.live_power);
    assert!(partition.faulty_power.is_zero());
    assert!(partition.recovering_power.is_zero());

    let p_queue = h.collect_partition_expirations(&rt, &partition);
    let entry = &p_queue[&quantized_expiration];
    assert_bitfield_equals(&entry.on_time_sectors, &[sector_no]);
    assert!(entry.early_sectors.is_empty());
    assert_eq!(expected_initial_pledge, entry.on_time_pledge);
    assert_eq!(sector_power, entry.active_power);
    assert!(entry.faulty_power.is_zero());
}

/*
    t.Run("prove sectors from batch pre-commit", func(t *testing.T) {
        actor := newHarness(t, periodOffset)
        rt := builderForHarness(actor).
            WithBalance(bigBalance, big.Zero()).
            Build(t)
        precommitEpoch := periodOffset + 1
        rt.SetEpoch(precommitEpoch)
        actor.constructAndVerify(rt)
        dlInfo := actor.deadline(rt)

        sectorExpiration := dlInfo.PeriodEnd() + defaultSectorExpiration*miner.WPoStProvingPeriod

        sectors := []miner0.SectorPreCommitInfo{
            *actor.makePreCommit(100, precommitEpoch-1, sectorExpiration, nil),
            *actor.makePreCommit(101, precommitEpoch-1, sectorExpiration, []abi.DealID{1}),    // 1 * 32GiB verified deal
            *actor.makePreCommit(102, precommitEpoch-1, sectorExpiration, []abi.DealID{2, 3}), // 2 * 16GiB verified deals
        }

        dealSpace := uint64(32 << 30)
        dealWeight := big.Zero()
        proveCommitEpoch := precommitEpoch + miner.PreCommitChallengeDelay + 1
        dealLifespan := sectorExpiration - proveCommitEpoch
        verifiedDealWeight := big.Mul(big.NewIntUnsigned(dealSpace), big.NewInt(int64(dealLifespan)))

        // Power estimates made a pre-commit time
        noDealPowerEstimate := miner.QAPowerForWeight(actor.sectorSize, sectorExpiration-precommitEpoch, big.Zero(), big.Zero())
        fullDealPowerEstimate := miner.QAPowerForWeight(actor.sectorSize, sectorExpiration-precommitEpoch, dealWeight, verifiedDealWeight)

        deposits := []big.Int{
            miner.PreCommitDepositForPower(actor.epochRewardSmooth, actor.epochQAPowerSmooth, noDealPowerEstimate),
            miner.PreCommitDepositForPower(actor.epochRewardSmooth, actor.epochQAPowerSmooth, fullDealPowerEstimate),
            miner.PreCommitDepositForPower(actor.epochRewardSmooth, actor.epochQAPowerSmooth, fullDealPowerEstimate),
        }
        conf := preCommitBatchConf{
            sectorWeights: []market.SectorWeights{
                {DealSpace: 0, DealWeight: big.Zero(), verified_deal_weight: big.Zero()},
                {DealSpace: dealSpace, DealWeight: dealWeight, verified_deal_weight: verifiedDealWeight},
                {DealSpace: dealSpace, DealWeight: dealWeight, verified_deal_weight: verifiedDealWeight},
            },
            firstForMiner: true,
        }

        precommits := actor.preCommitSectorBatch(rt, &miner.PreCommitSectorBatchParams{Sectors: sectors}, conf, big.Zero())

        rt.SetEpoch(proveCommitEpoch)
        noDealPower := miner.QAPowerForWeight(actor.sectorSize, sectorExpiration-proveCommitEpoch, big.Zero(), big.Zero())
        noDealPledge := miner.InitialPledgeForPower(noDealPower, actor.baselinePower, actor.epochRewardSmooth, actor.epochQAPowerSmooth, rt.TotalFilCircSupply())
        fullDealPower := miner.QAPowerForWeight(actor.sectorSize, sectorExpiration-proveCommitEpoch, dealWeight, verifiedDealWeight)
        assert_eq!( big.Mul(big.NewInt(int64(actor.sectorSize)), big.Div(builtin.verified_deal_weightMultiplier, builtin.QualityBaseMultiplier)), fullDealPower)
        fullDealPledge := miner.InitialPledgeForPower(fullDealPower, actor.baselinePower, actor.epochRewardSmooth, actor.epochQAPowerSmooth, rt.TotalFilCircSupply())

        // Prove just the first sector, with no deals
        {
            precommit := precommits[0]
            sector := actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(precommit.Info.SectorNumber), proveCommitConf{})
            assert_eq!( rt.Epoch(), sector.Activation)
            st := getState(rt)
            expectedDeposit := big.Sum(deposits[1:]...) // First sector deposit released
            assert_eq!( expectedDeposit, st.PreCommitDeposits)

            // Expect power/pledge for a sector with no deals
            assert_eq!( noDealPledge, sector.initial_pledge)
            assert_eq!( noDealPledge, st.initial_pledge)
        }
        // Prove the next, with one deal
        {
            precommit := precommits[1]
            sector := actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(precommit.Info.SectorNumber), proveCommitConf{})
            assert_eq!( rt.Epoch(), sector.Activation)
            st := getState(rt)
            expectedDeposit := big.Sum(deposits[2:]...) // First and second sector deposits released
            assert_eq!( expectedDeposit, st.PreCommitDeposits)

            // Expect power/pledge for the two sectors (only this one having any deal weight)
            assert_eq!( fullDealPledge, sector.initial_pledge)
            assert_eq!( big.Add(noDealPledge, fullDealPledge), st.initial_pledge)
        }
        // Prove the last
        {
            precommit := precommits[2]
            sector := actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(precommit.Info.SectorNumber), proveCommitConf{})
            assert_eq!( rt.Epoch(), sector.Activation)
            st := getState(rt)
            assert_eq!( big.Zero(), st.PreCommitDeposits)

            // Expect power/pledge for the three sectors
            assert_eq!( fullDealPledge, sector.initial_pledge)
            assert_eq!( big.Sum(noDealPledge, fullDealPledge, fullDealPledge), st.initial_pledge)
        }
    })

    t.Run("invalid proof rejected", func(t *testing.T) {
        actor := newHarness(t, periodOffset)
        rt := builderForHarness(actor).
            WithBalance(bigBalance, big.Zero()).
            Build(t)
        precommitEpoch := periodOffset + 1
        rt.SetEpoch(precommitEpoch)
        actor.constructAndVerify(rt)
        deadline := actor.deadline(rt)

        // Make a good commitment for the proof to target.
        sectorNo := abi.SectorNumber(100)
        params := actor.makePreCommit(sectorNo, precommitEpoch-1, deadline.PeriodEnd()+defaultSectorExpiration*miner.WPoStProvingPeriod, []abi.DealID{1})
        precommit := actor.preCommitSector(rt, params, preCommitConf{}, true)

        // Sector pre-commitment missing.
        rt.SetEpoch(precommitEpoch + miner.PreCommitChallengeDelay + 1)
        rt.ExpectAbort(exitcode.ErrNotFound, func() {
            actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(sectorNo+1), proveCommitConf{})
        })
        rt.Reset()

        // Too late.
        rt.SetEpoch(precommitEpoch + miner.MaxProveCommitDuration[precommit.Info.SealProof] + 1)
        rt.ExpectAbort(exitcode.ErrIllegalArgument, func() {
            actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(sectorNo), proveCommitConf{})
        })
        rt.Reset()

        // Too early.
        rt.SetEpoch(precommitEpoch + miner.PreCommitChallengeDelay - 1)
        rt.ExpectAbort(exitcode.ErrForbidden, func() {
            actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(sectorNo), proveCommitConf{})
        })
        rt.Reset()

        // Set the right epoch for all following tests
        rt.SetEpoch(precommitEpoch + miner.PreCommitChallengeDelay + 1)

        // Invalid deals (market ActivateDeals aborts)
        verifyDealsExit := map[abi.SectorNumber]exitcode.ExitCode{
            precommit.Info.SectorNumber: exitcode.ErrIllegalArgument,
        }
        rt.ExpectAbort(exitcode.ErrIllegalArgument, func() {
            actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(sectorNo), proveCommitConf{
                verifyDealsExit: verifyDealsExit,
            })
        })
        rt.Reset()

        rt.SetBalance(big.Mul(big.NewInt(1000), big.NewInt(1e18)))

        proveCommit := makeProveCommit(sectorNo)
        actor.proveCommitSectorAndConfirm(rt, precommit, proveCommit, proveCommitConf{})
        st := getState(rt)

        // Verify new sectors
        // TODO minerstate
        //newSectors, err := st.NewSectors.All(miner.SectorsMax)
        //require.NoError(t, err)
        //assert_eq!( []uint64{uint64(sectorNo)}, newSectors)
        // Verify pledge lock-up
        assert.True(t, st.initial_pledge.GreaterThan(big.Zero()))
        rt.Reset()

        // Duplicate proof (sector no-longer pre-committed)
        rt.ExpectAbort(exitcode.ErrNotFound, func() {
            actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(sectorNo), proveCommitConf{})
        })
        rt.Reset()
        actor.checkState(rt)
    })

    t.Run("prove commit aborts if pledge requirement not met", func(t *testing.T) {
        actor := newHarness(t, periodOffset)
        rt := builderForHarness(actor).
            WithBalance(bigBalance, big.Zero()).
            Build(t)
        actor.constructAndVerify(rt)
        // Set the circulating supply high and expected reward low in order to coerce
        // pledge requirements (BR + share of money supply, but capped at 1FIL)
        // to exceed pre-commit deposit (BR only).
        rt.SetCirculatingSupply(big.Mul(big.NewInt(100_000_000), big.NewInt(1e18)))
        actor.epochRewardSmooth = smoothing.TestingConstantEstimate(big.NewInt(1e15))

        // prove one sector to establish collateral and locked funds
        sectors := actor.commitAndProveSectors(rt, 1, defaultSectorExpiration, nil, true)

        // preecommit another sector so we may prove it
        expiration := defaultSectorExpiration*miner.WPoStProvingPeriod + periodOffset - 1
        precommitEpoch := rt.Epoch() + 1
        rt.SetEpoch(precommitEpoch)
        params := actor.makePreCommit(actor.nextSectorNo, rt.Epoch()-1, expiration, nil)
        precommit := actor.preCommitSector(rt, params, preCommitConf{}, false)

        // Confirm the unlocked PCD will not cover the new IP
        assert.True(t, sectors[0].initial_pledge.GreaterThan(precommit.PreCommitDeposit))

        // Set balance to exactly cover locked funds.
        st := getState(rt)
        rt.SetBalance(big.Sum(st.PreCommitDeposits, st.initial_pledge, st.LockedFunds))

        rt.SetEpoch(precommitEpoch + miner.MaxProveCommitDuration[actor.sealProofType] - 1)
        rt.ExpectAbort(exitcode.ErrInsufficientFunds, func() {
            actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(actor.nextSectorNo), proveCommitConf{})
        })
        rt.Reset()

        // succeeds with enough free balance (enough to cover 2x IP)
        rt.SetBalance(big.Sum(st.PreCommitDeposits, st.initial_pledge, st.initial_pledge, st.LockedFunds))
        actor.proveCommitSectorAndConfirm(rt, precommit, makeProveCommit(actor.nextSectorNo), proveCommitConf{})
        actor.checkState(rt)
    })

    t.Run("drop invalid prove commit while processing valid one", func(t *testing.T) {
        actor := newHarness(t, periodOffset)
        rt := builderForHarness(actor).
            WithBalance(bigBalance, big.Zero()).
            Build(t)
        actor.constructAndVerify(rt)

        // make two precommits
        expiration := defaultSectorExpiration*miner.WPoStProvingPeriod + periodOffset - 1
        precommitEpoch := rt.Epoch() + 1
        rt.SetEpoch(precommitEpoch)
        paramsA := actor.makePreCommit(actor.nextSectorNo, rt.Epoch()-1, expiration, []abi.DealID{1})
        preCommitA := actor.preCommitSector(rt, paramsA, preCommitConf{}, true)
        sectorNoA := actor.nextSectorNo
        actor.nextSectorNo++
        paramsB := actor.makePreCommit(actor.nextSectorNo, rt.Epoch()-1, expiration, []abi.DealID{2})
        preCommitB := actor.preCommitSector(rt, paramsB, preCommitConf{}, false)
        sectorNoB := actor.nextSectorNo

        // handle both prove commits in the same epoch
        rt.SetEpoch(precommitEpoch + miner.MaxProveCommitDuration[actor.sealProofType] - 1)

        actor.proveCommitSector(rt, preCommitA, makeProveCommit(sectorNoA))
        actor.proveCommitSector(rt, preCommitB, makeProveCommit(sectorNoB))

        conf := proveCommitConf{
            verifyDealsExit: map[abi.SectorNumber]exitcode.ExitCode{
                sectorNoA: exitcode.ErrIllegalArgument,
            },
        }
        actor.confirmSectorProofsValid(rt, conf, preCommitA, preCommitB)
        actor.checkState(rt)
    })

    t.Run("prove commit just after period start permits PoSt", func(t *testing.T) {
        actor := newHarness(t, periodOffset)
        rt := builderForHarness(actor).
            WithBalance(bigBalance, big.Zero()).
            Build(t)

        // Epoch 101 should be at the beginning of the miner's proving period so there will be time to commit
        // and PoSt a sector.
        rt.SetEpoch(101)
        actor.constructAndVerify(rt)

        // Commit a sector the very next epoch
        rt.SetEpoch(102)
        sector := actor.commitAndProveSector(rt, abi.MaxSectorNumber, defaultSectorExpiration, nil)

        // advance cron to activate power.
        advanceAndSubmitPoSts(rt, actor, sector)
        actor.checkState(rt)
    })

    t.Run("sector with non-positive lifetime is skipped in confirmation", func(t *testing.T) {
        actor := newHarness(t, periodOffset)
        rt := builderForHarness(actor).
            WithBalance(bigBalance, big.Zero()).
            Build(t)
        precommitEpoch := periodOffset + 1
        rt.SetEpoch(precommitEpoch)
        actor.constructAndVerify(rt)
        deadline := actor.deadline(rt)

        sectorNo := abi.SectorNumber(100)
        params := actor.makePreCommit(sectorNo, precommitEpoch-1, deadline.PeriodEnd()+defaultSectorExpiration*miner.WPoStProvingPeriod, nil)
        precommit := actor.preCommitSector(rt, params, preCommitConf{}, true)

        // precommit at correct epoch
        rt.SetEpoch(rt.Epoch() + miner.PreCommitChallengeDelay + 1)
        actor.proveCommitSector(rt, precommit, makeProveCommit(sectorNo))

        // confirm at sector expiration (this probably can't happen)
        rt.SetEpoch(precommit.Info.Expiration)
        // sector skipped but no failure occurs
        actor.confirmSectorProofsValid(rt, proveCommitConf{}, precommit)
        rt.ExpectLogsContain("less than minimum. ignoring")

        // it still skips if sector lifetime is negative
        rt.ClearLogs()
        rt.SetEpoch(precommit.Info.Expiration + 1)
        actor.confirmSectorProofsValid(rt, proveCommitConf{}, precommit)
        rt.ExpectLogsContain("less than minimum. ignoring")

        // it fails up to the miniumum expiration
        rt.ClearLogs()
        rt.SetEpoch(precommit.Info.Expiration - miner.MinSectorExpiration + 1)
        actor.confirmSectorProofsValid(rt, proveCommitConf{}, precommit)
        rt.ExpectLogsContain("less than minimum. ignoring")
        actor.checkState(rt)
    })

    t.Run("verify proof does not vest funds", func(t *testing.T) {
        actor := newHarness(t, periodOffset)
        actor.setProofType(abi.RegisteredSealProof_StackedDrg32GiBV1_1)
        rt := builderForHarness(actor).
            WithBalance(bigBalance, big.Zero()).
            Build(t)
        precommitEpoch := periodOffset + 1
        rt.SetEpoch(precommitEpoch)
        actor.constructAndVerify(rt)
        deadline := actor.deadline(rt)

        // Make a good commitment for the proof to target.
        sectorNo := abi.SectorNumber(100)
        params := actor.makePreCommit(sectorNo, precommitEpoch-1, deadline.PeriodEnd()+defaultSectorExpiration*miner.WPoStProvingPeriod, []abi.DealID{1})
        precommit := actor.preCommitSector(rt, params, preCommitConf{}, true)

        // add 1000 tokens that vest immediately
        st := getState(rt)
        _, err := st.AddLockedFunds(rt.AdtStore(), rt.Epoch(), abi.NewTokenAmount(1000), &miner.VestSpec{
            InitialDelay: 0,
            VestPeriod:   1,
            StepDuration: 1,
            Quantization: 1,
        })
        require.NoError(t, err)
        rt.ReplaceState(st)

        // Set the right epoch for all following tests
        rt.SetEpoch(precommitEpoch + miner.PreCommitChallengeDelay + 1)
        rt.SetBalance(big.Mul(big.NewInt(1000), big.NewInt(1e18)))

        proveCommit := makeProveCommit(sectorNo)
        proveCommit.Proof = make([]byte, 192)
        // The below call expects exactly the pledge delta for the proven sector, zero for any other vesting.
        actor.proveCommitSectorAndConfirm(rt, precommit, proveCommit, proveCommitConf{})
    })
} */

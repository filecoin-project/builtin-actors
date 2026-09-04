use fil_actor_miner::{
    Actor, ExpirationExtension2, ExtendSectorExpiration2Params, Method, PoStPartition, SectorClaim,
    SectorOnChainInfo, SectorOnChainInfoFlags, State, daily_proof_fee,
    pledge_penalty_for_termination, power_for_sector, seal_proof_sector_maximum_lifetime,
};
use fil_actors_runtime::{
    EPOCHS_IN_DAY,
    runtime::{Runtime, RuntimePolicy},
    test_utils::{ACCOUNT_ACTOR_CODE_ID, MockRuntime, expect_abort_contains_message},
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::bigint::BigInt;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::piece::PieceInfo;
use fvm_shared::{
    clock::ChainEpoch,
    error::ExitCode,
    sector::{RegisteredSealProof, SectorNumber},
};

use num_traits::Zero;
use std::collections::HashMap;

mod util;

use itertools::Itertools;
use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;
    let precommit_epoch = 1;

    let mut h = ActorHarness::new(period_offset);
    // reduce the partition size
    // if changed to V1P1 the rejects_extension_past_max_for_seal_proof test fails
    h.set_proof_type(RegisteredSealProof::StackedDRG512MiBV1);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

fn commit_sector(h: &mut ActorHarness, rt: &MockRuntime) -> SectorOnChainInfo {
    h.construct_and_verify(rt);

    h.commit_and_prove_sectors(rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, Vec::new(), true)[0]
        .to_owned()
}

#[test]
fn rejects_negative_extensions() {
    let (mut h, rt) = setup();
    let sector = commit_sector(&mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&sector));

    // attempt to shorten epoch
    let new_expiration = sector.expiration - rt.policy().wpost_proving_period;

    // find deadline and partition
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), sector.sector_number).unwrap();

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration,
            sectors_with_claims: vec![],
        }],
    };

    let res = h.extend_sectors2(&rt, params);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("cannot reduce sector {} expiration", sector.sector_number),
        res,
    );
    h.check_state(&rt);
}

// At its expiration epoch a sector is still live, so an unchanged expiration passes the
// expired and reduction checks. An unflagged sector would then reach quality_for_weight with
// zero spacetime.
#[test]
fn rejects_extension_with_no_remaining_duration() {
    let (mut h, rt) = setup();
    let sector = commit_sector(&mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&sector));
    h.rewrite_sectors(&rt, &[sector.sector_number], |sector| {
        sector.flags = SectorOnChainInfoFlags::empty()
    });
    rt.set_epoch(sector.expiration);

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), sector.sector_number).unwrap();
    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration: sector.expiration,
            sectors_with_claims: vec![],
        }],
    };

    // Called directly: the harness precomputes the power delta over the same zero duration.
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());
    let res = rt.call::<Actor>(
        Method::ExtendSectorExpiration2 as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("cannot extend sector {} to {}", sector.sector_number, sector.expiration),
        res,
    );
    rt.verify();
    h.check_state(&rt);
}

/// The deadline index is validated before anything else because
/// `replace_sector_records` indexes a per-deadline vector with it. Out of range must be
/// rejected, not carried through to that index.
#[test]
fn rejects_out_of_range_deadline() {
    let (mut h, rt) = setup();
    let sector = commit_sector(&mut h, &rt);

    let state: State = rt.get_state();
    let (_, partition_index) = state.find_sector(rt.store(), sector.sector_number).unwrap();
    let out_of_range = rt.policy().wpost_period_deadlines;

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: out_of_range,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration: sector.expiration + rt.policy().wpost_proving_period,
            sectors_with_claims: vec![],
        }],
    };

    // The range check precedes caller validation, so this bypasses the harness helper and
    // its expectation of a validate-caller call.
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    let res = rt.call::<Actor>(
        Method::ExtendSectorExpiration2 as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("deadline {} not in range 0..{}", out_of_range, out_of_range),
        res,
    );
    rt.reset();

    assert_eq!(sector, h.get_sector(&rt, sector.sector_number));
    h.check_state(&rt);
}

#[test]
fn rejects_extension_too_far_in_future() {
    let (mut h, rt) = setup();
    let sector = commit_sector(&mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&sector));

    // extend by even proving period after max
    rt.set_epoch(sector.expiration);
    let extension = rt.policy().wpost_proving_period + rt.policy().max_sector_expiration_extension;
    let new_expiration = *rt.epoch.borrow() + extension;

    // find deadline and partition
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), sector.sector_number).unwrap();

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration,
            sectors_with_claims: vec![],
        }],
    };

    let res = h.extend_sectors2(&rt, params);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!(
            "cannot be more than {} past current epoch",
            rt.policy().max_sector_expiration_extension
        ),
        res,
    );
    h.check_state(&rt);
}

#[test]
fn rejects_extension_past_max_for_seal_proof() {
    let (mut h, rt) = setup();
    let mut sector = commit_sector(&mut h, &rt);
    // and prove it once to activate it.
    h.advance_and_submit_posts(&rt, &[sector.clone()]);

    let max_lifetime = seal_proof_sector_maximum_lifetime(sector.seal_proof).unwrap();

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), sector.sector_number).unwrap();

    // extend sector until just below threshold
    rt.set_epoch(sector.expiration);
    let extension = rt.policy().min_sector_expiration;

    let mut expiration = sector.expiration + extension;
    while expiration - sector.activation < max_lifetime {
        let params = ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline: deadline_index,
                partition: partition_index,
                sectors: make_bitfield(&[sector.sector_number]),
                new_expiration: expiration,
                sectors_with_claims: vec![],
            }],
        };
        h.extend_sectors2(&rt, params).unwrap();
        sector.expiration = expiration;

        expiration += extension;
    }

    // next extension fails because it extends sector past max lifetime
    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration: expiration,
            sectors_with_claims: vec![],
        }],
    };

    let res = h.extend_sectors2(&rt, params);
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "total sector lifetime", res);
    h.check_state(&rt);
}

fn expect_not_active(h: &ActorHarness, rt: &MockRuntime, params: ExtendSectorExpiration2Params) {
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "is not active in",
        h.extend_sectors2(rt, params),
    );
    rt.reset();
}

/// Only active sectors can be extended: an unproven, foreign, faulty or terminated sector aborts
/// the call as the caller's error, before the expiration queue is touched.
#[test]
fn rejects_sectors_that_are_not_active() {
    let (mut h, rt) = setup();
    h.construct_and_verify(&rt);
    // Three sectors: two share a partition, the third lands in another.
    let sectors =
        h.commit_and_prove_sectors(&rt, 3, DEFAULT_SECTOR_EXPIRATION as u64, Vec::new(), true);
    let location = |sector: &SectorOnChainInfo| {
        rt.get_state::<State>().find_sector(rt.store(), sector.sector_number).unwrap()
    };
    let (deadline, partition) = location(&sectors[0]);
    assert_eq!((deadline, partition), location(&sectors[1]));
    let (other_deadline, other_partition) = location(&sectors[2]);
    assert_ne!((deadline, partition), (other_deadline, other_partition));
    let new_expiration = sectors[0].expiration + 42 * EPOCHS_IN_DAY;
    let declare = |deadline, partition, sector: &SectorOnChainInfo| ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline,
            partition,
            sectors: make_bitfield(&[sector.sector_number]),
            sectors_with_claims: vec![],
            new_expiration,
        }],
    };

    // Committed but not yet proven in a WindowPoSt.
    expect_not_active(&h, &rt, declare(deadline, partition, &sectors[0]));
    h.advance_and_submit_posts(&rt, &sectors);

    // Held by a partition other than the declared one.
    expect_not_active(&h, &rt, declare(deadline, partition, &sectors[2]));

    // Faulty.
    h.declare_faults(&rt, std::slice::from_ref(&sectors[0]));
    expect_not_active(&h, &rt, declare(deadline, partition, &sectors[0]));

    // Terminated, its record still in place until cleanup. Locked rewards make the fee
    // unlockable, as in the terminate tests.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let fault_fee = h.continued_fault_penalty(std::slice::from_ref(&sectors[1]));
    let termination_fee = pledge_penalty_for_termination(
        &sectors[1].initial_pledge,
        *rt.epoch.borrow() - sectors[1].activation,
        &fault_fee,
    );
    h.terminate_sectors(&rt, &make_bitfield(&[sectors[1].sector_number]), termination_fee);
    expect_not_active(&h, &rt, declare(deadline, partition, &sectors[1]));

    // The sector rejected as foreign extends normally under its own partition.
    h.extend_sectors2(&rt, declare(other_deadline, other_partition, &sectors[2])).unwrap();
    check_for_expiration(
        &mut h,
        &rt,
        new_expiration,
        sectors[2].sector_number,
        other_deadline,
        other_partition,
    );
}

#[test]
fn updates_expiration_with_valid_params() {
    let (mut h, rt) = setup();

    let old_sector = commit_sector(&mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&old_sector));

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.expiration + extension;

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sector.sector_number]),
            new_expiration,
            sectors_with_claims: vec![],
        }],
    };

    // Change the circulating supply so we can detect fee changes (that shouldn't happen).
    rt.set_circulating_supply(rt.total_fil_circ_supply() * 2);

    h.extend_sectors2(&rt, params).unwrap();

    // assert sector expiration is set to the new value
    let new_sector = h.get_sector(&rt, old_sector.sector_number);
    assert_eq!(new_expiration, new_sector.expiration);

    // assert that the fee hasn't changed
    assert_eq!(old_sector.daily_fee, new_sector.daily_fee);
    let deadline = h.get_deadline(&rt, deadline_index);
    assert_eq!(new_sector.daily_fee, deadline.daily_fee);

    let quant = state.quant_spec_for_deadline(rt.policy(), deadline_index);

    // assert that new expiration exists
    let (_, mut partition) = h.get_deadline_and_partition(&rt, deadline_index, partition_index);
    let expiration_set =
        partition.pop_expired_sectors(rt.store(), new_expiration - 1, quant).unwrap();
    assert!(expiration_set.is_empty());

    let expiration_set = partition
        .pop_expired_sectors(rt.store(), quant.quantize_up(new_expiration), quant)
        .unwrap();
    assert_eq!(expiration_set.len(), 1);
    assert!(expiration_set.on_time_sectors.get(old_sector.sector_number));

    h.check_state(&rt);
}

#[test]
fn updates_expiration_and_daily_fee() {
    // Start with sectors that have a zero fee (i.e. indicating they are pre-FIP-0100). Two sectors
    // for both cases, but we will make the second sector fully verified to test the fee
    // calculation.

    let (mut h, rt) = setup();

    // Common setup
    h.construct_and_verify(&rt);
    rt.set_circulating_supply(TokenAmount::zero());

    // Create deal for v2 cases
    let deal = PieceInfo { size: PaddedPieceSize(h.sector_size as u64), cid: Default::default() };

    // Configure sectors
    let sector_pieces = vec![vec![], vec![1]];
    let activated_deals = HashMap::from_iter(vec![(1, vec![deal.clone()])]);
    // Commit sectors
    let config = ProveCommitConfig {
        verify_deals_exit: Default::default(),
        claim_allocs_exit: Default::default(),
        activated_deals,
    };

    let old_sectors = h.commit_and_prove_sectors_with_cfgs(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION as u64,
        sector_pieces,
        true,
        config,
    );
    h.advance_and_submit_posts(&rt, &old_sectors);

    // Verify initial state (fees should be zero)
    for sector in &old_sectors {
        assert_eq!(
            sector.daily_fee,
            TokenAmount::zero(),
            "expected sector's daily fee to be zero because the circulating supply is zero"
        );
    }
    h.advance_and_submit_posts(&rt, &old_sectors);

    // Prepare extension parameters
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), old_sectors[0].sector_number).unwrap();
    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sectors[0].expiration + extension;

    assert!(h.get_deadline(&rt, deadline_index).daily_fee.is_zero());

    // Set circulating supply to trigger fee calculation
    let new_circulating_supply = TokenAmount::from_whole(500_000_000);
    rt.set_circulating_supply(new_circulating_supply.clone());

    // Extend sectors
    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sectors[0].sector_number]),
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sectors[1].sector_number,
                maintain_claims: vec![1],
                drop_claims: vec![],
            }],
            new_expiration,
        }],
    };
    h.extend_sectors2(&rt, params).unwrap();

    let new_sectors = old_sectors.iter().map(|s| h.get_sector(&rt, s.sector_number)).collect_vec();

    // Verify expirations
    for sector in &new_sectors {
        assert_eq!(new_expiration, sector.expiration);
    }

    // Extending a pre-FIP-0100 sector attaches its first fee.
    let full_verified_fee = daily_proof_fee(
        &rt.policy,
        &rt.circulating_supply.borrow(),
        &BigInt::from(h.sector_size as u64 * 10),
    );
    let total_fee = new_sectors[0].daily_fee.clone() + new_sectors[1].daily_fee.clone();

    // FIP-0118: both sectors have FULL_QA_POWER, so both get full verified fee
    assert_eq!(full_verified_fee, new_sectors[0].daily_fee);
    assert_eq!(full_verified_fee, new_sectors[1].daily_fee);

    let (deadline, partition) = h.get_deadline_and_partition(&rt, deadline_index, partition_index);
    // Deadline has the two fees
    assert_eq!(total_fee, deadline.daily_fee);

    // Partition expiration queue has the total fee as a deduction
    let quant = h.get_state(&rt).quant_spec_for_deadline(&rt.policy, deadline_index);
    let quantized_expiration = quant.quantize_up(new_sectors[0].expiration);
    let p_queue = h.collect_partition_expirations(&rt, &partition);
    let entry = p_queue.get(&quantized_expiration).cloned().unwrap();
    assert_eq!(total_fee, entry.fee_deduction);

    h.check_state(&rt);
}

#[test]
fn updates_many_sectors() {
    let (mut h, rt) = setup();
    h.construct_and_verify(&rt);

    let sector_count = 4;

    // commit a bunch of sectors to ensure that we get multiple partitions
    let sector_infos = h.commit_and_prove_sectors(
        &rt,
        sector_count as usize,
        DEFAULT_SECTOR_EXPIRATION as u64,
        Vec::new(),
        true,
    );
    h.advance_and_submit_posts(&rt, &sector_infos);

    let new_expiration = sector_infos[0].expiration + 42 * rt.policy().wpost_proving_period;
    let mut extensions: Vec<ExpirationExtension2> = Vec::new();

    let state: State = rt.get_state();
    let deadlines = state.load_deadlines(rt.store()).unwrap();
    deadlines
        .for_each(rt.store(), |deadline_index, deadline| {
            let partitions = deadline.partitions_amt(rt.store()).unwrap();
            partitions
                .for_each(|partition_index, partition| {
                    // filter out even-numbered sectors
                    let sectors = partition
                        .sectors
                        .bounded_iter(rt.policy().addressed_sectors_max)
                        .unwrap()
                        .filter(|n| n % 2 != 0)
                        .collect_vec();

                    extensions.push(ExpirationExtension2 {
                        deadline: deadline_index,
                        partition: partition_index,
                        sectors: make_bitfield(&sectors),
                        new_expiration,
                        sectors_with_claims: vec![],
                    });

                    Ok(())
                })
                .unwrap();
            Ok(())
        })
        .unwrap();

    // make sure we're touching at least two partitions
    assert!(extensions.len() >= 2, "test error: this test should touch more than one partition");
    let params = ExtendSectorExpiration2Params { extensions };

    h.extend_sectors2(&rt, params).unwrap();
    let state: State = rt.get_state();
    let deadlines = state.load_deadlines(rt.store()).unwrap();

    // half of the sectors should expire on-time
    let (mut on_time_total, mut extended_total) = (0, 0);
    deadlines
        .for_each(rt.store(), |deadline_index, mut deadline| {
            let expiration_set = deadline
                .pop_expired_sectors(
                    rt.store(),
                    new_expiration - 1,
                    state.quant_spec_for_deadline(rt.policy(), deadline_index),
                )
                .unwrap();
            on_time_total += expiration_set.len();

            // ensure we pop queue up to the deadline-specific expiration
            let quant = state.quant_spec_for_deadline(rt.policy(), deadline_index);
            let expiration_set = deadline
                .pop_expired_sectors(rt.store(), quant.quantize_up(new_expiration), quant)
                .unwrap();
            extended_total += expiration_set.len();
            Ok(())
        })
        .unwrap();
    assert_eq!(sector_count / 2, on_time_total);
    assert_eq!(sector_count / 2, extended_total);

    h.check_state(&rt);
}

#[test]
fn supports_extensions_off_deadline_boundary() {
    let (mut h, rt) = setup();
    let old_sector = commit_sector(&mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&old_sector));

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period + rt.policy().wpost_proving_period / 3;
    let new_expiration = old_sector.expiration + extension;

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sector.sector_number]),
            new_expiration,
            sectors_with_claims: vec![],
        }],
    };

    h.extend_sectors2(&rt, params).unwrap();

    // assert sector expiration is set to the new value
    let mut state: State = rt.get_state();
    let new_sector = h.get_sector(&rt, old_sector.sector_number);
    assert_eq!(new_expiration, new_sector.expiration);

    // advance clock to expiration
    rt.set_epoch(new_sector.expiration);
    state.proving_period_start += rt.policy().wpost_proving_period
        * ((*rt.epoch.borrow() - state.proving_period_start) / rt.policy().wpost_proving_period
            + 1);
    rt.replace_state(&state);

    // confirm it is not in sector's deadline to make sure we're testing extensions off the deadline boundary"
    let deadline_info = h.deadline(&rt);
    assert_ne!(deadline_index, deadline_info.index);

    // advance to deadline and submit one last PoSt
    let deadline_info = h.advance_to_deadline(&rt, deadline_index);

    let partitions = vec![PoStPartition { index: partition_index, skipped: BitField::default() }];
    h.submit_window_post(
        &rt,
        &deadline_info,
        partitions,
        vec![new_sector.clone()],
        PoStConfig::empty(),
    );

    // advance one more time. No missed PoSt fees are charged. Total power and pledge are lowered.
    let power = -power_for_sector(h.sector_size, &new_sector);
    let mut cron_config = CronConfig::empty();
    cron_config.no_enrollment = true;
    cron_config.power_delta = Some(power);
    cron_config.pledge_delta = -new_sector.initial_pledge;

    h.advance_deadline(&rt, cron_config);

    let state: State = rt.get_state();
    assert!(!state.deadline_cron_active);

    h.check_state(&rt);
}

#[test]
fn update_expiration2_multiple_claims() {
    let (mut h, rt) = setup();
    // add in verified deal
    let verified_deals = vec![
        test_activated_deal(h.sector_size as u64 / 2),
        test_activated_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&old_sector));

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.expiration + extension;

    let claim_ids = vec![400, 500];

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration,
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sector.sector_number,
                maintain_claims: claim_ids,
                drop_claims: vec![],
            }],
        }],
    };

    h.extend_sectors2(&rt, params).unwrap();

    // assert sector expiration is set to the new value
    check_for_expiration(
        &mut h,
        &rt,
        new_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );

    // fee should not have changed
    let new_sector = h.get_sector(&rt, old_sector.sector_number);
    assert_eq!(old_sector.daily_fee, new_sector.daily_fee);

    let (deadline, partition) = h.get_deadline_and_partition(&rt, deadline_index, partition_index);

    // deadline has the fee
    assert_eq!(new_sector.daily_fee, deadline.daily_fee);

    // partition expiration queue has the fee as a deduction
    let quant = h.get_state(&rt).quant_spec_for_deadline(&rt.policy, deadline_index);
    let quantized_expiration = quant.quantize_up(new_sector.expiration);
    let p_queue = h.collect_partition_expirations(&rt, &partition);
    let entry = p_queue.get(&quantized_expiration).cloned().unwrap();
    assert_eq!(new_sector.daily_fee, entry.fee_deduction);
}

#[test]
fn update_expiration2_failure_cases() {
    // FIP-0118: claim validation has been removed from extension. sectors_with_claims
    // references are now ignored (sector numbers are just merged into the sectors bitfield).
    // Extensions that previously failed due to claim validation now succeed.
    let (mut h, rt) = setup();
    let verified_deals = vec![
        test_activated_deal(h.sector_size as u64 / 2),
        test_activated_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&old_sector));

    let (deadline_index, partition_index) =
        rt.get_state::<State>().find_sector(rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.expiration + extension;

    // Extension with claim references now succeeds (claims are ignored)
    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration,
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sector.sector_number,
                maintain_claims: vec![400],
                drop_claims: vec![],
            }],
        }],
    };
    h.extend_sectors2(&rt, params).unwrap();
    check_for_expiration(
        &mut h,
        &rt,
        new_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );
}

#[test]
fn extend_expiration2_drop_claims() {
    // FIP-0118: claim validation and dropping has been removed from extensions.
    // sectors_with_claims references are ignored; all sectors get FULL_QA_POWER (10x).
    let (mut h, rt) = setup();
    let verified_deals = vec![
        test_activated_deal(h.sector_size as u64 / 2),
        test_activated_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &rt);
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), old_sector.sector_number).unwrap();

    // Piece spacetime is recorded in verified_deal_weight; deal_weight is unused.
    assert!(old_sector.deal_weight.is_zero());
    assert!(!old_sector.verified_deal_weight.is_zero());

    {
        // sanity check deadline and partition state is correct for original sector's fees
        let (deadline, partition) =
            h.get_deadline_and_partition(&rt, deadline_index, partition_index);

        // deadline has the fee
        assert_eq!(old_sector.daily_fee, deadline.daily_fee);

        // partition expiration queue has the fee as a deduction
        let quant = h.get_state(&rt).quant_spec_for_deadline(&rt.policy, deadline_index);
        let quantized_expiration = quant.quantize_up(old_sector.expiration);
        let p_queue = h.collect_partition_expirations(&rt, &partition);
        let entry = p_queue.get(&quantized_expiration).cloned().unwrap();
        assert_eq!(old_sector.daily_fee, entry.fee_deduction);
    }

    h.advance_and_submit_posts(&rt, std::slice::from_ref(&old_sector));

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.expiration + extension;
    let second_expiration = new_expiration + 42 * EPOCHS_IN_DAY;

    // Extension with claim references succeeds (claims are ignored by FIP-0118)
    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration,
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sector.sector_number,
                maintain_claims: vec![400],
                drop_claims: vec![500],
            }],
        }],
    };
    h.extend_sectors2(&rt, params).unwrap();
    check_for_expiration(
        &mut h,
        &rt,
        new_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );

    let new_sector = h.get_sector(&rt, old_sector.sector_number);

    // FIP-0118: FULL_QA_POWER sectors keep same fee (no claim-based adjustments)
    // Fee doesn't change since QA power doesn't change (already at 10x)
    assert_eq!(old_sector.daily_fee, new_sector.daily_fee);

    {
        // check the deadline and partition state is correct for the replaced sector's fee
        let (deadline, partition) =
            h.get_deadline_and_partition(&rt, deadline_index, partition_index);
        assert_eq!(new_sector.daily_fee, deadline.daily_fee);

        let quant = h.get_state(&rt).quant_spec_for_deadline(&rt.policy, deadline_index);
        let quantized_expiration = quant.quantize_up(new_sector.expiration);
        let p_queue = h.collect_partition_expirations(&rt, &partition);
        let entry = p_queue.get(&quantized_expiration).cloned().unwrap();
        assert_eq!(new_sector.daily_fee, entry.fee_deduction);
    }

    // Extending again also succeeds (no claim validation)
    let params2 = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: bitfield_from_slice(&[old_sector.sector_number]),
            new_expiration: second_expiration,
            sectors_with_claims: vec![],
        }],
    };
    h.extend_sectors2(&rt, params2).unwrap();
    check_for_expiration(
        &mut h,
        &rt,
        second_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );
}

#[test]
fn update_expiration2_drop_claims_failure_cases() {
    // FIP-0118: claim validation and the end_of_life_claim_drop_period constraint have been
    // removed from extensions. Extensions with drop_claims now succeed regardless of timing.
    let (mut h, rt) = setup();
    let verified_deals = vec![
        test_activated_deal(h.sector_size as u64 / 2),
        test_activated_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &rt);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&old_sector));

    let (deadline_index, partition_index) =
        rt.get_state::<State>().find_sector(rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.expiration + extension;

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration,
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sector.sector_number,
                maintain_claims: vec![400],
                drop_claims: vec![500],
            }],
        }],
    };

    // Extension now succeeds since claim validation was removed
    h.extend_sectors2(&rt, params).unwrap();
    check_for_expiration(
        &mut h,
        &rt,
        new_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );
}

fn commit_sector_verified_deals(
    verified_deals: &[PieceInfo],
    h: &mut ActorHarness,
    rt: &MockRuntime,
) -> SectorOnChainInfo {
    h.construct_and_verify(rt);
    assert!(!verified_deals.is_empty());

    let mut pcc = ProveCommitConfig::empty();
    pcc.add_activated_deals(h.next_sector_no, verified_deals.to_owned());

    let mut deal_ids: Vec<DealID> = vec![];
    for i in 0..verified_deals.len() {
        deal_ids.push(i as u64);
    }

    let sector_info = &h.commit_and_prove_sectors_with_cfgs(
        rt,
        1,
        DEFAULT_SECTOR_EXPIRATION as u64,
        vec![deal_ids],
        true,
        pcc,
    )[0];

    sector_info.clone()
}

// assert that state tracks an expiration at the provided epoch in the provided deadline and partition for the provided sector
fn check_for_expiration(
    h: &mut ActorHarness,
    rt: &MockRuntime,
    expiration: ChainEpoch,
    sector_number: SectorNumber,
    deadline_index: u64,
    partition_index: u64,
) {
    let new_sector = h.get_sector(rt, sector_number);
    assert_eq!(expiration, new_sector.expiration);
    let state: State = rt.get_state();
    let quant = state.quant_spec_for_deadline(rt.policy(), deadline_index);

    // assert that new expiration exists
    let (_, mut partition) = h.get_deadline_and_partition(rt, deadline_index, partition_index);
    let expiration_set = partition.pop_expired_sectors(rt.store(), expiration - 1, quant).unwrap();
    assert!(expiration_set.is_empty());

    let expiration_set =
        partition.pop_expired_sectors(rt.store(), quant.quantize_up(expiration), quant).unwrap();
    assert_eq!(expiration_set.len(), 1);
    assert!(expiration_set.on_time_sectors.get(sector_number));

    h.check_state(rt);
}

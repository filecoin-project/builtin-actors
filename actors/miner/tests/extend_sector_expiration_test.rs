use anyhow::Chain;
use fil_actor_market::VerifiedDealInfo;
use fil_actor_miner::ext::verifreg::Claim as FILPlusClaim;
use fil_actor_miner::{
    power_for_sector, seal_proof_sector_maximum_lifetime, ExpirationExtension,
    ExpirationExtension2, ExtendSectorExpiration2Params, ExtendSectorExpirationParams,
    PoStPartition, SectorClaim, SectorOnChainInfo, State,
};
use fil_actors_runtime::DealWeight;
use fil_actors_runtime::{
    actor_error,
    runtime::{Runtime, RuntimePolicy},
    test_utils::{expect_abort_contains_message, make_piece_cid, MockRuntime},
    EPOCHS_IN_DAY,
};
use fvm_ipld_bitfield::BitField;
use fvm_shared::{
    address::Address,
    clock::ChainEpoch,
    error::ExitCode,
    sector::{RegisteredSealProof, SectorNumber},
    ActorID,
};
use std::collections::HashMap;

mod util;
use fil_actors_runtime::runtime::Policy;
use itertools::Itertools;
use test_case::test_case;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;
    let precommit_epoch = 1;

    let mut h = ActorHarness::new(period_offset);
    // reduce the partition size
    // if changed to V1P1 the rejects_extension_past_max_for_seal_proof test fails
    h.set_proof_type(RegisteredSealProof::StackedDRG512MiBV1P1);
    let mut rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

fn commit_sector(h: &mut ActorHarness, rt: &mut MockRuntime) -> SectorOnChainInfo {
    h.construct_and_verify(rt);

    h.commit_and_prove_sectors(rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, Vec::new(), true)[0]
        .to_owned()
}

#[test]
fn proof_extension_happy_path() {
    let (mut h, mut rt) = setup();
    let sector = commit_sector(&mut h, &mut rt);
    // and prove it once to activate it.
    h.advance_and_submit_posts(&mut rt, &vec![sector.clone()]);

    // extend the sector so that it now expires early from proof expiration
    let new_expiration = sector.proof_expiration + 1;

    // advance in time so extension passes policy limits
    if rt.epoch + rt.policy().max_sector_commitment_extension < new_expiration {
        rt.set_epoch(new_expiration - rt.policy().max_sector_commitment_extension);
    }
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), sector.sector_number).unwrap();
    
    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration,
        }],
    };

    h.extend_sectors_versioned(&mut rt, params, true).unwrap();
    h.assert_queue_state(&mut rt, sector.sector_number, deadline_index, partition_index, sector.proof_expiration, ExpirationKind::Proof);

    // extend proof expiration so that it now expires on time
    if rt.epoch < sector.proof_expiration - rt.policy().proof_refresh_window {
        rt.set_epoch(sector.proof_expiration - rt.policy().proof_refresh_window);
    }
    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration: 0, 
        }],
    };
    h.refresh_proof_expiration(& mut rt, params).unwrap();
    h.assert_queue_state(&mut rt, sector.sector_number, deadline_index, partition_index, new_expiration, ExpirationKind::OnTime);

    let new_sector = h.get_sector(&rt, sector.sector_number);
    assert_eq!(new_sector.proof_expiration, sector.proof_expiration + rt.policy().max_proof_validity - rt.policy().proof_refresh_window);

    // check that it expires happily with no termination fee 

    // Go to start of expiration deadline
    let st = h.get_state(&rt);
    let q_exp = st.quant_spec_for_deadline(rt.policy(), deadline_index).quantize_up(new_expiration);
    let exp_dlinfo = st.deadline_info(rt.policy(), q_exp);
    rt.set_epoch(exp_dlinfo.open);

    // PoSt for this deadline so we're expiring without faulting
    let partitions = vec![PoStPartition { index: partition_index, skipped: BitField::default() }];
    h.submit_window_post(
        &mut rt,
        &exp_dlinfo,
        partitions,
        vec![new_sector.clone()],
        PoStConfig::empty(),
    );

    // Handle proving deadline. No missed PoSt fees are charged. Total power and pledge are lowered.
    let power = -power_for_sector(h.sector_size, &new_sector);
    let mut cron_config = CronConfig::empty();
    cron_config.no_enrollment = true;
    cron_config.expired_sectors_power_delta = Some(power);
    cron_config.expired_sectors_pledge_delta = -new_sector.initial_pledge;
    rt.set_epoch(exp_dlinfo.last());

    h.on_deadline_cron(&mut rt, cron_config);

    h.check_state(&rt);

}

#[test]
fn proof_extension_validation_checks() {
    // expiration must be set to 0

    // can't extend invalid proof types (maybe too hard to test)

    // can't extend beyond policy limits into the future

    // faulty sector cannot have proof extension refreshed
}

#[test]
fn proof_extension_early_sector_pays_fee() {
    let (mut h, mut rt) = setup();
    let sector = commit_sector(&mut h, &mut rt);
    // and prove it once to activate it.
    h.advance_and_submit_posts(&mut rt, &vec![sector.clone()]);

    // extend the sector so that it now expires early from proof expiration
    let new_expiration = sector.proof_expiration + 1;

    // advance in time so extension passes policy limits
    if rt.epoch + rt.policy().max_sector_commitment_extension < new_expiration {
        rt.set_epoch(new_expiration - rt.policy().max_sector_commitment_extension);
    }
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), sector.sector_number).unwrap();
    
    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration,
        }],
    };

    h.extend_sectors_versioned(&mut rt, params, true).unwrap();
    h.assert_queue_state(&mut rt, sector.sector_number, deadline_index, partition_index, sector.proof_expiration, ExpirationKind::Proof);

    let new_sector = h.get_sector(&rt, sector.sector_number);

    // check that it expires early with termination fee

    // Go to start of expiration deadline
    let st = h.get_state(&rt);
    let q_exp = st.quant_spec_for_deadline(rt.policy(), deadline_index).quantize_up(sector.proof_expiration);
    let exp_dlinfo = st.deadline_info(rt.policy(), q_exp);
    rt.set_epoch(exp_dlinfo.open);

    // PoSt for this deadline so we're expiring without faulting
    let partitions = vec![PoStPartition { index: partition_index, skipped: BitField::default() }];
    h.submit_window_post(
        &mut rt,
        &exp_dlinfo,
        partitions,
        vec![new_sector.clone()],
        PoStConfig::empty(),
    );

    // Handle proving deadline. No missed PoSt fees are charged. Termination fee charged. Total power and pledge are lowered.
    let power = -power_for_sector(h.sector_size, &new_sector);
    let mut cron_config = CronConfig::empty();
    cron_config.no_enrollment = true;
    cron_config.expired_sectors_power_delta = Some(power);
    cron_config.expired_sectors_pledge_delta = -new_sector.initial_pledge;
    rt.set_epoch(exp_dlinfo.last());

    h.on_deadline_cron(&mut rt, cron_config);

    h.check_state(&rt);


    // extend the sector so that it now expires early from proof expiration

    // check that it expires with fee and state is correct
}
#[test]
fn fault_proof_expiring_sector() {
    // extend sector so it now expires early from proof expiration

    // fault

    // recover

    // fault again and let it terminate with fault fees 
}




#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn rejects_negative_extensions(v2: bool) {
    let (mut h, mut rt) = setup();
    let sector = commit_sector(&mut h, &mut rt);

    // attempt to shorten epoch
    let new_expiration = sector.commitment_expiration - rt.policy().wpost_proving_period;

    // find deadline and partition
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), sector.sector_number).unwrap();

    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration,
        }],
    };

    let res = h.extend_sectors_versioned(&mut rt, params, v2);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("cannot reduce sector {} expiration", sector.sector_number),
        res,
    );
    h.check_state(&rt);
}

#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn rejects_extension_too_far_in_future(v2: bool) {
    let (mut h, mut rt) = setup();
    let sector = commit_sector(&mut h, &mut rt);

    // extend by even proving period after max
    rt.set_epoch(sector.commitment_expiration);
    let extension = rt.policy().wpost_proving_period + rt.policy().max_sector_commitment_extension;
    let new_expiration = rt.epoch + extension;

    // find deadline and partition
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), sector.sector_number).unwrap();

    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration,
        }],
    };

    let res = h.extend_sectors_versioned(&mut rt, params, v2);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!(
            "cannot be more than {} past current epoch",
            rt.policy().max_sector_commitment_extension
        ),
        res,
    );
    h.check_state(&rt);
}

#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn rejects_extension_past_max_for_seal_proof(v2: bool) {
    let (mut h, mut rt) = setup();
    let mut sector = commit_sector(&mut h, &mut rt);
    // and prove it once to activate it.
    h.advance_and_submit_posts(&mut rt, &vec![sector.clone()]);

    let max_lifetime = seal_proof_sector_maximum_lifetime(sector.seal_proof).unwrap();

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), sector.sector_number).unwrap();
    // extend sector until just below threshold
    rt.set_epoch(sector.commitment_expiration);
    let extension = rt.policy().min_sector_commitment;

    let mut expiration = sector.commitment_expiration + extension;

    while expiration - sector.activation < max_lifetime {
        let params = ExtendSectorExpirationParams {
            extensions: vec![ExpirationExtension {
                deadline: deadline_index,
                partition: partition_index,
                sectors: make_bitfield(&[sector.sector_number]),
                new_expiration: expiration,
            }],
        };
        h.extend_sectors(&mut rt, params.clone()).unwrap();
        sector.commitment_expiration = expiration;
        expiration += extension;
        rt.set_epoch(rt.epoch + extension);

        if sector.commitment_expiration > sector.proof_expiration {
            // advance to proof refresh window if needed 
            if rt.epoch < sector.proof_expiration - rt.policy().proof_refresh_window {
                rt.set_epoch(sector.proof_expiration - rt.policy().proof_refresh_window)
            }
            let refresh_params = ExtendSectorExpirationParams {
                extensions: vec![ExpirationExtension {
                    deadline: deadline_index,
                    partition: partition_index,
                    sectors: make_bitfield(&[sector.sector_number]),
                    new_expiration: 0,
                }],
            };
        
            h.refresh_proof_expiration(&mut rt, refresh_params).unwrap();
            sector.proof_expiration = sector.proof_expiration + (rt.policy().max_proof_validity - rt.policy().proof_refresh_window);
        }
        h.assert_queue_state(&mut rt, sector.sector_number, deadline_index, partition_index, sector.commitment_expiration, ExpirationKind::OnTime);
    }

    // next extension fails because it extends sector past max lifetime
    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector.sector_number]),
            new_expiration: expiration,
        }],
    };

    let res = h.extend_sectors_versioned(&mut rt, params, v2);
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "total sector lifetime", res);
    h.check_state(&rt);
}

#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn updates_expiration_with_valid_params(v2: bool) {
    let (mut h, mut rt) = setup();
    let old_sector = commit_sector(&mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.commitment_expiration + extension;

    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sector.sector_number]),
            new_expiration,
        }],
    };

    h.extend_sectors_versioned(&mut rt, params, v2).unwrap();

    // assert sector expiration is set to the new value
    let new_sector = h.get_sector(&rt, old_sector.sector_number);
    assert_eq!(new_expiration, new_sector.commitment_expiration);

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

#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn updates_many_sectors(v2: bool) {
    let (mut h, mut rt) = setup();
    h.construct_and_verify(&mut rt);

    let sector_count = 4;

    // commit a bunch of sectors to ensure that we get multiple partitions
    let sector_infos = h.commit_and_prove_sectors(
        &mut rt,
        sector_count as usize,
        DEFAULT_SECTOR_EXPIRATION as u64,
        Vec::new(),
        true,
    );
    h.advance_and_submit_posts(&mut rt, &sector_infos);

    let new_expiration =
        sector_infos[0].commitment_expiration + 42 * rt.policy().wpost_proving_period;
    let mut extensions: Vec<ExpirationExtension> = Vec::new();

    let state: State = rt.get_state();
    let deadlines = state.load_deadlines(rt.store()).unwrap();
    deadlines
        .for_each(rt.policy(), rt.store(), |deadline_index, deadline| {
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

                    extensions.push(ExpirationExtension {
                        deadline: deadline_index,
                        partition: partition_index,
                        sectors: make_bitfield(&sectors),
                        new_expiration,
                    });

                    Ok(())
                })
                .unwrap();
            Ok(())
        })
        .unwrap();

    // make sure we're touching at least two partitions
    assert!(extensions.len() >= 2, "test error: this test should touch more than one partition");
    let params = ExtendSectorExpirationParams { extensions };

    h.extend_sectors_versioned(&mut rt, params, v2).unwrap();
    let state: State = rt.get_state();
    let deadlines = state.load_deadlines(rt.store()).unwrap();

    // half of the sectors should expire on-time
    let (mut on_time_total, mut extended_total) = (0, 0);
    deadlines
        .for_each(rt.policy(), rt.store(), |deadline_index, mut deadline| {
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

#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn supports_extensions_off_deadline_boundary(v2: bool) {
    let (mut h, mut rt) = setup();
    let old_sector = commit_sector(&mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period + rt.policy().wpost_proving_period / 3;
    let new_expiration = old_sector.commitment_expiration + extension;

    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sector.sector_number]),
            new_expiration,
        }],
    };

    h.extend_sectors_versioned(&mut rt, params, v2).unwrap();

    // assert sector expiration is set to the new value
    let mut state: State = rt.get_state();
    let new_sector = h.get_sector(&rt, old_sector.sector_number);
    assert_eq!(new_expiration, new_sector.commitment_expiration);

    // advance clock to expiration
    rt.set_epoch(new_sector.commitment_expiration);
    state.proving_period_start += rt.policy().wpost_proving_period
        * ((rt.epoch - state.proving_period_start) / rt.policy().wpost_proving_period + 1);
    rt.replace_state(&state);

    // confirm it is not in sector's deadline to make sure we're testing extensions off the deadline boundary"
    let deadline_info = h.deadline(&rt);
    assert_ne!(deadline_index, deadline_info.index);

    // advance to deadline and submit one last PoSt
    let deadline_info = h.advance_to_deadline(&mut rt, deadline_index);

    let partitions = vec![PoStPartition { index: partition_index, skipped: BitField::default() }];
    h.submit_window_post(
        &mut rt,
        &deadline_info,
        partitions,
        vec![new_sector.clone()],
        PoStConfig::empty(),
    );

    // advance one more time. No missed PoSt fees are charged. Total power and pledge are lowered.
    let power = -power_for_sector(h.sector_size, &new_sector);
    let mut cron_config = CronConfig::empty();
    cron_config.no_enrollment = true;
    cron_config.expired_sectors_power_delta = Some(power);
    cron_config.expired_sectors_pledge_delta = -new_sector.initial_pledge;

    h.advance_deadline(&mut rt, cron_config);

    let state: State = rt.get_state();
    assert!(!state.deadline_cron_active);

    h.check_state(&rt);
}

#[test]
fn update_expiration2_multiple_claims() {
    let (mut h, mut rt) = setup();
    // add in verified deal
    let verified_deals = vec![
        test_verified_deal(h.sector_size as u64 / 2),
        test_verified_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.commitment_expiration + extension;

    let claim_ids = vec![400, 500];
    let client = Address::new_id(3000).id().unwrap();

    let claim0 = make_claim(
        claim_ids[0],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        new_expiration,
        &verified_deals[0],
        rt.policy.minimum_verified_allocation_term,
    );
    let claim1 = make_claim(
        claim_ids[1],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        new_expiration,
        &verified_deals[1],
        rt.policy.minimum_verified_allocation_term,
    );

    let mut claims = HashMap::new();
    claims.insert(claim_ids[0], Ok(claim0));
    claims.insert(claim_ids[1], Ok(claim1));

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

    h.extend_sectors2(&mut rt, params, claims).unwrap();

    // assert sector expiration is set to the new value
    check_for_expiration(
        &mut h,
        &mut rt,
        new_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );
}

#[test]
fn update_expiration2_failure_cases() {
    let (mut h, mut rt) = setup();
    // add in verified deal
    let verified_deals = vec![
        test_verified_deal(h.sector_size as u64 / 2),
        test_verified_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.commitment_expiration + extension;

    let claim_ids = vec![400, 500];
    let client = Address::new_id(3000).id().unwrap();

    let claim0 = make_claim(
        claim_ids[0],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        new_expiration,
        &verified_deals[0],
        rt.policy.minimum_verified_allocation_term,
    );
    let mut claim1 = make_claim(
        claim_ids[1],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        new_expiration,
        &verified_deals[1],
        rt.policy.minimum_verified_allocation_term,
    );

    /* 1. Claim used by sector not referenced in extension params */
    {
        let mut claims = HashMap::new();
        claims.insert(claim_ids[0], Ok(claim0.clone()));

        let params = ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline: deadline_index,
                partition: partition_index,
                sectors: BitField::new(),
                new_expiration,
                sectors_with_claims: vec![SectorClaim {
                    sector_number: old_sector.sector_number,
                    maintain_claims: vec![claim_ids[0]],
                    drop_claims: vec![],
                }],
            }],
        };

        let res = h.extend_sectors2(&mut rt, params, claims);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "does not match verified deal space",
            res,
        );
        // assert sector expiration is same as the old value
        check_for_expiration(
            &mut h,
            &mut rt,
            old_sector.commitment_expiration,
            old_sector.sector_number,
            deadline_index,
            partition_index,
        );
        rt.replace_state(&state);
        rt.reset();
    }

    /* Claim not found */
    {
        let mut claims = HashMap::new();
        claims.insert(claim_ids[0], Ok(claim0.clone()));
        claims.insert(claim_ids[1], Err(actor_error!(not_found, "claim not found")));

        let params = ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline: deadline_index,
                partition: partition_index,
                sectors: BitField::new(),
                new_expiration,
                sectors_with_claims: vec![SectorClaim {
                    sector_number: old_sector.sector_number,
                    maintain_claims: claim_ids.clone(),
                    drop_claims: vec![],
                }],
            }],
        };

        let res = h.extend_sectors2(&mut rt, params, claims);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "failed to get claims for sector",
            res,
        );
        // assert sector expiration is set to the new value
        check_for_expiration(
            &mut h,
            &mut rt,
            old_sector.commitment_expiration,
            old_sector.sector_number,
            deadline_index,
            partition_index,
        );
        rt.replace_state(&state);
        rt.reset();
    }
    /* Bad claim -- max term too small for expiration */
    {
        claim1.term_max -= 1;
        let mut claims = HashMap::new();
        claims.insert(claim_ids[0], Ok(claim0));
        claims.insert(claim_ids[1], Ok(claim1));

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

        let res = h.extend_sectors2(&mut rt, params, claims);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            &format!(
                "claim only allows extension to {} but declared new expiration is {}",
                new_expiration - 1,
                new_expiration
            ),
            res,
        );
        // assert sector expiration is set to the new value
        check_for_expiration(
            &mut h,
            &mut rt,
            old_sector.commitment_expiration,
            old_sector.sector_number,
            deadline_index,
            partition_index,
        );
        rt.replace_state(&state);
        rt.reset();
    }
}

#[test]
fn extend_expiration2_drop_claims() {
    let (mut h, mut rt) = setup();
    // add in verified deal
    let verified_deals = vec![
        test_verified_deal(h.sector_size as u64 / 2),
        test_verified_deal(h.sector_size as u64 / 2),
    ];
    let policy = Policy::default();
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.commitment_expiration + extension;

    let claim_ids = vec![400, 500];
    let client = Address::new_id(3000).id().unwrap();
    let second_expiration = new_expiration + 42 * EPOCHS_IN_DAY;

    let claim0 = make_claim(
        claim_ids[0],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        second_expiration,
        &verified_deals[0],
        rt.policy.minimum_verified_allocation_term,
    );
    let claim1 = make_claim(
        claim_ids[1],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        new_expiration,
        &verified_deals[1],
        rt.policy.minimum_verified_allocation_term,
    );
    let mut claims = HashMap::new();
    claims.insert(claim_ids[0], Ok(claim0.clone()));
    claims.insert(claim_ids[1], Ok(claim1));

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration,
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sector.sector_number,
                maintain_claims: vec![claim_ids[0]],
                drop_claims: vec![claim_ids[1]],
            }],
        }],
    };
    rt.set_epoch(old_sector.commitment_expiration - policy.end_of_life_claim_drop_period);
    h.extend_sectors2(&mut rt, params, claims).unwrap();
    check_for_expiration(
        &mut h,
        &mut rt,
        new_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );

    assert_sector_verified_space(
        &mut h,
        &mut rt,
        old_sector.sector_number,
        verified_deals[0].size.0,
    );

    // only claim0 stored in verifreg now
    let mut claims = HashMap::new();
    claims.insert(claim_ids[0], Ok(claim0));

    // attempting to extend without accounting for claim0 fails
    let bad_params2 = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: bitfield_from_slice(&[old_sector.sector_number]),
            new_expiration: second_expiration,
            sectors_with_claims: vec![],
        }],
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "claim missing from declaration for sector",
        h.extend_sectors2(&mut rt, bad_params2, claims.clone()),
    );
    rt.reset();

    // if we extend again the dropped claim is irrelevant
    let params2 = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration: second_expiration,
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sector.sector_number,
                maintain_claims: vec![claim_ids[0]],
                drop_claims: vec![],
            }],
        }],
    };
    h.extend_sectors2(&mut rt, params2, claims).unwrap();
    check_for_expiration(
        &mut h,
        &mut rt,
        second_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );

    assert_sector_verified_space(
        &mut h,
        &mut rt,
        old_sector.sector_number,
        verified_deals[0].size.0,
    );
}

#[test]
fn update_expiration_legacy_fails_on_new_sector_with_deals() {
    let (mut h, mut rt) = setup();
    // add in verified deal
    let verified_deals = vec![
        test_verified_deal(h.sector_size as u64 / 2),
        test_verified_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.commitment_expiration + extension;

    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sector.sector_number]),
            new_expiration,
        }],
    };

    // legacy extend_sectors will fail to extend newly created sectors with deals
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "cannot use legacy sector extension for simple qa power with deal weight",
        h.extend_sectors(&mut rt, params),
    );
    rt.reset();
    check_for_expiration(
        &mut h,
        &mut rt,
        old_sector.commitment_expiration,
        old_sector.sector_number,
        deadline_index,
        partition_index,
    );
}

#[test]
fn update_expiration2_drop_claims_failure_cases() {
    let (mut h, mut rt) = setup();
    let policy = Policy::default();
    // add in verified deal
    let verified_deals = vec![
        test_verified_deal(h.sector_size as u64 / 2),
        test_verified_deal(h.sector_size as u64 / 2),
    ];
    let old_sector = commit_sector_verified_deals(&verified_deals, &mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.commitment_expiration + extension;

    let claim_ids = vec![400, 500];
    let client = Address::new_id(3000).id().unwrap();

    let claim0 = make_claim(
        claim_ids[0],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        new_expiration,
        &verified_deals[0],
        rt.policy.minimum_verified_allocation_term,
    );
    let mut claim1 = make_claim(
        claim_ids[1],
        &old_sector,
        client,
        h.receiver.id().unwrap(),
        new_expiration,
        &verified_deals[1],
        rt.policy.minimum_verified_allocation_term,
    );

    /* Drop claim before grace period */

    let mut claims = HashMap::new();
    claims.insert(claim_ids[0], Ok(claim0.clone()));
    claims.insert(claim_ids[1], Ok(claim1.clone()));

    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration,
            sectors_with_claims: vec![SectorClaim {
                sector_number: old_sector.sector_number,
                maintain_claims: vec![claim_ids[0]],
                drop_claims: vec![claim_ids[1]],
            }],
        }],
    };
    rt.set_epoch(old_sector.commitment_expiration - policy.end_of_life_claim_drop_period - 1);
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "attempt to drop claims with 86401 epochs > end of life claim drop period 86400 remaining",
        h.extend_sectors2(&mut rt, params.clone(), claims.clone()),
    );
    rt.reset();
    rt.set_epoch(old_sector.commitment_expiration - policy.end_of_life_claim_drop_period);

    /* Dropped claim not found */
    let mut claims = HashMap::new();
    claims.insert(claim_ids[0], Ok(claim0.clone()));
    claims.insert(claim_ids[1], Err(actor_error!(not_found, "claim not found")));
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "failed to get claims for sector",
        h.extend_sectors2(&mut rt, params.clone(), claims.clone()),
    );
    rt.reset();

    /* Dropped claim provider mismatch */
    claim1.provider = h.receiver.id().unwrap() + 7;
    let mut claims = HashMap::new();
    claims.insert(claim_ids[0], Ok(claim0.clone()));
    claims.insert(claim_ids[1], Ok(claim1.clone()));
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!(
            "expected claim provider to be {} but found {}",
            h.receiver.id().unwrap(),
            h.receiver.id().unwrap() + 7
        ),
        h.extend_sectors2(&mut rt, params.clone(), claims.clone()),
    );
    rt.reset();
    claim1.provider = h.receiver.id().unwrap(); // reset

    /* Dropped claim sector number mismatch */
    claim1.sector = old_sector.sector_number + 7;
    let mut claims = HashMap::new();
    claims.insert(claim_ids[0], Ok(claim0));
    claims.insert(claim_ids[1], Ok(claim1.clone()));
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!(
            "expected claim sector number to be {} but found {}",
            old_sector.sector_number,
            old_sector.sector_number + 7
        ),
        h.extend_sectors2(&mut rt, params, claims.clone()),
    );
    rt.reset();
    claim1.sector = old_sector.sector_number;
}

fn commit_sector_verified_deals(
    verified_deals: &Vec<VerifiedDealInfo>,
    h: &mut ActorHarness,
    rt: &mut MockRuntime,
) -> SectorOnChainInfo {
    h.construct_and_verify(rt);
    assert!(!verified_deals.is_empty());

    let mut pcc = ProveCommitConfig::empty();
    pcc.add_verified_deals(h.next_sector_no, verified_deals.clone());

    let sector_info = &h.commit_and_prove_sectors_with_cfgs(
        rt,
        1,
        DEFAULT_SECTOR_EXPIRATION as u64,
        vec![vec![42]],
        true,
        pcc,
    )[0];

    sector_info.clone()
}

// assert that state tracks an expiration at the provided epoch in the provided deadline and partition for the provided sector
fn check_for_expiration(
    h: &mut ActorHarness,
    rt: &mut MockRuntime,
    expiration: ChainEpoch,
    sector_number: SectorNumber,
    deadline_index: u64,
    partition_index: u64,
) {
    let new_sector = h.get_sector(rt, sector_number);
    assert_eq!(expiration, new_sector.commitment_expiration);

    // assert that new expiration exists
    h.assert_queue_state(rt, sector_number, deadline_index, partition_index, expiration, ExpirationKind::OnTime);

    h.check_state(rt);
}

fn assert_sector_verified_space(
    h: &mut ActorHarness,
    rt: &mut MockRuntime,
    sector_number: SectorNumber,
    v_deal_space: u64,
) {
    let new_sector = h.get_sector(rt, sector_number);
    assert_eq!(
        DealWeight::from(v_deal_space),
        new_sector.verified_deal_weight
            / (new_sector.commitment_expiration - new_sector.activation)
    );
}

fn make_claim(
    claim_id: u64,
    sector: &SectorOnChainInfo,
    client: ActorID,
    provider: ActorID,
    new_expiration: ChainEpoch,
    deal: &VerifiedDealInfo,
    term_min: ChainEpoch,
) -> FILPlusClaim {
    FILPlusClaim {
        provider,
        client,
        data: make_piece_cid(format!("piece for claim {}", claim_id).as_bytes()),
        size: deal.size,
        term_min,
        term_max: new_expiration - sector.activation,
        term_start: sector.activation,
        sector: sector.sector_number,
    }
}

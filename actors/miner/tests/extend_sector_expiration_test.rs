use fil_actor_miner::{
    power_for_sector, seal_proof_sector_maximum_lifetime, ExpirationExtension,
    ExtendSectorExpirationParams, PoStPartition, SectorOnChainInfo, State,
};
use fil_actors_runtime::{
    runtime::{Runtime, RuntimePolicy},
    test_utils::{expect_abort_contains_message, MockRuntime},
};
use fvm_ipld_bitfield::{BitField, UnvalidatedBitField};
use fvm_shared::{
    clock::ChainEpoch, econ::TokenAmount, error::ExitCode, sector::RegisteredSealProof,
};

mod util;
use itertools::Itertools;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 10u128.pow(24);
    let period_offset = 100;
    let precommit_epoch = 1;

    let mut h = ActorHarness::new(period_offset);
    // reduce the partition size
    h.set_proof_type(RegisteredSealProof::StackedDRG512MiBV1P1);
    let mut rt = h.new_runtime();
    rt.balance.replace(TokenAmount::from(big_balance));
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

fn commit_sector(h: &mut ActorHarness, rt: &mut MockRuntime) -> SectorOnChainInfo {
    h.construct_and_verify(rt);

    h.commit_and_prove_sectors(rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, Vec::new(), true)[0]
        .to_owned()
}

#[test]
fn rejects_negative_extensions() {
    let (mut h, mut rt) = setup();
    let sector = commit_sector(&mut h, &mut rt);

    // attempt to shorten epoch
    let new_expiration = sector.expiration - rt.policy().wpost_proving_period;

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

    let res = h.extend_sectors(&mut rt, params);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("cannot reduce sector {} expiration", sector.sector_number),
        res,
    );
    h.check_state(&rt);
}

#[test]
fn rejects_extension_too_far_in_future() {
    let (mut h, mut rt) = setup();
    let sector = commit_sector(&mut h, &mut rt);

    // extend by even proving period after max
    rt.set_epoch(sector.expiration);
    let extension = rt.policy().wpost_proving_period + rt.policy().max_sector_expiration_extension;
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

    let res = h.extend_sectors(&mut rt, params);
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
    let (mut h, mut rt) = setup();
    let mut sector = commit_sector(&mut h, &mut rt);
    // and prove it once to activate it.
    h.advance_and_submit_posts(&mut rt, &vec![sector.clone()]);

    let max_lifetime = seal_proof_sector_maximum_lifetime(sector.seal_proof).unwrap();

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), sector.sector_number).unwrap();

    // extend sector until just below threshold
    rt.set_epoch(sector.expiration);
    let extension = rt.policy().min_sector_expiration;

    let mut expiration = sector.expiration + extension;
    while expiration - sector.activation < max_lifetime {
        let params = ExtendSectorExpirationParams {
            extensions: vec![ExpirationExtension {
                deadline: deadline_index,
                partition: partition_index,
                sectors: make_bitfield(&[sector.sector_number]),
                new_expiration: expiration,
            }],
        };
        h.extend_sectors(&mut rt, params).unwrap();
        sector.expiration = expiration;

        expiration += extension;
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

    let res = h.extend_sectors(&mut rt, params);
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "total sector lifetime", res);
    h.check_state(&rt);
}

#[test]
fn updates_expiration_with_valid_params() {
    let (mut h, mut rt) = setup();
    let old_sector = commit_sector(&mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();

    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period;
    let new_expiration = old_sector.expiration + extension;

    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sector.sector_number]),
            new_expiration,
        }],
    };

    h.extend_sectors(&mut rt, params).unwrap();

    // assert sector expiration is set to the new value
    let new_sector = h.get_sector(&rt, old_sector.sector_number);
    assert_eq!(new_expiration, new_sector.expiration);

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
fn updates_many_sectors() {
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

    let new_expiration = sector_infos[0].expiration + 42 * rt.policy().wpost_proving_period;
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

    h.extend_sectors(&mut rt, params).unwrap();
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

#[test]
fn supports_extensions_off_deadline_boundary() {
    let (mut h, mut rt) = setup();
    let old_sector = commit_sector(&mut h, &mut rt);
    h.advance_and_submit_posts(&mut rt, &vec![old_sector.clone()]);

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.policy(), rt.store(), old_sector.sector_number).unwrap();

    let extension = 42 * rt.policy().wpost_proving_period + rt.policy().wpost_proving_period / 3;
    let new_expiration = old_sector.expiration + extension;

    let params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[old_sector.sector_number]),
            new_expiration,
        }],
    };

    h.extend_sectors(&mut rt, params).unwrap();

    // assert sector expiration is set to the new value
    let mut state: State = rt.get_state();
    let new_sector = h.get_sector(&rt, old_sector.sector_number);
    assert_eq!(new_expiration, new_sector.expiration);

    // advance clock to expiration
    rt.set_epoch(new_sector.expiration);
    state.proving_period_start += rt.policy().wpost_proving_period
        * ((rt.epoch - state.proving_period_start) / rt.policy().wpost_proving_period + 1);
    rt.replace_state(&state);

    // confirm it is not in sector's deadline to make sure we're testing extensions off the deadline boundary"
    let deadline_info = h.deadline(&rt);
    assert_ne!(deadline_index, deadline_info.index);

    // advance to deadline and submit one last PoSt
    let deadline_info = h.advance_to_deadline(&mut rt, deadline_index);

    let partitions = vec![PoStPartition {
        index: partition_index,
        skipped: UnvalidatedBitField::Validated(BitField::default()),
    }];
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

// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actor_miner::{power_for_sectors, Deadline, PoStPartition, PowerPair, SectorOnChainInfo};
use fil_actors_runtime::{runtime::Policy, test_utils::make_sealed_cid};
use fvm_ipld_bitfield::BitField;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;

use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::sector::SectorNumber;

mod util;
use util::*;

mod state_harness;
use state_harness::*;

/// returns a unique SectorOnChainInfo with each invocation with SectorNumber set to `sectorNo`.
fn new_sector_on_chain_info(
    sector_number: SectorNumber,
    sealed_cid: Cid,
    weight: BigInt,
    activation: ChainEpoch,
) -> SectorOnChainInfo {
    SectorOnChainInfo {
        sector_number,
        seal_proof: RegisteredSealProof::StackedDRG32GiBV1P1,
        sealed_cid,
        activation,
        expiration: 1,
        deal_weight: weight.clone(),
        verified_deal_weight: weight,
        ..SectorOnChainInfo::default()
    }
}

mod sector_assignment {

    use super::*;

    #[test]
    fn assign_sectors_to_deadlines() {
        let proof_type = RegisteredSealProof::StackedDRG32GiBV1P1;
        let partition_sectors = proof_type.window_post_partitions_sector().unwrap();
        let sector_size = proof_type.sector_size().unwrap();

        let open_deadlines = Policy::default().wpost_period_deadlines - 2;

        let partitions_per_deadline: u64 = 3;
        let num_sectors = partition_sectors * open_deadlines * partitions_per_deadline;
        let sector_infos: Vec<SectorOnChainInfo> = (0..num_sectors)
            .map(|i| {
                new_sector_on_chain_info(
                    i as SectorNumber,
                    make_sealed_cid("{i}".as_bytes()),
                    BigInt::from(1u8),
                    0,
                )
            })
            .collect();

        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, 0);

        h.assign_sectors_to_deadlines(
            &policy,
            0,
            sector_infos.clone(),
            partition_sectors,
            sector_size,
        );

        let sectors_array = sectors_arr(&h.store, sector_infos.clone());

        let deadlines = h.st.load_deadlines(&h.store).unwrap();

        deadlines
            .for_each(&policy, &h.store, |dl_idx: u64, mut dl: Deadline| {
                let dl_state = ExpectedDeadlineState {
                    sector_size,
                    partition_size: partition_sectors,
                    sectors: sector_infos.clone(),
                    ..Default::default()
                };

                let quant_spec = h.quant_spec_for_deadline(&policy, dl_idx);
                // deadlines 0 & 1 are closed for assignment right now.
                if dl_idx < 2 {
                    dl_state.with_quant_spec(quant_spec).assert(
                        &h.store,
                        &sector_infos.clone(),
                        &dl,
                    );
                    return Ok(());
                }

                let mut partitions = Vec::<BitField>::new();
                let mut post_partitions = Vec::<PoStPartition>::new();
                for i in 0..partitions_per_deadline {
                    let start = ((i * open_deadlines) + (dl_idx - 2)) * partition_sectors;
                    let part_bf = seq(start, partition_sectors);
                    partitions.push(part_bf);
                    post_partitions.push(PoStPartition {
                        index: i,
                        skipped: BitField::new(),
                    });
                }
                let all_sector_bf = BitField::union(&partitions);
                let all_sector_numbers: Vec<u64> =
                    all_sector_bf.bounded_iter(num_sectors).unwrap().collect();

                dl_state
                    .with_quant_spec(quant_spec)
                    .with_unproven(&all_sector_numbers)
                    .with_partitions(partitions.clone())
                    .assert(&h.store, &sector_infos.clone(), &dl);

                // Now make sure proving activates power.

                let result = dl
                    .record_proven_sectors(
                        &h.store,
                        &sectors_array,
                        sector_size,
                        quant_spec,
                        0,
                        &mut post_partitions,
                    )
                    .unwrap();

                let expected_power_delta =
                    power_for_sectors(sector_size, &select_sectors(&sector_infos, &all_sector_bf));

                assert_eq!(all_sector_bf, result.sectors);
                assert!(result.ignored_sectors.is_empty());
                assert_eq!(result.new_faulty_power, PowerPair::zero());
                assert_eq!(result.power_delta, expected_power_delta);
                assert_eq!(result.recovered_power, PowerPair::zero());
                assert_eq!(result.retracted_recovery_power, PowerPair::zero());
                Ok(())
            })
            .unwrap();
    }
}

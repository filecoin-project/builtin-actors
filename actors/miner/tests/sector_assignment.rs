// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actor_miner::{power_for_sectors, SectorOnChainInfo, BitFieldQueue, Deadline, PowerPair, PoStPartition};
use fil_actors_runtime::{
    test_utils::{make_sealed_cid},
    runtime::Policy,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorNumber;

use fvm_ipld_bitfield::{BitField, UnvalidatedBitField};

use std::collections::BTreeMap;

mod util;
use util::*;

mod state_harness;
use state_harness::*;

const PERIOD_OFFSET: ChainEpoch = 0;

/// returns a unique SectorOnChainInfo with each invocation with SectorNumber set to `sectorNo`.
fn new_sector_on_chain_info(
    sector_number: SectorNumber,
    sealed_cid: Cid,
    weight: TokenAmount,
    activation: ChainEpoch,
) -> SectorOnChainInfo {
    SectorOnChainInfo {
        sector_number,
        sealed_cid,
        activation,
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

        let open_dealines = Policy::default().wpost_period_deadlines - 2;

        let partitions_per_deadline: u64 = 3;
        let num_sectors = partition_sectors * open_dealines * partitions_per_deadline;
        let sector_infos: Vec<SectorOnChainInfo> = (0..num_sectors).map(|i| {
            new_sector_on_chain_info(
                i as u64,
                make_sealed_cid("{i}".as_bytes()),
                TokenAmount::from(1u8),
                0,
            )
        }).collect();

        let mut dl_state = ExpectedDeadlineState {
            sector_size,
            partition_size: partition_sectors,
            sectors: sector_infos.clone(),
            ..Default::default()
        };

        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);
        let rt = h.new_runtime();

        h.assign_sectors_to_deadlines(&policy, 0, sector_infos.clone(), partition_sectors, sector_size);

        let sectors_array = sectors_array(&rt, &h.store, sector_infos);

        let mut deadlines = h.st.load_deadlines(&rt.store).unwrap();

        deadlines
            .for_each(&policy, &rt.store, |dl_idx: u64, mut dl: Deadline| {
                let quant_spec = h.quant_spec_for_deadline(&policy, dl_idx);
                // deadlines 0 & 1 are closed for assignment right now.
                if dl_idx < 2 {
                    // dl_state.with_quant_spec(quant_spec)
                    //     .assert(&h.store, todo!(), &dl);
                    return Ok(());
                }

                let mut partitions = Vec::<BitField>::new();
                let mut post_partitions = Vec::<PoStPartition>::new();
                for i in 0..partitions_per_deadline {
                    let start = ((i * open_dealines) + (dl_idx - 2)) * partition_sectors;
                    let part_bf = seq(start, partition_sectors);
                    partitions.push(part_bf);
                    post_partitions.push(PoStPartition {
                        index: 0,
                        skipped: UnvalidatedBitField::Validated(BitField::new()),
                    });
                    let all_sector_bf = BitField::union(&partitions);
                    let all_sector_numbers = all_sector_bf.bounded_iter(num_sectors);

                    // dl_state.with_quant_spec(quant_spec)
                    //     .with_unproven(all_sector_numbers)
                    //     .with_partitions(partitions)
                    //     .assert(&h.store, todo!(), &dl);

                    // Now make sure proving activates power.

                    let result = dl
                        .record_proven_sectors(
                            &rt.store,
                            &sectors_array,
                            SECTOR_SIZE,
                            QUANT_SPEC,
                            0,
                            &mut post_partitions,
                        )
                        .unwrap();

                    let expected_power_delta = todo!();//power_for_sectors(sector_size, &select_sectors(&sector_infos, &all_sector_bf));

                    assert_eq!(all_sector_bf, result.sectors);
                    assert!(result.ignored_sectors.is_empty());
                    assert_eq!(result.new_faulty_power, PowerPair::zero());
                    assert_eq!(result.power_delta, expected_power_delta);
                    assert_eq!(result.recovered_power, PowerPair::zero());
                    assert_eq!(result.retracted_recovery_power, PowerPair::zero());
                }
                Ok(())
            })
            .unwrap();

        // Now prove and activate/check power.
    }
}

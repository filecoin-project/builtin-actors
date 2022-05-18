// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actor_miner::{SectorOnChainInfo, BitFieldQueue, Deadline};
use fil_actors_runtime::{
    test_utils::{make_sealed_cid},
    runtime::Policy,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorNumber;

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
        let mut sector_infos = vec![SectorOnChainInfo::default(); num_sectors as usize];
        for (i, si) in sector_infos.iter_mut().enumerate() {
            *si = new_sector_on_chain_info(
                i as u64,
                make_sealed_cid("{i}".as_bytes()),
                TokenAmount::from(1u8),
                0,
            );
        }

        let dl_state = ExpectedDeadlineState {
            sector_size,
            partition_size: partition_sectors,
            sectors: sector_infos.clone(),
            ..Default::default()
        };

        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);
        let rt = h.new_runtime();

        h.assign_sectors_to_deadlines(&policy, 0, sector_infos.clone(), partition_sectors, sector_size);

        let sector_arr = sectors_array(&rt, &h.store, sector_infos);

        //let dls =
        let dls = h.st.load_deadlines(&rt.store).unwrap();

        dls.for_each(&policy, &rt.store, |dl_idx: u64, dl: Deadline| {
            Ok(())
        }).unwrap();

        // Now prove and activate/check power.
    }
}

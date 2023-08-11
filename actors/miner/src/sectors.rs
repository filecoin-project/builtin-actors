// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::BTreeSet;

use cid::Cid;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::sector::{MAX_SECTOR_NUMBER, SectorNumber};

use fil_actors_runtime::{actor_error, ActorError, Array, AsActorErrors};

use super::SectorOnChainInfo;

pub struct Sectors<'db, BS> {
    pub amt: Array<'db, SectorOnChainInfo, BS>,
}

impl<'db, BS: Blockstore> Sectors<'db, BS> {
    pub fn load(store: &'db BS, root: &Cid) -> Result<Self, ActorError> {
        Ok(Self { amt: Array::load(root, store).or_illegal_state("failed to load sectors")? })
    }

    pub fn load_sector(
        &self,
        sector_numbers: &BitField,
    ) -> Result<Vec<SectorOnChainInfo>, ActorError> {
        let mut sector_infos: Vec<SectorOnChainInfo> = Vec::new();
        for sector_number in sector_numbers.iter() {
            let sector_on_chain = self
                .amt
                .get(sector_number)
                .or_with_illegal_state(|| format!("failed to load sector {}", sector_number))?
                .cloned()
                .ok_or_else(|| actor_error!(not_found; "sector not found: {}", sector_number))?;
            sector_infos.push(sector_on_chain);
        }
        Ok(sector_infos)
    }

    pub fn get(
        &self,
        sector_number: SectorNumber,
    ) -> Result<Option<SectorOnChainInfo>, ActorError> {
        Ok(self
            .amt
            .get(sector_number)
            .or_with_illegal_state(|| format!("failed to get sector {}", sector_number))?
            .cloned())
    }

    pub fn store(&mut self, infos: Vec<SectorOnChainInfo>) -> Result<(), ActorError> {
        for info in infos {
            let sector_number = info.sector_number;

            if sector_number > MAX_SECTOR_NUMBER {
                return Err(actor_error!(
                    illegal_argument,
                    "sector number {} out of range",
                    info.sector_number
                ));
            }

            self.amt
                .set(sector_number, info)
                .or_with_illegal_state(|| format!("failed to store sector {}", sector_number))?;
        }

        Ok(())
    }

    pub fn must_get(&self, sector_number: SectorNumber) -> Result<SectorOnChainInfo, ActorError> {
        self.get(sector_number)
            .or_with_illegal_state(|| format!("failed to load sector {}", sector_number))?
            .ok_or_else(|| actor_error!(not_found, "sector {} not found", sector_number))
    }

    /// Loads info for a set of sectors to be proven.
    /// If any of the sectors are declared faulty and not to be recovered, info for the first non-faulty sector is substituted instead.
    /// If any of the sectors are declared recovered, they are returned from this method.
    pub fn load_for_proof(
        &self,
        proven_sectors: &BitField,
        expected_faults: &BitField,
    ) -> Result<Vec<SectorOnChainInfo>, ActorError> {
        let non_faults = proven_sectors - expected_faults;

        if non_faults.is_empty() {
            return Ok(Vec::new());
        }

        let good_sector_number = non_faults.first().expect("faults are not empty");

        let sector_infos = self.load_with_fault_max(
            proven_sectors,
            expected_faults,
            good_sector_number as SectorNumber,
        )?;

        Ok(sector_infos)
    }
    /// Loads sector info for a sequence of sectors, substituting info for a stand-in sector for any that are faulty.
    pub fn load_with_fault_max(
        &self,
        sectors: &BitField,
        faults: &BitField,
        fault_stand_in: SectorNumber,
    ) -> Result<Vec<SectorOnChainInfo>, ActorError> {
        let stand_in_info = self.must_get(fault_stand_in)?;

        // Expand faults into a map for quick lookups.
        // The faults bitfield should already be a subset of the sectors bitfield.
        let sector_count = sectors.len();

        let fault_set: BTreeSet<u64> = faults.iter().collect();

        let mut sector_infos = Vec::with_capacity(sector_count as usize);
        for i in sectors.iter() {
            let faulty = fault_set.contains(&i);
            let sector = if !faulty { self.must_get(i)? } else { stand_in_info.clone() };
            sector_infos.push(sector);
        }

        Ok(sector_infos)
    }
}

pub fn select_sectors(
    sectors: &[SectorOnChainInfo],
    field: &BitField,
) -> Result<Vec<SectorOnChainInfo>, ActorError> {
    let mut to_include: BTreeSet<_> = field.iter().collect();
    let included =
        sectors.iter().filter(|si| to_include.remove(&si.sector_number)).cloned().collect();

    if !to_include.is_empty() {
        return Err(actor_error!(
            not_found,
            "failed to find {} expected sectors",
            to_include.len()
        ));
    }

    Ok(included)
}

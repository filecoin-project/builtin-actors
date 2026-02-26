// Copyright 2024 Curio Storage Inc.
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::{ActorError, ActorDowncast, actor_error};
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use cid::Cid;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::error::ExitCode;
use multihash_codetable::Code;
use fvm_ipld_encoding::CborStore;

/// State for the Sealer actor
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone)]
pub struct State {
    pub allocated_sectors: Cid, // BitField

    // Address of a validator actor which learns about all consumed sector numbers with the ability to veto a transaction
    pub validator: Address,
}

#[derive(PartialEq, Eq)]
pub enum CollisionPolicy {
    AllowCollisions,
    DenyCollisions,
}

impl State {
    /// Marks a set of sector numbers as having been allocated.
    /// If policy is `DenyCollisions`, fails if the set intersects with the sector numbers already allocated.
    pub fn allocate_sector_numbers<BS: Blockstore>(
        &mut self,
        store: &BS,
        sector_numbers: &BitField,
        policy: CollisionPolicy,
    ) -> Result<(), ActorError> {
        let prior_allocation = store
            .get_cbor(&self.allocated_sectors)
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to load allocated sectors bitfield",
                )
            })?
            .ok_or_else(|| actor_error!(illegal_state, "allocated sectors bitfield not found"))?;

        if policy != CollisionPolicy::AllowCollisions {
            // NOTE: A fancy merge algorithm could extract this intersection while merging, below, saving
            // one iteration of the runs
            let collisions = &prior_allocation & sector_numbers;
            if !collisions.is_empty() {
                return Err(actor_error!(
                    illegal_argument,
                    "sector numbers {:?} already allocated",
                    collisions
                ));
            }
        }
        let new_allocation = &prior_allocation | sector_numbers;
        self.allocated_sectors =
            store.put_cbor(&new_allocation, Code::Blake2b256).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_ARGUMENT,
                    format!(
                        "failed to store allocated sectors bitfield after adding {:?}",
                        sector_numbers,
                    ),
                )
            })?;
        Ok(())
    }
}
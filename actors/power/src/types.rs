// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{strict_bytes, BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredPoStProof, StoragePower};
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::ActorID;

use serde::{Deserialize, Serialize};

pub type SectorTermination = i64;

/// Implicit termination after all deals expire
pub const SECTOR_TERMINATION_EXPIRED: SectorTermination = 0;
/// Unscheduled explicit termination by the miner
pub const SECTOR_TERMINATION_MANUAL: SectorTermination = 1;
/// Implicit termination due to unrecovered fault
pub const SECTOR_TERMINATION_FAULTY: SectorTermination = 3;

pub const CRON_QUEUE_HAMT_BITWIDTH: u32 = 6;
pub const CRON_QUEUE_AMT_BITWIDTH: u32 = 6;
pub const PROOF_VALIDATION_BATCH_AMT_BITWIDTH: u32 = 4;

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct CreateMinerParams {
    pub owner: Address,
    pub worker: Address,
    pub window_post_proof_type: RegisteredPoStProof,
    #[serde(with = "strict_bytes")]
    pub peer: Vec<u8>,
    pub multiaddrs: Vec<BytesDe>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct CreateMinerReturn {
    /// Canonical ID-based address for the actor.
    pub id_address: Address,
    /// Re-org safe address for created actor.
    pub robust_address: Address,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct UpdateClaimedPowerParams {
    #[serde(with = "bigint_ser")]
    pub raw_byte_delta: StoragePower,
    #[serde(with = "bigint_ser")]
    pub quality_adjusted_delta: StoragePower,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct EnrollCronEventParams {
    pub event_epoch: ChainEpoch,
    pub payload: RawBytes,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Default)]
#[serde(transparent)]
pub struct UpdatePledgeTotalParams {
    pub pledge_delta: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, PartialEq, Eq)]
pub struct CurrentTotalPowerReturn {
    #[serde(with = "bigint_ser")]
    pub raw_byte_power: StoragePower,
    #[serde(with = "bigint_ser")]
    pub quality_adj_power: StoragePower,
    pub pledge_collateral: TokenAmount,
    pub quality_adj_power_smoothed: FilterEstimate,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct NetworkRawPowerReturn {
    #[serde(with = "bigint_ser")]
    pub raw_byte_power: StoragePower,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct MinerRawPowerParams {
    pub miner: ActorID,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct MinerRawPowerReturn {
    #[serde(with = "bigint_ser")]
    pub raw_byte_power: StoragePower,
    pub meets_consensus_minimum: bool,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct MinerCountReturn {
    pub miner_count: i64,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct MinerConsensusCountReturn {
    pub miner_consensus_count: i64,
}

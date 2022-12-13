// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use super::ext::verifreg::AllocationID;
use cid::Cid;
use fil_actors_runtime::Array;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::serde_bytes;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::bigint::{bigint_ser, BigInt};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::ActorID;

use crate::Label;
use fvm_shared::sector::RegisteredSealProof;

use super::deal::{ClientDealProposal, DealProposal, DealState};

pub const PROPOSALS_AMT_BITWIDTH: u32 = 5;
pub const STATES_AMT_BITWIDTH: u32 = 6;

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct WithdrawBalanceParams {
    pub provider_or_client: Address,
    pub amount: TokenAmount,
}

impl Cbor for WithdrawBalanceParams {}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct WithdrawBalanceReturn {
    pub amount_withdrawn: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct GetBalanceReturn {
    pub balance: TokenAmount,
    pub locked: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct OnMinerSectorsTerminateParams {
    pub epoch: ChainEpoch,
    pub deal_ids: Vec<DealID>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct PublishStorageDealsParams {
    pub deals: Vec<ClientDealProposal>,
}

impl Cbor for PublishStorageDealsParams {}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, PartialEq)] // Add Eq when BitField does
pub struct PublishStorageDealsReturn {
    pub ids: Vec<DealID>,
    pub valid_deals: BitField,
}

// Changed since V2:
// - Array of Sectors rather than just one
// - Removed SectorStart
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct VerifyDealsForActivationParams {
    pub sectors: Vec<SectorDeals>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct SectorDeals {
    pub sector_type: RegisteredSealProof,
    pub sector_expiry: ChainEpoch,
    pub deal_ids: Vec<DealID>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct VerifyDealsForActivationReturn {
    pub sectors: Vec<SectorDealData>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq, Default)]
pub struct SectorDealData {
    /// Option::None signifies commitment to empty sector, meaning no deals.
    pub commd: Option<Cid>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct ActivateDealsParams {
    pub deal_ids: Vec<DealID>,
    pub sector_expiry: ChainEpoch,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct VerifiedDealInfo {
    pub client: ActorID,
    pub allocation_id: AllocationID,
    pub data: Cid,
    pub size: PaddedPieceSize,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct ActivateDealsResult {
    #[serde(with = "bigint_ser")]
    pub nonverified_deal_space: BigInt,
    pub verified_infos: Vec<VerifiedDealInfo>,
}
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct DealSpaces {
    #[serde(with = "bigint_ser")]
    pub deal_space: BigInt,
    #[serde(with = "bigint_ser")]
    pub verified_deal_space: BigInt,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct ComputeDataCommitmentParams {
    pub inputs: Vec<SectorDataSpec>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct ComputeDataCommitmentReturn {
    pub commds: Vec<Cid>,
}

/// A specialization of a array to deals.
pub type DealArray<'bs, BS> = Array<'bs, DealProposal, BS>;

/// A specialization of a array to deals.
pub type DealMetaArray<'bs, BS> = Array<'bs, DealState, BS>;

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct SectorDataSpec {
    pub deal_ids: Vec<DealID>,
    pub sector_type: RegisteredSealProof,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct DealQueryParams {
    pub id: DealID,
}

pub type GetDealDataCommitmentParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct GetDealDataCommitmentReturn {
    pub data: Cid,
    pub size: PaddedPieceSize,
}

pub type GetDealClientParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealClientReturn {
    pub client: ActorID,
}

pub type GetDealProviderParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealProviderReturn {
    pub provider: ActorID,
}

pub type GetDealLabelParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealLabelReturn {
    pub label: Label,
}

pub type GetDealTermParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct GetDealTermReturn {
    pub start: ChainEpoch,    // First epoch for the deal (inclusive)
    pub duration: ChainEpoch, // Duration of the deal.
}

pub type GetDealTotalPriceParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealTotalPriceReturn {
    pub total_price: TokenAmount,
}

pub type GetDealClientCollateralParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealClientCollateralReturn {
    pub collateral: TokenAmount,
}

pub type GetDealProviderCollateralParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealProviderCollateralReturn {
    pub collateral: TokenAmount,
}

pub type GetDealVerifiedParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealVerifiedReturn {
    pub verified: bool,
}

pub type GetDealActivationParams = DealQueryParams;
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct GetDealActivationReturn {
    /// Epoch at which the deal was activated, or -1.
    /// This may be before the proposed start epoch.
    pub activated: ChainEpoch,
    /// Epoch at which the deal was terminated abnormally, or -1.
    pub terminated: ChainEpoch,
}

// Interface market clients can implement to receive notifications from builtin market
pub const MARKET_NOTIFY_DEAL_METHOD: u64 = frc42_dispatch::method_hash!("MarketNotifyDeal");

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct MarketNotifyDealParams {
    #[serde(with = "serde_bytes")]
    pub proposal: Vec<u8>,
    pub deal_id: u64,
}

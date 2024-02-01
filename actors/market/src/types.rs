// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use super::ext::verifreg::AllocationID;
use cid::Cid;
use fil_actors_runtime::Array;
use fil_actors_runtime::BatchReturn;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::{bigint_ser, BigInt};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::ActorID;

use crate::Label;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};

use super::deal::{ClientDealProposal, DealProposal, DealState};

pub const PROPOSALS_AMT_BITWIDTH: u32 = 5;
pub const STATES_AMT_BITWIDTH: u32 = 6;

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct AddBalanceParams {
    pub provider_or_client: Address,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct WithdrawBalanceParams {
    pub provider_or_client: Address,
    pub amount: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct WithdrawBalanceReturn {
    pub amount_withdrawn: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetBalanceParams {
    pub account: Address,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct GetBalanceReturn {
    pub balance: TokenAmount,
    pub locked: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, PartialEq)] // Add Eq when BitField does
pub struct OnMinerSectorsTerminateParams {
    pub epoch: ChainEpoch,
    pub sectors: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct PublishStorageDealsParams {
    pub deals: Vec<ClientDealProposal>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, PartialEq)] // Add Eq when BitField does
pub struct PublishStorageDealsReturn {
    pub ids: Vec<DealID>,
    pub valid_deals: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct VerifyDealsForActivationParams {
    /// Deals to verify, grouped by sector.
    pub sectors: Vec<SectorDeals>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct SectorDeals {
    pub sector_number: SectorNumber,
    pub sector_type: RegisteredSealProof,
    pub sector_expiry: ChainEpoch,
    pub deal_ids: Vec<DealID>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct VerifyDealsForActivationReturn {
    // The unsealed CID computed from the deals specified for each sector.
    // A None indicates no deals were specified.
    pub unsealed_cids: Vec<Option<Cid>>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct BatchActivateDealsParams {
    /// Deals to activate, grouped by sector.
    /// A failed deal activation will cause other deals in the same sector group to also fail,
    /// but allow other sectors to proceed.
    pub sectors: Vec<SectorDeals>,
    /// Requests computation of an unsealed CID for each sector from the provided deals.
    pub compute_cid: bool,
}

// Information about a deal that has been activated.
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct ActivatedDeal {
    pub client: ActorID,
    pub allocation_id: AllocationID, // NO_ALLOCATION_ID for unverified deals.
    pub data: Cid,
    pub size: PaddedPieceSize,
}

// Information about a sector-grouping of deals that have been activated.
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct SectorDealActivation {
    /// Information about each deal activated.
    pub activated: Vec<ActivatedDeal>,
    /// Unsealed CID computed from the deals specified for the sector.
    /// A None indicates no deals were specified, or the computation was not requested.
    pub unsealed_cid: Option<Cid>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct BatchActivateDealsResult {
    /// Status of each sector grouping of deals.
    pub activation_results: BatchReturn,
    /// Activation information for the sector groups that were activated.
    pub activations: Vec<SectorDealActivation>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct DealSpaces {
    #[serde(with = "bigint_ser")]
    pub deal_space: BigInt,
    #[serde(with = "bigint_ser")]
    pub verified_deal_space: BigInt,
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
    // First epoch for the deal (inclusive)
    pub start: ChainEpoch,
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

pub type GetDealSectorParams = DealQueryParams;

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct GetDealSectorReturn {
    /// Sector number with the provider that has committed the deal.
    pub sector: SectorNumber,
}

// Interface market clients can implement to receive notifications from builtin market
pub const MARKET_NOTIFY_DEAL_METHOD: u64 = frc42_dispatch::method_hash!("MarketNotifyDeal");

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct MarketNotifyDealParams {
    #[serde(with = "strict_bytes")]
    pub proposal: Vec<u8>,
    pub deal_id: u64,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone)]
#[serde(transparent)]
pub struct SettleDealPaymentsParams {
    pub deal_ids: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct SettleDealPaymentsReturn {
    /// Indicators of success or failure for each deal
    pub results: BatchReturn,
    /// Results for the deals that succesfully settled
    pub settlements: Vec<DealSettlementSummary>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct DealSettlementSummary {
    /// Incremental amount of funds transferred from client to provider for deal payment
    pub payment: TokenAmount,
    /// Whether the deal has settled for the final time
    pub completed: bool,
}

// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::{BytesDe, strict_bytes};
use fvm_ipld_encoding::{RawBytes, tuple::*};
use fvm_shared::ActorID;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{
    PoStProof, RegisteredAggregateProof, RegisteredPoStProof, RegisteredSealProof,
    RegisteredUpdateProof, SectorNumber, SectorSize, StoragePower,
};
use serde::{Deserialize, Serialize};

use fil_actors_runtime::reward::FilterEstimate;
use fil_actors_runtime::{BatchReturn, DealWeight};

use crate::commd::CompactCommD;
use crate::ext::verifreg::AllocationID;
use crate::ext::verifreg::ClaimID;

use super::beneficiary::*;

pub type CronEvent = i64;

pub const CRON_EVENT_WORKER_KEY_CHANGE: CronEvent = 0;
pub const CRON_EVENT_PROVING_DEADLINE: CronEvent = 1;
pub const CRON_EVENT_PROCESS_EARLY_TERMINATIONS: CronEvent = 2;

/// Storage miner actor constructor params are defined here so the power actor can send them to the init actor
/// to instantiate miners.
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct MinerConstructorParams {
    pub owner: Address,
    pub worker: Address,
    pub control_addresses: Vec<Address>,
    pub window_post_proof_type: RegisteredPoStProof,
    #[serde(with = "strict_bytes")]
    pub peer_id: Vec<u8>,
    pub multi_addresses: Vec<BytesDe>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct CronEventPayload {
    pub event_type: i64,
}

#[derive(Debug)]
pub struct PartitionKey {
    pub deadline: u64,
    pub partition: u64,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct GetControlAddressesReturn {
    pub owner: Address,
    pub worker: Address,
    pub control_addresses: Vec<Address>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ChangeWorkerAddressParams {
    pub new_worker: Address,
    pub new_control_addresses: Vec<Address>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct ChangeOwnerAddressParams {
    pub new_owner: Address,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ChangePeerIDParams {
    #[serde(with = "strict_bytes")]
    pub new_id: Vec<u8>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ChangeMultiaddrsParams {
    pub new_multi_addrs: Vec<BytesDe>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InternalSectorSetupForPresealParams {
    pub sectors: Vec<SectorNumber>,
    pub reward_smoothed: FilterEstimate,
    #[serde(with = "bigint_ser")]
    pub reward_baseline_power: StoragePower,
    pub quality_adj_power_smoothed: FilterEstimate,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct DeferredCronEventParams {
    #[serde(with = "strict_bytes")]
    pub event_payload: Vec<u8>,
    pub reward_smoothed: FilterEstimate,
    pub quality_adj_power_smoothed: FilterEstimate,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct PoStPartition {
    /// Partitions are numbered per-deadline, from zero.
    pub index: u64,
    /// Sectors skipped while proving that weren't already declared faulty.
    pub skipped: BitField,
}

/// Information submitted by a miner to provide a Window PoSt.
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct SubmitWindowedPoStParams {
    /// The deadline index which the submission targets.
    pub deadline: u64,
    /// The partitions being proven.
    pub partitions: Vec<PoStPartition>,
    /// Array of proofs, one per distinct registered proof type present in the sectors being proven.
    /// In the usual case of a single proof type, this array will always have a single element (independent of number of partitions).
    pub proofs: Vec<PoStProof>,
    /// The epoch at which these proofs is being committed to a particular chain.
    pub chain_commit_epoch: ChainEpoch,
    /// The ticket randomness on the chain at the `chain_commit_epoch` on the chain this post is committed to.
    pub chain_commit_rand: Randomness,
}

// Deprecated as of FIP 0084 -- kept for legacy testing
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ProveCommitSectorParams {
    pub sector_number: SectorNumber,
    pub proof: RawBytes,
}

// Note no UnsealedCID because it must be "zero" data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct SectorNIActivationInfo {
    pub sealing_number: SectorNumber, // Sector number used to generate replica id
    pub sealer_id: ActorID,           // Must be set to ID of receiving actor for now
    pub sealed_cid: Cid,              // CommR
    pub sector_number: SectorNumber,  // Unique id of sector in actor state
    pub seal_rand_epoch: ChainEpoch,
    pub expiration: ChainEpoch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct ProveCommitSectorsNIParams {
    pub sectors: Vec<SectorNIActivationInfo>, // Information about sealing of each sector
    pub aggregate_proof: RawBytes,            // Aggregate proof for all sectors
    pub seal_proof_type: RegisteredSealProof, // Proof type for each seal (must be an NI-PoRep variant)
    pub aggregate_proof_type: RegisteredAggregateProof, // Proof type for aggregation
    pub proving_deadline: u64, // The Window PoST deadline index at which to schedule the new sectors
    pub require_activation_success: bool, // Whether to abort if any sector activation fails
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct ProveCommitSectors3Params {
    // Activation manifest for each sector being proven.
    pub sector_activations: Vec<SectorActivationManifest>,
    // Proofs for each sector, parallel to activation manifests.
    // Exactly one of sector_proofs or aggregate_proof must be non-empty.
    pub sector_proofs: Vec<RawBytes>,
    // Aggregate proof for all sectors.
    // Exactly one of sector_proofs or aggregate_proof must be non-empty.
    pub aggregate_proof: RawBytes,
    // The proof type for the aggregate proof (must be None if no aggregate proof).
    pub aggregate_proof_type: Option<RegisteredAggregateProof>,
    // Whether to abort if any sector activation fails.
    pub require_activation_success: bool,
    // Whether to abort if any notification returns a non-zero exit code.
    pub require_notification_success: bool,
}

// Data to activate a commitment to one sector and its data.
// All pieces of data must be specified, whether or not not claiming a FIL+ activation or being
// notified to a data consumer.
// An implicit zero piece fills any remaining sector capacity.
// Note: we should consider fast tracking the special case where there is only
//  one piece not claiming or notifying other actors to allow an empty piece vector.
//  We could interpret this as a single piece, size == sector size, cid == commD, empty allocation empty notify vector
#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct SectorActivationManifest {
    // Sector to be activated.
    pub sector_number: SectorNumber,
    // Pieces comprising the sector content, in order.
    pub pieces: Vec<PieceActivationManifest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct PieceActivationManifest {
    // Piece data commitment.
    pub cid: Cid,
    // Piece size.
    pub size: PaddedPieceSize,
    // Identifies a verified allocation to be claimed.
    pub verified_allocation_key: Option<VerifiedAllocationKey>,
    // Synchronous notifications to be sent to other actors after activation.
    pub notify: Vec<DataActivationNotification>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct VerifiedAllocationKey {
    pub client: ActorID,
    pub id: AllocationID,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct DataActivationNotification {
    // Actor to be notified.
    pub address: Address,
    // Data to send in the notification.
    pub payload: RawBytes,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct ProveCommitSectors3Return {
    pub activation_results: BatchReturn,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct ProveCommitSectorsNIReturn {
    pub activation_results: BatchReturn,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct CheckSectorProvenParams {
    pub sector_number: SectorNumber,
}

#[derive(Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ExtendSectorExpiration2Params {
    pub extensions: Vec<ExpirationExtension2>,
}

#[derive(Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct SectorClaim {
    pub sector_number: SectorNumber,
    pub maintain_claims: Vec<ClaimID>,
    pub drop_claims: Vec<ClaimID>,
}

#[derive(Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ExpirationExtension2 {
    pub deadline: u64,
    pub partition: u64,
    // IDs of sectors without FIL+ claims
    pub sectors: BitField,
    pub sectors_with_claims: Vec<SectorClaim>,
    pub new_expiration: ChainEpoch,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct TerminateSectorsParams {
    pub terminations: Vec<TerminationDeclaration>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct TerminationDeclaration {
    pub deadline: u64,
    pub partition: u64,
    pub sectors: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct TerminateSectorsReturn {
    // Set to true if all early termination work has been completed. When
    // false, the miner may choose to repeatedly invoke TerminateSectors
    // with no new sectors to process the remainder of the pending
    // terminations. While pending terminations are outstanding, the miner
    // will not be able to withdraw funds.
    pub done: bool,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct DeclareFaultsParams {
    pub faults: Vec<FaultDeclaration>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct FaultDeclaration {
    /// The deadline to which the faulty sectors are assigned, in range [0..WPoStPeriodDeadlines)
    pub deadline: u64,
    /// Partition index within the deadline containing the faulty sectors.
    pub partition: u64,
    /// Sectors in the partition being declared faulty.
    pub sectors: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct DeclareFaultsRecoveredParams {
    pub recoveries: Vec<RecoveryDeclaration>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct RecoveryDeclaration {
    /// The deadline to which the recovered sectors are assigned, in range [0..WPoStPeriodDeadlines)
    pub deadline: u64,
    /// Partition index within the deadline containing the recovered sectors.
    pub partition: u64,
    /// Sectors in the partition being declared recovered.
    pub sectors: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct CompactPartitionsParams {
    pub deadline: u64,
    pub partitions: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct CompactSectorNumbersParams {
    pub mask_sector_numbers: BitField,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ReportConsensusFaultParams {
    #[serde(with = "strict_bytes")]
    pub header1: Vec<u8>,
    #[serde(with = "strict_bytes")]
    pub header2: Vec<u8>,
    #[serde(with = "strict_bytes")]
    pub header_extra: Vec<u8>,
}

#[derive(Clone, Serialize_tuple, Deserialize_tuple)]
pub struct WithdrawBalanceParams {
    pub amount_requested: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct WithdrawBalanceReturn {
    pub amount_withdrawn: TokenAmount,
}

#[derive(Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct WorkerKeyChange {
    /// Must be an ID address
    pub new_worker: Address,
    pub effective_at: ChainEpoch,
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct PreCommitSectorParams {
    pub seal_proof: RegisteredSealProof,
    pub sector_number: SectorNumber,
    /// CommR
    pub sealed_cid: Cid,
    pub seal_rand_epoch: ChainEpoch,
    pub deal_ids: Vec<DealID>,
    pub expiration: ChainEpoch,
    /// Deprecated:
    /// Whether to replace a "committed capacity" no-deal sector (requires non-empty DealIDs)
    pub replace_capacity: bool,
    /// Deprecated:
    /// The committed capacity sector to replace, and its deadline/partition location
    pub replace_sector_deadline: u64,
    pub replace_sector_partition: u64,
    pub replace_sector_number: SectorNumber,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct PreCommitSectorBatchParams {
    pub sectors: Vec<PreCommitSectorParams>,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct PreCommitSectorBatchParams2 {
    pub sectors: Vec<SectorPreCommitInfo>,
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct SectorPreCommitInfo {
    pub seal_proof: RegisteredSealProof,
    pub sector_number: SectorNumber,
    /// CommR
    pub sealed_cid: Cid,
    pub seal_rand_epoch: ChainEpoch,
    pub deal_ids: Vec<DealID>,
    pub expiration: ChainEpoch,
    /// CommD
    pub unsealed_cid: CompactCommD,
}

/// Information stored on-chain for a pre-committed sector.
#[derive(Debug, PartialEq, Eq, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct SectorPreCommitOnChainInfo {
    pub info: SectorPreCommitInfo,
    pub pre_commit_deposit: TokenAmount,
    pub pre_commit_epoch: ChainEpoch,
}

/// Information stored on-chain for a proven sector.
#[derive(Debug, Default, PartialEq, Eq, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct SectorOnChainInfo {
    pub sector_number: SectorNumber,
    /// The seal proof type implies the PoSt proofs
    pub seal_proof: RegisteredSealProof,
    /// CommR
    pub sealed_cid: Cid,
    pub deprecated_deal_ids: Vec<DealID>,
    /// Epoch during which the sector proof was accepted
    pub activation: ChainEpoch,
    /// Epoch during which the sector expires
    pub expiration: ChainEpoch,
    /// Integral of active deals over sector lifetime
    #[serde(with = "bigint_ser")]
    pub deal_weight: DealWeight,
    /// Integral of active verified deals over sector lifetime
    #[serde(with = "bigint_ser")]
    pub verified_deal_weight: DealWeight,
    /// Pledge collected to commit this sector
    pub initial_pledge: TokenAmount,
    /// Expected one day projection of reward for sector computed at activation / update / extension time
    /// This field is no longer used and all new sectors will have this set to zero. It can be
    /// removed in a future state migration.
    pub expected_day_reward: Option<TokenAmount>,
    /// Expected twenty day projection of reward for sector computed at activation / update / extension time
    /// This field is no longer used and all new sectors will have this set to zero. It can be
    /// removed in a future state migration.
    pub expected_storage_pledge: Option<TokenAmount>,
    /// Epoch at which this sector's power was most recently updated
    pub power_base_epoch: ChainEpoch,
    /// Maximum day reward this sector has had in previous iterations (zero for brand new sectors)
    /// This field is no longer used and all new sectors will have this set to zero. It can be
    /// removed in a future state migration.
    pub replaced_day_reward: Option<TokenAmount>,
    /// The original SealedSectorCID, only gets set on the first ReplicaUpdate
    pub sector_key_cid: Option<Cid>,
    /// Additional flags, see [`SectorOnChainInfoFlags`]
    pub flags: SectorOnChainInfoFlags,
    /// The total fee payable per day for this sector. The value of this field is set at the time of
    /// sector activation, extension and whenever a sector's QAP is changed. This fee is payable for
    /// the lifetime of the sector and is aggregated in the deadline's `daily_fee` field.
    ///
    /// This field is not included in the serialised form of the struct prior to the activation of
    /// FIP-0100, and is added as the 16th element of the array after that point only for new sectors
    /// or sectors that are updated after that point. For old sectors, the value of this field will
    /// always be zero.
    #[serde(default)]
    pub daily_fee: TokenAmount,
}

bitflags::bitflags! {
    #[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
    #[serde(transparent)]
    pub struct SectorOnChainInfoFlags: u32 {
        /// QA power mechanism introduced in FIP-0045
        const SIMPLE_QA_POWER = 0x1;
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct Fault {
    pub miner: Address,
    pub fault: ChainEpoch,
}

// * Added in v2 -- param was previously a big int.
#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ApplyRewardParams {
    pub reward: TokenAmount,
    pub penalty: TokenAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Copy, Serialize_tuple, Deserialize_tuple)]
pub struct DisputeWindowedPoStParams {
    pub deadline: u64,
    pub post_index: u64, // only one is allowed at a time to avoid loading too many sector infos.
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ProveReplicaUpdates3Params {
    pub sector_updates: Vec<SectorUpdateManifest>,
    // Proofs for each sector, parallel to activation manifests.
    // Exactly one of sector_proofs or aggregate_proof must be non-empty.
    pub sector_proofs: Vec<RawBytes>,
    // Aggregate proof for all sectors.
    // Exactly one of sector_proofs or aggregate_proof must be non-empty.
    pub aggregate_proof: RawBytes,
    // The proof type for all sector update proofs, individually or before aggregation.
    pub update_proofs_type: RegisteredUpdateProof,
    // The proof type for the aggregate proof (must be None if no aggregate proof).
    pub aggregate_proof_type: Option<RegisteredAggregateProof>,
    // Whether to abort if any sector update activation fails.
    pub require_activation_success: bool,
    // Whether to abort if any notification returns a non-zero exit code.
    pub require_notification_success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SectorUpdateManifest {
    pub sector: SectorNumber,
    pub deadline: u64,
    pub partition: u64,
    pub new_sealed_cid: Cid, // CommR
    // Declaration of all pieces that make up the new sector data, in order.
    // Until we support re-snap, pieces must all be new because the sector was previously empty.
    // Implicit "zero" piece fills any remaining capacity.
    // These pieces imply the new unsealed sector CID.
    pub pieces: Vec<PieceActivationManifest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct ProveReplicaUpdates3Return {
    pub activation_results: BatchReturn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ChangeBeneficiaryParams {
    pub new_beneficiary: Address,
    pub new_quota: TokenAmount,
    pub new_expiration: ChainEpoch,
}

impl ChangeBeneficiaryParams {
    pub fn new(beneficiary: Address, quota: TokenAmount, expiration: ChainEpoch) -> Self {
        ChangeBeneficiaryParams {
            new_beneficiary: beneficiary,
            new_quota: quota,
            new_expiration: expiration,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ActiveBeneficiary {
    pub beneficiary: Address,
    pub term: BeneficiaryTerm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct GetBeneficiaryReturn {
    pub active: ActiveBeneficiary,
    pub proposed: Option<PendingBeneficiaryChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct GetOwnerReturn {
    pub owner: Address,
    pub proposed: Option<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct IsControllingAddressParam {
    pub address: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct IsControllingAddressReturn {
    pub is_controlling: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct GetSectorSizeReturn {
    pub sector_size: SectorSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct GetAvailableBalanceReturn {
    pub available_balance: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct GetVestingFundsReturn {
    pub vesting_funds: Vec<(ChainEpoch, TokenAmount)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct GetPeerIDReturn {
    #[serde(with = "strict_bytes")]
    pub peer_id: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct GetMultiaddrsReturn {
    pub multi_addrs: Vec<BytesDe>,
}

// Notification of change committed to one or more sectors.
// The relevant state must be already committed so the receiver can observe any impacts
// at the sending miner actor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct SectorContentChangedParams {
    // Distinct sectors with changed content.
    pub sectors: Vec<SectorChanges>,
}

// Description of changes to one sector's content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SectorChanges {
    // Identifier of sector being updated.
    pub sector: SectorNumber,
    // Minimum epoch until which the data is committed to the sector.
    // Note the sector may later be extended without necessarily another notification.
    pub minimum_commitment_epoch: ChainEpoch,
    // Information about some pieces added to (or retained in) the sector.
    // This may be only a subset of sector content.
    // Inclusion here does not mean the piece was definitely absent previously.
    // Exclusion here does not mean a piece has been removed since a prior notification.
    pub added: Vec<PieceChange>,
}

// Description of a piece of data committed to a sector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct PieceChange {
    pub data: Cid,
    pub size: PaddedPieceSize,
    // A receiver-specific identifier.
    // E.g. an encoded deal ID which the provider claims this piece satisfies.
    pub payload: RawBytes,
}

// For each piece in each sector, the notifee returns an exit code and
// (possibly-empty) result data.
// The miner actor will pass through results to its caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct SectorContentChangedReturn {
    // A result for each sector that was notified, in the same order.
    pub sectors: Vec<SectorReturn>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct SectorReturn {
    // A result for each piece for the sector that was notified, in the same order.
    pub added: Vec<PieceReturn>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct PieceReturn {
    // Indicates whether the receiver accepted the notification.
    // The caller is free to ignore this, but may chose to abort and roll back.
    pub accepted: bool,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
pub struct MaxTerminationFeeParams {
    #[serde(with = "bigint_ser")]
    pub power: StoragePower,
    pub initial_pledge: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct MaxTerminationFeeReturn {
    pub max_fee: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Eq, PartialEq)]
#[serde(transparent)]
pub struct InitialPledgeReturn {
    pub initial_pledge: TokenAmount,
}

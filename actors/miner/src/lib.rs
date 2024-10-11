// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cmp;
use std::cmp::max;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Neg;

use anyhow::{anyhow, Error};
use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use cid::multihash::Code::Blake2b256;
use cid::Cid;
use fvm_ipld_bitfield::{BitField, Validate};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{from_slice, BytesDe, CborStore, RawBytes};
use fvm_shared::address::{Address, Payload, Protocol};
use fvm_shared::bigint::{BigInt, Integer};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::*;
use fvm_shared::piece::PieceInfo;
use fvm_shared::randomness::*;
use fvm_shared::sector::{
    AggregateSealVerifyInfo, AggregateSealVerifyProofAndInfos, InteractiveSealRandomness,
    PoStProof, RegisteredAggregateProof, RegisteredPoStProof, RegisteredSealProof,
    RegisteredUpdateProof, ReplicaUpdateInfo, SealRandomness, SealVerifyInfo, SectorID, SectorInfo,
    SectorNumber, SectorSize, StoragePower, WindowPoStVerifyInfo,
};
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use itertools::Itertools;
use log::{error, info, warn};
use num_derive::FromPrimitive;
use num_traits::{Signed, Zero};

pub use beneficiary::*;
pub use bitfield_queue::*;
pub use commd::*;
pub use deadline_assignment::*;
pub use deadline_info::*;
pub use deadline_state::*;
pub use deadlines::*;
pub use expiration_queue::*;
use fil_actors_runtime::cbor::{serialize, serialize_vec};
use fil_actors_runtime::reward::{FilterEstimate, ThisEpochRewardReturn};
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::policy_constants::MAX_SECTOR_NUMBER;
use fil_actors_runtime::runtime::{ActorCode, DomainSeparationTag, Policy, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, deserialize_block, extract_send_result, util, ActorContext,
    ActorDowncast, ActorError, AsActorError, BatchReturn, BatchReturnGen, DealWeight,
    BURNT_FUNDS_ACTOR_ADDR, INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
pub use monies::*;
pub use partition_state::*;
pub use policy::*;
pub use quantize::*;
pub use sector_map::*;
pub use sectors::*;
pub use state::*;
pub use termination::*;
pub use types::*;
pub use vesting_state::*;

use crate::ext::market::NO_ALLOCATION_ID;
use crate::notifications::{notify_data_consumers, ActivationNotifications};

// The following errors are particular cases of illegal state.
// They're not expected to ever happen, but if they do, distinguished codes can help us
// diagnose the problem.

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

mod beneficiary;
mod bitfield_queue;
mod commd;
mod deadline_assignment;
mod deadline_info;
mod deadline_state;
mod deadlines;
mod emit;
mod expiration_queue;
#[doc(hidden)]
pub mod ext;
mod monies;
mod notifications;
mod partition_state;
mod policy;
mod quantize;
mod sector_map;
mod sectors;
mod state;
mod termination;
pub mod testing;
mod types;
mod vesting_state;

/// Storage Miner actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    ControlAddresses = 2,
    ChangeWorkerAddress = 3,
    ChangePeerID = 4,
    SubmitWindowedPoSt = 5,
    //PreCommitSector = 6, // Deprecated
    //ProveCommitSector = 7, // Deprecated
    ExtendSectorExpiration = 8,
    TerminateSectors = 9,
    DeclareFaults = 10,
    DeclareFaultsRecovered = 11,
    OnDeferredCronEvent = 12,
    CheckSectorProven = 13,
    ApplyRewards = 14,
    ReportConsensusFault = 15,
    WithdrawBalance = 16,
    InternalSectorSetupForPreseal = 17,
    ChangeMultiaddrs = 18,
    CompactPartitions = 19,
    CompactSectorNumbers = 20,
    ConfirmChangeWorkerAddress = 21,
    RepayDebt = 22,
    ChangeOwnerAddress = 23,
    DisputeWindowedPoSt = 24,
    //PreCommitSectorBatch = 25, // Deprecated
    ProveCommitAggregate = 26,
    ProveReplicaUpdates = 27,
    PreCommitSectorBatch2 = 28,
    //ProveReplicaUpdates2 = 29, // Deprecated
    ChangeBeneficiary = 30,
    GetBeneficiary = 31,
    ExtendSectorExpiration2 = 32,
    // MovePartitions = 33,
    ProveCommitSectors3 = 34,
    ProveReplicaUpdates3 = 35,
    ProveCommitSectorsNI = 36,
    // Method numbers derived from FRC-0042 standards
    ChangeWorkerAddressExported = frc42_dispatch::method_hash!("ChangeWorkerAddress"),
    ChangePeerIDExported = frc42_dispatch::method_hash!("ChangePeerID"),
    WithdrawBalanceExported = frc42_dispatch::method_hash!("WithdrawBalance"),
    ChangeMultiaddrsExported = frc42_dispatch::method_hash!("ChangeMultiaddrs"),
    ConfirmChangeWorkerAddressExported = frc42_dispatch::method_hash!("ConfirmChangeWorkerAddress"),
    RepayDebtExported = frc42_dispatch::method_hash!("RepayDebt"),
    ChangeOwnerAddressExported = frc42_dispatch::method_hash!("ChangeOwnerAddress"),
    ChangeBeneficiaryExported = frc42_dispatch::method_hash!("ChangeBeneficiary"),
    GetBeneficiaryExported = frc42_dispatch::method_hash!("GetBeneficiary"),
    GetOwnerExported = frc42_dispatch::method_hash!("GetOwner"),
    IsControllingAddressExported = frc42_dispatch::method_hash!("IsControllingAddress"),
    GetSectorSizeExported = frc42_dispatch::method_hash!("GetSectorSize"),
    GetAvailableBalanceExported = frc42_dispatch::method_hash!("GetAvailableBalance"),
    GetVestingFundsExported = frc42_dispatch::method_hash!("GetVestingFunds"),
    GetPeerIDExported = frc42_dispatch::method_hash!("GetPeerID"),
    GetMultiaddrsExported = frc42_dispatch::method_hash!("GetMultiaddrs"),
}

pub const SECTOR_CONTENT_CHANGED: MethodNum = frc42_dispatch::method_hash!("SectorContentChanged");

pub const ERR_BALANCE_INVARIANTS_BROKEN: ExitCode = ExitCode::new(1000);
pub const ERR_NOTIFICATION_SEND_FAILED: ExitCode = ExitCode::new(1001);
pub const ERR_NOTIFICATION_RECEIVER_ABORTED: ExitCode = ExitCode::new(1002);
pub const ERR_NOTIFICATION_RESPONSE_INVALID: ExitCode = ExitCode::new(1003);
pub const ERR_NOTIFICATION_REJECTED: ExitCode = ExitCode::new(1004);

/// Miner Actor
/// here in order to update the Power Actor to v3.
pub struct Actor;

impl Actor {
    pub fn constructor(
        rt: &impl Runtime,
        params: MinerConstructorParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&INIT_ACTOR_ADDR))?;

        check_control_addresses(rt.policy(), &params.control_addresses)?;
        check_peer_info(rt.policy(), &params.peer_id, &params.multi_addresses)?;
        check_valid_post_proof_type(rt.policy(), params.window_post_proof_type)?;

        let owner = rt.resolve_address(&params.owner).ok_or_else(|| {
            actor_error!(illegal_argument, "unable to resolve owner address: {}", params.owner)
        })?;

        let worker = resolve_worker_address(rt, params.worker)?;
        let control_addresses: Vec<_> = params
            .control_addresses
            .into_iter()
            .map(|address| {
                rt.resolve_address(&address).ok_or_else(|| {
                    actor_error!(illegal_argument, "unable to resolve control address: {}", address)
                })
            })
            .collect::<Result<_, _>>()?;

        let policy = rt.policy();
        let current_epoch = rt.curr_epoch();
        let blake2b = |b: &[u8]| rt.hash_blake2b(b);
        let offset =
            assign_proving_period_offset(policy, rt.message().receiver(), current_epoch, blake2b)
                .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_SERIALIZATION,
                    "failed to assign proving period offset",
                )
            })?;

        let period_start = current_proving_period_start(policy, current_epoch, offset);
        if period_start > current_epoch {
            return Err(actor_error!(
                illegal_state,
                "computed proving period start {} after current epoch {}",
                period_start,
                current_epoch
            ));
        }

        let deadline_idx = current_deadline_index(policy, current_epoch, period_start);
        if deadline_idx >= policy.wpost_period_deadlines {
            return Err(actor_error!(
                illegal_state,
                "computed proving deadline index {} invalid",
                deadline_idx
            ));
        }

        let info = MinerInfo::new(
            owner,
            worker,
            control_addresses,
            params.peer_id,
            params.multi_addresses,
            params.window_post_proof_type,
        )?;
        let info_cid = rt.store().put_cbor(&info, Blake2b256).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to construct illegal state")
        })?;

        let st = State::new(policy, rt.store(), info_cid, period_start, deadline_idx)?;
        rt.create(&st)?;
        Ok(())
    }

    /// Returns the "controlling" addresses: the owner, the worker, and all control addresses
    fn control_addresses(rt: &impl Runtime) -> Result<GetControlAddressesReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let state: State = rt.state()?;
        let info = get_miner_info(rt.store(), &state)?;
        Ok(GetControlAddressesReturn {
            owner: info.owner,
            worker: info.worker,
            control_addresses: info.control_addresses,
        })
    }

    /// Returns the owner address, as well as the proposed new owner (if any).
    fn get_owner(rt: &impl Runtime) -> Result<GetOwnerReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let state: State = rt.state()?;
        let info = get_miner_info(rt.store(), &state)?;
        Ok(GetOwnerReturn { owner: info.owner, proposed: info.pending_owner_address })
    }

    /// Returns whether the provided address is "controlling".
    /// The "controlling" addresses are the Owner, the Worker, and all Control Addresses.
    fn is_controlling_address(
        rt: &impl Runtime,
        params: IsControllingAddressParam,
    ) -> Result<IsControllingAddressReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let input = match rt.resolve_address(&params.address) {
            Some(a) => Address::new_id(a),
            None => return Ok(IsControllingAddressReturn { is_controlling: false }),
        };
        let state: State = rt.state()?;
        let info = get_miner_info(rt.store(), &state)?;
        let is_controlling =
            info.control_addresses.iter().chain(&[info.worker, info.owner]).any(|a| *a == input);

        Ok(IsControllingAddressReturn { is_controlling })
    }

    /// Returns the miner's sector size.
    fn get_sector_size(rt: &impl Runtime) -> Result<GetSectorSizeReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let state: State = rt.state()?;
        let sector_size = get_miner_info(rt.store(), &state)?.sector_size;
        Ok(GetSectorSizeReturn { sector_size })
    }

    /// Returns the available balance of this miner.
    /// This is calculated as actor balance - (vesting funds + pre-commit deposit + ip requirement + fee debt)
    /// Can go negative if the miner is in IP debt.
    fn get_available_balance(rt: &impl Runtime) -> Result<GetAvailableBalanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let state: State = rt.state()?;
        let available_balance =
            state.get_available_balance(&rt.current_balance()).map_err(|e| {
                actor_error!(illegal_state, "failed to calculate available balance: {}", e)
            })?;
        Ok(GetAvailableBalanceReturn { available_balance })
    }

    /// Returns the funds vesting in this miner as a list of (vesting_epoch, vesting_amount) tuples.
    fn get_vesting_funds(rt: &impl Runtime) -> Result<GetVestingFundsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let state: State = rt.state()?;
        let vesting_funds = state
            .load_vesting_funds(rt.store())
            .map_err(|e| actor_error!(illegal_state, "failed to load vesting funds: {}", e))?;
        let ret = vesting_funds.funds.into_iter().map(|v| (v.epoch, v.amount)).collect_vec();
        Ok(GetVestingFundsReturn { vesting_funds: ret })
    }

    /// Will ALWAYS overwrite the existing control addresses with the control addresses passed in the params.
    /// If an empty addresses vector is passed, the control addresses will be cleared.
    /// A worker change will be scheduled if the worker passed in the params is different from the existing worker.
    fn change_worker_address(
        rt: &impl Runtime,
        params: ChangeWorkerAddressParams,
    ) -> Result<(), ActorError> {
        check_control_addresses(rt.policy(), &params.new_control_addresses)?;

        let new_worker = Address::new_id(resolve_worker_address(rt, params.new_worker)?);
        let control_addresses: Vec<Address> = params
            .new_control_addresses
            .into_iter()
            .map(|address| {
                rt.resolve_address(&address).ok_or_else(|| {
                    actor_error!(illegal_argument, "unable to resolve control address: {}", address)
                })
            })
            .map(|id_result| id_result.map(Address::new_id))
            .collect::<Result<_, _>>()?;

        rt.transaction(|state: &mut State, rt| {
            let mut info = get_miner_info(rt.store(), state)?;

            // Only the Owner is allowed to change the new_worker and control addresses.
            rt.validate_immediate_caller_is(std::iter::once(&info.owner))?;

            // save the new control addresses
            info.control_addresses = control_addresses;

            // save new_worker addr key change request
            if new_worker != info.worker && info.pending_worker_key.is_none() {
                info.pending_worker_key = Some(WorkerKeyChange {
                    new_worker,
                    effective_at: rt.curr_epoch() + rt.policy().worker_key_change_delay,
                })
            }

            state.save_info(rt.store(), &info).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "could not save miner info")
            })?;

            Ok(())
        })?;

        Ok(())
    }

    /// Triggers a worker address change if a change has been requested and its effective epoch has arrived.
    fn confirm_change_worker_address(rt: &impl Runtime) -> Result<(), ActorError> {
        rt.transaction(|state: &mut State, rt| {
            let mut info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(std::iter::once(&info.owner))?;

            process_pending_worker(&mut info, rt, state)?;

            Ok(())
        })
    }

    /// Proposes or confirms a change of owner address.
    /// If invoked by the current owner, proposes a new owner address for confirmation. If the proposed address is the
    /// current owner address, revokes any existing proposal.
    /// If invoked by the previously proposed address, with the same proposal, changes the current owner address to be
    /// that proposed address.
    fn change_owner_address(
        rt: &impl Runtime,
        params: ChangeOwnerAddressParams,
    ) -> Result<(), ActorError> {
        let new_address = params.new_owner;
        // * Cannot match go checking for undef address, does go impl allow this to be
        // * deserialized over the wire? If so, a workaround will be needed

        if !matches!(new_address.protocol(), Protocol::ID) {
            return Err(actor_error!(illegal_argument, "owner address must be an ID address"));
        }

        rt.transaction(|state: &mut State, rt| {
            let mut info = get_miner_info(rt.store(), state)?;

            if rt.message().caller() == info.owner || info.pending_owner_address.is_none() {
                rt.validate_immediate_caller_is(std::iter::once(&info.owner))?;
                info.pending_owner_address = Some(new_address);
            } else {
                let pending_address = info.pending_owner_address.unwrap();
                rt.validate_immediate_caller_is(std::iter::once(&pending_address))?;
                if new_address != pending_address {
                    return Err(actor_error!(
                        illegal_argument,
                        "expected confirmation of {} got {}",
                        pending_address,
                        new_address
                    ));
                }

                // Change beneficiary address to new owner if current beneficiary address equal to old owner address
                if info.beneficiary == info.owner {
                    info.beneficiary = pending_address;
                }
                // Cancel pending beneficiary term change when the owner changes
                info.pending_beneficiary_term = None;

                // Set the new owner address
                info.owner = pending_address;
            }

            // Clear any no-op change
            if let Some(p_addr) = info.pending_owner_address {
                if p_addr == info.owner {
                    info.pending_owner_address = None;
                }
            }

            state.save_info(rt.store(), &info).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save miner info")
            })?;

            Ok(())
        })
    }

    /// Returns the Peer ID for this miner.
    fn get_peer_id(rt: &impl Runtime) -> Result<GetPeerIDReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let state: State = rt.state()?;
        let peer_id = get_miner_info(rt.store(), &state)?.peer_id;
        Ok(GetPeerIDReturn { peer_id })
    }

    fn change_peer_id(rt: &impl Runtime, params: ChangePeerIDParams) -> Result<(), ActorError> {
        let policy = rt.policy();
        check_peer_info(policy, &params.new_id, &[])?;

        rt.transaction(|state: &mut State, rt| {
            let mut info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            info.peer_id = params.new_id;
            state.save_info(rt.store(), &info).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "could not save miner info")
            })?;

            Ok(())
        })?;
        Ok(())
    }

    /// Returns the multiaddresses set for this miner.
    fn get_multiaddresses(rt: &impl Runtime) -> Result<GetMultiaddrsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let state: State = rt.state()?;
        let multi_addrs = get_miner_info(rt.store(), &state)?.multi_address;
        Ok(GetMultiaddrsReturn { multi_addrs })
    }

    fn change_multiaddresses(
        rt: &impl Runtime,
        params: ChangeMultiaddrsParams,
    ) -> Result<(), ActorError> {
        let policy = rt.policy();
        check_peer_info(policy, &[], &params.new_multi_addrs)?;

        rt.transaction(|state: &mut State, rt| {
            let mut info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            info.multi_address = params.new_multi_addrs;
            state.save_info(rt.store(), &info).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "could not save miner info")
            })?;

            Ok(())
        })?;
        Ok(())
    }

    /// Invoked by miner's worker address to submit their fallback post
    fn submit_windowed_post(
        rt: &impl Runtime,
        mut params: SubmitWindowedPoStParams,
    ) -> Result<(), ActorError> {
        let current_epoch = rt.curr_epoch();

        {
            let policy = rt.policy();
            if params.proofs.len() != 1 {
                return Err(actor_error!(
                    illegal_argument,
                    "expected exactly one proof, got {}",
                    params.proofs.len()
                ));
            }

            if check_valid_post_proof_type(policy, params.proofs[0].post_proof).is_err() {
                return Err(actor_error!(
                    illegal_argument,
                    "proof type {:?} not allowed",
                    params.proofs[0].post_proof
                ));
            }

            if params.deadline >= policy.wpost_period_deadlines {
                return Err(actor_error!(
                    illegal_argument,
                    "invalid deadline {} of {}",
                    params.deadline,
                    policy.wpost_period_deadlines
                ));
            }

            if params.chain_commit_rand.0.len() > RANDOMNESS_LENGTH {
                return Err(actor_error!(
                    illegal_argument,
                    "expected at most {} bytes of randomness, got {}",
                    RANDOMNESS_LENGTH,
                    params.chain_commit_rand.0.len()
                ));
            }
        }

        let post_result = rt.transaction(|state: &mut State, rt| {
            let info = get_miner_info(rt.store(), state)?;

            let max_proof_size = info.window_post_proof_type.proof_size().map_err(|e| {
                actor_error!(illegal_state, "failed to determine max window post proof size: {}", e)
            })?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            // Make sure the miner is using the correct proof type.
            if params.proofs[0].post_proof != info.window_post_proof_type {
                return Err(actor_error!(
                    illegal_argument,
                    "expected proof of type {:?}, got {:?}",
                    info.window_post_proof_type,
                    params.proofs[0].post_proof
                ));
            }

            // Make sure the proof size doesn't exceed the max. We could probably check for an exact match, but this is safer.
            let max_size = max_proof_size * params.partitions.len();
            if params.proofs[0].proof_bytes.len() > max_size {
                return Err(actor_error!(
                    illegal_argument,
                    "expected proof to be smaller than {} bytes",
                    max_size
                ));
            }

            // Validate that the miner didn't try to prove too many partitions at once.
            let submission_partition_limit = cmp::min(
                load_partitions_sectors_max(rt.policy(), info.window_post_partition_sectors),
                rt.policy().posted_partitions_max,
            );

            if params.partitions.len() as u64 > submission_partition_limit {
                return Err(actor_error!(
                    illegal_argument,
                    "too many partitions {}, limit {}",
                    params.partitions.len(),
                    submission_partition_limit
                ));
            }
            let current_deadline = state.deadline_info(rt.policy(), current_epoch);

            // Check that the miner state indicates that the current proving deadline has started.
            // This should only fail if the cron actor wasn't invoked, and matters only in case that it hasn't been
            // invoked for a whole proving period, and hence the missed PoSt submissions from the prior occurrence
            // of this deadline haven't been processed yet.
            if !current_deadline.is_open() {
                return Err(actor_error!(
                    illegal_state,
                    "proving period {} not yet open at {}",
                    current_deadline.period_start,
                    current_epoch
                ));
            }

            // The miner may only submit a proof for the current deadline.
            if params.deadline != current_deadline.index {
                return Err(actor_error!(
                    illegal_argument,
                    "invalid deadline {} at epoch {}, expected {}",
                    params.deadline,
                    current_epoch,
                    current_deadline.index
                ));
            }

            // Verify that the PoSt was committed to the chain at most
            // WPoStChallengeLookback+WPoStChallengeWindow in the past.
            if params.chain_commit_epoch < current_deadline.challenge {
                return Err(actor_error!(
                    illegal_argument,
                    "expected chain commit epoch {} to be after {}",
                    params.chain_commit_epoch,
                    current_deadline.challenge
                ));
            }

            if params.chain_commit_epoch >= current_epoch {
                return Err(actor_error!(
                    illegal_argument,
                    "chain commit epoch {} must be less than the current epoch {}",
                    params.chain_commit_epoch,
                    current_epoch
                ));
            }

            // Verify the chain commit randomness
            let comm_rand = rt.get_randomness_from_tickets(
                DomainSeparationTag::PoStChainCommit,
                params.chain_commit_epoch,
                &[],
            )?;
            if Randomness(comm_rand.into()) != params.chain_commit_rand {
                return Err(actor_error!(illegal_argument, "post commit randomness mismatched"));
            }

            let sectors = Sectors::load(rt.store(), &state.sectors).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors")
            })?;

            let mut deadlines =
                state.load_deadlines(rt.store()).map_err(|e| e.wrap("failed to load deadlines"))?;

            let mut deadline = deadlines.load_deadline(rt.store(), params.deadline)?;

            // Record proven sectors/partitions, returning updates to power and the final set of sectors
            // proven/skipped.
            //
            // NOTE: This function does not actually check the proofs but does assume that they're correct. Instead,
            // it snapshots the deadline's state and the submitted proofs at the end of the challenge window and
            // allows third-parties to dispute these proofs.
            //
            // While we could perform _all_ operations at the end of challenge window, we do as we can here to avoid
            // overloading cron.
            let policy = rt.policy();
            let fault_expiration = current_deadline.last() + policy.fault_max_age;
            let post_result = deadline
                .record_proven_sectors(
                    rt.store(),
                    &sectors,
                    info.sector_size,
                    current_deadline.quant_spec(),
                    fault_expiration,
                    &mut params.partitions,
                )
                .map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!(
                            "failed to process post submission for deadline {}",
                            params.deadline
                        ),
                    )
                })?;

            // Make sure we actually proved something.
            let proven_sectors = &post_result.sectors - &post_result.ignored_sectors;
            if proven_sectors.is_empty() {
                // Abort verification if all sectors are (now) faults. There's nothing to prove.
                // It's not rational for a miner to submit a Window PoSt marking *all* non-faulty sectors as skipped,
                // since that will just cause them to pay a penalty at deadline end that would otherwise be zero
                // if they had *not* declared them.
                return Err(actor_error!(
                    illegal_argument,
                    "cannot prove partitions with no active sectors"
                ));
            }
            // If we're not recovering power, record the proof for optimistic verification.
            if post_result.recovered_power.is_zero() {
                deadline
                    .record_post_proofs(rt.store(), &post_result.partitions, &params.proofs)
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "failed to record proof for optimistic verification",
                        )
                    })?
            } else {
                // Load sector infos for proof, substituting a known-good sector for known-faulty sectors.
                // Note: this is slightly sub-optimal, loading info for the recovering sectors again after they were already
                // loaded above.
                let sector_infos = sectors
                    .load_for_proof(&post_result.sectors, &post_result.ignored_sectors)
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "failed to load sectors for post verification",
                        )
                    })?;
                if !verify_windowed_post(
                    rt,
                    current_deadline.challenge,
                    &sector_infos,
                    params.proofs,
                )
                .map_err(|e| e.wrap("window post failed"))?
                {
                    return Err(actor_error!(illegal_argument, "invalid post was submitted"));
                }
            }

            let deadline_idx = params.deadline;
            deadlines.update_deadline(policy, rt.store(), params.deadline, &deadline).map_err(
                |e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to update deadline {}", deadline_idx),
                    )
                },
            )?;

            state.save_deadlines(rt.store(), deadlines).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save deadlines")
            })?;

            Ok(post_result)
        })?;

        // Restore power for recovered sectors. Remove power for new faults.
        // NOTE: It would be permissible to delay the power loss until the deadline closes, but that would require
        // additional accounting state.
        // https://github.com/filecoin-project/specs-actors/issues/414
        request_update_power(rt, post_result.power_delta)?;

        let state: State = rt.state()?;
        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;

        Ok(())
    }
    /// Checks state of the corresponding sector pre-commitments and verifies aggregate proof of replication
    /// of these sectors. If valid, the sectors' deals are activated, sectors are assigned a deadline and charged pledge
    /// and precommit state is removed.
    fn prove_commit_aggregate(
        rt: &impl Runtime,
        params: ProveCommitAggregateParams,
    ) -> Result<(), ActorError> {
        // Validate caller and parameters.
        let state: State = rt.state()?;
        let store = rt.store();
        let policy = rt.policy();
        let info = get_miner_info(store, &state)?;
        rt.validate_immediate_caller_is(
            info.control_addresses.iter().chain(&[info.worker, info.owner]),
        )?;

        let sector_numbers = params.sector_numbers.validate().context_code(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "failed to validate bitfield of sector numbers",
        )?;
        if sector_numbers.is_empty() {
            return Err(actor_error!(illegal_argument, "no sectors"));
        }

        validate_seal_aggregate_proof(&params.aggregate_proof, sector_numbers.len(), policy, true)?;

        // Load and validate pre-commits.
        // Fail if any don't exist, but otherwise continue with valid ones.
        let precommits = state
            .get_precommitted_sectors(store, sector_numbers.iter())
            .context("failed to get precommits")?;

        let allow_deals = true; // Legacy onboarding entry points allow pre-committed deals.
        let all_or_nothing = false;
        let (batch_return, proof_inputs) =
            validate_precommits(rt, &precommits, allow_deals, all_or_nothing)?;

        let miner_actor_id = rt.message().receiver().id().unwrap();
        verify_aggregate_seal(
            rt,
            // All the proof inputs, even for invalid pre-commits,
            // must be provided as witnesses to the aggregate proof.
            &proof_inputs,
            miner_actor_id,
            precommits[0].info.seal_proof,
            RegisteredAggregateProof::SnarkPackV2,
            &params.aggregate_proof,
        )?;

        let valid_precommits: Vec<SectorPreCommitOnChainInfo> =
            batch_return.successes(&precommits).into_iter().cloned().collect();
        let data_activation_inputs: Vec<DealsActivationInput> =
            valid_precommits.iter().map(|x| x.clone().into()).collect();
        let rew = request_current_epoch_block_reward(rt)?;
        let pwr = request_current_total_power(rt)?;
        let circulating_supply = rt.total_fil_circ_supply();
        let pledge_inputs = NetworkPledgeInputs {
            network_qap: pwr.quality_adj_power_smoothed,
            network_baseline: rew.this_epoch_baseline_power,
            circulating_supply,
            epoch_reward: rew.this_epoch_reward_smoothed,
            epochs_since_ramp_start: rt.curr_epoch() - pwr.ramp_start_epoch,
            ramp_duration_epochs: pwr.ramp_duration_epochs,
        };

        /*
           For all sectors
           - CommD was specified at precommit
           - If deal IDs were specified at precommit the CommD was checked against them
           Therefore CommD on precommit has already been provided and checked so no further processing needed
        */
        let compute_commd = false;
        let (batch_return, activated_data) =
            activate_sectors_deals(rt, &data_activation_inputs, compute_commd)?;
        let activated_precommits = batch_return.successes(&valid_precommits);

        activate_new_sector_infos(
            rt,
            activated_precommits.clone(),
            activated_data.clone(),
            &pledge_inputs,
            &info,
        )?;

        for (pc, data) in activated_precommits.iter().zip(activated_data.iter()) {
            let unsealed_cid = pc.info.unsealed_cid.0;
            emit::sector_activated(rt, pc.info.sector_number, unsealed_cid, &data.pieces)?;
        }

        // The aggregate fee is paid on the sectors successfully proven.
        pay_aggregate_seal_proof_fee(rt, valid_precommits.len())?;
        Ok(())
    }

    fn prove_replica_updates<RT>(
        rt: &RT,
        params: ProveReplicaUpdatesParams,
    ) -> Result<BitField, ActorError>
    where
        // + Clone because we messed up and need to keep a copy around between transactions.
        // https://github.com/filecoin-project/builtin-actors/issues/133
        RT::Blockstore: Clone,
        RT: Runtime,
    {
        // In this entry point, the unsealed CID is computed from deals via the market actor.
        // A future entry point will take the unsealed CID as parameter
        let updates = params
            .updates
            .into_iter()
            .map(|ru| ReplicaUpdateInner {
                sector_number: ru.sector_number,
                deadline: ru.deadline,
                partition: ru.partition,
                new_sealed_cid: ru.new_sealed_cid,
                new_unsealed_cid: None, // Unknown
                deals: ru.deals,
                update_proof_type: ru.update_proof_type,
                replica_proof: ru.replica_proof,
            })
            .collect();
        Self::prove_replica_updates_inner(rt, updates)
    }

    fn prove_replica_updates_inner<RT>(
        rt: &RT,
        updates: Vec<ReplicaUpdateInner>,
    ) -> Result<BitField, ActorError>
    where
        RT::Blockstore: Blockstore,
        RT: Runtime,
    {
        let state: State = rt.state()?;
        let store = rt.store();
        let info = get_miner_info(store, &state)?;

        rt.validate_immediate_caller_is(
            info.control_addresses.iter().chain(&[info.owner, info.worker]),
        )?;

        let mut sectors = Sectors::load(&store, &state.sectors)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors array")?;
        let mut sector_infos = Vec::with_capacity(updates.len());
        for update in &updates {
            sector_infos.push(sectors.must_get(update.sector_number)?);
        }

        // Validate inputs
        let require_deals = true; // Legacy PRU requires deals to be specified in the update.
        let all_or_nothing = false; // Skip invalid updates.
        let (batch_return, update_sector_infos) = validate_replica_updates(
            &updates,
            &sector_infos,
            &state,
            info.sector_size,
            rt.policy(),
            rt.curr_epoch(),
            store,
            require_deals,
            all_or_nothing,
        )?;
        // Drop invalid inputs.
        let update_sector_infos: Vec<UpdateAndSectorInfo> =
            batch_return.successes(&update_sector_infos).into_iter().cloned().collect();

        let data_activation_inputs: Vec<DealsActivationInput> =
            update_sector_infos.iter().map_into().collect();

        /*
           - no CommD was specified on input so it must be computed for the first time here
        */
        let compute_commd = true;
        let (batch_return, data_activations) =
            activate_sectors_deals(rt, &data_activation_inputs, compute_commd)?;

        // associate the successfully activated sectors with the ReplicaUpdateInner and SectorOnChainInfo
        let validated_updates: Vec<(&UpdateAndSectorInfo, DataActivationOutput)> = batch_return
            .successes(&update_sector_infos)
            .into_iter()
            .zip(data_activations)
            .collect();

        if validated_updates.is_empty() {
            return Err(actor_error!(illegal_argument, "no valid updates"));
        }

        // Errors past this point cause the prove_replica_updates call to fail (no more skipping sectors)
        // Group inputs by deadline
        let mut updated_sectors: Vec<SectorNumber> = Vec::new();
        let mut decls_by_deadline = BTreeMap::<u64, Vec<ReplicaUpdateStateInputs>>::new();
        let mut deadlines_to_load = Vec::<u64>::new();
        for (usi, data_activation) in &validated_updates {
            updated_sectors.push(usi.update.sector_number);
            let dl = usi.update.deadline;
            if !decls_by_deadline.contains_key(&dl) {
                deadlines_to_load.push(dl);
            }

            let computed_commd = CompactCommD::new(data_activation.unsealed_cid)
                .get_cid(usi.sector_info.seal_proof)?;
            let proof_inputs = ReplicaUpdateInfo {
                update_proof_type: usi.update.update_proof_type,
                new_sealed_cid: usi.update.new_sealed_cid,
                old_sealed_cid: usi.sector_info.sealed_cid,
                new_unsealed_cid: computed_commd,
                proof: usi.update.replica_proof.clone().into(),
            };
            rt.verify_replica_update(&proof_inputs).with_context_code(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                || {
                    format!(
                        "failed to verify replica proof for sector {}",
                        usi.sector_info.sector_number
                    )
                },
            )?;

            let activated_data = ReplicaUpdateActivatedData {
                seal_cid: usi.update.new_sealed_cid,
                unverified_space: data_activation.unverified_space.clone(),
                verified_space: data_activation.verified_space.clone(),
            };
            decls_by_deadline.entry(dl).or_default().push(ReplicaUpdateStateInputs {
                deadline: usi.update.deadline,
                partition: usi.update.partition,
                sector_info: usi.sector_info,
                activated_data,
            });

            emit::sector_updated(
                rt,
                usi.update.sector_number,
                data_activation.unsealed_cid,
                &data_activation.pieces,
            )?;
        }

        let (power_delta, pledge_delta) = update_replica_states(
            rt,
            &decls_by_deadline,
            validated_updates.len(),
            &mut sectors,
            info.sector_size,
        )?;

        notify_pledge_changed(rt, &pledge_delta)?;
        request_update_power(rt, power_delta)?;

        let updated_bitfield = BitField::try_from_bits(updated_sectors)
            .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "invalid sector number")?;
        Ok(updated_bitfield)
    }

    fn prove_replica_updates3(
        rt: &impl Runtime,
        params: ProveReplicaUpdates3Params,
    ) -> Result<ProveReplicaUpdates3Return, ActorError> {
        let state: State = rt.state()?;
        let store = rt.store();
        let info = get_miner_info(store, &state)?;

        // Validate parameters.
        rt.validate_immediate_caller_is(
            info.control_addresses.iter().chain(&[info.worker, info.owner]),
        )?;
        if !params.sector_proofs.is_empty() {
            if !params.aggregate_proof.is_empty() {
                return Err(actor_error!(
                    illegal_argument,
                    "exactly one of sector proofs or aggregate proof must be non-empty"
                ));
            }
            if params.aggregate_proof_type.is_some() {
                return Err(actor_error!(
                    illegal_argument,
                    "aggregate proof type must be empty when sector proofs are specified"
                ));
            }
        } else {
            if params.aggregate_proof.is_empty() {
                return Err(actor_error!(
                    illegal_argument,
                    "exactly one of sector proofs or aggregate proof must be non-empty"
                ));
            }
            if params.aggregate_proof_type.is_none() {
                return Err(actor_error!(
                    illegal_argument,
                    "aggregate proof type must be specified when aggregate proof is specified"
                ));
            }
        }
        if params.sector_proofs.is_empty() == params.aggregate_proof.is_empty() {
            return Err(actor_error!(
                illegal_argument,
                "exactly one of sector proofs or aggregate proof must be non-empty"
            ));
        }

        // Load sector infos for validation, failing if any don't exist.
        let mut sectors = Sectors::load(&store, &state.sectors)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors array")?;
        let mut sector_infos = Vec::with_capacity(params.sector_updates.len());
        let mut updates = Vec::with_capacity(params.sector_updates.len());
        let mut sector_commds: HashMap<SectorNumber, CompactCommD> =
            HashMap::with_capacity(params.sector_updates.len());
        for (i, update) in params.sector_updates.iter().enumerate() {
            let sector = sectors.must_get(update.sector)?;
            let sector_type = sector.seal_proof;
            sector_infos.push(sector);

            let computed_commd = unsealed_cid_from_pieces(rt, &update.pieces, sector_type)?;

            updates.push(ReplicaUpdateInner {
                sector_number: update.sector,
                deadline: update.deadline,
                partition: update.partition,
                new_sealed_cid: update.new_sealed_cid,
                new_unsealed_cid: Some(computed_commd.get_cid(sector_type)?),
                deals: vec![],
                update_proof_type: params.update_proofs_type,
                // Replica proof may be empty if an aggregate is being proven.
                // Validation needs to accept this empty proof.
                replica_proof: params.sector_proofs.get(i).unwrap_or(&RawBytes::default()).clone(),
            });

            sector_commds.insert(update.sector, computed_commd);
        }

        // Validate inputs.
        let require_deals = false; // No deals can be specified in new replica update.
        let (validation_batch, update_sector_infos) = validate_replica_updates(
            &updates,
            &sector_infos,
            &state,
            info.sector_size,
            rt.policy(),
            rt.curr_epoch(),
            store,
            require_deals,
            params.require_activation_success,
        )?;
        let valid_unproven_usis = validation_batch.successes(&update_sector_infos);
        let valid_manifests = validation_batch.successes(&params.sector_updates);

        // Verify proofs before activating anything.
        let mut proven_manifests: Vec<(&SectorUpdateManifest, &SectorOnChainInfo)> = vec![];
        let mut proven_batch_gen = BatchReturnGen::new(validation_batch.success_count as usize);
        if !params.sector_proofs.is_empty() {
            // Batched proofs, one per sector
            if params.sector_updates.len() != params.sector_proofs.len() {
                return Err(actor_error!(
                    illegal_argument,
                    "mismatched lengths: {} sector updates, {} proofs",
                    params.sector_updates.len(),
                    params.sector_proofs.len()
                ));
            }

            // Note: an alternate factoring here could pull this block out to a separate function,
            // return a BatchReturn, and then extract successes from
            // valid_unproven_usis and valid_manifests, following the pattern used elsewhere.
            for (usi, manifest) in valid_unproven_usis.iter().zip(valid_manifests) {
                let proof_inputs = ReplicaUpdateInfo {
                    update_proof_type: usi.update.update_proof_type,
                    new_sealed_cid: usi.update.new_sealed_cid,
                    old_sealed_cid: usi.sector_info.sealed_cid,
                    new_unsealed_cid: usi.update.new_unsealed_cid.unwrap(), // set above
                    proof: usi.update.replica_proof.clone().into(),
                };
                match rt.verify_replica_update(&proof_inputs) {
                    Ok(_) => {
                        proven_manifests.push((manifest, usi.sector_info));
                        proven_batch_gen.add_success();
                    }
                    Err(e) => {
                        warn!(
                            "failed to verify replica update for sector {}: {e}",
                            usi.sector_info.sector_number
                        );
                        proven_batch_gen.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
                        if params.require_activation_success {
                            return Err(actor_error!(
                                illegal_argument,
                                "invalid proof for sector {} while requiring activation success: {}",
                                usi.sector_info.sector_number,
                                e
                            ));
                        }
                    }
                }
            }
        } else {
            return Err(actor_error!(
                illegal_argument,
                "aggregate update proofs not yet supported"
            ));
            // proven_batch_gen.add_successes(valid_manifests.len());
        }
        if proven_manifests.is_empty() {
            return Err(actor_error!(illegal_argument, "no valid updates"));
        }
        let proven_batch = proven_batch_gen.gen();
        if proven_batch.success_count == 0 {
            return Err(actor_error!(illegal_argument, "no valid proofs specified"));
        }

        // Activate data.
        let data_activation_inputs: Vec<SectorPiecesActivationInput> = proven_manifests
            .iter()
            .map(|(update, info)| SectorPiecesActivationInput {
                piece_manifests: update.pieces.clone(),
                sector_expiry: info.expiration,
                sector_number: info.sector_number,
                sector_type: info.seal_proof,
                expected_commd: None, // CommD was computed, doesn't need checking.
            })
            .collect();

        // Activate data for proven updates.
        let (data_batch, data_activations) =
            activate_sectors_pieces(rt, data_activation_inputs, params.require_activation_success)?;
        if data_batch.success_count == 0 {
            return Err(actor_error!(illegal_argument, "all data activations failed"));
        }

        // Successful data activation is required for sector activation.
        let successful_manifests = data_batch.successes(&proven_manifests);

        let mut state_updates_by_dline = BTreeMap::<u64, Vec<ReplicaUpdateStateInputs>>::new();
        for ((update, sector_info), data_activation) in
            successful_manifests.iter().zip(data_activations)
        {
            let activated_data = ReplicaUpdateActivatedData {
                seal_cid: update.new_sealed_cid,
                unverified_space: data_activation.unverified_space.clone(),
                verified_space: data_activation.verified_space.clone(),
            };
            state_updates_by_dline.entry(update.deadline).or_default().push(
                ReplicaUpdateStateInputs {
                    deadline: update.deadline,
                    partition: update.partition,
                    sector_info,
                    activated_data,
                },
            );
        }

        let (power_delta, pledge_delta) = update_replica_states(
            rt,
            &state_updates_by_dline,
            successful_manifests.len(),
            &mut sectors,
            info.sector_size,
        )?;

        notify_pledge_changed(rt, &pledge_delta)?;
        request_update_power(rt, power_delta)?;

        // Notify data consumers.
        let mut notifications: Vec<ActivationNotifications> = vec![];
        for (update, sector_info) in successful_manifests {
            notifications.push(ActivationNotifications {
                sector_number: update.sector,
                sector_expiration: sector_info.expiration,
                pieces: &update.pieces,
            });

            let pieces: Vec<(Cid, u64)> = update.pieces.iter().map(|x| (x.cid, x.size.0)).collect();

            emit::sector_updated(
                rt,
                update.sector,
                sector_commds.get(&update.sector).unwrap().0,
                &pieces,
            )?;
        }
        notify_data_consumers(rt, &notifications, params.require_notification_success)?;

        let result = util::stack(&[validation_batch, proven_batch, data_batch]);
        Ok(ProveReplicaUpdates3Return { activation_results: result })
    }

    fn dispute_windowed_post(
        rt: &impl Runtime,
        params: DisputeWindowedPoStParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let reporter = rt.message().caller();

        {
            let policy = rt.policy();
            if params.deadline >= policy.wpost_period_deadlines {
                return Err(actor_error!(
                    illegal_argument,
                    "invalid deadline {} of {}",
                    params.deadline,
                    policy.wpost_period_deadlines
                ));
            }
        }
        let current_epoch = rt.curr_epoch();

        // Note: these are going to be slightly inaccurate as time
        // will have moved on from when the post was actually
        // submitted.
        //
        // However, these are estimates _anyways_.
        let epoch_reward = request_current_epoch_block_reward(rt)?;
        let power_total = request_current_total_power(rt)?;

        let (pledge_delta, mut to_burn, power_delta, to_reward) =
            rt.transaction(|st: &mut State, rt| {
                let policy = rt.policy();
                let dl_info = st.deadline_info(policy, current_epoch);

                if !deadline_available_for_optimistic_post_dispute(
                    policy,
                    dl_info.period_start,
                    params.deadline,
                    current_epoch,
                ) {
                    return Err(actor_error!(
                        forbidden,
                        "can only dispute window posts during the dispute window \
                    ({} epochs after the challenge window closes)",
                        policy.wpost_dispute_window
                    ));
                }

                let info = get_miner_info(rt.store(), st)?;
                // --- check proof ---

                // Find the proving period start for the deadline in question.
                let mut pp_start = dl_info.period_start;
                if dl_info.index < params.deadline {
                    pp_start -= policy.wpost_proving_period
                }
                let target_deadline =
                    new_deadline_info(policy, pp_start, params.deadline, current_epoch);
                // Load the target deadline
                let mut deadlines_current = st
                    .load_deadlines(rt.store())
                    .map_err(|e| e.wrap("failed to load deadlines"))?;

                let mut dl_current =
                    deadlines_current.load_deadline(rt.store(), params.deadline)?;

                // Take the post from the snapshot for dispute.
                // This operation REMOVES the PoSt from the snapshot so
                // it can't be disputed again. If this method fails,
                // this operation must be rolled back.
                let (partitions, proofs) =
                    dl_current.take_post_proofs(rt.store(), params.post_index).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "failed to load proof for dispute",
                        )
                    })?;

                // Load the partition info we need for the dispute.
                let mut dispute_info = dl_current
                    .load_partitions_for_dispute(rt.store(), partitions)
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "failed to load partition for dispute",
                        )
                    })?;

                // This includes power that is no longer active (e.g., due to sector terminations).
                // It must only be used for penalty calculations, not power adjustments.
                let penalised_power = dispute_info.disputed_power.clone();

                // Load sectors for the dispute.
                let sectors =
                    Sectors::load(rt.store(), &dl_current.sectors_snapshot).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "failed to load sectors array",
                        )
                    })?;
                let sector_infos = sectors
                    .load_for_proof(&dispute_info.all_sector_nos, &dispute_info.ignored_sector_nos)
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "failed to load sectors to dispute window post",
                        )
                    })?;

                // Check proof, we fail if validation succeeds.
                if verify_windowed_post(rt, target_deadline.challenge, &sector_infos, proofs)? {
                    return Err(actor_error!(illegal_argument, "failed to dispute valid post"));
                } else {
                    info!("Successfully disputed post- window post was invalid");
                }

                // Ok, now we record faults. This always works because
                // we don't allow compaction/moving sectors during the
                // challenge window.
                //
                // However, some of these sectors may have been
                // terminated. That's fine, we'll skip them.
                let fault_expiration_epoch = target_deadline.last() + policy.fault_max_age;
                let power_delta = dl_current
                    .record_faults(
                        rt.store(),
                        &sectors,
                        info.sector_size,
                        quant_spec_for_deadline(policy, &target_deadline),
                        fault_expiration_epoch,
                        &mut dispute_info.disputed_sectors,
                    )
                    .map_err(|e| {
                        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to declare faults")
                    })?;

                deadlines_current
                    .update_deadline(policy, rt.store(), params.deadline, &dl_current)
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to update deadline {}", params.deadline),
                        )
                    })?;

                st.save_deadlines(rt.store(), deadlines_current).map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save deadlines")
                })?;

                // --- penalties ---

                // Calculate the base penalty.
                let penalty_base = pledge_penalty_for_invalid_windowpost(
                    &epoch_reward.this_epoch_reward_smoothed,
                    &power_total.quality_adj_power_smoothed,
                    &penalised_power.qa,
                );

                // Calculate the target reward.
                let reward_target =
                    reward_for_disputed_window_post(info.window_post_proof_type, penalised_power);

                // Compute the target penalty by adding the
                // base penalty to the target reward. We don't
                // take reward out of the penalty as the miner
                // could end up receiving a substantial
                // portion of their fee back as a reward.
                let penalty_target = &penalty_base + &reward_target;
                st.apply_penalty(&penalty_target)
                    .map_err(|e| actor_error!(illegal_state, "failed to apply penalty {}", e))?;
                let (penalty_from_vesting, penalty_from_balance) = st
                    .repay_partial_debt_in_priority_order(
                        rt.store(),
                        current_epoch,
                        &rt.current_balance(),
                    )
                    .map_err(|e| {
                        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to pay debt")
                    })?;

                let to_burn = &penalty_from_vesting + &penalty_from_balance;

                // Now, move as much of the target reward as
                // we can from the burn to the reward.
                let to_reward = std::cmp::min(&to_burn, &reward_target);
                let to_burn = &to_burn - to_reward;
                let pledge_delta = penalty_from_vesting.neg();

                Ok((pledge_delta, to_burn, power_delta, to_reward.clone()))
            })?;

        request_update_power(rt, power_delta)?;
        if !to_reward.is_zero() {
            if let Err(e) =
                extract_send_result(rt.send_simple(&reporter, METHOD_SEND, None, to_reward.clone()))
            {
                error!("failed to send reward: {}", e);
                to_burn += to_reward;
            }
        }

        burn_funds(rt, to_burn)?;
        notify_pledge_changed(rt, &pledge_delta)?;

        let st: State = rt.state()?;
        st.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;
        Ok(())
    }

    /// Pledges the miner to seal and commit some new sectors.
    /// The caller specifies sector numbers, sealed sector CIDs, unsealed sector CID, seal randomness epoch, expiration, and the IDs
    /// of any storage deals contained in the sector data. The storage deal proposals must be already submitted
    /// to the storage market actor.
    /// This method calculates the sector's power, locks a pre-commit deposit for the sector, stores information about the
    /// sector in state and waits for it to be proven or expire.
    fn pre_commit_sector_batch2(
        rt: &impl Runtime,
        params: PreCommitSectorBatchParams2,
    ) -> Result<(), ActorError> {
        Self::pre_commit_sector_batch_inner(
            rt,
            params
                .sectors
                .into_iter()
                .map(|spci| SectorPreCommitInfoInner {
                    seal_proof: spci.seal_proof,
                    sector_number: spci.sector_number,
                    sealed_cid: spci.sealed_cid,
                    seal_rand_epoch: spci.seal_rand_epoch,
                    deal_ids: spci.deal_ids,
                    expiration: spci.expiration,

                    unsealed_cid: spci.unsealed_cid,
                })
                .collect(),
        )
    }

    /// This function combines old and new flows for PreCommit with use Option<CommpactCommD>
    /// The old PreCommits will call this with None, new ones with Some(CompactCommD).
    fn pre_commit_sector_batch_inner(
        rt: &impl Runtime,
        sectors: Vec<SectorPreCommitInfoInner>,
    ) -> Result<(), ActorError> {
        let curr_epoch = rt.curr_epoch();
        {
            let policy = rt.policy();
            if sectors.is_empty() {
                return Err(actor_error!(illegal_argument, "batch empty"));
            }
        }
        // Check per-sector preconditions before opening state transaction or sending other messages.
        let challenge_earliest = curr_epoch - rt.policy().max_pre_commit_randomness_lookback;
        let mut sectors_deals = Vec::with_capacity(sectors.len());
        let mut sector_numbers = BitField::new();
        for precommit in sectors.iter() {
            let set = sector_numbers.get(precommit.sector_number);
            if set {
                return Err(actor_error!(
                    illegal_argument,
                    "duplicate sector number {}",
                    precommit.sector_number
                ));
            }
            sector_numbers.set(precommit.sector_number);

            if !can_pre_commit_seal_proof(rt.policy(), precommit.seal_proof) {
                return Err(actor_error!(
                    illegal_argument,
                    "unsupported seal proof type {}",
                    i64::from(precommit.seal_proof)
                ));
            }
            if precommit.sector_number > MAX_SECTOR_NUMBER {
                return Err(actor_error!(
                    illegal_argument,
                    "sector number {} out of range 0..(2^63-1)",
                    precommit.sector_number
                ));
            }
            // Skip checking if CID is defined because it cannot be so in Rust

            if !is_sealed_sector(&precommit.sealed_cid) {
                return Err(actor_error!(illegal_argument, "sealed CID had wrong prefix"));
            }
            if precommit.seal_rand_epoch >= curr_epoch {
                return Err(actor_error!(
                    illegal_argument,
                    "seal challenge epoch {} must be before now {}",
                    precommit.seal_rand_epoch,
                    curr_epoch
                ));
            }
            if precommit.seal_rand_epoch < challenge_earliest {
                return Err(actor_error!(
                    illegal_argument,
                    "seal challenge epoch {} too old, must be after {}",
                    precommit.seal_rand_epoch,
                    challenge_earliest
                ));
            }

            if let Some(commd) = &precommit.unsealed_cid.0 {
                if !is_unsealed_sector(commd) {
                    return Err(actor_error!(illegal_argument, "unsealed CID had wrong prefix"));
                }
            }

            // Require sector lifetime meets minimum by assuming activation happens at last epoch permitted for seal proof.
            // This could make sector maximum lifetime validation more lenient if the maximum sector limit isn't hit first.
            let max_activation = curr_epoch
                + max_prove_commit_duration(rt.policy(), precommit.seal_proof).unwrap_or_default();
            validate_expiration(
                rt.policy(),
                curr_epoch,
                max_activation,
                precommit.expiration,
                precommit.seal_proof,
            )?;

            sectors_deals.push(ext::market::SectorDeals {
                sector_number: precommit.sector_number,
                sector_type: precommit.seal_proof,
                sector_expiry: precommit.expiration,
                deal_ids: precommit.deal_ids.clone(),
            })
        }
        // gather information from other actors
        let reward_stats = request_current_epoch_block_reward(rt)?;
        let power_total = request_current_total_power(rt)?;
        let verify_return = verify_deals(rt, &sectors_deals)?;
        if verify_return.unsealed_cids.len() != sectors.len() {
            return Err(actor_error!(
                illegal_state,
                "deal weight request returned {} records, expected {}",
                verify_return.unsealed_cids.len(),
                sectors.len()
            ));
        }
        let mut fee_to_burn = TokenAmount::zero();
        let mut needs_cron = false;
        rt.transaction(|state: &mut State, rt| {
            // Aggregate fee applies only when batching.
            if sectors.len() > 1 {
                let aggregate_fee = aggregate_pre_commit_network_fee(sectors.len(), &rt.base_fee());
                // AggregateFee applied to fee debt to consolidate burn with outstanding debts
                state.apply_penalty(&aggregate_fee)
                    .map_err(|e| {
                        actor_error!(
                        illegal_state,
                        "failed to apply penalty: {}",
                        e
                    )
                    })?;
            }
            // available balance already accounts for fee debt so it is correct to call
            // this before RepayDebts. We would have to
            // subtract fee debt explicitly if we called this after.
            let available_balance = state
                .get_available_balance(&rt.current_balance())
                .map_err(|e| {
                    actor_error!(
                        illegal_state,
                        "failed to calculate available balance: {}",
                        e
                    )
                })?;
            fee_to_burn = repay_debts_or_abort(rt, state)?;

            let info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses
                    .iter()
                    .chain(&[info.worker, info.owner]),
            )?;
            let store = rt.store();
            if consensus_fault_active(&info, curr_epoch) {
                return Err(actor_error!(forbidden, "pre-commit not allowed during active consensus fault"));
            }

            let mut chain_infos = Vec::with_capacity(sectors.len());
            let mut total_deposit_required = TokenAmount::zero();
            let mut clean_up_events = Vec::with_capacity(sectors.len());
            let deal_count_max = sector_deals_max(rt.policy(), info.sector_size);

            let sector_weight_for_deposit = qa_power_max(info.sector_size);
            let deposit_req = pre_commit_deposit_for_power(&reward_stats.this_epoch_reward_smoothed, &power_total.quality_adj_power_smoothed, &sector_weight_for_deposit);

            for (i, precommit) in sectors.into_iter().enumerate() {
                // Sector must have the same Window PoSt proof type as the miner's recorded seal type.
                let sector_wpost_proof = precommit.seal_proof
                    .registered_window_post_proof()
                    .map_err(|_e|
                        actor_error!(
                        illegal_argument,
                        "failed to lookup Window PoSt proof type for sector seal proof {}",
                        i64::from(precommit.seal_proof)
                    ))?;
                if sector_wpost_proof != info.window_post_proof_type {
                    return Err(actor_error!(illegal_argument, "sector Window PoSt proof type %d must match miner Window PoSt proof type {} (seal proof type {})", i64::from(sector_wpost_proof), i64::from(info.window_post_proof_type)));
                }
                if precommit.deal_ids.len() as u64 > deal_count_max {
                    return Err(actor_error!(illegal_argument, "too many deals for sector {} > {}", precommit.deal_ids.len(), deal_count_max));
                }

                // 1. verify that precommit.unsealed_cid is correct
                // 2. create a new on_chain_precommit

                // Presence of unsealed CID is checked in the preconditions.
                // It must always be specified from nv22 onwards.
                let declared_commd = precommit.unsealed_cid;
                // This is not a CompactCommD, None means that nothing was computed and nothing needs to be checked
                if let Some(computed_cid) = verify_return.unsealed_cids[i] {
                    // It is possible the computed commd is the zero commd so expand declared_commd
                    if declared_commd.get_cid(precommit.seal_proof)? != computed_cid {
                        return Err(actor_error!(illegal_argument, "computed {:?} and passed {:?} CommDs not equal",
                                computed_cid, declared_commd));
                    }
                }

                let on_chain_precommit = SectorPreCommitInfo {
                    seal_proof: precommit.seal_proof,
                    sector_number: precommit.sector_number,
                    sealed_cid: precommit.sealed_cid,
                    seal_rand_epoch: precommit.seal_rand_epoch,
                    deal_ids: precommit.deal_ids,
                    expiration: precommit.expiration,
                    unsealed_cid: declared_commd,
                };

                // Build on-chain record.
                chain_infos.push(SectorPreCommitOnChainInfo {
                    info: on_chain_precommit,
                    pre_commit_deposit: deposit_req.clone(),
                    pre_commit_epoch: curr_epoch,
                });

                total_deposit_required += &deposit_req;

                // Calculate pre-commit cleanup
                let seal_proof = precommit.seal_proof;
                let msd = max_prove_commit_duration(rt.policy(), seal_proof)
                    .ok_or_else(|| actor_error!(illegal_argument, "no max seal duration set for proof type: {}", i64::from(seal_proof)))?;
                // PreCommitCleanUpDelay > 0 here is critical for the batch verification of proofs. Without it, if a proof arrived exactly on the
                // due epoch, ProveCommitSector would accept it, then the expiry event would remove it, and then
                // ConfirmSectorProofsValid would fail to find it.
                let clean_up_bound = curr_epoch + msd + rt.policy().expired_pre_commit_clean_up_delay;
                clean_up_events.push((clean_up_bound, precommit.sector_number));
            }
            // Batch update actor state.
            if available_balance < total_deposit_required {
                return Err(actor_error!(insufficient_funds, "insufficient funds {} for pre-commit deposit: {}", available_balance, total_deposit_required));
            }
            state.add_pre_commit_deposit(&total_deposit_required)
                .map_err(|e|
                    actor_error!(
                        illegal_state,
                        "failed to add pre-commit deposit {}: {}",
                        total_deposit_required, e
                ))?;
            state.allocate_sector_numbers(store, &sector_numbers, CollisionPolicy::DenyCollisions)?;
            state.put_precommitted_sectors(store, chain_infos)
                .map_err(|e|
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to write pre-committed sectors")
                )?;
            state.add_pre_commit_clean_ups(rt.policy(), store, clean_up_events)
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to add pre-commit expiry to queue")
                })?;

            for sector_num in sector_numbers.iter() {
                emit::sector_precommitted(rt, sector_num)?;
            }
            // Activate miner cron
            needs_cron = !state.deadline_cron_active;
            state.deadline_cron_active = true;
            Ok(())
        })?;
        burn_funds(rt, fee_to_burn)?;
        let state: State = rt.state()?;
        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;
        if needs_cron {
            let new_dl_info = state.deadline_info(rt.policy(), curr_epoch);
            enroll_cron_event(
                rt,
                new_dl_info.last(),
                CronEventPayload { event_type: CRON_EVENT_PROVING_DEADLINE },
            )?;
        }
        Ok(())
    }

    fn prove_commit_sectors3(
        rt: &impl Runtime,
        params: ProveCommitSectors3Params,
    ) -> Result<ProveCommitSectors3Return, ActorError> {
        let state: State = rt.state()?;
        let store = rt.store();
        let policy = rt.policy();
        let miner_id = rt.message().receiver().id().unwrap();
        let info = get_miner_info(rt.store(), &state)?;

        // Validate caller and parameters.
        rt.validate_immediate_caller_is(
            info.control_addresses.iter().chain(&[info.worker, info.owner]),
        )?;

        // Load pre-commits, failing if any don't exist.
        let sector_numbers = params.sector_activations.iter().map(|sa| sa.sector_number);
        let precommits =
            state.get_precommitted_sectors(store, sector_numbers).context("loading precommits")?;
        if precommits.is_empty() {
            return Err(actor_error!(illegal_argument, "no sectors to prove"));
        }

        if params.sector_proofs.is_empty() == params.aggregate_proof.is_empty() {
            return Err(actor_error!(
                illegal_argument,
                "exactly one of sector proofs or aggregate proof must be non-empty"
            ));
        }

        if !params.sector_proofs.is_empty() {
            // Batched proofs, one per sector
            if params.aggregate_proof_type.is_some() {
                return Err(actor_error!(
                    illegal_argument,
                    "aggregate proof type must be null with batched proofs"
                ));
            }
            if params.sector_activations.len() != params.sector_proofs.len() {
                return Err(actor_error!(
                    illegal_argument,
                    "mismatched lengths: {} sector activations, {} proofs",
                    params.sector_activations.len(),
                    params.sector_proofs.len()
                ));
            }
            validate_seal_proofs(precommits[0].info.seal_proof, &params.sector_proofs)?;
        } else {
            if params.aggregate_proof_type != Some(RegisteredAggregateProof::SnarkPackV2) {
                return Err(actor_error!(
                    illegal_argument,
                    "aggregate proof type must be SnarkPackV2"
                ));
            }
            validate_seal_aggregate_proof(
                &params.aggregate_proof,
                params.sector_activations.len() as u64,
                policy,
                true,
            )?;
        }

        // Validate pre-commits.
        let allow_deals = false; // New onboarding entry point does not allow pre-committed deals.
        let (validation_batch, proof_inputs) =
            validate_precommits(rt, &precommits, allow_deals, params.require_activation_success)?;
        if validation_batch.success_count == 0 {
            return Err(actor_error!(illegal_argument, "no valid precommits specified"));
        }
        let valid_precommits = validation_batch.successes(&precommits);
        let valid_activation_inputs = validation_batch.successes(&params.sector_activations);
        let eligible_activation_inputs_iter = valid_activation_inputs.iter().zip(valid_precommits);

        // Verify seal proof(s), either batch or aggregate.
        let mut proven_activation_inputs: Vec<(
            &SectorActivationManifest,
            &SectorPreCommitOnChainInfo,
        )> = vec![];
        let mut proven_batch_gen = BatchReturnGen::new(validation_batch.success_count as usize);
        if !params.sector_proofs.is_empty() {
            // Verify batched proofs.
            // Filter proof inputs to those for valid pre-commits.
            let seal_verify_inputs: Vec<SealVerifyInfo> = validation_batch
                .successes(&proof_inputs)
                .iter()
                .zip(validation_batch.successes(&params.sector_proofs))
                .map(|(info, proof)| -> SealVerifyInfo {
                    info.to_seal_verify_info(miner_id, proof)
                })
                .collect();

            let res = rt
                .batch_verify_seals(&seal_verify_inputs)
                .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "failed to batch verify")?;

            // Filter eligible activations to those that were proven.
            for (verified, (activation, precommit)) in
                res.iter().zip(eligible_activation_inputs_iter)
            {
                if *verified {
                    proven_activation_inputs.push((*activation, precommit));
                    proven_batch_gen.add_success();
                } else {
                    proven_batch_gen.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
                    if params.require_activation_success {
                        return Err(actor_error!(
                            illegal_argument,
                            "invalid proof for sector {} while requiring activation success: {:?}",
                            precommit.info.sector_number,
                            res
                        ));
                    }
                }
            }
        } else {
            // Verify a single aggregate proof.
            verify_aggregate_seal(
                rt,
                // All the proof inputs, even for invalid pre-commits,
                // must be provided as witnesses to the aggregate proof.
                &proof_inputs,
                miner_id,
                precommits[0].info.seal_proof,
                params.aggregate_proof_type.unwrap(),
                &params.aggregate_proof,
            )?;

            // All eligible activations are proven.
            proven_activation_inputs = eligible_activation_inputs_iter
                .map(|(activation, precommit)| (*activation, precommit))
                .collect();
            proven_batch_gen.add_successes(proven_activation_inputs.len());
        }
        let proven_batch = proven_batch_gen.gen();
        if proven_batch.success_count == 0 {
            return Err(actor_error!(illegal_argument, "no valid proofs specified"));
        }

        // Activate data and verify CommD matches the declared one.
        let data_activation_inputs = proven_activation_inputs
            .iter()
            .map(|(activation, precommit)| -> SectorPiecesActivationInput {
                SectorPiecesActivationInput {
                    piece_manifests: activation.pieces.clone(),
                    sector_expiry: precommit.info.expiration,
                    sector_number: precommit.info.sector_number,
                    sector_type: precommit.info.seal_proof,
                    expected_commd: Some(precommit.info.unsealed_cid.clone()), // Check CommD
                }
            })
            .collect();

        // Activate data for proven sectors.
        let (data_batch, data_activations) =
            activate_sectors_pieces(rt, data_activation_inputs, params.require_activation_success)?;
        if data_batch.success_count == 0 {
            return Err(actor_error!(illegal_argument, "all data activations failed"));
        }

        // Successful data activation is required for sector activation.
        let successful_sector_activations = data_batch.successes(&proven_activation_inputs);
        let successful_precommits =
            successful_sector_activations.iter().map(|(_, second)| *second).collect();

        // Activate sector info state
        let rew = request_current_epoch_block_reward(rt)?;
        let pwr = request_current_total_power(rt)?;
        let circulating_supply = rt.total_fil_circ_supply();
        let pledge_inputs = NetworkPledgeInputs {
            network_qap: pwr.quality_adj_power_smoothed,
            network_baseline: rew.this_epoch_baseline_power,
            circulating_supply,
            epoch_reward: rew.this_epoch_reward_smoothed,
            epochs_since_ramp_start: rt.curr_epoch() - pwr.ramp_start_epoch,
            ramp_duration_epochs: pwr.ramp_duration_epochs,
        };
        activate_new_sector_infos(
            rt,
            successful_precommits,
            data_activations,
            &pledge_inputs,
            &info,
        )?;

        if !params.aggregate_proof.is_empty() {
            // Aggregate fee is paid on the sectors successfully proven,
            // but without regard to data activation which may have subsequently failed
            // and prevented sector activation.
            pay_aggregate_seal_proof_fee(rt, proven_activation_inputs.len())?;
        }

        // Notify data consumers.
        let mut notifications: Vec<ActivationNotifications> = vec![];
        for (activations, sector) in &successful_sector_activations {
            notifications.push(ActivationNotifications {
                sector_number: activations.sector_number,
                sector_expiration: sector.info.expiration,
                pieces: &activations.pieces,
            });

            let pieces: Vec<(Cid, u64)> =
                activations.pieces.iter().map(|p| (p.cid, p.size.0)).collect();
            let unsealed_cid = sector.info.unsealed_cid.0;

            emit::sector_activated(rt, sector.info.sector_number, unsealed_cid, &pieces)?;
        }
        notify_data_consumers(rt, &notifications, params.require_notification_success)?;

        let result = util::stack(&[validation_batch, proven_batch, data_batch]);
        Ok(ProveCommitSectors3Return { activation_results: result })
    }

    fn internal_sector_setup_preseal(
        rt: &impl Runtime,
        params: InternalSectorSetupForPresealParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;
        let st: State = rt.state()?;
        let store = rt.store();
        // This skips missing pre-commits.
        let precommited_sectors =
            st.find_precommitted_sectors(store, &params.sectors).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to load pre-committed sectors",
                )
            })?;

        let data_activations: Vec<DealsActivationInput> =
            precommited_sectors.iter().map(|x| x.clone().into()).collect();
        let info = get_miner_info(rt.store(), &st)?;

        /*
            For all sectors
            - CommD was specified at precommit
            - If deal IDs were specified at precommit the CommD was checked against them
            Therefore CommD on precommit has already been provided and checked so no further processing needed
        */
        let compute_commd = false;
        let (batch_return, data_activations) =
            activate_sectors_deals(rt, &data_activations, compute_commd)?;
        let successful_activations = batch_return.successes(&precommited_sectors);

        let pledge_inputs = NetworkPledgeInputs {
            network_qap: params.quality_adj_power_smoothed,
            network_baseline: params.reward_baseline_power,
            circulating_supply: rt.total_fil_circ_supply(),
            epoch_reward: params.reward_smoothed,
            epochs_since_ramp_start: 0,
            ramp_duration_epochs: 0,
        };
        activate_new_sector_infos(
            rt,
            successful_activations.clone(),
            data_activations.clone(),
            &pledge_inputs,
            &info,
        )?;

        for (pc, data) in successful_activations.iter().zip(data_activations.iter()) {
            let unsealed_cid = pc.info.unsealed_cid.0;
            emit::sector_activated(rt, pc.info.sector_number, unsealed_cid, &data.pieces)?;
        }

        Ok(())
    }

    fn prove_commit_sectors_ni(
        rt: &impl Runtime,
        params: ProveCommitSectorsNIParams,
    ) -> Result<ProveCommitSectorsNIReturn, ActorError> {
        let policy = rt.policy();
        let curr_epoch = rt.curr_epoch();
        let state: State = rt.state()?;
        let store = rt.store();
        let info = get_miner_info(rt.store(), &state)?;

        validate_seal_aggregate_proof(
            &params.aggregate_proof,
            params.sectors.len() as u64,
            policy,
            false,
        )?;

        rt.validate_immediate_caller_is(
            info.control_addresses.iter().chain(&[info.worker, info.owner]),
        )?;

        if params.proving_deadline >= policy.wpost_period_deadlines {
            return Err(actor_error!(
                illegal_argument,
                "proving deadline index {} invalid",
                params.proving_deadline
            ));
        }

        if !deadline_is_mutable(
            policy,
            state.current_proving_period_start(policy, curr_epoch),
            params.proving_deadline,
            curr_epoch,
        ) {
            return Err(actor_error!(
                forbidden,
                "proving deadline {} must not be the current or next deadline ",
                params.proving_deadline
            ));
        }

        if consensus_fault_active(&info, rt.curr_epoch()) {
            return Err(actor_error!(
                forbidden,
                "ProveCommitSectorsNI not allowed during active consensus fault"
            ));
        }

        if !can_prove_commit_ni_seal_proof(rt.policy(), params.seal_proof_type) {
            return Err(actor_error!(
                illegal_argument,
                "unsupported seal proof type {}",
                i64::from(params.seal_proof_type)
            ));
        }

        if params.aggregate_proof_type != RegisteredAggregateProof::SnarkPackV2 {
            return Err(actor_error!(illegal_argument, "aggregate proof type must be SnarkPackV2"));
        }

        let (validation_batch, proof_inputs, sector_numbers) = validate_ni_sectors(
            rt,
            &params.sectors,
            params.seal_proof_type,
            params.require_activation_success,
        )?;

        if validation_batch.success_count == 0 {
            return Err(actor_error!(illegal_argument, "no valid NI commits specified"));
        }
        let valid_sectors = validation_batch.successes(&params.sectors);

        verify_aggregate_seal(
            rt,
            // All the proof inputs, even for invalid activations,
            // must be provided as witnesses to the aggregate proof.
            &proof_inputs,
            valid_sectors[0].sealer_id,
            params.seal_proof_type,
            params.aggregate_proof_type,
            &params.aggregate_proof,
        )?;

        // With no data, QA power = raw power
        let qa_sector_power = raw_power_for_sector(info.sector_size);

        let rew = request_current_epoch_block_reward(rt)?;
        let pwr = request_current_total_power(rt)?;
        let circulating_supply = rt.total_fil_circ_supply();
        let pledge_inputs = NetworkPledgeInputs {
            network_qap: pwr.quality_adj_power_smoothed,
            network_baseline: rew.this_epoch_baseline_power,
            circulating_supply,
            epoch_reward: rew.this_epoch_reward_smoothed,
            epochs_since_ramp_start: rt.curr_epoch() - pwr.ramp_start_epoch,
            ramp_duration_epochs: pwr.ramp_duration_epochs,
        };

        let sector_day_reward = expected_reward_for_power(
            &pledge_inputs.epoch_reward,
            &pledge_inputs.network_qap,
            &qa_sector_power,
            fil_actors_runtime::EPOCHS_IN_DAY,
        );

        let sector_storage_pledge = expected_reward_for_power(
            &pledge_inputs.epoch_reward,
            &pledge_inputs.network_qap,
            &qa_sector_power,
            INITIAL_PLEDGE_PROJECTION_PERIOD,
        );

        let sector_initial_pledge = initial_pledge_for_power(
            &qa_sector_power,
            &pledge_inputs.network_baseline,
            &pledge_inputs.epoch_reward,
            &pledge_inputs.network_qap,
            &pledge_inputs.circulating_supply,
            pledge_inputs.epochs_since_ramp_start,
            pledge_inputs.ramp_duration_epochs,
        );

        let sectors_to_add = valid_sectors
            .iter()
            .map(|sector| SectorOnChainInfo {
                sector_number: sector.sector_number,
                seal_proof: params.seal_proof_type,
                sealed_cid: sector.sealed_cid,
                deprecated_deal_ids: vec![],
                expiration: sector.expiration,
                activation: curr_epoch,
                deal_weight: DealWeight::zero(),
                verified_deal_weight: DealWeight::zero(),
                initial_pledge: sector_initial_pledge.clone(),
                expected_day_reward: sector_day_reward.clone(),
                expected_storage_pledge: sector_storage_pledge.clone(),
                power_base_epoch: curr_epoch,
                replaced_day_reward: TokenAmount::zero(),
                sector_key_cid: None,
                flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
            })
            .collect::<Vec<SectorOnChainInfo>>();

        let sectors_len = sectors_to_add.len();

        let total_pledge = BigInt::from(sectors_len) * sector_initial_pledge;

        let (needs_cron, fee_to_burn) = rt.transaction(|state: &mut State, rt| {
            let current_balance = rt.current_balance();
            let available_balance = state
                .get_unlocked_balance(&current_balance)
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    "failed to calculate unlocked balance"
                })?;
            if available_balance < total_pledge {
                return Err(actor_error!(
                    insufficient_funds,
                    "insufficient funds for aggregate initial pledge requirement {}, available: {}",
                    total_pledge,
                    available_balance
                ));
            }
            let needs_cron = !state.deadline_cron_active;
            state.deadline_cron_active = true;

            state.allocate_sector_numbers(
                store,
                &sector_numbers,
                CollisionPolicy::DenyCollisions,
            )?;

            state
                .put_sectors(store, sectors_to_add.clone())
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || "failed to put new sectors")?;

            state.assign_sectors_to_deadline(
                policy,
                store,
                rt.curr_epoch(),
                sectors_to_add,
                info.window_post_partition_sectors,
                info.sector_size,
                params.proving_deadline,
            )?;

            state
                .add_initial_pledge(&total_pledge)
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    "failed to add initial pledgs"
                })?;

            let fee_to_burn = repay_debts_or_abort(rt, state)?;

            Ok((needs_cron, fee_to_burn))
        })?;

        burn_funds(rt, fee_to_burn)?;

        let len_for_aggregate_fee = if sectors_len <= NI_AGGREGATE_FEE_BASE_SECTOR_COUNT {
            0
        } else {
            sectors_len - NI_AGGREGATE_FEE_BASE_SECTOR_COUNT
        };
        pay_aggregate_seal_proof_fee(rt, len_for_aggregate_fee)?;

        notify_pledge_changed(rt, &total_pledge)?;

        let state: State = rt.state()?;
        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;

        for sector in valid_sectors.iter() {
            emit::sector_activated(rt, sector.sector_number, None, &[])?;
        }

        if needs_cron {
            let new_dl_info = state.deadline_info(rt.policy(), curr_epoch);
            enroll_cron_event(
                rt,
                new_dl_info.last(),
                CronEventPayload { event_type: CRON_EVENT_PROVING_DEADLINE },
            )?;
        }

        Ok(ProveCommitSectorsNIReturn { activation_results: validation_batch })
    }

    fn check_sector_proven(
        rt: &impl Runtime,
        params: CheckSectorProvenParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;

        if params.sector_number > MAX_SECTOR_NUMBER {
            return Err(actor_error!(illegal_argument, "sector number out of range"));
        }

        let st: State = rt.state()?;

        match st.get_sector(rt.store(), params.sector_number) {
            Err(e) => Err(actor_error!(
                illegal_state,
                "failed to load proven sector {}: {}",
                params.sector_number,
                e
            )),
            Ok(None) => Err(actor_error!(not_found, "sector {} not proven", params.sector_number)),
            Ok(Some(_sector)) => Ok(()),
        }
    }

    /// Changes the expiration epoch for a sector to a new, later one.
    /// The sector must not be terminated or faulty.
    /// The sector's power is recomputed for the new expiration.
    /// This method is legacy and should be replaced with calls to extend_sector_expiration2
    fn extend_sector_expiration(
        rt: &impl Runtime,
        params: ExtendSectorExpirationParams,
    ) -> Result<(), ActorError> {
        let extend_expiration_inner =
            validate_legacy_extension_declarations(&params.extensions, rt.policy())?;
        Self::extend_sector_expiration_inner(
            rt,
            extend_expiration_inner,
            ExtensionKind::ExtendCommittmentLegacy,
        )
    }

    // Up to date version of extend_sector_expiration that correctly handles simple qap sectors
    // with FIL+ claims. Extension is only allowed if all claim max terms extend past new expiration
    // or claims are dropped.  Power only changes when claims are dropped.
    fn extend_sector_expiration2(
        rt: &impl Runtime,
        params: ExtendSectorExpiration2Params,
    ) -> Result<(), ActorError> {
        let extend_expiration_inner = validate_extension_declarations(rt, params.extensions)?;
        Self::extend_sector_expiration_inner(
            rt,
            extend_expiration_inner,
            ExtensionKind::ExtendCommittment,
        )
    }

    fn extend_sector_expiration_inner(
        rt: &impl Runtime,
        inner: ExtendExpirationsInner,
        kind: ExtensionKind,
    ) -> Result<(), ActorError> {
        let curr_epoch = rt.curr_epoch();
        let reward_stats = &request_current_epoch_block_reward(rt)?;
        let power_stats = &request_current_total_power(rt)?;

        /* Loop over sectors and do extension */
        let (power_delta, pledge_delta) = rt.transaction(|state: &mut State, rt| {
            let info = get_miner_info(rt.store(), state)?;
            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            let mut deadlines =
                state.load_deadlines(rt.store()).map_err(|e| e.wrap("failed to load deadlines"))?;

            // Group declarations by deadline, and remember iteration order.
            //
            let mut decls_by_deadline: Vec<_> = std::iter::repeat_with(Vec::new)
                .take(rt.policy().wpost_period_deadlines as usize)
                .collect();
            let mut deadlines_to_load = Vec::<u64>::new();
            for decl in &inner.extensions {
                // the deadline indices are already checked.
                let decls = &mut decls_by_deadline[decl.deadline as usize];
                if decls.is_empty() {
                    deadlines_to_load.push(decl.deadline);
                }
                decls.push(decl);
            }

            let mut sectors = Sectors::load(rt.store(), &state.sectors).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors array")
            })?;

            let mut power_delta = PowerPair::zero();
            let mut pledge_delta = TokenAmount::zero();

            for deadline_idx in deadlines_to_load {
                let policy = rt.policy();
                let mut deadline = deadlines.load_deadline(rt.store(), deadline_idx)?;

                let mut partitions = deadline.partitions_amt(rt.store()).map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to load partitions for deadline {}", deadline_idx),
                    )
                })?;

                let quant = state.quant_spec_for_deadline(policy, deadline_idx);

                // Group modified partitions by epoch to which they are extended. Duplicates are ok.
                let mut partitions_by_new_epoch = BTreeMap::<ChainEpoch, Vec<u64>>::new();
                let mut epochs_to_reschedule = Vec::<ChainEpoch>::new();

                for decl in &mut decls_by_deadline[deadline_idx as usize] {
                    let key = PartitionKey { deadline: deadline_idx, partition: decl.partition };

                    let mut partition = partitions
                        .get(decl.partition)
                        .map_err(|e| {
                            e.downcast_default(
                                ExitCode::USR_ILLEGAL_STATE,
                                format!("failed to load partition {:?}", key),
                            )
                        })?
                        .cloned()
                        .ok_or_else(|| actor_error!(not_found, "no such partition {:?}", key))?;

                    let old_sectors = sectors
                        .load_sector(&decl.sectors)
                        .map_err(|e| e.wrap("failed to load sectors"))?;
                    let new_sectors: Vec<SectorOnChainInfo> = old_sectors
                        .iter()
                        .map(|sector| match kind {
                            ExtensionKind::ExtendCommittmentLegacy => {
                                extend_sector_committment_legacy(
                                    rt.policy(),
                                    curr_epoch,
                                    decl.new_expiration,
                                    sector,
                                )
                            }
                            ExtensionKind::ExtendCommittment => match &inner.claims {
                                None => Err(actor_error!(
                                    unspecified,
                                    "extend2 always specifies (potentially empty) claim mapping"
                                )),
                                Some(claim_space_by_sector) => extend_sector_committment(
                                    rt.policy(),
                                    curr_epoch,
                                    reward_stats,
                                    power_stats,
                                    decl.new_expiration,
                                    sector,
                                    info.sector_size,
                                    claim_space_by_sector,
                                ),
                            },
                        })
                        .collect::<Result<_, _>>()?;

                    // Overwrite sector infos.
                    sectors.store(new_sectors.clone()).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to update sectors {:?}", decl.sectors),
                        )
                    })?;

                    // Remove old sectors from partition and assign new sectors.
                    let (partition_power_delta, partition_pledge_delta) = partition
                        .replace_sectors(
                            rt.store(),
                            &old_sectors,
                            &new_sectors,
                            info.sector_size,
                            quant,
                        )
                        .map_err(|e| {
                            e.downcast_default(
                                ExitCode::USR_ILLEGAL_STATE,
                                format!("failed to replace sector expirations at {:?}", key),
                            )
                        })?;

                    power_delta += &partition_power_delta;
                    pledge_delta += partition_pledge_delta; // expected to be zero, see note below.

                    partitions.set(decl.partition, partition).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to save partition {:?}", key),
                        )
                    })?;

                    // Record the new partition expiration epoch for setting outside this loop
                    // over declarations.
                    let prev_epoch_partitions = partitions_by_new_epoch.entry(decl.new_expiration);
                    let not_exists = matches!(prev_epoch_partitions, Entry::Vacant(_));

                    // Add declaration partition
                    prev_epoch_partitions.or_default().push(decl.partition);
                    if not_exists {
                        // reschedule epoch if the partition for new epoch didn't already exist
                        epochs_to_reschedule.push(decl.new_expiration);
                    }
                }

                deadline.partitions = partitions.flush().map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to save partitions for deadline {}", deadline_idx),
                    )
                })?;

                // Record partitions in deadline expiration queue
                for epoch in epochs_to_reschedule {
                    let p_idxs = partitions_by_new_epoch.get(&epoch).unwrap();
                    deadline.add_expiration_partitions(rt.store(), epoch, p_idxs, quant).map_err(
                        |e| {
                            e.downcast_default(
                                ExitCode::USR_ILLEGAL_STATE,
                                format!(
                                    "failed to add expiration partitions to \
                                        deadline {} epoch {}",
                                    deadline_idx, epoch
                                ),
                            )
                        },
                    )?;
                }

                deadlines.update_deadline(policy, rt.store(), deadline_idx, &deadline).map_err(
                    |e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to save deadline {}", deadline_idx),
                        )
                    },
                )?;
            }

            state.sectors = sectors.amt.flush().map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save sectors")
            })?;
            state.save_deadlines(rt.store(), deadlines).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save deadlines")
            })?;

            Ok((power_delta, pledge_delta))
        })?;

        request_update_power(rt, power_delta)?;

        // Note: the pledge delta is expected to be zero, since pledge is not re-calculated for the extension.
        // But in case that ever changes, we can do the right thing here.
        notify_pledge_changed(rt, &pledge_delta)?;
        Ok(())
    }

    /// Marks some sectors as terminated at the present epoch, earlier than their
    /// scheduled termination, and adds these sectors to the early termination queue.
    /// This method then processes up to AddressedSectorsMax sectors and
    /// AddressedPartitionsMax partitions from the early termination queue,
    /// terminating deals, paying fines, and returning pledge collateral. While
    /// sectors remain in this queue:
    ///
    ///  1. The miner will be unable to withdraw funds.
    ///  2. The chain will process up to AddressedSectorsMax sectors and
    ///     AddressedPartitionsMax per epoch until the queue is empty.
    ///
    /// The sectors are immediately ignored for Window PoSt proofs, and should be
    /// masked in the same way as faulty sectors. A miner may not terminate sectors in the
    /// current deadline or the next deadline to be proven.
    ///
    /// This function may be invoked with no new sectors to explicitly process the
    /// next batch of sectors.
    fn terminate_sectors(
        rt: &impl Runtime,
        params: TerminateSectorsParams,
    ) -> Result<TerminateSectorsReturn, ActorError> {
        // Note: this cannot terminate pre-committed but un-proven sectors.
        // They must be allowed to expire (and deposit burnt).

        let mut to_process = DeadlineSectorMap::new();

        for term in params.terminations {
            let deadline = term.deadline;
            let partition = term.partition;

            to_process.add(rt.policy(), deadline, partition, term.sectors).map_err(|e| {
                actor_error!(
                    illegal_argument,
                    "failed to process deadline {}, partition {}: {}",
                    deadline,
                    partition,
                    e
                )
            })?;
        }

        {
            let policy = rt.policy();
            to_process
                .check(policy.addressed_partitions_max, policy.addressed_sectors_max)
                .map_err(|e| {
                    actor_error!(illegal_argument, "cannot process requested parameters: {}", e)
                })?;
        }

        let (had_early_terminations, power_delta) = rt.transaction(|state: &mut State, rt| {
            let had_early_terminations = have_pending_early_terminations(state);

            let info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            let store = rt.store();
            let curr_epoch = rt.curr_epoch();
            let mut power_delta = PowerPair::zero();

            let mut deadlines =
                state.load_deadlines(store).map_err(|e| e.wrap("failed to load deadlines"))?;

            // We're only reading the sectors, so there's no need to save this back.
            // However, we still want to avoid re-loading this array per-partition.
            let sectors = Sectors::load(store, &state.sectors).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors")
            })?;

            for (deadline_idx, partition_sectors) in to_process.iter() {
                // If the deadline is the current or next deadline to prove, don't allow terminating sectors.
                // We assume that deadlines are immutable when being proven.
                if !deadline_is_mutable(
                    rt.policy(),
                    state.current_proving_period_start(rt.policy(), curr_epoch),
                    deadline_idx,
                    curr_epoch,
                ) {
                    return Err(actor_error!(
                        illegal_argument,
                        "cannot terminate sectors in immutable deadline {}",
                        deadline_idx
                    ));
                }

                let quant = state.quant_spec_for_deadline(rt.policy(), deadline_idx);
                let mut deadline = deadlines.load_deadline(store, deadline_idx)?;

                let removed_power = deadline
                    .terminate_sectors(
                        rt.policy(),
                        store,
                        &sectors,
                        curr_epoch,
                        partition_sectors,
                        info.sector_size,
                        quant,
                    )
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to terminate sectors in deadline {}", deadline_idx),
                        )
                    })?;

                state.early_terminations.set(deadline_idx);
                power_delta -= &removed_power;

                deadlines.update_deadline(rt.policy(), store, deadline_idx, &deadline).map_err(
                    |e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to update deadline {}", deadline_idx),
                        )
                    },
                )?;
            }

            state.save_deadlines(store, deadlines).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save deadlines")
            })?;

            Ok((had_early_terminations, power_delta))
        })?;
        let epoch_reward = request_current_epoch_block_reward(rt)?;
        let pwr_total = request_current_total_power(rt)?;

        // Now, try to process these sectors.
        let more = process_early_terminations(
            rt,
            &epoch_reward.this_epoch_reward_smoothed,
            &pwr_total.quality_adj_power_smoothed,
        )?;

        if more && !had_early_terminations {
            // We have remaining terminations, and we didn't _previously_
            // have early terminations to process, schedule a cron job.
            // NOTE: This isn't quite correct. If we repeatedly fill, empty,
            // fill, and empty, the queue, we'll keep scheduling new cron
            // jobs. However, in practice, that shouldn't be all that bad.
            schedule_early_termination_work(rt)?;
        }
        let state: State = rt.state()?;
        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;

        request_update_power(rt, power_delta)?;
        Ok(TerminateSectorsReturn { done: !more })
    }

    fn declare_faults(rt: &impl Runtime, params: DeclareFaultsParams) -> Result<(), ActorError> {

        let mut to_process = DeadlineSectorMap::new();

        for term in params.faults {
            let deadline = term.deadline;
            let partition = term.partition;

            to_process.add(rt.policy(), deadline, partition, term.sectors).map_err(|e| {
                actor_error!(
                    illegal_argument,
                    "failed to process deadline {}, partition {}: {}",
                    deadline,
                    partition,
                    e
                )
            })?;
        }

        {
            let policy = rt.policy();
            to_process
                .check(policy.addressed_partitions_max, policy.addressed_sectors_max)
                .map_err(|e| {
                    actor_error!(illegal_argument, "cannot process requested parameters: {}", e)
                })?;
        }

        let power_delta = rt.transaction(|state: &mut State, rt| {
            let info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            let store = rt.store();

            let mut deadlines =
                state.load_deadlines(store).map_err(|e| e.wrap("failed to load deadlines"))?;

            let sectors = Sectors::load(store, &state.sectors).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors array")
            })?;

            let mut new_fault_power_total = PowerPair::zero();
            let curr_epoch = rt.curr_epoch();
            for (deadline_idx, partition_map) in to_process.iter() {
                let policy = rt.policy();
                let target_deadline = declaration_deadline_info(
                    policy,
                    state.current_proving_period_start(policy, curr_epoch),
                    deadline_idx,
                    curr_epoch,
                )
                .map_err(|e| {
                    actor_error!(
                        illegal_argument,
                        "invalid fault declaration deadline {}: {}",
                        deadline_idx,
                        e
                    )
                })?;

                validate_fr_declaration_deadline(&target_deadline).map_err(|e| {
                    actor_error!(
                        illegal_argument,
                        "failed fault declaration at deadline {}: {}",
                        deadline_idx,
                        e
                    )
                })?;

                let mut deadline = deadlines.load_deadline(store, deadline_idx)?;

                let fault_expiration_epoch = target_deadline.last() + policy.fault_max_age;

                let deadline_power_delta = deadline
                    .record_faults(
                        store,
                        &sectors,
                        info.sector_size,
                        target_deadline.quant_spec(),
                        fault_expiration_epoch,
                        partition_map,
                    )
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to declare faults for deadline {}", deadline_idx),
                        )
                    })?;

                deadlines.update_deadline(policy, store, deadline_idx, &deadline).map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to store deadline {} partitions", deadline_idx),
                    )
                })?;

                new_fault_power_total += &deadline_power_delta;
            }

            state.save_deadlines(store, deadlines).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save deadlines")
            })?;

            Ok(new_fault_power_total)
        })?;

        // Remove power for new faulty sectors.
        // NOTE: It would be permissible to delay the power loss until the deadline closes, but that would require
        // additional accounting state.
        // https://github.com/filecoin-project/specs-actors/issues/414
        request_update_power(rt, power_delta)?;

        // Payment of penalty for declared faults is deferred to the deadline cron.
        Ok(())
    }

    fn declare_faults_recovered(
        rt: &impl Runtime,
        params: DeclareFaultsRecoveredParams,
    ) -> Result<(), ActorError> {

        let mut to_process = DeadlineSectorMap::new();

        for term in params.recoveries {
            let deadline = term.deadline;
            let partition = term.partition;

            to_process.add(rt.policy(), deadline, partition, term.sectors).map_err(|e| {
                actor_error!(
                    illegal_argument,
                    "failed to process deadline {}, partition {}: {}",
                    deadline,
                    partition,
                    e
                )
            })?;
        }

        {
            let policy = rt.policy();
            to_process
                .check(policy.addressed_partitions_max, policy.addressed_sectors_max)
                .map_err(|e| {
                    actor_error!(illegal_argument, "cannot process requested parameters: {}", e)
                })?;
        }

        let fee_to_burn = rt.transaction(|state: &mut State, rt| {
            // Verify unlocked funds cover both InitialPledgeRequirement and FeeDebt
            // and repay fee debt now.
            let fee_to_burn = repay_debts_or_abort(rt, state)?;

            let info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            if consensus_fault_active(&info, rt.curr_epoch()) {
                return Err(actor_error!(
                    forbidden,
                    "recovery not allowed during active consensus fault"
                ));
            }

            let store = rt.store();

            let mut deadlines =
                state.load_deadlines(store).map_err(|e| e.wrap("failed to load deadlines"))?;

            let sectors = Sectors::load(store, &state.sectors).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors array")
            })?;
            let curr_epoch = rt.curr_epoch();
            for (deadline_idx, partition_map) in to_process.iter() {
                let policy = rt.policy();
                let target_deadline = declaration_deadline_info(
                    policy,
                    state.current_proving_period_start(policy, curr_epoch),
                    deadline_idx,
                    curr_epoch,
                )
                .map_err(|e| {
                    actor_error!(
                        illegal_argument,
                        "invalid recovery declaration deadline {}: {}",
                        deadline_idx,
                        e
                    )
                })?;

                validate_fr_declaration_deadline(&target_deadline).map_err(|e| {
                    actor_error!(
                        illegal_argument,
                        "failed recovery declaration at deadline {}: {}",
                        deadline_idx,
                        e
                    )
                })?;

                let mut deadline = deadlines.load_deadline(store, deadline_idx)?;

                deadline
                    .declare_faults_recovered(store, &sectors, info.sector_size, partition_map)
                    .map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to declare recoveries for deadline {}", deadline_idx),
                        )
                    })?;

                deadlines.update_deadline(policy, store, deadline_idx, &deadline).map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to store deadline {}", deadline_idx),
                    )
                })?;
            }

            state.save_deadlines(store, deadlines).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save deadlines")
            })?;

            Ok(fee_to_burn)
        })?;

        burn_funds(rt, fee_to_burn)?;
        let state: State = rt.state()?;
        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;

        // Power is not restored yet, but when the recovered sectors are successfully PoSted.
        Ok(())
    }

    /// Compacts a number of partitions at one deadline by removing terminated sectors, re-ordering the remaining sectors,
    /// and assigning them to new partitions so as to completely fill all but one partition with live sectors.
    /// The addressed partitions are removed from the deadline, and new ones appended.
    /// The final partition in the deadline is always included in the compaction, whether or not explicitly requested.
    /// Removed sectors are removed from state entirely.
    /// May not be invoked if the deadline has any un-processed early terminations.
    fn compact_partitions(
        rt: &impl Runtime,
        params: CompactPartitionsParams,
    ) -> Result<(), ActorError> {
        {
            let policy = rt.policy();
            if params.deadline >= policy.wpost_period_deadlines {
                return Err(actor_error!(illegal_argument, "invalid deadline {}", params.deadline));
            }
        }

        let partitions = params.partitions.validate().map_err(|e| {
            actor_error!(illegal_argument, "failed to parse partitions bitfield: {}", e)
        })?;
        let partition_count = partitions.len();

        let params_deadline = params.deadline;

        rt.transaction(|state: &mut State, rt| {
            let info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            let store = rt.store();
            let policy = rt.policy();

            if !deadline_available_for_compaction(
                policy,
                state.current_proving_period_start(policy, rt.curr_epoch()),
                params_deadline,
                rt.curr_epoch(),
            ) {
                return Err(actor_error!(
                    forbidden,
                    "cannot compact deadline {} during its challenge window, \
                    or the prior challenge window,
                    or before {} epochs have passed since its last challenge window ended",
                    params_deadline,
                    policy.wpost_dispute_window
                ));
            }

            let submission_partition_limit =
                load_partitions_sectors_max(policy, info.window_post_partition_sectors);
            if partition_count > submission_partition_limit {
                return Err(actor_error!(
                    illegal_argument,
                    "too many partitions {}, limit {}",
                    partition_count,
                    submission_partition_limit
                ));
            }

            let quant = state.quant_spec_for_deadline(policy, params_deadline);
            let mut deadlines =
                state.load_deadlines(store).map_err(|e| e.wrap("failed to load deadlines"))?;

            let mut deadline = deadlines.load_deadline(store, params_deadline)?;

            let (live, dead, removed_power) =
                deadline.remove_partitions(store, partitions, quant).map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to remove partitions from deadline {}", params_deadline),
                    )
                })?;

            state.delete_sectors(store, &dead).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to delete dead sectors")
            })?;

            let sectors = state.load_sector_infos(store, &live).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load moved sectors")
            })?;
            let proven = true;
            let added_power = deadline
                .add_sectors(
                    store,
                    info.window_post_partition_sectors,
                    proven,
                    &sectors,
                    info.sector_size,
                    quant,
                )
                .map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        "failed to add back moved sectors",
                    )
                })?;

            if removed_power != added_power {
                return Err(actor_error!(
                    illegal_state,
                    "power changed when compacting partitions: was {:?}, is now {:?}",
                    removed_power,
                    added_power
                ));
            }

            deadlines.update_deadline(policy, store, params_deadline, &deadline).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to update deadline {}", params_deadline),
                )
            })?;

            state.save_deadlines(store, deadlines).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to save deadline {}", params_deadline),
                )
            })?;

            Ok(())
        })?;

        Ok(())
    }

    /// Compacts sector number allocations to reduce the size of the allocated sector
    /// number bitfield.
    ///
    /// When allocating sector numbers sequentially, or in sequential groups, this
    /// bitfield should remain fairly small. However, if the bitfield grows large
    /// enough such that PreCommitSector fails (or becomes expensive), this method
    /// can be called to mask out (throw away) entire ranges of unused sector IDs.
    /// For example, if sectors 1-99 and 101-200 have been allocated, sector number
    /// 99 can be masked out to collapse these two ranges into one.
    fn compact_sector_numbers(
        rt: &impl Runtime,
        params: CompactSectorNumbersParams,
    ) -> Result<(), ActorError> {
        let mask_sector_numbers = params
            .mask_sector_numbers
            .validate()
            .map_err(|e| actor_error!(illegal_argument, "invalid mask bitfield: {}", e))?;

        let last_sector_number = mask_sector_numbers
            .last()
            .ok_or_else(|| actor_error!(illegal_argument, "invalid mask bitfield"))?
            as SectorNumber;

        if last_sector_number > MAX_SECTOR_NUMBER {
            return Err(actor_error!(
                illegal_argument,
                "masked sector number {} exceeded max sector number",
                last_sector_number
            ));
        }

        rt.transaction(|state: &mut State, rt| {
            let info = get_miner_info(rt.store(), state)?;

            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            state.allocate_sector_numbers(
                rt.store(),
                mask_sector_numbers,
                CollisionPolicy::AllowCollisions,
            )
        })?;

        Ok(())
    }

    /// Locks up some amount of a the miner's unlocked balance (including funds received alongside the invoking message).
    fn apply_rewards(rt: &impl Runtime, params: ApplyRewardParams) -> Result<(), ActorError> {
        if params.reward.is_negative() {
            return Err(actor_error!(
                illegal_argument,
                "cannot lock up a negative amount of funds"
            ));
        }
        if params.penalty.is_negative() {
            return Err(actor_error!(
                illegal_argument,
                "cannot penalize a negative amount of funds"
            ));
        }

        let (pledge_delta_total, to_burn) = rt.transaction(|st: &mut State, rt| {
            let mut pledge_delta_total = TokenAmount::zero();

            rt.validate_immediate_caller_is(std::iter::once(&REWARD_ACTOR_ADDR))?;

            let (reward_to_lock, locked_reward_vesting_spec) =
                locked_reward_from_reward(params.reward);

            // This ensures the miner has sufficient funds to lock up amountToLock.
            // This should always be true if reward actor sends reward funds with the message.
            let unlocked_balance = st.get_unlocked_balance(&rt.current_balance()).map_err(|e| {
                actor_error!(illegal_state, "failed to calculate unlocked balance: {}", e)
            })?;

            if unlocked_balance < reward_to_lock {
                return Err(actor_error!(
                    insufficient_funds,
                    "insufficient funds to lock, available: {}, requested: {}",
                    unlocked_balance,
                    reward_to_lock
                ));
            }

            let newly_vested = st
                .add_locked_funds(
                    rt.store(),
                    rt.curr_epoch(),
                    &reward_to_lock,
                    locked_reward_vesting_spec,
                )
                .map_err(|e| {
                    actor_error!(illegal_state, "failed to lock funds in vesting table: {}", e)
                })?;
            pledge_delta_total -= &newly_vested;
            pledge_delta_total += &reward_to_lock;

            st.apply_penalty(&params.penalty)
                .map_err(|e| actor_error!(illegal_state, "failed to apply penalty: {}", e))?;

            // Attempt to repay all fee debt in this call. In most cases the miner will have enough
            // funds in the *reward alone* to cover the penalty. In the rare case a miner incurs more
            // penalty than it can pay for with reward and existing funds, it will go into fee debt.
            let (penalty_from_vesting, penalty_from_balance) = st
                .repay_partial_debt_in_priority_order(
                    rt.store(),
                    rt.curr_epoch(),
                    &rt.current_balance(),
                )
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to repay penalty")
                })?;
            pledge_delta_total -= &penalty_from_vesting;
            let to_burn = penalty_from_vesting + penalty_from_balance;
            Ok((pledge_delta_total, to_burn))
        })?;

        notify_pledge_changed(rt, &pledge_delta_total)?;
        burn_funds(rt, to_burn)?;
        let st: State = rt.state()?;
        st.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;
        Ok(())
    }

    fn report_consensus_fault(
        rt: &impl Runtime,
        params: ReportConsensusFaultParams,
    ) -> Result<(), ActorError> {
        // Note: only the first report of any fault is processed because it sets the
        // ConsensusFaultElapsed state variable to an epoch after the fault, and reports prior to
        // that epoch are no longer valid
        rt.validate_immediate_caller_accept_any()?;
        let reporter = rt.message().caller();

        let fault = rt
            .verify_consensus_fault(&params.header1, &params.header2, &params.header_extra)
            .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_ARGUMENT, "fault not verified"))?
            .ok_or_else(|| actor_error!(illegal_argument, "No consensus fault found"))?;
        if fault.target != rt.message().receiver() {
            return Err(actor_error!(
                illegal_argument,
                "fault by {} reported to miner {}",
                fault.target,
                rt.message().receiver()
            ));
        }

        // Elapsed since the fault (i.e. since the higher of the two blocks)
        let fault_age = rt.curr_epoch() - fault.epoch;
        if fault_age <= 0 {
            return Err(actor_error!(
                illegal_argument,
                "invalid fault epoch {} ahead of current {}",
                fault.epoch,
                rt.curr_epoch()
            ));
        }

        // Reward reporter with a share of the miner's current balance.
        let reward_stats = request_current_epoch_block_reward(rt)?;

        // The policy amounts we should burn and send to reporter
        // These may differ from actual funds send when miner goes into fee debt
        let this_epoch_reward =
            TokenAmount::from_atto(reward_stats.this_epoch_reward_smoothed.estimate());
        let fault_penalty = consensus_fault_penalty(this_epoch_reward.clone());
        let slasher_reward = reward_for_consensus_slash_report(&this_epoch_reward);

        let mut pledge_delta = TokenAmount::zero();

        let (burn_amount, reward_amount) = rt.transaction(|st: &mut State, rt| {
            let mut info = get_miner_info(rt.store(), st)?;

            // Verify miner hasn't already been faulted
            if fault.epoch < info.consensus_fault_elapsed {
                return Err(actor_error!(
                    forbidden,
                    "fault epoch {} is too old, last exclusion period ended at {}",
                    fault.epoch,
                    info.consensus_fault_elapsed
                ));
            }

            st.apply_penalty(&fault_penalty).map_err(|e| {
                actor_error!(illegal_state, format!("failed to apply penalty: {}", e))
            })?;

            // Pay penalty
            let (penalty_from_vesting, penalty_from_balance) = st
                .repay_partial_debt_in_priority_order(
                    rt.store(),
                    rt.curr_epoch(),
                    &rt.current_balance(),
                )
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to pay fees")
                })?;

            let mut burn_amount = &penalty_from_vesting + &penalty_from_balance;
            pledge_delta -= penalty_from_vesting;

            // clamp reward at funds burnt
            let reward_amount = std::cmp::min(&burn_amount, &slasher_reward).clone();
            burn_amount -= &reward_amount;

            info.consensus_fault_elapsed =
                rt.curr_epoch() + rt.policy().consensus_fault_ineligibility_duration;

            st.save_info(rt.store(), &info).map_err(|e| {
                e.downcast_default(ExitCode::USR_SERIALIZATION, "failed to save miner info")
            })?;

            Ok((burn_amount, reward_amount))
        })?;

        if let Err(e) =
            extract_send_result(rt.send_simple(&reporter, METHOD_SEND, None, reward_amount))
        {
            error!("failed to send reward: {}", e);
        }

        burn_funds(rt, burn_amount)?;
        notify_pledge_changed(rt, &pledge_delta)?;

        let state: State = rt.state()?;
        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;
        Ok(())
    }

    fn withdraw_balance(
        rt: &impl Runtime,
        params: WithdrawBalanceParams,
    ) -> Result<WithdrawBalanceReturn, ActorError> {
        if params.amount_requested.is_negative() {
            return Err(actor_error!(
                illegal_argument,
                "negative fund requested for withdrawal: {}",
                params.amount_requested
            ));
        }

        let (info, amount_withdrawn, newly_vested, fee_to_burn, state) =
            rt.transaction(|state: &mut State, rt| {
                let mut info = get_miner_info(rt.store(), state)?;

                // Only the owner or the beneficiary is allowed to withdraw the balance.
                rt.validate_immediate_caller_is(&[info.owner, info.beneficiary])?;

                // Ensure we don't have any pending terminations.
                if !state.early_terminations.is_empty() {
                    return Err(actor_error!(
                        forbidden,
                        "cannot withdraw funds while {} deadlines have terminated sectors \
                        with outstanding fees",
                        state.early_terminations.len()
                    ));
                }

                // Unlock vested funds so we can spend them.
                let newly_vested =
                    state.unlock_vested_funds(rt.store(), rt.curr_epoch()).map_err(|e| {
                        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "Failed to vest fund")
                    })?;

                // available balance already accounts for fee debt so it is correct to call
                // this before RepayDebts. We would have to
                // subtract fee debt explicitly if we called this after.
                let available_balance =
                    state.get_available_balance(&rt.current_balance()).map_err(|e| {
                        actor_error!(
                            illegal_state,
                            format!("failed to calculate available balance: {}", e)
                        )
                    })?;

                // Verify unlocked funds cover both InitialPledgeRequirement and FeeDebt
                // and repay fee debt now.
                let fee_to_burn = repay_debts_or_abort(rt, state)?;
                let mut amount_withdrawn =
                    std::cmp::min(&available_balance, &params.amount_requested);
                if amount_withdrawn.is_negative() {
                    return Err(actor_error!(
                        illegal_state,
                        "negative amount to withdraw: {}",
                        amount_withdrawn
                    ));
                }
                if info.beneficiary != info.owner {
                    // remaining_quota always zero and positive
                    let remaining_quota = info.beneficiary_term.available(rt.curr_epoch());
                    if remaining_quota.is_zero() {
                        return Err(actor_error!(
                            forbidden,
                            "beneficiary expiration of epoch {} passed or quota of {} depleted with {} used",
                            info.beneficiary_term.expiration,
                            info.beneficiary_term.quota,
                            info.beneficiary_term.used_quota
                        ));
                    }
                    amount_withdrawn = std::cmp::min(amount_withdrawn, &remaining_quota);
                    if amount_withdrawn.is_positive() {
                        info.beneficiary_term.used_quota += amount_withdrawn;
                        state.save_info(rt.store(), &info).map_err(|e| {
                            e.downcast_default(
                                ExitCode::USR_ILLEGAL_STATE,
                                "failed to save miner info",
                            )
                        })?;
                    }
                    Ok((info, amount_withdrawn.clone(), newly_vested, fee_to_burn, state.clone()))
                } else {
                    Ok((info, amount_withdrawn.clone(), newly_vested, fee_to_burn, state.clone()))
                }
            })?;

        if amount_withdrawn.is_positive() {
            extract_send_result(rt.send_simple(
                &info.beneficiary,
                METHOD_SEND,
                None,
                amount_withdrawn.clone(),
            ))?;
        }

        burn_funds(rt, fee_to_burn)?;
        notify_pledge_changed(rt, &newly_vested.neg())?;

        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;
        Ok(WithdrawBalanceReturn { amount_withdrawn })
    }

    /// Proposes or confirms a change of beneficiary address.
    /// A proposal must be submitted by the owner, and takes effect after approval of both the proposed beneficiary and current beneficiary,
    /// if applicable, any current beneficiary that has time and quota remaining.
    //// See FIP-0029, https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0029.md
    fn change_beneficiary(
        rt: &impl Runtime,
        params: ChangeBeneficiaryParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let caller = rt.message().caller();
        let new_beneficiary =
            Address::new_id(rt.resolve_address(&params.new_beneficiary).ok_or_else(|| {
                actor_error!(
                    illegal_argument,
                    "unable to resolve address: {}",
                    params.new_beneficiary
                )
            })?);

        rt.transaction(|state: &mut State, rt| {
            let mut info = get_miner_info(rt.store(), state)?;
            if caller == info.owner {
                // This is a ChangeBeneficiary proposal when the caller is Owner
                if new_beneficiary != info.owner {
                    // When beneficiary is not owner, just check quota in params,
                    // Expiration maybe an expiration value, but wouldn't cause problem, just the new beneficiary never get any benefit
                    if !params.new_quota.is_positive() {
                        return Err(actor_error!(
                            illegal_argument,
                            "beneficial quota {} must bigger than zero",
                            params.new_quota
                        ));
                    }
                } else {
                    // Expiration/quota must set to 0 while change beneficiary to owner
                    if !params.new_quota.is_zero() {
                        return Err(actor_error!(
                            illegal_argument,
                            "owner beneficial quota {} must be zero",
                            params.new_quota
                        ));
                    }

                    if params.new_expiration != 0 {
                        return Err(actor_error!(
                            illegal_argument,
                            "owner beneficial expiration {} must be zero",
                            params.new_expiration
                        ));
                    }
                }

                let mut pending_beneficiary_term = PendingBeneficiaryChange::new(
                    new_beneficiary,
                    params.new_quota,
                    params.new_expiration,
                );
                if info.beneficiary_term.available(rt.curr_epoch()).is_zero() {
                    // Set current beneficiary to approved when current beneficiary is not effective
                    pending_beneficiary_term.approved_by_beneficiary = true;
                }
                info.pending_beneficiary_term = Some(pending_beneficiary_term);
            } else if let Some(pending_term) = &info.pending_beneficiary_term {
                if caller != info.beneficiary && caller != pending_term.new_beneficiary {
                    return Err(actor_error!(
                        forbidden,
                        "message caller {} is neither proposal beneficiary{} nor current beneficiary{}",
                        caller,
                        params.new_beneficiary,
                        info.beneficiary
                    ));
                }

                if pending_term.new_beneficiary != new_beneficiary {
                    return Err(actor_error!(
                        illegal_argument,
                        "new beneficiary address must be equal expect {}, but got {}",
                        pending_term.new_beneficiary,
                        params.new_beneficiary
                    ));
                }
                if pending_term.new_quota != params.new_quota {
                    return Err(actor_error!(
                        illegal_argument,
                        "new beneficiary quota must be equal expect {}, but got {}",
                        pending_term.new_quota,
                        params.new_quota
                    ));
                }
                if pending_term.new_expiration != params.new_expiration {
                    return Err(actor_error!(
                        illegal_argument,
                        "new beneficiary expire date must be equal expect {}, but got {}",
                        pending_term.new_expiration,
                        params.new_expiration
                    ));
                }
            } else {
                return Err(actor_error!(forbidden, "No changeBeneficiary proposal exists"));
            }

            if let Some(pending_term) = info.pending_beneficiary_term.as_mut() {
                if caller == info.beneficiary {
                    pending_term.approved_by_beneficiary = true
                }

                if caller == new_beneficiary {
                    pending_term.approved_by_nominee = true
                }

                if pending_term.approved_by_beneficiary && pending_term.approved_by_nominee {
                    //approved by both beneficiary and nominee
                    if new_beneficiary != info.beneficiary {
                        //if beneficiary changes, reset used_quota to zero
                        info.beneficiary_term.used_quota = TokenAmount::zero();
                    }
                    info.beneficiary = new_beneficiary;
                    info.beneficiary_term.quota = pending_term.new_quota.clone();
                    info.beneficiary_term.expiration = pending_term.new_expiration;
                    // clear the pending proposal
                    info.pending_beneficiary_term = None;
                }
            }

            state.save_info(rt.store(), &info).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save miner info")
            })?;
            Ok(())
        })
    }

    // GetBeneficiary retrieves the currently active and proposed beneficiary information.
    // This method is for use by other actors (such as those acting as beneficiaries),
    // and to abstract the state representation for clients.
    fn get_beneficiary(rt: &impl Runtime) -> Result<GetBeneficiaryReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        let info = get_miner_info(rt.store(), &st)?;

        Ok(GetBeneficiaryReturn {
            active: ActiveBeneficiary {
                beneficiary: info.beneficiary,
                term: info.beneficiary_term,
            },
            proposed: info.pending_beneficiary_term,
        })
    }

    fn repay_debt(rt: &impl Runtime) -> Result<(), ActorError> {
        let (from_vesting, from_balance, state) = rt.transaction(|state: &mut State, rt| {
            let info = get_miner_info(rt.store(), state)?;
            rt.validate_immediate_caller_is(
                info.control_addresses.iter().chain(&[info.worker, info.owner]),
            )?;

            // Repay as much fee debt as possible.
            let (from_vesting, from_balance) = state
                .repay_partial_debt_in_priority_order(
                    rt.store(),
                    rt.curr_epoch(),
                    &rt.current_balance(),
                )
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to unlock fee debt")
                })?;

            Ok((from_vesting, from_balance, state.clone()))
        })?;

        let burn_amount = from_balance + &from_vesting;
        notify_pledge_changed(rt, &from_vesting.neg())?;
        burn_funds(rt, burn_amount)?;

        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;
        Ok(())
    }

    fn on_deferred_cron_event(
        rt: &impl Runtime,
        params: DeferredCronEventParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&STORAGE_POWER_ACTOR_ADDR))?;

        let payload: CronEventPayload = from_slice(&params.event_payload).map_err(|e| {
            actor_error!(
                illegal_state,
                format!("failed to unmarshal miner cron payload into expected structure: {}", e)
            )
        })?;

        match payload.event_type {
            CRON_EVENT_PROVING_DEADLINE => handle_proving_deadline(
                rt,
                &params.reward_smoothed,
                &params.quality_adj_power_smoothed,
            )?,
            CRON_EVENT_PROCESS_EARLY_TERMINATIONS => {
                if process_early_terminations(
                    rt,
                    &params.reward_smoothed,
                    &params.quality_adj_power_smoothed,
                )? {
                    schedule_early_termination_work(rt)?
                }
            }
            _ => {
                error!("onDeferredCronEvent invalid event type: {}", payload.event_type);
            }
        };
        let state: State = rt.state()?;
        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Clone)]
struct SectorPreCommitInfoInner {
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

/// ReplicaUpdate param with Option<Cid> for CommD
/// None means unknown
#[derive(Debug, Clone)]
pub struct ReplicaUpdateInner {
    pub sector_number: SectorNumber,
    pub deadline: u64,
    pub partition: u64,
    pub new_sealed_cid: Cid,
    /// None means unknown
    pub new_unsealed_cid: Option<Cid>,
    pub deals: Vec<DealID>,
    pub update_proof_type: RegisteredUpdateProof,
    pub replica_proof: RawBytes,
}

enum ExtensionKind {
    // handle only legacy sectors
    ExtendCommittmentLegacy,
    // handle both Simple QAP and legacy sectors
    ExtendCommittment,
}

// ExtendSectorExpiration param
struct ExtendExpirationsInner {
    extensions: Vec<ValidatedExpirationExtension>,
    // Map from sector being extended to (check, maintain)
    // `check` is the space of active claims, checked to ensure all claims are checked
    // `maintain` is the space of claims to maintain
    // maintain <= check with equality in the case no claims are dropped
    claims: Option<BTreeMap<SectorNumber, (u64, u64)>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedExpirationExtension {
    pub deadline: u64,
    pub partition: u64,
    pub sectors: BitField,
    pub new_expiration: ChainEpoch,
}

impl From<ExpirationExtension2> for ValidatedExpirationExtension {
    fn from(e2: ExpirationExtension2) -> Self {
        let mut sectors = BitField::new();
        for sc in e2.sectors_with_claims {
            sectors.set(sc.sector_number)
        }
        sectors |= &e2.sectors;

        Self {
            deadline: e2.deadline,
            partition: e2.partition,
            sectors,
            new_expiration: e2.new_expiration,
        }
    }
}

fn validate_legacy_extension_declarations(
    extensions: &[ExpirationExtension],
    policy: &Policy,
) -> Result<ExtendExpirationsInner, ActorError> {
    let vec_validated = extensions
        .iter()
        .map(|decl| {
            if decl.deadline >= policy.wpost_period_deadlines {
                return Err(actor_error!(
                    illegal_argument,
                    "deadline {} not in range 0..{}",
                    decl.deadline,
                    policy.wpost_period_deadlines
                ));
            }

            Ok(ValidatedExpirationExtension {
                deadline: decl.deadline,
                partition: decl.partition,
                sectors: decl.sectors.clone(),
                new_expiration: decl.new_expiration,
            })
        })
        .collect::<Result<_, _>>()?;

    Ok(ExtendExpirationsInner { extensions: vec_validated, claims: None })
}

fn validate_extension_declarations(
    rt: &impl Runtime,
    extensions: Vec<ExpirationExtension2>,
) -> Result<ExtendExpirationsInner, ActorError> {
    let mut claim_space_by_sector = BTreeMap::<SectorNumber, (u64, u64)>::new();

    for decl in &extensions {
        let policy = rt.policy();
        if decl.deadline >= policy.wpost_period_deadlines {
            return Err(actor_error!(
                illegal_argument,
                "deadline {} not in range 0..{}",
                decl.deadline,
                policy.wpost_period_deadlines
            ));
        }

        for sc in &decl.sectors_with_claims {
            let mut drop_claims = sc.drop_claims.clone();
            let mut all_claim_ids = sc.maintain_claims.clone();
            all_claim_ids.append(&mut drop_claims);
            let claims = get_claims(rt, &all_claim_ids)
                .with_context(|| format!("failed to get claims for sector {}", sc.sector_number))?;
            let first_drop = sc.maintain_claims.len();

            for (i, claim) in claims.iter().enumerate() {
                // check provider and sector matches
                if claim.provider != rt.message().receiver().id().unwrap() {
                    return Err(actor_error!(illegal_argument, "failed to validate declaration sector={}, claim={}, expected claim provider to be {} but found {} ", sc.sector_number, all_claim_ids[i], rt.message().receiver().id().unwrap(), claim.provider));
                }
                if claim.sector != sc.sector_number {
                    return Err(actor_error!(illegal_argument, "failed to validate declaration sector={}, claim={} expected claim sector number to be {} but found {} ", sc.sector_number, all_claim_ids[i], sc.sector_number, claim.sector));
                }

                // If we are not dropping check expiration does not exceed term max
                let mut maintain_delta: u64 = 0;
                if i < first_drop {
                    if decl.new_expiration > claim.term_start + claim.term_max {
                        return Err(actor_error!(forbidden, "failed to validate declaration sector={}, claim={} claim only allows extension to {} but declared new expiration is {}", sc.sector_number, sc.maintain_claims[i], claim.term_start + claim.term_max, decl.new_expiration));
                    }
                    maintain_delta = claim.size.0
                }

                claim_space_by_sector
                    .entry(sc.sector_number)
                    .and_modify(|(check, maintain)| {
                        *check += claim.size.0;
                        *maintain += maintain_delta;
                    })
                    .or_insert((claim.size.0, maintain_delta));
            }
        }
    }
    Ok(ExtendExpirationsInner {
        extensions: extensions.into_iter().map(|e2| e2.into()).collect(),
        claims: Some(claim_space_by_sector),
    })
}

#[allow(clippy::too_many_arguments)]
fn extend_sector_committment(
    policy: &Policy,
    curr_epoch: ChainEpoch,
    reward_stats: &ThisEpochRewardReturn,
    power_stats: &ext::power::CurrentTotalPowerReturn,
    new_expiration: ChainEpoch,
    sector: &SectorOnChainInfo,
    sector_size: SectorSize,
    claim_space_by_sector: &BTreeMap<SectorNumber, (u64, u64)>,
) -> Result<SectorOnChainInfo, ActorError> {
    validate_extended_expiration(policy, curr_epoch, new_expiration, sector)?;

    // all simple_qa_power sectors with VerifiedDealWeight > 0 MUST check all claims
    if sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER) {
        extend_simple_qap_sector(
            policy,
            new_expiration,
            curr_epoch,
            reward_stats,
            power_stats,
            sector,
            sector_size,
            claim_space_by_sector,
        )
    } else {
        extend_non_simple_qap_sector(new_expiration, curr_epoch, sector)
    }
}

fn extend_sector_committment_legacy(
    policy: &Policy,
    curr_epoch: ChainEpoch,
    new_expiration: ChainEpoch,
    sector: &SectorOnChainInfo,
) -> Result<SectorOnChainInfo, ActorError> {
    validate_extended_expiration(policy, curr_epoch, new_expiration, sector)?;

    // it is an error to do legacy sector expiration on simple-qa power sectors with deal weight
    if sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER)
        && (sector.verified_deal_weight > BigInt::zero() || sector.deal_weight > BigInt::zero())
    {
        return Err(actor_error!(
            forbidden,
            "cannot use legacy sector extension for simple qa power with deal weight {}",
            sector.sector_number
        ));
    }
    extend_non_simple_qap_sector(new_expiration, curr_epoch, sector)
}

fn validate_extended_expiration(
    policy: &Policy,
    curr_epoch: ChainEpoch,
    new_expiration: ChainEpoch,
    sector: &SectorOnChainInfo,
) -> Result<(), ActorError> {
    if !can_extend_seal_proof_type(sector.seal_proof) {
        return Err(actor_error!(
            forbidden,
            "cannot extend expiration for sector {} with unsupported \
            seal type {:?}",
            sector.sector_number,
            sector.seal_proof
        ));
    }
    // This can happen if the sector should have already expired, but hasn't
    // because the end of its deadline hasn't passed yet.
    if sector.expiration < curr_epoch {
        return Err(actor_error!(
            forbidden,
            "cannot extend expiration for expired sector {} at {}",
            sector.sector_number,
            sector.expiration
        ));
    }

    if new_expiration < sector.expiration {
        return Err(actor_error!(
            illegal_argument,
            "cannot reduce sector {} expiration to {} from {}",
            sector.sector_number,
            new_expiration,
            sector.expiration
        ));
    }

    validate_expiration(policy, curr_epoch, sector.activation, new_expiration, sector.seal_proof)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn extend_simple_qap_sector(
    policy: &Policy,
    new_expiration: ChainEpoch,
    curr_epoch: ChainEpoch,
    reward_stats: &ThisEpochRewardReturn,
    power_stats: &ext::power::CurrentTotalPowerReturn,
    sector: &SectorOnChainInfo,
    sector_size: SectorSize,
    claim_space_by_sector: &BTreeMap<SectorNumber, (u64, u64)>,
) -> Result<SectorOnChainInfo, ActorError> {
    let mut new_sector = sector.clone();

    new_sector.expiration = new_expiration;
    new_sector.power_base_epoch = curr_epoch;
    let old_duration = sector.expiration - sector.power_base_epoch;
    let new_duration = new_sector.expiration - new_sector.power_base_epoch;

    // Update the non-verified deal weights. This won't change power, it'll just keep it the same
    // relative to the updated power base epoch.
    if sector.deal_weight.is_positive() {
        // (old_deal_weight) / old_duration -> old_space
        // old_space * (old_expiration - curr_epoch) -> remaining spacetime in the deals.
        new_sector.deal_weight =
            &sector.deal_weight * (sector.expiration - curr_epoch) / old_duration;
    }

    // Update the verified deal weights, and pledge if necessary.
    if sector.verified_deal_weight.is_positive() {
        let old_verified_deal_space = &sector.verified_deal_weight / old_duration;
        let (expected_verified_deal_space, new_verified_deal_space) = match claim_space_by_sector
            .get(&sector.sector_number)
        {
            None => {
                return Err(actor_error!(
                        illegal_argument,
                        "claim missing from declaration for sector {} with non-zero verified deal weight {}",
                        sector.sector_number,
                        &sector.verified_deal_weight
                    ));
            }
            Some(space) => space,
        };
        // claims must be completely accounted for
        if BigInt::from(*expected_verified_deal_space as i64) != old_verified_deal_space {
            return Err(actor_error!(illegal_argument, "declared verified deal space in claims ({}) does not match verified deal space ({}) for sector {}", expected_verified_deal_space, old_verified_deal_space, sector.sector_number));
        }
        // claim dropping is restricted to extensions at the end of a sector's life

        let dropping_claims = expected_verified_deal_space != new_verified_deal_space;
        if dropping_claims && sector.expiration - curr_epoch > policy.end_of_life_claim_drop_period
        {
            return Err(actor_error!(
                forbidden,
                "attempt to drop claims with {} epochs > end of life claim drop period {} remaining",
                sector.expiration - curr_epoch,
                policy.end_of_life_claim_drop_period
            ));
        }

        new_sector.verified_deal_weight = BigInt::from(*new_verified_deal_space) * new_duration;

        // We only bother updating the expected_day_reward, expected_storage_pledge, and replaced_day_reward
        //  for verified deals, as it can increase power.
        let qa_pow =
            qa_power_for_weight(sector_size, new_duration, &new_sector.verified_deal_weight);
        new_sector.expected_day_reward = expected_reward_for_power(
            &reward_stats.this_epoch_reward_smoothed,
            &power_stats.quality_adj_power_smoothed,
            &qa_pow,
            fil_actors_runtime::network::EPOCHS_IN_DAY,
        );
        new_sector.expected_storage_pledge = max(
            sector.expected_storage_pledge.clone(),
            expected_reward_for_power(
                &reward_stats.this_epoch_reward_smoothed,
                &power_stats.quality_adj_power_smoothed,
                &qa_pow,
                INITIAL_PLEDGE_PROJECTION_PERIOD,
            ),
        );
        new_sector.replaced_day_reward =
            max(sector.expected_day_reward.clone(), sector.replaced_day_reward.clone());
    }

    Ok(new_sector)
}

fn extend_non_simple_qap_sector(
    new_expiration: ChainEpoch,
    curr_epoch: ChainEpoch,
    sector: &SectorOnChainInfo,
) -> Result<SectorOnChainInfo, ActorError> {
    let mut new_sector = sector.clone();
    // Remove "spent" deal weights for non simple_qa_power sectors with deal weight > 0
    let new_deal_weight = (&sector.deal_weight * (sector.expiration - curr_epoch))
        .div_floor(&BigInt::from(sector.expiration - sector.power_base_epoch));

    let new_verified_deal_weight = (&sector.verified_deal_weight
        * (sector.expiration - curr_epoch))
        .div_floor(&BigInt::from(sector.expiration - sector.power_base_epoch));

    new_sector.expiration = new_expiration;
    new_sector.deal_weight = new_deal_weight;
    new_sector.verified_deal_weight = new_verified_deal_weight;
    new_sector.power_base_epoch = curr_epoch;

    Ok(new_sector)
}

// Validates a list of replica update requests and parallel sector infos.
// Returns all pairs of update and sector info, even those that fail validation.
// The proof verification inputs are needed as witnesses to verify an aggregate proof to allow
// other, valid, updates to succeed.
#[allow(clippy::too_many_arguments)]
fn validate_replica_updates<'a, BS>(
    updates: &'a [ReplicaUpdateInner],
    sector_infos: &'a [SectorOnChainInfo],
    state: &State,
    sector_size: SectorSize,
    policy: &Policy,
    curr_epoch: ChainEpoch,
    store: BS,
    require_deals: bool,
    all_or_nothing: bool,
) -> Result<(BatchReturn, Vec<UpdateAndSectorInfo<'a>>), ActorError>
where
    BS: Blockstore,
{
    let mut sector_numbers = BTreeSet::<SectorNumber>::new();
    let mut validate_one = |update: &ReplicaUpdateInner,
                            sector_info: &SectorOnChainInfo|
     -> Result<(), ActorError> {
        if !sector_numbers.insert(update.sector_number) {
            return Err(actor_error!(
                illegal_argument,
                "skipping duplicate sector {}",
                update.sector_number
            ));
        }

        if update.replica_proof.len() > 4096 {
            return Err(actor_error!(
                illegal_argument,
                "update proof is too large ({}), skipping sector {}",
                update.replica_proof.len(),
                update.sector_number
            ));
        }

        if require_deals && update.deals.is_empty() {
            return Err(actor_error!(
                illegal_argument,
                "must have deals to update, skipping sector {}",
                update.sector_number
            ));
        }

        if update.deals.len() as u64 > sector_deals_max(policy, sector_size) {
            return Err(actor_error!(
                illegal_argument,
                "more deals than policy allows, skipping sector {}",
                update.sector_number
            ));
        }

        if update.deadline >= policy.wpost_period_deadlines {
            return Err(actor_error!(
                illegal_argument,
                "deadline {} not in range 0..{}, skipping sector {}",
                update.deadline,
                policy.wpost_period_deadlines,
                update.sector_number
            ));
        }

        if !is_sealed_sector(&update.new_sealed_cid) {
            return Err(actor_error!(
                illegal_argument,
                "new sealed CID had wrong prefix {}, skipping sector {}",
                update.new_sealed_cid,
                update.sector_number
            ));
        }

        // Disallow upgrading sectors in immutable deadlines.
        if !deadline_is_mutable(
            policy,
            state.current_proving_period_start(policy, curr_epoch),
            update.deadline,
            curr_epoch,
        ) {
            return Err(actor_error!(
                illegal_argument,
                "cannot upgrade sectors in immutable deadline {}, skipping sector {}",
                update.deadline,
                update.sector_number
            ));
        }

        // This inefficiently loads deadline/partition info for each update.
        if !state.check_sector_active(
            &store,
            update.deadline,
            update.partition,
            update.sector_number,
            true,
        )? {
            return Err(actor_error!(
                illegal_argument,
                "sector isn't active, skipping sector {}",
                update.sector_number
            ));
        }

        if (&sector_info.deal_weight + &sector_info.verified_deal_weight) != DealWeight::zero() {
            return Err(actor_error!(
                illegal_argument,
                "cannot update sector with non-zero data, skipping sector {}",
                update.sector_number
            ));
        }

        let expected_proof_type = sector_info
            .seal_proof
            .registered_update_proof()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "couldn't load update proof type")?;
        if update.update_proof_type != expected_proof_type {
            return Err(actor_error!(
                illegal_argument,
                "expected proof type {}, was {}",
                i64::from(expected_proof_type),
                i64::from(update.update_proof_type)
            ));
        }
        Ok(())
    };

    let mut batch = BatchReturnGen::new(updates.len());
    let mut update_sector_infos: Vec<UpdateAndSectorInfo> = Vec::with_capacity(updates.len());
    for (i, (update, sector_info)) in updates.iter().zip(sector_infos).enumerate() {
        // Build update and sector info for all updates, even if they fail validation.
        update_sector_infos.push(UpdateAndSectorInfo { update, sector_info });

        match validate_one(update, sector_info) {
            Ok(_) => {
                batch.add_success();
            }
            Err(e) => {
                let e = e.wrap(format!("invalid update {} while requiring activation success", i));
                info!("{}", e.msg());
                if all_or_nothing {
                    return Err(e);
                }
                batch.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
            }
        }
    }
    Ok((batch.gen(), update_sector_infos))
}

fn update_replica_states<BS>(
    rt: &impl Runtime,
    updates_by_deadline: &BTreeMap<u64, Vec<ReplicaUpdateStateInputs>>,
    expected_count: usize,
    sectors: &mut Sectors<BS>,
    sector_size: SectorSize,
) -> Result<(PowerPair, TokenAmount), ActorError>
where
    BS: Blockstore,
{
    let rew = request_current_epoch_block_reward(rt)?;
    let pow = request_current_total_power(rt)?;
    let circulating_supply = rt.total_fil_circ_supply();
    let pledge_inputs = NetworkPledgeInputs {
        network_qap: pow.quality_adj_power_smoothed,
        network_baseline: rew.this_epoch_baseline_power,
        circulating_supply,
        epoch_reward: rew.this_epoch_reward_smoothed,
        epochs_since_ramp_start: rt.curr_epoch() - pow.ramp_start_epoch,
        ramp_duration_epochs: pow.ramp_duration_epochs,
    };
    let mut power_delta = PowerPair::zero();
    let mut pledge_delta = TokenAmount::zero();

    rt.transaction(|state: &mut State, rt| {
        let mut deadlines = state.load_deadlines(rt.store())?;
        let mut new_sectors = Vec::with_capacity(expected_count);
        // Process updates grouped by deadline.
        for (&dl_idx, updates) in updates_by_deadline {
            let mut deadline = deadlines.load_deadline(rt.store(), dl_idx)?;

            let mut partitions = deadline
                .partitions_amt(rt.store())
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    format!("failed to load partitions for deadline {}", dl_idx)
                })?;

            let quant = state.quant_spec_for_deadline(rt.policy(), dl_idx);

            for update in updates {
                // Compute updated sector info.
                let new_sector_info = update_existing_sector_info(
                    update.sector_info,
                    &update.activated_data,
                    &pledge_inputs,
                    sector_size,
                    rt.curr_epoch(),
                );

                let mut partition = partitions
                    .get(update.partition)
                    .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                        format!(
                            "failed to load deadline {} partition {}",
                            update.deadline, update.partition
                        )
                    })?
                    .cloned()
                    .ok_or_else(|| {
                        actor_error!(
                            not_found,
                            "no such deadline {} partition {}",
                            dl_idx,
                            update.partition
                        )
                    })?;

                // Note: replacing sectors one at a time in each partition is inefficient.
                let (partition_power_delta, partition_pledge_delta) = partition
                    .replace_sectors(
                        rt.store(),
                        std::slice::from_ref(update.sector_info),
                        std::slice::from_ref(&new_sector_info),
                        sector_size,
                        quant,
                    )
                    .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                        format!(
                            "failed to replace sector at deadline {} partition {}",
                            update.deadline, update.partition
                        )
                    })?;

                power_delta += &partition_power_delta;
                pledge_delta += &partition_pledge_delta;

                partitions.set(update.partition, partition).with_context_code(
                    ExitCode::USR_ILLEGAL_STATE,
                    || {
                        format!(
                            "failed to save deadline {} partition {}",
                            update.deadline, update.partition
                        )
                    },
                )?;

                new_sectors.push(new_sector_info);
            } // End loop over declarations in one deadline.

            deadline.partitions =
                partitions.flush().with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    format!("failed to save partitions for deadline {}", dl_idx)
                })?;

            deadlines
                .update_deadline(rt.policy(), rt.store(), dl_idx, &deadline)
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    format!("failed to save deadline {}", dl_idx)
                })?;
        } // End loop over deadlines

        if new_sectors.len() != expected_count {
            return Err(actor_error!(
                illegal_state,
                "unexpected new_sectors len {} != {}",
                new_sectors.len(),
                expected_count
            ));
        }

        // Overwrite sector infos.
        sectors.store(new_sectors).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to update sector infos")
        })?;

        state.sectors = sectors.amt.flush().map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save sectors")
        })?;
        state.save_deadlines(rt.store(), deadlines).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save deadlines")
        })?;

        // Update pledge.
        let current_balance = rt.current_balance();
        if pledge_delta.is_positive() {
            let unlocked_balance = state.get_unlocked_balance(&current_balance).map_err(|e| {
                actor_error!(illegal_state, "failed to calculate unlocked balance: {}", e)
            })?;
            if unlocked_balance < pledge_delta {
                return Err(actor_error!(
                    insufficient_funds,
                    "insufficient funds for aggregate initial pledge requirement {}, available: {}",
                    pledge_delta,
                    unlocked_balance
                ));
            }
        }

        state
            .add_initial_pledge(&pledge_delta)
            .map_err(|e| actor_error!(illegal_state, "failed to add initial pledge: {}", e))?;

        state.check_balance_invariants(&current_balance).map_err(balance_invariants_broken)?;
        Ok(())
    })?;
    Ok((power_delta, pledge_delta))
}

// Builds a new sector info representing newly activated data in an existing sector.
fn update_existing_sector_info(
    sector_info: &SectorOnChainInfo,
    activated_data: &ReplicaUpdateActivatedData,
    pledge_inputs: &NetworkPledgeInputs,
    sector_size: SectorSize,
    curr_epoch: ChainEpoch,
) -> SectorOnChainInfo {
    let mut new_sector_info = sector_info.clone();

    new_sector_info.flags.set(SectorOnChainInfoFlags::SIMPLE_QA_POWER, true);
    new_sector_info.sealed_cid = activated_data.seal_cid;
    new_sector_info.sector_key_cid = match new_sector_info.sector_key_cid {
        None => Some(sector_info.sealed_cid),
        Some(x) => Some(x),
    };

    new_sector_info.power_base_epoch = curr_epoch;

    let duration = new_sector_info.expiration - new_sector_info.power_base_epoch;

    new_sector_info.deal_weight = activated_data.unverified_space.clone() * duration;
    new_sector_info.verified_deal_weight = activated_data.verified_space.clone() * duration;

    // compute initial pledge
    let qa_pow = qa_power_for_weight(sector_size, duration, &new_sector_info.verified_deal_weight);

    new_sector_info.replaced_day_reward =
        max(&sector_info.expected_day_reward, &sector_info.replaced_day_reward).clone();
    new_sector_info.expected_day_reward = expected_reward_for_power(
        &pledge_inputs.epoch_reward,
        &pledge_inputs.network_qap,
        &qa_pow,
        fil_actors_runtime::network::EPOCHS_IN_DAY,
    );
    new_sector_info.expected_storage_pledge = max(
        new_sector_info.expected_storage_pledge,
        expected_reward_for_power(
            &pledge_inputs.epoch_reward,
            &pledge_inputs.network_qap,
            &qa_pow,
            INITIAL_PLEDGE_PROJECTION_PERIOD,
        ),
    );

    new_sector_info.initial_pledge = max(
        new_sector_info.initial_pledge,
        initial_pledge_for_power(
            &qa_pow,
            &pledge_inputs.network_baseline,
            &pledge_inputs.epoch_reward,
            &pledge_inputs.network_qap,
            &pledge_inputs.circulating_supply,
            pledge_inputs.epochs_since_ramp_start,
            pledge_inputs.ramp_duration_epochs,
        ),
    );
    new_sector_info
}

// Note: We're using the current power+epoch reward, rather than at time of termination.
fn process_early_terminations(
    rt: &impl Runtime,
    reward_smoothed: &FilterEstimate,
    quality_adj_power_smoothed: &FilterEstimate,
) -> Result</* more */ bool, ActorError> {
    let mut terminated_sector_nums = vec![];
    let mut sectors_with_data = vec![];
    let (result, more, penalty, pledge_delta) = rt.transaction(|state: &mut State, rt| {
        let store = rt.store();
        let policy = rt.policy();

        let (result, more) = state
            .pop_early_terminations(
                policy,
                store,
                policy.addressed_partitions_max,
                policy.addressed_sectors_max,
            )
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to pop early terminations")?;

        // Nothing to do, don't waste any time.
        // This can happen if we end up processing early terminations
        // before the cron callback fires.
        if result.is_empty() {
            info!("no early terminations (maybe cron callback hasn't happened yet?)");
            return Ok((result, more, TokenAmount::zero(), TokenAmount::zero()));
        }

        let info = get_miner_info(rt.store(), state)?;
        let sectors = Sectors::load(store, &state.sectors).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load sectors array")
        })?;

        let mut total_initial_pledge = TokenAmount::zero();
        let mut total_penalty = TokenAmount::zero();

        for (epoch, sector_numbers) in result.iter() {
            let sectors = sectors
                .load_sector(sector_numbers)
                .map_err(|e| e.wrap("failed to load sector infos"))?;

            for sector in &sectors {
                total_initial_pledge += &sector.initial_pledge;
                let sector_power = qa_power_for_sector(info.sector_size, sector);
                terminated_sector_nums.push(sector.sector_number);
                total_penalty += pledge_penalty_for_termination(
                    &sector.expected_day_reward,
                    epoch - sector.power_base_epoch,
                    &sector.expected_storage_pledge,
                    quality_adj_power_smoothed,
                    &sector_power,
                    reward_smoothed,
                    &sector.replaced_day_reward,
                    sector.power_base_epoch - sector.activation,
                );
                if sector.deal_weight.is_positive() || sector.verified_deal_weight.is_positive() {
                    sectors_with_data.push(sector.sector_number);
                }
            }
        }

        // Apply penalty (add to fee debt)
        state
            .apply_penalty(&total_penalty)
            .map_err(|e| actor_error!(illegal_state, "failed to apply penalty: {}", e))?;

        // Remove pledge requirement.
        let mut pledge_delta = -total_initial_pledge;
        state.add_initial_pledge(&pledge_delta).map_err(|e| {
            actor_error!(illegal_state, "failed to add initial pledge {}: {}", pledge_delta, e)
        })?;

        // Use unlocked pledge to pay down outstanding fee debt
        let (penalty_from_vesting, penalty_from_balance) = state
            .repay_partial_debt_in_priority_order(
                rt.store(),
                rt.curr_epoch(),
                &rt.current_balance(),
            )
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to repay penalty")
            })?;

        let penalty = &penalty_from_vesting + penalty_from_balance;
        pledge_delta -= penalty_from_vesting;

        Ok((result, more, penalty, pledge_delta))
    })?;

    // We didn't do anything, abort.
    if result.is_empty() {
        info!("no early terminations");
        return Ok(more);
    }

    // Burn penalty.
    log::debug!(
        "storage provider {} penalized {} for sector termination",
        rt.message().receiver(),
        penalty
    );
    burn_funds(rt, penalty)?;

    // Return pledge.
    notify_pledge_changed(rt, &pledge_delta)?;

    // Terminate deals.
    let terminated_data = BitField::try_from_bits(sectors_with_data)
        .context_code(ExitCode::USR_ILLEGAL_STATE, "invalid sector number")?;
    request_terminate_deals(rt, rt.curr_epoch(), &terminated_data)?;

    for sector in terminated_sector_nums {
        emit::sector_terminated(rt, sector)?;
    }

    // reschedule cron worker, if necessary.
    Ok(more)
}

/// Invoked at the end of the last epoch for each proving deadline.
fn handle_proving_deadline(
    rt: &impl Runtime,
    reward_smoothed: &FilterEstimate,
    quality_adj_power_smoothed: &FilterEstimate,
) -> Result<(), ActorError> {
    let curr_epoch = rt.curr_epoch();

    let mut had_early_terminations = false;

    let mut power_delta_total = PowerPair::zero();
    let mut penalty_total = TokenAmount::zero();
    let mut pledge_delta_total = TokenAmount::zero();
    let mut continue_cron = false;

    let state: State = rt.transaction(|state: &mut State, rt| {
        let policy = rt.policy();

        // Vesting rewards for a miner are quantized to every 12 hours and we can determine what those "vesting epochs" are.
        // So, only do the vesting here if the current epoch is a "vesting epoch"
        let q = QuantSpec {
            unit: REWARD_VESTING_SPEC.quantization,
            offset: state.proving_period_start,
        };

        if q.quantize_up(curr_epoch) == curr_epoch {
            // Vest locked funds.
            // This happens first so that any subsequent penalties are taken
            // from locked vesting funds before funds free this epoch.
            let newly_vested =
                state.unlock_vested_funds(rt.store(), rt.curr_epoch()).map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to vest funds")
                })?;

            pledge_delta_total -= newly_vested;
        }

        // Process pending worker change if any
        let mut info = get_miner_info(rt.store(), state)?;
        process_pending_worker(&mut info, rt, state)?;

        let deposit_to_burn = state
            .cleanup_expired_pre_commits(policy, rt.store(), rt.curr_epoch())
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to expire pre-committed sectors",
                )
            })?;
        state
            .apply_penalty(&deposit_to_burn)
            .map_err(|e| actor_error!(illegal_state, "failed to apply penalty: {}", e))?;

        log::debug!(
            "storage provider {} penalized {} for expired pre commits",
            rt.message().receiver(),
            deposit_to_burn
        );

        // Record whether or not we _had_ early terminations in the queue before this method.
        // That way, don't re-schedule a cron callback if one is already scheduled.
        had_early_terminations = have_pending_early_terminations(state);

        let result = state.advance_deadline(policy, rt.store(), rt.curr_epoch()).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to advance deadline")
        })?;

        // Faults detected by this missed PoSt pay no penalty, but sectors that were already faulty
        // and remain faulty through this deadline pay the fault fee.
        let penalty_target = pledge_penalty_for_continued_fault(
            reward_smoothed,
            quality_adj_power_smoothed,
            &result.previously_faulty_power.qa,
        );

        power_delta_total += &result.power_delta;
        pledge_delta_total += &result.pledge_delta;

        state
            .apply_penalty(&penalty_target)
            .map_err(|e| actor_error!(illegal_state, "failed to apply penalty: {}", e))?;

        log::debug!(
            "storage provider {} penalized {} for continued fault",
            rt.message().receiver(),
            penalty_target
        );

        let (penalty_from_vesting, penalty_from_balance) = state
            .repay_partial_debt_in_priority_order(
                rt.store(),
                rt.curr_epoch(),
                &rt.current_balance(),
            )
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to unlock penalty")
            })?;

        penalty_total = &penalty_from_vesting + penalty_from_balance;
        pledge_delta_total -= penalty_from_vesting;

        continue_cron = state.continue_deadline_cron();
        if !continue_cron {
            state.deadline_cron_active = false;
        }

        Ok(state.clone())
    })?;

    // Remove power for new faults, and burn penalties.
    request_update_power(rt, power_delta_total)?;
    burn_funds(rt, penalty_total)?;
    notify_pledge_changed(rt, &pledge_delta_total)?;

    // Schedule cron callback for next deadline's last epoch.
    if continue_cron {
        let new_deadline_info = state.deadline_info(rt.policy(), curr_epoch + 1);
        enroll_cron_event(
            rt,
            new_deadline_info.last(),
            CronEventPayload { event_type: CRON_EVENT_PROVING_DEADLINE },
        )?;
    } else {
        info!("miner {} going inactive, deadline cron discontinued", rt.message().receiver())
    }

    // Record whether or not we _have_ early terminations now.
    let has_early_terminations = have_pending_early_terminations(&state);

    // If we didn't have pending early terminations before, but we do now,
    // handle them at the next epoch.
    if !had_early_terminations && has_early_terminations {
        // First, try to process some of these terminations.
        if process_early_terminations(rt, reward_smoothed, quality_adj_power_smoothed)? {
            // If that doesn't work, just defer till the next epoch.
            schedule_early_termination_work(rt)?;
        }

        // Note: _don't_ process early terminations if we had a cron
        // callback already scheduled. In that case, we'll already have
        // processed AddressedSectorsMax terminations this epoch.
    }

    Ok(())
}

fn validate_expiration(
    policy: &Policy,
    curr_epoch: ChainEpoch,
    activation: ChainEpoch,
    expiration: ChainEpoch,
    seal_proof: RegisteredSealProof,
) -> Result<(), ActorError> {
    // Expiration must be after activation. Check this explicitly to avoid an underflow below.
    if expiration <= activation {
        return Err(actor_error!(
            illegal_argument,
            "sector expiration {} must be after activation {}",
            expiration,
            activation
        ));
    }

    // expiration cannot be less than minimum after activation
    if expiration - activation < policy.min_sector_expiration {
        return Err(actor_error!(
            illegal_argument,
            "invalid expiration {}, total sector lifetime ({}) must exceed {} after activation {}",
            expiration,
            expiration - activation,
            policy.min_sector_expiration,
            activation
        ));
    }

    // expiration cannot exceed MaxSectorExpirationExtension from now
    if expiration > curr_epoch + policy.max_sector_expiration_extension {
        return Err(actor_error!(
            illegal_argument,
            "invalid expiration {}, cannot be more than {} past current epoch {}",
            expiration,
            policy.max_sector_expiration_extension,
            curr_epoch
        ));
    }

    // total sector lifetime cannot exceed SectorMaximumLifetime for the sector's seal proof
    let max_lifetime = seal_proof_sector_maximum_lifetime(seal_proof).ok_or_else(|| {
        actor_error!(illegal_argument, "unrecognized seal proof type {:?}", seal_proof)
    })?;
    if expiration - activation > max_lifetime {
        return Err(actor_error!(
        illegal_argument,
        "invalid expiration {}, total sector lifetime ({}) cannot exceed {} after activation {}",
        expiration,
        expiration - activation,
        max_lifetime,
        activation
    ));
    }

    Ok(())
}

fn enroll_cron_event(
    rt: &impl Runtime,
    event_epoch: ChainEpoch,
    cb: CronEventPayload,
) -> Result<(), ActorError> {
    let payload = serialize(&cb, "cron payload")?;
    let ser_params =
        IpldBlock::serialize_cbor(&ext::power::EnrollCronEventParams { event_epoch, payload })?;
    extract_send_result(rt.send_simple(
        &STORAGE_POWER_ACTOR_ADDR,
        ext::power::ENROLL_CRON_EVENT_METHOD,
        ser_params,
        TokenAmount::zero(),
    ))?;

    Ok(())
}

fn request_update_power(rt: &impl Runtime, delta: PowerPair) -> Result<(), ActorError> {
    if delta.is_zero() {
        return Ok(());
    }

    let delta_clone = delta.clone();

    extract_send_result(rt.send_simple(
        &STORAGE_POWER_ACTOR_ADDR,
        ext::power::UPDATE_CLAIMED_POWER_METHOD,
        IpldBlock::serialize_cbor(&ext::power::UpdateClaimedPowerParams {
            raw_byte_delta: delta.raw,
            quality_adjusted_delta: delta.qa,
        })?,
        TokenAmount::zero(),
    ))
    .map_err(|e| e.wrap(format!("failed to update power with {:?}", delta_clone)))?;

    Ok(())
}

fn request_terminate_deals(
    rt: &impl Runtime,
    epoch: ChainEpoch,
    sectors: &BitField,
) -> Result<(), ActorError> {
    if !sectors.is_empty() {
        // The sectors bitfield could be large, but will fit into a single parameters block.
        // The FVM max block size of 1MiB supports 130K 8-byte integers, but the policy parameter
        // ADDRESSED_SECTORS_MAX (currently 25k) will avoid reaching that.
        let res = extract_send_result(rt.send_simple(
            &STORAGE_MARKET_ACTOR_ADDR,
            ext::market::ON_MINER_SECTORS_TERMINATE_METHOD,
            IpldBlock::serialize_cbor(&ext::market::OnMinerSectorsTerminateParams {
                epoch,
                sectors: sectors.clone(),
            })?,
            TokenAmount::zero(),
        ));
        // If running in a system / cron context intentionally swallow this error to prevent
        // frozen market cron corruption from also freezing this miner cron.
        if rt.message().origin() == SYSTEM_ACTOR_ADDR {
            if let Err(e) = res {
                error!("OnSectorsTerminate event failed from cron caller {}", e)
            }
        } else {
            res?;
        }
    }
    Ok(())
}

fn schedule_early_termination_work(rt: &impl Runtime) -> Result<(), ActorError> {
    info!("scheduling early terminations with cron...");
    enroll_cron_event(
        rt,
        rt.curr_epoch() + 1,
        CronEventPayload { event_type: CRON_EVENT_PROCESS_EARLY_TERMINATIONS },
    )
}

fn have_pending_early_terminations(state: &State) -> bool {
    let no_early_terminations = state.early_terminations.is_empty();
    !no_early_terminations
}

// returns true if valid, false if invalid, error if failed to validate either way!
fn verify_windowed_post(
    rt: &impl Runtime,
    challenge_epoch: ChainEpoch,
    sectors: &[SectorOnChainInfo],
    proofs: Vec<PoStProof>,
) -> Result<bool, ActorError> {
    let miner_actor_id: u64 = if let Payload::ID(i) = rt.message().receiver().payload() {
        *i
    } else {
        return Err(actor_error!(
            illegal_state,
            "runtime provided bad receiver address {}",
            rt.message().receiver()
        ));
    };

    // Regenerate challenge randomness, which must match that generated for the proof.
    let entropy = serialize(&rt.message().receiver(), "address for window post challenge")?;
    let randomness = rt.get_randomness_from_beacon(
        DomainSeparationTag::WindowedPoStChallengeSeed,
        challenge_epoch,
        &entropy,
    )?;

    let challenged_sectors = sectors
        .iter()
        .map(|s| SectorInfo {
            proof: s.seal_proof,
            sector_number: s.sector_number,
            sealed_cid: s.sealed_cid,
        })
        .collect();

    // get public inputs
    let pv_info = WindowPoStVerifyInfo {
        randomness: Randomness(randomness.into()),
        proofs,
        challenged_sectors,
        prover: miner_actor_id,
    };

    // verify the post proof
    let result = rt.verify_post(&pv_info);
    Ok(result.is_ok())
}

struct SectorSealProofInput {
    pub registered_proof: RegisteredSealProof,
    pub sector_number: SectorNumber,
    pub randomness: SealRandomness,
    pub interactive_randomness: InteractiveSealRandomness,
    // Commr
    pub sealed_cid: Cid,
    // Commd
    pub unsealed_cid: Cid,
}

impl SectorSealProofInput {
    fn to_seal_verify_info(&self, miner_actor_id: u64, proof: &RawBytes) -> SealVerifyInfo {
        SealVerifyInfo {
            registered_proof: self.registered_proof,
            sector_id: SectorID { miner: miner_actor_id, number: self.sector_number },
            deal_ids: vec![], // unused by the proofs api so this is safe to leave empty
            randomness: self.randomness.clone(),
            interactive_randomness: self.interactive_randomness.clone(),
            proof: proof.clone().into(),
            sealed_cid: self.sealed_cid,
            unsealed_cid: self.unsealed_cid,
        }
    }

    fn to_aggregate_seal_verify_info(&self) -> AggregateSealVerifyInfo {
        AggregateSealVerifyInfo {
            sector_number: self.sector_number,
            randomness: self.randomness.clone(),
            interactive_randomness: self.interactive_randomness.clone(),
            sealed_cid: self.sealed_cid,
            unsealed_cid: self.unsealed_cid,
        }
    }
}

// Validates pre-committed sectors are ready for proving and committing this epoch.
// Returns seal proof verification inputs for every pre-commit, even those that fail validation.
// The proof verification inputs are needed as witnesses to verify an aggregated proof to allow
// other, valid, sectors to succeed.
fn validate_precommits(
    rt: &impl Runtime,
    precommits: &[SectorPreCommitOnChainInfo],
    allow_deal_ids: bool,
    all_or_nothing: bool,
) -> Result<(BatchReturn, Vec<SectorSealProofInput>), ActorError> {
    if precommits.is_empty() {
        return Ok((BatchReturn::empty(), vec![]));
    }
    let mut batch = BatchReturnGen::new(precommits.len());

    let mut verify_infos = vec![];
    for (i, precommit) in precommits.iter().enumerate() {
        // We record failures and continue validation rather than continuing the loop in order to:
        // 1. compute aggregate seal verification inputs
        // 2. check for whole message failure conditions
        let mut fail_validation = false;
        if !(allow_deal_ids || precommit.info.deal_ids.is_empty()) {
            warn!(
                "skipping commitment for sector {}, precommit has deal ids which are disallowed",
                precommit.info.sector_number,
            );
            fail_validation = true;
        }
        let msd =
            max_prove_commit_duration(rt.policy(), precommit.info.seal_proof).ok_or_else(|| {
                actor_error!(
                    illegal_state,
                    "no max seal duration for proof type: {}",
                    i64::from(precommit.info.seal_proof)
                )
            })?;
        let prove_commit_due = precommit.pre_commit_epoch + msd;
        if rt.curr_epoch() > prove_commit_due {
            warn!(
                "skipping commitment for sector {}, too late at {}, due {}",
                precommit.info.sector_number,
                rt.curr_epoch(),
                prove_commit_due,
            );
            fail_validation = true
        }

        // All seal proof types should match
        if i >= 1 {
            let prev_seal_proof = precommits[i - 1].info.seal_proof;
            if prev_seal_proof != precommit.info.seal_proof {
                return Err(actor_error!(
                    illegal_state,
                    "seal proof group for verification contains mismatched seal proofs {} and {}",
                    i64::from(prev_seal_proof),
                    i64::from(precommit.info.seal_proof)
                ));
            }
        }
        let interactive_epoch = precommit.pre_commit_epoch + rt.policy().pre_commit_challenge_delay;
        if rt.curr_epoch() <= interactive_epoch {
            return Err(actor_error!(forbidden, "too early to prove sector"));
        }

        // Compute svi for all commits even those that will not be activated.
        // Callers might prove using aggregates and need witnesses for invalid commits.
        let entropy = serialize(&rt.message().receiver(), "address for get verify info")?;
        let randomness = Randomness(
            rt.get_randomness_from_tickets(
                DomainSeparationTag::SealRandomness,
                precommit.info.seal_rand_epoch,
                &entropy,
            )?
            .into(),
        );
        let interactive_randomness = Randomness(
            rt.get_randomness_from_beacon(
                DomainSeparationTag::InteractiveSealChallengeSeed,
                interactive_epoch,
                &entropy,
            )?
            .into(),
        );

        let unsealed_cid = precommit.info.unsealed_cid.get_cid(precommit.info.seal_proof)?;
        verify_infos.push(SectorSealProofInput {
            registered_proof: precommit.info.seal_proof,
            sector_number: precommit.info.sector_number,
            randomness,
            interactive_randomness,
            sealed_cid: precommit.info.sealed_cid,
            unsealed_cid,
        });

        if fail_validation {
            if all_or_nothing {
                return Err(actor_error!(
                    illegal_argument,
                    "invalid pre-commit {} while requiring activation success: {:?}",
                    i,
                    precommit
                ));
            }
            batch.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
        } else {
            batch.add_success();
        }
    }
    Ok((batch.gen(), verify_infos))
}

fn validate_ni_sectors(
    rt: &impl Runtime,
    sectors: &[SectorNIActivationInfo],
    seal_proof_type: RegisteredSealProof,
    all_or_nothing: bool,
) -> Result<(BatchReturn, Vec<SectorSealProofInput>, BitField), ActorError> {
    let receiver = rt.message().receiver();
    let miner_id = receiver.id().unwrap();
    let curr_epoch = rt.curr_epoch();
    let activation_epoch = curr_epoch;
    let challenge_earliest = curr_epoch - rt.policy().max_prove_commit_ni_randomness_lookback;
    let unsealed_cid = CompactCommD::empty().get_cid(seal_proof_type).unwrap();
    let entropy = serialize(&receiver, "address for get verify info")?;

    if sectors.is_empty() {
        return Ok((BatchReturn::empty(), vec![], BitField::new()));
    }
    let mut batch = BatchReturnGen::new(sectors.len());

    let mut verify_infos = vec![];
    let mut sector_numbers = BitField::new();
    for (i, sector) in sectors.iter().enumerate() {
        let mut fail_validation = false;

        if sector_numbers.get(sector.sector_number) {
            return Err(actor_error!(
                illegal_argument,
                "duplicate sector number {}",
                sector.sector_number
            ));
        }

        if sector.sector_number > MAX_SECTOR_NUMBER {
            warn!("sector number {} out of range 0..(2^63-1)", sector.sector_number);
            fail_validation = true;
        }

        sector_numbers.set(sector.sector_number);

        if let Err(err) = validate_expiration(
            rt.policy(),
            curr_epoch,
            activation_epoch,
            sector.expiration,
            seal_proof_type,
        ) {
            warn!("invalid expiration: {}", err);
            fail_validation = true;
        }

        if sector.sealer_id != miner_id {
            warn!("sealer must be the same as the receiver actor for all sectors");
            fail_validation = true;
        }

        if sector.sector_number != sector.sealing_number {
            warn!("sealing number must be same as sector number for all sectors");
            fail_validation = true;
        }

        if !is_sealed_sector(&sector.sealed_cid) {
            warn!("sealed CID had wrong prefix");
            fail_validation = true;
        }

        if sector.seal_rand_epoch >= curr_epoch {
            // hard-fail because we can't access necessary randomness from the future
            return Err(actor_error!(
                illegal_argument,
                "seal challenge epoch {} must be before now {}",
                sector.seal_rand_epoch,
                curr_epoch
            ));
        }

        if sector.seal_rand_epoch < challenge_earliest {
            warn!(
                "seal challenge epoch {} too old, must be after {}",
                sector.seal_rand_epoch, challenge_earliest
            );
            fail_validation = true;
        }

        verify_infos.push(SectorSealProofInput {
            registered_proof: seal_proof_type,
            sector_number: sector.sealing_number,
            randomness: Randomness(
                rt.get_randomness_from_tickets(
                    DomainSeparationTag::SealRandomness,
                    sector.seal_rand_epoch,
                    &entropy,
                )?
                .into(),
            ),
            interactive_randomness: Randomness(vec![1u8; 32]),
            sealed_cid: sector.sealed_cid,
            unsealed_cid,
        });

        if fail_validation {
            if all_or_nothing {
                return Err(actor_error!(
                    illegal_argument,
                    "invalid NI commit {} while requiring activation success: {:?}",
                    i,
                    sector
                ));
            }
            batch.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
        } else {
            batch.add_success();
        }
    }

    Ok((batch.gen(), verify_infos, sector_numbers))
}

// Validates a batch of sector sealing proofs.
fn validate_seal_proofs(
    seal_proof_type: RegisteredSealProof,
    proofs: &[RawBytes],
) -> Result<(), ActorError> {
    let max_proof_size =
        seal_proof_type.proof_size().with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to determine max proof size for type {:?}", seal_proof_type,)
        })?;
    for proof in proofs {
        if proof.len() > max_proof_size {
            return Err(actor_error!(
                illegal_argument,
                "sector proof size {} exceeds max {}",
                proof.len(),
                max_proof_size
            ));
        }
    }
    Ok(())
}

fn validate_seal_aggregate_proof(
    proof: &RawBytes,
    sector_count: u64,
    policy: &Policy,
    interactive: bool,
) -> Result<(), ActorError> {
    let (min, max) = match interactive {
        true => (policy.min_aggregated_sectors, policy.max_aggregated_sectors),
        false => (policy.min_aggregated_sectors_ni, policy.max_aggregated_sectors_ni),
    };

    if sector_count > max {
        return Err(actor_error!(
            illegal_argument,
            "too many sectors addressed, addressed {} want <= {}",
            sector_count,
            max
        ));
    } else if sector_count < min {
        return Err(actor_error!(
            illegal_argument,
            "too few sectors addressed, addressed {} want >= {}",
            sector_count,
            min
        ));
    }
    if proof.len() > policy.max_aggregated_proof_size {
        return Err(actor_error!(
            illegal_argument,
            "sector prove-commit proof of size {} exceeds max size of {}",
            proof.len(),
            policy.max_aggregated_proof_size
        ));
    }
    Ok(())
}

fn verify_aggregate_seal(
    rt: &impl Runtime,
    proof_inputs: &[SectorSealProofInput],
    miner_actor_id: ActorID,
    seal_proof: RegisteredSealProof,
    aggregate_proof: RegisteredAggregateProof,
    proof_bytes: &RawBytes,
) -> Result<(), ActorError> {
    let seal_verify_inputs =
        proof_inputs.iter().map(|pi| pi.to_aggregate_seal_verify_info()).collect();

    rt.verify_aggregate_seals(&AggregateSealVerifyProofAndInfos {
        miner: miner_actor_id,
        seal_proof,
        aggregate_proof,
        proof: proof_bytes.clone().into(),
        infos: seal_verify_inputs,
    })
    .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "aggregate seal verify failed")
}

// Compute and burn the aggregate network fee.
fn pay_aggregate_seal_proof_fee(
    rt: &impl Runtime,
    aggregate_size: usize,
) -> Result<(), ActorError> {
    // State is loaded afresh as earlier operations for sector/data activation can change it.
    let state: State = rt.state()?;
    let aggregate_fee = aggregate_prove_commit_network_fee(aggregate_size, &rt.base_fee());
    let unlocked_balance = state
        .get_unlocked_balance(&rt.current_balance())
        .map_err(|_e| actor_error!(illegal_state, "failed to determine unlocked balance"))?;
    if unlocked_balance < aggregate_fee {
        return Err(actor_error!(
                insufficient_funds,
                "remaining unlocked funds after prove-commit {} are insufficient to pay aggregation fee of {}",
                unlocked_balance,
                aggregate_fee
            ));
    }
    burn_funds(rt, aggregate_fee)?;
    state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)
}

fn verify_deals(
    rt: &impl Runtime,
    sectors: &[ext::market::SectorDeals],
) -> Result<ext::market::VerifyDealsForActivationReturn, ActorError> {
    // Short-circuit if there are no deals in any of the sectors.
    let mut deal_count = 0;
    for sector in sectors {
        deal_count += sector.deal_ids.len();
    }
    if deal_count == 0 {
        return Ok(ext::market::VerifyDealsForActivationReturn {
            unsealed_cids: vec![None; sectors.len()],
        });
    }

    deserialize_block(extract_send_result(rt.send_simple(
        &STORAGE_MARKET_ACTOR_ADDR,
        ext::market::VERIFY_DEALS_FOR_ACTIVATION_METHOD,
        IpldBlock::serialize_cbor(&ext::market::VerifyDealsForActivationParamsRef { sectors })?,
        TokenAmount::zero(),
    ))?)
}

/// Requests the current epoch target block reward from the reward actor.
/// return value includes reward, smoothed estimate of reward, and baseline power
fn request_current_epoch_block_reward(
    rt: &impl Runtime,
) -> Result<ThisEpochRewardReturn, ActorError> {
    deserialize_block(
        extract_send_result(rt.send_simple(
            &REWARD_ACTOR_ADDR,
            ext::reward::THIS_EPOCH_REWARD_METHOD,
            Default::default(),
            TokenAmount::zero(),
        ))
        .map_err(|e| e.wrap("failed to check epoch baseline power"))?,
    )
}

/// Requests the current network total power and pledge from the power actor.
fn request_current_total_power(
    rt: &impl Runtime,
) -> Result<ext::power::CurrentTotalPowerReturn, ActorError> {
    deserialize_block(
        extract_send_result(rt.send_simple(
            &STORAGE_POWER_ACTOR_ADDR,
            ext::power::CURRENT_TOTAL_POWER_METHOD,
            Default::default(),
            TokenAmount::zero(),
        ))
        .map_err(|e| e.wrap("failed to check current power"))?,
    )
}

/// Resolves an address to an ID address and verifies that it is address of an account actor with an associated BLS key.
/// The worker must be BLS since the worker key will be used alongside a BLS-VRF.
fn resolve_worker_address(rt: &impl Runtime, raw: Address) -> Result<ActorID, ActorError> {
    let resolved = rt
        .resolve_address(&raw)
        .ok_or_else(|| actor_error!(illegal_argument, "unable to resolve address: {}", raw))?;

    let worker_code = rt
        .get_actor_code_cid(&resolved)
        .ok_or_else(|| actor_error!(illegal_argument, "no code for address: {}", resolved))?;
    if rt.resolve_builtin_actor_type(&worker_code) != Some(Type::Account) {
        return Err(actor_error!(
            illegal_argument,
            "worker actor type must be an account, was {}",
            worker_code
        ));
    }

    if raw.protocol() != Protocol::BLS {
        let pub_key: Address = deserialize_block(extract_send_result(rt.send_simple(
            &Address::new_id(resolved),
            ext::account::PUBKEY_ADDRESS_METHOD,
            None,
            TokenAmount::zero(),
        ))?)?;
        if pub_key.protocol() != Protocol::BLS {
            return Err(actor_error!(
                illegal_argument,
                "worker account {} must have BLS pubkey, was {}",
                resolved,
                pub_key.protocol()
            ));
        }
    }
    Ok(resolved)
}

fn burn_funds(rt: &impl Runtime, amount: TokenAmount) -> Result<(), ActorError> {
    log::debug!("storage provder {} burning {}", rt.message().receiver(), amount);
    if amount.is_positive() {
        extract_send_result(rt.send_simple(&BURNT_FUNDS_ACTOR_ADDR, METHOD_SEND, None, amount))?;
    }
    Ok(())
}

fn notify_pledge_changed(rt: &impl Runtime, pledge_delta: &TokenAmount) -> Result<(), ActorError> {
    if !pledge_delta.is_zero() {
        extract_send_result(rt.send_simple(
            &STORAGE_POWER_ACTOR_ADDR,
            ext::power::UPDATE_PLEDGE_TOTAL_METHOD,
            IpldBlock::serialize_cbor(pledge_delta)?,
            TokenAmount::zero(),
        ))?;
    }
    Ok(())
}

fn get_claims(
    rt: &impl Runtime,
    ids: &[ext::verifreg::ClaimID],
) -> Result<Vec<ext::verifreg::Claim>, ActorError> {
    let params = ext::verifreg::GetClaimsParams {
        provider: rt.message().receiver().id().unwrap(),
        claim_ids: ids.to_owned(),
    };
    let claims_ret: ext::verifreg::GetClaimsReturn =
        deserialize_block(extract_send_result(rt.send_simple(
            &VERIFIED_REGISTRY_ACTOR_ADDR,
            ext::verifreg::GET_CLAIMS_METHOD,
            IpldBlock::serialize_cbor(&params)?,
            TokenAmount::zero(),
        ))?)?;
    if (claims_ret.batch_info.success_count as usize) < ids.len() {
        return Err(actor_error!(illegal_argument, "invalid claims"));
    }
    Ok(claims_ret.claims)
}

/// Assigns proving period offset randomly in the range [0, WPoStProvingPeriod) by hashing
/// the actor's address and current epoch.
fn assign_proving_period_offset(
    policy: &Policy,
    addr: Address,
    current_epoch: ChainEpoch,
    blake2b: impl FnOnce(&[u8]) -> [u8; 32],
) -> anyhow::Result<ChainEpoch> {
    let mut my_addr = serialize_vec(&addr, "address")?;
    my_addr.write_i64::<BigEndian>(current_epoch)?;

    let digest = blake2b(&my_addr);

    let mut offset: u64 = BigEndian::read_u64(&digest);
    offset %= policy.wpost_proving_period as u64;

    // Conversion from i64 to u64 is safe because it's % WPOST_PROVING_PERIOD which is i64
    Ok(offset as ChainEpoch)
}

/// Computes the epoch at which a proving period should start such that it is greater than the current epoch, and
/// has a defined offset from being an exact multiple of WPoStProvingPeriod.
/// A miner is exempt from Winow PoSt until the first full proving period starts.
fn current_proving_period_start(
    policy: &Policy,
    current_epoch: ChainEpoch,
    offset: ChainEpoch,
) -> ChainEpoch {
    let curr_modulus = current_epoch % policy.wpost_proving_period;

    let period_progress = if curr_modulus >= offset {
        curr_modulus - offset
    } else {
        policy.wpost_proving_period - (offset - curr_modulus)
    };

    current_epoch - period_progress
}

fn current_deadline_index(
    policy: &Policy,
    current_epoch: ChainEpoch,
    period_start: ChainEpoch,
) -> u64 {
    ((current_epoch - period_start) / policy.wpost_challenge_window) as u64
}

/// Computes deadline information for a fault or recovery declaration.
/// If the deadline has not yet elapsed, the declaration is taken as being for the current proving period.
/// If the deadline has elapsed, it's instead taken as being for the next proving period after the current epoch.
fn declaration_deadline_info(
    policy: &Policy,
    period_start: ChainEpoch,
    deadline_idx: u64,
    current_epoch: ChainEpoch,
) -> anyhow::Result<DeadlineInfo> {
    if deadline_idx >= policy.wpost_period_deadlines {
        return Err(anyhow!(
            "invalid deadline {}, must be < {}",
            deadline_idx,
            policy.wpost_period_deadlines
        ));
    }

    let deadline =
        new_deadline_info(policy, period_start, deadline_idx, current_epoch).next_not_elapsed();
    Ok(deadline)
}

/// Checks that a fault or recovery declaration at a specific deadline is outside the exclusion window for the deadline.
fn validate_fr_declaration_deadline(deadline: &DeadlineInfo) -> anyhow::Result<()> {
    if deadline.fault_cutoff_passed() {
        Err(anyhow!("late fault or recovery declaration"))
    } else {
        Ok(())
    }
}

/// Validates that a partition contains the given sectors.
fn validate_partition_contains_sectors(
    partition: &Partition,
    sectors: &BitField,
) -> anyhow::Result<()> {
    // Check that the declared sectors are actually assigned to the partition.
    if partition.sectors.contains_all(sectors) {
        Ok(())
    } else {
        Err(anyhow!("not all sectors are assigned to the partition"))
    }
}

fn consensus_fault_active(info: &MinerInfo, curr_epoch: ChainEpoch) -> bool {
    // For penalization period to last for exactly finality epochs
    // consensus faults are active until currEpoch exceeds ConsensusFaultElapsed
    curr_epoch <= info.consensus_fault_elapsed
}

pub fn power_for_sector(sector_size: SectorSize, sector: &SectorOnChainInfo) -> PowerPair {
    PowerPair {
        raw: BigInt::from(sector_size as u64),
        qa: qa_power_for_sector(sector_size, sector),
    }
}

/// Returns the sum of the raw byte and quality-adjusted power for sectors.
pub fn power_for_sectors(sector_size: SectorSize, sectors: &[SectorOnChainInfo]) -> PowerPair {
    let qa = sectors.iter().map(|s| qa_power_for_sector(sector_size, s)).sum();

    PowerPair { raw: BigInt::from(sector_size as u64) * BigInt::from(sectors.len()), qa }
}

fn get_miner_info<BS>(store: &BS, state: &State) -> Result<MinerInfo, ActorError>
where
    BS: Blockstore,
{
    state
        .get_info(store)
        .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "could not read miner info"))
}

fn process_pending_worker(
    info: &mut MinerInfo,
    rt: &impl Runtime,
    state: &mut State,
) -> Result<(), ActorError> {
    let pending_worker_key = if let Some(k) = &info.pending_worker_key {
        k
    } else {
        return Ok(());
    };

    if rt.curr_epoch() < pending_worker_key.effective_at {
        return Ok(());
    }

    info.worker = pending_worker_key.new_worker;
    info.pending_worker_key = None;

    state
        .save_info(rt.store(), info)
        .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save miner info"))
}

/// Repays all fee debt and then verifies that the miner has amount needed to cover
/// the pledge requirement after burning all fee debt.  If not aborts.
/// Returns an amount that must be burnt by the actor.
/// Note that this call does not compute recent vesting so reported unlocked balance
/// may be slightly lower than the true amount. Computing vesting here would be
/// almost always redundant since vesting is quantized to ~daily units.  Vesting
/// will be at most one proving period old if computed in the cron callback.
fn repay_debts_or_abort(rt: &impl Runtime, state: &mut State) -> Result<TokenAmount, ActorError> {
    let res = state.repay_debts(&rt.current_balance()).map_err(|e| {
        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "unlocked balance can not repay fee debt")
    })?;
    info!("RepayDebtsOrAbort was called and succeeded");
    Ok(res)
}

fn check_control_addresses(policy: &Policy, control_addrs: &[Address]) -> Result<(), ActorError> {
    if control_addrs.len() > policy.max_control_addresses {
        return Err(actor_error!(
            illegal_argument,
            "control addresses length {} exceeds max control addresses length {}",
            control_addrs.len(),
            policy.max_control_addresses
        ));
    }

    Ok(())
}

fn check_valid_post_proof_type(
    policy: &Policy,
    proof_type: RegisteredPoStProof,
) -> Result<(), ActorError> {
    if policy.valid_post_proof_type.contains(proof_type) {
        Ok(())
    } else {
        Err(actor_error!(
            illegal_argument,
            "proof type {:?} not allowed for new miner actors",
            proof_type
        ))
    }
}

fn check_peer_info(
    policy: &Policy,
    peer_id: &[u8],
    multiaddrs: &[BytesDe],
) -> Result<(), ActorError> {
    if peer_id.len() > policy.max_peer_id_length {
        return Err(actor_error!(
            illegal_argument,
            "peer ID size of {} exceeds maximum size of {}",
            peer_id.len(),
            policy.max_peer_id_length
        ));
    }

    let mut total_size = 0;
    for ma in multiaddrs {
        if ma.0.is_empty() {
            return Err(actor_error!(illegal_argument, "invalid empty multiaddr"));
        }
        total_size += ma.0.len();
    }

    if total_size > policy.max_multiaddr_data {
        return Err(actor_error!(
            illegal_argument,
            "multiaddr size of {} exceeds maximum of {}",
            total_size,
            policy.max_multiaddr_data
        ));
    }

    Ok(())
}

fn activate_new_sector_infos(
    rt: &impl Runtime,
    precommits: Vec<&SectorPreCommitOnChainInfo>,
    data_activations: Vec<DataActivationOutput>,
    pledge_inputs: &NetworkPledgeInputs,
    info: &MinerInfo,
) -> Result<(), ActorError> {
    let activation_epoch = rt.curr_epoch();

    let (total_pledge, newly_vested) = rt.transaction(|state: &mut State, rt| {
        let policy = rt.policy();
        let store = rt.store();

        let mut new_sector_numbers = Vec::<SectorNumber>::with_capacity(data_activations.len());
        let mut deposit_to_unlock = TokenAmount::zero();
        let mut new_sectors = Vec::<SectorOnChainInfo>::new();
        let mut total_pledge = TokenAmount::zero();

        for (pci, deal_spaces) in precommits.iter().zip(data_activations) {
            // compute initial pledge
            let duration = pci.info.expiration - activation_epoch;
            // This is probably always caught in precommit but fail cleanly if it occurs
            if duration < policy.min_sector_expiration {
                return Err(actor_error!(
                    illegal_argument,
                    "precommit {} has lifetime {} less than minimum {}. ignoring",
                    pci.info.sector_number,
                    duration,
                    policy.min_sector_expiration
                ));
            }

            let deal_weight = &deal_spaces.unverified_space * duration;
            let verified_deal_weight = &deal_spaces.verified_space * duration;

            let power = qa_power_for_weight(info.sector_size, duration, &verified_deal_weight);

            let day_reward = expected_reward_for_power(
                &pledge_inputs.epoch_reward,
                &pledge_inputs.network_qap,
                &power,
                fil_actors_runtime::EPOCHS_IN_DAY,
            );

            // The storage pledge is recorded for use in computing the penalty if this sector is terminated
            // before its declared expiration.
            // It's not capped to 1 FIL, so can exceed the actual initial pledge requirement.
            let storage_pledge = expected_reward_for_power(
                &pledge_inputs.epoch_reward,
                &pledge_inputs.network_qap,
                &power,
                INITIAL_PLEDGE_PROJECTION_PERIOD,
            );

            let initial_pledge = initial_pledge_for_power(
                &power,
                &pledge_inputs.network_baseline,
                &pledge_inputs.epoch_reward,
                &pledge_inputs.network_qap,
                &pledge_inputs.circulating_supply,
                pledge_inputs.epochs_since_ramp_start,
                pledge_inputs.ramp_duration_epochs,
            );

            deposit_to_unlock += pci.pre_commit_deposit.clone();
            total_pledge += &initial_pledge;

            let new_sector_info = SectorOnChainInfo {
                sector_number: pci.info.sector_number,
                seal_proof: pci.info.seal_proof,
                sealed_cid: pci.info.sealed_cid,
                deprecated_deal_ids: vec![], // deal ids field deprecated
                expiration: pci.info.expiration,
                activation: activation_epoch,
                deal_weight,
                verified_deal_weight,
                initial_pledge,
                expected_day_reward: day_reward,
                expected_storage_pledge: storage_pledge,
                power_base_epoch: activation_epoch,
                replaced_day_reward: TokenAmount::zero(),
                sector_key_cid: None,
                flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
            };

            new_sector_numbers.push(new_sector_info.sector_number);
            new_sectors.push(new_sector_info);
        }

        state.put_sectors(store, new_sectors.clone()).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to put new sectors")
        })?;
        state.delete_precommitted_sectors(store, &new_sector_numbers)?;
        state
            .assign_sectors_to_deadlines(
                policy,
                store,
                rt.curr_epoch(),
                new_sectors,
                info.window_post_partition_sectors,
                info.sector_size,
            )
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to assign new sectors to deadlines",
                )
            })?;
        let newly_vested = TokenAmount::zero();

        // Unlock deposit for successful proofs, make it available for lock-up as initial pledge.
        state
            .add_pre_commit_deposit(&(-deposit_to_unlock))
            .map_err(|e| actor_error!(illegal_state, "failed to add precommit deposit: {}", e))?;

        let unlocked_balance = state.get_unlocked_balance(&rt.current_balance()).map_err(|e| {
            actor_error!(illegal_state, "failed to calculate unlocked balance: {}", e)
        })?;
        if unlocked_balance < total_pledge {
            return Err(actor_error!(
                insufficient_funds,
                "insufficient funds for aggregate initial pledge requirement {}, available: {}",
                total_pledge,
                unlocked_balance
            ));
        }

        state
            .add_initial_pledge(&total_pledge)
            .map_err(|e| actor_error!(illegal_state, "failed to add initial pledge: {}", e))?;

        state.check_balance_invariants(&rt.current_balance()).map_err(balance_invariants_broken)?;

        Ok((total_pledge, newly_vested))
    })?;
    // Request pledge update for activated sectors.
    // Power is not activated until first Window poST.
    notify_pledge_changed(rt, &(total_pledge - newly_vested))?;

    Ok(())
}

pub struct SectorPiecesActivationInput {
    pub piece_manifests: Vec<PieceActivationManifest>,
    pub sector_expiry: ChainEpoch,
    pub sector_number: SectorNumber,
    pub sector_type: RegisteredSealProof,
    pub expected_commd: Option<CompactCommD>,
}

// Inputs for activating builtin market deals for one sector
#[derive(Debug, Clone)]
pub struct DealsActivationInput {
    pub deal_ids: Vec<DealID>,
    pub sector_expiry: ChainEpoch,
    pub sector_number: SectorNumber,
    pub sector_type: RegisteredSealProof,
}

impl From<SectorPreCommitOnChainInfo> for DealsActivationInput {
    fn from(pci: SectorPreCommitOnChainInfo) -> DealsActivationInput {
        DealsActivationInput {
            deal_ids: pci.info.deal_ids,
            sector_expiry: pci.info.expiration,
            sector_number: pci.info.sector_number,
            sector_type: pci.info.seal_proof,
        }
    }
}

impl From<&UpdateAndSectorInfo<'_>> for DealsActivationInput {
    fn from(usi: &UpdateAndSectorInfo) -> DealsActivationInput {
        DealsActivationInput {
            sector_number: usi.sector_info.sector_number,
            sector_expiry: usi.sector_info.expiration,
            deal_ids: usi.update.deals.clone(),
            sector_type: usi.sector_info.seal_proof,
        }
    }
}

// Data activation results for one sector
#[derive(Clone)]
struct DataActivationOutput {
    pub unverified_space: BigInt,
    pub verified_space: BigInt,
    // None indicates either no deals or computation was not requested.
    pub unsealed_cid: Option<Cid>,
    pub pieces: Vec<(Cid, u64)>,
}

// Track information needed to update a sector info's data during ProveReplicaUpdate
#[derive(Clone, Debug)]
struct UpdateAndSectorInfo<'a> {
    update: &'a ReplicaUpdateInner,
    sector_info: &'a SectorOnChainInfo,
}

// Inputs to state update for a single sector replica update.
struct ReplicaUpdateStateInputs<'a> {
    deadline: u64,
    partition: u64,
    sector_info: &'a SectorOnChainInfo,
    activated_data: ReplicaUpdateActivatedData,
}

// Summary of activated data for a replica update.
struct ReplicaUpdateActivatedData {
    seal_cid: Cid,
    unverified_space: BigInt,
    verified_space: BigInt,
}

// Activates data pieces by claiming allocations with the verified registry.
// Pieces are grouped by sector and succeed or fail in sector groups.
// If an activation input specifies an expected CommD for the sector, a CommD
// is calculated from the pieces and must match.
// This method never returns CommDs in the output type; either the caller provided
// them and they are correct, or the caller did not provide anything that needs checking.
fn activate_sectors_pieces(
    rt: &impl Runtime,
    activation_inputs: Vec<SectorPiecesActivationInput>,
    all_or_nothing: bool,
) -> Result<(BatchReturn, Vec<DataActivationOutput>), ActorError> {
    // Get a flattened list of verified claims for all activated sectors
    let mut verified_claims = Vec::new();
    let mut sectors_pieces = Vec::new();

    for activation_info in &activation_inputs {
        // Check a declared CommD matches that computed from the data.
        if let Some(declared_commd) = &activation_info.expected_commd {
            let computed_commd = unsealed_cid_from_pieces(
                rt,
                &activation_info.piece_manifests,
                activation_info.sector_type,
            )?
            .get_cid(activation_info.sector_type)?;
            // A declared zero CommD might be compact or fully computed,
            // so normalize to the computed value before checking.
            if !declared_commd.get_cid(activation_info.sector_type)?.eq(&computed_commd) {
                return Err(actor_error!(
                    illegal_argument,
                    "unsealed CID does not match pieces for sector {}, computed {:?} declared {:?}",
                    activation_info.sector_number,
                    computed_commd,
                    declared_commd
                ));
            }
        }

        let mut sector_claims = vec![];
        sectors_pieces.push(&activation_info.piece_manifests);

        for piece in &activation_info.piece_manifests {
            if let Some(alloc_key) = &piece.verified_allocation_key {
                sector_claims.push(ext::verifreg::AllocationClaim {
                    client: alloc_key.client,
                    allocation_id: alloc_key.id,
                    data: piece.cid,
                    size: piece.size,
                });
            }
        }
        verified_claims.push(ext::verifreg::SectorAllocationClaims {
            sector: activation_info.sector_number,
            expiry: activation_info.sector_expiry,
            claims: sector_claims,
        });
    }
    let claim_res = batch_claim_allocations(rt, verified_claims, all_or_nothing)?;
    if all_or_nothing {
        assert!(
            claim_res.sector_results.all_ok() || claim_res.sector_results.success_count == 0,
            "batch return of claim allocations partially succeeded but request was all_or_nothing {:?}",
            claim_res
        );
    }

    let activation_outputs = claim_res
        .sector_claims
        .iter()
        .zip(claim_res.sector_results.successes(&sectors_pieces))
        .map(|(sector_claim, sector_pieces)| {
            let mut unverified_space = BigInt::zero();
            let mut pieces = Vec::new();
            for piece in *sector_pieces {
                if piece.verified_allocation_key.is_none() {
                    unverified_space += piece.size.0;
                }
                pieces.push((piece.cid, piece.size.0));
            }
            DataActivationOutput {
                unverified_space: unverified_space.clone(),
                verified_space: sector_claim.claimed_space.clone(),
                unsealed_cid: None,
                pieces,
            }
        })
        .collect();

    Ok((claim_res.sector_results, activation_outputs))
}

/// Activates deals then claims allocations for any verified deals
/// Deals and claims are grouped by sectors
/// Successfully activated sectors have their DealSpaces returned
/// Failure to claim datacap for any verified deal results in the whole batch failing
fn activate_sectors_deals(
    rt: &impl Runtime,
    activation_infos: &[DealsActivationInput],
    compute_unsealed_cid: bool,
) -> Result<(BatchReturn, Vec<DataActivationOutput>), ActorError> {
    let batch_activation_res = match activation_infos.iter().all(|p| p.deal_ids.is_empty()) {
        true => ext::market::BatchActivateDealsResult {
            // if all sectors are empty of deals, skip calling the market actor
            activations: vec![
                ext::market::SectorDealActivation {
                    activated: Vec::default(),
                    unsealed_cid: None,
                };
                activation_infos.len()
            ],
            activation_results: BatchReturn::ok(activation_infos.len() as u32),
        },
        false => {
            let sector_activation_params = activation_infos
                .iter()
                .map(|activation_info| ext::market::SectorDeals {
                    sector_number: activation_info.sector_number,
                    deal_ids: activation_info.deal_ids.clone(),
                    sector_expiry: activation_info.sector_expiry,
                    sector_type: activation_info.sector_type,
                })
                .collect();
            let activate_raw = extract_send_result(rt.send_simple(
                &STORAGE_MARKET_ACTOR_ADDR,
                ext::market::BATCH_ACTIVATE_DEALS_METHOD,
                IpldBlock::serialize_cbor(&ext::market::BatchActivateDealsParams {
                    sectors: sector_activation_params,
                    compute_cid: compute_unsealed_cid,
                })?,
                TokenAmount::zero(),
            ))?;
            deserialize_block::<ext::market::BatchActivateDealsResult>(activate_raw)?
        }
    };

    // When all prove commits have failed abort early
    if batch_activation_res.activation_results.success_count == 0 {
        return Err(actor_error!(illegal_argument, "all deals failed to activate"));
    }

    // Filter the DealsActivationInfo for successfully activated sectors
    let successful_activation_infos =
        batch_activation_res.activation_results.successes(activation_infos);

    // Get a flattened list of verified claims for all activated sectors
    let mut verified_claims = Vec::new();
    for (activation_info, activate_res) in
        successful_activation_infos.iter().zip(&batch_activation_res.activations)
    {
        let sector_claims = activate_res
            .activated
            .iter()
            .filter(|info| info.allocation_id != NO_ALLOCATION_ID)
            .map(|info| ext::verifreg::AllocationClaim {
                client: info.client,
                allocation_id: info.allocation_id,
                data: info.data,
                size: info.size,
            })
            .collect();

        verified_claims.push(ext::verifreg::SectorAllocationClaims {
            sector: activation_info.sector_number,
            expiry: activation_info.sector_expiry,
            claims: sector_claims,
        });
    }

    let all_or_nothing = true;
    let claim_res = batch_claim_allocations(rt, verified_claims, all_or_nothing)?;
    assert!(
        claim_res.sector_results.all_ok() || claim_res.sector_results.success_count == 0,
        "batch return of claim allocations partially succeeded but request was all_or_nothing {:?}",
        claim_res
    );

    // reassociate the verified claims with corresponding DealActivation information
    let activation_and_claim_results = batch_activation_res
        .activations
        .iter()
        .zip(claim_res.sector_claims)
        .map(|(sector_deals, sector_claim)| {
            let mut sector_pieces = Vec::new();
            let mut unverified_deal_space = BigInt::zero();
            for info in &sector_deals.activated {
                sector_pieces.push((info.data, info.size.0));
                if info.allocation_id == NO_ALLOCATION_ID {
                    unverified_deal_space += info.size.0;
                }
            }
            DataActivationOutput {
                unverified_space: unverified_deal_space,
                verified_space: sector_claim.claimed_space,
                unsealed_cid: sector_deals.unsealed_cid,
                pieces: sector_pieces,
            }
        })
        .collect();

    // Return the deal spaces for activated sectors only
    Ok((batch_activation_res.activation_results, activation_and_claim_results))
}

fn batch_claim_allocations(
    rt: &impl Runtime,
    verified_claims: Vec<ext::verifreg::SectorAllocationClaims>,
    all_or_nothing: bool,
) -> Result<ext::verifreg::ClaimAllocationsReturn, ActorError> {
    let claim_res = match verified_claims.iter().all(|sector| sector.claims.is_empty()) {
        // Short-circuit the call if there are no claims,
        // but otherwise send a group for each sector (even if empty) to ease association of results.
        true => ext::verifreg::ClaimAllocationsReturn {
            sector_results: BatchReturn::ok(verified_claims.len() as u32),
            sector_claims: vec![
                ext::verifreg::SectorClaimSummary { claimed_space: BigInt::zero() };
                verified_claims.len()
            ],
        },
        false => {
            let claim_raw = extract_send_result(rt.send_simple(
                &VERIFIED_REGISTRY_ACTOR_ADDR,
                ext::verifreg::CLAIM_ALLOCATIONS_METHOD,
                IpldBlock::serialize_cbor(&ext::verifreg::ClaimAllocationsParams {
                    sectors: verified_claims,
                    all_or_nothing,
                })?,
                TokenAmount::zero(),
            ))
            .context("error claiming allocations on batch")?;

            let claim_res: ext::verifreg::ClaimAllocationsReturn = deserialize_block(claim_raw)?;
            claim_res
        }
    };
    Ok(claim_res)
}

fn unsealed_cid_from_pieces(
    rt: &impl Runtime,
    pieces: &[PieceActivationManifest],
    sector_type: RegisteredSealProof,
) -> Result<CompactCommD, ActorError> {
    let computed_commd = if !pieces.is_empty() {
        let pieces: Vec<PieceInfo> =
            pieces.iter().map(|piece| PieceInfo { cid: piece.cid, size: piece.size }).collect();
        let computed = rt.compute_unsealed_sector_cid(sector_type, &pieces).context_code(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "failed to compute unsealed sector CID",
        )?;
        CompactCommD::of(computed)
    } else {
        CompactCommD::empty()
    };
    Ok(computed_commd)
}

// Network inputs to calculation of sector pledge and associated parameters.
struct NetworkPledgeInputs {
    pub network_qap: FilterEstimate,
    pub network_baseline: StoragePower,
    pub circulating_supply: TokenAmount,
    pub epoch_reward: FilterEstimate,
    pub epochs_since_ramp_start: i64,
    pub ramp_duration_epochs: u64,
}

// Note: probably better to push this one level down into state
fn balance_invariants_broken(e: Error) -> ActorError {
    ActorError::unchecked(
        ERR_BALANCE_INVARIANTS_BROKEN,
        format!("balance invariants broken: {}", e),
    )
}

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "StorageMiner"
    }

    actor_dispatch! {
        Constructor => constructor,
        ControlAddresses => control_addresses,
        ChangeWorkerAddress|ChangeWorkerAddressExported => change_worker_address,
        ChangePeerID|ChangePeerIDExported => change_peer_id,
        SubmitWindowedPoSt => submit_windowed_post,
        ExtendSectorExpiration => extend_sector_expiration,
        TerminateSectors => terminate_sectors,
        DeclareFaults => declare_faults,
        DeclareFaultsRecovered => declare_faults_recovered,
        OnDeferredCronEvent => on_deferred_cron_event,
        CheckSectorProven => check_sector_proven,
        ApplyRewards => apply_rewards,
        ReportConsensusFault => report_consensus_fault,
        WithdrawBalance|WithdrawBalanceExported => withdraw_balance,
        InternalSectorSetupForPreseal => internal_sector_setup_preseal,
        ChangeMultiaddrs|ChangeMultiaddrsExported => change_multiaddresses,
        CompactPartitions => compact_partitions,
        CompactSectorNumbers => compact_sector_numbers,
        ConfirmChangeWorkerAddress|ConfirmChangeWorkerAddressExported => confirm_change_worker_address,
        RepayDebt|RepayDebtExported => repay_debt,
        ChangeOwnerAddress|ChangeOwnerAddressExported => change_owner_address,
        DisputeWindowedPoSt => dispute_windowed_post,
        ProveCommitAggregate => prove_commit_aggregate,
        ProveReplicaUpdates => prove_replica_updates,
        PreCommitSectorBatch2 => pre_commit_sector_batch2,
        ChangeBeneficiary|ChangeBeneficiaryExported => change_beneficiary,
        GetBeneficiary|GetBeneficiaryExported => get_beneficiary,
        ExtendSectorExpiration2 => extend_sector_expiration2,
        GetOwnerExported => get_owner,
        IsControllingAddressExported => is_controlling_address,
        GetSectorSizeExported => get_sector_size,
        GetAvailableBalanceExported => get_available_balance,
        GetVestingFundsExported => get_vesting_funds,
        GetPeerIDExported => get_peer_id,
        GetMultiaddrsExported => get_multiaddresses,
        ProveCommitSectors3 => prove_commit_sectors3,
        ProveReplicaUpdates3 => prove_replica_updates3,
        ProveCommitSectorsNI => prove_commit_sectors_ni,
    }
}

#[cfg(test)]
mod internal_tests;

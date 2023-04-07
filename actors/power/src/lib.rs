// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::BTreeSet;
use std::convert::TryInto;

use anyhow::anyhow;
use ext::init;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, deserialize_block, extract_send_result,
    make_map_with_root_and_bitwidth, ActorDowncast, ActorError, Multimap, CRON_ACTOR_ADDR,
    INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::reward::ThisEpochRewardReturn;
use fvm_shared::sector::SealVerifyInfo;
use fvm_shared::{MethodNum, HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR};
use log::{debug, error};
use num_derive::FromPrimitive;
use num_traits::Zero;

pub use self::policy::*;
pub use self::state::*;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

#[doc(hidden)]
pub mod ext;
mod policy;
mod state;
pub mod testing;
mod types;

// * Updated to specs-actors commit: 999e57a151cc7ada020ca2844b651499ab8c0dec (v3.0.1)

/// GasOnSubmitVerifySeal is amount of gas charged for SubmitPoRepForBulkVerify
/// This number is empirically determined
pub mod detail {
    pub const GAS_ON_SUBMIT_VERIFY_SEAL: i64 = 34721049;
}

/// Storage power actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    /// Constructor for Storage Power Actor
    Constructor = METHOD_CONSTRUCTOR,
    CreateMiner = 2,
    UpdateClaimedPower = 3,
    EnrollCronEvent = 4,
    OnEpochTickEnd = 5,
    UpdatePledgeTotal = 6,
    // * Deprecated in v2
    // OnConsensusFault = 7,
    SubmitPoRepForBulkVerify = 8,
    CurrentTotalPower = 9,
    // Method numbers derived from FRC-0042 standards
    CreateMinerExported = frc42_dispatch::method_hash!("CreateMiner"),
    NetworkRawPowerExported = frc42_dispatch::method_hash!("NetworkRawPower"),
    MinerRawPowerExported = frc42_dispatch::method_hash!("MinerRawPower"),
    MinerCountExported = frc42_dispatch::method_hash!("MinerCount"),
    MinerConsensusCountExported = frc42_dispatch::method_hash!("MinerConsensusCount"),
}

pub const ERR_TOO_MANY_PROVE_COMMITS: ExitCode = ExitCode::new(32);

/// Storage Power Actor
pub struct Actor;

impl Actor {
    /// Constructor for StoragePower actor
    fn constructor(rt: &impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        let st = State::new(rt.store()).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "Failed to create power actor state")
        })?;
        rt.create(&st)?;
        Ok(())
    }

    fn create_miner(
        rt: &impl Runtime,
        params: CreateMinerParams,
    ) -> Result<CreateMinerReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let value = rt.message().value_received();

        let constructor_params = RawBytes::serialize(ext::miner::MinerConstructorParams {
            owner: params.owner,
            worker: params.worker,
            window_post_proof_type: params.window_post_proof_type,
            peer_id: params.peer,
            multi_addresses: params.multiaddrs,
            control_addresses: Default::default(),
        })?;

        let miner_actor_code_cid = rt.get_code_cid_for_type(Type::Miner);
        let ext::init::ExecReturn { id_address, robust_address } =
            deserialize_block(extract_send_result(rt.send_simple(
                &INIT_ACTOR_ADDR,
                ext::init::EXEC_METHOD,
                IpldBlock::serialize_cbor(&init::ExecParams {
                    code_cid: miner_actor_code_cid,
                    constructor_params,
                })?,
                value,
            ))?)?;

        let window_post_proof_type = params.window_post_proof_type;
        rt.transaction(|st: &mut State, rt| {
            let mut claims =
                make_map_with_root_and_bitwidth(&st.claims, rt.store(), HAMT_BIT_WIDTH).map_err(
                    |e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load claims"),
                )?;
            set_claim(
                &mut claims,
                &id_address,
                Claim {
                    window_post_proof_type,
                    quality_adj_power: Default::default(),
                    raw_byte_power: Default::default(),
                },
            )
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to put power in claimed table while creating miner",
                )
            })?;
            st.miner_count += 1;

            st.update_stats_for_new_miner(rt.policy(), window_post_proof_type).map_err(|e| {
                actor_error!(
                    illegal_state,
                    "failed to update power stats for new miner {}: {}",
                    &id_address,
                    e
                )
            })?;

            st.claims = claims.flush().map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush claims")
            })?;
            Ok(())
        })?;
        Ok(CreateMinerReturn { id_address, robust_address })
    }

    /// Adds or removes claimed power for the calling actor.
    /// May only be invoked by a miner actor.
    fn update_claimed_power(
        rt: &impl Runtime,
        params: UpdateClaimedPowerParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();

        rt.transaction(|st: &mut State, rt| {
            let mut claims =
                make_map_with_root_and_bitwidth(&st.claims, rt.store(), HAMT_BIT_WIDTH).map_err(
                    |e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load claims"),
                )?;

            st.add_to_claim(
                rt.policy(),
                &mut claims,
                &miner_addr,
                &params.raw_byte_delta,
                &params.quality_adjusted_delta,
            )
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!(
                        "failed to update power raw {}, qa {}",
                        params.raw_byte_delta, params.quality_adjusted_delta,
                    ),
                )
            })?;

            st.claims = claims.flush().map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush claims")
            })?;
            Ok(())
        })
    }

    fn enroll_cron_event(
        rt: &impl Runtime,
        params: EnrollCronEventParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_event = CronEvent {
            miner_addr: rt.message().caller(),
            callback_payload: params.payload.clone(),
        };

        // Ensure it is not possible to enter a large negative number which would cause
        // problems in cron processing.
        if params.event_epoch < 0 {
            return Err(actor_error!(illegal_argument;
                "cron event epoch {} cannot be less than zero", params.event_epoch));
        }

        rt.transaction(|st: &mut State, rt| {
            let mut events = Multimap::from_root(
                rt.store(),
                &st.cron_event_queue,
                CRON_QUEUE_HAMT_BITWIDTH,
                CRON_QUEUE_AMT_BITWIDTH,
            )
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load cron events")
            })?;

            st.append_cron_event(&mut events, params.event_epoch, miner_event).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to enroll cron event")
            })?;

            st.cron_event_queue = events.root().map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush cron events")
            })?;
            Ok(())
        })?;
        Ok(())
    }

    fn on_epoch_tick_end(rt: &impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&CRON_ACTOR_ADDR))?;

        let rewret: ThisEpochRewardReturn = deserialize_block(
            extract_send_result(rt.send_simple(
                &REWARD_ACTOR_ADDR,
                ext::reward::Method::ThisEpochReward as MethodNum,
                None,
                TokenAmount::zero(),
            ))
            .map_err(|e| e.wrap("failed to check epoch baseline power"))?,
        )?;

        if let Err(e) = Self::process_batch_proof_verifies(rt, &rewret) {
            error!("unexpected error processing batch proof verifies: {}. Skipping all verification for epoch {}", e, rt.curr_epoch());
        }
        Self::process_deferred_cron_events(rt, rewret)?;

        let this_epoch_raw_byte_power = rt.transaction(|st: &mut State, _| {
            let (raw_byte_power, qa_power) = st.current_total_power();
            st.this_epoch_pledge_collateral = st.total_pledge_collateral.clone();
            st.this_epoch_quality_adj_power = qa_power;
            st.this_epoch_raw_byte_power = raw_byte_power;
            // Can assume delta is one since cron is invoked every epoch.
            st.update_smoothed_estimate(1);

            Ok(IpldBlock::serialize_cbor(&BigIntSer(&st.this_epoch_raw_byte_power))?)
        })?;

        // Update network KPA in reward actor
        extract_send_result(rt.send_simple(
            &REWARD_ACTOR_ADDR,
            ext::reward::UPDATE_NETWORK_KPI,
            this_epoch_raw_byte_power,
            TokenAmount::zero(),
        ))
        .map_err(|e| e.wrap("failed to update network KPI with reward actor"))?;

        Ok(())
    }

    fn update_pledge_total(
        rt: &impl Runtime,
        params: UpdatePledgeTotalParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        rt.transaction(|st: &mut State, rt| {
            st.validate_miner_has_claim(rt.store(), &rt.message().caller())?;
            st.add_pledge_total(params.pledge_delta);
            if st.total_pledge_collateral.is_negative() {
                return Err(actor_error!(
                    illegal_state,
                    "negative total pledge collateral {}",
                    st.total_pledge_collateral
                ));
            }
            Ok(())
        })
    }

    fn submit_porep_for_bulk_verify(
        rt: &impl Runtime,
        params: SubmitPoRepForBulkVerifyParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;

        rt.transaction(|st: &mut State, rt| {
            st.validate_miner_has_claim(rt.store(), &rt.message().caller())?;

            let mut mmap = if let Some(ref batch) = st.proof_validation_batch {
                Multimap::from_root(
                    rt.store(),
                    batch,
                    HAMT_BIT_WIDTH,
                    PROOF_VALIDATION_BATCH_AMT_BITWIDTH,
                )
                .map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        "failed to load proof batching set",
                    )
                })?
            } else {
                debug!("ProofValidationBatch created");
                Multimap::new(rt.store(), HAMT_BIT_WIDTH, PROOF_VALIDATION_BATCH_AMT_BITWIDTH)
            };
            let miner_addr = rt.message().caller();
            let arr = mmap.get::<SealVerifyInfo>(&miner_addr.to_bytes()).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to get seal verify infos at addr {}", miner_addr),
                )
            })?;
            if let Some(arr) = arr {
                if arr.count() >= MAX_MINER_PROVE_COMMITS_PER_EPOCH {
                    return Err(ActorError::unchecked(
                        ERR_TOO_MANY_PROVE_COMMITS,
                        format!(
                            "miner {} attempting to prove commit over {} sectors in epoch",
                            miner_addr, MAX_MINER_PROVE_COMMITS_PER_EPOCH
                        ),
                    ));
                }
            }

            mmap.add(miner_addr.to_bytes().into(), params.seal_info).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to insert proof into set")
            })?;

            let mmrc = mmap.root().map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush proofs batch map")
            })?;

            rt.charge_gas("OnSubmitVerifySeal", detail::GAS_ON_SUBMIT_VERIFY_SEAL);
            st.proof_validation_batch = Some(mmrc);
            Ok(())
        })?;

        Ok(())
    }

    /// Returns the total power and pledge recorded by the power actor.
    /// The returned values are frozen during the cron tick before this epoch
    /// so that this method returns consistent values while processing all messages
    /// of an epoch.
    fn current_total_power(rt: &impl Runtime) -> Result<CurrentTotalPowerReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;

        Ok(CurrentTotalPowerReturn {
            raw_byte_power: st.this_epoch_raw_byte_power,
            quality_adj_power: st.this_epoch_quality_adj_power,
            pledge_collateral: st.this_epoch_pledge_collateral,
            quality_adj_power_smoothed: st.this_epoch_qa_power_smoothed,
        })
    }

    /// Returns the total raw power of the network.
    /// This is defined as the sum of the active (i.e. non-faulty) byte commitments
    /// of all miners that have more than the consensus minimum amount of storage active.
    /// This value is static over an epoch, and does NOT get updated as messages are executed.
    /// It is recalculated after all messages at an epoch have been executed.
    fn network_raw_power(rt: &impl Runtime) -> Result<NetworkRawPowerReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;

        Ok(NetworkRawPowerReturn { raw_byte_power: st.this_epoch_raw_byte_power })
    }

    /// Returns the raw power claimed by the specified miner,
    /// and whether the miner has more than the consensus minimum amount of storage active.
    /// The raw power is defined as the active (i.e. non-faulty) byte commitments of the miner.
    fn miner_raw_power(
        rt: &impl Runtime,
        params: MinerRawPowerParams,
    ) -> Result<MinerRawPowerReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;

        let (raw_byte_power, meets_consensus_minimum) =
            st.miner_nominal_power_meets_consensus_minimum(rt.policy(), rt.store(), params.miner)?;

        Ok(MinerRawPowerReturn { raw_byte_power, meets_consensus_minimum })
    }

    /// Returns the total number of miners created, regardless of whether or not
    /// they have any pledged storage.
    fn miner_count(rt: &impl Runtime) -> Result<MinerCountReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;

        Ok(MinerCountReturn { miner_count: st.miner_count })
    }

    /// Returns the total number of miners that have more than the consensus minimum amount of storage active.
    /// Active means that the storage must not be faulty.
    fn miner_consensus_count(rt: &impl Runtime) -> Result<MinerConsensusCountReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;

        Ok(MinerConsensusCountReturn { miner_consensus_count: st.miner_above_min_power_count })
    }

    fn process_batch_proof_verifies(
        rt: &impl Runtime,
        rewret: &ThisEpochRewardReturn,
    ) -> Result<(), String> {
        let mut miners: Vec<(Address, usize)> = Vec::new();
        let mut infos: Vec<SealVerifyInfo> = Vec::new();
        let mut st_err: Option<String> = None;
        let this_epoch_qa_power_smoothed = rt
            .transaction(|st: &mut State, rt| {
                let result = Ok(st.this_epoch_qa_power_smoothed.clone());
                let batch = match &st.proof_validation_batch {
                    None => {
                        debug!("ProofValidationBatch was nil, quitting verification");
                        return result;
                    }
                    Some(batch) => batch,
                };
                let mmap = match Multimap::from_root(
                    rt.store(),
                    batch,
                    HAMT_BIT_WIDTH,
                    PROOF_VALIDATION_BATCH_AMT_BITWIDTH,
                ) {
                    Ok(mmap) => mmap,
                    Err(e) => {
                        st_err = Some(format!("failed to load proofs validation batch {}", e));
                        return result;
                    }
                };

                let claims = match make_map_with_root_and_bitwidth::<_, Claim>(
                    &st.claims,
                    rt.store(),
                    HAMT_BIT_WIDTH,
                ) {
                    Ok(claims) => claims,
                    Err(e) => {
                        st_err = Some(format!("failed to load claims: {}", e));
                        return result;
                    }
                };

                if let Err(e) = mmap.for_all::<_, SealVerifyInfo>(|k, arr| {
                    let addr = match Address::from_bytes(&k.0) {
                        Ok(addr) => addr,
                        Err(e) => {
                            return Err(anyhow!("failed to parse address key: {}", e));
                        }
                    };

                    let contains_claim = match claims.contains_key(&addr.to_bytes()) {
                        Ok(contains_claim) => contains_claim,
                        Err(e) => return Err(anyhow!("failed to look up clain: {}", e)),
                    };

                    if !contains_claim {
                        debug!("skipping batch verifies for unknown miner: {}", addr);
                        return Ok(());
                    }

                    let num_proofs: usize = arr.count().try_into()?;
                    infos.reserve(num_proofs);
                    arr.for_each(|_, svi| {
                        infos.push(svi.clone());
                        Ok(())
                    })
                    .map_err(|e| {
                        anyhow!(
                            "failed to iterate over proof verify array for miner {}: {}",
                            addr,
                            e
                        )
                    })?;

                    miners.push((addr, num_proofs));
                    Ok(())
                }) {
                    // Do not return immediately, all runs that get this far should wipe the ProofValidationBatchQueue.
                    // If we leave the validation batch then in the case of a repeating state error the queue
                    // will quickly fill up and repeated traversals will start ballooning cron execution time.
                    st_err = Some(format!("failed to iterate proof batch: {}", e));
                }

                st.proof_validation_batch = None;
                result
            })
            .map_err(|e| {
                format!("failed to do transaction in process batch proof verifies: {}", e)
            })?;
        if let Some(st_err) = st_err {
            return Err(st_err);
        }

        let res =
            rt.batch_verify_seals(&infos).map_err(|e| format!("failed to batch verify: {}", e))?;

        let mut res_iter = infos.iter().zip(res.iter().copied());
        for (m, count) in miners {
            let successful: Vec<_> = res_iter
                .by_ref()
                // Take the miner's sectors.
                .take(count)
                // Filter by successful
                .filter(|(_, r)| *r)
                // Pull out the sector numbers.
                .map(|(info, _)| info.sector_id.number)
                // Deduplicate
                .filter({
                    let mut seen = BTreeSet::<_>::new();
                    move |snum| seen.insert(*snum)
                })
                .collect();

            // Result intentionally ignored
            if successful.is_empty() {
                continue;
            }
            if let Err(e) = extract_send_result(
                rt.send_simple(
                    &m,
                    ext::miner::CONFIRM_SECTOR_PROOFS_VALID_METHOD,
                    IpldBlock::serialize_cbor(&ext::miner::ConfirmSectorProofsParams {
                        sectors: successful,
                        reward_smoothed: rewret.this_epoch_reward_smoothed.clone(),
                        reward_baseline_power: rewret.this_epoch_baseline_power.clone(),
                        quality_adj_power_smoothed: this_epoch_qa_power_smoothed.clone(),
                    })
                    .map_err(|e| format!("failed to serialize ConfirmSectorProofsParams: {}", e))?,
                    Default::default(),
                ),
            ) {
                error!("failed to confirm sector proof validity to {}, error code {}", m, e);
            }
        }
        Ok(())
    }

    fn process_deferred_cron_events(
        rt: &impl Runtime,
        rewret: ThisEpochRewardReturn,
    ) -> Result<(), ActorError> {
        let rt_epoch = rt.curr_epoch();
        let mut cron_events = Vec::new();
        let st: State = rt.state()?;
        rt.transaction(|st: &mut State, rt| {
            let mut events = Multimap::from_root(
                rt.store(),
                &st.cron_event_queue,
                CRON_QUEUE_HAMT_BITWIDTH,
                CRON_QUEUE_AMT_BITWIDTH,
            )
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load cron events")
            })?;

            let claims =
                make_map_with_root_and_bitwidth::<_, Claim>(&st.claims, rt.store(), HAMT_BIT_WIDTH)
                    .map_err(|e| {
                        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load claims")
                    })?;
            for epoch in st.first_cron_epoch..=rt_epoch {
                let epoch_events = load_cron_events(&events, epoch).map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to load cron events at {}", epoch),
                    )
                })?;

                if epoch_events.is_empty() {
                    continue;
                }

                for evt in epoch_events.into_iter() {
                    let miner_has_claim =
                        claims.contains_key(&evt.miner_addr.to_bytes()).map_err(|e| {
                            e.downcast_default(
                                ExitCode::USR_ILLEGAL_STATE,
                                "failed to look up claim",
                            )
                        })?;
                    if !miner_has_claim {
                        debug!("skipping cron event for unknown miner: {}", evt.miner_addr);
                        continue;
                    }
                    cron_events.push(evt);
                }

                events.remove_all(&epoch_key(epoch)).map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to clear cron events at {}", epoch),
                    )
                })?;
            }

            st.first_cron_epoch = rt_epoch + 1;
            st.cron_event_queue = events.root().map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush events")
            })?;

            Ok(())
        })?;

        let mut failed_miner_crons = Vec::new();
        for event in cron_events {
            let params = IpldBlock::serialize_cbor(&ext::miner::DeferredCronEventParams {
                event_payload: event.callback_payload.bytes().to_owned(),
                reward_smoothed: rewret.this_epoch_reward_smoothed.clone(),
                quality_adj_power_smoothed: st.this_epoch_qa_power_smoothed.clone(),
            })?;
            let res = extract_send_result(rt.send_simple(
                &event.miner_addr,
                ext::miner::ON_DEFERRED_CRON_EVENT_METHOD,
                params,
                Default::default(),
            ));
            // If a callback fails, this actor continues to invoke other callbacks
            // and persists state removing the failed event from the event queue. It won't be tried again.
            // Failures are unexpected here but will result in removal of miner power
            // A log message would really help here.
            if let Err(e) = res {
                error!("OnDeferredCronEvent failed for miner {}: res {}", event.miner_addr, e);
                failed_miner_crons.push(event.miner_addr)
            }
        }

        if !failed_miner_crons.is_empty() {
            rt.transaction(|st: &mut State, rt| {
                let mut claims =
                    make_map_with_root_and_bitwidth(&st.claims, rt.store(), HAMT_BIT_WIDTH)
                        .map_err(|e| {
                            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load claims")
                        })?;

                // Remove power and leave miner frozen
                for miner_addr in failed_miner_crons {
                    if let Err(e) = st.delete_claim(rt.policy(), &mut claims, &miner_addr) {
                        error!(
                            "failed to delete claim for miner {} after\
                            failing on deferred cron event: {}",
                            miner_addr, e
                        );
                        continue;
                    }
                    st.miner_count -= 1
                }

                st.claims = claims.flush().map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush claims")
                })?;
                Ok(())
            })?;
        }
        Ok(())
    }
}

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "StoragePower"
    }

    actor_dispatch! {
        Constructor => constructor,
        CreateMiner|CreateMinerExported => create_miner,
        UpdateClaimedPower => update_claimed_power            ,
        EnrollCronEvent => enroll_cron_event,
        OnEpochTickEnd => on_epoch_tick_end,
        UpdatePledgeTotal => update_pledge_total,
        SubmitPoRepForBulkVerify => submit_porep_for_bulk_verify,
        CurrentTotalPower => current_total_power,
        NetworkRawPowerExported => network_raw_power,
        MinerRawPowerExported => miner_raw_power,
        MinerCountExported => miner_count,
        MinerConsensusCountExported => miner_consensus_count,
    }
}

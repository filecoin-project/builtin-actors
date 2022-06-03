#![allow(clippy::all)]

use fil_actor_account::Method as AccountMethod;
use fil_actor_market::{
    ActivateDealsParams, ComputeDataCommitmentParams, ComputeDataCommitmentReturn,
    Method as MarketMethod, OnMinerSectorsTerminateParams, SectorDataSpec, SectorDeals,
    SectorWeights, VerifyDealsForActivationParams, VerifyDealsForActivationReturn,
};
use fil_actor_miner::ext::market::ON_MINER_SECTORS_TERMINATE_METHOD;
use fil_actor_miner::ext::power::{UPDATE_CLAIMED_POWER_METHOD, UPDATE_PLEDGE_TOTAL_METHOD};
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, aggregate_prove_commit_network_fee, consensus_fault_penalty,
    initial_pledge_for_power, locked_reward_from_reward, max_prove_commit_duration,
    new_deadline_info_from_offset_and_epoch, pledge_penalty_for_continued_fault, power_for_sectors,
    qa_power_for_sector, qa_power_for_weight, reward_for_consensus_slash_report, Actor,
    ApplyRewardParams, BitFieldQueue, ChangeMultiaddrsParams, ChangePeerIDParams,
    ChangeWorkerAddressParams, CheckSectorProvenParams, CompactPartitionsParams,
    CompactSectorNumbersParams, ConfirmSectorProofsParams, CronEventPayload, Deadline,
    DeadlineInfo, Deadlines, DeclareFaultsParams, DeclareFaultsRecoveredParams,
    DeferredCronEventParams, DisputeWindowedPoStParams, ExpirationQueue, ExpirationSet,
    ExtendSectorExpirationParams, FaultDeclaration, GetControlAddressesReturn, Method,
    MinerConstructorParams as ConstructorParams, MinerInfo, Partition, PoStPartition, PowerPair,
    PreCommitSectorBatchParams, PreCommitSectorParams, ProveCommitSectorParams,
    RecoveryDeclaration, ReportConsensusFaultParams, SectorOnChainInfo, SectorPreCommitOnChainInfo,
    Sectors, State, SubmitWindowedPoStParams, TerminateSectorsParams, TerminationDeclaration,
    VestingFunds, WindowedPoSt, WithdrawBalanceParams, WithdrawBalanceReturn,
    CRON_EVENT_PROVING_DEADLINE,
};
use fil_actor_miner::{Method as MinerMethod, ProveCommitAggregateParams};
use fil_actor_power::{
    CurrentTotalPowerReturn, EnrollCronEventParams, Method as PowerMethod, UpdateClaimedPowerParams,
};
use fil_actor_reward::{Method as RewardMethod, ThisEpochRewardReturn};
use fil_actors_runtime::runtime::{DomainSeparationTag, Policy, Runtime, RuntimePolicy};
use fil_actors_runtime::{parse_uint_key, ActorDowncast};
use fil_actors_runtime::{test_utils::*, Map};
use fil_actors_runtime::{
    ActorError, Array, DealWeight, BURNT_FUNDS_ACTOR_ADDR, INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
};
use fvm_ipld_amt::Amt;
use fvm_shared::bigint::Zero;
use fvm_shared::HAMT_BIT_WIDTH;

use fvm_ipld_bitfield::iter::Ranges;
use fvm_ipld_bitfield::{BitField, UnvalidatedBitField, Validate};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::de::Deserialize;
use fvm_ipld_encoding::ser::Serialize;
use fvm_ipld_encoding::{BytesDe, Cbor, CborStore, RawBytes};
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::{ChainEpoch, QuantSpec, NO_QUANTIZATION};
use fvm_shared::commcid::{FIL_COMMITMENT_SEALED, FIL_COMMITMENT_UNSEALED};
use fvm_shared::consensus::ConsensusFault;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{
    AggregateSealVerifyInfo, PoStProof, RegisteredPoStProof, RegisteredSealProof, SealVerifyInfo,
    SectorID, SectorInfo, SectorNumber, SectorSize, StoragePower, WindowPoStVerifyInfo,
};
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::METHOD_SEND;

use cid::Cid;
use itertools::Itertools;
use multihash::derive::Multihash;
use multihash::MultihashDigest;
use num_traits::sign::Signed;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::convert::TryInto;
use std::ops::Neg;

const RECEIVER_ID: u64 = 1000;
pub type SectorsMap = BTreeMap<SectorNumber, SectorOnChainInfo>;

// A reward amount for use in tests where the vesting amount wants to be large enough to cover penalties.
#[allow(dead_code)]
pub const BIG_REWARDS: u128 = 10u128.pow(24);

// an expriration ~10 days greater than effective min expiration taking into account 30 days max between pre and prove commit
#[allow(dead_code)]
pub const DEFAULT_SECTOR_EXPIRATION: u64 = 220;

#[allow(dead_code)]
pub fn setup() -> (ActorHarness, MockRuntime) {
    let mut rt = MockRuntime::default();
    let h = ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    (h, rt)
}

pub struct ActorHarness {
    pub receiver: Address,
    pub owner: Address,
    pub worker: Address,
    pub worker_key: Address,

    pub control_addrs: Vec<Address>,

    pub seal_proof_type: RegisteredSealProof,
    pub window_post_proof_type: RegisteredPoStProof,
    pub sector_size: SectorSize,
    pub partition_size: u64,
    pub period_offset: ChainEpoch,
    pub next_sector_no: SectorNumber,

    pub network_pledge: TokenAmount,
    pub network_raw_power: StoragePower,
    pub network_qa_power: StoragePower,
    pub baseline_power: StoragePower,

    pub epoch_reward_smooth: FilterEstimate,
    pub epoch_qa_power_smooth: FilterEstimate,
}

#[allow(dead_code)]
impl ActorHarness {
    pub fn new(proving_period_offset: ChainEpoch) -> ActorHarness {
        let owner = Address::new_id(100);
        let worker = Address::new_id(101);
        let control_addrs = vec![Address::new_id(999), Address::new_id(998), Address::new_id(997)];
        let worker_key = new_bls_addr(0);
        let receiver = Address::new_id(RECEIVER_ID);
        let rwd = TokenAmount::from(10_000_000_000_000_000_000i128);
        let pwr = StoragePower::from(1i128 << 50);
        let proof_type = RegisteredSealProof::StackedDRG32GiBV1P1;

        ActorHarness {
            receiver,
            owner,
            worker,
            worker_key,
            control_addrs,

            seal_proof_type: proof_type,
            window_post_proof_type: proof_type.registered_window_post_proof().unwrap(),
            sector_size: proof_type.sector_size().unwrap(),
            partition_size: proof_type.window_post_partitions_sector().unwrap(),

            period_offset: proving_period_offset,
            next_sector_no: 0,

            network_pledge: rwd.clone() * TokenAmount::from(1000),
            network_raw_power: pwr.clone(),
            network_qa_power: pwr.clone(),
            baseline_power: pwr.clone(),

            epoch_reward_smooth: FilterEstimate::new(rwd, BigInt::from(0)),
            epoch_qa_power_smooth: FilterEstimate::new(pwr, BigInt::from(0)),
        }
    }

    pub fn get_state(&self, rt: &MockRuntime) -> State {
        rt.get_state::<State>()
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        check_state_invariants_from_mock_runtime(rt);
    }

    pub fn new_runtime(&self) -> MockRuntime {
        let mut rt = MockRuntime::default();

        rt.policy.valid_post_proof_type.insert(self.window_post_proof_type);
        rt.policy.valid_pre_commit_proof_type.insert(self.seal_proof_type);

        rt.receiver = self.receiver;
        rt.actor_code_cids.insert(self.owner, *ACCOUNT_ACTOR_CODE_ID);
        rt.actor_code_cids.insert(self.worker, *ACCOUNT_ACTOR_CODE_ID);
        for addr in &self.control_addrs {
            rt.actor_code_cids.insert(*addr, *ACCOUNT_ACTOR_CODE_ID);
        }

        rt.hash_func = fixed_hasher(self.period_offset);

        rt
    }

    pub fn set_proof_type(&mut self, proof_type: RegisteredSealProof) {
        self.seal_proof_type = proof_type;
        self.window_post_proof_type = proof_type.registered_window_post_proof().unwrap();
        self.sector_size = proof_type.sector_size().unwrap();
        self.partition_size = proof_type.window_post_partitions_sector().unwrap();
    }

    pub fn construct_and_verify(&self, rt: &mut MockRuntime) {
        let params = ConstructorParams {
            owner: self.owner,
            worker: self.worker,
            control_addresses: self.control_addrs.clone(),
            window_post_proof_type: self.window_post_proof_type,
            peer_id: vec![0],
            multi_addresses: vec![],
        };

        rt.actor_code_cids.insert(self.owner, *ACCOUNT_ACTOR_CODE_ID);
        rt.actor_code_cids.insert(self.worker, *ACCOUNT_ACTOR_CODE_ID);
        for a in self.control_addrs.iter() {
            rt.actor_code_cids.insert(*a, *ACCOUNT_ACTOR_CODE_ID);
        }

        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.expect_send(
            self.worker,
            AccountMethod::PubkeyAddress as u64,
            RawBytes::default(),
            TokenAmount::from(0u8),
            RawBytes::serialize(self.worker_key).unwrap(),
            ExitCode::OK,
        );

        let result = rt
            .call::<Actor>(Method::Constructor as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        expect_empty(result);
        rt.verify();
    }

    pub fn set_peer_id(&self, rt: &mut MockRuntime, new_id: Vec<u8>) {
        let params = ChangePeerIDParams { new_id: new_id.clone() };

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);

        let mut caller_addrs = self.control_addrs.clone();
        caller_addrs.push(self.worker);
        caller_addrs.push(self.owner);
        rt.expect_validate_caller_addr(caller_addrs);

        let result = rt
            .call::<Actor>(Method::ChangePeerID as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        expect_empty(result);
        rt.verify();

        let state = self.get_state(rt);
        let info = state.get_info(&rt.store).unwrap();

        assert_eq!(new_id, info.peer_id);
    }

    pub fn set_peer_id_fail(&self, rt: &mut MockRuntime, new_id: Vec<u8>) {
        let params = ChangePeerIDParams { new_id };

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);

        let result = rt
            .call::<Actor>(Method::ChangePeerID as u64, &RawBytes::serialize(params).unwrap())
            .unwrap_err();
        assert_eq!(result.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
        rt.verify();
    }

    pub fn set_multiaddr(&self, rt: &mut MockRuntime, new_multiaddrs: Vec<BytesDe>) {
        let params = ChangeMultiaddrsParams { new_multi_addrs: new_multiaddrs.clone() };

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());

        let result = rt
            .call::<Actor>(Method::ChangeMultiaddrs as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        expect_empty(result);
        rt.verify();

        let state = self.get_state(rt);
        let info = state.get_info(&rt.store).unwrap();

        assert_eq!(new_multiaddrs, info.multi_address);
    }

    pub fn set_multiaddr_fail(&self, rt: &mut MockRuntime, new_multiaddrs: Vec<BytesDe>) {
        let params = ChangeMultiaddrsParams { new_multi_addrs: new_multiaddrs };

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);

        let result = rt
            .call::<Actor>(Method::ChangeMultiaddrs as u64, &RawBytes::serialize(params).unwrap())
            .unwrap_err();
        assert_eq!(result.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
        rt.verify();
    }

    pub fn get_control_addresses(&self, rt: &mut MockRuntime) -> (Address, Address, Vec<Address>) {
        rt.expect_validate_caller_any();

        let result =
            rt.call::<Actor>(Method::ControlAddresses as u64, &RawBytes::default()).unwrap();
        rt.verify();

        let value = result.deserialize::<GetControlAddressesReturn>().unwrap();
        (value.owner, value.worker, value.control_addresses)
    }

    pub fn commit_and_prove_sectors(
        &mut self,
        rt: &mut MockRuntime,
        num_sectors: usize,
        lifetime_periods: u64,
        deal_ids: Vec<Vec<DealID>>,
        first: bool,
    ) -> Vec<SectorOnChainInfo> {
        let precommit_epoch = rt.epoch;
        let deadline = self.get_deadline_info(rt);
        let expiration =
            deadline.period_end() + lifetime_periods as i64 * rt.policy.wpost_proving_period;

        let mut precommits = Vec::with_capacity(num_sectors);
        for i in 0..num_sectors {
            let sector_no = self.next_sector_no;
            let sector_deal_ids =
                deal_ids.get(i).and_then(|ids| Some(ids.clone())).unwrap_or_default();
            let params = self.make_pre_commit_params(
                sector_no,
                precommit_epoch - 1,
                expiration,
                sector_deal_ids,
            );
            let precommit = self.pre_commit_sector_and_get(
                rt,
                params,
                PreCommitConfig::default(),
                first && i == 0,
            );
            precommits.push(precommit);
            self.next_sector_no += 1;
        }

        self.advance_to_epoch_with_cron(
            rt,
            precommit_epoch + rt.policy.pre_commit_challenge_delay + 1,
        );

        let mut info = Vec::with_capacity(num_sectors);
        for pc in precommits {
            let sector = self
                .prove_commit_sector_and_confirm(
                    rt,
                    &pc,
                    self.make_prove_commit_params(pc.info.sector_number),
                    ProveCommitConfig::empty(),
                )
                .unwrap();
            info.push(sector);
        }
        rt.reset();
        info
    }

    pub fn commit_and_prove_sector(
        &self,
        rt: &mut MockRuntime,
        sector_no: SectorNumber,
        lifetime_periods: i64,
        deal_ids: Vec<DealID>,
    ) -> SectorOnChainInfo {
        let precommit_epoch = rt.epoch;
        let deadline = self.deadline(rt);
        let expiration = deadline.period_end() + lifetime_periods * rt.policy.wpost_proving_period;

        // Precommit
        let pre_commit_params =
            self.make_pre_commit_params(sector_no, precommit_epoch - 1, expiration, deal_ids);
        let precommit = self.pre_commit_sector_and_get(
            rt,
            pre_commit_params.clone(),
            PreCommitConfig::default(),
            true,
        );

        self.advance_to_epoch_with_cron(
            rt,
            precommit_epoch + rt.policy.pre_commit_challenge_delay + 1,
        );

        let sector_info = self
            .prove_commit_sector_and_confirm(
                rt,
                &precommit,
                self.make_prove_commit_params(pre_commit_params.sector_number),
                ProveCommitConfig::empty(),
            )
            .unwrap();
        rt.reset();
        sector_info
    }

    pub fn compact_sector_numbers_raw(
        &self,
        rt: &mut MockRuntime,
        addr: Address,
        bf: BitField,
    ) -> Result<RawBytes, ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addr);
        rt.expect_validate_caller_addr(self.caller_addrs());

        let params =
            CompactSectorNumbersParams { mask_sector_numbers: UnvalidatedBitField::Validated(bf) };

        rt.call::<Actor>(Method::CompactSectorNumbers as u64, &RawBytes::serialize(params).unwrap())
    }

    pub fn compact_sector_numbers(&self, rt: &mut MockRuntime, addr: Address, bf: BitField) {
        self.compact_sector_numbers_raw(rt, addr, bf).unwrap();
        rt.verify();
    }

    pub fn get_deadline_info(&self, rt: &MockRuntime) -> DeadlineInfo {
        let state = self.get_state(rt);
        state.recorded_deadline_info(&rt.policy, rt.epoch)
    }

    pub fn make_pre_commit_params(
        &self,
        sector_no: u64,
        challenge: ChainEpoch,
        expiration: ChainEpoch,
        sector_deal_ids: Vec<DealID>,
    ) -> PreCommitSectorParams {
        PreCommitSectorParams {
            seal_proof: self.seal_proof_type,
            sector_number: sector_no,
            sealed_cid: make_sealed_cid(b"commr"),
            seal_rand_epoch: challenge,
            deal_ids: sector_deal_ids,
            expiration,
            // unused
            replace_capacity: false,
            replace_sector_deadline: 0,
            replace_sector_partition: 0,
            replace_sector_number: 0,
        }
    }

    pub fn make_prove_commit_params(&self, sector_no: u64) -> ProveCommitSectorParams {
        ProveCommitSectorParams { sector_number: sector_no, proof: vec![0u8; 192] }
    }

    pub fn pre_commit_sector_batch(
        &self,
        rt: &mut MockRuntime,
        params: PreCommitSectorBatchParams,
        conf: &PreCommitBatchConfig,
        base_fee: TokenAmount,
    ) -> Result<RawBytes, ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());

        self.expect_query_network_info(rt);
        let mut sector_deals = Vec::new();
        let mut sector_weights = Vec::new();
        let mut any_deals = false;
        for (i, sector) in params.sectors.iter().enumerate() {
            sector_deals.push(SectorDeals {
                sector_expiry: sector.expiration,
                deal_ids: sector.deal_ids.clone(),
            });

            if conf.sector_weights.len() > i {
                sector_weights.push(conf.sector_weights[i].clone());
            } else {
                sector_weights.push(SectorWeights {
                    deal_space: 0,
                    deal_weight: DealWeight::zero(),
                    verified_deal_weight: DealWeight::zero(),
                });
            }

            // Sanity check on expectations
            let sector_has_deals = !sector.deal_ids.is_empty();
            let deal_total_weight =
                &sector_weights[i].deal_weight + &sector_weights[i].verified_deal_weight;
            assert_eq!(
                sector_has_deals,
                !deal_total_weight.is_zero(),
                "sector deals inconsistent with configured weight"
            );
            assert_eq!(
                sector_has_deals,
                (sector_weights[i].deal_space != 0),
                "sector deals inconsistent with configured space"
            );
            any_deals |= sector_has_deals;
        }
        if any_deals {
            let vdparams = VerifyDealsForActivationParams { sectors: sector_deals };
            let vdreturn = VerifyDealsForActivationReturn { sectors: sector_weights };
            rt.expect_send(
                *STORAGE_MARKET_ACTOR_ADDR,
                MarketMethod::VerifyDealsForActivation as u64,
                RawBytes::serialize(vdparams).unwrap(),
                TokenAmount::from(0u8),
                RawBytes::serialize(vdreturn).unwrap(),
                ExitCode::OK,
            );
        }

        let state = self.get_state(rt);
        // burn networkFee
        if state.fee_debt > TokenAmount::from(0u8) || params.sectors.len() > 1 {
            let expected_network_fee =
                aggregate_pre_commit_network_fee(params.sectors.len() as i64, &base_fee);
            let expected_burn = expected_network_fee + state.fee_debt;
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                expected_burn,
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        if conf.first_for_miner {
            let dlinfo = new_deadline_info_from_offset_and_epoch(
                &rt.policy,
                state.proving_period_start,
                rt.epoch,
            );
            let cron_params = make_deadline_cron_event_params(dlinfo.last());
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::EnrollCronEvent as u64,
                RawBytes::serialize(cron_params).unwrap(),
                TokenAmount::from(0u8),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let result = rt.call::<Actor>(
            Method::PreCommitSectorBatch as u64,
            &RawBytes::serialize(params.clone()).unwrap(),
        );
        result
    }

    pub fn pre_commit_sector_batch_and_get(
        &self,
        rt: &mut MockRuntime,
        params: PreCommitSectorBatchParams,
        conf: &PreCommitBatchConfig,
        base_fee: TokenAmount,
    ) -> Vec<SectorPreCommitOnChainInfo> {
        let result = self.pre_commit_sector_batch(rt, params.clone(), conf, base_fee).unwrap();

        expect_empty(result);
        rt.verify();

        params.sectors.iter().map(|sector| self.get_precommit(rt, sector.sector_number)).collect()
    }

    pub fn pre_commit_sector(
        &self,
        rt: &mut MockRuntime,
        params: PreCommitSectorParams,
        conf: PreCommitConfig,
        first: bool,
    ) -> Result<RawBytes, ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());
        self.expect_query_network_info(rt);

        if !params.deal_ids.is_empty() {
            let vdparams = VerifyDealsForActivationParams {
                sectors: vec![SectorDeals {
                    sector_expiry: params.expiration,
                    deal_ids: params.deal_ids.clone(),
                }],
            };
            let vdreturn = VerifyDealsForActivationReturn {
                sectors: vec![SectorWeights {
                    deal_space: conf.deal_space as u64,
                    deal_weight: conf.deal_weight,
                    verified_deal_weight: conf.verified_deal_weight,
                }],
            };

            rt.expect_send(
                *STORAGE_MARKET_ACTOR_ADDR,
                MarketMethod::VerifyDealsForActivation as u64,
                RawBytes::serialize(vdparams).unwrap(),
                TokenAmount::from(0u8),
                RawBytes::serialize(vdreturn).unwrap(),
                ExitCode::OK,
            );
        }
        // in the original test the else branch does some redundant checks which we can omit.

        let state = self.get_state(rt);
        if state.fee_debt > TokenAmount::from(0u8) {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                state.fee_debt.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        if first {
            let dlinfo = new_deadline_info_from_offset_and_epoch(
                &rt.policy,
                state.proving_period_start,
                rt.epoch,
            );
            let cron_params = make_deadline_cron_event_params(dlinfo.last());
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::EnrollCronEvent as u64,
                RawBytes::serialize(cron_params).unwrap(),
                TokenAmount::from(0u8),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let result = rt.call::<Actor>(
            Method::PreCommitSector as u64,
            &RawBytes::serialize(params.clone()).unwrap(),
        );
        result
    }

    pub fn pre_commit_sector_and_get(
        &self,
        rt: &mut MockRuntime,
        params: PreCommitSectorParams,
        conf: PreCommitConfig,
        first: bool,
    ) -> SectorPreCommitOnChainInfo {
        let result = self.pre_commit_sector(rt, params.clone(), conf, first);

        expect_empty(result.unwrap());
        rt.verify();

        self.get_precommit(rt, params.sector_number)
    }

    pub fn has_precommit(&self, rt: &MockRuntime, sector_number: SectorNumber) -> bool {
        let state = self.get_state(rt);
        state.get_precommitted_sector(&rt.store, sector_number).unwrap().is_some()
    }

    pub fn get_precommit(
        &self,
        rt: &mut MockRuntime,
        sector_number: SectorNumber,
    ) -> SectorPreCommitOnChainInfo {
        let state = self.get_state(rt);
        state.get_precommitted_sector(&rt.store, sector_number).unwrap().unwrap()
    }

    pub fn expect_query_network_info(&self, rt: &mut MockRuntime) {
        let current_power = CurrentTotalPowerReturn {
            raw_byte_power: self.network_raw_power.clone(),
            quality_adj_power: self.network_qa_power.clone(),
            pledge_collateral: self.network_pledge.clone(),
            quality_adj_power_smoothed: self.epoch_qa_power_smooth.clone(),
        };
        let current_reward = ThisEpochRewardReturn {
            this_epoch_baseline_power: self.baseline_power.clone(),
            this_epoch_reward_smoothed: self.epoch_reward_smooth.clone(),
        };
        rt.expect_send(
            *REWARD_ACTOR_ADDR,
            RewardMethod::ThisEpochReward as u64,
            RawBytes::default(),
            TokenAmount::from(0u8),
            RawBytes::serialize(current_reward).unwrap(),
            ExitCode::OK,
        );
        rt.expect_send(
            *STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::CurrentTotalPower as u64,
            RawBytes::default(),
            TokenAmount::from(0u8),
            RawBytes::serialize(current_power).unwrap(),
            ExitCode::OK,
        );
    }

    pub fn prove_commit_sector_and_confirm(
        &self,
        rt: &mut MockRuntime,
        pc: &SectorPreCommitOnChainInfo,
        params: ProveCommitSectorParams,
        cfg: ProveCommitConfig,
    ) -> Result<SectorOnChainInfo, ActorError> {
        let sector_number = params.sector_number;
        self.prove_commit_sector(rt, pc, params)?;
        self.confirm_sector_proofs_valid(rt, cfg, vec![pc.clone()])?;

        Ok(self.get_sector(rt, sector_number))
    }

    pub fn prove_commit_sector(
        &self,
        rt: &mut MockRuntime,
        pc: &SectorPreCommitOnChainInfo,
        params: ProveCommitSectorParams,
    ) -> Result<(), ActorError> {
        let commd = make_piece_cid(b"commd");
        let seal_rand = Randomness(vec![1, 2, 3, 4]);
        let seal_int_rand = Randomness(vec![5, 6, 7, 8]);
        let interactive_epoch = pc.pre_commit_epoch + rt.policy.pre_commit_challenge_delay;

        // Prepare for and receive call to ProveCommitSector
        let input =
            SectorDataSpec { deal_ids: pc.info.deal_ids.clone(), sector_type: pc.info.seal_proof };
        let cdc_params = ComputeDataCommitmentParams { inputs: vec![input] };
        let cdc_ret = ComputeDataCommitmentReturn { commds: vec![commd] };
        rt.expect_send(
            *STORAGE_MARKET_ACTOR_ADDR,
            MarketMethod::ComputeDataCommitment as u64,
            RawBytes::serialize(cdc_params).unwrap(),
            TokenAmount::from(0u8),
            RawBytes::serialize(cdc_ret).unwrap(),
            ExitCode::OK,
        );

        let entropy = RawBytes::serialize(self.receiver).unwrap();
        rt.expect_get_randomness_from_tickets(
            DomainSeparationTag::SealRandomness,
            pc.info.seal_rand_epoch,
            entropy.to_vec(),
            seal_rand.clone(),
        );
        rt.expect_get_randomness_from_beacon(
            DomainSeparationTag::InteractiveSealChallengeSeed,
            interactive_epoch,
            entropy.to_vec(),
            seal_int_rand.clone(),
        );

        let actor_id = RECEIVER_ID;
        let seal = SealVerifyInfo {
            sector_id: SectorID { miner: actor_id, number: pc.info.sector_number },
            sealed_cid: pc.info.sealed_cid,
            registered_proof: pc.info.seal_proof,
            proof: params.proof.clone(),
            deal_ids: pc.info.deal_ids.clone(),
            randomness: seal_rand,
            interactive_randomness: seal_int_rand,
            unsealed_cid: commd,
        };
        rt.expect_send(
            *STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::SubmitPoRepForBulkVerify as u64,
            RawBytes::serialize(seal).unwrap(),
            TokenAmount::from(0u8),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.expect_validate_caller_any();
        let result = rt.call::<Actor>(
            Method::ProveCommitSector as u64,
            &RawBytes::serialize(params).unwrap(),
        )?;
        expect_empty(result);
        rt.verify();
        Ok(())
    }

    pub fn prove_commit_aggregate_sector(
        &self,
        rt: &mut MockRuntime,
        config: ProveCommitConfig,
        precommits: Vec<SectorPreCommitOnChainInfo>,
        params: ProveCommitAggregateParams,
        base_fee: BigInt,
    ) {
        // recieve call to ComputeDataCommittments
        let mut comm_ds = Vec::new();
        let mut cdc_inputs = Vec::new();
        for (i, precommit) in precommits.iter().enumerate() {
            let sector_data = SectorDataSpec {
                deal_ids: precommit.info.deal_ids.clone(),
                sector_type: precommit.info.seal_proof,
            };
            cdc_inputs.push(sector_data);
            let comm_d = make_piece_cid(format!("commd-{}", i).as_bytes());
            comm_ds.push(comm_d);
        }
        let cdc_params = ComputeDataCommitmentParams { inputs: cdc_inputs };
        let cdc_ret = ComputeDataCommitmentReturn { commds: comm_ds.clone() };
        rt.expect_send(
            *STORAGE_MARKET_ACTOR_ADDR,
            MarketMethod::ComputeDataCommitment as u64,
            RawBytes::serialize(cdc_params).unwrap(),
            BigInt::zero(),
            RawBytes::serialize(cdc_ret).unwrap(),
            ExitCode::OK,
        );
        self.expect_query_network_info(rt);

        // expect randomness queries for provided precommits
        let mut seal_rands = Vec::new();
        let mut seal_int_rands = Vec::new();

        for precommit in precommits.iter() {
            let seal_rand = Randomness(vec![1, 2, 3, 4]);
            seal_rands.push(seal_rand.clone());
            let seal_int_rand = Randomness(vec![5, 6, 7, 8]);
            seal_int_rands.push(seal_int_rand.clone());
            let interactive_epoch =
                precommit.pre_commit_epoch + rt.policy.pre_commit_challenge_delay;

            let receiver = rt.receiver;
            let buf = receiver.marshal_cbor().unwrap();
            rt.expect_get_randomness_from_tickets(
                DomainSeparationTag::SealRandomness,
                precommit.info.seal_rand_epoch,
                buf.clone(),
                seal_rand,
            );
            rt.expect_get_randomness_from_beacon(
                DomainSeparationTag::InteractiveSealChallengeSeed,
                interactive_epoch,
                buf,
                seal_int_rand,
            );
        }

        // verify syscall
        let mut svis = Vec::new();
        for (i, precommit) in precommits.iter().enumerate() {
            svis.push(AggregateSealVerifyInfo {
                sector_number: precommit.info.sector_number,
                randomness: seal_rands.get(i).cloned().unwrap(),
                interactive_randomness: seal_int_rands.get(i).cloned().unwrap(),
                sealed_cid: precommit.info.sealed_cid,
                unsealed_cid: comm_ds[i],
            })
        }
        rt.expect_aggregate_verify_seals(svis, params.aggregate_proof.clone(), Ok(()));

        // confirm sector proofs valid
        self.confirm_sector_proofs_valid_internal(rt, config, &precommits);

        // burn network fee
        let expected_fee = aggregate_prove_commit_network_fee(precommits.len() as i64, &base_fee);
        assert!(expected_fee > BigInt::zero());
        rt.expect_send(
            *BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            RawBytes::default(),
            expected_fee,
            RawBytes::default(),
            ExitCode::OK,
        );

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        let addrs = self.caller_addrs().clone();
        rt.expect_validate_caller_addr(addrs);
        rt.call::<Actor>(
            MinerMethod::ProveCommitAggregate as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
        rt.verify();
    }

    pub fn confirm_sector_proofs_valid(
        &self,
        rt: &mut MockRuntime,
        cfg: ProveCommitConfig,
        pcs: Vec<SectorPreCommitOnChainInfo>,
    ) -> Result<(), ActorError> {
        self.confirm_sector_proofs_valid_internal(rt, cfg, &pcs);

        let mut all_sector_numbers = Vec::new();
        for pc in pcs {
            all_sector_numbers.push(pc.info.sector_number);
        }

        rt.set_caller(*POWER_ACTOR_CODE_ID, *STORAGE_POWER_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![*STORAGE_POWER_ACTOR_ADDR]);

        let params = ConfirmSectorProofsParams {
            sectors: all_sector_numbers,
            reward_smoothed: self.epoch_reward_smooth.clone(),
            reward_baseline_power: self.baseline_power.clone(),
            quality_adj_power_smoothed: self.epoch_qa_power_smooth.clone(),
        };
        rt.call::<Actor>(
            Method::ConfirmSectorProofsValid as u64,
            &RawBytes::serialize(params).unwrap(),
        )?;
        rt.verify();
        Ok(())
    }

    fn confirm_sector_proofs_valid_internal(
        &self,
        rt: &mut MockRuntime,
        cfg: ProveCommitConfig,
        pcs: &[SectorPreCommitOnChainInfo],
    ) {
        let mut valid_pcs = Vec::new();
        for pc in pcs {
            if !pc.info.deal_ids.is_empty() {
                let params = ActivateDealsParams {
                    deal_ids: pc.info.deal_ids.clone(),
                    sector_expiry: pc.info.expiration,
                };

                let mut exit = ExitCode::OK;
                match cfg.verify_deals_exit.get(&pc.info.sector_number) {
                    Some(exit_code) => {
                        exit = *exit_code;
                    }
                    None => {
                        valid_pcs.push(pc);
                    }
                }

                rt.expect_send(
                    *STORAGE_MARKET_ACTOR_ADDR,
                    MarketMethod::ActivateDeals as u64,
                    RawBytes::serialize(params).unwrap(),
                    TokenAmount::from(0u8),
                    RawBytes::default(),
                    exit,
                );
            } else {
                valid_pcs.push(pc);
            }
        }

        if !valid_pcs.is_empty() {
            let mut expected_pledge = TokenAmount::from(0u8);
            let mut expected_qa_power = BigInt::from(0);
            let mut expected_raw_power = BigInt::from(0);

            for pc in valid_pcs {
                let pc_on_chain = self.get_precommit(rt, pc.info.sector_number);
                let duration = pc.info.expiration - rt.epoch;
                if duration >= rt.policy.min_sector_expiration {
                    let qa_power_delta = qa_power_for_weight(
                        self.sector_size,
                        duration,
                        &pc_on_chain.deal_weight,
                        &pc_on_chain.verified_deal_weight,
                    );
                    expected_qa_power += &qa_power_delta;
                    expected_raw_power += self.sector_size as u64;
                    let pledge = initial_pledge_for_power(
                        &qa_power_delta,
                        &self.baseline_power,
                        &self.epoch_reward_smooth,
                        &self.epoch_qa_power_smooth,
                        &rt.total_fil_circ_supply(),
                    );

                    if pc_on_chain.info.replace_capacity {
                        let replaced = self.get_sector(rt, pc_on_chain.info.replace_sector_number);
                        // Note: following snap deals, this behavior is *strictly* deprecated;
                        // if we get here, fail the test -- as opposed to the obsolete original
                        // test logic that would go like this:
                        // if replaced.initial_pledge > pledge {
                        //     pledge = replaced.initial_pledge;
                        // }
                        assert!(replaced.initial_pledge <= pledge);
                    }

                    expected_pledge += pledge;
                }
            }

            if expected_pledge != TokenAmount::from(0u8) {
                rt.expect_send(
                    *STORAGE_POWER_ACTOR_ADDR,
                    PowerMethod::UpdatePledgeTotal as u64,
                    RawBytes::serialize(BigIntSer(&expected_pledge)).unwrap(),
                    TokenAmount::from(0u8),
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }
        }
    }

    pub fn get_sector(&self, rt: &MockRuntime, sector_number: SectorNumber) -> SectorOnChainInfo {
        let state = self.get_state(rt);
        state.get_sector(&rt.store, sector_number).unwrap().unwrap()
    }

    pub fn advance_to_epoch_with_cron(&self, rt: &mut MockRuntime, epoch: ChainEpoch) {
        let mut deadline = self.get_deadline_info(rt);
        while deadline.last() < epoch {
            self.advance_deadline(rt, CronConfig::empty());
            deadline = self.get_deadline_info(rt);
        }
        rt.epoch = epoch;
    }

    pub fn advance_to_deadline(&self, rt: &mut MockRuntime, dlidx: u64) -> DeadlineInfo {
        let mut dlinfo = self.deadline(rt);
        while dlinfo.index != dlidx {
            dlinfo = self.advance_deadline(rt, CronConfig::empty());
        }
        dlinfo
    }

    pub fn deadline(&self, rt: &MockRuntime) -> DeadlineInfo {
        let state = self.get_state(rt);
        state.recorded_deadline_info(&rt.policy, rt.epoch)
    }

    pub fn advance_deadline(&self, rt: &mut MockRuntime, mut cfg: CronConfig) -> DeadlineInfo {
        let state = self.get_state(rt);
        let deadline = new_deadline_info_from_offset_and_epoch(
            &rt.policy,
            state.proving_period_start,
            rt.epoch,
        );

        if state.deadline_cron_active {
            rt.epoch = deadline.last();
            cfg.expected_enrollment = deadline.last() + rt.policy.wpost_challenge_window;
            self.on_deadline_cron(rt, cfg);
        }
        rt.epoch = deadline.next_open();

        let state = self.get_state(rt);
        state.deadline_info(&rt.policy, rt.epoch)
    }

    pub fn on_deadline_cron(&self, rt: &mut MockRuntime, cfg: CronConfig) {
        let state = self.get_state(rt);
        rt.expect_validate_caller_addr(vec![*STORAGE_POWER_ACTOR_ADDR]);

        // preamble
        let mut power_delta = PowerPair::zero();
        power_delta += &cfg.detected_faults_power_delta.unwrap_or_else(PowerPair::zero);
        power_delta += &cfg.expired_sectors_power_delta.unwrap_or_else(PowerPair::zero);

        if !power_delta.is_zero() {
            let params = UpdateClaimedPowerParams {
                raw_byte_delta: power_delta.raw,
                quality_adjusted_delta: power_delta.qa,
            };
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::UpdateClaimedPower as u64,
                RawBytes::serialize(params).unwrap(),
                TokenAmount::from(0u8),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let mut penalty_total = TokenAmount::from(0u8);
        let mut pledge_delta = TokenAmount::from(0u8);

        penalty_total += cfg.continued_faults_penalty.clone();
        penalty_total += cfg.repaid_fee_debt.clone();
        penalty_total += cfg.expired_precommit_penalty.clone();

        if penalty_total != TokenAmount::from(0u8) {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                penalty_total.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );

            let mut penalty_from_vesting = penalty_total.clone();
            // Outstanding fee debt is only repaid from unlocked balance, not vesting funds.
            penalty_from_vesting -= cfg.repaid_fee_debt.clone();
            // Precommit deposit burns are repaid from PCD account
            penalty_from_vesting -= cfg.expired_precommit_penalty.clone();
            // New penalties are paid first from vesting funds but, if exhausted, overflow to unlocked balance.
            penalty_from_vesting -= cfg.penalty_from_unlocked.clone();

            pledge_delta -= penalty_from_vesting;
        }

        pledge_delta += cfg.expired_sectors_pledge_delta;
        pledge_delta -= immediately_vesting_funds(rt, &state);

        if pledge_delta != TokenAmount::from(0u8) {
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::UpdatePledgeTotal as u64,
                RawBytes::serialize(BigIntSer(&pledge_delta)).unwrap(),
                TokenAmount::from(0u8),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        // Re-enrollment for next period.
        if !cfg.no_enrollment {
            let params = make_deadline_cron_event_params(cfg.expected_enrollment);
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::EnrollCronEvent as u64,
                RawBytes::serialize(params).unwrap(),
                TokenAmount::from(0u8),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let params = make_deferred_cron_event_params(
            self.epoch_reward_smooth.clone(),
            self.epoch_qa_power_smooth.clone(),
        );
        rt.set_caller(*POWER_ACTOR_CODE_ID, *STORAGE_POWER_ACTOR_ADDR);
        rt.call::<Actor>(Method::OnDeferredCronEvent as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        rt.verify();
    }

    pub fn submit_window_post(
        &self,
        rt: &mut MockRuntime,
        deadline: &DeadlineInfo,
        partitions: Vec<PoStPartition>,
        infos: Vec<SectorOnChainInfo>,
        cfg: PoStConfig,
    ) {
        let params = SubmitWindowedPoStParams {
            deadline: deadline.index,
            partitions,
            proofs: make_post_proofs(self.window_post_proof_type),
            chain_commit_epoch: deadline.challenge,
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        self.submit_window_post_raw(rt, deadline, infos, params, cfg).unwrap();
        rt.verify();
    }

    pub fn submit_window_post_raw(
        &self,
        rt: &mut MockRuntime,
        deadline: &DeadlineInfo,
        infos: Vec<SectorOnChainInfo>,
        params: SubmitWindowedPoStParams,
        cfg: PoStConfig,
    ) -> Result<RawBytes, ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        let chain_commit_rand = match cfg.chain_randomness {
            Some(r) => r,
            None => params.chain_commit_rand.clone(),
        };
        rt.expect_get_randomness_from_tickets(
            DomainSeparationTag::PoStChainCommit,
            params.chain_commit_epoch,
            Vec::new(),
            chain_commit_rand,
        );
        rt.expect_validate_caller_addr(self.caller_addrs());

        let challenge_rand = Randomness(Vec::from([10, 11, 12, 13]));

        // only sectors that are not skipped and not existing non-recovered faults will be verified
        let mut all_ignored = BitField::new();
        let mut all_recovered = BitField::new();
        let dln = self.get_deadline(rt, deadline.index);
        for p in &params.partitions {
            let maybe_partition = dln.load_partition(&rt.store, p.index);
            if let Ok(partition) = maybe_partition {
                let expected_faults = &partition.faults - &partition.recoveries;
                let skipped = get_bitfield(&p.skipped);
                all_ignored |= &(&expected_faults | &skipped);
                all_recovered |= &(&partition.recoveries - &skipped);
            }
        }
        let optimistic = all_recovered.is_empty();

        // find the first non-faulty, non-skipped sector in poSt to replace all faulty sectors.
        let mut maybe_good_info: Option<SectorOnChainInfo> = None;
        for ci in &infos {
            if !all_ignored.get(ci.sector_number) {
                maybe_good_info = Some(ci.clone());
                break;
            }
        }

        // good_info == None indicates all the sectors have been skipped and PoSt verification should not occur
        if !optimistic {
            if let Some(good_info) = maybe_good_info {
                let entropy = RawBytes::serialize(self.receiver).unwrap();
                rt.expect_get_randomness_from_beacon(
                    DomainSeparationTag::WindowedPoStChallengeSeed,
                    deadline.challenge,
                    entropy.to_vec(),
                    challenge_rand.clone(),
                );

                let vi = self.make_window_post_verify_info(
                    &infos,
                    &all_ignored,
                    good_info,
                    challenge_rand,
                    params.proofs.clone(),
                );
                let exit_code = match cfg.verification_exit {
                    Some(exit_code) => exit_code,
                    None => ExitCode::OK,
                };
                rt.expect_verify_post(vi, exit_code);
            }
        }

        if let Some(power_delta) = cfg.expected_power_delta {
            if !power_delta.is_zero() {
                let claim = UpdateClaimedPowerParams {
                    raw_byte_delta: power_delta.raw,
                    quality_adjusted_delta: power_delta.qa,
                };
                rt.expect_send(
                    *STORAGE_POWER_ACTOR_ADDR,
                    PowerMethod::UpdateClaimedPower as u64,
                    RawBytes::serialize(claim).unwrap(),
                    TokenAmount::from(0u8),
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }
        }

        rt.call::<Actor>(Method::SubmitWindowedPoSt as u64, &RawBytes::serialize(params).unwrap())
    }

    fn make_window_post_verify_info(
        &self,
        infos: &[SectorOnChainInfo],
        all_ignored: &BitField,
        good_info: SectorOnChainInfo,
        challenge_rand: Randomness,
        proofs: Vec<PoStProof>,
    ) -> WindowPoStVerifyInfo {
        let mut proof_infos = Vec::with_capacity(infos.len());
        for ci in infos {
            let mut si = ci.clone();
            if all_ignored.get(ci.sector_number) {
                si = good_info.clone();
            }
            let proof_info = SectorInfo {
                proof: si.seal_proof,
                sector_number: si.sector_number,
                sealed_cid: si.sealed_cid,
            };
            proof_infos.push(proof_info);
        }

        WindowPoStVerifyInfo {
            randomness: challenge_rand,
            proofs,
            challenged_sectors: proof_infos,
            prover: RECEIVER_ID,
        }
    }

    pub fn dispute_window_post(
        &self,
        rt: &mut MockRuntime,
        deadline: &DeadlineInfo,
        proof_index: u64,
        infos: &[SectorOnChainInfo],
        expect_success: Option<PoStDisputeResult>,
    ) {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

        self.expect_query_network_info(rt);

        let challenge_rand = Randomness(Vec::from([10, 11, 12, 13]));
        let mut all_ignored = BitField::new();
        let dln = self.get_deadline(rt, deadline.index);
        let post = self.get_submitted_proof(rt, &dln, proof_index);

        for idx in post.partitions.iter() {
            let partition = self.get_partition_snapshot(rt, &dln, idx);
            all_ignored |= &partition.faults;
            assert!(partition.recoveries.is_empty());
        }

        // find the first non-faulty, non-skipped sector in poSt to replace all faulty sectors.
        let mut maybe_good_info: Option<SectorOnChainInfo> = None;
        for ci in infos {
            if !all_ignored.get(ci.sector_number) {
                maybe_good_info = Some(ci.clone());
                break;
            }
        }
        let good_info = maybe_good_info.unwrap();

        let entropy = RawBytes::serialize(self.receiver).unwrap();
        rt.expect_get_randomness_from_beacon(
            DomainSeparationTag::WindowedPoStChallengeSeed,
            deadline.challenge,
            entropy.to_vec(),
            challenge_rand.clone(),
        );

        let vi = self.make_window_post_verify_info(
            infos,
            &all_ignored,
            good_info,
            challenge_rand,
            post.proofs,
        );
        let verify_result = match expect_success {
            Some(_) => ExitCode::USR_ILLEGAL_ARGUMENT,
            None => ExitCode::OK,
        };
        rt.expect_verify_post(vi, verify_result);

        if expect_success.is_some() {
            let dispute_result = expect_success.clone().unwrap();

            if dispute_result.expected_power_delta.is_some() {
                let expected_power_delta = dispute_result.expected_power_delta.unwrap();
                let claim = UpdateClaimedPowerParams {
                    raw_byte_delta: expected_power_delta.raw,
                    quality_adjusted_delta: expected_power_delta.qa,
                };
                rt.expect_send(
                    *STORAGE_POWER_ACTOR_ADDR,
                    PowerMethod::UpdateClaimedPower as u64,
                    RawBytes::serialize(claim).unwrap(),
                    TokenAmount::from(0u8),
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }

            if dispute_result.expected_reward.is_some() {
                let expected_reward = dispute_result.expected_reward.unwrap();
                rt.expect_send(
                    self.worker,
                    METHOD_SEND,
                    RawBytes::default(),
                    expected_reward,
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }

            if dispute_result.expected_penalty.is_some() {
                let expected_penalty = dispute_result.expected_penalty.unwrap();
                rt.expect_send(
                    *BURNT_FUNDS_ACTOR_ADDR,
                    METHOD_SEND,
                    RawBytes::default(),
                    expected_penalty,
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }

            if dispute_result.expected_pledge_delta.is_some() {
                let expected_pledge_delta = dispute_result.expected_pledge_delta.unwrap();
                rt.expect_send(
                    *STORAGE_POWER_ACTOR_ADDR,
                    PowerMethod::UpdatePledgeTotal as u64,
                    RawBytes::serialize(BigIntSer(&expected_pledge_delta)).unwrap(),
                    TokenAmount::from(0u8),
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }
        }

        let params =
            DisputeWindowedPoStParams { deadline: deadline.index, post_index: proof_index };
        let result = rt.call::<Actor>(
            Method::DisputeWindowedPoSt as u64,
            &RawBytes::serialize(params).unwrap(),
        );

        if expect_success.is_some() {
            result.unwrap();
        } else {
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "failed to dispute valid post",
                result,
            );
        }
        rt.verify();
    }

    fn get_submitted_proof(&self, rt: &MockRuntime, deadline: &Deadline, idx: u64) -> WindowedPoSt {
        amt_get::<WindowedPoSt>(rt, &deadline.optimistic_post_submissions_snapshot, idx)
    }

    fn get_partition_snapshot(&self, rt: &MockRuntime, deadline: &Deadline, idx: u64) -> Partition {
        deadline.load_partition_snapshot(&rt.store, idx).unwrap()
    }

    pub fn get_deadline(&self, rt: &MockRuntime, dlidx: u64) -> Deadline {
        let dls = self.get_deadlines(rt);
        dls.load_deadline(&rt.policy, &rt.store, dlidx).unwrap()
    }

    fn get_deadlines(&self, rt: &MockRuntime) -> Deadlines {
        let state = self.get_state(rt);
        state.load_deadlines(&rt.store).unwrap()
    }

    pub fn caller_addrs(&self) -> Vec<Address> {
        let mut caller_addrs = self.control_addrs.clone();
        caller_addrs.push(self.worker);
        caller_addrs.push(self.owner);
        caller_addrs
    }

    pub fn apply_rewards(&self, rt: &mut MockRuntime, amt: TokenAmount, penalty: TokenAmount) {
        // This harness function does not handle the state where apply rewards is
        // on a miner with existing fee debt.  This state is not protocol reachable
        // because currently fee debt prevents election participation.
        //
        // We further assume the miner can pay the penalty.  If the miner
        // goes into debt we can't rely on the harness call
        // TODO unify those cases
        let (lock_amt, _) = locked_reward_from_reward(amt.clone());
        let pledge_delta = lock_amt - &penalty;

        rt.set_caller(*REWARD_ACTOR_CODE_ID, *REWARD_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![*REWARD_ACTOR_ADDR]);
        // expect pledge update
        rt.expect_send(
            *STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::UpdatePledgeTotal as u64,
            RawBytes::serialize(BigIntSer(&pledge_delta)).unwrap(),
            TokenAmount::from(0u8),
            RawBytes::default(),
            ExitCode::OK,
        );

        if penalty > TokenAmount::from(0u8) {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                penalty.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let params = ApplyRewardParams { reward: amt, penalty: penalty };
        rt.call::<Actor>(Method::ApplyRewards as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        rt.verify();
    }

    pub fn get_locked_funds(&self, rt: &MockRuntime) -> TokenAmount {
        let state = self.get_state(rt);
        state.locked_funds
    }

    pub fn advance_and_submit_posts(&self, rt: &mut MockRuntime, sectors: &[SectorOnChainInfo]) {
        // Advance between 0 and 48 deadlines submitting window posts where necessary to keep
        // sectors proven.  If sectors is empty this is a noop. If sectors is a singleton this
        // will advance to that sector's proving deadline running deadline crons up to and
        // including this deadline. If sectors includes a sector assigned to the furthest
        // away deadline this will process a whole proving period.
        let state = self.get_state(rt);

        // this has to go into the loop or else go deal with the borrow checker (hint: you lose)
        //let sector_arr = Sectors::load(&rt.store, &state.sectors).unwrap();
        let mut deadlines: BTreeMap<u64, Vec<SectorOnChainInfo>> = BTreeMap::new();
        for sector in sectors {
            let (dlidx, _) =
                state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();
            match deadlines.get_mut(&dlidx) {
                Some(dl_sectors) => {
                    dl_sectors.push(sector.clone());
                }
                None => {
                    deadlines.insert(dlidx, vec![sector.clone()]);
                }
            }
        }

        let mut dlinfo = self.current_deadline(rt);
        while deadlines.len() > 0 {
            match deadlines.get(&dlinfo.index) {
                None => {}
                Some(dl_sectors) => {
                    let mut sector_nos = BitField::new();
                    for sector in dl_sectors {
                        sector_nos.set(sector.sector_number);
                    }

                    let dl_arr = state.load_deadlines(&rt.store).unwrap();
                    let dl = dl_arr.load_deadline(&rt.policy, &rt.store, dlinfo.index).unwrap();
                    let parts = Array::<Partition, _>::load(&dl.partitions, &rt.store).unwrap();

                    let mut partitions: Vec<PoStPartition> = Vec::new();
                    let mut power_delta = PowerPair::zero();
                    parts
                        .for_each(|part_idx, part| {
                            let sector_arr = Sectors::load(&rt.store, &state.sectors).unwrap();
                            let live = part.live_sectors();
                            let to_prove = &live & &sector_nos;
                            if to_prove.is_empty() {
                                return Ok(());
                            }

                            let mut to_skip = &live - &to_prove;
                            let not_recovering = &part.faults - &part.recoveries;

                            // Don't double-count skips.
                            to_skip -= &not_recovering;

                            let skipped_proven = &to_skip - &part.unproven;
                            let mut skipped_proven_sector_infos = Vec::new();
                            sector_arr
                                .amt
                                .for_each(|i, sector| {
                                    if skipped_proven.get(i) {
                                        skipped_proven_sector_infos.push(sector.clone());
                                    }
                                    Ok(())
                                })
                                .unwrap();
                            let new_faulty_power =
                                self.power_pair_for_sectors(&skipped_proven_sector_infos);

                            let new_proven = &part.unproven - &to_skip;
                            let mut new_proven_infos = Vec::new();
                            sector_arr
                                .amt
                                .for_each(|i, sector| {
                                    if new_proven.get(i) {
                                        new_proven_infos.push(sector.clone());
                                    }
                                    Ok(())
                                })
                                .unwrap();
                            let new_proven_power = self.power_pair_for_sectors(&new_proven_infos);

                            power_delta -= &new_faulty_power;
                            power_delta += &new_proven_power;

                            partitions.push(PoStPartition {
                                index: part_idx,
                                skipped: UnvalidatedBitField::Validated(to_skip),
                            });

                            Ok(())
                        })
                        .unwrap();

                    self.submit_window_post(
                        rt,
                        &dlinfo,
                        partitions,
                        dl_sectors.clone(),
                        PoStConfig::with_expected_power_delta(&power_delta),
                    );
                    deadlines.remove(&dlinfo.index);
                }
            }

            self.advance_deadline(rt, CronConfig::empty());
            dlinfo = self.current_deadline(rt);
        }
    }

    pub fn declare_faults(
        &self,
        rt: &mut MockRuntime,
        fault_sector_infos: &[SectorOnChainInfo],
    ) -> PowerPair {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());

        let ss = fault_sector_infos[0].seal_proof.sector_size().unwrap();
        let expected_delta = power_for_sectors(ss, fault_sector_infos);
        let expected_raw_delta = -expected_delta.raw;
        let expected_qa_delta = -expected_delta.qa;

        // expect power update
        let claim = UpdateClaimedPowerParams {
            raw_byte_delta: expected_raw_delta.clone(),
            quality_adjusted_delta: expected_qa_delta.clone(),
        };
        rt.expect_send(
            *STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::UpdateClaimedPower as u64,
            RawBytes::serialize(claim).unwrap(),
            TokenAmount::from(0u8),
            RawBytes::default(),
            ExitCode::OK,
        );

        // Calculate params from faulted sector infos
        let state = self.get_state(rt);
        let params = make_fault_params_from_faulting_sectors(&rt, &state, fault_sector_infos);
        rt.call::<Actor>(Method::DeclareFaults as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        rt.verify();

        PowerPair { raw: expected_raw_delta, qa: expected_qa_delta }
    }

    pub fn declare_recoveries(
        &self,
        rt: &mut MockRuntime,
        dlidx: u64,
        pidx: u64,
        recovery_sectors: BitField,
        expected_debt_repaid: TokenAmount,
    ) -> Result<RawBytes, ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());

        if expected_debt_repaid > TokenAmount::from(0u8) {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                expected_debt_repaid,
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        // Calculate params from faulted sector infos
        let recovery = RecoveryDeclaration {
            deadline: dlidx,
            partition: pidx,
            sectors: UnvalidatedBitField::Validated(recovery_sectors),
        };
        let params = DeclareFaultsRecoveredParams { recoveries: vec![recovery] };
        let ret = rt.call::<Actor>(
            Method::DeclareFaultsRecovered as u64,
            &RawBytes::serialize(params).unwrap(),
        );
        if ret.is_ok() {
            rt.verify();
        } else {
            rt.reset();
        }
        ret
    }

    pub fn continued_fault_penalty(&self, sectors: &[SectorOnChainInfo]) -> TokenAmount {
        let pwr = power_for_sectors(self.sector_size, sectors);
        pledge_penalty_for_continued_fault(
            &self.epoch_reward_smooth,
            &self.epoch_qa_power_smooth,
            &pwr.qa,
        )
    }

    pub fn collect_precommit_expirations(
        &self,
        rt: &MockRuntime,
        st: &State,
    ) -> HashMap<ChainEpoch, Vec<u64>> {
        let quant = st.quant_spec_every_deadline(&rt.policy);
        let queue = BitFieldQueue::new(&rt.store, &st.pre_committed_sectors_cleanup, quant)
            .map_err(|e| e.downcast_wrap("failed to load pre-commit clean up queue"))
            .unwrap();
        let mut expirations: HashMap<ChainEpoch, Vec<u64>> = HashMap::new();
        queue
            .amt
            .for_each(|epoch, bf| {
                let expanded: Vec<u64> =
                    bf.bounded_iter(rt.policy.addressed_sectors_max).unwrap().collect();
                expirations.insert(epoch.try_into().unwrap(), expanded);
                Ok(())
            })
            .unwrap();
        expirations
    }

    pub fn find_sector(&self, rt: &MockRuntime, sno: SectorNumber) -> (Deadline, Partition) {
        let state = self.get_state(rt);
        let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, sno).unwrap();
        self.get_deadline_and_partition(rt, dlidx, pidx)
    }

    pub fn current_deadline(&self, rt: &MockRuntime) -> DeadlineInfo {
        let state = self.get_state(rt);
        state.deadline_info(&rt.policy, rt.epoch)
    }

    fn power_pair_for_sectors(&self, sectors: &[SectorOnChainInfo]) -> PowerPair {
        power_for_sectors(self.sector_size, sectors)
    }

    pub fn get_deadline_and_partition(
        &self,
        rt: &MockRuntime,
        dlidx: u64,
        pidx: u64,
    ) -> (Deadline, Partition) {
        let deadline = self.get_deadline(&rt, dlidx);
        let partition = self.get_partition(&rt, &deadline, pidx);
        (deadline, partition)
    }

    fn get_partition(&self, rt: &MockRuntime, deadline: &Deadline, pidx: u64) -> Partition {
        deadline.load_partition(&rt.store, pidx).unwrap()
    }

    pub fn report_consensus_fault(
        &self,
        rt: &mut MockRuntime,
        from: Address,
        fault: Option<ConsensusFault>,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, from);
        let params =
            ReportConsensusFaultParams { header1: vec![], header2: vec![], header_extra: vec![] };

        rt.expect_verify_consensus_fault(
            params.header1.clone(),
            params.header2.clone(),
            params.header_extra.clone(),
            fault,
            ExitCode::OK,
        );

        let current_reward = ThisEpochRewardReturn {
            this_epoch_baseline_power: self.baseline_power.clone(),
            this_epoch_reward_smoothed: self.epoch_reward_smooth.clone(),
        };
        rt.expect_send(
            *REWARD_ACTOR_ADDR,
            RewardMethod::ThisEpochReward as u64,
            RawBytes::default(),
            TokenAmount::from(0u8),
            RawBytes::serialize(current_reward).unwrap(),
            ExitCode::OK,
        );
        let this_epoch_reward = self.epoch_reward_smooth.estimate();
        let penalty_total = consensus_fault_penalty(this_epoch_reward.clone());
        let reward_total = reward_for_consensus_slash_report(&this_epoch_reward);
        rt.expect_send(
            from,
            METHOD_SEND,
            RawBytes::default(),
            reward_total.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );

        // pay fault fee
        let to_burn = &penalty_total - &reward_total;
        rt.expect_send(
            *BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            RawBytes::default(),
            to_burn,
            RawBytes::default(),
            ExitCode::OK,
        );

        let result = rt.call::<Actor>(
            Method::ReportConsensusFault as u64,
            &RawBytes::serialize(params).unwrap(),
        )?;
        expect_empty(result);
        rt.verify();
        Ok(())
    }

    pub fn collect_deadline_expirations(
        &self,
        rt: &MockRuntime,
        deadline: &Deadline,
    ) -> HashMap<ChainEpoch, Vec<u64>> {
        let queue =
            BitFieldQueue::new(&rt.store, &deadline.expirations_epochs, NO_QUANTIZATION).unwrap();
        let mut expirations = HashMap::new();
        queue
            .amt
            .for_each(|epoch, bitfield| {
                let expanded = bitfield.bounded_iter(rt.policy.addressed_sectors_max).unwrap();
                expirations.insert(epoch as ChainEpoch, expanded.collect::<Vec<u64>>());
                Ok(())
            })
            .unwrap();
        expirations
    }

    pub fn collect_partition_expirations(
        &self,
        rt: &MockRuntime,
        partition: &Partition,
    ) -> HashMap<ChainEpoch, ExpirationSet> {
        let queue = ExpirationQueue::new(&rt.store, &partition.expirations_epochs, NO_QUANTIZATION)
            .unwrap();
        let mut expirations = HashMap::new();
        queue
            .amt
            .for_each(|epoch, set| {
                expirations.insert(epoch as ChainEpoch, set.clone());
                Ok(())
            })
            .unwrap();
        expirations
    }

    pub fn terminate_sectors(
        &self,
        rt: &mut MockRuntime,
        sectors: &BitField,
        expected_fee: TokenAmount,
    ) -> (PowerPair, TokenAmount) {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());

        let mut deal_ids: Vec<DealID> = Vec::new();
        let mut sector_infos: Vec<SectorOnChainInfo> = Vec::new();

        for sector in sectors.iter() {
            let sector = self.get_sector(rt, sector);
            deal_ids.extend(sector.deal_ids.iter());
            sector_infos.push(sector);
        }

        self.expect_query_network_info(rt);

        let mut pledge_delta = BigInt::zero();
        if expected_fee.is_positive() {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                expected_fee.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );
            pledge_delta = expected_fee.neg();
        }

        // notify change to initial pledge
        for sector_info in &sector_infos {
            pledge_delta -= sector_info.initial_pledge.to_owned();
        }

        if !pledge_delta.is_zero() {
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                UPDATE_PLEDGE_TOTAL_METHOD,
                RawBytes::serialize(BigIntSer(&pledge_delta)).unwrap(),
                BigInt::zero(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        if !deal_ids.is_empty() {
            let max_length = 8192;
            let size = deal_ids.len().min(max_length);
            let params = OnMinerSectorsTerminateParams {
                epoch: rt.epoch,
                deal_ids: deal_ids[0..size].to_owned(),
            };
            rt.expect_send(
                *STORAGE_MARKET_ACTOR_ADDR,
                ON_MINER_SECTORS_TERMINATE_METHOD,
                RawBytes::serialize(params).unwrap(),
                TokenAmount::zero(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let sector_power = power_for_sectors(self.sector_size, &sector_infos);
        let params = UpdateClaimedPowerParams {
            raw_byte_delta: -sector_power.raw.clone(),
            quality_adjusted_delta: -sector_power.qa.clone(),
        };
        rt.expect_send(
            *STORAGE_POWER_ACTOR_ADDR,
            UPDATE_CLAIMED_POWER_METHOD,
            RawBytes::serialize(params).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        // create declarations
        let state: State = rt.get_state();
        let deadlines = state.load_deadlines(rt.store()).unwrap();

        let mut terminations: Vec<TerminationDeclaration> = Vec::new();

        let policy = Policy::default();
        for sector in sectors.iter() {
            let (deadline, partition) = deadlines.find_sector(&policy, rt.store(), sector).unwrap();
            terminations.push(TerminationDeclaration {
                sectors: make_bitfield(&[sector]),
                deadline,
                partition,
            });
        }

        let params = TerminateSectorsParams { terminations };

        rt.call::<Actor>(Method::TerminateSectors as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        rt.verify();

        (-sector_power, pledge_delta)
    }

    pub fn change_peer_id(&self, rt: &mut MockRuntime, new_id: Vec<u8>) {
        let params = ChangePeerIDParams { new_id: new_id.to_owned() };

        rt.expect_validate_caller_addr(self.caller_addrs());
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);

        rt.call::<Actor>(Method::ChangePeerID as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        rt.verify();

        let state: State = rt.get_state();
        let info = state.get_info(rt.store()).unwrap();

        assert_eq!(new_id, info.peer_id);
    }

    pub fn repay_debts(
        &self,
        rt: &mut MockRuntime,
        value: &TokenAmount,
        expected_repaid_from_vest: &TokenAmount,
        expected_repaid_from_balance: &TokenAmount,
    ) -> Result<(), ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());

        rt.add_balance(value.clone());
        rt.set_received(value.clone());
        if expected_repaid_from_vest > &TokenAmount::zero() {
            let pledge_delta = expected_repaid_from_vest.neg();
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::UpdatePledgeTotal as u64,
                RawBytes::serialize(BigIntSer(&pledge_delta)).unwrap(),
                TokenAmount::zero(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let total_repaid = expected_repaid_from_vest + expected_repaid_from_balance;
        if total_repaid > TokenAmount::zero() {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                total_repaid.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }
        let result = rt.call::<Actor>(Method::RepayDebt as u64, &RawBytes::default())?;
        expect_empty(result);
        Ok(())
    }

    pub fn withdraw_funds(
        &self,
        rt: &mut MockRuntime,
        amount_requested: &TokenAmount,
        expected_withdrawn: &TokenAmount,
        expected_debt_repaid: &TokenAmount,
    ) -> Result<(), ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.owner);
        rt.expect_validate_caller_addr(vec![self.owner]);

        rt.expect_send(
            self.owner,
            METHOD_SEND,
            RawBytes::default(),
            expected_withdrawn.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        if expected_debt_repaid.is_positive() {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                expected_debt_repaid.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }
        let ret = rt
            .call::<Actor>(
                Method::WithdrawBalance as u64,
                &RawBytes::serialize(WithdrawBalanceParams {
                    amount_requested: amount_requested.clone(),
                })
                .unwrap(),
            )?
            .deserialize::<WithdrawBalanceReturn>()
            .unwrap();
        let withdrawn = ret.amount_withdrawn;
        rt.verify();

        assert_eq!(
            expected_withdrawn, &withdrawn,
            "return value indicates {} withdrawn but expected {}",
            withdrawn, expected_withdrawn
        );

        Ok(())
    }

    pub fn check_sector_proven(
        &self,
        rt: &mut MockRuntime,
        sector_number: SectorNumber,
    ) -> Result<(), ActorError> {
        let params = CheckSectorProvenParams { sector_number };
        rt.expect_validate_caller_any();
        rt.call::<Actor>(Method::CheckSectorProven as u64, &RawBytes::serialize(params).unwrap())?;
        rt.verify();
        Ok(())
    }

    pub fn change_worker_address(
        &self,
        rt: &mut MockRuntime,
        new_worker: Address,
        new_control_addresses: Vec<Address>,
    ) -> Result<RawBytes, ActorError> {
        rt.set_address_actor_type(new_worker.clone(), *ACCOUNT_ACTOR_CODE_ID);

        let params = ChangeWorkerAddressParams {
            new_worker: new_worker.clone(),
            new_control_addresses: new_control_addresses.clone(),
        };
        rt.expect_send(
            new_worker,
            AccountMethod::PubkeyAddress as u64,
            RawBytes::default(),
            TokenAmount::zero(),
            RawBytes::serialize(self.worker_key).unwrap(),
            ExitCode::OK,
        );

        rt.expect_validate_caller_addr(vec![self.owner]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.owner);
        let ret = rt.call::<Actor>(
            Method::ChangeWorkerAddress as u64,
            &RawBytes::serialize(params).unwrap(),
        );

        if ret.is_err() {
            rt.reset();
            return ret;
        }

        rt.verify();

        let state: State = rt.get_state();
        let info = state.get_info(rt.store()).unwrap();

        let control_addresses = new_control_addresses
            .iter()
            .map(|address| rt.get_id_address(&address).unwrap())
            .collect_vec();
        assert_eq!(control_addresses, info.control_addresses);

        ret
    }

    pub fn confirm_update_worker_key(&self, rt: &mut MockRuntime) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![self.owner]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.owner);
        rt.call::<Actor>(Method::ConfirmUpdateWorkerKey as u64, &RawBytes::default())?;
        rt.verify();

        Ok(())
    }

    pub fn extend_sectors(
        &self,
        rt: &mut MockRuntime,
        mut params: ExtendSectorExpirationParams,
    ) -> Result<RawBytes, ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);
        rt.expect_validate_caller_addr(self.caller_addrs());

        let mut qa_delta = BigInt::zero();
        for extension in params.extensions.iter_mut() {
            for sector_nr in extension.sectors.validate().unwrap().iter() {
                let sector = self.get_sector(&rt, sector_nr);
                let mut new_sector = sector.clone();
                new_sector.expiration = extension.new_expiration;
                qa_delta += qa_power_for_sector(self.sector_size, &new_sector)
                    - qa_power_for_sector(self.sector_size, &sector);
            }
        }

        if !qa_delta.is_zero() {
            let params = UpdateClaimedPowerParams {
                raw_byte_delta: BigInt::zero(),
                quality_adjusted_delta: qa_delta,
            };
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                UPDATE_CLAIMED_POWER_METHOD,
                RawBytes::serialize(params).unwrap(),
                TokenAmount::zero(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }

        let ret = rt.call::<Actor>(
            Method::ExtendSectorExpiration as u64,
            &RawBytes::serialize(params).unwrap(),
        )?;

        rt.verify();
        Ok(ret)
    }

    pub fn compact_partitions(
        &self,
        rt: &mut MockRuntime,
        deadline: u64,
        partition: BitField,
    ) -> Result<(), ActorError> {
        let params = CompactPartitionsParams {
            deadline,
            partitions: UnvalidatedBitField::Validated(partition),
        };

        rt.expect_validate_caller_addr(self.caller_addrs());
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.worker);

        rt.call::<Actor>(Method::CompactPartitions as u64, &RawBytes::serialize(params).unwrap())?;
        rt.verify();
        Ok(())
    }

    pub fn get_info(&self, rt: &MockRuntime) -> MinerInfo {
        let state: State = rt.get_state();
        state.get_info(rt.store()).unwrap()
    }

    pub fn change_owner_address(
        &self,
        rt: &mut MockRuntime,
        new_address: Address,
    ) -> Result<RawBytes, ActorError> {
        let expected = if rt.caller == self.owner {
            self.owner
        } else {
            if let Some(pending_owner) = self.get_info(rt).pending_owner_address {
                pending_owner
            } else {
                self.owner
            }
        };
        rt.expect_validate_caller_addr(vec![expected]);
        let ret = rt.call::<Actor>(
            Method::ChangeOwnerAddress as u64,
            &RawBytes::serialize(new_address).unwrap(),
        );

        if ret.is_ok() {
            rt.verify();
        } else {
            rt.reset();
        }
        ret
    }
}

#[allow(dead_code)]
pub struct PoStConfig {
    pub chain_randomness: Option<Randomness>,
    pub expected_power_delta: Option<PowerPair>,
    pub verification_exit: Option<ExitCode>,
}

#[allow(dead_code)]
impl PoStConfig {
    pub fn with_expected_power_delta(pwr: &PowerPair) -> PoStConfig {
        PoStConfig {
            chain_randomness: None,
            expected_power_delta: Some(pwr.clone()),
            verification_exit: None,
        }
    }

    pub fn with_randomness(rand: Randomness) -> PoStConfig {
        PoStConfig {
            chain_randomness: Some(rand),
            expected_power_delta: None,
            verification_exit: None,
        }
    }

    pub fn empty() -> PoStConfig {
        PoStConfig { chain_randomness: None, expected_power_delta: None, verification_exit: None }
    }
}

#[derive(Default)]
pub struct PreCommitConfig {
    pub deal_weight: DealWeight,
    pub verified_deal_weight: DealWeight,
    pub deal_space: u64,
}

#[allow(dead_code)]
impl PreCommitConfig {
    pub fn empty() -> PreCommitConfig {
        PreCommitConfig {
            deal_weight: DealWeight::from(0),
            verified_deal_weight: DealWeight::from(0),
            deal_space: 0,
        }
    }

    pub fn new(
        deal_weight: DealWeight,
        verified_deal_weight: DealWeight,
        deal_space: SectorSize,
    ) -> PreCommitConfig {
        PreCommitConfig { deal_weight, verified_deal_weight, deal_space: deal_space as u64 }
    }

    pub fn default() -> PreCommitConfig {
        PreCommitConfig {
            deal_weight: DealWeight::from(0),
            verified_deal_weight: DealWeight::from(0),
            deal_space: SectorSize::_2KiB as u64,
        }
    }
}

#[derive(Default, Clone)]
pub struct ProveCommitConfig {
    pub verify_deals_exit: HashMap<SectorNumber, ExitCode>,
}

#[allow(dead_code)]
impl ProveCommitConfig {
    pub fn empty() -> ProveCommitConfig {
        ProveCommitConfig { verify_deals_exit: HashMap::new() }
    }
}

#[derive(Default)]
pub struct PreCommitBatchConfig {
    pub sector_weights: Vec<SectorWeights>,
    pub first_for_miner: bool,
}

#[derive(Default)]
pub struct CronConfig {
    pub no_enrollment: bool, // true if expect not to continue enrollment false otherwise
    pub expected_enrollment: ChainEpoch,
    pub detected_faults_power_delta: Option<PowerPair>,
    pub expired_sectors_power_delta: Option<PowerPair>,
    pub expired_sectors_pledge_delta: TokenAmount,
    pub continued_faults_penalty: TokenAmount, // Expected amount burnt to pay continued fault penalties.
    pub expired_precommit_penalty: TokenAmount, // Expected amount burnt to pay for expired precommits
    pub repaid_fee_debt: TokenAmount,           // Expected amount burnt to repay fee debt.
    pub penalty_from_unlocked: TokenAmount, // Expected reduction in unlocked balance from penalties exceeding vesting funds.
}

#[allow(dead_code)]
impl CronConfig {
    pub fn empty() -> CronConfig {
        CronConfig {
            no_enrollment: false,
            expected_enrollment: 0,
            detected_faults_power_delta: None,
            expired_sectors_power_delta: None,
            expired_sectors_pledge_delta: TokenAmount::from(0u8),
            continued_faults_penalty: TokenAmount::from(0u8),
            expired_precommit_penalty: TokenAmount::from(0u8),
            repaid_fee_debt: TokenAmount::from(0u8),
            penalty_from_unlocked: TokenAmount::from(0u8),
        }
    }

    pub fn with_continued_faults_penalty(fault_fee: TokenAmount) -> CronConfig {
        let mut cfg = CronConfig::empty();
        cfg.continued_faults_penalty = fault_fee;
        cfg
    }

    pub fn with_detected_faults_power_delta_and_continued_faults_penalty(
        pwr_delta: &PowerPair,
        fault_fee: TokenAmount,
    ) -> CronConfig {
        let mut cfg = CronConfig::empty();
        cfg.detected_faults_power_delta = Some(pwr_delta.clone());
        cfg.continued_faults_penalty = fault_fee;
        cfg
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct PoStDisputeResult {
    pub expected_power_delta: Option<PowerPair>,
    pub expected_pledge_delta: Option<TokenAmount>,
    pub expected_penalty: Option<TokenAmount>,
    pub expected_reward: Option<TokenAmount>,
}

#[allow(dead_code)]
pub fn assert_bitfield_equals(bf: &BitField, bits: &[u64]) {
    let mut rbf = BitField::new();
    for bit in bits {
        rbf.set(*bit);
    }
    assert!(bf == &rbf);
}

#[allow(dead_code)]
pub fn make_empty_bitfield() -> UnvalidatedBitField {
    UnvalidatedBitField::Validated(BitField::new())
}

#[allow(dead_code)]
pub fn make_bitfield(bits: &[u64]) -> UnvalidatedBitField {
    UnvalidatedBitField::Validated(BitField::try_from_bits(bits.iter().copied()).unwrap())
}

#[allow(dead_code)]
pub fn get_bitfield(ubf: &UnvalidatedBitField) -> BitField {
    match ubf {
        UnvalidatedBitField::Validated(bf) => bf.clone(),
        UnvalidatedBitField::Unvalidated(bytes) => BitField::from_bytes(bytes).unwrap(),
    }
}

#[allow(dead_code)]
pub fn make_prove_commit_aggregate(sector_nos: &BitField) -> ProveCommitAggregateParams {
    ProveCommitAggregateParams {
        sector_numbers: UnvalidatedBitField::Validated(sector_nos.clone()),
        aggregate_proof: vec![0; 1024],
    }
}

// multihash library doesn't support poseidon hashing, so we fake it
#[derive(Clone, Copy, Debug, Eq, Multihash, PartialEq)]
#[mh(alloc_size = 64)]
enum MhCode {
    #[mh(code = 0xb401, hasher = multihash::Sha2_256)]
    PoseidonFake,
    #[mh(code = 0x1012, hasher = multihash::Sha2_256)]
    Sha256TruncPaddedFake,
}

fn immediately_vesting_funds(rt: &MockRuntime, state: &State) -> TokenAmount {
    let vesting = rt.store.get_cbor::<VestingFunds>(&state.vesting_funds).unwrap().unwrap();
    let mut sum = TokenAmount::from(0u8);
    for vf in vesting.funds {
        if vf.epoch < rt.epoch {
            sum += vf.amount;
        } else {
            break;
        }
    }
    sum
}

pub fn make_post_proofs(proof_type: RegisteredPoStProof) -> Vec<PoStProof> {
    let proof = PoStProof { post_proof: proof_type, proof_bytes: Vec::from(*b"proof1") };
    vec![proof]
}

fn make_sealed_cid(input: &[u8]) -> Cid {
    // Note: multihash library doesn't support Poseidon hashing, so we fake it
    let h = MhCode::PoseidonFake.digest(input);
    Cid::new_v1(FIL_COMMITMENT_SEALED, h)
}

fn make_piece_cid(input: &[u8]) -> Cid {
    let h = MhCode::Sha256TruncPaddedFake.digest(input);
    Cid::new_v1(FIL_COMMITMENT_UNSEALED, h)
}

fn make_deadline_cron_event_params(epoch: ChainEpoch) -> EnrollCronEventParams {
    let payload = CronEventPayload { event_type: CRON_EVENT_PROVING_DEADLINE };
    EnrollCronEventParams { event_epoch: epoch, payload: RawBytes::serialize(payload).unwrap() }
}

fn make_deferred_cron_event_params(
    epoch_reward_smooth: FilterEstimate,
    epoch_qa_power_smooth: FilterEstimate,
) -> DeferredCronEventParams {
    let payload = CronEventPayload { event_type: CRON_EVENT_PROVING_DEADLINE };
    DeferredCronEventParams {
        event_payload: Vec::from(RawBytes::serialize(payload).unwrap().bytes()),
        reward_smoothed: epoch_reward_smooth,
        quality_adj_power_smoothed: epoch_qa_power_smooth,
    }
}

fn make_fault_params_from_faulting_sectors(
    rt: &MockRuntime,
    state: &State,
    fault_sector_infos: &[SectorOnChainInfo],
) -> DeclareFaultsParams {
    let mut declaration_map: BTreeMap<(u64, u64), FaultDeclaration> = BTreeMap::new();
    for sector in fault_sector_infos {
        let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();
        match declaration_map.get_mut(&(dlidx, pidx)) {
            Some(declaration) => {
                declaration.sectors.validate_mut().unwrap().set(sector.sector_number);
            }
            None => {
                let mut bf = BitField::new();
                bf.set(sector.sector_number);

                let declaration = FaultDeclaration {
                    deadline: dlidx,
                    partition: pidx,
                    sectors: UnvalidatedBitField::Validated(bf),
                };

                declaration_map.insert((dlidx, pidx), declaration);
            }
        }
    }

    // I want to just write:
    // let declarations = declaration_map.values().collect();
    // but the compiler doesn't let me; so I do it by hand like a savange
    let keys: Vec<(u64, u64)> = declaration_map.keys().cloned().collect();
    let declarations = keys.iter().map(|k| declaration_map.remove(k).unwrap()).collect();

    DeclareFaultsParams { faults: declarations }
}

#[allow(dead_code)]
pub fn amt_to_vec<T>(rt: &MockRuntime, c: &Cid) -> Vec<T>
where
    T: Clone + Serialize + for<'a> Deserialize<'a>,
{
    let mut result = Vec::new();
    let arr = Array::<T, _>::load(c, &rt.store).unwrap();
    arr.for_each(|_, v: &T| {
        result.push(v.clone());
        Ok(())
    })
    .unwrap();
    result
}

#[allow(dead_code)]
pub fn amt_get<T>(rt: &MockRuntime, c: &Cid, i: u64) -> T
where
    T: Clone + Serialize + for<'a> Deserialize<'a>,
{
    let arr = Array::<T, _>::load(c, &rt.store).unwrap();
    arr.get(i).unwrap().unwrap().clone()
}

// Returns a fake hashing function that always arranges the first 8 bytes of the digest to be the binary
// encoding of a target uint64.
fn fixed_hasher(offset: ChainEpoch) -> Box<dyn Fn(&[u8]) -> [u8; 32]> {
    let hash = move |_: &[u8]| -> [u8; 32] {
        let mut result = [0u8; 32];
        for (i, item) in result.iter_mut().enumerate().take(8) {
            *item = ((offset >> (8 * (7 - i))) & 0xff) as u8;
        }
        result
    };
    Box::new(hash)
}

pub struct DealSummary {
    pub sector_start: ChainEpoch,
    pub sector_expiration: ChainEpoch,
}

pub struct StateSummary {
    pub live_power: PowerPair,
    pub active_power: PowerPair,
    pub faulty_power: PowerPair,
    pub deals: BTreeMap<DealID, DealSummary>,
    pub window_post_proof_type: RegisteredPoStProof,
    pub deadline_cron_active: bool,
}

impl Default for StateSummary {
    fn default() -> Self {
        StateSummary {
            live_power: PowerPair::zero(),
            active_power: PowerPair::zero(),
            faulty_power: PowerPair::zero(),
            window_post_proof_type: RegisteredPoStProof::Invalid(0),
            deadline_cron_active: false,
            deals: BTreeMap::new(),
        }
    }
}

fn check_miner_info(info: MinerInfo, acc: &MessageAccumulator) {
    acc.require(
        info.owner.protocol() == Protocol::ID,
        &format!("owner address {} is not an ID address", info.owner),
    );
    acc.require(
        info.worker.protocol() == Protocol::ID,
        &format!("worker address {} is not an ID address", info.worker),
    );
    info.control_addresses.iter().for_each(|address| {
        acc.require(
            address.protocol() == Protocol::ID,
            &format!("control address {} is not an ID address", address),
        )
    });

    if let Some(pending_worker_key) = info.pending_worker_key {
        acc.require(
            pending_worker_key.new_worker.protocol() == Protocol::ID,
            &format!(
                "pending worker address {} is not an ID address",
                pending_worker_key.new_worker
            ),
        );
        acc.require(
            pending_worker_key.new_worker != info.worker,
            &format!(
                "pending worker key {} is same as existing worker {}",
                pending_worker_key.new_worker, info.worker
            ),
        );
    }

    if let Some(pending_owner_address) = info.pending_owner_address {
        acc.require(
            pending_owner_address.protocol() == Protocol::ID,
            &format!("pending owner address {} is not an ID address", pending_owner_address),
        );
        acc.require(
            pending_owner_address != info.owner,
            &format!(
                "pending owner address {} is same as existing owner {}",
                pending_owner_address, info.owner
            ),
        );
    }

    if let RegisteredPoStProof::Invalid(id) = info.window_post_proof_type {
        acc.add(&format!("invalid Window PoSt proof type {id}"));
    } else {
        // safe to unwrap as we know it's valid at this point
        let sector_size = info.window_post_proof_type.sector_size().unwrap();
        acc.require(
            info.sector_size == sector_size,
            &format!(
                "sector size {} is wrong for Window PoSt proof type {:?}: {}",
                info.sector_size, info.window_post_proof_type, sector_size
            ),
        );

        let partition_sectors =
            info.window_post_proof_type.window_post_partitions_sector().unwrap();
        acc.require(info.window_post_partition_sectors == partition_sectors, &format!("miner partition sectors {} does not match partition sectors {} for PoSt proof type {:?}", info.window_post_partition_sectors, partition_sectors, info.window_post_proof_type));
    }
}

fn check_miner_balances<BS: Blockstore>(
    policy: &Policy,
    state: &State,
    store: &BS,
    balance: &BigInt,
    acc: &MessageAccumulator,
) {
    acc.require(
        !balance.is_negative(),
        &format!("miner actor balance is less than zero: {balance}"),
    );
    acc.require(
        !state.locked_funds.is_negative(),
        &format!("miner locked funds is less than zero: {}", state.locked_funds),
    );
    acc.require(
        !state.pre_commit_deposits.is_negative(),
        &format!("miner precommit deposit is less than zero: {}", state.pre_commit_deposits),
    );
    acc.require(
        !state.initial_pledge.is_negative(),
        &format!("miner initial pledge is less than zero: {}", state.initial_pledge),
    );
    acc.require(
        !state.fee_debt.is_negative(),
        &format!("miner fee debt is less than zero: {}", state.fee_debt),
    );

    acc.require(balance - &state.locked_funds - &state.pre_commit_deposits - &state.initial_pledge >= BigInt::zero(), &format!("miner balance {balance} is less than sum of locked funds ({}), precommit deposit ({}) and initial pledge ({})", state.locked_funds, state.pre_commit_deposits, state.initial_pledge));

    // locked funds must be sum of vesting table and vesting table payments must be quantized
    let mut vesting_sum = BigInt::zero();
    match state.load_vesting_funds(store) {
        Ok(funds) => {
            let quant = state.quant_spec_every_deadline(policy);
            funds.funds.iter().for_each(|entry| {
                acc.require(
                    entry.amount.is_positive(),
                    &format!("non-positive amount in miner vesting table entry {entry:?}"),
                );
                vesting_sum += &entry.amount;

                let quantized = quant.quantize_up(entry.epoch);
                acc.require(
                    entry.epoch == quantized,
                    &format!(
                        "vesting table entry has non-quantized epoch {} (should be {quantized})",
                        entry.epoch
                    ),
                );
            });
        }
        Err(e) => {
            acc.add(&format!("error loading vesting funds: {e}"));
        }
    };

    acc.require(
        state.locked_funds == vesting_sum,
        &format!(
            "locked funds {} is not sum of vesting table entries {vesting_sum}",
            state.locked_funds
        ),
    );

    // non zero funds implies that DeadlineCronActive is true
    if state.continue_deadline_cron() {
        acc.require(state.deadline_cron_active, "DeadlineCronActive == false when IP+PCD+LF > 0");
    }
}

fn check_precommits<BS: Blockstore>(
    policy: &Policy,
    state: &State,
    store: &BS,
    allocated_sectors: &BTreeSet<u64>,
    acc: &MessageAccumulator,
) {
    let quant = state.quant_spec_every_deadline(policy);

    // invert pre-commit clean up queue into a lookup by sector number
    let mut cleanup_epochs: BTreeMap<u64, ChainEpoch> = BTreeMap::new();
    match BitFieldQueue::new(store, &state.pre_committed_sectors_cleanup, quant) {
        Ok(queue) => {
            let ret = queue.amt.for_each(|epoch, expiration_bitfield| {
                let epoch = epoch as ChainEpoch;
                let quantized = quant.quantize_up(epoch);
                acc.require(
                    quantized == epoch,
                    &format!("pre-commit expiration {epoch} is not quantized"),
                );

                expiration_bitfield.iter().for_each(|sector_number| {
                    cleanup_epochs.insert(sector_number, epoch);
                });
                Ok(())
            });
            acc.require_no_error(ret, "error iterating pre-commit clean-up queue");
        }
        Err(e) => {
            acc.add(&format!("error loading pre-commit clean-up queue: {e}"));
        }
    };

    let mut precommit_total = BigInt::zero();

    let precommited_sectors =
        Map::<_, SectorPreCommitOnChainInfo>::load(&state.pre_committed_sectors, store);

    match precommited_sectors {
        Ok(precommited_sectors) => {
            let ret = precommited_sectors.for_each(|key, precommit| {
                let sector_number = match parse_uint_key(&key) {
                    Ok(sector_number) => sector_number,
                    Err(e) => {
                        acc.add(&format!("error parsing pre-commit key as uint: {e}"));
                        return Ok(());
                    }
                };

                acc.require(
                    allocated_sectors.contains(&sector_number),
                    &format!("pre-commited sector number has not been allocated {sector_number}"),
                );

                acc.require(
                    cleanup_epochs.contains_key(&sector_number),
                    &format!("no clean-up epoch for pre-commit at {}", precommit.pre_commit_epoch),
                );
                precommit_total += &precommit.pre_commit_deposit;
                Ok(())
            });
            acc.require_no_error(ret, "error iterating pre-commited sectors");
        }
        Err(e) => {
            acc.add(&format!("error loading precommited_sectors: {e}"));
        }
    };

    acc.require(state.pre_commit_deposits == precommit_total,&format!("sum of pre-commit deposits {precommit_total} does not equal recorded pre-commit deposit {}", state.pre_commit_deposits));
}

pub fn check_state_invariants_from_mock_runtime(rt: &MockRuntime) {
    let (_, acc) =
        check_state_invariants(rt.policy(), rt.get_state::<State>(), rt.store(), &rt.get_balance());
    assert!(acc.is_empty(), "{}", acc.messages().join("\n"));
}

#[allow(dead_code)]
pub fn check_state_invariants<BS: Blockstore>(
    policy: &Policy,
    state: State,
    store: &BS,
    balance: &TokenAmount,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();
    let sector_size;

    let mut miner_summary =
        StateSummary { deadline_cron_active: state.deadline_cron_active, ..Default::default() };

    // load data from linked structures
    match state.get_info(store) {
        Ok(info) => {
            miner_summary.window_post_proof_type = info.window_post_proof_type;
            sector_size = info.sector_size;
            check_miner_info(info, &acc);
        }
        Err(e) => {
            // Stop here, it's too hard to make other useful checks.
            acc.add(&format!("error loading miner info: {e}"));
            return (miner_summary, acc);
        }
    };

    check_miner_balances(policy, &state, store, &balance, &acc);

    let allocated_sectors = match store.get_cbor::<BitField>(&state.allocated_sectors) {
        Ok(Some(allocated_sectors)) => {
            if let Some(sectors) = allocated_sectors.bounded_iter(1 << 30) {
                sectors.map(|i| i as SectorNumber).collect()
            } else {
                acc.add(&format!("error expanding allocated sector bitfield"));
                BTreeSet::new()
            }
        }
        Ok(None) => {
            acc.add(&format!("error loading allocated sector bitfield"));
            BTreeSet::new()
        }
        Err(e) => {
            acc.add(&format!("error loading allocated sector bitfield: {e}"));
            BTreeSet::new()
        }
    };

    check_precommits(policy, &state, store, &allocated_sectors, &acc);

    let mut all_sectors: BTreeMap<SectorNumber, SectorOnChainInfo> = BTreeMap::new();
    match Sectors::load(&store, &state.sectors) {
        Ok(sectors) => {
            let ret = sectors.amt.for_each(|sector_number, sector| {
                all_sectors.insert(sector_number, sector.clone());
                acc.require(
                    allocated_sectors.contains(&sector_number),
                    &format!(
                        "on chain sector's sector number has not been allocated {sector_number}"
                    ),
                );
                sector.deal_ids.iter().for_each(|&deal| {
                    miner_summary.deals.insert(
                        deal,
                        DealSummary {
                            sector_start: sector.activation,
                            sector_expiration: sector.expiration,
                        },
                    );
                });
                Ok(())
            });

            acc.require_no_error(ret, "error iterating sectors");
        }
        Err(e) => acc.add(&format!("error loading sectors: {e}")),
    };

    // check deadlines
    acc.require(
        state.current_deadline < policy.wpost_period_deadlines,
        &format!(
            "current deadline index is greater than deadlines per period({}): {}",
            policy.wpost_period_deadlines, state.current_deadline
        ),
    );

    match state.load_deadlines(store) {
        Ok(deadlines) => {
            let ret = deadlines.for_each(policy, store, |deadline_index, deadline| {
                let acc = acc.with_prefix(&format!("deadline {deadline_index}: "));
                let quant = state.quant_spec_for_deadline(policy, deadline_index);
                let deadline_summary = check_deadline_state_invariants(
                    &deadline,
                    store,
                    quant,
                    sector_size,
                    &all_sectors,
                    &acc,
                );

                miner_summary.live_power += &deadline_summary.live_power;
                miner_summary.active_power += &deadline_summary.active_power;
                miner_summary.faulty_power += &deadline_summary.faulty_power;
                Ok(())
            });

            acc.require_no_error(ret, "error iterating deadlines");
        }
        Err(e) => {
            acc.add(&format!("error loading deadlines: {e}"));
        }
    };

    (miner_summary, acc)
}

#[allow(dead_code)]
pub fn test_sector(
    expiration: ChainEpoch,
    sector_number: SectorNumber,
    deal_weight: u64,
    verified_deal_weight: u64,
    pledge: u64,
) -> SectorOnChainInfo {
    SectorOnChainInfo {
        expiration,
        sector_number,
        deal_weight: DealWeight::from(deal_weight),
        verified_deal_weight: DealWeight::from(verified_deal_weight),
        initial_pledge: TokenAmount::from(pledge),
        sealed_cid: make_sealed_cid(format!("commR-{sector_number}").as_bytes()),
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn sectors_array<'a, BS: Blockstore>(
    rt: &MockRuntime,
    store: &'a BS,
    sectors_info: Vec<SectorOnChainInfo>,
) -> Sectors<'a, BS> {
    let state: State = rt.get_state();
    let mut sectors = Sectors::load(store, &state.sectors).unwrap();
    sectors.store(sectors_info).unwrap();
    sectors
}

#[allow(dead_code)]
pub fn sectors_arr<'a, BS: Blockstore>(
    store: &'a BS,
    sectors_info: Vec<SectorOnChainInfo>,
) -> Sectors<'a, BS> {
    let empty_array =
        Amt::<SectorOnChainInfo, _>::new_with_bit_width(&store, HAMT_BIT_WIDTH).flush().unwrap();
    let mut sectors = Sectors::load(store, &empty_array).unwrap();
    sectors.store(sectors_info).unwrap();
    sectors
}

#[derive(Default)]
pub struct DeadlineStateSummary {
    pub all_sectors: BitField,
    pub live_sectors: BitField,
    pub faulty_sectors: BitField,
    pub recovering_sectors: BitField,
    pub unproven_sectors: BitField,
    pub terminated_sectors: BitField,
    pub live_power: PowerPair,
    pub active_power: PowerPair,
    pub faulty_power: PowerPair,
}

#[derive(Default)]
pub struct PartitionStateSummary {
    pub all_sectors: BitField,
    pub live_sectors: BitField,
    pub faulty_sectors: BitField,
    pub recovering_sectors: BitField,
    pub unproven_sectors: BitField,
    pub terminated_sectors: BitField,
    pub live_power: PowerPair,
    pub active_power: PowerPair,
    pub faulty_power: PowerPair,
    pub recovering_power: PowerPair,
    // Epochs at which some sector is scheduled to expire.
    pub expiration_epochs: Vec<ChainEpoch>,
    pub early_termination_count: usize,
}

impl PartitionStateSummary {
    pub fn check_partition_state_invariants<BS: Blockstore>(
        partition: &Partition,
        store: &BS,
        quant: QuantSpec,
        sector_size: SectorSize,
        sectors_map: &SectorsMap,
        acc: &MessageAccumulator,
    ) -> Self {
        let live = partition.live_sectors();
        let active = partition.active_sectors();

        // live contains all live sectors
        require_contains_all(&live, &active, acc, "live does not contain active");

        // Live contains all faults.
        require_contains_all(&live, &partition.faults, acc, "live does not contain faults");

        // Live contains all unproven.
        require_contains_all(&live, &partition.unproven, acc, "live does not contain unproven");

        // Active contains no faults
        require_contains_none(&active, &partition.faults, acc, "active includes faults");

        // Active contains no unproven
        require_contains_none(&active, &partition.unproven, acc, "active includes unproven");

        // Faults contains all recoveries.
        require_contains_all(
            &partition.faults,
            &partition.recoveries,
            acc,
            "faults do not contain recoveries",
        );

        // Live contains no terminated sectors
        require_contains_none(&live, &partition.terminated, acc, "live includes terminations");

        // Unproven contains no faults
        require_contains_none(
            &partition.faults,
            &partition.unproven,
            acc,
            "unproven includes faults",
        );

        // All terminated sectors are part of the partition.
        require_contains_all(
            &partition.sectors,
            &partition.terminated,
            acc,
            "sectors do not contain terminations",
        );

        // Validate power
        let mut live_power = PowerPair::zero();
        let mut faulty_power = PowerPair::zero();
        let mut unproven_power = PowerPair::zero();

        let (live_sectors, missing) = select_sectors_map(sectors_map, &live);
        if missing.is_empty() {
            live_power =
                power_for_sectors(sector_size, &live_sectors.values().cloned().collect::<Vec<_>>());
            acc.require(
                partition.live_power == live_power,
                &format!("live power was {:?}, expected {:?}", partition.live_power, live_power),
            );
        } else {
            acc.add(&format!("live sectors missing from all sectors: {missing:?}"));
        }

        let (unproven_sectors, missing) = select_sectors_map(sectors_map, &partition.unproven);
        if missing.is_empty() {
            unproven_power = power_for_sectors(
                sector_size,
                &unproven_sectors.values().cloned().collect::<Vec<_>>(),
            );
            acc.require(
                partition.unproven_power == unproven_power,
                &format!(
                    "unproven power power was {:?}, expected {:?}",
                    partition.unproven_power, unproven_power
                ),
            );
        } else {
            acc.add(&format!("unproven sectors missing from all sectors: {missing:?}"));
        }

        let (faulty_sectors, missing) = select_sectors_map(sectors_map, &partition.faults);
        if missing.is_empty() {
            faulty_power = power_for_sectors(
                sector_size,
                &faulty_sectors.values().cloned().collect::<Vec<_>>(),
            );
            acc.require(
                partition.faulty_power == faulty_power,
                &format!(
                    "faulty power power was {:?}, expected {:?}",
                    partition.faulty_power, faulty_power
                ),
            );
        } else {
            acc.add(&format!("faulty sectors missing from all sectors: {missing:?}"));
        }

        let (recovering_sectors, missing) = select_sectors_map(sectors_map, &partition.recoveries);
        if missing.is_empty() {
            let recovering_power = power_for_sectors(
                sector_size,
                &recovering_sectors.values().cloned().collect::<Vec<_>>(),
            );
            acc.require(
                partition.recovering_power == recovering_power,
                &format!(
                    "recovering power power was {:?}, expected {:?}",
                    partition.recovering_power, recovering_power
                ),
            );
        } else {
            acc.add(&format!("recovering sectors missing from all sectors: {missing:?}"));
        }

        let active_power = &live_power - &faulty_power - unproven_power;
        let partition_active_power = partition.active_power();
        acc.require(
            partition_active_power == active_power,
            &format!("active power was {active_power:?}, expected {:?}", partition_active_power),
        );

        // validate the expiration queue
        let mut expiration_epochs = Vec::new();
        match ExpirationQueue::new(store, &partition.expirations_epochs, quant) {
            Ok(expiration_queue) => {
                let queue_summary = ExpirationQueueStateSummary::check_expiration_queue(
                    &expiration_queue,
                    &live_sectors,
                    &partition.faults,
                    quant,
                    sector_size,
                    acc,
                );

                expiration_epochs = queue_summary.expiration_epochs;
                // check the queue is compatible with partition fields
                let queue_sectors =
                    BitField::union([&queue_summary.on_time_sectors, &queue_summary.early_sectors]);
                require_equal(&live, &queue_sectors, acc, "live does not equal all expirations");
            }
            Err(err) => {
                acc.add(&format!("error loading expiration_queue: {err}"));
            }
        };

        // validate the early termination queue
        let early_termination_count =
            match BitFieldQueue::new(store, &partition.early_terminated, NO_QUANTIZATION) {
                Ok(queue) => check_early_termination_queue(queue, &partition.terminated, acc),
                Err(err) => {
                    acc.add(&format!("error loading early termination queue: {err}"));
                    0
                }
            };

        let partition = partition.clone();
        PartitionStateSummary {
            all_sectors: partition.sectors,
            live_sectors: live,
            faulty_sectors: partition.faults,
            recovering_sectors: partition.recoveries,
            unproven_sectors: partition.unproven,
            terminated_sectors: partition.terminated,
            live_power,
            active_power,
            faulty_power: partition.faulty_power,
            recovering_power: partition.recovering_power,
            expiration_epochs,
            early_termination_count,
        }
    }
}

#[derive(Default)]
struct ExpirationQueueStateSummary {
    on_time_sectors: BitField,
    early_sectors: BitField,
    #[allow(dead_code)]
    active_power: PowerPair,
    #[allow(dead_code)]
    faulty_power: PowerPair,
    #[allow(dead_code)]
    on_time_pledge: TokenAmount,
    expiration_epochs: Vec<ChainEpoch>,
}

impl ExpirationQueueStateSummary {
    // Checks the expiration queue for consistency.
    fn check_expiration_queue<BS: Blockstore>(
        expiration_queue: &ExpirationQueue<BS>,
        live_sectors: &SectorsMap,
        partition_faults: &BitField,
        quant: QuantSpec,
        sector_size: SectorSize,
        acc: &MessageAccumulator,
    ) -> Self {
        let mut seen_sectors: HashSet<SectorNumber> = HashSet::new();
        let mut all_on_time: Vec<BitField> = Vec::new();
        let mut all_early: Vec<BitField> = Vec::new();
        let mut expiration_epochs: Vec<ChainEpoch> = Vec::new();
        let mut all_active_power = PowerPair::zero();
        let mut all_faulty_power = PowerPair::zero();
        let mut all_on_time_pledge = BigInt::zero();

        let ret = expiration_queue.amt.for_each(|epoch, expiration_set| {
            let epoch = epoch as i64;
            let acc = acc.with_prefix(&format!("expiration epoch {epoch}: "));
            let quant_up = quant.quantize_up(epoch);
            acc.require(quant_up == epoch, &format!("expiration queue key {epoch} is not quantized, expected {quant_up}"));

            expiration_epochs.push(epoch);

            let mut on_time_sectors_pledge = BigInt::zero();
            for sector_number in expiration_set.on_time_sectors.iter() {
                // check sectors are present only once
                if !seen_sectors.insert(sector_number) {
                    acc.add(&format!("sector {sector_number} in expiration queue twice"));
                }

                // check expiring sectors are still alive
                if let Some(sector) = live_sectors.get(&sector_number) {
                    let target = quant.quantize_up(sector.expiration);
                    acc.require(epoch == target, &format!("invalid expiration {epoch} for sector {sector_number}, expected {target}"));
                    on_time_sectors_pledge += sector.initial_pledge.clone();
                } else {
                    acc.add(&format!("on time expiration sector {sector_number} isn't live"));
                }
            }

            for sector_number in expiration_set.early_sectors.iter() {
                // check sectors are present only once
                if !seen_sectors.insert(sector_number) {
                    acc.add(&format!("sector {sector_number} in expiration queue twice"));
                }

                // check early sectors are faulty
                acc.require(partition_faults.get(sector_number), &format!("sector {sector_number} expiring early but not faulty"));

                // check expiring sectors are still alive
                if let Some(sector) = live_sectors.get(&sector_number) {
                    let target = quant.quantize_up(sector.expiration);
                    acc.require(epoch < target, &format!("invalid early expiration {epoch} for sector {sector_number}, expected < {target}"));
                } else {
                    acc.add(&format!("on time expiration sector {sector_number} isn't live"));
                }
            }


            // validate power and pledge
            let all = BitField::union([&expiration_set.on_time_sectors, &expiration_set.early_sectors]);
            let all_active = &all - &partition_faults;
            let (active_sectors, missing) = select_sectors_map(live_sectors, &all_active);
            acc.require(missing.is_empty(), &format!("active sectors missing from live: {missing:?}"));

            let all_faulty = &all & &partition_faults;
            let (faulty_sectors, missing) = select_sectors_map(live_sectors, &all_faulty);
            acc.require(missing.is_empty(), &format!("faulty sectors missing from live: {missing:?}"));

            let active_sectors_power = power_for_sectors(sector_size, &active_sectors.values().cloned().collect::<Vec<_>>());
            acc.require(expiration_set.active_power == active_sectors_power, &format!("active power recorded {:?} doesn't match computed {active_sectors_power:?}", expiration_set.active_power));

            let faulty_sectors_power = power_for_sectors(sector_size, &faulty_sectors.values().cloned().collect::<Vec<_>>());
            acc.require(expiration_set.faulty_power == faulty_sectors_power, &format!("faulty power recorded {:?} doesn't match computed {faulty_sectors_power:?}", expiration_set.faulty_power));

            acc.require(expiration_set.on_time_pledge == on_time_sectors_pledge, &format!("on time pledge recorded {} doesn't match computed: {on_time_sectors_pledge}", expiration_set.on_time_pledge));

            all_on_time.push(expiration_set.on_time_sectors.clone());
            all_early.push(expiration_set.early_sectors.clone());
            all_active_power += &expiration_set.active_power;
            all_faulty_power += &expiration_set.faulty_power;
            all_on_time_pledge += &expiration_set.on_time_pledge;

            Ok(())
        });
        acc.require_no_error(ret, "error iterating early termination bitfield");

        let union_on_time = BitField::union(&all_on_time);
        let union_early = BitField::union(&all_early);

        Self {
            on_time_sectors: union_on_time,
            early_sectors: union_early,
            active_power: all_active_power,
            faulty_power: all_faulty_power,
            on_time_pledge: all_on_time_pledge,
            expiration_epochs,
        }
    }
}

// Checks the early termination queue for consistency.
// Returns the number of sectors in the queue.
fn check_early_termination_queue<BS: Blockstore>(
    early_queue: BitFieldQueue<BS>,
    terminated: &BitField,
    acc: &MessageAccumulator,
) -> usize {
    let mut seen: HashSet<u64> = HashSet::new();
    let mut seen_bitfield = BitField::new();

    let iter_result = early_queue.amt.for_each(|epoch, bitfield| {
        let acc = acc.with_prefix(&format!("early termination epoch {epoch}: "));
        for i in bitfield.iter() {
            acc.require(
                !seen.contains(&i),
                &format!("sector {i} in early termination queue twice"),
            );
            seen.insert(i);
            seen_bitfield.set(i);
        }
        Ok(())
    });

    acc.require_no_error(iter_result, "error iterating early termination bitfield");
    require_contains_all(
        terminated,
        &seen_bitfield,
        acc,
        "terminated sectors missing early termination entry",
    );

    seen.len()
}

// Selects a subset of sectors from a map by sector number.
// Returns the selected sectors, and a slice of any sector numbers not found.
fn select_sectors_map(sectors: &SectorsMap, include: &BitField) -> (SectorsMap, Vec<SectorNumber>) {
    let mut included = SectorsMap::new();
    let mut missing = Vec::new();

    for n in include.iter() {
        if let Some(sector) = sectors.get(&n) {
            included.insert(n, sector.clone());
        } else {
            missing.push(n);
        }
    }

    (included, missing)
}

pub fn select_sectors(sectors: &[SectorOnChainInfo], field: &BitField) -> Vec<SectorOnChainInfo> {
    let mut to_include: BTreeSet<_> = field.iter().collect();
    let included =
        sectors.iter().filter(|sector| to_include.remove(&sector.sector_number)).cloned().collect();

    assert!(to_include.is_empty(), "failed to find {} expected sectors", to_include.len());

    included
}

fn require_contains_all(
    superset: &BitField,
    subset: &BitField,
    acc: &MessageAccumulator,
    error_msg: &str,
) {
    if !superset.contains_all(subset) {
        acc.add(&format!("{error_msg}: {subset:?}, {superset:?}"));
    }
}

fn require_contains_none(
    superset: &BitField,
    subset: &BitField,
    acc: &MessageAccumulator,
    error_msg: &str,
) {
    if superset.contains_any(subset) {
        acc.add(&format!("{error_msg}: {subset:?}, {superset:?}"));
    }
}

fn require_equal(first: &BitField, second: &BitField, acc: &MessageAccumulator, msg: &str) {
    require_contains_all(first, second, acc, msg);
    require_contains_all(second, first, acc, msg);
}

#[allow(dead_code)]
pub fn sectors_as_map(sectors: &[SectorOnChainInfo]) -> SectorsMap {
    sectors.iter().map(|sector| (sector.sector_number, sector.to_owned())).collect()
}

#[allow(dead_code)]
pub fn check_deadline_state_invariants<BS: Blockstore>(
    deadline: &Deadline,
    store: &BS,
    quant: QuantSpec,
    sector_size: SectorSize,
    sectors: &SectorsMap,
    acc: &MessageAccumulator,
) -> DeadlineStateSummary {
    // load linked structures
    let partitions = match deadline.partitions_amt(store) {
        Ok(partitions) => partitions,
        Err(e) => {
            // Hard to do any useful checks.
            acc.add(&format!("error loading partitions: {e}"));
            return DeadlineStateSummary::default();
        }
    };

    let mut all_sectors = BitField::new();
    let mut all_live_sectors: Vec<BitField> = Vec::new();
    let mut all_faulty_sectors: Vec<BitField> = Vec::new();
    let mut all_recovering_sectors: Vec<BitField> = Vec::new();
    let mut all_unproven_sectors: Vec<BitField> = Vec::new();
    let mut all_terminated_sectors: Vec<BitField> = Vec::new();
    let mut all_live_power = PowerPair::zero();
    let mut all_active_power = PowerPair::zero();
    let mut all_faulty_power = PowerPair::zero();

    let mut partition_count = 0;

    // check partitions
    let mut partitions_with_expirations: HashMap<ChainEpoch, Vec<u64>> = HashMap::new();
    let mut partitions_with_early_terminations = BitField::new();
    partitions
        .for_each(|index, partition| {
            // check sequential partitions
            acc.require(
                index == partition_count,
                &format!(
                    "Non-sequential partitions, expected index {partition_count}, found {index}"
                ),
            );
            partition_count += 1;

            let acc = acc.with_prefix(&format!("partition {index}"));
            let summary = PartitionStateSummary::check_partition_state_invariants(
                partition,
                store,
                quant,
                sector_size,
                sectors,
                &acc,
            );

            acc.require(
                !all_sectors.contains_any(&summary.all_sectors),
                &format!("duplicate sector in partition {index}"),
            );

            summary.expiration_epochs.iter().for_each(|&epoch| {
                partitions_with_expirations.entry(epoch).or_insert(Vec::new()).push(index);
            });

            if summary.early_termination_count > 0 {
                partitions_with_early_terminations.set(index);
            }

            all_sectors = BitField::union([&all_sectors, &summary.all_sectors]);
            all_live_sectors.push(summary.live_sectors);
            all_faulty_sectors.push(summary.faulty_sectors);
            all_recovering_sectors.push(summary.recovering_sectors);
            all_unproven_sectors.push(summary.unproven_sectors);
            all_terminated_sectors.push(summary.terminated_sectors);
            all_live_power += &summary.live_power;
            all_active_power += &summary.active_power;
            all_faulty_power += &summary.faulty_power;

            Ok(())
        })
        .expect("error iterating partitions");

    // Check invariants on partitions proven
    if let Some(last_proof) = deadline.partitions_posted.last() {
        acc.require(
            partition_count >= last_proof + 1,
            &format!("expected at least {} partitions, found {partition_count}", last_proof + 1),
        );
        acc.require(
            deadline.live_sectors > 0,
            &format!("expected at least one live sector when partitions have been proven"),
        );
    }

    // Check partitions snapshot to make sure we take the snapshot after
    // dealing with recovering power and unproven power.
    match deadline.partitions_snapshot_amt(store) {
        Ok(partition_snapshot) => {
            let ret = partition_snapshot.for_each(|i, partition| {
                let acc = acc.with_prefix(&format!("partition snapshot {i}"));
                acc.require(
                    partition.recovering_power.is_zero(),
                    "snapshot partition has recovering power",
                );
                acc.require(
                    partition.recoveries.is_empty(),
                    "snapshot partition has pending recoveries",
                );
                acc.require(
                    partition.unproven_power.is_zero(),
                    "snapshot partition has unproven power",
                );
                acc.require(
                    partition.unproven.is_empty(),
                    "snapshot partition has unproven sectors",
                );

                Ok(())
            });
            acc.require_no_error(ret, "error iterating partitions snapshot");
        }
        Err(e) => acc.add(&format!("error loading partitions snapshot: {e}")),
    };

    // Check that we don't have any proofs proving partitions that are not in the snapshot.
    match deadline.optimistic_proofs_amt(store) {
        Ok(proofs_snapshot) => {
            if let Ok(partitions_snapshot) = deadline.partitions_snapshot_amt(store) {
                let ret = proofs_snapshot.for_each(|_, proof| {
                    for partition in proof.partitions.iter() {
                        match partitions_snapshot.get(partition) {
                            Ok(snapshot) => acc.require(
                                snapshot.is_some(),
                                "failed to find partition for recorded proof in the snapshot",
                            ),
                            Err(e) => acc.add(&format!("error loading partition snapshot: {e}")),
                        }
                    }
                    Ok(())
                });
                acc.require_no_error(ret, "error iterating proofs snapshot");
            }
        }
        Err(e) => acc.add(&format!("error loading proofs snapshot: {e}")),
    };

    // check memoized sector and power values
    let live_sectors = BitField::union(&all_live_sectors);
    acc.require(
        deadline.live_sectors == live_sectors.len(),
        &format!(
            "deadline live sectors {} != partitions count {}",
            deadline.live_sectors,
            live_sectors.len()
        ),
    );

    acc.require(
        deadline.total_sectors == all_sectors.len(),
        &format!(
            "deadline total sectors {} != partitions count {}",
            deadline.total_sectors,
            all_sectors.len()
        ),
    );

    let faulty_sectors = BitField::union(&all_faulty_sectors);
    let recovering_sectors = BitField::union(&all_recovering_sectors);
    let unproven_sectors = BitField::union(&all_unproven_sectors);
    let terminated_sectors = BitField::union(&all_terminated_sectors);

    acc.require(
        deadline.faulty_power == all_faulty_power,
        &format!(
            "deadline faulty power {:?} != partitions total {all_faulty_power:?}",
            deadline.faulty_power
        ),
    );

    // Validate partition expiration queue contains an entry for each partition and epoch with an expiration.
    // The queue may be a superset of the partitions that have expirations because we never remove from it.
    match BitFieldQueue::new(store, &deadline.expirations_epochs, quant) {
        Ok(expiration_queue) => {
            for (epoch, expiring_idx) in partitions_with_expirations {
                match expiration_queue.amt.get(epoch as u64) {
                    Ok(expiration_bitfield) if expiration_bitfield.is_some() => {
                        for partition in expiring_idx {
                            acc.require(expiration_bitfield.unwrap().get(partition), &format!("expected partition {partition} to be present in deadline expiration queue at epoch {epoch}"));
                        }
                    }
                    Ok(_) => acc.add(&format!(
                        "expected to find partition expiration entry at epoch {epoch}"
                    )),
                    Err(e) => acc.add(&format!("error fetching expiration bitfield: {e}")),
                }
            }
        }
        Err(e) => acc.add(&format!("error loading expiration queue: {e}")),
    }

    // Validate the early termination queue contains exactly the partitions with early terminations.
    require_equal(
        &partitions_with_early_terminations,
        &deadline.early_terminations,
        acc,
        "deadline early terminations doesn't match expected partitions",
    );

    DeadlineStateSummary {
        all_sectors,
        live_sectors,
        faulty_sectors,
        recovering_sectors,
        unproven_sectors,
        terminated_sectors,
        live_power: all_live_power,
        active_power: all_active_power,
        faulty_power: all_faulty_power,
    }
}

// Helper type for validating deadline state.
//
// All methods take the state by value so one can (and should) construct a
// sane base-state.

pub fn new_bitfield(sector_numbers: &[u64]) -> BitField {
    BitField::try_from_bits(sector_numbers.iter().copied()).unwrap()
}

pub struct ExpectedDeadlineState {
    pub quant: QuantSpec,
    #[allow(dead_code)]
    pub sector_size: SectorSize,
    #[allow(dead_code)]
    pub partition_size: u64,
    #[allow(dead_code)]
    pub sectors: Vec<SectorOnChainInfo>,
    pub faults: BitField,
    pub recovering: BitField,
    pub terminations: BitField,
    pub unproven: BitField,
    pub posts: BitField,
    pub partition_sectors: Vec<BitField>,
}

impl Default for ExpectedDeadlineState {
    fn default() -> Self {
        Self {
            quant: QuantSpec { offset: 0, unit: 0 },
            sector_size: SectorSize::_32GiB,
            partition_size: 0,
            sectors: vec![],
            faults: BitField::default(),
            recovering: BitField::default(),
            terminations: BitField::default(),
            unproven: BitField::default(),
            posts: BitField::default(),
            partition_sectors: vec![],
        }
    }
}

impl ExpectedDeadlineState {
    #[allow(dead_code)]
    pub fn with_quant_spec(mut self, quant: QuantSpec) -> Self {
        self.quant = quant;
        self
    }

    pub fn with_faults(mut self, faults: &[u64]) -> Self {
        self.faults = new_bitfield(faults);
        self
    }

    #[allow(dead_code)]
    pub fn with_recovering(mut self, recovering: &[u64]) -> Self {
        self.recovering = new_bitfield(recovering);
        self
    }

    pub fn with_terminations(mut self, terminations: &[u64]) -> Self {
        self.terminations = new_bitfield(terminations);
        self
    }

    pub fn with_unproven(mut self, unproven: &[u64]) -> Self {
        self.unproven = new_bitfield(unproven);
        self
    }

    #[allow(dead_code)]
    pub fn with_posts(mut self, posts: &[u64]) -> Self {
        self.posts = new_bitfield(posts);
        self
    }

    pub fn with_partitions(mut self, partitions: Vec<BitField>) -> Self {
        self.partition_sectors = partitions;
        self
    }

    // Assert that the deadline's state matches the expected state.
    pub fn assert<BS: Blockstore>(
        self,
        store: &BS,
        sectors: &[SectorOnChainInfo],
        deadline: &Deadline,
    ) -> Self {
        let summary = self.check_deadline_invariants(store, sectors, deadline);

        assert_eq!(self.faults, summary.faulty_sectors);
        assert_eq!(self.recovering, summary.recovering_sectors);
        assert_eq!(self.terminations, summary.terminated_sectors);
        assert_eq!(self.unproven, summary.unproven_sectors);
        assert_eq!(self.posts, deadline.partitions_posted);

        let partitions = deadline.partitions_amt(store).unwrap();
        assert_eq!(self.partition_sectors.len() as u64, partitions.count());

        for (i, partition_sectors) in self.partition_sectors.iter().enumerate() {
            let partitions = partitions.get(i as u64).unwrap().unwrap();
            assert_eq!(partition_sectors, &partitions.sectors);
        }

        self
    }

    // check the deadline's invariants, returning all contained sectors, faults,
    // recoveries, terminations, and partition/sector assignments.
    pub fn check_deadline_invariants<BS: Blockstore>(
        &self,
        store: &BS,
        sectors: &[SectorOnChainInfo],
        deadline: &Deadline,
    ) -> DeadlineStateSummary {
        let acc = MessageAccumulator::default();
        let summary = check_deadline_state_invariants(
            deadline,
            store,
            QUANT_SPEC,
            SECTOR_SIZE,
            &sectors_as_map(sectors),
            &acc,
        );

        assert!(acc.is_empty(), "{}", acc.messages().join("\n"));

        summary
    }
}

pub const SECTOR_SIZE: SectorSize = SectorSize::_32GiB;
pub const QUANT_SPEC: QuantSpec = QuantSpec { unit: 4, offset: 1 };

/// Create a bitfield with count bits set, starting at "start".
pub fn seq(start: u64, count: u64) -> BitField {
    let ranges = Ranges::new([start..(start + count)]);
    BitField::from_ranges(ranges)
}

#[derive(Clone, Copy, Default)]
pub struct CronControl {
    pub pre_commit_num: u64,
}

#[allow(dead_code)]
impl CronControl {
    pub fn require_cron_inactive(&self, h: &ActorHarness, rt: &MockRuntime) {
        let st = h.get_state(&rt);
        assert!(!st.deadline_cron_active); // No cron running now
        assert!(!st.continue_deadline_cron()); // No reason to cron now, state inactive
    }

    pub fn require_cron_active(&self, h: &ActorHarness, rt: &MockRuntime) {
        let st = h.get_state(rt);
        assert!(st.deadline_cron_active);
        assert!(st.continue_deadline_cron());
    }

    // Start cron by precommitting at preCommitEpoch, return clean up epoch.
    // Verifies that cron is not started, precommit is run and cron is enrolled.
    // Returns epoch at which precommit is scheduled for clean up and removed from state by cron.
    pub fn pre_commit_to_start_cron(
        &mut self,
        h: &ActorHarness,
        rt: &mut MockRuntime,
        pre_commit_epoch: ChainEpoch,
    ) -> ChainEpoch {
        rt.set_epoch(pre_commit_epoch);
        let st = h.get_state(rt);
        self.require_cron_inactive(h, rt);

        let dlinfo = new_deadline_info_from_offset_and_epoch(
            &rt.policy,
            st.proving_period_start,
            pre_commit_epoch,
        ); // actor.deadline might be out of date
        let sector_no = self.pre_commit_num;
        self.pre_commit_num += 1;
        let expiration =
            dlinfo.period_end() + DEFAULT_SECTOR_EXPIRATION as i64 * rt.policy.wpost_proving_period; // something on deadline boundary but > 180 days
        let precommit_params =
            h.make_pre_commit_params(sector_no, pre_commit_epoch - 1, expiration, vec![]);
        h.pre_commit_sector(rt, precommit_params, PreCommitConfig::default(), true).unwrap();

        // PCD != 0 so cron must be active
        self.require_cron_active(h, rt);

        let clean_up_epoch = pre_commit_epoch
            + max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap()
            + rt.policy.expired_pre_commit_clean_up_delay;
        clean_up_epoch
    }

    // Stop cron by advancing to the preCommit clean up epoch.
    // Assumes no proved sectors, no vesting funds.
    // Verifies cron runs until clean up, PCD burnt and cron discontinued during last deadline
    // Return open of first deadline after expiration.
    fn expire_pre_commit_stop_cron(
        &self,
        h: &ActorHarness,
        rt: &mut MockRuntime,
        start_epoch: ChainEpoch,
        clean_up_epoch: ChainEpoch,
    ) -> ChainEpoch {
        self.require_cron_active(h, rt);
        let st = h.get_state(rt);

        let mut dlinfo = new_deadline_info_from_offset_and_epoch(
            &rt.policy,
            st.proving_period_start,
            start_epoch,
        ); // actor.deadline might be out of date
        while dlinfo.open <= clean_up_epoch {
            // PCDs are quantized to be burnt on the *next* new deadline after the one they are cleaned up in
            // asserts cron is rescheduled
            dlinfo = h.advance_deadline(rt, CronConfig::empty());
        }
        // We expect PCD burnt and cron not rescheduled here.
        rt.set_epoch(dlinfo.last());
        h.on_deadline_cron(
            rt,
            CronConfig {
                no_enrollment: true,
                expired_precommit_penalty: st.pre_commit_deposits,
                ..CronConfig::empty()
            },
        );
        rt.set_epoch(dlinfo.next_open());

        self.require_cron_inactive(h, rt);
        rt.epoch
    }

    pub fn pre_commit_start_cron_expire_stop_cron(
        &mut self,
        h: &ActorHarness,
        rt: &mut MockRuntime,
        start_epoch: ChainEpoch,
    ) -> ChainEpoch {
        let clean_up_epoch = self.pre_commit_to_start_cron(h, rt, start_epoch);
        self.expire_pre_commit_stop_cron(h, rt, start_epoch, clean_up_epoch)
    }
}

#[allow(dead_code)]
pub fn bitfield_from_slice(sector_numbers: &[u64]) -> BitField {
    BitField::try_from_bits(sector_numbers.iter().copied()).unwrap()
}

#[derive(Default, Clone)]
pub struct BitFieldQueueExpectation {
    pub expected: BTreeMap<ChainEpoch, Vec<u64>>,
}

impl BitFieldQueueExpectation {
    #[allow(dead_code)]
    pub fn add(&self, epoch: ChainEpoch, values: &[u64]) -> Self {
        let mut expected = self.expected.clone();
        let _ = expected.insert(epoch, values.to_vec());
        Self { expected }
    }

    #[allow(dead_code)]
    pub fn equals<BS: Blockstore>(&self, queue: &BitFieldQueue<BS>) {
        // ensure cached changes are ready to be iterated

        let length = queue.amt.count();
        assert_eq!(self.expected.len(), length as usize);

        queue
            .amt
            .for_each(|epoch, bf| {
                let values = self
                    .expected
                    .get(&(epoch as i64))
                    .unwrap_or_else(|| panic!("expected entry at epoch {}", epoch));

                assert_bitfield_equals(bf, values);
                Ok(())
            })
            .unwrap();
    }
}

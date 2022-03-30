use fil_actor_account::Method as AccountMethod;
use fil_actor_market::{
    ActivateDealsParams, ComputeDataCommitmentParams, ComputeDataCommitmentReturn,
    Method as MarketMethod, SectorDataSpec, SectorDeals, SectorWeights,
    VerifyDealsForActivationParams, VerifyDealsForActivationReturn,
};
use fil_actor_miner::{
    initial_pledge_for_power, new_deadline_info_from_offset_and_epoch, qa_power_for_weight, Actor,
    ChangeMultiaddrsParams, ChangePeerIDParams, ConfirmSectorProofsParams, CronEventPayload,
    Deadline, DeadlineInfo, Deadlines, DeferredCronEventParams, DisputeWindowedPoStParams,
    GetControlAddressesReturn, Method, MinerConstructorParams as ConstructorParams, Partition,
    PoStPartition, PowerPair, PreCommitSectorParams, ProveCommitSectorParams, SectorOnChainInfo,
    SectorPreCommitOnChainInfo, State, SubmitWindowedPoStParams, VestingFunds, WindowedPoSt,
    CRON_EVENT_PROVING_DEADLINE,
};
use fil_actor_power::{
    CurrentTotalPowerReturn, EnrollCronEventParams, Method as PowerMethod, UpdateClaimedPowerParams,
};
use fil_actor_reward::{Method as RewardMethod, ThisEpochRewardReturn};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    ActorError, Array, DealWeight, BURNT_FUNDS_ACTOR_ADDR, INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
};

use fvm_ipld_bitfield::{BitField, UnvalidatedBitField};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::bigint::BigInt;
use fvm_shared::blockstore::CborStore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::commcid::{FIL_COMMITMENT_SEALED, FIL_COMMITMENT_UNSEALED};
use fvm_shared::crypto::randomness::DomainSeparationTag;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::de::Deserialize;
use fvm_shared::encoding::ser::Serialize;
use fvm_shared::encoding::{BytesDe, RawBytes};
use fvm_shared::error::ExitCode;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{
    PoStProof, RegisteredPoStProof, RegisteredSealProof, SealVerifyInfo, SectorID, SectorInfo,
    SectorNumber, SectorSize, StoragePower, WindowPoStVerifyInfo,
};
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::METHOD_SEND;

use cid::Cid;
use multihash::derive::Multihash;
use multihash::MultihashDigest;

use rand::prelude::*;

use std::collections::HashMap;

const RECEIVER_ID: u64 = 1000;

pub fn new_bls_addr(s: u8) -> Address {
    let seed = [s; 32];
    let mut rng: StdRng = SeedableRng::from_seed(seed);
    let mut key = [0u8; 48];
    rng.fill_bytes(&mut key);
    Address::new_bls(&key).unwrap()
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
        let proof_type = RegisteredSealProof::StackedDRG32GiBV1;

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
        rt.get_state::<State>().unwrap()
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
            TokenAmount::from(0),
            RawBytes::serialize(self.worker_key).unwrap(),
            ExitCode::Ok,
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
        assert_eq!(result.exit_code(), ExitCode::ErrIllegalArgument);
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
        assert_eq!(result.exit_code(), ExitCode::ErrIllegalArgument);
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
        deal_ids: Vec<DealID>, // TODO: this should be Vec<Vec<DealID>>
        first: bool,
    ) -> Vec<SectorOnChainInfo> {
        let precommit_epoch = rt.epoch;
        let deadline = self.get_deadline_info(rt);
        let expiration =
            deadline.period_end() + lifetime_periods as i64 * rt.policy.wpost_proving_period;

        let mut precommits = Vec::with_capacity(num_sectors);
        for i in 0..num_sectors {
            let sector_no = self.next_sector_no;
            let mut sector_deal_ids = vec![];
            if !deal_ids.is_empty() {
                sector_deal_ids.push(deal_ids[i]);
            }
            let params = self.make_pre_commit_params(
                sector_no,
                precommit_epoch - 1,
                expiration,
                sector_deal_ids,
            );
            let precommit =
                self.pre_commit_sector(rt, params, PreCommitConfig::empty(), first && i == 0);
            precommits.push(precommit);
            self.next_sector_no += 1;
        }

        self.advance_to_epoch_with_cron(
            rt,
            precommit_epoch + rt.policy.pre_commit_challenge_delay + 1,
        );

        let mut info = Vec::with_capacity(num_sectors);
        for pc in precommits {
            let sector = self.prove_commit_sector_and_confirm(
                rt,
                &pc,
                self.make_prove_commit_params(pc.info.sector_number),
                ProveCommitConfig::empty(),
            );
            info.push(sector);
        }
        rt.reset();
        info
    }

    pub fn get_deadline_info(&self, rt: &MockRuntime) -> DeadlineInfo {
        let state = self.get_state(rt);
        state.recorded_deadline_info(&rt.policy, rt.epoch)
    }

    fn make_pre_commit_params(
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

    fn make_prove_commit_params(&self, sector_no: u64) -> ProveCommitSectorParams {
        ProveCommitSectorParams { sector_number: sector_no, proof: vec![0u8; 192] }
    }

    fn pre_commit_sector(
        &self,
        rt: &mut MockRuntime,
        params: PreCommitSectorParams,
        conf: PreCommitConfig,
        first: bool,
    ) -> SectorPreCommitOnChainInfo {
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
                    deal_space: conf.deal_space.unwrap() as u64,
                    deal_weight: conf.deal_weight,
                    verified_deal_weight: conf.verified_deal_weight,
                }],
            };

            rt.expect_send(
                *STORAGE_MARKET_ACTOR_ADDR,
                MarketMethod::VerifyDealsForActivation as u64,
                RawBytes::serialize(vdparams).unwrap(),
                TokenAmount::from(0),
                RawBytes::serialize(vdreturn).unwrap(),
                ExitCode::Ok,
            );
        }
        // in the original test the else branch does some redundant checks which we can omit.

        let state = self.get_state(rt);
        if state.fee_debt > TokenAmount::from(0) {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                state.fee_debt.clone(),
                RawBytes::default(),
                ExitCode::Ok,
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
                TokenAmount::from(0),
                RawBytes::default(),
                ExitCode::Ok,
            );
        }

        let result = rt
            .call::<Actor>(
                Method::PreCommitSector as u64,
                &RawBytes::serialize(params.clone()).unwrap(),
            )
            .unwrap();
        expect_empty(result);
        rt.verify();

        self.get_precommit(rt, params.sector_number)
    }

    fn get_precommit(
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
            TokenAmount::from(0),
            RawBytes::serialize(current_reward).unwrap(),
            ExitCode::Ok,
        );
        rt.expect_send(
            *STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::CurrentTotalPower as u64,
            RawBytes::default(),
            TokenAmount::from(0),
            RawBytes::serialize(current_power).unwrap(),
            ExitCode::Ok,
        );
    }

    fn prove_commit_sector_and_confirm(
        &self,
        rt: &mut MockRuntime,
        pc: &SectorPreCommitOnChainInfo,
        params: ProveCommitSectorParams,
        cfg: ProveCommitConfig,
    ) -> SectorOnChainInfo {
        let sector_number = params.sector_number;
        self.prove_commit_sector(rt, pc, params);
        self.confirm_sector_proofs_valid(rt, cfg, vec![pc.clone()]);

        self.get_sector(rt, sector_number)
    }

    fn prove_commit_sector(
        &self,
        rt: &mut MockRuntime,
        pc: &SectorPreCommitOnChainInfo,
        params: ProveCommitSectorParams,
    ) {
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
            TokenAmount::from(0),
            RawBytes::serialize(cdc_ret).unwrap(),
            ExitCode::Ok,
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
            TokenAmount::from(0),
            RawBytes::default(),
            ExitCode::Ok,
        );
        rt.expect_validate_caller_any();
        let result = rt
            .call::<Actor>(Method::ProveCommitSector as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        expect_empty(result);
        rt.verify();
    }

    fn confirm_sector_proofs_valid(
        &self,
        rt: &mut MockRuntime,
        cfg: ProveCommitConfig,
        pcs: Vec<SectorPreCommitOnChainInfo>,
    ) {
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
        )
        .unwrap();
        rt.verify();
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

                let mut exit = ExitCode::Ok;
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
                    TokenAmount::from(0),
                    RawBytes::default(),
                    exit,
                );
            } else {
                valid_pcs.push(pc);
            }
        }

        if !valid_pcs.is_empty() {
            let mut expected_pledge = TokenAmount::from(0);
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

            if expected_pledge != TokenAmount::from(0) {
                rt.expect_send(
                    *STORAGE_POWER_ACTOR_ADDR,
                    PowerMethod::UpdatePledgeTotal as u64,
                    RawBytes::serialize(BigIntSer(&expected_pledge)).unwrap(),
                    TokenAmount::from(0),
                    RawBytes::default(),
                    ExitCode::Ok,
                );
            }
        }
    }

    fn get_sector(&self, rt: &MockRuntime, sector_number: SectorNumber) -> SectorOnChainInfo {
        let state = self.get_state(rt);
        state.get_sector(&rt.store, sector_number).unwrap().unwrap()
    }

    fn advance_to_epoch_with_cron(&self, rt: &mut MockRuntime, epoch: ChainEpoch) {
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

    fn deadline(&self, rt: &MockRuntime) -> DeadlineInfo {
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

    fn on_deadline_cron(&self, rt: &mut MockRuntime, cfg: CronConfig) {
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
                TokenAmount::from(0),
                RawBytes::default(),
                ExitCode::Ok,
            );
        }

        let mut penalty_total = TokenAmount::from(0);
        let mut pledge_delta = TokenAmount::from(0);

        penalty_total += cfg.continued_faults_penalty.clone();
        penalty_total += cfg.repaid_fee_debt.clone();
        penalty_total += cfg.expired_precommit_penalty.clone();

        if penalty_total != TokenAmount::from(0) {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                penalty_total.clone(),
                RawBytes::default(),
                ExitCode::Ok,
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

        if pledge_delta != TokenAmount::from(0) {
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::UpdatePledgeTotal as u64,
                RawBytes::serialize(BigIntSer(&pledge_delta)).unwrap(),
                TokenAmount::from(0),
                RawBytes::default(),
                ExitCode::Ok,
            );
        }

        // Re-enrollment for next period.
        if !cfg.no_enrollment {
            let params = make_deadline_cron_event_params(cfg.expected_enrollment);
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::EnrollCronEvent as u64,
                RawBytes::serialize(params).unwrap(),
                TokenAmount::from(0),
                RawBytes::default(),
                ExitCode::Ok,
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
                    None => ExitCode::Ok,
                };
                rt.expect_verify_post(vi, exit_code);
            }
        }

        if cfg.expected_power_delta.is_some() {
            let power_delta = cfg.expected_power_delta.unwrap();
            let claim = UpdateClaimedPowerParams {
                raw_byte_delta: power_delta.raw,
                quality_adjusted_delta: power_delta.qa,
            };
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::UpdateClaimedPower as u64,
                RawBytes::serialize(claim).unwrap(),
                TokenAmount::from(0),
                RawBytes::default(),
                ExitCode::Ok,
            );
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
            Some(_) => ExitCode::ErrIllegalArgument,
            None => ExitCode::Ok,
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
                    TokenAmount::from(0),
                    RawBytes::default(),
                    ExitCode::Ok,
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
                    ExitCode::Ok,
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
                    ExitCode::Ok,
                );
            }

            if dispute_result.expected_pledge_delta.is_some() {
                let expected_pledge_delta = dispute_result.expected_pledge_delta.unwrap();
                rt.expect_send(
                    *STORAGE_POWER_ACTOR_ADDR,
                    PowerMethod::UpdatePledgeTotal as u64,
                    RawBytes::serialize(BigIntSer(&expected_pledge_delta)).unwrap(),
                    TokenAmount::from(0),
                    RawBytes::default(),
                    ExitCode::Ok,
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
                ExitCode::ErrIllegalArgument,
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

pub struct PreCommitConfig {
    deal_weight: DealWeight,
    verified_deal_weight: DealWeight,
    deal_space: Option<SectorSize>,
}

#[allow(dead_code)]
impl PreCommitConfig {
    pub fn empty() -> PreCommitConfig {
        PreCommitConfig {
            deal_weight: DealWeight::from(0),
            verified_deal_weight: DealWeight::from(0),
            deal_space: None,
        }
    }
}

pub struct ProveCommitConfig {
    verify_deals_exit: HashMap<SectorNumber, ExitCode>,
}

#[allow(dead_code)]
impl ProveCommitConfig {
    pub fn empty() -> ProveCommitConfig {
        ProveCommitConfig { verify_deals_exit: HashMap::new() }
    }
}

pub struct CronConfig {
    no_enrollment: bool, // true if expect not to continue enrollment false otherwise
    expected_enrollment: ChainEpoch,
    detected_faults_power_delta: Option<PowerPair>,
    expired_sectors_power_delta: Option<PowerPair>,
    expired_sectors_pledge_delta: TokenAmount,
    continued_faults_penalty: TokenAmount, // Expected amount burnt to pay continued fault penalties.
    expired_precommit_penalty: TokenAmount, // Expected amount burnt to pay for expired precommits
    repaid_fee_debt: TokenAmount,          // Expected amount burnt to repay fee debt.
    penalty_from_unlocked: TokenAmount, // Expected reduction in unlocked balance from penalties exceeding vesting funds.
}

#[allow(dead_code)]
impl CronConfig {
    pub fn empty() -> CronConfig {
        CronConfig {
            no_enrollment: false,
            expected_enrollment: 0,
            detected_faults_power_delta: None,
            expired_sectors_power_delta: None,
            expired_sectors_pledge_delta: TokenAmount::from(0),
            continued_faults_penalty: TokenAmount::from(0),
            expired_precommit_penalty: TokenAmount::from(0),
            repaid_fee_debt: TokenAmount::from(0),
            penalty_from_unlocked: TokenAmount::from(0),
        }
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

fn get_bitfield(ubf: &UnvalidatedBitField) -> BitField {
    match ubf {
        UnvalidatedBitField::Validated(bf) => bf.clone(),
        UnvalidatedBitField::Unvalidated(bytes) => BitField::from_bytes(bytes).unwrap(),
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
    let mut sum = TokenAmount::from(0);
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
            *item = ((offset >> (7 - i)) & 0xff) as u8;
        }
        result
    };
    Box::new(hash)
}

#[allow(dead_code)]
pub fn check_state_invariants(_rt: &MockRuntime) {
    // TODO check state invariants
}

use std::cell::RefCell;

use cid::Cid;
use fil_actor_power::detail::GAS_ON_SUBMIT_VERIFY_SEAL;
use fil_actor_power::ext::miner::ConfirmSectorProofsParams;
use fil_actor_power::ext::miner::CONFIRM_SECTOR_PROOFS_VALID_METHOD;
use fil_actor_power::ext::reward::Method::ThisEpochReward;
use fil_actor_power::ext::reward::UPDATE_NETWORK_KPI;
use fil_actor_power::testing::check_state_invariants;
use fil_actor_power::EnrollCronEventParams;
use fil_actor_power::CRON_QUEUE_AMT_BITWIDTH;
use fil_actor_power::CRON_QUEUE_HAMT_BITWIDTH;
use fil_actor_power::{epoch_key, MinerCountReturn};
use fil_actor_power::{CronEvent, MinerConsensusCountReturn};
use fil_actors_runtime::runtime::RuntimePolicy;
use fil_actors_runtime::test_utils::CRON_ACTOR_CODE_ID;
use fil_actors_runtime::Multimap;
use fil_actors_runtime::CRON_ACTOR_ADDR;
use fil_actors_runtime::REWARD_ACTOR_ADDR;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_ipld_hamt::BytesKey;
use fvm_ipld_hamt::Error;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::reward::ThisEpochRewardReturn;
use fvm_shared::sector::SealVerifyInfo;
use fvm_shared::sector::SectorNumber;
use fvm_shared::sector::{RegisteredPoStProof, RegisteredSealProof, StoragePower};
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::MethodNum;
use lazy_static::lazy_static;
use num_traits::Zero;
use serde::de::DeserializeOwned;
use serde::Serialize;

use fil_actor_power::ext::init::ExecParams;
use fil_actor_power::ext::miner::MinerConstructorParams;
use fil_actor_power::{
    ext, Claim, CreateMinerParams, CreateMinerReturn, CurrentTotalPowerReturn, Method, State,
    UpdateClaimedPowerParams,
};
use fil_actors_runtime::builtin::HAMT_BIT_WIDTH;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::{
    MockRuntime, ACCOUNT_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID, SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth, ActorError, Map, INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;

use crate::PowerActor;

lazy_static! {
    pub static ref OWNER: Address = Address::new_id(101);
    pub static ref MINER: Address = Address::new_id(201);
    pub static ref ACTOR: Address = Address::new_actor("actor".as_bytes());
}

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: STORAGE_POWER_ACTOR_ADDR,
        caller: RefCell::new(SYSTEM_ACTOR_ADDR),
        caller_type: RefCell::new(*SYSTEM_ACTOR_CODE_ID),
        ..Default::default()
    }
}

pub fn new_harness() -> Harness {
    let rwd = TokenAmount::from_whole(10);
    Harness {
        miner_seq: 0,
        seal_proof: RegisteredSealProof::StackedDRG32GiBV1P1,
        window_post_proof: RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
        this_epoch_baseline_power: StoragePower::from(1i64 << 50),
        this_epoch_reward_smoothed: FilterEstimate::new(rwd.atto().clone(), BigInt::zero()),
    }
}

pub fn setup() -> (Harness, MockRuntime) {
    let rt = new_runtime();
    let h = new_harness();
    h.construct(&rt);
    (h, rt)
}

#[allow(dead_code)]
pub struct Harness {
    miner_seq: i64,
    seal_proof: RegisteredSealProof,
    pub window_post_proof: RegisteredPoStProof,
    this_epoch_baseline_power: StoragePower,
    pub this_epoch_reward_smoothed: FilterEstimate,
}

impl Harness {
    pub fn construct(&self, rt: &MockRuntime) {
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        rt.call::<PowerActor>(Method::Constructor as MethodNum, None).unwrap();
        rt.verify()
    }

    pub fn construct_and_verify(&self, rt: &MockRuntime) {
        self.construct(rt);
        let st: State = rt.get_state();
        assert_eq!(StoragePower::zero(), st.total_raw_byte_power);
        assert_eq!(StoragePower::zero(), st.total_bytes_committed);
        assert_eq!(StoragePower::zero(), st.total_quality_adj_power);
        assert_eq!(StoragePower::zero(), st.total_qa_bytes_committed);
        assert_eq!(TokenAmount::zero(), st.total_pledge_collateral);
        assert_eq!(StoragePower::zero(), st.total_raw_byte_power);
        assert_eq!(StoragePower::zero(), st.this_epoch_quality_adj_power);
        assert_eq!(TokenAmount::zero(), st.this_epoch_pledge_collateral);
        assert_eq!(ChainEpoch::zero(), st.first_cron_epoch);
        assert_eq!(0, st.miner_count);
        assert_eq!(0, st.miner_above_min_power_count);

        verify_empty_map(rt, st.claims);
        verify_empty_map(rt, st.cron_event_queue);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_miner(
        &self,
        rt: &MockRuntime,
        owner: &Address,
        worker: &Address,
        miner: &Address,
        robust: &Address,
        peer: Vec<u8>,
        multiaddrs: Vec<BytesDe>,
        window_post_proof_type: RegisteredPoStProof,
        value: &TokenAmount,
    ) -> Result<(), ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *owner);
        rt.set_received(value.clone());
        rt.set_balance(value.clone());
        rt.expect_validate_caller_any();

        let miner_ctor_params = MinerConstructorParams {
            owner: *owner,
            worker: *worker,
            control_addresses: vec![],
            window_post_proof_type,
            peer_id: peer.clone(),
            multi_addresses: multiaddrs.clone(),
        };
        let expected_init_params = ExecParams {
            code_cid: *MINER_ACTOR_CODE_ID,
            constructor_params: RawBytes::serialize(miner_ctor_params).unwrap(),
        };
        let create_miner_ret = CreateMinerReturn { id_address: *miner, robust_address: *robust };
        rt.expect_send_simple(
            INIT_ACTOR_ADDR,
            ext::init::EXEC_METHOD,
            IpldBlock::serialize_cbor(&expected_init_params).unwrap(),
            value.clone(),
            IpldBlock::serialize_cbor(&create_miner_ret).unwrap(),
            ExitCode::OK,
        );
        let params = CreateMinerParams {
            owner: *owner,
            worker: *worker,
            window_post_proof_type,
            peer,
            multiaddrs,
        };
        rt.call::<PowerActor>(
            Method::CreateMiner as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )?;
        Ok(())
    }

    pub fn create_miner_basic(
        &mut self,
        rt: &MockRuntime,
        owner: Address,
        worker: Address,
        miner: Address,
    ) -> Result<(), ActorError> {
        let label = format!("{}", self.miner_seq);
        let actr_addr = Address::new_actor(label.as_bytes());
        self.miner_seq += 1;
        let peer = label.as_bytes().to_vec();
        self.create_miner(
            rt,
            &owner,
            &worker,
            &miner,
            &actr_addr,
            peer,
            vec![],
            self.window_post_proof,
            &TokenAmount::zero(),
        )
    }

    pub fn list_miners(&self, rt: &MockRuntime) -> Vec<Address> {
        let st: State = rt.get_state();
        let claims: Map<_, Claim> =
            make_map_with_root_and_bitwidth(&st.claims, rt.store(), HAMT_BIT_WIDTH).unwrap();
        let keys = collect_keys(claims).unwrap();
        keys.iter().map(|k| Address::from_bytes(k).unwrap()).collect::<Vec<_>>()
    }

    pub fn miner_count(&self, rt: &MockRuntime) -> i64 {
        rt.expect_validate_caller_any();
        let ret: MinerCountReturn = rt
            .call::<PowerActor>(Method::MinerCountExported as MethodNum, None)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();

        ret.miner_count
    }

    pub fn this_epoch_baseline_power(&self) -> &StoragePower {
        &self.this_epoch_baseline_power
    }

    pub fn get_claim(&self, rt: &MockRuntime, miner: &Address) -> Option<Claim> {
        let st: State = rt.get_state();
        st.get_claim(rt.store(), miner).unwrap()
    }

    pub fn delete_claim(&mut self, rt: &MockRuntime, miner: &Address) {
        let mut state: State = rt.get_state();

        let mut claims =
            make_map_with_root_and_bitwidth::<_, Claim>(&state.claims, rt.store(), HAMT_BIT_WIDTH)
                .unwrap();
        claims.delete(&miner.to_bytes()).expect("Failed to delete claim");
        state.claims = claims.flush().unwrap();

        rt.replace_state(&state);
    }

    pub fn enroll_cron_event(
        &self,
        rt: &MockRuntime,
        epoch: ChainEpoch,
        miner_address: &Address,
        payload: &RawBytes,
    ) -> Result<(), ActorError> {
        rt.set_caller(*MINER_ACTOR_CODE_ID, miner_address.to_owned());
        rt.expect_validate_caller_type(vec![Type::Miner]);
        let params = IpldBlock::serialize_cbor(&EnrollCronEventParams {
            event_epoch: epoch,
            payload: payload.clone(),
        })
        .unwrap();
        rt.call::<PowerActor>(Method::EnrollCronEvent as u64, params)?;
        rt.verify();
        Ok(())
    }

    pub fn get_enrolled_cron_ticks(&self, rt: &MockRuntime, epoch: ChainEpoch) -> Vec<CronEvent> {
        let state: State = rt.get_state();
        let events_map = Multimap::from_root(
            &rt.store,
            &state.cron_event_queue,
            CRON_QUEUE_HAMT_BITWIDTH,
            CRON_QUEUE_AMT_BITWIDTH,
        )
        .expect("failed to load cron events");

        let mut events: Vec<CronEvent> = Vec::new();
        events_map
            .for_each::<_, CronEvent>(&epoch_key(epoch), |_, v| {
                events.push(v.to_owned());
                Ok(())
            })
            .unwrap();

        events
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        let (_, acc) = check_state_invariants(rt.policy(), &rt.get_state::<State>(), rt.store());
        acc.assert_empty();
    }

    pub fn update_pledge_total(&self, rt: &MockRuntime, miner: Address, delta: &TokenAmount) {
        let st: State = rt.get_state();
        let prev = st.total_pledge_collateral;

        rt.set_caller(*MINER_ACTOR_CODE_ID, miner);
        rt.expect_validate_caller_type(vec![Type::Miner]);
        rt.call::<PowerActor>(
            Method::UpdatePledgeTotal as MethodNum,
            IpldBlock::serialize_cbor(&delta).unwrap(),
        )
        .unwrap();
        rt.verify();

        let st: State = rt.get_state();
        assert_eq!(prev + delta, st.total_pledge_collateral);
    }

    pub fn current_power_total(&self, rt: &MockRuntime) -> CurrentTotalPowerReturn {
        rt.expect_validate_caller_any();
        let ret: CurrentTotalPowerReturn = rt
            .call::<PowerActor>(Method::CurrentTotalPower as u64, None)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();
        rt.verify();
        ret
    }

    pub fn update_claimed_power(
        &self,
        rt: &MockRuntime,
        miner: Address,
        raw_delta: &StoragePower,
        qa_delta: &StoragePower,
    ) {
        let prev_cl = self.get_claim(rt, &miner).unwrap();

        let params = UpdateClaimedPowerParams {
            raw_byte_delta: raw_delta.clone(),
            quality_adjusted_delta: qa_delta.clone(),
        };
        rt.set_caller(*MINER_ACTOR_CODE_ID, miner);
        rt.expect_validate_caller_type(vec![Type::Miner]);
        rt.call::<PowerActor>(
            Method::UpdateClaimedPower as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap();
        rt.verify();

        let cl = self.get_claim(rt, &miner).unwrap();
        let expected_raw = &prev_cl.raw_byte_power + raw_delta;
        let expected_adjusted = &prev_cl.quality_adj_power + qa_delta;
        if expected_raw.is_zero() {
            assert!(cl.raw_byte_power.is_zero());
        } else {
            assert_eq!(prev_cl.raw_byte_power + raw_delta, cl.raw_byte_power);
        }

        if expected_adjusted.is_zero() {
            assert!(cl.quality_adj_power.is_zero());
        } else {
            assert_eq!(prev_cl.quality_adj_power + qa_delta, cl.quality_adj_power);
        }
    }

    pub fn expect_total_power_eager(
        &self,
        rt: &MockRuntime,
        expected_raw: &StoragePower,
        expected_qa: &StoragePower,
    ) {
        let st: State = rt.get_state();

        let (raw_byte_power, quality_adj_power) = st.current_total_power();
        assert_eq!(expected_raw, &raw_byte_power);
        assert_eq!(expected_qa, &quality_adj_power);
    }

    pub fn expect_total_pledge_eager(&self, rt: &MockRuntime, expected_pledge: &TokenAmount) {
        let st: State = rt.get_state();
        assert_eq!(expected_pledge, &st.total_pledge_collateral);
    }

    pub fn expect_miners_above_min_power(&self, rt: &MockRuntime, count: i64) {
        rt.expect_validate_caller_any();
        let ret: MinerConsensusCountReturn = rt
            .call::<PowerActor>(Method::MinerConsensusCountExported as MethodNum, None)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();

        assert_eq!(count, ret.miner_consensus_count);
    }

    pub fn expect_query_network_info(&self, rt: &MockRuntime) {
        let current_reward = ThisEpochRewardReturn {
            this_epoch_baseline_power: self.this_epoch_baseline_power.clone(),
            this_epoch_reward_smoothed: self.this_epoch_reward_smoothed.clone(),
        };

        rt.expect_send_simple(
            REWARD_ACTOR_ADDR,
            ThisEpochReward as u64,
            None,
            TokenAmount::zero(),
            IpldBlock::serialize_cbor(&current_reward).unwrap(),
            ExitCode::OK,
        );
    }

    pub fn on_epoch_tick_end(
        &self,
        rt: &MockRuntime,
        current_epoch: ChainEpoch,
        expected_raw_power: &StoragePower,
        confirmed_sectors: Vec<ConfirmedSectorSend>,
        infos: Vec<SealVerifyInfo>,
    ) {
        self.expect_query_network_info(rt);

        let state: State = rt.get_state();

        //expect sends for confirmed sectors
        for sector in confirmed_sectors {
            let param = ConfirmSectorProofsParams {
                sectors: sector.sector_nums,
                reward_smoothed: self.this_epoch_reward_smoothed.clone(),
                reward_baseline_power: self.this_epoch_baseline_power.clone(),
                quality_adj_power_smoothed: state.this_epoch_qa_power_smoothed.clone(),
            };
            rt.expect_send_simple(
                sector.miner,
                CONFIRM_SECTOR_PROOFS_VALID_METHOD,
                IpldBlock::serialize_cbor(&param).unwrap(),
                TokenAmount::zero(),
                None,
                ExitCode::new(0),
            );
        }

        let verified_seals = batch_verify_default_output(&infos);
        rt.expect_batch_verify_seals(infos, anyhow::Ok(verified_seals));

        // expect power sends to reward actor
        rt.expect_send_simple(
            REWARD_ACTOR_ADDR,
            UPDATE_NETWORK_KPI,
            IpldBlock::serialize_cbor(&BigIntSer(expected_raw_power)).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::new(0),
        );
        rt.expect_validate_caller_addr(vec![CRON_ACTOR_ADDR]);

        rt.set_epoch(current_epoch);
        rt.set_caller(*CRON_ACTOR_CODE_ID, CRON_ACTOR_ADDR);

        rt.call::<PowerActor>(Method::OnEpochTickEnd as u64, None).unwrap();

        rt.verify();
        let state: State = rt.get_state();
        assert!(state.proof_validation_batch.is_none());
    }

    pub fn submit_porep_for_bulk_verify(
        &self,
        rt: &MockRuntime,
        miner_address: Address,
        seal_info: SealVerifyInfo,
        expect_success: bool,
    ) -> Result<(), ActorError> {
        if expect_success {
            rt.expect_gas_charge(GAS_ON_SUBMIT_VERIFY_SEAL);
        }
        rt.expect_validate_caller_type(vec![Type::Miner]);
        rt.set_caller(*MINER_ACTOR_CODE_ID, miner_address);
        rt.call::<PowerActor>(
            Method::SubmitPoRepForBulkVerify as u64,
            IpldBlock::serialize_cbor(&seal_info).unwrap(),
        )?;
        rt.verify();
        Ok(())
    }
}

pub struct ConfirmedSectorSend {
    pub miner: Address,
    pub sector_nums: Vec<SectorNumber>,
}

pub fn batch_verify_default_output(infos: &[SealVerifyInfo]) -> Vec<bool> {
    vec![true; infos.len()]
}

/// Collects all keys from a map into a vector.
fn collect_keys<BS, V>(m: Map<BS, V>) -> Result<Vec<BytesKey>, Error>
where
    BS: Blockstore,
    V: DeserializeOwned + Serialize,
{
    let mut ret_keys = Vec::new();
    m.for_each(|k, _| {
        ret_keys.push(k.clone());
        Ok(())
    })?;

    Ok(ret_keys)
}

pub fn verify_empty_map(rt: &MockRuntime, key: Cid) {
    let map =
        make_map_with_root_and_bitwidth::<_, BigIntDe>(&key, &rt.store, HAMT_BIT_WIDTH).unwrap();
    map.for_each(|_key, _val| panic!("expected no keys")).unwrap();
}

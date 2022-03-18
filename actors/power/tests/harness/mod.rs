use std::borrow::Borrow;

use cid::Cid;
use fvm_ipld_hamt::BytesKey;
use fvm_ipld_hamt::Error;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::{BytesDe, RawBytes};
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredPoStProof, RegisteredSealProof, StoragePower};
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::MethodNum;
use lazy_static::lazy_static;
use num_traits::Zero;
use serde::de::DeserializeOwned;
use serde::Serialize;

use fil_actor_power::ext::init::ExecParams;
use fil_actor_power::ext::miner::MinerConstructorParams;
use fil_actor_power::{ext, Claim, CreateMinerParams, CreateMinerReturn, Method, State};
use fil_actors_runtime::builtin::HAMT_BIT_WIDTH;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::{
    MockRuntime, ACCOUNT_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID, MULTISIG_ACTOR_CODE_ID,
    SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth, ActorError, Map, INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};

use crate::PowerActor;

lazy_static! {
    pub static ref OWNER: Address = Address::new_id(101);
    pub static ref MINER: Address = Address::new_id(201);
    pub static ref ACTOR: Address = Address::new_actor("actor".as_bytes());
}

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: *STORAGE_POWER_ACTOR_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

pub fn new_harness() -> Harness {
    let rwd = TokenAmount::from(10) * TokenAmount::from(10_i128.pow(18));
    Harness {
        miner_seq: 0,
        seal_proof: RegisteredSealProof::StackedDRG32GiBV1P1,
        window_post_proof: RegisteredPoStProof::StackedDRGWindow32GiBV1,
        this_epoch_baseline_power: StoragePower::from(1i64 << 50),
        this_epoch_reward_smoothed: FilterEstimate::new(rwd, TokenAmount::zero()),
    }
}

pub fn setup() -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = new_harness();
    h.construct(&mut rt);
    return (h, rt);
}

pub struct Harness {
    miner_seq: i64,
    seal_proof: RegisteredSealProof,
    window_post_proof: RegisteredPoStProof,
    this_epoch_baseline_power: StoragePower,
    this_epoch_reward_smoothed: FilterEstimate,
}

impl Harness {
    pub fn construct(&self, rt: &mut MockRuntime) {
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        rt.call::<PowerActor>(Method::Constructor as MethodNum, &RawBytes::default()).unwrap();
        rt.verify()
    }

    pub fn construct_and_verify(&self, rt: &mut MockRuntime) {
        self.construct(rt);
        let st: State = rt.get_state().unwrap();
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

    pub fn create_miner(
        &self,
        rt: &mut MockRuntime,
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
        rt.set_value(value.clone());
        rt.set_balance(value.clone());
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

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
        rt.expect_send(
            *INIT_ACTOR_ADDR,
            ext::init::EXEC_METHOD,
            RawBytes::serialize(expected_init_params).unwrap(),
            value.clone(),
            RawBytes::serialize(create_miner_ret).unwrap(),
            ExitCode::Ok,
        );
        let params = CreateMinerParams {
            owner: *owner,
            worker: *worker,
            window_post_proof_type,
            peer: peer.clone(),
            multiaddrs: multiaddrs.clone(),
        };
        rt.call::<PowerActor>(
            Method::CreateMiner as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        Ok(())
    }

    pub fn list_miners(&self, rt: &MockRuntime) -> Vec<Address> {
        let st: State = rt.get_state().unwrap();
        let claims: Map<_, Claim> =
            make_map_with_root_and_bitwidth(&st.claims, rt.store(), HAMT_BIT_WIDTH).unwrap();
        let keys = collect_keys(claims).unwrap();
        keys.iter().map(|k| Address::from_bytes(k).unwrap()).collect::<Vec<_>>()
    }

    pub fn get_claim(&self, rt: &MockRuntime, miner: &Address) -> Option<Claim> {
        let st: State = rt.get_state().unwrap();
        let claims =
            make_map_with_root_and_bitwidth(&st.claims, rt.store(), HAMT_BIT_WIDTH).unwrap();
        claims.get(&miner.to_bytes()).unwrap().cloned()
    }

    pub fn check_state(&self) {
        // TODO: https://github.com/filecoin-project/builtin-actors/issues/44
    }
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
    map.for_each(|key, val| panic!("expected no keys")).unwrap();
}

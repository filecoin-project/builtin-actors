use cid::Cid;
use fil_actors_runtime::test_utils::expect_abort;
use fil_actors_runtime::Array;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;
use lazy_static::lazy_static;

use fil_actors_runtime::builtin::HAMT_BIT_WIDTH;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::{MockRuntime, SUBNET_ACTOR_CODE_ID, SYSTEM_ACTOR_CODE_ID};
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth, ActorError, BURNT_FUNDS_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use hierarchical_sca::{
    new_id, Checkpoint, ConstructorParams, FundParams, Method, State, Subnet, SubnetID,
    SubnetIDParam, CROSSMSG_AMT_BITWIDTH, DEFAULT_CHECKPOINT_PERIOD, MAX_NONCE,
    MIN_COLLATERAL_AMOUNT, ROOTNET_ID,
};

use crate::SCAActor;

lazy_static! {
    pub static ref SUBNET_ONE: Address = Address::new_id(101);
    pub static ref SUBNET_TWO: Address = Address::new_id(202);
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
    Harness { net_name: ROOTNET_ID.clone() }
}

pub fn setup() -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = new_harness();
    h.construct(&mut rt);
    (h, rt)
}

#[allow(dead_code)]
pub struct Harness {
    pub net_name: SubnetID,
}

impl Harness {
    pub fn construct(&self, rt: &mut MockRuntime) {
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        let params =
            ConstructorParams { network_name: self.net_name.to_string(), checkpoint_period: 10 };
        rt.call::<SCAActor>(
            Method::Constructor as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
        rt.verify()
    }

    pub fn construct_and_verify(&self, rt: &mut MockRuntime) {
        self.construct(rt);
        let st: State = rt.get_state();
        let store = &rt.store;

        let empty_bottomup_array =
            Array::<(), _>::new_with_bit_width(store, CROSSMSG_AMT_BITWIDTH).flush().unwrap();

        assert_eq!(st.network_name, self.net_name);
        assert_eq!(st.min_stake, TokenAmount::from(MIN_COLLATERAL_AMOUNT));
        assert_eq!(st.check_period, DEFAULT_CHECKPOINT_PERIOD);
        assert_eq!(st.applied_bottomup_nonce, MAX_NONCE);
        assert_eq!(st.bottomup_msg_meta, empty_bottomup_array);
        verify_empty_map(rt, st.subnets);
        verify_empty_map(rt, st.checkpoints);
        verify_empty_map(rt, st.check_msg_registry);
        verify_empty_map(rt, st.atomic_exec_registry);
    }

    pub fn register(
        &self,
        rt: &mut MockRuntime,
        subnet_addr: &Address,
        value: &TokenAmount,
        code: ExitCode,
    ) -> Result<(), ActorError> {
        rt.set_caller(*SUBNET_ACTOR_CODE_ID, *subnet_addr);
        rt.set_value(value.clone());
        rt.set_balance(value.clone());
        rt.expect_validate_caller_type(vec![*SUBNET_ACTOR_CODE_ID]);

        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(Method::Register as MethodNum, &RawBytes::default()),
            );
            rt.verify();
            return Ok(());
        }

        let register_ret = SubnetIDParam { id: new_id(&ROOTNET_ID, *subnet_addr).to_string() };
        let ret = rt.call::<SCAActor>(Method::Register as MethodNum, &RawBytes::default()).unwrap();
        rt.verify();
        let ret: SubnetIDParam = RawBytes::deserialize(&ret).unwrap();
        assert_eq!(ret.id, register_ret.id);
        Ok(())
    }

    pub fn add_stake(
        &self,
        rt: &mut MockRuntime,
        id: &SubnetID,
        value: &TokenAmount,
        code: ExitCode,
    ) -> Result<(), ActorError> {
        rt.set_caller(*SUBNET_ACTOR_CODE_ID, id.subnet_actor());
        rt.set_value(value.clone());
        rt.expect_validate_caller_type(vec![*SUBNET_ACTOR_CODE_ID]);

        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(Method::AddStake as MethodNum, &RawBytes::default()),
            );
            rt.verify();
            return Ok(());
        }

        rt.call::<SCAActor>(Method::AddStake as MethodNum, &RawBytes::default()).unwrap();
        rt.verify();

        Ok(())
    }

    pub fn release_stake(
        &self,
        rt: &mut MockRuntime,
        id: &SubnetID,
        value: &TokenAmount,
        code: ExitCode,
    ) -> Result<(), ActorError> {
        rt.set_caller(*SUBNET_ACTOR_CODE_ID, id.subnet_actor());
        rt.expect_validate_caller_type(vec![*SUBNET_ACTOR_CODE_ID]);
        let params = FundParams { value: value.clone() };

        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(
                    Method::ReleaseStake as MethodNum,
                    &RawBytes::serialize(params).unwrap(),
                ),
            );
            rt.verify();
            return Ok(());
        }

        rt.expect_send(
            id.subnet_actor(),
            METHOD_SEND,
            RawBytes::default(),
            value.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.call::<SCAActor>(
            Method::ReleaseStake as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
        rt.verify();

        Ok(())
    }

    pub fn kill(
        &self,
        rt: &mut MockRuntime,
        id: &SubnetID,
        release_value: &TokenAmount,
        code: ExitCode,
    ) -> Result<(), ActorError> {
        rt.set_caller(*SUBNET_ACTOR_CODE_ID, id.subnet_actor());
        rt.expect_validate_caller_type(vec![*SUBNET_ACTOR_CODE_ID]);

        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(Method::Kill as MethodNum, &RawBytes::default()),
            );
            rt.verify();
            return Ok(());
        }

        rt.expect_send(
            id.subnet_actor(),
            METHOD_SEND,
            RawBytes::default(),
            release_value.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.call::<SCAActor>(Method::Kill as MethodNum, &RawBytes::default()).unwrap();
        rt.verify();

        Ok(())
    }

    pub fn commit_child_check(
        &self,
        rt: &mut MockRuntime,
        id: &SubnetID,
        ch: Checkpoint,
        code: ExitCode,
        burn_value: TokenAmount,
    ) -> Result<(), ActorError> {
        rt.set_caller(*SUBNET_ACTOR_CODE_ID, id.subnet_actor());
        rt.expect_validate_caller_type(vec![*SUBNET_ACTOR_CODE_ID]);

        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(
                    Method::CommitChildCheckpoint as MethodNum,
                    &RawBytes::serialize(ch).unwrap(),
                ),
            );
            rt.verify();
            return Ok(());
        }

        if burn_value > TokenAmount::zero() {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                burn_value.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }
        rt.call::<SCAActor>(
            Method::CommitChildCheckpoint as MethodNum,
            &RawBytes::serialize(ch).unwrap(),
        )
        .unwrap();
        rt.verify();

        Ok(())
    }

    pub fn check_state(&self) {
        // TODO: https://github.com/filecoin-project/builtin-actors/issues/44
    }

    pub fn get_subnet(&self, rt: &MockRuntime, id: &SubnetID) -> Option<Subnet> {
        let st: State = rt.get_state();
        let subnets =
            make_map_with_root_and_bitwidth(&st.subnets, rt.store(), HAMT_BIT_WIDTH).unwrap();
        subnets.get(&id.to_bytes()).unwrap().cloned()
    }
}

pub fn verify_empty_map(rt: &MockRuntime, key: Cid) {
    let map =
        make_map_with_root_and_bitwidth::<_, BigIntDe>(&key, &rt.store, HAMT_BIT_WIDTH).unwrap();
    map.for_each(|_key, _val| panic!("expected no keys")).unwrap();
}

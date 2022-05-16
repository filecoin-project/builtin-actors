use anyhow::anyhow;
use cid::multihash::Code;
use cid::multihash::MultihashDigest;
use cid::Cid;
use fil_actors_runtime::test_utils::expect_abort;
use fil_actors_runtime::Array;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::subnet::ROOTNET_ID;
use fvm_shared::address::{Address, SubnetID};
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;
use lazy_static::lazy_static;

use fil_actors_runtime::builtin::HAMT_BIT_WIDTH;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::{
    MockRuntime, ACCOUNT_ACTOR_CODE_ID, MULTISIG_ACTOR_CODE_ID, SUBNET_ACTOR_CODE_ID,
    SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth, ActorError, Map, BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR,
    SCA_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use hierarchical_sca::checkpoint::ChildCheck;
use hierarchical_sca::ext;
use hierarchical_sca::{
    get_topdown_msg, is_bottomup, Checkpoint, ConstructorParams, CrossMsgArray, CrossMsgMeta,
    CrossMsgParams, CrossMsgs, FundParams, HCMsgType, Method, State, StorableMsg, Subnet,
    CROSSMSG_AMT_BITWIDTH, DEFAULT_CHECKPOINT_PERIOD, MAX_NONCE, MIN_COLLATERAL_AMOUNT,
};

use crate::SCAActor;

lazy_static! {
    pub static ref SUBNET_ONE: Address = Address::new_id(101);
    pub static ref SUBNET_TWO: Address = Address::new_id(102);
    pub static ref TEST_BLS: Address =
        Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap();
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

pub fn new_harness(id: SubnetID) -> Harness {
    Harness { net_name: id }
}

pub fn setup_root() -> (Harness, MockRuntime) {
    setup(ROOTNET_ID.clone())
}

pub fn setup(id: SubnetID) -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = new_harness(id);
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

        let register_ret = SubnetID::new(&self.net_name, *subnet_addr);
        let ret = rt.call::<SCAActor>(Method::Register as MethodNum, &RawBytes::default()).unwrap();
        rt.verify();
        let ret: SubnetID = RawBytes::deserialize(&ret).unwrap();
        assert_eq!(ret, register_ret);
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
        ch: &Checkpoint,
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

    pub fn fund(
        &self,
        rt: &mut MockRuntime,
        funder: &Address,
        id: &SubnetID,
        code: ExitCode,
        value: TokenAmount,
        expected_nonce: u64,
        expected_circ_sup: &TokenAmount,
    ) -> Result<(), ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *funder);
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

        rt.set_value(value.clone());
        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(
                    Method::Fund as MethodNum,
                    &RawBytes::serialize(id.clone()).unwrap(),
                ),
            );
            rt.verify();
            return Ok(());
        }

        rt.expect_send(
            *funder,
            ext::account::PUBKEY_ADDRESS_METHOD,
            RawBytes::default(),
            TokenAmount::zero(),
            RawBytes::serialize(*TEST_BLS).unwrap(),
            ExitCode::OK,
        );
        rt.call::<SCAActor>(Method::Fund as MethodNum, &RawBytes::serialize(id.clone()).unwrap())
            .unwrap();
        rt.verify();

        let sub = self.get_subnet(rt, id).unwrap();
        let crossmsgs = CrossMsgArray::load(&sub.top_down_msgs, rt.store()).unwrap();
        let msg = get_topdown_msg(&crossmsgs, expected_nonce - 1).unwrap().unwrap();
        assert_eq!(&sub.circ_supply, expected_circ_sup);
        assert_eq!(sub.nonce, expected_nonce);
        let from = Address::new_hierarchical(&self.net_name, &TEST_BLS).unwrap();
        let to = Address::new_hierarchical(&id, &TEST_BLS).unwrap();
        assert_eq!(msg.from, from);
        assert_eq!(msg.to, to);
        assert_eq!(msg.nonce, expected_nonce - 1);
        assert_eq!(msg.value, value);

        Ok(())
    }

    pub fn release(
        &self,
        rt: &mut MockRuntime,
        releaser: &Address,
        code: ExitCode,
        value: TokenAmount,
        expected_nonce: u64,
        prev_meta: &Cid,
    ) -> Result<Cid, ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *releaser);
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

        rt.set_value(value.clone());
        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(Method::Release as MethodNum, &RawBytes::default()),
            );
            rt.verify();
            return Ok(Cid::default());
        }

        rt.expect_send(
            *releaser,
            ext::account::PUBKEY_ADDRESS_METHOD,
            RawBytes::default(),
            TokenAmount::zero(),
            RawBytes::serialize(*TEST_BLS).unwrap(),
            ExitCode::OK,
        );
        rt.expect_send(
            *BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            RawBytes::default(),
            value.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.call::<SCAActor>(Method::Release as MethodNum, &RawBytes::default()).unwrap();
        rt.verify();

        let st: State = rt.get_state();

        let parent = &self.net_name.parent().unwrap();
        let from = Address::new_hierarchical(&self.net_name, &BURNT_FUNDS_ACTOR_ADDR).unwrap();
        let to = Address::new_hierarchical(&parent, &TEST_BLS).unwrap();
        rt.set_epoch(0);
        let ch = st.get_window_checkpoint(rt.store(), 0).unwrap();
        let chmeta_ind = ch.crossmsg_meta_index(&self.net_name, &parent).unwrap();
        let chmeta = &ch.data.cross_msgs[chmeta_ind];

        let cross_reg = make_map_with_root_and_bitwidth::<_, CrossMsgs>(
            &st.check_msg_registry,
            rt.store(),
            HAMT_BIT_WIDTH,
        )
        .unwrap();
        let meta = get_cross_msgs(&cross_reg, &chmeta.msgs_cid).unwrap().unwrap();
        let msg = meta.msgs[expected_nonce as usize].clone();

        assert_eq!(meta.msgs.len(), (expected_nonce + 1) as usize);
        assert_eq!(msg.from, from);
        assert_eq!(msg.to, to);
        assert_eq!(msg.nonce, expected_nonce);
        assert_eq!(msg.value, value);

        if prev_meta != &Cid::default() {
            match get_cross_msgs(&cross_reg, &prev_meta).unwrap() {
                Some(_) => panic!("previous meta should have been removed"),
                None => {}
            }
        }

        Ok(chmeta.msgs_cid)
    }

    pub fn send_cross(
        &self,
        rt: &mut MockRuntime,
        from: &Address,
        to: &Address,
        sub: SubnetID,
        code: ExitCode,
        value: TokenAmount,
        nonce: u64,
        expected_circ_sup: &TokenAmount,
    ) -> Result<(), ActorError> {
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *from);
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

        rt.set_value(value.clone());

        let msg = StorableMsg {
            from: from.clone(),
            to: to.clone(),
            nonce: nonce,
            method: METHOD_SEND,
            params: RawBytes::default(),
            value: value.clone(),
        };
        let dest = sub.clone();
        let params = CrossMsgParams { destination: sub, msg: msg };
        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(
                    Method::SendCross as MethodNum,
                    &RawBytes::serialize(params).unwrap(),
                ),
            );
            rt.verify();
            return Ok(());
        }

        rt.expect_send(
            *from,
            ext::account::PUBKEY_ADDRESS_METHOD,
            RawBytes::default(),
            TokenAmount::zero(),
            RawBytes::serialize(*TEST_BLS).unwrap(),
            ExitCode::OK,
        );

        let is_bu = is_bottomup(&self.net_name, &dest);
        if is_bu {
            rt.expect_send(
                *BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                RawBytes::default(),
                value.clone(),
                RawBytes::default(),
                ExitCode::OK,
            );
        }
        rt.call::<SCAActor>(Method::SendCross as MethodNum, &RawBytes::serialize(params).unwrap())
            .unwrap();
        rt.verify();

        let st: State = rt.get_state();
        if is_bu {
            let from = Address::new_hierarchical(&self.net_name, &TEST_BLS).unwrap();
            let to = Address::new_hierarchical(&dest, &to).unwrap();
            rt.set_epoch(0);
            let ch = st.get_window_checkpoint(rt.store(), 0).unwrap();
            let chmeta_ind = ch.crossmsg_meta_index(&self.net_name, &dest).unwrap();
            let chmeta = &ch.data.cross_msgs[chmeta_ind];

            let cross_reg = make_map_with_root_and_bitwidth::<_, CrossMsgs>(
                &st.check_msg_registry,
                rt.store(),
                HAMT_BIT_WIDTH,
            )
            .unwrap();
            let meta = get_cross_msgs(&cross_reg, &chmeta.msgs_cid).unwrap().unwrap();
            let msg = meta.msgs[nonce as usize].clone();

            assert_eq!(meta.msgs.len(), (nonce + 1) as usize);
            assert_eq!(msg.from, from);
            assert_eq!(msg.to, to);
            assert_eq!(msg.nonce, nonce);
            assert_eq!(msg.value, value);
        } else {
            // top-down
            let sub = self.get_subnet(rt, &dest.down(&self.net_name).unwrap()).unwrap();
            let crossmsgs = CrossMsgArray::load(&sub.top_down_msgs, rt.store()).unwrap();
            let msg = get_topdown_msg(&crossmsgs, nonce - 1).unwrap().unwrap();
            assert_eq!(&sub.circ_supply, expected_circ_sup);
            assert_eq!(sub.nonce, nonce);
            let from = Address::new_hierarchical(&self.net_name, &TEST_BLS).unwrap();
            let to = Address::new_hierarchical(&dest, &to).unwrap();
            assert_eq!(msg.from, from);
            assert_eq!(msg.to, to);
            assert_eq!(msg.nonce, nonce - 1);
            assert_eq!(msg.value, value);
        }

        Ok(())
    }

    pub fn apply_cross_msg(
        &self,
        rt: &mut MockRuntime,
        from: &Address,
        to: &Address,
        value: TokenAmount,
        msg_nonce: u64,
        td_nonce: u64,
        code: ExitCode,
        noop: bool,
    ) -> Result<(), ActorError> {
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, *SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);

        rt.set_balance(value.clone());
        let params = StorableMsg {
            to: to.clone(),
            from: from.clone(),
            method: METHOD_SEND,
            value: value.clone(),
            params: RawBytes::default(),
            nonce: msg_nonce,
        };

        let st: State = rt.get_state();
        let sto = params.to.subnet().unwrap();
        let rto = to.raw_addr().unwrap();

        // if expected code is not ok
        if code != ExitCode::OK {
            expect_abort(
                code,
                rt.call::<SCAActor>(
                    Method::ApplyMessage as MethodNum,
                    &RawBytes::serialize(params).unwrap(),
                ),
            );
            rt.verify();
            return Ok(());
        }

        if params.apply_type(&st.network_name).unwrap() == HCMsgType::BottomUp {
            if sto == st.network_name {
                rt.expect_send(
                    rto,
                    METHOD_SEND,
                    RawBytes::default(),
                    params.value.clone(),
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }

            rt.call::<SCAActor>(
                Method::ApplyMessage as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            )?;
            rt.verify();
            let st: State = rt.get_state();
            assert_eq!(st.applied_bottomup_nonce, msg_nonce);
        } else {
            let rew_params =
                ext::reward::FundingParams { addr: *SCA_ACTOR_ADDR, value: params.value.clone() };
            rt.expect_send(
                *REWARD_ACTOR_ADDR,
                ext::reward::EXTERNAL_FUNDING_METHOD,
                RawBytes::serialize(rew_params).unwrap(),
                TokenAmount::zero(),
                RawBytes::default(),
                ExitCode::OK,
            );
            if sto == st.network_name {
                rt.expect_send(
                    rto,
                    METHOD_SEND,
                    RawBytes::default(),
                    params.value.clone(),
                    RawBytes::default(),
                    ExitCode::OK,
                );
            }
            rt.call::<SCAActor>(
                Method::ApplyMessage as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            )?;
            rt.verify();
            let st: State = rt.get_state();
            assert_eq!(st.applied_topdown_nonce, msg_nonce + 1);

            if sto != st.network_name {
                let sub = self.get_subnet(rt, &sto.down(&self.net_name).unwrap()).unwrap();
                let crossmsgs = CrossMsgArray::load(&sub.top_down_msgs, rt.store()).unwrap();
                let msg = get_topdown_msg(&crossmsgs, td_nonce).unwrap().unwrap();
                assert_eq!(&msg.from, from);
                assert_eq!(&msg.to, to);
                assert_eq!(msg.nonce, td_nonce);
                assert_eq!(msg.value, value);
            }
        }

        if noop {
            panic!("TODO: Not implemented yet");
        }
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

pub fn has_childcheck_source<'a>(
    children: &'a Vec<ChildCheck>,
    source: &SubnetID,
) -> Option<&'a ChildCheck> {
    children.iter().find(|m| source == &m.source)
}

pub fn has_cid<'a>(children: &'a Vec<Cid>, cid: &Cid) -> bool {
    children.iter().any(|c| c == cid)
}

pub fn add_msg_meta(
    ch: &mut Checkpoint,
    from: &SubnetID,
    to: &SubnetID,
    rand: Vec<u8>,
    value: TokenAmount,
) {
    let mh_code = Code::Blake2b256;
    let c = Cid::new_v1(fvm_ipld_encoding::DAG_CBOR, mh_code.digest(&rand));
    let meta =
        CrossMsgMeta { from: from.clone(), to: to.clone(), msgs_cid: c, nonce: 0, value: value };
    ch.append_msgmeta(meta).unwrap();
}

fn get_cross_msgs<'m, BS: Blockstore>(
    registry: &'m Map<BS, CrossMsgs>,
    cid: &Cid,
) -> anyhow::Result<Option<&'m CrossMsgs>> {
    registry.get(&cid.to_bytes()).map_err(|e| anyhow!("error getting fross messages: {}", e))
}

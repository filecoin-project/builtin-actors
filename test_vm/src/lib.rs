use cid::multihash::Code;
use cid::Cid;
use fil_actor_account::{Actor as AccountActor, State as AccountState};
use fil_actor_cron::{Actor as CronActor, Entry as CronEntry, State as CronState};
use fil_actor_init::{Actor as InitActor, ExecReturn, State as InitState};
use fil_actor_market::{Actor as MarketActor, Method as MarketMethod, State as MarketState};
use fil_actor_miner::Actor as MinerActor;
use fil_actor_multisig::Actor as MultisigActor;
use fil_actor_paych::Actor as PaychActor;
use fil_actor_power::{Actor as PowerActor, Method as MethodPower, State as PowerState};
use fil_actor_reward::{Actor as RewardActor, State as RewardState};
use fil_actor_system::{Actor as SystemActor, State as SystemState};
use fil_actor_verifreg::{Actor as VerifregActor, State as VerifRegState};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::{
    ActorCode, MessageInfo, Policy, Primitives, Runtime, RuntimePolicy, Verifier,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    ActorError, BURNT_FUNDS_ACTOR_ADDR, FIRST_NON_SINGLETON_ADDR, INIT_ACTOR_ADDR,
    REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{Cbor, CborStore, RawBytes};
use fvm_ipld_hamt::{BytesKey, Hamt, Sha256};
use fvm_shared::actor::builtin::Type;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::{bigint_ser, BigInt, Zero};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::consensus::ConsensusFault;
use fvm_shared::crypto::randomness::DomainSeparationTag;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PieceInfo;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{
    AggregateSealVerifyProofAndInfos, RegisteredSealProof, ReplicaUpdateInfo, SealVerifyInfo,
    StoragePower, WindowPoStVerifyInfo,
};
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use num_traits::Signed;
use serde::ser;
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::ops::Add;

pub struct VM<'bs> {
    store: &'bs MemoryBlockstore,
    state_root: RefCell<Cid>,
    actors_dirty: RefCell<bool>,
    actors_cache: RefCell<HashMap<Address, Actor>>,
    empty_obj_cid: Cid,
    network_version: NetworkVersion,
    curr_epoch: ChainEpoch,
    invocations: RefCell<Vec<InvocationTrace>>,
}

pub const VERIFREG_ROOT_KEY: &[u8] = &[200; fvm_shared::address::BLS_PUB_LEN];
// Account actor seeding funds created by new_with_singletons
pub const FAUCET_ROOT_KEY: &[u8] = &[153; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_FAUCET_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 2);
pub const FIRST_TEST_USER_ADDR: ActorID = FIRST_NON_SINGLETON_ADDR + 3; // accounts for verifreg root signer and msig
impl<'bs> VM<'bs> {
    pub fn new(store: &'bs MemoryBlockstore) -> VM<'bs> {
        let mut actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::new(store);
        let empty = store.put_cbor(&(), Code::Blake2b256).unwrap();
        VM {
            store,
            state_root: RefCell::new(actors.flush().unwrap()),
            actors_dirty: RefCell::new(false),
            actors_cache: RefCell::new(HashMap::new()),
            empty_obj_cid: empty,
            network_version: NetworkVersion::V16,
            curr_epoch: ChainEpoch::zero(),
            invocations: RefCell::new(vec![]),
        }
    }

    pub fn new_with_singletons(store: &'bs MemoryBlockstore) -> VM<'bs> {
        // funding
        let fil = TokenAmount::from(1_000_000_000i32)
            .checked_mul(&TokenAmount::from(1_000_000_000i32))
            .unwrap();
        let reward_total = TokenAmount::from(1_100_000_000i32).checked_mul(&fil).unwrap();
        let faucet_total = TokenAmount::from(1_000_000_000u32).checked_mul(&fil).unwrap();

        let v = VM::new(store);

        // system
        let sys_st = SystemState::new(store).unwrap();
        let sys_head = v.put_store(&sys_st);
        let sys_value = faucet_total.clone(); // delegate faucet funds to system so we can construct faucet by sending to bls addr
        v.set_actor(*SYSTEM_ACTOR_ADDR, actor(*SYSTEM_ACTOR_CODE_ID, sys_head, 0, sys_value));

        // init
        let init_st = InitState::new(store, "integration-test".to_string()).unwrap();
        let init_head = v.put_store(&init_st);
        v.set_actor(
            *INIT_ACTOR_ADDR,
            actor(*INIT_ACTOR_CODE_ID, init_head, 0, TokenAmount::zero()),
        );

        // reward

        let reward_head = v.put_store(&RewardState::new(StoragePower::zero()));
        v.set_actor(*REWARD_ACTOR_ADDR, actor(*REWARD_ACTOR_CODE_ID, reward_head, 0, reward_total));

        // cron
        let builtin_entries = vec![
            CronEntry {
                receiver: *STORAGE_POWER_ACTOR_ADDR,
                method_num: MethodPower::OnEpochTickEnd as u64,
            },
            CronEntry {
                receiver: *STORAGE_MARKET_ACTOR_ADDR,
                method_num: MarketMethod::CronTick as u64,
            },
        ];
        let cron_head = v.put_store(&CronState { entries: builtin_entries });
        v.set_actor(
            *STORAGE_MARKET_ACTOR_ADDR,
            actor(*MARKET_ACTOR_CODE_ID, cron_head, 0, TokenAmount::zero()),
        );

        // power
        let power_head = v.put_store(&PowerState::new(&v.store).unwrap());
        v.set_actor(
            *STORAGE_POWER_ACTOR_ADDR,
            actor(*POWER_ACTOR_CODE_ID, power_head, 0, TokenAmount::zero()),
        );

        // market
        let market_head = v.put_store(&MarketState::new(&v.store).unwrap());
        v.set_actor(
            *STORAGE_MARKET_ACTOR_ADDR,
            actor(*MARKET_ACTOR_CODE_ID, market_head, 0, TokenAmount::zero()),
        );

        // verifreg
        // initialize verifreg root signer
        v.apply_message(
            *INIT_ACTOR_ADDR,
            Address::new_bls(VERIFREG_ROOT_KEY).unwrap(),
            TokenAmount::zero(),
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap();
        let verifreg_root_signer =
            v.normalize_address(&Address::new_bls(VERIFREG_ROOT_KEY).unwrap()).unwrap();
        // verifreg root msig
        let msig_ctor_params = serialize(
            &fil_actor_multisig::ConstructorParams {
                signers: vec![verifreg_root_signer],
                num_approvals_threshold: 1,
                unlock_duration: 0,
                start_epoch: 0,
            },
            "multisig ctor params",
        )
        .unwrap();
        let msig_ctor_ret: ExecReturn = v
            .apply_message(
                *SYSTEM_ACTOR_ADDR,
                *INIT_ACTOR_ADDR,
                BigInt::zero(),
                fil_actor_init::Method::Exec as u64,
                fil_actor_init::ExecParams {
                    code_cid: *MULTISIG_ACTOR_CODE_ID,
                    constructor_params: msig_ctor_params,
                },
            )
            .unwrap()
            .ret
            .deserialize()
            .unwrap();
        let root_msig_addr = msig_ctor_ret.id_address;
        // verifreg
        let verifreg_head = v.put_store(&VerifRegState::new(&v.store, root_msig_addr).unwrap());
        v.set_actor(
            *VERIFIED_REGISTRY_ACTOR_ADDR,
            actor(*VERIFREG_ACTOR_CODE_ID, verifreg_head, 0, TokenAmount::zero()),
        );

        // burnt funds
        let burnt_funds_head = v.put_store(&AccountState { address: *BURNT_FUNDS_ACTOR_ADDR });
        v.set_actor(
            *BURNT_FUNDS_ACTOR_ADDR,
            actor(*ACCOUNT_ACTOR_CODE_ID, burnt_funds_head, 0, TokenAmount::zero()),
        );

        // create a faucet with 1 billion FIL for setting up test accounts
        v.apply_message(
            *SYSTEM_ACTOR_ADDR,
            Address::new_bls(FAUCET_ROOT_KEY).unwrap(),
            faucet_total,
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap();

        v.checkpoint();
        v
    }

    pub fn put_store<S>(&self, obj: &S) -> Cid
    where
        S: ser::Serialize,
    {
        self.store.put_cbor(obj, Code::Blake2b256).unwrap()
    }

    pub fn get_actor(&self, addr: Address) -> Option<Actor> {
        // check for inclusion in cache of changed actors
        if let Some(act) = self.actors_cache.borrow().get(&addr) {
            return Some(act.clone());
        }

        // go to persisted map
        let actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::load(
            &self.state_root.borrow(),
            self.store,
        )
        .unwrap();
        actors.get(&addr.to_bytes()).unwrap().cloned()
    }

    // blindly overwrite the actor at this address whether it previously existed or not
    pub fn set_actor(&self, key: Address, a: Actor) {
        self.actors_cache.borrow_mut().insert(key, a);
        self.actors_dirty.replace(true);
    }

    pub fn checkpoint(&self) -> Cid {
        // persist cache on top of latest checkpoint and clear
        let mut actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::load(
            &self.state_root.borrow(),
            self.store,
        )
        .unwrap();
        for (addr, act) in self.actors_cache.borrow().iter() {
            actors.set(addr.to_bytes().into(), act.clone()).unwrap();
        }

        // roll "back" to latest head, flushing cache
        self.rollback(actors.flush().unwrap());

        *self.state_root.borrow()
    }

    pub fn rollback(&self, root: Cid) {
        self.actors_cache.replace(HashMap::new());
        self.state_root.replace(root);
        self.actors_dirty.replace(false);
    }

    pub fn normalize_address(&self, addr: &Address) -> Option<Address> {
        let st = self.get_state::<InitState>(*INIT_ACTOR_ADDR).unwrap();
        st.resolve_address::<MemoryBlockstore>(self.store, addr).unwrap()
    }

    pub fn get_state<C: Cbor>(&self, addr: Address) -> Option<C> {
        let a_opt = self.get_actor(addr);
        if a_opt == None {
            return None;
        };
        let a = a_opt.unwrap();
        self.store.get_cbor::<C>(&a.head).unwrap()
    }

    pub fn apply_message<C: Cbor>(
        &self,
        from: Address,
        to: Address,
        value: TokenAmount,
        method: MethodNum,
        params: C,
    ) -> Result<MessageResult, TestVMError> {
        let from_id = self.normalize_address(&from).unwrap();
        let mut a = self.get_actor(from_id).unwrap();
        let call_seq = a.call_seq_num;
        a.call_seq_num = call_seq + 1;
        self.set_actor(from_id, a);

        let prior_root = self.checkpoint();

        // make top level context with internal context
        let top = TopCtx {
            originator_stable_addr: from,
            _originator_call_seq: call_seq,
            new_actor_addr_count: RefCell::new(0),
            _circ_supply: BigInt::zero(),
        };
        let msg = InternalMessage {
            from: from_id,
            to,
            value,
            method,
            params: serialize(&params, "params for apply message").unwrap(),
        };
        let mut new_ctx = InvocationCtx {
            v: self,
            top,
            msg,
            allow_side_effects: true,
            caller_validated: false,
            policy: &Policy::default(),
            subinvocations: RefCell::new(vec![]),
        };
        let res = new_ctx.invoke();
        let invoc = new_ctx.gather_trace(res.clone());
        RefMut::map(self.invocations.borrow_mut(), |invocs| {
            invocs.push(invoc);
            invocs
        });
        match res {
            Err(ae) => {
                self.rollback(prior_root);
                Ok(MessageResult { code: ae.exit_code(), ret: RawBytes::default() })
            }
            Ok(ret) => {
                self.checkpoint();
                Ok(MessageResult { code: ExitCode::OK, ret })
            }
        }
    }

    pub fn take_invocations(&self) -> Vec<InvocationTrace> {
        self.invocations.take()
    }
}
#[derive(Clone)]
pub struct TopCtx {
    originator_stable_addr: Address,
    _originator_call_seq: u64,
    new_actor_addr_count: RefCell<u64>,
    _circ_supply: BigInt,
}

#[derive(Clone)]
pub struct InternalMessage {
    from: Address,
    to: Address,
    value: TokenAmount,
    method: MethodNum,
    params: RawBytes,
}

impl MessageInfo for InternalMessage {
    fn caller(&self) -> Address {
        self.from
    }
    fn receiver(&self) -> Address {
        self.to
    }
    fn value_received(&self) -> TokenAmount {
        self.value.clone()
    }
}

pub const TEST_VM_RAND_STRING: &str = "i_am_random_____i_am_random_____";

pub struct InvocationCtx<'invocation, 'bs> {
    v: &'invocation VM<'bs>,
    top: TopCtx,
    msg: InternalMessage,
    allow_side_effects: bool,
    caller_validated: bool,
    policy: &'invocation Policy,
    subinvocations: RefCell<Vec<InvocationTrace>>,
}

impl<'invocation, 'bs> InvocationCtx<'invocation, 'bs> {
    fn resolve_target(&'invocation self, target: &Address) -> Result<(Actor, Address), ActorError> {
        if let Some(a) = self.v.normalize_address(target) {
            if let Some(act) = self.v.get_actor(a) {
                return Ok((act, a));
            }
        };
        // Address does not yet exist, create it
        match target.protocol() {
            Protocol::Actor | Protocol::ID => {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_INVALID_RECEIVER,
                    "cannot create account for address type".to_string(),
                ))
            }
            _ => (),
        }
        let mut st = self.v.get_state::<InitState>(*INIT_ACTOR_ADDR).unwrap();
        let target_id = st.map_address_to_new_id(self.v.store, target).unwrap();
        let target_id_addr = Address::new_id(target_id);
        let mut init_actor = self.v.get_actor(*INIT_ACTOR_ADDR).unwrap();
        init_actor.head = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.v.set_actor(*INIT_ACTOR_ADDR, init_actor);

        let new_actor_msg = InternalMessage {
            from: *SYSTEM_ACTOR_ADDR,
            to: target_id_addr,
            value: TokenAmount::zero(),
            method: METHOD_CONSTRUCTOR,
            params: serialize::<Address>(target, "address").unwrap(),
        };
        {
            let mut new_ctx = InvocationCtx {
                v: self.v,
                top: self.top.clone(),
                msg: new_actor_msg,
                allow_side_effects: true,
                caller_validated: false,
                policy: self.policy,
                subinvocations: RefCell::new(vec![]),
            };
            new_ctx.create_actor(*ACCOUNT_ACTOR_CODE_ID, target_id).unwrap();
            let res = new_ctx.invoke();
            let invoc = new_ctx.gather_trace(res);
            RefMut::map(self.subinvocations.borrow_mut(), |subinvocs| {
                subinvocs.push(invoc);
                subinvocs
            });
        }

        Ok((self.v.get_actor(target_id_addr).unwrap(), target_id_addr))
    }

    fn gather_trace(&mut self, invoke_result: Result<RawBytes, ActorError>) -> InvocationTrace {
        let (ret, code) = match invoke_result {
            Ok(rb) => (Some(rb), None),
            Err(ae) => (None, Some(ae.exit_code())),
        };
        InvocationTrace {
            msg: self.msg.clone(),
            code,
            ret,
            subinvocations: self.subinvocations.take(),
        }
    }

    fn invoke(&mut self) -> Result<RawBytes, ActorError> {
        let prior_root = self.v.checkpoint();

        // Transfer funds
        let mut from_actor = self.v.get_actor(self.msg.from).unwrap();
        if !self.msg.value.is_zero() {
            if self.msg.value.lt(&BigInt::zero()) {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_ASSERTION_FAILED,
                    "attempt to transfer negative value".to_string(),
                ));
            }
            if from_actor.balance.lt(&self.msg.value) {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_INSUFFICIENT_FUNDS,
                    "insufficient balance to transfer".to_string(),
                ));
            }
        }

        // Load, deduct, store from actor before loading to actor to handle self-send case
        from_actor.balance = from_actor.balance.abs_sub(&self.msg.value);
        self.v.set_actor(self.msg.from, from_actor);

        let (mut to_actor, to_addr) = self.resolve_target(&self.msg.to)?;
        to_actor.balance = to_actor.balance.add(&self.msg.value);
        self.v.set_actor(to_addr, to_actor);

        println!("to: {}, from: {}\n", to_addr, self.msg.from);

        // Exit early on send
        if self.msg.method == METHOD_SEND {
            return Ok(RawBytes::default());
        }

        // call target actor
        let to_actor = self.v.get_actor(to_addr).unwrap();
        let params = self.msg.params.clone();
        let res = match ACTOR_TYPES.get(&to_actor.code).expect("Target actor is not a builtin") {
            // XXX Review: is there a way to do one call on an object implementing ActorCode trait?
            // I tried using `dyn` keyword couldn't get the compiler on board.
            Type::Account => AccountActor::invoke_method(self, self.msg.method, &params),
            Type::Cron => CronActor::invoke_method(self, self.msg.method, &params),
            Type::Init => InitActor::invoke_method(self, self.msg.method, &params),
            Type::Market => MarketActor::invoke_method(self, self.msg.method, &params),
            Type::Miner => MinerActor::invoke_method(self, self.msg.method, &params),
            Type::Multisig => MultisigActor::invoke_method(self, self.msg.method, &params),
            Type::System => SystemActor::invoke_method(self, self.msg.method, &params),
            Type::Reward => RewardActor::invoke_method(self, self.msg.method, &params),
            Type::Power => PowerActor::invoke_method(self, self.msg.method, &params),
            Type::PaymentChannel => PaychActor::invoke_method(self, self.msg.method, &params),
            Type::VerifiedRegistry => VerifregActor::invoke_method(self, self.msg.method, &params),
        };
        if res.is_err() {
            self.v.rollback(prior_root)
        };
        res
    }
}

impl<'invocation, 'bs> Runtime<MemoryBlockstore> for InvocationCtx<'invocation, 'bs> {
    fn create_actor(&mut self, code_id: Cid, actor_id: ActorID) -> Result<(), ActorError> {
        match NON_SINGLETON_CODES.get(&code_id) {
            Some(_) => (),
            None => {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_ASSERTION_FAILED,
                    "create_actor called with singleton builtin actor code cid".to_string(),
                ))
            }
        }
        let addr = Address::new_id(actor_id);
        if self.v.get_actor(addr).is_some() {
            return Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "attempt to create new actor at existing address".to_string(),
            ));
        }
        let a = actor(code_id, self.v.empty_obj_cid, 0, BigInt::zero());
        self.v.set_actor(addr, a);
        Ok(())
    }

    fn store(&self) -> &MemoryBlockstore {
        self.v.store
    }

    fn network_version(&self) -> NetworkVersion {
        self.v.network_version
    }

    fn message(&self) -> &dyn MessageInfo {
        &self.msg
    }

    fn curr_epoch(&self) -> ChainEpoch {
        self.v.curr_epoch
    }

    fn validate_immediate_caller_accept_any(&mut self) -> Result<(), ActorError> {
        if self.caller_validated {
            Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "caller double validated".to_string(),
            ))
        } else {
            self.caller_validated = true;
            Ok(())
        }
    }

    fn validate_immediate_caller_is<'a, I>(&mut self, addresses: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Address>,
    {
        if self.caller_validated {
            return Err(ActorError::unchecked(
                ExitCode::USR_ASSERTION_FAILED,
                "caller double validated".to_string(),
            ));
        }
        for addr in addresses {
            if *addr == self.msg.from {
                return Ok(());
            }
        }
        Err(ActorError::unchecked(
            ExitCode::USR_FORBIDDEN,
            "immediate caller address forbidden".to_string(),
        ))
    }

    fn validate_immediate_caller_type<'a, I>(&mut self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        if self.caller_validated {
            return Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "caller double validated".to_string(),
            ));
        }
        let to_match = ACTOR_TYPES.get(&self.v.get_actor(self.msg.from).unwrap().code).unwrap();
        if types.into_iter().any(|t| *t == *to_match) {
            return Ok(());
        }
        Err(ActorError::unchecked(
            ExitCode::SYS_ASSERTION_FAILED,
            "immediate caller actor type forbidden".to_string(),
        ))
    }

    fn current_balance(&self) -> TokenAmount {
        self.v.get_actor(self.msg.to).unwrap().balance
    }

    fn resolve_address(&self, addr: &Address) -> Option<Address> {
        self.v.normalize_address(addr)
    }

    fn get_actor_code_cid(&self, addr: &Address) -> Option<Cid> {
        let maybe_act = self.v.get_actor(*addr);
        match maybe_act {
            None => None,
            Some(act) => Some(act.code),
        }
    }

    fn send(
        &self,
        to: Address,
        method: MethodNum,
        params: RawBytes,
        value: TokenAmount,
    ) -> Result<RawBytes, ActorError> {
        if !self.allow_side_effects {
            return Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "Calling send is not allowed during side-effect lock".to_string(),
            ));
        }

        let new_actor_msg = InternalMessage { from: self.msg.to, to, value, method, params };
        let mut new_ctx = InvocationCtx {
            v: self.v,
            top: self.top.clone(),
            msg: new_actor_msg,
            allow_side_effects: true,
            caller_validated: false,
            policy: self.policy,
            subinvocations: RefCell::new(vec![]),
        };
        println!("starting send invoc [{}:{}]", to, method);
        let res = new_ctx.invoke();
        println!("finished send invoc [{}:{}]", to, method);

        let invoc = new_ctx.gather_trace(res.clone());
        RefMut::map(self.subinvocations.borrow_mut(), |subinvocs| {
            subinvocs.push(invoc);
            subinvocs
        });
        res
    }

    fn get_randomness_from_tickets(
        &self,
        _personalization: DomainSeparationTag,
        _rand_epoch: ChainEpoch,
        _entropy: &[u8],
    ) -> Result<Randomness, ActorError> {
        Ok(Randomness(TEST_VM_RAND_STRING.as_bytes().to_vec()))
    }

    fn get_randomness_from_beacon(
        &self,
        _personalization: DomainSeparationTag,
        _rand_epoch: ChainEpoch,
        _entropy: &[u8],
    ) -> Result<Randomness, ActorError> {
        Ok(Randomness(TEST_VM_RAND_STRING.as_bytes().to_vec()))
    }

    fn create<C: Cbor>(&mut self, obj: &C) -> Result<(), ActorError> {
        let maybe_act = self.v.get_actor(self.msg.to);
        match maybe_act {
            None => Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "failed to create state".to_string(),
            )),
            Some(mut act) => {
                if act.head != self.v.empty_obj_cid {
                    Err(ActorError::unchecked(
                        ExitCode::SYS_ASSERTION_FAILED,
                        "failed to construct state: already initialized".to_string(),
                    ))
                } else {
                    act.head = self.v.store.put_cbor(obj, Code::Blake2b256).unwrap();
                    self.v.set_actor(self.msg.to, act);
                    Ok(())
                }
            }
        }
    }

    fn state<C: Cbor>(&self) -> Result<C, ActorError> {
        Ok(self.v.get_state::<C>(self.msg.to).unwrap())
    }

    fn transaction<C, RT, F>(&mut self, f: F) -> Result<RT, ActorError>
    where
        C: Cbor,
        F: FnOnce(&mut C, &mut Self) -> Result<RT, ActorError>,
    {
        let mut st = self.state::<C>().unwrap();
        self.allow_side_effects = false;
        let result = f(&mut st, self);
        self.allow_side_effects = true;
        let ret = result?;
        let mut act = self.v.get_actor(self.msg.to).unwrap();
        act.head = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.v.set_actor(self.msg.to, act);
        Ok(ret)
    }

    fn new_actor_address(&mut self) -> Result<Address, ActorError> {
        let osa_bytes = self.top.originator_stable_addr.to_bytes();
        let mut seq_num_bytes = self.top.originator_stable_addr.to_bytes();
        let cnt = self.top.new_actor_addr_count.take();
        self.top.new_actor_addr_count.replace(cnt + 1);
        let mut cnt_bytes = serialize(&cnt, "count failed").unwrap().to_vec();
        let mut out = osa_bytes;
        out.append(&mut seq_num_bytes);
        out.append(&mut cnt_bytes);
        Ok(Address::new_actor(out.as_slice()))
    }

    fn delete_actor(&mut self, _beneficiary: &Address) -> Result<(), ActorError> {
        panic!("TODO implement me")
    }

    fn resolve_builtin_actor_type(&self, code_id: &Cid) -> Option<Type> {
        ACTOR_TYPES.get(code_id).cloned()
    }

    fn get_code_cid_for_type(&self, typ: Type) -> Cid {
        ACTOR_CODES.get(&typ).cloned().unwrap()
    }

    fn total_fil_circ_supply(&self) -> TokenAmount {
        self.top._circ_supply.clone()
    }

    fn charge_gas(&mut self, _name: &'static str, _compute: i64) {}

    fn base_fee(&self) -> TokenAmount {
        TokenAmount::zero()
    }
}

impl Primitives for InvocationCtx<'_, '_> {
    fn verify_signature(
        &self,
        _signature: &Signature,
        _signer: &Address,
        _plaintext: &[u8],
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn hash_blake2b(&self, _data: &[u8]) -> [u8; 32] {
        // TODO: actual blake 2b
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ]
    }

    fn compute_unsealed_sector_cid(
        &self,
        _proof_type: RegisteredSealProof,
        _pieces: &[PieceInfo],
    ) -> Result<Cid, anyhow::Error> {
        panic!("TODO implement me")
    }
}

impl Verifier for InvocationCtx<'_, '_> {
    fn verify_seal(&self, _vi: &SealVerifyInfo) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn verify_post(&self, _verify_info: &WindowPoStVerifyInfo) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn verify_consensus_fault(
        &self,
        _h1: &[u8],
        _h2: &[u8],
        _extra: &[u8],
    ) -> Result<Option<ConsensusFault>, anyhow::Error> {
        Ok(None)
    }

    fn batch_verify_seals(&self, batch: &[SealVerifyInfo]) -> anyhow::Result<Vec<bool>> {
        Ok(vec![true; batch.len()]) // everyone wins
    }

    fn verify_aggregate_seals(
        &self,
        _aggregate: &AggregateSealVerifyProofAndInfos,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn verify_replica_update(&self, _replica: &ReplicaUpdateInfo) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

impl RuntimePolicy for InvocationCtx<'_, '_> {
    fn policy(&self) -> &Policy {
        self.policy
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MessageResult {
    pub code: ExitCode,
    pub ret: RawBytes,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Debug)]
pub struct Actor {
    pub code: Cid,
    pub head: Cid,
    pub call_seq_num: u64,
    #[serde(with = "bigint_ser")]
    pub balance: TokenAmount,
}

pub fn actor(code: Cid, head: Cid, seq: u64, bal: TokenAmount) -> Actor {
    Actor { code, head, call_seq_num: seq, balance: bal }
}

pub struct InvocationTrace {
    pub msg: InternalMessage,
    pub code: Option<ExitCode>,
    pub ret: Option<RawBytes>,
    pub subinvocations: Vec<InvocationTrace>,
}

pub struct ExpectInvocation {
    pub to: Address,       // required
    pub method: MethodNum, // required
    pub code: Option<ExitCode>,
    pub from: Option<Address>,
    pub params: Option<RawBytes>,
    pub ret: Option<RawBytes>,
    pub subinvocs: Option<Vec<ExpectInvocation>>,
}

impl ExpectInvocation {
    // testing method that panics on no match
    pub fn matches(&self, invoc: &InvocationTrace) {
        let id = format!("[{}:{}]", invoc.msg.to, invoc.msg.method);
        self.quick_match(invoc, String::new());
        if let Some(c) = self.code {
            assert_ne!(
                None,
                invoc.code,
                "{} unexpected code: expected:{}was:{}",
                id,
                c,
                ExitCode::OK
            );
            assert_eq!(
                c,
                invoc.code.unwrap(),
                "{} unexpected code expected:{}was:{}",
                id,
                c,
                invoc.code.unwrap()
            );
        }
        if let Some(f) = self.from {
            assert_eq!(
                f, invoc.msg.from,
                "{} unexpected from addr: expected:{}was:{} ",
                id, f, invoc.msg.from
            );
        }
        if let Some(p) = &self.params {
            assert_eq!(
                p, &invoc.msg.params,
                "{} unexpected params: expected:{:x?}was:{:x?}",
                id, p, invoc.msg.params
            );
        }
        if let Some(r) = &self.ret {
            assert_ne!(None, invoc.ret, "{} unexpected ret: expected:{:x?}was:None", id, r);
            let ret = &invoc.ret.clone().unwrap();
            assert_eq!(r, ret, "{} unexpected ret: expected:{:x?}was:{:x?}", id, r, ret);
        }
        if let Some(expect_subinvocs) = &self.subinvocs {
            let subinvocs = &invoc.subinvocations;

            let panic_str = format!(
                "unexpected subinvocs:\n expected: \n[\n{}]\n was:\n[\n{}]\n",
                self.fmt_expect_invocs(expect_subinvocs),
                self.fmt_invocs(subinvocs)
            );
            assert!(subinvocs.len() == expect_subinvocs.len(), "{}", panic_str);

            for (i, invoc) in subinvocs.iter().enumerate() {
                let expect_invoc = expect_subinvocs.get(i).unwrap();
                // only try to match if required fields match
                expect_invoc.quick_match(invoc, panic_str.clone());
                expect_invoc.matches(invoc);
            }
        }
    }

    pub fn fmt_invocs(&self, invocs: &[InvocationTrace]) -> String {
        invocs
            .iter()
            .enumerate()
            .map(|(i, invoc)| format!("{}: [{}:{}],\n", i, invoc.msg.to, invoc.msg.method))
            .collect()
    }

    pub fn fmt_expect_invocs(&self, invocs: &[ExpectInvocation]) -> String {
        invocs
            .iter()
            .enumerate()
            .map(|(i, invoc)| format!("{}: [{}:{}],\n", i, invoc.to, invoc.method))
            .collect()
    }

    pub fn quick_match(&self, invoc: &InvocationTrace, extra_msg: String) {
        let id = format!("[{}:{}]", invoc.msg.to, invoc.msg.method);
        assert_eq!(
            self.to, invoc.msg.to,
            "{} unexpected to addr: expected:{} was:{} \n{}",
            id, self.to, invoc.msg.to, extra_msg
        );
        assert_eq!(
            self.method, invoc.msg.method,
            "{} unexpected method: expected:{}was:{} \n{}",
            id, self.method, invoc.msg.from, extra_msg
        );
    }
}

#[derive(Debug)]
pub struct TestVMError {
    msg: String,
}

impl fmt::Display for TestVMError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl Error for TestVMError {
    fn description(&self) -> &str {
        &self.msg
    }
}

impl From<fvm_ipld_hamt::Error> for TestVMError {
    fn from(h_err: fvm_ipld_hamt::Error) -> Self {
        vm_err(h_err.to_string().as_str())
    }
}

pub fn vm_err(msg: &str) -> TestVMError {
    TestVMError { msg: msg.to_string() }
}

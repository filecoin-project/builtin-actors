use cid::multihash::Code;
use cid::Cid;
use fil_actor_account::Actor as AccountActor;
use fil_actor_cron::Actor as CronActor;
use fil_actor_init::{Actor as InitActor, State as InitState};
use fil_actor_market::Actor as MarketActor;
use fil_actor_miner::Actor as MinerActor;
use fil_actor_multisig::Actor as MultisigActor;
use fil_actor_paych::Actor as PaychActor;
use fil_actor_power::Actor as PowerActor;
use fil_actor_reward::Actor as RewardActor;
use fil_actor_system::{Actor as SystemActor, State as SystemState};
use fil_actor_verifreg::Actor as VerifregActor;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::{actor_error};
use fil_actors_runtime::runtime::{
    ActorCode, MessageInfo, Policy, Runtime, RuntimePolicy, Syscalls,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{ActorError, INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
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
    WindowPoStVerifyInfo,
};
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use num_traits::Signed;
use std::cell::RefCell;
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
    // invocationStack
}

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
        }
    }

    pub fn new_with_singletons(store: &'bs MemoryBlockstore) -> VM<'bs> {
        // craft init state directly
        let v = VM::new(store);
        let init_st = InitState::new(store, "integration-test".to_string()).unwrap();
        let init_head = store.put_cbor(&init_st, Code::Blake2b256).unwrap();
        v.set_actor(*INIT_ACTOR_ADDR, actor(*INIT_ACTOR_CODE_ID, init_head, 0, BigInt::from(200_000_000)));
        let sys_st = SystemState::new(store).unwrap();
        let sys_head = store.put_cbor(&sys_st, Code::Blake2b256).unwrap();
        v.set_actor(*SYSTEM_ACTOR_ADDR, actor(*SYSTEM_ACTOR_CODE_ID, sys_head, 0, BigInt::zero()));
        v.checkpoint();
        // TODO: actually add remaining singletons for testing
        v
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
        a.call_seq_num += 1;
        let call_seq_num = a.call_seq_num;
        self.set_actor(from_id, a);

        let prior_root = self.checkpoint();

        // make top level context with internal context
        let top = TopCtx {
            _originator_stable_addr: to,
            _originator_call_seq: call_seq_num,
            _new_actor_addr_count: 0,
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
            allow_side_effects: false,
            _caller_validated: false,
            policy: &Policy::default(),
        };
        let res = new_ctx.invoke();
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
}
#[derive(Clone)]
pub struct TopCtx {
    _originator_stable_addr: Address,
    _originator_call_seq: u64,
    _new_actor_addr_count: u64,
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

pub struct InvocationCtx<'invocation, 'bs> {
    v: &'invocation VM<'bs>,
    top: TopCtx,
    msg: InternalMessage,
    allow_side_effects: bool,
    _caller_validated: bool,
    policy: &'invocation Policy,
}

impl<'invocation, 'bs> InvocationCtx<'invocation, 'bs> {
    fn resolve_target(&'invocation self, target: &Address) -> Result<(Actor, Address), ActorError> {
        if let Some(a) = self.v.normalize_address(target) {
            if let Some(act) = self.v.get_actor(a) {
                return Ok((act, a))
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
                allow_side_effects: false,
                _caller_validated: false,
                policy: self.policy,
            };
            new_ctx.create_actor(*ACCOUNT_ACTOR_CODE_ID, target_id).unwrap();
            _ = new_ctx.invoke();
        }

        Ok((self.v.get_actor(target_id_addr).unwrap(), target_id_addr))
    }

    fn invoke(&mut self) -> Result<RawBytes, ActorError> {
        let prior_root = self.v.checkpoint();
        let (mut to_actor, to_addr) = self.resolve_target(&self.msg.to)?;

        // Transfer funds
        println!("to: {}, from: {}\n", to_addr, self.msg.from);
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
        to_actor.balance = to_actor.balance.add(&self.msg.value);
        from_actor.balance = from_actor.balance.abs_sub(&self.msg.value);
        self.v.set_actor(self.msg.from, from_actor);
        self.v.set_actor(to_addr, to_actor);

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
            // _ => Err(ActorError::unchecked(
            //     ExitCode::SYS_INVALID_METHOD,
            //     "actor code type unhanlded by test vm".to_string(),
            // )),
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
        panic!("TODO implement me")
    }

    fn message(&self) -> &dyn MessageInfo {
        &self.msg
    }

    fn curr_epoch(&self) -> ChainEpoch {
        panic!("TODO implement me")
    }

    fn validate_immediate_caller_accept_any(&mut self) -> Result<(), ActorError> {
        panic!("TODO implement me")
    }

    fn validate_immediate_caller_is<'a, I>(&mut self, addresses: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Address>,
    {
        for addr in addresses {
            if *addr == self.msg.from {
                return Ok(())
            }
        }
        Err(ActorError::unchecked(ExitCode::SYS_ASSERTION_FAILED, "immediate caller address forbidden".to_string()))
    }

    fn validate_immediate_caller_type<'a, I>(&mut self, _types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        panic!("TODO implement me")
    }

    fn current_balance(&self) -> TokenAmount {
        panic!("TODO implement me")
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
            allow_side_effects: false,
            _caller_validated: false,
            policy: self.policy,
        };
        new_ctx.invoke()
    }

    fn get_randomness_from_tickets(
        &self,
        _personalization: DomainSeparationTag,
        _rand_epoch: ChainEpoch,
        _entropy: &[u8],
    ) -> Result<Randomness, ActorError> {
        panic!("TODO implement me")
    }

    fn get_randomness_from_beacon(
        &self,
        _personalization: DomainSeparationTag,
        _rand_epoch: ChainEpoch,
        _entropy: &[u8],
    ) -> Result<Randomness, ActorError> {
        panic!("TODO implement me")
    }

    fn create<C: Cbor>(&mut self, obj: &C) -> Result<(), ActorError> {
        let maybe_act = self.v.get_actor(self.msg.to);
        match maybe_act {
            None => Err(ActorError::unchecked(ExitCode::SYS_ASSERTION_FAILED, "failed to create state".to_string())),
            Some(mut act) => {
                if act.head != self.v.empty_obj_cid {
                    Err(ActorError::unchecked(ExitCode::SYS_ASSERTION_FAILED, "failed to construct state: already initialized".to_string()))
                } else {
                    act.head = self.v.store.put_cbor(obj, Code::Blake2b256).unwrap();
                    self.v.set_actor(self.msg.to, act);
                    Ok(())
                }
            },
        }
    }

    fn state<C: Cbor>(&self) -> Result<C, ActorError> {
        panic!("TODO implement me")
    }

    fn transaction<C, RT, F>(&mut self, _f: F) -> Result<RT, ActorError>
    where
        C: Cbor,
        F: FnOnce(&mut C, &mut Self) -> Result<RT, ActorError>,
    {
        panic!("TODO implement me")
    }

    fn new_actor_address(&mut self) -> Result<Address, ActorError> {
        panic!("TODO implement me")
    }

    fn delete_actor(&mut self, _beneficiary: &Address) -> Result<(), ActorError> {
        panic!("TODO implement me")
    }

    fn resolve_builtin_actor_type(&self, _code_id: &Cid) -> Option<Type> {
        panic!("TODO implement me")
    }

    fn get_code_cid_for_type(&self, _typ: Type) -> Cid {
        panic!("TODO implement me")
    }

    fn total_fil_circ_supply(&self) -> TokenAmount {
        panic!("TODO implement me")
    }

    fn charge_gas(&mut self, _name: &'static str, _compute: i64) {}

    fn base_fee(&self) -> TokenAmount {
        TokenAmount::zero()
    }
}

impl Syscalls for InvocationCtx<'_, '_> {
    fn verify_signature(
        &self,
        _signature: &Signature,
        _signer: &Address,
        _plaintext: &[u8],
    ) -> Result<(), anyhow::Error> {
        panic!("TODO implement me")
    }

    fn hash_blake2b(&self, _data: &[u8]) -> [u8; 32] {
        panic!("TODO implement me")
    }

    fn compute_unsealed_sector_cid(
        &self,
        _proof_type: RegisteredSealProof,
        _pieces: &[PieceInfo],
    ) -> Result<Cid, anyhow::Error> {
        panic!("TODO implement me")
    }

    fn verify_seal(&self, _vi: &SealVerifyInfo) -> Result<(), anyhow::Error> {
        panic!("TODO implement me")
    }

    fn verify_post(&self, _verify_info: &WindowPoStVerifyInfo) -> Result<(), anyhow::Error> {
        panic!("TODO implement me")
    }

    fn verify_consensus_fault(
        &self,
        _h1: &[u8],
        _h2: &[u8],
        _extra: &[u8],
    ) -> Result<Option<ConsensusFault>, anyhow::Error> {
        panic!("TODO implement me")
    }

    fn batch_verify_seals(&self, _batch: &[SealVerifyInfo]) -> anyhow::Result<Vec<bool>> {
        panic!("TODO implement me")
    }

    fn verify_aggregate_seals(
        &self,
        _aggregate: &AggregateSealVerifyProofAndInfos,
    ) -> Result<(), anyhow::Error> {
        panic!("TODO implement me")
    }

    fn verify_replica_update(&self, _replica: &ReplicaUpdateInfo) -> Result<(), anyhow::Error> {
        panic!("TODO implement me")
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

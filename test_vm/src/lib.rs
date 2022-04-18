use cid::Cid;
use fil_actors_runtime::runtime::fvm::FvmRuntime;
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, ActorError};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::{ActorCode}; 
use fil_actor_init::{State as InitState, Actor as InitActor};
use fil_actor_cron::{Actor as CronActor};
use fil_actor_system::{Actor as SystemActor};
use fil_actor_account::{Actor as AccountActor};
use fil_actor_multisig::{Actor as MultisigActor};
use fil_actor_paych::{Actor as PaychActor};
use fil_actor_power::{Actor as PowerActor};
use fil_actor_reward::{Actor as RewardActor};
use fil_actor_market::{Actor as MarketActor};
use fil_actor_miner::{Actor as MinerActor};
use fil_actor_verifreg::{Actor as VerifregActor};
use fvm_shared::actor::builtin::Type;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{Cbor, RawBytes, CborStore};
use fvm_ipld_hamt::{BytesKey, Hamt, Sha256};
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::{bigint_ser, BigInt, Zero};
use fvm_shared::econ::TokenAmount;
use fvm_shared::{ActorID, MethodNum, METHOD_SEND, METHOD_CONSTRUCTOR};
use fvm_shared::error::ExitCode;
use std::error::Error;
use std::thread::AccessError;
use cid::multihash::Code;
use std::fmt;
use num_traits::Signed;
use std::ops::Add;
use serde::{Serialize};



pub struct VM<'bs> {
    store: &'bs MemoryBlockstore,
    state_root: Cid,
    actors_dirty: bool,
    actors: Hamt<&'bs MemoryBlockstore, Actor, BytesKey>,
    empty_obj_cid: Cid,
    // invocationStack
}

impl<'bs> VM<'bs> {
    pub fn new(store: &'bs MemoryBlockstore) -> VM<'bs> {
        let mut actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::new(store);
        let empty = store.put_cbor(&(), Code::Blake2b256).unwrap();
        VM { store, state_root: actors.flush().unwrap(), actors_dirty: false, actors, empty_obj_cid: empty}
    }

    pub fn get_actor(&self, addr: &Address) -> Option<&Actor> {
        self.actors.get(&addr.to_bytes()).unwrap()
    }

    // blindly overwrite the actor at this address whether it previously existed or not
    pub fn set_actor(&mut self, key: &Address, a: Actor) {
        let _ = self.actors.set(key.to_bytes().into(), a).unwrap();
    }

    pub fn checkpoint(&mut self) -> Cid {
        self.state_root = self.actors.flush().unwrap();
        self.actors_dirty = false;
        self.state_root
    }

    pub fn rollback(&mut self, root: &Cid) {
        self.actors =
            Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::load(root, self.store).unwrap();
        self.state_root = *root;
        self.actors_dirty = false;
    }

    pub fn normalize_address(&self, addr: &Address) -> Option<Address> {
        let st = self.get_state::<InitState>(&INIT_ACTOR_ADDR).unwrap();
        st.resolve_address::<MemoryBlockstore>(self.store, &addr).unwrap()
    }

    pub fn get_state<C: Cbor>(&self, addr: &Address) -> Option<C>{
        let a_opt = self.get_actor(addr);
        match a_opt {
            None => return None,
            _ => (),
        }
        let a = a_opt.unwrap();
        self.store.get_cbor::<C>(&a.head).unwrap()
    }

    pub fn apply_message<C: Cbor>(&mut self, from: &Address, to: &Address, value: &TokenAmount, method: MethodNum, params: C) -> Result<MessageResult, TestVMError> {
        let from_id = self.normalize_address(&from).unwrap();
        let mut a = self.get_actor(&from_id).unwrap().clone();
        a.call_seq_num += 1; 
        self.set_actor(&from_id, a);

        let prior_root = self.checkpoint();

        // make top level context with internal context
        // let ret, exitcode = ctx.invoke()
        let ret = RawBytes::default();
        let code = ExitCode::OK;

        if code != ExitCode::OK { // if exitcode != ok
            self.rollback(&prior_root);
        } else {
            self.checkpoint();
        }

        Ok(MessageResult{code: code, ret: ret})
    }
}

pub struct TopCtx {
    originator_stable_addr: Address,
    originator_call_seq: u64,
    new_actor_addr_count: u64,
    circ_supply: BigInt,
}

pub struct InternalMessage {
    from: Address,
    to: Address,
    value: TokenAmount,
    method: MethodNum,
    params: RawBytes,
}

pub struct InvocationCtx<'bs> {
    v: &'bs mut VM::<'bs>,
    top: &'bs TopCtx,
    msg: &'bs InternalMessage,
    allow_side_effects: bool,
    caller_validated: bool,
}

impl<'bs> InvocationCtx<'bs> {
    fn resolve_target(&mut self, target: &Address) -> Result<(&Actor, Address), ActorError> {
        match self.v.normalize_address(target) {
            Some(a) => return Ok((self.v.get_actor(&a).unwrap(), a)),
            None => (),
        };
        // Address does not yet exist, create it
        match target.protocol() {
            Protocol::Actor | Protocol::ID => return Err(ActorError::unchecked(ExitCode::SYS_INVALID_RECEIVER, "cannot create account for address type".to_string())),
            _ => (),
        }
        let mut st = self.v.get_state::<InitState>(&INIT_ACTOR_ADDR).unwrap();
        let target_id = st.map_address_to_new_id(self.v.store, target).unwrap();
        let target_id_addr = Address::new_id(target_id);
        let init_actor = self.v.get_actor(&INIT_ACTOR_ADDR).unwrap().clone();
        init_actor.head = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.v.set_actor(&INIT_ACTOR_ADDR, init_actor);

        self.create_actor(*ACCOUNT_ACTOR_CODE_ID, target_id);
        let new_actor_msg = InternalMessage{
            from: *SYSTEM_ACTOR_ADDR, 
            to: target_id_addr, 
            value: TokenAmount::zero(), 
            method: METHOD_CONSTRUCTOR,
            params: serialize::<Address>(target, "address").unwrap(),
        };
        let new_ctx = &InvocationCtx{
            v: self.v,
            top: self.top,
            msg: &new_actor_msg,
            allow_side_effects: false,
            caller_validated: false,
        };
        _ = new_ctx.invoke()?;
        Ok((self.v.get_actor(&target_id_addr).unwrap(), target_id_addr))
    }

    fn invoke(&mut self) -> Result<RawBytes, ActorError> {
        let prior_root = self.v.checkpoint();
        // TODO resolve target for id ification + creation of new account actors
        let mut to_actor: Actor = self.v.get_actor(&self.msg.to).unwrap().clone(); // XXX resolve target

        // Transfer funds
        let mut from_actor = self.v.get_actor(&self.msg.from).unwrap().clone();
        if !self.msg.value.is_zero() {
            if self.msg.value.lt(&BigInt::zero()) {
                return Err(ActorError::unchecked(ExitCode::SYS_ASSERTION_FAILED, "attempt to transfer negative value".to_string()))
            }
            if from_actor.balance.lt(&self.msg.value) {
                return Err(ActorError::unchecked(ExitCode::SYS_INSUFFICIENT_FUNDS, "insufficient balance to transfer".to_string()))
            }
        }
        to_actor.balance = to_actor.balance.add(&self.msg.value);
        from_actor.balance = from_actor.balance.abs_sub(&self.msg.value);
        self.v.set_actor(&self.msg.from, from_actor);
        self.v.set_actor(&self.msg.to, to_actor);

        // Exit early on send
        if self.msg.method == METHOD_SEND {
            return Ok(RawBytes::default())
        }

        // call target actor
        let to_actor = self.v.get_actor(&self.msg.to).unwrap();
        let mut rt = FvmRuntime::default(); // XXX todo use invocation context instead        
        match ACTOR_TYPES.get(&to_actor.code).expect("Target actor is not a builtin") {
            // XXX Review: is there a way to do one call on an object implementing ActorCode trait?
            // I tried using `dyn` keyword couldn't get the compiler on board.
            Type::Account => AccountActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::Cron => CronActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::Init => InitActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::Market => MarketActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::Miner => MinerActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::Multisig => MultisigActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::System => SystemActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::Reward => RewardActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::Power => PowerActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::PaymentChannel => PaychActor::invoke_method(&mut rt, self.msg.method, &self.msg.params),
            Type::VerifiedRegistry => VerifregActor::invoke_method(& mut rt, self.msg.method, &self.msg.params),
            _=> Err(ActorError::unchecked(ExitCode::SYS_INVALID_METHOD, "actor code type unhanlded by test vm".to_string())),
        }
    }

    fn create_actor(&mut self, code_id: Cid, actor_id: ActorID) -> Result<(), ActorError> {
        match NON_SINGLETON_CODES.get(&code_id) {
            Some(_) => (),
            None => return Err(ActorError::unchecked(ExitCode::SYS_ASSERTION_FAILED, "create_actor called with singleton builtin actor code cid".to_string())),
        }
        let addr = Address::new_id(actor_id);
        match self.v.get_actor(&addr) {
            Some(_) => return Err(ActorError::unchecked(ExitCode::SYS_ASSERTION_FAILED, "attempt to create new actor at existing address".to_string())),
            None => (),
        }
        let a = actor(code_id, self.v.empty_obj_cid, 0, BigInt::zero());
        self.v.set_actor(&addr, a);
        Ok(())
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

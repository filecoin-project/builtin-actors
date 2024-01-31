use anyhow::anyhow;
use cid::multihash::Code;
use cid::Cid;
use fil_actor_account::Actor as AccountActor;
use fil_actor_cron::Actor as CronActor;
use fil_actor_datacap::Actor as DataCapActor;
use fil_actor_eam::EamActor;
use fil_actor_ethaccount::EthAccountActor;
use fil_actor_evm::EvmContractActor;
use fil_actor_init::{Actor as InitActor, State as InitState};
use fil_actor_market::Actor as MarketActor;
use fil_actor_miner::Actor as MinerActor;
use fil_actor_multisig::Actor as MultisigActor;
use fil_actor_paych::Actor as PaychActor;
use fil_actor_power::Actor as PowerActor;
use fil_actor_reward::Actor as RewardActor;
use fil_actor_system::Actor as SystemActor;
use fil_actor_verifreg::Actor as VerifregActor;

use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{
    ActorCode, DomainSeparationTag, MessageInfo, Policy, Primitives, Runtime, RuntimePolicy,
    EMPTY_ARR_CID,
};
use fil_actors_runtime::{actor_error, SendError};
use fil_actors_runtime::{test_utils::*, SYSTEM_ACTOR_ID};
use fil_actors_runtime::{ActorError, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::CborStore;

use fvm_shared::address::Address;
use fvm_shared::address::Payload;
use fvm_shared::bigint::Zero;
use fvm_shared::chainid::ChainID;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::consensus::ConsensusFault;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::crypto::signature::{
    Signature, SECP_PUB_LEN, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE,
};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::event::ActorEvent;
use fvm_shared::piece::PieceInfo;

use fvm_shared::randomness::RANDOMNESS_LENGTH;
use fvm_shared::sector::{
    AggregateSealVerifyProofAndInfos, RegisteredSealProof, ReplicaUpdateInfo, SealVerifyInfo,
    WindowPoStVerifyInfo,
};

use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, Response, IPLD_RAW, METHOD_CONSTRUCTOR, METHOD_SEND};

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::cell::{RefCell, RefMut};
use vm_api::trace::{EmittedEvent, InvocationTrace};
use vm_api::util::get_state;
use vm_api::{new_actor, ActorState, VM};

use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use std::ops::Add;
use std::rc::Rc;

use crate::{TestVM, TEST_VM_INVALID_POST, TEST_VM_RAND_ARRAY};

#[derive(Clone)]
pub struct TopCtx {
    pub originator_stable_addr: Address,
    pub originator_call_seq: u64,
    pub new_actor_addr_count: RefCell<u64>,
    pub circ_supply: TokenAmount,
}

#[derive(Clone, Debug)]
pub struct InternalMessage {
    pub from: ActorID,
    pub to: Address,
    pub value: TokenAmount,
    pub method: MethodNum,
    pub params: Option<IpldBlock>,
}

impl MessageInfo for InvocationCtx<'_> {
    fn nonce(&self) -> u64 {
        self.top.originator_call_seq
    }
    fn caller(&self) -> Address {
        Address::new_id(self.msg.from)
    }
    fn origin(&self) -> Address {
        Address::new_id(self.resolve_address(&self.top.originator_stable_addr).unwrap())
    }
    fn receiver(&self) -> Address {
        self.to()
    }
    fn value_received(&self) -> TokenAmount {
        self.msg.value.clone()
    }
    fn gas_premium(&self) -> TokenAmount {
        TokenAmount::zero()
    }
}

pub struct InvocationCtx<'invocation> {
    pub v: &'invocation TestVM,
    pub top: TopCtx,
    pub msg: InternalMessage,
    pub allow_side_effects: RefCell<bool>,
    pub caller_validated: RefCell<bool>,
    pub read_only: bool,
    pub policy: &'invocation Policy,
    pub subinvocations: RefCell<Vec<InvocationTrace>>,
    pub events: RefCell<Vec<EmittedEvent>>,
}

impl<'invocation> InvocationCtx<'invocation> {
    fn resolve_target(
        &'invocation self,
        target: &Address,
    ) -> Result<(ActorState, Address), ActorError> {
        if let Some(a) = self.v.resolve_id_address(target) {
            if let Some(act) = self.v.actor(&a) {
                return Ok((act, a));
            }
        };

        // Address does not yet exist, create it
        let is_account = match target.payload() {
            Payload::Secp256k1(_) | Payload::BLS(_) => true,
            Payload::Delegated(da)
            // Validate that there's an actor at the target ID (we don't care what is there,
            // just that something is there).
            if self.v.actor(&Address::new_id(da.namespace())).is_some() =>
                {
                    false
                }
            _ => {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_INVALID_RECEIVER,
                    format!("cannot create account for address {} type {}", target, target.protocol()),
                ));
            }
        };

        // But only if we're not in read-only mode.
        if self.read_only() {
            return Err(ActorError::unchecked(
                ExitCode::USR_READ_ONLY,
                format!("cannot create actor {target} in read-only mode"),
            ));
        }

        let mut st: InitState = get_state(self.v, &INIT_ACTOR_ADDR).unwrap();
        let (target_id, existing) = st.map_addresses_to_id(&self.v.store, target, None).unwrap();
        assert!(!existing, "should never have existing actor when no f4 address is specified");
        let target_id_addr = Address::new_id(target_id);
        let mut init_actor = self.v.actor(&INIT_ACTOR_ADDR).unwrap();
        init_actor.state = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.v.set_actor(&INIT_ACTOR_ADDR, init_actor);

        let new_actor_msg = InternalMessage {
            from: SYSTEM_ACTOR_ID,
            to: target_id_addr,
            value: TokenAmount::zero(),
            method: METHOD_CONSTRUCTOR,
            params: IpldBlock::serialize_cbor(target).unwrap(),
        };
        {
            let mut new_ctx = InvocationCtx {
                v: self.v,
                top: self.top.clone(),
                msg: new_actor_msg,
                allow_side_effects: RefCell::new(true),
                caller_validated: RefCell::new(false),
                read_only: false,
                policy: self.policy,
                subinvocations: RefCell::new(vec![]),
                events: RefCell::new(vec![]),
            };
            if is_account {
                new_ctx.create_actor(*ACCOUNT_ACTOR_CODE_ID, target_id, None).unwrap();
                let res = new_ctx.invoke();
                let invoc = new_ctx.gather_trace(res);
                RefMut::map(self.subinvocations.borrow_mut(), |subinvocs| {
                    subinvocs.push(invoc);
                    subinvocs
                });
            } else {
                new_ctx.create_actor(*PLACEHOLDER_ACTOR_CODE_ID, target_id, Some(*target)).unwrap();
            }
        }

        Ok((self.v.actor(&target_id_addr).unwrap(), target_id_addr))
    }

    pub fn gather_trace(
        &mut self,
        invoke_result: Result<Option<IpldBlock>, ActorError>,
    ) -> InvocationTrace {
        let (ret, code) = match invoke_result {
            Ok(rb) => (rb, ExitCode::OK),
            Err(ae) => (None, ae.exit_code()),
        };
        let mut msg = self.msg.clone();
        msg.to = match self.resolve_target(&self.msg.to) {
            Ok((_, addr)) => addr, // use normalized address in trace
            _ => self.msg.to, // if target resolution fails don't fail whole invoke, just use non normalized
        };
        InvocationTrace {
            from: msg.from,
            to: msg.to,
            value: msg.value,
            method: msg.method,
            params: msg.params,
            // Actors should wrap syscall errors
            error_number: None,
            return_value: ret,
            exit_code: code,
            subinvocations: self.subinvocations.take(),
            events: self.events.take(),
        }
    }

    fn to(&'_ self) -> Address {
        self.resolve_target(&self.msg.to).unwrap().1
    }

    pub fn invoke(&mut self) -> Result<Option<IpldBlock>, ActorError> {
        let prior_root = self.v.checkpoint();

        // Transfer funds
        let mut from_actor = self.v.actor(&Address::new_id(self.msg.from)).unwrap();
        if !self.msg.value.is_zero() {
            if self.msg.value.is_negative() {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_ASSERTION_FAILED,
                    "attempt to transfer negative value".to_string(),
                ));
            }
            if from_actor.balance < self.msg.value {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_INSUFFICIENT_FUNDS,
                    "insufficient balance to transfer".to_string(),
                ));
            }
            if self.read_only() {
                return Err(ActorError::unchecked(
                    ExitCode::USR_READ_ONLY,
                    "cannot transfer value in read-only mode".to_string(),
                ));
            }
        }

        // Load, deduct, store from actor before loading to actor to handle self-send case
        from_actor.balance -= &self.msg.value;
        self.v.set_actor(&Address::new_id(self.msg.from), from_actor);

        let (mut to_actor, to_addr) = self.resolve_target(&self.msg.to)?;
        to_actor.balance = to_actor.balance.add(&self.msg.value);
        self.v.set_actor(&to_addr, to_actor);

        // Exit early on send
        if self.msg.method == METHOD_SEND {
            return Ok(None);
        }
        self.msg.to = to_addr;

        // call target actor
        let to_actor = self.v.actor(&to_addr).unwrap();
        let params = self.msg.params.clone();
        let mut res = match ACTOR_TYPES.get(&to_actor.code).expect("Target actor is not a builtin")
        {
            Type::Account => AccountActor::invoke_method(self, self.msg.method, params),
            Type::Cron => CronActor::invoke_method(self, self.msg.method, params),
            Type::Init => InitActor::invoke_method(self, self.msg.method, params),
            Type::Market => MarketActor::invoke_method(self, self.msg.method, params),
            Type::Miner => MinerActor::invoke_method(self, self.msg.method, params),
            Type::Multisig => MultisigActor::invoke_method(self, self.msg.method, params),
            Type::System => SystemActor::invoke_method(self, self.msg.method, params),
            Type::Reward => RewardActor::invoke_method(self, self.msg.method, params),
            Type::Power => PowerActor::invoke_method(self, self.msg.method, params),
            Type::PaymentChannel => PaychActor::invoke_method(self, self.msg.method, params),
            Type::VerifiedRegistry => VerifregActor::invoke_method(self, self.msg.method, params),
            Type::DataCap => DataCapActor::invoke_method(self, self.msg.method, params),
            Type::Placeholder => {
                Err(ActorError::unhandled_message("placeholder actors only handle method 0".into()))
            }
            Type::EVM => EvmContractActor::invoke_method(self, self.msg.method, params),
            Type::EAM => EamActor::invoke_method(self, self.msg.method, params),
            Type::EthAccount => EthAccountActor::invoke_method(self, self.msg.method, params),
        };
        if res.is_ok() && !*self.caller_validated.borrow() {
            res = Err(actor_error!(assertion_failed, "failed to validate caller"));
        }
        if res.is_err() {
            self.v.rollback(prior_root)
        };

        res
    }
}

impl<'invocation> Runtime for InvocationCtx<'invocation> {
    type Blockstore = Rc<MemoryBlockstore>;

    fn create_actor(
        &self,
        code_id: Cid,
        actor_id: ActorID,
        predictable_address: Option<Address>,
    ) -> Result<(), ActorError> {
        match NON_SINGLETON_CODES.get(&code_id) {
            Some(_) => (),
            None => {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_ASSERTION_FAILED,
                    "create_actor called with singleton builtin actor code cid".to_string(),
                ));
            }
        }
        let addr = &Address::new_id(actor_id);
        let actor = match self.v.actor(addr) {
            Some(mut act) if act.code == *PLACEHOLDER_ACTOR_CODE_ID => {
                act.code = code_id;
                act
            }
            None => new_actor(code_id, EMPTY_ARR_CID, 0, TokenAmount::zero(), predictable_address),
            _ => {
                return Err(actor_error!(forbidden;
                    "attempt to create new actor at existing address {}", addr));
            }
        };

        if self.read_only() {
            return Err(ActorError::unchecked(
                ExitCode::USR_READ_ONLY,
                "cannot create actor in read-only mode".into(),
            ));
        }

        self.top.new_actor_addr_count.replace_with(|old| *old + 1);
        self.v.set_actor(addr, actor);
        Ok(())
    }

    fn store(&self) -> &Rc<MemoryBlockstore> {
        &self.v.store
    }

    fn network_version(&self) -> NetworkVersion {
        self.v.network_version
    }

    fn message(&self) -> &dyn MessageInfo {
        self
    }

    fn curr_epoch(&self) -> ChainEpoch {
        self.v.epoch()
    }

    fn chain_id(&self) -> ChainID {
        ChainID::from(0)
    }

    fn validate_immediate_caller_accept_any(&self) -> Result<(), ActorError> {
        if *self.caller_validated.borrow() {
            Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "caller double validated".to_string(),
            ))
        } else {
            self.caller_validated.replace(true);
            Ok(())
        }
    }

    fn validate_immediate_caller_namespace<I>(
        &self,
        namespace_manager_addresses: I,
    ) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = u64>,
    {
        if *self.caller_validated.borrow() {
            return Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "caller double validated".to_string(),
            ));
        }
        let managers: Vec<_> = namespace_manager_addresses.into_iter().collect();

        if let Some(delegated) =
            self.lookup_delegated_address(self.message().caller().id().unwrap())
        {
            for id in managers {
                if match delegated.payload() {
                    Payload::Delegated(d) => d.namespace() == id,
                    _ => false,
                } {
                    return Ok(());
                }
            }
        } else {
            return Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "immediate caller actor expected to have namespace".to_string(),
            ));
        }

        Err(ActorError::unchecked(
            ExitCode::SYS_ASSERTION_FAILED,
            "immediate caller actor namespace forbidden".to_string(),
        ))
    }

    fn validate_immediate_caller_is<'a, I>(&self, addresses: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Address>,
    {
        if *self.caller_validated.borrow() {
            return Err(ActorError::unchecked(
                ExitCode::USR_ASSERTION_FAILED,
                "caller double validated".to_string(),
            ));
        }
        self.caller_validated.replace(true);
        for addr in addresses {
            if *addr == Address::new_id(self.msg.from) {
                return Ok(());
            }
        }
        Err(ActorError::unchecked(
            ExitCode::USR_FORBIDDEN,
            "immediate caller address forbidden".to_string(),
        ))
    }

    fn validate_immediate_caller_type<'a, I>(&self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>,
    {
        if *self.caller_validated.borrow() {
            return Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "caller double validated".to_string(),
            ));
        }
        self.caller_validated.replace(true);
        let to_match =
            ACTOR_TYPES.get(&self.v.actor(&Address::new_id(self.msg.from)).unwrap().code).unwrap();
        if types.into_iter().any(|t| *t == *to_match) {
            return Ok(());
        }
        Err(ActorError::unchecked(
            ExitCode::SYS_ASSERTION_FAILED,
            "immediate caller actor type forbidden".to_string(),
        ))
    }

    fn current_balance(&self) -> TokenAmount {
        self.v.actor(&self.to()).unwrap().balance
    }

    fn resolve_address(&self, addr: &Address) -> Option<ActorID> {
        if let Some(normalize_addr) = self.v.resolve_id_address(addr) {
            if let &Payload::ID(id) = normalize_addr.payload() {
                return Some(id);
            }
        }
        None
    }

    fn get_actor_code_cid(&self, id: &ActorID) -> Option<Cid> {
        let maybe_act = self.v.actor(&Address::new_id(*id));
        match maybe_act {
            None => None,
            Some(act) => Some(act.code),
        }
    }

    fn lookup_delegated_address(&self, id: ActorID) -> Option<Address> {
        self.v.actor(&Address::new_id(id)).and_then(|act| act.delegated_address)
    }

    fn send(
        &self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        _gas_limit: Option<u64>,
        mut send_flags: SendFlags,
    ) -> Result<Response, SendError> {
        // replicate FVM by silently propagating read only flag to subcalls
        if self.read_only() {
            send_flags.set(SendFlags::READ_ONLY, true)
        }

        if !*self.allow_side_effects.borrow() {
            return Ok(Response { exit_code: ExitCode::SYS_ASSERTION_FAILED, return_data: None });
        }

        let from_id = self.resolve_address(&self.to()).unwrap();

        let new_actor_msg = InternalMessage { from: from_id, to: *to, value, method, params };
        let mut new_ctx = InvocationCtx {
            v: self.v,
            top: self.top.clone(),
            msg: new_actor_msg,
            allow_side_effects: RefCell::new(true),
            caller_validated: RefCell::new(false),
            read_only: send_flags.read_only(),
            policy: self.policy,
            subinvocations: RefCell::new(vec![]),
            events: RefCell::new(vec![]),
        };
        let res = new_ctx.invoke();
        let invoc = new_ctx.gather_trace(res.clone());
        RefMut::map(self.subinvocations.borrow_mut(), |subinvocs| {
            subinvocs.push(invoc);
            subinvocs
        });

        Ok(Response {
            exit_code: res.as_ref().err().map(|e| e.exit_code()).unwrap_or(ExitCode::OK),
            return_data: res.unwrap_or_else(|mut e| e.take_data()),
        })
    }

    fn get_randomness_from_tickets(
        &self,
        _personalization: DomainSeparationTag,
        _rand_epoch: ChainEpoch,
        _entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError> {
        Ok(TEST_VM_RAND_ARRAY)
    }

    fn get_randomness_from_beacon(
        &self,
        _personalization: DomainSeparationTag,
        _rand_epoch: ChainEpoch,
        _entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError> {
        Ok(TEST_VM_RAND_ARRAY)
    }

    fn get_state_root(&self) -> Result<Cid, ActorError> {
        Ok(self.v.actor(&self.to()).unwrap().state)
    }

    fn set_state_root(&self, root: &Cid) -> Result<(), ActorError> {
        let maybe_act = self.v.actor(&self.to());
        match maybe_act {
            None => Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "actor does not exist".to_string(),
            )),
            Some(mut act) if !self.read_only() => {
                act.state = *root;
                self.v.set_actor(&self.to(), act);
                Ok(())
            }
            _ => Err(ActorError::unchecked(
                ExitCode::USR_READ_ONLY,
                "actor is read-only".to_string(),
            )),
        }
    }

    fn transaction<S, RT, F>(&self, f: F) -> Result<RT, ActorError>
    where
        S: Serialize + DeserializeOwned,
        F: FnOnce(&mut S, &Self) -> Result<RT, ActorError>,
    {
        let mut st = self.state::<S>().unwrap();
        self.allow_side_effects.replace(false);
        let result = f(&mut st, self);
        self.allow_side_effects.replace(true);
        let ret = result?;
        let mut act = self.v.actor(&self.to()).unwrap();
        act.state = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();

        if self.read_only {
            return Err(ActorError::unchecked(
                ExitCode::USR_READ_ONLY,
                "actor is read-only".to_string(),
            ));
        }

        self.v.set_actor(&self.to(), act);
        Ok(ret)
    }

    fn new_actor_address(&self) -> Result<Address, ActorError> {
        let mut b = self.top.originator_stable_addr.to_bytes();
        b.extend_from_slice(&self.top.originator_call_seq.to_be_bytes());
        b.extend_from_slice(&self.top.new_actor_addr_count.borrow().to_be_bytes());
        Ok(Address::new_actor(&b))
    }

    fn delete_actor(&self) -> Result<(), ActorError> {
        panic!("TODO implement me")
    }

    fn resolve_builtin_actor_type(&self, code_id: &Cid) -> Option<Type> {
        ACTOR_TYPES.get(code_id).cloned()
    }

    fn get_code_cid_for_type(&self, typ: Type) -> Cid {
        ACTOR_CODES.get(&typ).cloned().unwrap()
    }

    fn total_fil_circ_supply(&self) -> TokenAmount {
        self.top.circ_supply.clone()
    }

    fn charge_gas(&self, _name: &'static str, _compute: i64) {}

    fn base_fee(&self) -> TokenAmount {
        TokenAmount::zero()
    }

    fn actor_balance(&self, id: ActorID) -> Option<TokenAmount> {
        self.v.actor(&Address::new_id(id)).map(|act| act.balance)
    }

    fn gas_available(&self) -> u64 {
        u32::MAX.into()
    }

    fn tipset_timestamp(&self) -> u64 {
        0
    }

    fn tipset_cid(&self, _epoch: i64) -> Result<Cid, ActorError> {
        Ok(Cid::new_v1(IPLD_RAW, Multihash::wrap(0, b"faketipset").unwrap()))
    }

    fn emit_event(&self, event: &ActorEvent) -> Result<(), ActorError> {
        self.events
            .borrow_mut()
            .push(EmittedEvent { emitter: self.msg.to.id().unwrap(), event: event.clone() });
        Ok(())
    }

    fn read_only(&self) -> bool {
        self.read_only
    }
}

impl Primitives for InvocationCtx<'_> {
    fn verify_signature(
        &self,
        signature: &Signature,
        signer: &Address,
        plaintext: &[u8],
    ) -> Result<(), anyhow::Error> {
        self.v.primitives().verify_signature(signature, signer, plaintext)
    }

    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        self.v.primitives().hash_blake2b(data)
    }

    fn compute_unsealed_sector_cid(
        &self,
        proof_type: RegisteredSealProof,
        pieces: &[PieceInfo],
    ) -> Result<Cid, anyhow::Error> {
        self.v.primitives().compute_unsealed_sector_cid(proof_type, pieces)
    }

    fn hash(&self, hasher: SupportedHashes, data: &[u8]) -> Vec<u8> {
        self.v.primitives().hash(hasher, data)
    }

    fn hash_64(&self, hasher: SupportedHashes, data: &[u8]) -> ([u8; 64], usize) {
        self.v.primitives().hash_64(hasher, data)
    }

    fn recover_secp_public_key(
        &self,
        hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
        signature: &[u8; SECP_SIG_LEN],
    ) -> Result<[u8; SECP_PUB_LEN], anyhow::Error> {
        self.v.primitives().recover_secp_public_key(hash, signature)
    }

    fn verify_post(&self, verify_info: &WindowPoStVerifyInfo) -> Result<(), anyhow::Error> {
        for proof in &verify_info.proofs {
            if proof.proof_bytes.eq(&TEST_VM_INVALID_POST.as_bytes().to_vec()) {
                return Err(anyhow!("invalid proof"));
            }
        }

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

    fn verify_replica_update(&self, replica: &ReplicaUpdateInfo) -> Result<(), anyhow::Error> {
        self.v.primitives().verify_replica_update(replica)
    }
}

impl RuntimePolicy for InvocationCtx<'_> {
    fn policy(&self) -> &Policy {
        self.policy
    }
}

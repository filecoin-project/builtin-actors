use anyhow::anyhow;
use bimap::BiBTreeMap;
use cid::multihash::Code;
use cid::Cid;
use fil_actor_account::{Actor as AccountActor, State as AccountState};
use fil_actor_cron::{Actor as CronActor, Entry as CronEntry, State as CronState};
use fil_actor_evm::EvmContractActor;
use fil_actor_init::{Actor as InitActor, ExecReturn, State as InitState};
use fil_actor_market::{Actor as MarketActor, Method as MarketMethod, State as MarketState};
use fil_actor_miner::{Actor as MinerActor, State as MinerState};
use fil_actor_multisig::Actor as MultisigActor;
use fil_actor_paych::Actor as PaychActor;
use fil_actor_power::{Actor as PowerActor, Method as MethodPower, State as PowerState};
use fil_actor_reward::{Actor as RewardActor, State as RewardState};
use fil_actor_system::{Actor as SystemActor, State as SystemState};
use fil_actor_verifreg::{Actor as VerifregActor, State as VerifRegState};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{
    ActorCode, DomainSeparationTag, MessageInfo, Policy, Primitives, Runtime, RuntimePolicy,
    Verifier,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::MessageAccumulator;
use fil_actors_runtime::{
    ActorError, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, FIRST_NON_SINGLETON_ADDR, INIT_ACTOR_ADDR,
    REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fil_builtin_actors_state::check::check_state_invariants;
use fil_builtin_actors_state::check::Tree;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{Cbor, CborStore, RawBytes};
use fvm_ipld_hamt::{BytesKey, Hamt, Sha256};
use fvm_shared::address::Payload;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::consensus::ConsensusFault;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::crypto::signature::{
    Signature, SECP_PUB_LEN, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE,
};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PieceInfo;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{
    AggregateSealVerifyProofAndInfos, RegisteredSealProof, ReplicaUpdateInfo, SealVerifyInfo,
    StoragePower, WindowPoStVerifyInfo,
};
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use multihash::MultihashDigest;
use regex::Regex;
use serde::ser;
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::ops::Add;

pub mod util;

pub struct VM<'bs> {
    pub store: &'bs MemoryBlockstore,
    pub state_root: RefCell<Cid>,
    total_fil: TokenAmount,
    actors_dirty: RefCell<bool>,
    actors_cache: RefCell<HashMap<Address, Actor>>,
    empty_obj_cid: Cid,
    network_version: NetworkVersion,
    curr_epoch: ChainEpoch,
    invocations: RefCell<Vec<InvocationTrace>>,
}

pub struct MinerBalances {
    pub available_balance: TokenAmount,
    pub vesting_balance: TokenAmount,
    pub initial_pledge: TokenAmount,
    pub pre_commit_deposit: TokenAmount,
}

pub struct NetworkStats {
    pub total_raw_byte_power: StoragePower,
    pub total_bytes_committed: StoragePower,
    pub total_quality_adj_power: StoragePower,
    pub total_qa_bytes_committed: StoragePower,
    pub total_pledge_collateral: TokenAmount,
    pub this_epoch_raw_byte_power: StoragePower,
    pub this_epoch_quality_adj_power: StoragePower,
    pub this_epoch_pledge_collateral: TokenAmount,
    pub miner_count: i64,
    pub miner_above_min_power_count: i64,
    pub this_epoch_reward: TokenAmount,
    pub this_epoch_reward_smoothed: FilterEstimate,
    pub this_epoch_baseline_power: StoragePower,
    pub total_storage_power_reward: TokenAmount,
    pub total_client_locked_collateral: TokenAmount,
    pub total_provider_locked_collateral: TokenAmount,
    pub total_client_storage_fee: TokenAmount,
}

pub const VERIFREG_ROOT_KEY: &[u8] = &[200; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_VERIFREG_ROOT_SIGNER_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR);
pub const TEST_VERIFREG_ROOT_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 1);
// Account actor seeding funds created by new_with_singletons
pub const FAUCET_ROOT_KEY: &[u8] = &[153; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_FAUCET_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 2);
pub const FIRST_TEST_USER_ADDR: ActorID = FIRST_NON_SINGLETON_ADDR + 3;

// accounts for verifreg root signer and msig
impl<'bs> VM<'bs> {
    pub fn new(store: &'bs MemoryBlockstore) -> VM<'bs> {
        let mut actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::new(store);
        let empty = store.put_cbor(&(), Code::Blake2b256).unwrap();
        VM {
            store,
            state_root: RefCell::new(actors.flush().unwrap()),
            total_fil: TokenAmount::zero(),
            actors_dirty: RefCell::new(false),
            actors_cache: RefCell::new(HashMap::new()),
            empty_obj_cid: empty,
            network_version: NetworkVersion::V16,
            curr_epoch: ChainEpoch::zero(),
            invocations: RefCell::new(vec![]),
        }
    }

    pub fn with_total_fil(self, total_fil: TokenAmount) -> Self {
        Self { total_fil, ..self }
    }

    pub fn new_with_singletons(store: &'bs MemoryBlockstore) -> VM<'bs> {
        let reward_total = TokenAmount::from_whole(1_100_000_000i64);
        let faucet_total = TokenAmount::from_whole(1_000_000_000i64);

        let v = VM::new(store).with_total_fil(&reward_total + &faucet_total);

        // system
        let sys_st = SystemState::new(store).unwrap();
        let sys_head = v.put_store(&sys_st);
        let sys_value = faucet_total.clone(); // delegate faucet funds to system so we can construct faucet by sending to bls addr
        v.set_actor(SYSTEM_ACTOR_ADDR, actor(*SYSTEM_ACTOR_CODE_ID, sys_head, 0, sys_value));

        // init
        let init_st = InitState::new(store, "integration-test".to_string()).unwrap();
        let init_head = v.put_store(&init_st);
        v.set_actor(INIT_ACTOR_ADDR, actor(*INIT_ACTOR_CODE_ID, init_head, 0, TokenAmount::zero()));

        // reward

        let reward_head = v.put_store(&RewardState::new(StoragePower::zero()));
        v.set_actor(REWARD_ACTOR_ADDR, actor(*REWARD_ACTOR_CODE_ID, reward_head, 0, reward_total));

        // cron
        let builtin_entries = vec![
            CronEntry {
                receiver: STORAGE_POWER_ACTOR_ADDR,
                method_num: MethodPower::OnEpochTickEnd as u64,
            },
            CronEntry {
                receiver: STORAGE_MARKET_ACTOR_ADDR,
                method_num: MarketMethod::CronTick as u64,
            },
        ];
        let cron_head = v.put_store(&CronState { entries: builtin_entries });
        v.set_actor(CRON_ACTOR_ADDR, actor(*CRON_ACTOR_CODE_ID, cron_head, 0, TokenAmount::zero()));

        // power
        let power_head = v.put_store(&PowerState::new(&v.store).unwrap());
        v.set_actor(
            STORAGE_POWER_ACTOR_ADDR,
            actor(*POWER_ACTOR_CODE_ID, power_head, 0, TokenAmount::zero()),
        );

        // market
        let market_head = v.put_store(&MarketState::new(&v.store).unwrap());
        v.set_actor(
            STORAGE_MARKET_ACTOR_ADDR,
            actor(*MARKET_ACTOR_CODE_ID, market_head, 0, TokenAmount::zero()),
        );

        // verifreg
        // initialize verifreg root signer
        v.apply_message(
            INIT_ACTOR_ADDR,
            Address::new_bls(VERIFREG_ROOT_KEY).unwrap(),
            TokenAmount::zero(),
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap();
        let verifreg_root_signer =
            v.normalize_address(&Address::new_bls(VERIFREG_ROOT_KEY).unwrap()).unwrap();
        assert_eq!(TEST_VERIFREG_ROOT_SIGNER_ADDR, verifreg_root_signer);
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
                SYSTEM_ACTOR_ADDR,
                INIT_ACTOR_ADDR,
                TokenAmount::zero(),
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
        assert_eq!(TEST_VERIFREG_ROOT_ADDR, root_msig_addr);
        // verifreg
        let verifreg_head = v.put_store(&VerifRegState::new(&v.store, root_msig_addr).unwrap());
        v.set_actor(
            VERIFIED_REGISTRY_ACTOR_ADDR,
            actor(*VERIFREG_ACTOR_CODE_ID, verifreg_head, 0, TokenAmount::zero()),
        );

        // burnt funds
        let burnt_funds_head = v.put_store(&AccountState { address: BURNT_FUNDS_ACTOR_ADDR });
        v.set_actor(
            BURNT_FUNDS_ACTOR_ADDR,
            actor(*ACCOUNT_ACTOR_CODE_ID, burnt_funds_head, 0, TokenAmount::zero()),
        );

        // create a faucet with 1 billion FIL for setting up test accounts
        v.apply_message(
            SYSTEM_ACTOR_ADDR,
            Address::new_bls(FAUCET_ROOT_KEY).unwrap(),
            faucet_total,
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap();

        v.checkpoint();
        v
    }

    pub fn with_epoch(self, epoch: ChainEpoch) -> VM<'bs> {
        self.checkpoint();
        VM {
            store: self.store,
            state_root: self.state_root.clone(),
            total_fil: self.total_fil,
            actors_dirty: RefCell::new(false),
            actors_cache: RefCell::new(HashMap::new()),
            empty_obj_cid: self.empty_obj_cid,
            network_version: self.network_version,
            curr_epoch: epoch,
            invocations: RefCell::new(vec![]),
        }
    }

    pub fn get_miner_balance(&self, maddr: Address) -> MinerBalances {
        let a = self.get_actor(maddr).unwrap();
        let st = self.get_state::<MinerState>(maddr).unwrap();
        MinerBalances {
            available_balance: st.get_available_balance(&a.balance).unwrap(),
            vesting_balance: st.locked_funds,
            initial_pledge: st.initial_pledge,
            pre_commit_deposit: st.pre_commit_deposits,
        }
    }

    pub fn get_network_stats(&self) -> NetworkStats {
        let power_state = self.get_state::<PowerState>(STORAGE_POWER_ACTOR_ADDR).unwrap();
        let reward_state = self.get_state::<RewardState>(REWARD_ACTOR_ADDR).unwrap();
        let market_state = self.get_state::<MarketState>(STORAGE_MARKET_ACTOR_ADDR).unwrap();

        NetworkStats {
            total_raw_byte_power: power_state.total_raw_byte_power,
            total_bytes_committed: power_state.total_bytes_committed,
            total_quality_adj_power: power_state.total_quality_adj_power,
            total_qa_bytes_committed: power_state.total_qa_bytes_committed,
            total_pledge_collateral: power_state.total_pledge_collateral,
            this_epoch_raw_byte_power: power_state.this_epoch_raw_byte_power,
            this_epoch_quality_adj_power: power_state.this_epoch_quality_adj_power,
            this_epoch_pledge_collateral: power_state.this_epoch_pledge_collateral,
            miner_count: power_state.miner_count,
            miner_above_min_power_count: power_state.miner_above_min_power_count,
            this_epoch_reward: reward_state.this_epoch_reward,
            this_epoch_reward_smoothed: reward_state.this_epoch_reward_smoothed,
            this_epoch_baseline_power: reward_state.this_epoch_baseline_power,
            total_storage_power_reward: reward_state.total_storage_power_reward,
            total_client_locked_collateral: market_state.total_client_locked_collateral,
            total_provider_locked_collateral: market_state.total_provider_locked_collateral,
            total_client_storage_fee: market_state.total_client_storage_fee,
        }
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
        let actor = actors.get(&addr.to_bytes()).unwrap().cloned();
        actor.iter().for_each(|a| {
            self.actors_cache.borrow_mut().insert(addr, a.clone());
        });
        actor
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

        self.state_root.replace(actors.flush().unwrap());
        self.actors_dirty.replace(false);
        *self.state_root.borrow()
    }

    pub fn rollback(&self, root: Cid) {
        self.actors_cache.replace(HashMap::new());
        self.state_root.replace(root);
        self.actors_dirty.replace(false);
    }

    pub fn normalize_address(&self, addr: &Address) -> Option<Address> {
        let st = self.get_state::<InitState>(INIT_ACTOR_ADDR).unwrap();
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

    pub fn get_epoch(&self) -> ChainEpoch {
        self.curr_epoch
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

        // big.Mul(big.NewInt(1e9), big.NewInt(1e18))
        // make top level context with internal context
        let top = TopCtx {
            originator_stable_addr: from,
            _originator_call_seq: call_seq,
            new_actor_addr_count: RefCell::new(0),
            circ_supply: TokenAmount::from_whole(1_000_000_000),
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

    /// Checks the state invariants and returns broken invariants.
    pub fn check_state_invariants(&self) -> anyhow::Result<MessageAccumulator> {
        self.checkpoint();
        let actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::load(
            &self.state_root.borrow(),
            self.store,
        )
        .unwrap();

        let mut manifest = BiBTreeMap::new();
        actors
            .for_each(|_, actor| {
                manifest.insert(actor.code, ACTOR_TYPES.get(&actor.code).unwrap().to_owned());
                Ok(())
            })
            .unwrap();

        let policy = Policy::default();
        let state_tree = Tree::load(&self.store, &self.state_root.borrow()).unwrap();
        check_state_invariants(
            &manifest,
            &policy,
            state_tree,
            &self.total_fil,
            self.get_epoch() - 1,
        )
    }

    /// Asserts state invariants are held without any errors.
    pub fn assert_state_invariants(&self) {
        self.check_state_invariants().unwrap().assert_empty()
    }

    /// Checks state, allowing expected invariants to fail. The invariants *must* fail in the
    /// provided order.
    pub fn expect_state_invariants(&self, expected_patterns: &[Regex]) {
        self.check_state_invariants().unwrap().assert_expected(expected_patterns)
    }

    pub fn get_total_actor_balance(
        &self,
        store: &MemoryBlockstore,
    ) -> anyhow::Result<TokenAmount, anyhow::Error> {
        let state_tree = Tree::load(store, &self.checkpoint())?;

        let mut total = TokenAmount::zero();
        state_tree.for_each(|_, actor| {
            total += &actor.balance.clone();
            Ok(())
        })?;
        Ok(total)
    }
}

#[derive(Clone)]
pub struct TopCtx {
    originator_stable_addr: Address,
    _originator_call_seq: u64,
    new_actor_addr_count: RefCell<u64>,
    circ_supply: TokenAmount,
}

#[derive(Clone, Debug)]
pub struct InternalMessage {
    from: Address,
    to: Address,
    value: TokenAmount,
    method: MethodNum,
    params: RawBytes,
}

impl InternalMessage {
    pub fn value(&self) -> TokenAmount {
        self.value.clone()
    }
}

impl MessageInfo for InvocationCtx<'_, '_> {
    fn caller(&self) -> Address {
        self.msg.from
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
}

pub const TEST_VM_RAND_STRING: &str = "i_am_random_____i_am_random_____";
pub const TEST_VM_INVALID: &str = "i_am_invalid";

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
        let protocol = target.protocol();
        match protocol {
            Protocol::Actor | Protocol::ID => {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_INVALID_RECEIVER,
                    format!("cannot create account for address {} type {}", target, protocol),
                ));
            }
            _ => (),
        }
        let mut st = self.v.get_state::<InitState>(INIT_ACTOR_ADDR).unwrap();
        let target_id = st.map_address_to_new_id(self.v.store, target).unwrap();
        let target_id_addr = Address::new_id(target_id);
        let mut init_actor = self.v.get_actor(INIT_ACTOR_ADDR).unwrap();
        init_actor.head = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.v.set_actor(INIT_ACTOR_ADDR, init_actor);

        let new_actor_msg = InternalMessage {
            from: SYSTEM_ACTOR_ADDR,
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
        let mut msg = self.msg.clone();
        msg.to = match self.resolve_target(&self.msg.to) {
            Ok((_, addr)) => addr, // use normalized address in trace
            _ => self.msg.to, // if target resolution fails don't fail whole invoke, just use non normalized
        };
        InvocationTrace { msg, code, ret, subinvocations: self.subinvocations.take() }
    }

    fn to(&'_ self) -> Address {
        self.resolve_target(&self.msg.to).unwrap().1
    }

    fn invoke(&mut self) -> Result<RawBytes, ActorError> {
        let prior_root = self.v.checkpoint();

        // Transfer funds
        let mut from_actor = self.v.get_actor(self.msg.from).unwrap();
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
        }

        // Load, deduct, store from actor before loading to actor to handle self-send case
        from_actor.balance -= &self.msg.value;
        self.v.set_actor(self.msg.from, from_actor);

        let (mut to_actor, to_addr) = self.resolve_target(&self.msg.to)?;
        to_actor.balance = to_actor.balance.add(&self.msg.value);
        self.v.set_actor(to_addr, to_actor);

        // Exit early on send
        if self.msg.method == METHOD_SEND {
            return Ok(RawBytes::default());
        }

        // call target actor
        let to_actor = self.v.get_actor(to_addr).unwrap();
        let params = self.msg.params.clone();
        let res = match ACTOR_TYPES.get(&to_actor.code).expect("Target actor is not a builtin") {
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
            Type::EVM => EvmContractActor::invoke_method(self, self.msg.method, &params),
        };
        if res.is_err() {
            self.v.rollback(prior_root)
        };
        res
    }
}

impl<'invocation, 'bs> Runtime<&'bs MemoryBlockstore> for InvocationCtx<'invocation, 'bs> {
    fn create_actor(&mut self, code_id: Cid, actor_id: ActorID) -> Result<(), ActorError> {
        match NON_SINGLETON_CODES.get(&code_id) {
            Some(_) => (),
            None => {
                return Err(ActorError::unchecked(
                    ExitCode::SYS_ASSERTION_FAILED,
                    "create_actor called with singleton builtin actor code cid".to_string(),
                ));
            }
        }
        let addr = Address::new_id(actor_id);
        if self.v.get_actor(addr).is_some() {
            return Err(ActorError::unchecked(
                ExitCode::SYS_ASSERTION_FAILED,
                "attempt to create new actor at existing address".to_string(),
            ));
        }
        let a = actor(code_id, self.v.empty_obj_cid, 0, TokenAmount::zero());
        self.v.set_actor(addr, a);
        Ok(())
    }

    fn store(&self) -> &&'bs MemoryBlockstore {
        &self.v.store
    }

    fn network_version(&self) -> NetworkVersion {
        self.v.network_version
    }

    fn message(&self) -> &dyn MessageInfo {
        self
    }

    fn curr_epoch(&self) -> ChainEpoch {
        self.v.get_epoch()
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
        self.v.get_actor(self.to()).unwrap().balance
    }

    fn resolve_address(&self, addr: &Address) -> Option<ActorID> {
        if let Some(normalize_addr) = self.v.normalize_address(addr) {
            if let &Payload::ID(id) = normalize_addr.payload() {
                return Some(id);
            }
        }
        None
    }

    fn get_actor_code_cid(&self, id: &ActorID) -> Option<Cid> {
        let maybe_act = self.v.get_actor(Address::new_id(*id));
        match maybe_act {
            None => None,
            Some(act) => Some(act.code),
        }
    }

    fn send(
        &self,
        to: &Address,
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

        let new_actor_msg = InternalMessage { from: self.to(), to: *to, value, method, params };
        let mut new_ctx = InvocationCtx {
            v: self.v,
            top: self.top.clone(),
            msg: new_actor_msg,
            allow_side_effects: true,
            caller_validated: false,
            policy: self.policy,
            subinvocations: RefCell::new(vec![]),
        };
        let res = new_ctx.invoke();

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
        Ok(Randomness(TEST_VM_RAND_STRING.as_bytes().into()))
    }

    fn get_randomness_from_beacon(
        &self,
        _personalization: DomainSeparationTag,
        _rand_epoch: ChainEpoch,
        _entropy: &[u8],
    ) -> Result<Randomness, ActorError> {
        Ok(Randomness(TEST_VM_RAND_STRING.as_bytes().into()))
    }

    fn create<C: Cbor>(&mut self, obj: &C) -> Result<(), ActorError> {
        let maybe_act = self.v.get_actor(self.to());
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
                    self.v.set_actor(self.to(), act);
                    Ok(())
                }
            }
        }
    }

    fn state<C: Cbor>(&self) -> Result<C, ActorError> {
        Ok(self.v.get_state::<C>(self.to()).unwrap())
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
        let mut act = self.v.get_actor(self.to()).unwrap();
        act.head = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.v.set_actor(self.to(), act);
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
        self.top.circ_supply.clone()
    }

    fn charge_gas(&mut self, _name: &'static str, _compute: i64) {}

    fn base_fee(&self) -> TokenAmount {
        TokenAmount::zero()
    }
}

impl Primitives for VM<'_> {
    fn verify_signature(
        &self,
        signature: &Signature,
        _signer: &Address,
        _plaintext: &[u8],
    ) -> Result<(), anyhow::Error> {
        if signature.bytes.clone() == TEST_VM_INVALID.as_bytes() {
            return Err(anyhow::format_err!(
                "verify signature syscall failing on TEST_VM_INVALID_SIG"
            ));
        }
        Ok(())
    }

    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        blake2b_simd::Params::new()
            .hash_length(32)
            .to_state()
            .update(data)
            .finalize()
            .as_bytes()
            .try_into()
            .unwrap()
    }

    fn compute_unsealed_sector_cid(
        &self,
        _proof_type: RegisteredSealProof,
        _pieces: &[PieceInfo],
    ) -> Result<Cid, anyhow::Error> {
        Ok(make_piece_cid(b"unsealed from itest vm"))
    }

    fn hash(&self, hasher: SupportedHashes, data: &[u8]) -> Vec<u8> {
        let hasher = Code::try_from(hasher as u64).unwrap(); // supported hashes are all implemented in multihash
        hasher.digest(data).to_bytes()
    }

    fn recover_secp_public_key(
        &self,
        hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
        signature: &[u8; SECP_SIG_LEN],
    ) -> Result<[u8; SECP_PUB_LEN], anyhow::Error> {
        recover_secp_public_key(hash, signature).map_err(|_| anyhow!("failed to recover pubkey"))
    }
}

impl Primitives for InvocationCtx<'_, '_> {
    fn verify_signature(
        &self,
        signature: &Signature,
        signer: &Address,
        plaintext: &[u8],
    ) -> Result<(), anyhow::Error> {
        self.v.verify_signature(signature, signer, plaintext)
    }

    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        self.v.hash_blake2b(data)
    }

    fn compute_unsealed_sector_cid(
        &self,
        proof_type: RegisteredSealProof,
        pieces: &[PieceInfo],
    ) -> Result<Cid, anyhow::Error> {
        self.v.compute_unsealed_sector_cid(proof_type, pieces)
    }

    #[cfg(feature = "m2-native")]
    fn install_actor(&self, _: &Cid) -> Result<(), anyhow::Error> {
        panic!("TODO implement me")
    }

    fn hash(&self, hasher: SupportedHashes, data: &[u8]) -> Vec<u8> {
        self.v.hash(hasher, data)
    }

    fn recover_secp_public_key(
        &self,
        hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
        signature: &[u8; SECP_SIG_LEN],
    ) -> Result<[u8; SECP_PUB_LEN], anyhow::Error> {
        self.v.recover_secp_public_key(hash, signature)
    }
}

impl Verifier for InvocationCtx<'_, '_> {
    fn verify_seal(&self, _vi: &SealVerifyInfo) -> Result<(), anyhow::Error> {
        Ok(())
    }

    fn verify_post(&self, verify_info: &WindowPoStVerifyInfo) -> Result<(), anyhow::Error> {
        for proof in &verify_info.proofs {
            if proof.proof_bytes.eq(&TEST_VM_INVALID.as_bytes().to_vec()) {
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

    fn verify_replica_update(&self, _replica: &ReplicaUpdateInfo) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

impl RuntimePolicy for InvocationCtx<'_, '_> {
    fn policy(&self) -> &Policy {
        self.policy
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MessageResult {
    pub code: ExitCode,
    pub ret: RawBytes,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Eq, Debug)]
pub struct Actor {
    pub code: Cid,
    pub head: Cid,
    pub call_seq_num: u64,
    pub balance: TokenAmount,
}

pub fn actor(code: Cid, head: Cid, seq: u64, bal: TokenAmount) -> Actor {
    Actor { code, head, call_seq_num: seq, balance: bal }
}

#[derive(Clone)]
pub struct InvocationTrace {
    pub msg: InternalMessage,
    pub code: Option<ExitCode>,
    pub ret: Option<RawBytes>,
    pub subinvocations: Vec<InvocationTrace>,
}

pub struct ExpectInvocation {
    pub to: Address,
    // required
    pub method: MethodNum,
    // required
    pub code: Option<ExitCode>,
    pub from: Option<Address>,
    pub value: Option<TokenAmount>,
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
        if let Some(v) = &self.value {
            assert_eq!(
                v, &invoc.msg.value,
                "{} unexpected value: expected:{}was:{} ",
                id, v, invoc.msg.value
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

impl Default for ExpectInvocation {
    fn default() -> Self {
        Self {
            method: 0,
            to: Address::new_id(0),
            code: None,
            from: None,
            value: None,
            params: None,
            ret: None,
            subinvocs: None,
        }
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

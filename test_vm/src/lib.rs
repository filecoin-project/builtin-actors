use crate::fakes::FakePrimitives;
use anyhow::anyhow;
use cid::multihash::Code;
use cid::Cid;
use fil_actor_account::{Actor as AccountActor, State as AccountState};
use fil_actor_cron::{Actor as CronActor, Entry as CronEntry, State as CronState};
use fil_actor_datacap::{Actor as DataCapActor, State as DataCapState};
use fil_actor_eam::EamActor;
use fil_actor_ethaccount::EthAccountActor;
use fil_actor_evm::EvmContractActor;
use fil_actor_init::{Actor as InitActor, ExecReturn, State as InitState};
use fil_actor_market::{Actor as MarketActor, Method as MarketMethod, State as MarketState};
use fil_actor_miner::{Actor as MinerActor, MinerInfo};
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
    Verifier, EMPTY_ARR_CID,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{actor_error, SendError};
use fil_actors_runtime::{
    ActorError, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, EAM_ACTOR_ADDR, FIRST_NON_SINGLETON_ADDR,
    INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fil_actors_runtime::{MessageAccumulator, DATACAP_TOKEN_ACTOR_ADDR};
use fil_builtin_actors_state::check::Tree;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_hamt::{BytesKey, Hamt, Sha256};
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
use fvm_shared::randomness::Randomness;
use fvm_shared::randomness::RANDOMNESS_LENGTH;
use fvm_shared::sector::{
    AggregateSealVerifyProofAndInfos, RegisteredSealProof, ReplicaUpdateInfo, SealVerifyInfo,
    StoragePower, WindowPoStVerifyInfo,
};
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, Response, IPLD_RAW, METHOD_CONSTRUCTOR, METHOD_SEND};
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{ser, Serialize};
use std::cell::{RefCell, RefMut};
use std::collections::{BTreeMap, HashMap};
use std::ops::Add;
use trace::InvocationTrace;

use crate::util::get_state;
use crate::util::serialize_ok;

pub mod deals;
pub mod expects;
pub mod fakes;
pub mod trace;
pub mod util;

mod vm;
pub use vm::*;

/// An in-memory rust-execution VM for testing that yields sensible stack traces and debug info
pub struct TestVM<'bs, BS>
where
    BS: Blockstore,
{
    pub primitives: FakePrimitives,
    pub store: &'bs BS,
    pub state_root: RefCell<Cid>,
    circulating_supply: RefCell<TokenAmount>,
    actors_dirty: RefCell<bool>,
    actors_cache: RefCell<HashMap<Address, ActorState>>,
    network_version: NetworkVersion,
    curr_epoch: RefCell<ChainEpoch>,
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

// accounts for verifreg root signer and msig
pub const VERIFREG_ROOT_KEY: &[u8] = &[200; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_VERIFREG_ROOT_SIGNER_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR);
pub const TEST_VERIFREG_ROOT_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 1);
// account actor seeding funds created by new_with_singletons
pub const FAUCET_ROOT_KEY: &[u8] = &[153; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_FAUCET_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 2);
pub const FIRST_TEST_USER_ADDR: ActorID = FIRST_NON_SINGLETON_ADDR + 3;

impl<'bs, BS> VM for TestVM<'bs, BS>
where
    BS: Blockstore,
{
    fn blockstore(&self) -> &dyn Blockstore {
        self.store
    }

    fn epoch(&self) -> ChainEpoch {
        *self.curr_epoch.borrow()
    }

    fn execute_message(
        &self,
        from: &Address,
        to: &Address,
        value: &TokenAmount,
        method: MethodNum,
        params: Option<IpldBlock>,
    ) -> Result<MessageResult, VMError> {
        let from_id = &self.resolve_id_address(from).unwrap();
        let mut a = self.get_actor(from_id).unwrap();
        let call_seq = a.call_seq;
        a.call_seq = call_seq + 1;
        // EthAccount abstractions turns Placeholders into EthAccounts
        if a.code == *PLACEHOLDER_ACTOR_CODE_ID {
            a.code = *ETHACCOUNT_ACTOR_CODE_ID;
        }
        self.set_actor(from_id, a);

        let prior_root = self.checkpoint();

        // big.Mul(big.NewInt(1e9), big.NewInt(1e18))
        // make top level context with internal context
        let top = TopCtx {
            originator_stable_addr: *from,
            originator_call_seq: call_seq,
            new_actor_addr_count: RefCell::new(0),
            circ_supply: TokenAmount::from_whole(1_000_000_000),
        };
        let msg = InternalMessage { from: *from_id, to: *to, value: value.clone(), method, params };
        let mut new_ctx = InvocationCtx {
            v: self,
            top,
            msg,
            allow_side_effects: RefCell::new(true),
            caller_validated: RefCell::new(false),
            read_only: false,
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
            Err(mut ae) => {
                self.rollback(prior_root);
                Ok(MessageResult {
                    code: ae.exit_code(),
                    message: ae.msg().to_string(),
                    ret: ae.take_data(),
                })
            }
            Ok(ret) => {
                self.checkpoint();
                Ok(MessageResult { code: ExitCode::OK, message: "OK".to_string(), ret })
            }
        }
    }

    fn execute_message_implicit(
        &self,
        from: &Address,
        to: &Address,
        value: &TokenAmount,
        method: MethodNum,
        params: Option<IpldBlock>,
    ) -> Result<MessageResult, VMError> {
        self.execute_message(from, to, value, method, params)
    }
    fn resolve_id_address(&self, address: &Address) -> Option<Address> {
        let st: InitState = get_state(self, &INIT_ACTOR_ADDR).unwrap();
        st.resolve_address::<BS>(self.store, address).unwrap()
    }

    fn set_epoch(&self, epoch: ChainEpoch) {
        self.curr_epoch.replace(epoch);
    }

    fn balance(&self, address: &Address) -> TokenAmount {
        let a = self.get_actor(address);
        a.map_or(TokenAmount::zero(), |a| a.balance)
    }

    fn take_invocations(&self) -> Vec<InvocationTrace> {
        self.invocations.take()
    }

    fn actor(&self, address: &Address) -> Option<ActorState> {
        // check for inclusion in cache of changed actors
        if let Some(act) = self.actors_cache.borrow().get(address) {
            return Some(act.clone());
        }
        // go to persisted map
        let actors = Hamt::<&'bs BS, ActorState, BytesKey, Sha256>::load(
            &self.state_root.borrow(),
            self.store,
        )
        .unwrap();
        let actor = actors.get(&address.to_bytes()).unwrap().cloned();
        actor.iter().for_each(|a| {
            self.actors_cache.borrow_mut().insert(*address, a.clone());
        });
        actor
    }

    fn primitives(&self) -> &dyn Primitives {
        &self.primitives
    }

    fn actor_manifest(&self) -> BTreeMap<Cid, Type> {
        ACTOR_TYPES.clone()
    }

    fn state_root(&self) -> Cid {
        *self.state_root.borrow()
    }

    fn circulating_supply(&self) -> TokenAmount {
        self.circulating_supply.borrow().clone()
    }

    fn set_circulating_supply(&self, supply: TokenAmount) {
        self.circulating_supply.replace(supply);
    }
}

impl<'bs, BS> TestVM<'bs, BS>
where
    BS: Blockstore,
{
    pub fn new(store: &'bs MemoryBlockstore) -> TestVM<'bs, MemoryBlockstore> {
        let mut actors = Hamt::<&'bs MemoryBlockstore, ActorState, BytesKey, Sha256>::new(store);
        TestVM {
            primitives: FakePrimitives {},
            store,
            state_root: RefCell::new(actors.flush().unwrap()),
            circulating_supply: RefCell::new(TokenAmount::zero()),
            actors_dirty: RefCell::new(false),
            actors_cache: RefCell::new(HashMap::new()),
            network_version: NetworkVersion::V16,
            curr_epoch: RefCell::new(ChainEpoch::zero()),
            invocations: RefCell::new(vec![]),
        }
    }

    pub fn new_with_singletons(store: &'bs MemoryBlockstore) -> TestVM<'bs, MemoryBlockstore> {
        let reward_total = TokenAmount::from_whole(1_100_000_000i64);
        let faucet_total = TokenAmount::from_whole(1_000_000_000i64);

        let v = TestVM::<'_, MemoryBlockstore>::new(store);
        v.set_circulating_supply(&reward_total + &faucet_total);

        // system
        let sys_st = SystemState::new(store).unwrap();
        let sys_head = v.put_store(&sys_st);
        let sys_value = faucet_total.clone(); // delegate faucet funds to system so we can construct faucet by sending to bls addr
        v.set_actor(&SYSTEM_ACTOR_ADDR, actor(*SYSTEM_ACTOR_CODE_ID, sys_head, 0, sys_value, None));

        // init
        let init_st = InitState::new(store, "integration-test".to_string()).unwrap();
        let init_head = v.put_store(&init_st);
        v.set_actor(
            &INIT_ACTOR_ADDR,
            actor(*INIT_ACTOR_CODE_ID, init_head, 0, TokenAmount::zero(), None),
        );

        // reward

        let reward_head = v.put_store(&RewardState::new(StoragePower::zero()));
        v.set_actor(
            &REWARD_ACTOR_ADDR,
            actor(*REWARD_ACTOR_CODE_ID, reward_head, 0, reward_total, None),
        );

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
        v.set_actor(
            &CRON_ACTOR_ADDR,
            actor(*CRON_ACTOR_CODE_ID, cron_head, 0, TokenAmount::zero(), None),
        );

        // power
        let power_head = v.put_store(&PowerState::new(&v.store).unwrap());
        v.set_actor(
            &STORAGE_POWER_ACTOR_ADDR,
            actor(*POWER_ACTOR_CODE_ID, power_head, 0, TokenAmount::zero(), None),
        );

        // market
        let market_head = v.put_store(&MarketState::new(&v.store).unwrap());
        v.set_actor(
            &STORAGE_MARKET_ACTOR_ADDR,
            actor(*MARKET_ACTOR_CODE_ID, market_head, 0, TokenAmount::zero(), None),
        );

        // verifreg
        // initialize verifreg root signer
        v.execute_message(
            &INIT_ACTOR_ADDR,
            &Address::new_bls(VERIFREG_ROOT_KEY).unwrap(),
            &TokenAmount::zero(),
            METHOD_SEND,
            None,
        )
        .unwrap();
        let verifreg_root_signer =
            v.resolve_id_address(&Address::new_bls(VERIFREG_ROOT_KEY).unwrap()).unwrap();
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
            .execute_message(
                &SYSTEM_ACTOR_ADDR,
                &INIT_ACTOR_ADDR,
                &TokenAmount::zero(),
                fil_actor_init::Method::Exec as u64,
                Some(serialize_ok(&fil_actor_init::ExecParams {
                    code_cid: *MULTISIG_ACTOR_CODE_ID,
                    constructor_params: msig_ctor_params,
                })),
            )
            .unwrap()
            .ret
            .unwrap()
            .deserialize()
            .unwrap();
        let root_msig_addr = msig_ctor_ret.id_address;
        assert_eq!(TEST_VERIFREG_ROOT_ADDR, root_msig_addr);
        // verifreg
        let verifreg_head = v.put_store(&VerifRegState::new(&v.store, root_msig_addr).unwrap());
        v.set_actor(
            &VERIFIED_REGISTRY_ACTOR_ADDR,
            actor(*VERIFREG_ACTOR_CODE_ID, verifreg_head, 0, TokenAmount::zero(), None),
        );

        // Ethereum Address Manager
        v.set_actor(
            &EAM_ACTOR_ADDR,
            actor(*EAM_ACTOR_CODE_ID, EMPTY_ARR_CID, 0, TokenAmount::zero(), None),
        );

        // datacap
        let datacap_head =
            v.put_store(&DataCapState::new(&v.store, VERIFIED_REGISTRY_ACTOR_ADDR).unwrap());
        v.set_actor(
            &DATACAP_TOKEN_ACTOR_ADDR,
            actor(*DATACAP_TOKEN_ACTOR_CODE_ID, datacap_head, 0, TokenAmount::zero(), None),
        );

        // burnt funds
        let burnt_funds_head = v.put_store(&AccountState { address: BURNT_FUNDS_ACTOR_ADDR });
        v.set_actor(
            &BURNT_FUNDS_ACTOR_ADDR,
            actor(*ACCOUNT_ACTOR_CODE_ID, burnt_funds_head, 0, TokenAmount::zero(), None),
        );

        // create a faucet with 1 billion FIL for setting up test accounts
        v.execute_message(
            &SYSTEM_ACTOR_ADDR,
            &Address::new_bls(FAUCET_ROOT_KEY).unwrap(),
            &faucet_total,
            METHOD_SEND,
            None,
        )
        .unwrap();

        v.checkpoint();
        v
    }

    pub fn with_epoch(self, epoch: ChainEpoch) -> TestVM<'bs, BS> {
        self.checkpoint();
        TestVM {
            primitives: FakePrimitives {},
            store: self.store,
            state_root: self.state_root.clone(),
            circulating_supply: self.circulating_supply,
            actors_dirty: RefCell::new(false),
            actors_cache: RefCell::new(HashMap::new()),
            network_version: self.network_version,
            curr_epoch: RefCell::new(epoch),
            invocations: RefCell::new(vec![]),
        }
    }

    pub fn put_store<S>(&self, obj: &S) -> Cid
    where
        S: ser::Serialize,
    {
        self.store.put_cbor(obj, Code::Blake2b256).unwrap()
    }

    pub fn get_actor(&self, addr: &Address) -> Option<ActorState> {
        // check for inclusion in cache of changed actors
        if let Some(act) = self.actors_cache.borrow().get(addr) {
            return Some(act.clone());
        }
        // go to persisted map
        let actors = Hamt::<&'bs BS, ActorState, BytesKey, Sha256>::load(
            &self.state_root.borrow(),
            self.store,
        )
        .unwrap();
        let actor = actors.get(&addr.to_bytes()).unwrap().cloned();
        actor.iter().for_each(|a| {
            self.actors_cache.borrow_mut().insert(*addr, a.clone());
        });
        actor
    }

    // blindly overwrite the actor at this address whether it previously existed or not
    pub fn set_actor(&self, key: &Address, a: ActorState) {
        self.actors_cache.borrow_mut().insert(*key, a);
        self.actors_dirty.replace(true);
    }

    pub fn checkpoint(&self) -> Cid {
        // persist cache on top of latest checkpoint and clear
        let mut actors = Hamt::<&'bs BS, ActorState, BytesKey, Sha256>::load(
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

    pub fn mutate_state<S, F>(&self, addr: &Address, f: F)
    where
        S: Serialize + DeserializeOwned,
        F: FnOnce(&mut S),
    {
        let mut a = self.get_actor(addr).unwrap();
        let mut st = self.store.get_cbor::<S>(&a.state).unwrap().unwrap();
        f(&mut st);
        a.state = self.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.set_actor(addr, a);
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
    originator_call_seq: u64,
    new_actor_addr_count: RefCell<u64>,
    circ_supply: TokenAmount,
}

#[derive(Clone, Debug)]
pub struct InternalMessage {
    from: Address,
    to: Address,
    value: TokenAmount,
    method: MethodNum,
    params: Option<IpldBlock>,
}

impl<BS> MessageInfo for InvocationCtx<'_, '_, BS>
where
    BS: Blockstore,
{
    fn nonce(&self) -> u64 {
        self.top.originator_call_seq
    }
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
    fn gas_premium(&self) -> TokenAmount {
        TokenAmount::zero()
    }
}

pub const TEST_VM_RAND_ARRAY: [u8; 32] = [
    1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32,
];
pub const TEST_VM_INVALID_POST: &str = "i_am_invalid_post";

pub struct InvocationCtx<'invocation, 'bs, BS>
where
    BS: Blockstore,
{
    v: &'invocation TestVM<'bs, BS>,
    top: TopCtx,
    msg: InternalMessage,
    allow_side_effects: RefCell<bool>,
    caller_validated: RefCell<bool>,
    read_only: bool,
    policy: &'invocation Policy,
    subinvocations: RefCell<Vec<InvocationTrace>>,
}

impl<'invocation, 'bs, BS> InvocationCtx<'invocation, 'bs, BS>
where
    BS: Blockstore,
{
    fn resolve_target(
        &'invocation self,
        target: &Address,
    ) -> Result<(ActorState, Address), ActorError> {
        if let Some(a) = self.v.resolve_id_address(target) {
            if let Some(act) = self.v.get_actor(&a) {
                return Ok((act, a));
            }
        };

        // Address does not yet exist, create it
        let is_account = match target.payload() {
            Payload::Secp256k1(_) | Payload::BLS(_) => true,
            Payload::Delegated(da)
            // Validate that there's an actor at the target ID (we don't care what is there,
            // just that something is there).
            if self.v.get_actor(&Address::new_id(da.namespace())).is_some() =>
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
        let (target_id, existing) = st.map_addresses_to_id(self.v.store, target, None).unwrap();
        assert!(!existing, "should never have existing actor when no f4 address is specified");
        let target_id_addr = Address::new_id(target_id);
        let mut init_actor = self.v.get_actor(&INIT_ACTOR_ADDR).unwrap();
        init_actor.state = self.v.store.put_cbor(&st, Code::Blake2b256).unwrap();
        self.v.set_actor(&INIT_ACTOR_ADDR, init_actor);

        let new_actor_msg = InternalMessage {
            from: SYSTEM_ACTOR_ADDR,
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

        Ok((self.v.get_actor(&target_id_addr).unwrap(), target_id_addr))
    }

    fn gather_trace(
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
            code,
            ret,
            subinvocations: self.subinvocations.take(),
        }
    }

    fn to(&'_ self) -> Address {
        self.resolve_target(&self.msg.to).unwrap().1
    }

    fn invoke(&mut self) -> Result<Option<IpldBlock>, ActorError> {
        let prior_root = self.v.checkpoint();

        // Transfer funds
        let mut from_actor = self.v.get_actor(&self.msg.from).unwrap();
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
        self.v.set_actor(&self.msg.from, from_actor);

        let (mut to_actor, ref to_addr) = self.resolve_target(&self.msg.to)?;
        to_actor.balance = to_actor.balance.add(&self.msg.value);
        self.v.set_actor(to_addr, to_actor);

        // Exit early on send
        if self.msg.method == METHOD_SEND {
            return Ok(None);
        }

        // call target actor
        let to_actor = self.v.get_actor(to_addr).unwrap();
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

impl<'invocation, 'bs, BS> Runtime for InvocationCtx<'invocation, 'bs, BS>
where
    BS: Blockstore,
{
    type Blockstore = &'bs BS;

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
        let actor = match self.v.get_actor(addr) {
            Some(mut act) if act.code == *PLACEHOLDER_ACTOR_CODE_ID => {
                act.code = code_id;
                act
            }
            None => actor(code_id, EMPTY_ARR_CID, 0, TokenAmount::zero(), predictable_address),
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

    fn store(&self) -> &&'bs BS {
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
            if *addr == self.msg.from {
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
        let to_match = ACTOR_TYPES.get(&self.v.get_actor(&self.msg.from).unwrap().code).unwrap();
        if types.into_iter().any(|t| *t == *to_match) {
            return Ok(());
        }
        Err(ActorError::unchecked(
            ExitCode::SYS_ASSERTION_FAILED,
            "immediate caller actor type forbidden".to_string(),
        ))
    }

    fn current_balance(&self) -> TokenAmount {
        self.v.get_actor(&self.to()).unwrap().balance
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
        let maybe_act = self.v.get_actor(&Address::new_id(*id));
        match maybe_act {
            None => None,
            Some(act) => Some(act.code),
        }
    }

    fn lookup_delegated_address(&self, id: ActorID) -> Option<Address> {
        self.v.get_actor(&Address::new_id(id)).and_then(|act| act.predictable_address)
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

        let new_actor_msg = InternalMessage { from: self.to(), to: *to, value, method, params };
        let mut new_ctx = InvocationCtx {
            v: self.v,
            top: self.top.clone(),
            msg: new_actor_msg,
            allow_side_effects: RefCell::new(true),
            caller_validated: RefCell::new(false),
            read_only: send_flags.read_only(),
            policy: self.policy,
            subinvocations: RefCell::new(vec![]),
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
        Ok(self.v.get_actor(&self.to()).unwrap().state)
    }

    fn set_state_root(&self, root: &Cid) -> Result<(), ActorError> {
        let maybe_act = self.v.get_actor(&self.to());
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
        let mut act = self.v.get_actor(&self.to()).unwrap();
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

    fn delete_actor(&self, _beneficiary: &Address) -> Result<(), ActorError> {
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
        self.v.get_actor(&Address::new_id(id)).map(|act| act.balance)
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

    // TODO No support for events yet.
    fn emit_event(&self, _event: &ActorEvent) -> Result<(), ActorError> {
        unimplemented!()
    }

    fn read_only(&self) -> bool {
        self.read_only
    }
}

impl<BS> Primitives for InvocationCtx<'_, '_, BS>
where
    BS: Blockstore,
{
    fn verify_signature(
        &self,
        signature: &Signature,
        signer: &Address,
        plaintext: &[u8],
    ) -> Result<(), anyhow::Error> {
        self.v.primitives.verify_signature(signature, signer, plaintext)
    }

    fn hash_blake2b(&self, data: &[u8]) -> [u8; 32] {
        self.v.primitives.hash_blake2b(data)
    }

    fn compute_unsealed_sector_cid(
        &self,
        proof_type: RegisteredSealProof,
        pieces: &[PieceInfo],
    ) -> Result<Cid, anyhow::Error> {
        self.v.primitives.compute_unsealed_sector_cid(proof_type, pieces)
    }

    fn hash(&self, hasher: SupportedHashes, data: &[u8]) -> Vec<u8> {
        self.v.primitives.hash(hasher, data)
    }

    fn hash_64(&self, hasher: SupportedHashes, data: &[u8]) -> ([u8; 64], usize) {
        self.v.primitives.hash_64(hasher, data)
    }

    fn recover_secp_public_key(
        &self,
        hash: &[u8; SECP_SIG_MESSAGE_HASH_SIZE],
        signature: &[u8; SECP_SIG_LEN],
    ) -> Result<[u8; SECP_PUB_LEN], anyhow::Error> {
        self.v.primitives.recover_secp_public_key(hash, signature)
    }
}

impl<BS> Verifier for InvocationCtx<'_, '_, BS>
where
    BS: Blockstore,
{
    fn verify_seal(&self, _vi: &SealVerifyInfo) -> Result<(), anyhow::Error> {
        Ok(())
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

    fn verify_replica_update(&self, _replica: &ReplicaUpdateInfo) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

impl<BS> RuntimePolicy for InvocationCtx<'_, '_, BS>
where
    BS: Blockstore,
{
    fn policy(&self) -> &Policy {
        self.policy
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MessageResult {
    pub code: ExitCode,
    pub message: String,
    pub ret: Option<IpldBlock>,
}

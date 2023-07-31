use crate::fakes::FakePrimitives;

use cid::multihash::Code;
use cid::Cid;
use fil_actor_account::State as AccountState;
use fil_actor_cron::{Entry as CronEntry, State as CronState};
use fil_actor_datacap::State as DataCapState;
use fil_actor_init::{ExecReturn, State as InitState};
use fil_actor_market::{Method as MarketMethod, State as MarketState};
use fil_actor_miner::MinerInfo;
use fil_actor_power::{Method as MethodPower, State as PowerState};
use fil_actor_reward::State as RewardState;
use fil_actor_system::State as SystemState;
use fil_actor_verifreg::State as VerifRegState;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{Policy, Primitives, EMPTY_ARR_CID};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{MessageAccumulator, DATACAP_TOKEN_ACTOR_ADDR};
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, EAM_ACTOR_ADDR, FIRST_NON_SINGLETON_ADDR,
    INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fil_builtin_actors_state::check::Tree;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_hamt::{BytesKey, Hamt, Sha256};
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, METHOD_SEND};
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{ser, Serialize};
use std::cell::{RefCell, RefMut};
use std::collections::{BTreeMap, HashMap};
use vm_api::trace::InvocationTrace;
use vm_api::{actor, ActorState, MessageResult, VMError, VM};

use vm_api::util::{get_state, serialize_ok};

pub mod deals;
pub mod expects;
pub mod fakes;
pub mod util;

mod messaging;
pub use messaging::*;

/// An in-memory rust-execution VM for testing builtin-actors that yields sensible stack traces and debug info
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

// accounts for verifreg root signer and msig
pub const VERIFREG_ROOT_KEY: &[u8] = &[200; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_VERIFREG_ROOT_SIGNER_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR);
pub const TEST_VERIFREG_ROOT_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 1);

// account actor seeding funds created by new_with_singletons
pub const FAUCET_ROOT_KEY: &[u8] = &[153; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_FAUCET_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 2);
pub const FIRST_TEST_USER_ADDR: ActorID = FIRST_NON_SINGLETON_ADDR + 3;

// static values for predictable testing
pub const TEST_VM_RAND_ARRAY: [u8; 32] = [
    1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32,
];
pub const TEST_VM_INVALID_POST: &str = "i_am_invalid_post";

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

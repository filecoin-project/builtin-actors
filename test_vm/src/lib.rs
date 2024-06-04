use cid::multihash::Code;
use cid::Cid;
use fil_actor_account::State as AccountState;
use fil_actor_cron::{Entry as CronEntry, State as CronState};
use fil_actor_datacap::State as DataCapState;
use fil_actor_init::{ExecReturn, State as InitState};
use fil_actor_market::{Method as MarketMethod, State as MarketState};
use fil_actor_power::{Method as MethodPower, State as PowerState};
use fil_actor_reward::State as RewardState;
use fil_actor_system::State as SystemState;
use fil_actor_verifreg::State as VerifRegState;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{Policy, Primitives, EMPTY_ARR_CID};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use fil_actors_runtime::DATACAP_TOKEN_ACTOR_ADDR;
use fil_actors_runtime::{test_utils::*, Map2, DEFAULT_HAMT_CONFIG};
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, EAM_ACTOR_ADDR, INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_hamt::{BytesKey, Hamt, Sha256};
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::StoragePower;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{MethodNum, METHOD_SEND};
use serde::ser;
use std::cell::{RefCell, RefMut};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use vm_api::trace::InvocationTrace;
use vm_api::{new_actor, ActorState, MessageResult, MockPrimitives, VMError, VM};

use vm_api::util::{get_state, serialize_ok};

mod constants;
pub use constants::*;
mod messaging;
pub use messaging::*;

/// An in-memory rust-execution VM for testing builtin-actors that yields sensible stack traces and debug info
pub struct TestVM {
    pub primitives: FakePrimitives,
    pub store: Rc<MemoryBlockstore>,
    pub state_root: RefCell<Cid>,
    actors_dirty: RefCell<bool>,
    actors_cache: RefCell<HashMap<Address, ActorState>>,
    invocations: RefCell<Vec<InvocationTrace>>,
    // MachineContext equivalents
    network_version: NetworkVersion,
    curr_epoch: RefCell<ChainEpoch>,
    circulating_supply: RefCell<TokenAmount>,
    base_fee: RefCell<TokenAmount>,
    timestamp: RefCell<u64>,
}

impl TestVM {
    pub fn new(store: impl Into<Rc<MemoryBlockstore>>) -> TestVM {
        let store = store.into();
        let mut actors =
            Hamt::<Rc<MemoryBlockstore>, ActorState, BytesKey, Sha256>::new_with_config(
                Rc::clone(&store),
                DEFAULT_HAMT_CONFIG,
            );

        TestVM {
            primitives: FakePrimitives::default(),
            store,
            state_root: RefCell::new(actors.flush().unwrap()),
            circulating_supply: RefCell::new(TokenAmount::zero()),
            actors_dirty: RefCell::new(false),
            actors_cache: RefCell::new(HashMap::new()),
            network_version: NetworkVersion::V16,
            curr_epoch: RefCell::new(ChainEpoch::zero()),
            invocations: RefCell::new(vec![]),
            base_fee: RefCell::new(TokenAmount::zero()),
            timestamp: RefCell::new(0),
        }
    }

    pub fn new_with_singletons(store: impl Into<Rc<MemoryBlockstore>>) -> TestVM {
        let reward_total = TokenAmount::from_whole(1_100_000_000i64);
        let faucet_total = TokenAmount::from_whole(1_000_000_000i64);

        let store = store.into();

        let v = TestVM::new(Rc::clone(&store));
        v.set_circulating_supply(&reward_total + &faucet_total);

        // system
        let sys_st = SystemState::new(&store).unwrap();
        let sys_head = v.put_store(&sys_st);
        let sys_value = faucet_total.clone(); // delegate faucet funds to system so we can construct faucet by sending to bls addr
        v.set_actor(
            &SYSTEM_ACTOR_ADDR,
            new_actor(*SYSTEM_ACTOR_CODE_ID, sys_head, 0, sys_value, None),
        );

        // init
        let init_st = InitState::new(&store, "integration-test".to_string()).unwrap();
        let init_head = v.put_store(&init_st);
        v.set_actor(
            &INIT_ACTOR_ADDR,
            new_actor(*INIT_ACTOR_CODE_ID, init_head, 0, TokenAmount::zero(), None),
        );

        // reward

        let reward_head = v.put_store(&RewardState::new(StoragePower::zero()));
        v.set_actor(
            &REWARD_ACTOR_ADDR,
            new_actor(*REWARD_ACTOR_CODE_ID, reward_head, 0, reward_total, None),
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
            new_actor(*CRON_ACTOR_CODE_ID, cron_head, 0, TokenAmount::zero(), None),
        );

        // power
        let power_head = v.put_store(&PowerState::new(&v.store).unwrap());
        v.set_actor(
            &STORAGE_POWER_ACTOR_ADDR,
            new_actor(*POWER_ACTOR_CODE_ID, power_head, 0, TokenAmount::zero(), None),
        );

        // market
        let market_head = v.put_store(&MarketState::new(&v.store).unwrap());
        v.set_actor(
            &STORAGE_MARKET_ACTOR_ADDR,
            new_actor(*MARKET_ACTOR_CODE_ID, market_head, 0, TokenAmount::zero(), None),
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
            new_actor(*VERIFREG_ACTOR_CODE_ID, verifreg_head, 0, TokenAmount::zero(), None),
        );

        // Ethereum Address Manager
        v.set_actor(
            &EAM_ACTOR_ADDR,
            new_actor(*EAM_ACTOR_CODE_ID, EMPTY_ARR_CID, 0, TokenAmount::zero(), None),
        );

        // datacap
        let datacap_head =
            v.put_store(&DataCapState::new(&v.store, VERIFIED_REGISTRY_ACTOR_ADDR).unwrap());
        v.set_actor(
            &DATACAP_TOKEN_ACTOR_ADDR,
            new_actor(*DATACAP_TOKEN_ACTOR_CODE_ID, datacap_head, 0, TokenAmount::zero(), None),
        );

        // burnt funds
        let burnt_funds_head = v.put_store(&AccountState { address: BURNT_FUNDS_ACTOR_ADDR });
        v.set_actor(
            &BURNT_FUNDS_ACTOR_ADDR,
            new_actor(*ACCOUNT_ACTOR_CODE_ID, burnt_funds_head, 0, TokenAmount::zero(), None),
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

    pub fn put_store<S>(&self, obj: &S) -> Cid
    where
        S: ser::Serialize,
    {
        self.store.put_cbor(obj, Code::Blake2b256).unwrap()
    }

    pub fn checkpoint(&self) -> Cid {
        // persist cache on top of latest checkpoint and clear
        let mut actors =
            Hamt::<Rc<MemoryBlockstore>, ActorState, BytesKey, Sha256>::load_with_config(
                &self.state_root.borrow(),
                Rc::clone(&self.store),
                DEFAULT_HAMT_CONFIG,
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

    fn actor_map(&self) -> Map2<&MemoryBlockstore, Address, ActorState> {
        Map2::load(self.store.as_ref(), &self.checkpoint(), DEFAULT_HAMT_CONFIG, "actors").unwrap()
    }
}

impl VM for TestVM {
    fn blockstore(&self) -> &dyn Blockstore {
        self.store.as_ref()
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
        // TODO: for non-implicit calls validate that from_id is either the
        // account actor or the ethereum account actor and error otherwise
        let mut a = self.actor(from_id).unwrap();
        let call_seq = a.sequence;
        a.sequence = call_seq + 1;
        // EthAccount abstractions turns Placeholders into EthAccounts
        if a.code == *PLACEHOLDER_ACTOR_CODE_ID {
            // TODO: for non-implicit calls validate that the actor has a
            // delegated f4 address in the EAM's namespace
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
        let msg = InternalMessage {
            from: from_id.id().unwrap(),
            to: *to,
            value: value.clone(),
            method,
            params,
        };
        let mut new_ctx = InvocationCtx {
            v: self,
            top,
            msg,
            allow_side_effects: RefCell::new(true),
            caller_validated: RefCell::new(false),
            read_only: false,
            policy: &Policy::default(),
            subinvocations: RefCell::new(vec![]),
            events: RefCell::new(vec![]),
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
        st.resolve_address(&self.store, address).unwrap()
    }

    fn balance(&self, address: &Address) -> TokenAmount {
        let a = self.actor(address);
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
        let actors = self.actor_map();
        let actor = actors.get(address).unwrap().cloned();
        actor.iter().for_each(|a| {
            self.actors_cache.borrow_mut().insert(*address, a.clone());
        });
        actor
    }

    fn set_actor(&self, key: &Address, a: ActorState) {
        self.actors_cache.borrow_mut().insert(*key, a);
        self.actors_dirty.replace(true);
    }

    fn primitives(&self) -> &dyn Primitives {
        &self.primitives
    }

    fn actor_manifest(&self) -> BTreeMap<Cid, Type> {
        ACTOR_TYPES.clone()
    }

    fn actor_states(&self) -> BTreeMap<Address, ActorState> {
        let map = self.actor_map();
        let mut tree = BTreeMap::new();
        map.for_each(|k, v| {
            tree.insert(k, v.clone());
            Ok(())
        })
        .unwrap();

        tree
    }

    fn epoch(&self) -> ChainEpoch {
        *self.curr_epoch.borrow()
    }

    fn set_epoch(&self, epoch: ChainEpoch) {
        self.curr_epoch.replace(epoch);
    }
    fn circulating_supply(&self) -> TokenAmount {
        self.circulating_supply.borrow().clone()
    }

    fn set_circulating_supply(&self, supply: TokenAmount) {
        self.circulating_supply.replace(supply);
    }

    fn base_fee(&self) -> TokenAmount {
        self.base_fee.borrow().clone()
    }

    fn set_base_fee(&self, amount: TokenAmount) {
        self.base_fee.replace(amount);
    }

    fn timestamp(&self) -> u64 {
        *self.timestamp.borrow()
    }

    fn set_timestamp(&self, timestamp: u64) {
        self.timestamp.replace(timestamp);
    }

    fn mut_primitives(&self) -> &dyn MockPrimitives {
        &self.primitives
    }
}

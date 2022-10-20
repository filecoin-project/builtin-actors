use fil_actor_verifreg_state_v8::testing::check_state_invariants;
use fil_actors_runtime::runtime::Runtime;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::{MethodNum, HAMT_BIT_WIDTH};
use lazy_static::lazy_static;

use fil_actor_verifreg_state_v8::{
    Actor as VerifregActor, AddVerifierClientParams, AddVerifierParams, DataCap, Method,
    RestoreBytesParams, State, UseBytesParams,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, make_map_with_root_and_bitwidth, ActorError, Map, STORAGE_MARKET_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};

lazy_static! {
    pub static ref ROOT_ADDR: Address = Address::new_id(101);
}

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: *ROOT_ADDR,
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

pub fn new_harness() -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = Harness { root: *ROOT_ADDR };
    h.construct_and_verify(&mut rt, &h.root);
    (h, rt)
}

pub struct Harness {
    pub root: Address,
}

impl Harness {
    pub fn construct_and_verify(&self, rt: &mut MockRuntime, root_param: &Address) {
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<VerifregActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(root_param).unwrap(),
            )
            .unwrap();

        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        let empty_map = make_empty_map::<_, ()>(&rt.store, HAMT_BIT_WIDTH).flush().unwrap();
        let state: State = rt.get_state();
        assert_eq!(self.root, state.root_key);
        assert_eq!(empty_map, state.verified_clients);
        assert_eq!(empty_map, state.verifiers);
    }

    pub fn add_verifier(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
        allowance: &DataCap,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![self.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.root);
        let params = AddVerifierParams { address: *verifier, allowance: allowance.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::AddVerifier as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        self.assert_verifier_allowance(rt, verifier, allowance);
        Ok(())
    }

    pub fn remove_verifier(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![self.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, self.root);
        let ret = rt.call::<VerifregActor>(
            Method::RemoveVerifier as MethodNum,
            &RawBytes::serialize(verifier).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        self.assert_verifier_removed(rt, verifier);
        Ok(())
    }

    pub fn assert_verifier_allowance(
        &self,
        rt: &MockRuntime,
        verifier: &Address,
        allowance: &DataCap,
    ) {
        let verifier_id_addr = rt.get_id_address(verifier).unwrap();
        assert_eq!(*allowance, self.get_verifier_allowance(rt, &verifier_id_addr));
    }

    pub fn get_verifier_allowance(&self, rt: &MockRuntime, verifier: &Address) -> DataCap {
        let verifiers = load_verifiers(rt);
        let BigIntDe(allowance) = verifiers.get(&verifier.to_bytes()).unwrap().unwrap();
        allowance.clone()
    }

    pub fn assert_verifier_removed(&self, rt: &MockRuntime, verifier: &Address) {
        let verifier_id_addr = rt.get_id_address(verifier).unwrap();
        let verifiers = load_verifiers(rt);
        assert!(!verifiers.contains_key(&verifier_id_addr.to_bytes()).unwrap())
    }

    pub fn add_client(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
        client: &Address,
        allowance: &DataCap,
        expected_allowance: &DataCap,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_any();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *verifier);
        let params = AddVerifierClientParams { address: *client, allowance: allowance.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::AddVerifiedClient as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        // Confirm the verifier was added to state.
        self.assert_client_allowance(rt, client, expected_allowance);
        Ok(())
    }

    pub fn assert_client_allowance(&self, rt: &MockRuntime, client: &Address, allowance: &DataCap) {
        let client_id_addr = rt.get_id_address(client).unwrap();
        assert_eq!(*allowance, self.get_client_allowance(rt, &client_id_addr));
    }

    pub fn get_client_allowance(&self, rt: &MockRuntime, client: &Address) -> DataCap {
        let clients = load_clients(rt);
        let BigIntDe(allowance) = clients.get(&client.to_bytes()).unwrap().unwrap();
        allowance.clone()
    }

    pub fn assert_client_removed(&self, rt: &MockRuntime, client: &Address) {
        let client_id_addr = rt.get_id_address(client).unwrap();
        let clients = load_clients(rt);
        assert!(!clients.contains_key(&client_id_addr.to_bytes()).unwrap())
    }

    pub fn add_verifier_and_client(
        &self,
        rt: &mut MockRuntime,
        verifier: &Address,
        client: &Address,
        verifier_allowance: &DataCap,
        client_allowance: &DataCap,
    ) {
        self.add_verifier(rt, verifier, verifier_allowance).unwrap();
        self.add_client(rt, verifier, client, client_allowance, client_allowance).unwrap();
    }

    pub fn use_bytes(
        &self,
        rt: &mut MockRuntime,
        client: &Address,
        amount: &DataCap,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, STORAGE_MARKET_ACTOR_ADDR);
        let params = UseBytesParams { address: *client, deal_size: amount.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::UseBytes as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();
        Ok(())
    }

    pub fn restore_bytes(
        &self,
        rt: &mut MockRuntime,
        client: &Address,
        amount: &DataCap,
    ) -> Result<(), ActorError> {
        rt.expect_validate_caller_addr(vec![STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, STORAGE_MARKET_ACTOR_ADDR);
        let params = RestoreBytesParams { address: *client, deal_size: amount.clone() };
        let ret = rt.call::<VerifregActor>(
            Method::RestoreBytes as MethodNum,
            &RawBytes::serialize(params).unwrap(),
        )?;
        assert_eq!(RawBytes::default(), ret);
        rt.verify();
        Ok(())
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        let (_, acc) = check_state_invariants(&rt.get_state(), rt.store());
        acc.assert_empty();
    }
}

fn load_verifiers(rt: &MockRuntime) -> Map<MemoryBlockstore, BigIntDe> {
    let state: State = rt.get_state();
    make_map_with_root_and_bitwidth::<_, BigIntDe>(
        &state.verifiers,
        rt.store.as_ref(),
        HAMT_BIT_WIDTH,
    )
    .unwrap()
}

fn load_clients(rt: &MockRuntime) -> Map<MemoryBlockstore, BigIntDe> {
    let state: State = rt.get_state();
    make_map_with_root_and_bitwidth::<_, BigIntDe>(
        &state.verified_clients,
        rt.store.as_ref(),
        HAMT_BIT_WIDTH,
    )
    .unwrap()
}

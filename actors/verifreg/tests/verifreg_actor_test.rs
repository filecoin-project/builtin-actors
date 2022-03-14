#![deny(unused_must_use)] // Force unwrapping Result<_, Err>

use fvm_shared::{HAMT_BIT_WIDTH, MethodNum};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::blockstore::MemoryBlockstore;
use fvm_shared::encoding::RawBytes;
use lazy_static::lazy_static;

use fil_actor_verifreg::{
    Actor as VerifregActor, AddVerifierClientParams, AddVerifierParams, DataCap, Method, MINIMUM_VERIFIED_DEAL_SIZE,
    State,
};
use fil_actors_runtime::{
    ActorError, make_empty_map, make_map_with_root_and_bitwidth, Map, SYSTEM_ACTOR_ADDR,
};
use fil_actors_runtime::test_utils::*;

lazy_static! {
    static ref ROOT_ADDR: Address = Address::new_id(101);
    static ref VERIFIER_ALLOWANCE: DataCap = MINIMUM_VERIFIED_DEAL_SIZE.clone() + DataCap::from(42);
    static ref CLIENT_ALLOWANCE: DataCap = VERIFIER_ALLOWANCE.clone() - DataCap::from(1);
}

fn construct_runtime() -> MockRuntime {
    MockRuntime {
        receiver: *ROOT_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

fn make_harness() -> (Harness, MockRuntime) {
    let mut rt = construct_runtime();
    let h = Harness { root: *ROOT_ADDR };
    h.construct_and_verify(&mut rt, &h.root);
    return (h, rt);
}

mod construction {
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::encoding::RawBytes;
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actor_verifreg::{Actor as VerifregActor, Method};
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
    use fil_actors_runtime::test_utils::*;

    use crate::{construct_runtime, Harness};

    #[test]
    fn construct_with_root_id() {
        let mut rt = construct_runtime();
        let h = Harness { root: Address::new_id(101) };
        h.construct_and_verify(&mut rt, &h.root);
        h.check_state();
    }

    #[test]
    fn construct_resolves_non_id() {
        let mut rt = construct_runtime();
        let h = Harness { root: Address::new_id(101) };
        let root_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.insert(root_pubkey, h.root);
        h.construct_and_verify(&mut rt, &root_pubkey);
        h.check_state();
    }

    #[test]
    fn construct_fails_if_root_unresolved() {
        let mut rt = construct_runtime();
        let root_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();

        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        expect_abort(
            ExitCode::ErrIllegalArgument,
            rt.call::<VerifregActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(root_pubkey).unwrap(),
            ),
        );
    }
}

mod verifiers {
    use fvm_shared::{METHOD_SEND, MethodNum};
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::encoding::RawBytes;
    use fvm_shared::error::ExitCode;

    use fil_actor_verifreg::{
        Actor as VerifregActor, AddVerifierParams, DataCap, Method, MINIMUM_VERIFIED_DEAL_SIZE,
    };
    use fil_actors_runtime::test_utils::*;

    use crate::{make_harness, ROOT_ADDR, VERIFIER_ALLOWANCE};

    #[test]
    fn add_verifier_requires_root_caller() {
        let (h, mut rt) = make_harness();

        rt.expect_validate_caller_addr(vec![h.root]);
        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, Address::new_id(501));
        let params = AddVerifierParams {
            address: Address::new_id(201),
            allowance: VERIFIER_ALLOWANCE.clone(),
        };
        expect_abort(
            ExitCode::SysErrForbidden,
            rt.call::<VerifregActor>(
                Method::AddVerifier as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state();
    }

    #[test]
    fn add_verifier_enforces_min_size() {
        let (h, mut rt) = make_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() - DataCap::from(1);
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_verifier(&mut rt, &Address::new_id(201), &allowance),
        );
        h.check_state();
    }

    #[test]
    fn add_verifier_rejects_root() {
        let (h, mut rt) = make_harness();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_verifier(&mut rt, &ROOT_ADDR, &VERIFIER_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn add_verifier_rejects_client() {
        let (h, mut rt) = make_harness();
        let client = Address::new_id(202);
        h.add_verifier_and_client(
            &mut rt,
            &Address::new_id(201),
            &client,
            &VERIFIER_ALLOWANCE,
            &VERIFIER_ALLOWANCE,
        );
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_verifier(&mut rt, &client, &VERIFIER_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn add_verifier_rejects_unresolved_address() {
        let (h, mut rt) = make_harness();
        let verifier_key_address = Address::new_secp256k1(&[3u8; 65]).unwrap();
        // Expect runtime to attempt to create the actor, but don't add it to the mock's
        // address resolution table.
        rt.expect_send(
            verifier_key_address,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::default(),
            RawBytes::default(),
            ExitCode::Ok,
        );
        expect_abort(
            ExitCode::ErrIllegalState,
            h.add_verifier(&mut rt, &verifier_key_address, &VERIFIER_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn add_verifier_id_address() {
        let (h, mut rt) = make_harness();
        h.add_verifier(&mut rt, &Address::new_id(201), &VERIFIER_ALLOWANCE).unwrap();
        h.check_state();
    }

    #[test]
    fn add_verifier_resolves_address() {
        let (h, mut rt) = make_harness();
        let pubkey_addr = Address::new_secp256k1(&[0u8; 65]).unwrap();
        rt.id_addresses.insert(pubkey_addr, Address::new_id(201));

        h.add_verifier(&mut rt, &pubkey_addr, &VERIFIER_ALLOWANCE).unwrap();
        h.check_state();
    }

    #[test]
    fn remove_requires_root() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();

        rt.expect_validate_caller_addr(vec![h.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(501));
        expect_abort(
            ExitCode::SysErrForbidden,
            rt.call::<VerifregActor>(
                Method::RemoveVerifier as MethodNum,
                &RawBytes::serialize(verifier).unwrap(),
            ),
        );
        h.check_state();
    }

    #[test]
    fn remove_requires_verifier_exists() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(501);
        expect_abort(ExitCode::ErrIllegalArgument, h.remove_verifier(&mut rt, &verifier));
        h.check_state();
    }

    #[test]
    fn remove_verifier() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();
        h.remove_verifier(&mut rt, &verifier).unwrap();
        h.check_state();
    }

    #[test]
    fn remove_verifier_id_address() {
        let (h, mut rt) = make_harness();
        let verifier_pubkey = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
        let verifier_id = Address::new_id(201);
        rt.id_addresses.insert(verifier_pubkey, verifier_id);
        // Add using pubkey address.
        h.add_verifier(&mut rt, &verifier_pubkey, &VERIFIER_ALLOWANCE).unwrap();
        // Remove using ID address.
        h.remove_verifier(&mut rt, &verifier_id).unwrap();
        h.check_state();
    }
}

mod clients {
    use fvm_shared::{METHOD_SEND, MethodNum};
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::encoding::RawBytes;
    use fvm_shared::error::ExitCode;

    use fil_actor_verifreg::{
        Actor as VerifregActor, AddVerifierClientParams, DataCap, Method,
        MINIMUM_VERIFIED_DEAL_SIZE,
    };
    use fil_actors_runtime::test_utils::*;

    use crate::{CLIENT_ALLOWANCE, make_harness, VERIFIER_ALLOWANCE};

    #[test]
    fn many_verifiers_and_clients() {
        let (h, mut rt) = make_harness();
        let verifier1 = Address::new_id(201);
        let verifier2 = Address::new_id(202);

        // Each verifier has enough allowance for two clients.
        let verifier_allowance = CLIENT_ALLOWANCE.clone() + CLIENT_ALLOWANCE.clone();
        h.add_verifier(&mut rt, &verifier1, &verifier_allowance).unwrap();
        h.add_verifier(&mut rt, &verifier2, &verifier_allowance).unwrap();

        let client1 = Address::new_id(301);
        let client2 = Address::new_id(302);
        h.add_client(&mut rt, &verifier1, &client1, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();
        h.add_client(&mut rt, &verifier1, &client2, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();

        let client3 = Address::new_id(303);
        let client4 = Address::new_id(304);
        h.add_client(&mut rt, &verifier2, &client3, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();
        h.add_client(&mut rt, &verifier2, &client4, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();

        // all clients should exist and verifiers should have no more allowance left
        h.assert_client_allowance(&rt, &client1, &CLIENT_ALLOWANCE);
        h.assert_client_allowance(&rt, &client2, &CLIENT_ALLOWANCE);
        h.assert_client_allowance(&rt, &client3, &CLIENT_ALLOWANCE);
        h.assert_client_allowance(&rt, &client4, &CLIENT_ALLOWANCE);
        h.assert_verifier_allowance(&rt, &verifier1, &DataCap::from(0));
        h.assert_verifier_allowance(&rt, &verifier2, &DataCap::from(0));
        h.check_state();
    }

    #[test]
    fn verifier_allowance_exhausted() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        // Verifier only has allowance for one client.
        h.add_verifier(&mut rt, &verifier, &CLIENT_ALLOWANCE).unwrap();

        let client1 = Address::new_id(301);
        h.add_client(&mut rt, &verifier, &client1, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();
        let client2 = Address::new_id(302);
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &verifier, &client2, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );

        // One client should exist and verifier should have no more allowance left.
        h.assert_client_allowance(&rt, &client1, &CLIENT_ALLOWANCE);
        h.assert_verifier_allowance(&rt, &verifier, &DataCap::from(0));
        h.check_state();
    }

    #[test]
    fn resolves_client_address() {
        let (h, mut rt) = make_harness();

        let client_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        let client_id = Address::new_id(301);
        rt.id_addresses.insert(client_pubkey, client_id);

        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();
        h.add_client(&mut rt, &verifier, &client_pubkey, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE)
            .unwrap();

        // Adding another verified client with the same ID address increments
        // the data cap which has already been granted.
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();
        let expected_allowance = CLIENT_ALLOWANCE.clone() + CLIENT_ALLOWANCE.clone();
        h.add_client(&mut rt, &verifier, &client_id, &CLIENT_ALLOWANCE, &expected_allowance)
            .unwrap();
        h.check_state();
    }

    #[test]
    fn minimum_allowance_ok() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();

        let client = Address::new_id(301);
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        h.add_client(&mut rt, &verifier, &client, &allowance, &allowance).unwrap();
        h.check_state();
    }

    #[test]
    fn rejects_unresolved_address() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();

        let client = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        // Expect runtime to attempt to create the actor, but don't add it to the mock's
        // address resolution table.
        rt.expect_send(
            client,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::default(),
            RawBytes::default(),
            ExitCode::Ok,
        );

        expect_abort(
            ExitCode::ErrIllegalState,
            h.add_client(&mut rt, &verifier, &client, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn rejects_allowance_below_minimum() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();

        let client = Address::new_id(301);
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() - DataCap::from(1);
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &verifier, &client, &allowance, &allowance),
        );
        h.check_state();
    }

    #[test]
    fn rejects_non_verifier_caller() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();

        let client = Address::new_id(301);
        let caller = Address::new_id(209);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        rt.expect_validate_caller_any();
        let params =
            AddVerifierClientParams { address: client, allowance: CLIENT_ALLOWANCE.clone() };
        expect_abort(
            ExitCode::ErrNotFound,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state();
    }

    #[test]
    fn rejects_allowance_greater_than_verifier_cap() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();

        let allowance = VERIFIER_ALLOWANCE.clone() + DataCap::from(1);
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &verifier, &h.root, &allowance, &allowance),
        );
        h.check_state();
    }

    #[test]
    fn rejects_root_as_client() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &verifier, &h.root, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn rejects_verifier_as_client() {
        let (h, mut rt) = make_harness();
        let verifier = Address::new_id(201);
        h.add_verifier(&mut rt, &verifier, &VERIFIER_ALLOWANCE).unwrap();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &verifier, &verifier, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );

        let another_verifier = Address::new_id(202);
        h.add_verifier(&mut rt, &another_verifier, &VERIFIER_ALLOWANCE).unwrap();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(
                &mut rt,
                &verifier,
                &another_verifier,
                &CLIENT_ALLOWANCE,
                &CLIENT_ALLOWANCE,
            ),
        );

        h.check_state();
    }
}

///// Test harness /////

struct Harness {
    root: Address,
}

impl Harness {
    fn construct_and_verify(&self, rt: &mut MockRuntime, root_param: &Address) {
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<VerifregActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(root_param).unwrap(),
            )
            .unwrap();

        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        let empty_map = make_empty_map::<_, ()>(&rt.store, HAMT_BIT_WIDTH).flush().unwrap();

        let state: State = rt.get_state().unwrap();

        assert_eq!(self.root, state.root_key);
        assert_eq!(empty_map, state.verified_clients);
        assert_eq!(empty_map, state.verifiers);
    }

    fn add_verifier(
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

        self.assert_verifier_allowance(rt, &verifier, allowance);
        Ok(())
    }

    fn remove_verifier(&self, rt: &mut MockRuntime, verifier: &Address) -> Result<(), ActorError> {
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

    fn assert_verifier_allowance(&self, rt: &MockRuntime, verifier: &Address, allowance: &DataCap) {
        let verifier_id_addr = rt.get_id_address(verifier).unwrap();
        assert_eq!(*allowance, self.get_verifier_allowance(rt, &verifier_id_addr));
    }

    fn get_verifier_allowance(&self, rt: &MockRuntime, verifier: &Address) -> DataCap {
        let verifiers = load_verifiers(&rt);
        let BigIntDe(allowance) = verifiers.get(&verifier.to_bytes()).unwrap().unwrap();
        return allowance.clone();
    }

    fn assert_verifier_removed(&self, rt: &MockRuntime, verifier: &Address) {
        let verifier_id_addr = rt.get_id_address(verifier).unwrap();
        let verifiers = load_verifiers(&rt);
        assert_eq!(false, verifiers.contains_key(&verifier_id_addr.to_bytes()).unwrap())
    }

    fn add_client(
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
        let client_id_addr = rt.get_id_address(&client).unwrap();
        assert_eq!(*expected_allowance, self.get_client_allowance(rt, &client_id_addr));
        Result::Ok(())
    }

    fn assert_client_allowance(&self, rt: &MockRuntime, client: &Address, allowance: &DataCap) {
        let client_id_addr = rt.get_id_address(client).unwrap();
        assert_eq!(*allowance, self.get_client_allowance(rt, &client_id_addr));
    }

    fn get_client_allowance(&self, rt: &MockRuntime, client: &Address) -> DataCap {
        let clients = load_clients(&rt);
        let BigIntDe(allowance) = clients.get(&client.to_bytes()).unwrap().unwrap();
        return allowance.clone();
    }

    fn add_verifier_and_client(
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

    fn check_state(&self) {
        // TODO: https://github.com/filecoin-project/builtin-actors/issues/44
    }
}

fn load_verifiers(rt: &MockRuntime) -> Map<MemoryBlockstore, BigIntDe> {
    let state: State = rt.get_state().unwrap();
    make_map_with_root_and_bitwidth::<_, BigIntDe>(&state.verifiers, &rt.store, HAMT_BIT_WIDTH)
        .unwrap()
}

fn load_clients(rt: &MockRuntime) -> Map<MemoryBlockstore, BigIntDe> {
    let state: State = rt.get_state().unwrap();
    make_map_with_root_and_bitwidth::<_, BigIntDe>(
        &state.verified_clients,
        &rt.store,
        HAMT_BIT_WIDTH,
    )
    .unwrap()
}

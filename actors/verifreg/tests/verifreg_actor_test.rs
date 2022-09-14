use fvm_shared::address::Address;
use lazy_static::lazy_static;

mod harness;

lazy_static! {
    static ref VERIFIER: Address = Address::new_id(201);
    static ref VERIFIER2: Address = Address::new_id(202);
    static ref CLIENT: Address = Address::new_id(301);
    static ref CLIENT2: Address = Address::new_id(302);
    static ref CLIENT3: Address = Address::new_id(303);
    static ref CLIENT4: Address = Address::new_id(304);
    static ref PROVIDER: Address = Address::new_id(305);
}

mod util {
    use fvm_shared::sector::StoragePower;

    use fil_actors_runtime::test_utils::MockRuntime;

    pub fn verifier_allowance(rt: &MockRuntime) -> StoragePower {
        rt.policy.minimum_verified_allocation_size.clone() + 42
    }

    pub fn client_allowance(rt: &MockRuntime) -> StoragePower {
        verifier_allowance(rt) - 1
    }
}

mod construction {
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actor_verifreg::{Actor as VerifregActor, Method};
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
    use harness::*;

    use crate::*;

    #[test]
    fn construct_with_root_id() {
        let mut rt = new_runtime();
        let h = Harness { root: *ROOT_ADDR };
        h.construct_and_verify(&mut rt, &h.root);
        h.check_state(&rt);
    }

    #[test]
    fn construct_resolves_non_id() {
        let mut rt = new_runtime();
        let h = Harness { root: *ROOT_ADDR };
        let root_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.insert(root_pubkey, h.root);
        h.construct_and_verify(&mut rt, &root_pubkey);
        h.check_state(&rt);
    }

    #[test]
    fn construct_fails_if_root_unresolved() {
        let mut rt = new_runtime();
        let root_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();

        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<VerifregActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(root_pubkey).unwrap(),
            ),
        );
    }
}

mod verifiers {
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{MethodNum, METHOD_SEND};

    use fil_actor_verifreg::{Actor as VerifregActor, AddVerifierParams, DataCap, Method};
    use fil_actors_runtime::test_utils::*;
    use harness::*;
    use util::*;

    use crate::*;

    #[test]
    fn add_verifier_requires_root_caller() {
        let (h, mut rt) = new_harness();
        rt.expect_validate_caller_addr(vec![h.root]);
        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, Address::new_id(501));
        let params =
            AddVerifierParams { address: Address::new_id(201), allowance: verifier_allowance(&rt) };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifier as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_enforces_min_size() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_allocation_size.clone() - 1;
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_verifier(&mut rt, &VERIFIER, &allowance),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_rejects_root() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_verifier(&mut rt, &ROOT_ADDR, &allowance),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_rejects_client() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_verifier_with_existing_cap(&mut rt, &VERIFIER, &allowance, &DataCap::from(1)),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_rejects_unresolved_address() {
        let (h, mut rt) = new_harness();
        let verifier_key_address = Address::new_secp256k1(&[3u8; 65]).unwrap();
        let allowance = verifier_allowance(&rt);
        // Expect runtime to attempt to create the actor, but don't add it to the mock's
        // address resolution table.
        rt.expect_send(
            verifier_key_address,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::default(),
            RawBytes::default(),
            ExitCode::OK,
        );
        expect_abort(
            ExitCode::USR_ILLEGAL_STATE,
            h.add_verifier(&mut rt, &verifier_key_address, &allowance),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_id_address() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_resolves_address() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        let pubkey_addr = Address::new_secp256k1(&[0u8; 65]).unwrap();
        rt.id_addresses.insert(pubkey_addr, *VERIFIER);
        h.add_verifier(&mut rt, &pubkey_addr, &allowance).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn remove_requires_root() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance).unwrap();

        let caller = Address::new_id(501);
        rt.expect_validate_caller_addr(vec![h.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        assert_ne!(h.root, caller);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::RemoveVerifier as MethodNum,
                &RawBytes::serialize(*VERIFIER).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn remove_requires_verifier_exists() {
        let (h, mut rt) = new_harness();
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.remove_verifier(&mut rt, &VERIFIER));
        h.check_state(&rt);
    }

    #[test]
    fn remove_verifier() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance).unwrap();
        h.remove_verifier(&mut rt, &VERIFIER).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn remove_verifier_id_address() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        let verifier_pubkey = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.insert(verifier_pubkey, *VERIFIER);
        // Add using pubkey address.
        h.add_verifier(&mut rt, &VERIFIER, &allowance).unwrap();
        // Remove using ID address.
        h.remove_verifier(&mut rt, &VERIFIER).unwrap();
        h.check_state(&rt);
    }
}

mod clients {
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{MethodNum, METHOD_SEND};
    use num_traits::Zero;

    use fil_actor_verifreg::{Actor as VerifregActor, AddVerifierClientParams, DataCap, Method};
    use fil_actors_runtime::test_utils::*;
    use harness::*;
    use util::*;

    use crate::*;

    #[test]
    fn many_verifiers_and_clients() {
        let (h, mut rt) = new_harness();
        // Each verifier has enough allowance for two clients.
        let allowance_client = client_allowance(&rt);
        let allowance_verifier = allowance_client.clone() + allowance_client.clone();
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();
        h.add_verifier(&mut rt, &VERIFIER2, &allowance_verifier).unwrap();

        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance_client).unwrap();
        h.add_client(&mut rt, &VERIFIER, &CLIENT2, &allowance_client).unwrap();

        h.add_client(&mut rt, &VERIFIER2, &CLIENT3, &allowance_client).unwrap();
        h.add_client(&mut rt, &VERIFIER2, &CLIENT4, &allowance_client).unwrap();

        // No more allowance left
        h.assert_verifier_allowance(&rt, &VERIFIER, &DataCap::from(0));
        h.assert_verifier_allowance(&rt, &VERIFIER2, &DataCap::from(0));
        h.check_state(&rt);
    }

    #[test]
    fn verifier_allowance_exhausted() {
        let (h, mut rt) = new_harness();
        let allowance = client_allowance(&rt);
        // Verifier only has allowance for one client.
        h.add_verifier(&mut rt, &VERIFIER, &allowance).unwrap();

        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &CLIENT2, &allowance),
        );

        h.assert_verifier_allowance(&rt, &VERIFIER, &DataCap::zero());
        h.check_state(&rt);
    }

    #[test]
    fn resolves_client_address() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);

        let client_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.insert(client_pubkey, *CLIENT);

        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();
        h.add_client(&mut rt, &VERIFIER, &client_pubkey, &allowance_client).unwrap();

        // Adding another client with the same address increments
        // the data cap which has already been granted.
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();
        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance_client).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn minimum_allowance_ok() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = rt.policy.minimum_verified_allocation_size.clone();
        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn rejects_unresolved_address() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();

        let client = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        // Expect runtime to attempt to create the actor, but don't add it to the mock's
        // address resolution table.
        rt.expect_send(
            client,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::default(),
            RawBytes::default(),
            ExitCode::OK,
        );

        expect_abort(
            ExitCode::USR_ILLEGAL_STATE,
            h.add_client(&mut rt, &VERIFIER, &client, &allowance_client),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_allowance_below_minimum() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = rt.policy.minimum_verified_allocation_size.clone() - 1;
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_non_verifier_caller() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();

        let caller = Address::new_id(209);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        rt.expect_validate_caller_any();
        let params = AddVerifierClientParams { address: *CLIENT, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_NOT_FOUND,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_allowance_greater_than_verifier_cap() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = allowance_verifier.clone() + 1;
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &h.root, &allowance),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_root_as_client() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &h.root, &allowance_client),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_verifier_as_client() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &VERIFIER, &allowance_client),
        );
        rt.reset();

        h.add_verifier(&mut rt, &VERIFIER2, &allowance_verifier).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &VERIFIER2, &allowance_client),
        );
        h.check_state(&rt);
    }
}

mod claims {
    use fvm_shared::error::ExitCode;

    use fil_actor_verifreg::State;
    use fil_actors_runtime::runtime::Runtime;
    use fil_actors_runtime::BatchReturnGen;
    use harness::*;

    use crate::*;

    #[test]
    fn expire_allocs() {
        let (h, mut rt) = new_harness();

        let mut alloc1 = make_alloc("1", &CLIENT, &PROVIDER, 128);
        alloc1.expiration = 100;
        let mut alloc2 = make_alloc("2", &CLIENT, &PROVIDER, 256);
        alloc2.expiration = 200;

        h.create_alloc(&mut rt, &alloc1).unwrap();
        h.create_alloc(&mut rt, &alloc2).unwrap();
        let state_with_allocs: State = rt.get_state();

        // Can't remove allocations that aren't expired
        let ret = h.remove_expired_allocations(&mut rt, &CLIENT, vec![1, 2], 0).unwrap();
        assert_eq!(
            BatchReturnGen::new(2)
                .add_fail(ExitCode::USR_FORBIDDEN)
                .add_fail(ExitCode::USR_FORBIDDEN)
                .gen(),
            ret
        );

        // Remove the first alloc, which expired.
        rt.set_epoch(100);
        let ret =
            h.remove_expired_allocations(&mut rt, &CLIENT, vec![1, 2], alloc1.size.0).unwrap();
        assert_eq!(
            BatchReturnGen::new(2).add_success().add_fail(ExitCode::USR_FORBIDDEN).gen(),
            ret
        );

        // Remove the second alloc (the first is no longer found).
        rt.set_epoch(200);
        let ret =
            h.remove_expired_allocations(&mut rt, &CLIENT, vec![1, 2], alloc2.size.0).unwrap();
        assert_eq!(
            BatchReturnGen::new(2).add_fail(ExitCode::USR_NOT_FOUND).add_success().gen(),
            ret
        );

        // Reset state and show we can remove two at once.
        rt.replace_state(&state_with_allocs);
        let total_size = alloc1.size.0 + alloc2.size.0;
        let ret = h.remove_expired_allocations(&mut rt, &CLIENT, vec![1, 2], total_size).unwrap();
        assert_eq!(BatchReturnGen::new(2).add_success().add_success().gen(), ret);
    }

    #[test]
    fn claim_allocs() {
        let (h, mut rt) = new_harness();
        let provider = *PROVIDER;

        let size = 128;
        let alloc1 = make_alloc("1", &CLIENT, &provider, size);
        let alloc2 = make_alloc("2", &CLIENT, &provider, size);
        let alloc3 = make_alloc("3", &CLIENT, &provider, size);

        h.create_alloc(&mut rt, &alloc1).unwrap();
        h.create_alloc(&mut rt, &alloc2).unwrap();
        h.create_alloc(&mut rt, &alloc3).unwrap();

        let ret = h
            .claim_allocations(
                &mut rt,
                provider,
                vec![
                    make_claim_req(1, alloc1, 1000, 1500),
                    make_claim_req(2, alloc2, 1000, 1500),
                    make_claim_req(3, alloc3, 1000, 1500),
                ],
                size * 3,
            )
            .unwrap();

        assert_eq!(ret.codes(), vec![ExitCode::OK, ExitCode::OK, ExitCode::OK]);

        // check that state is as expected
        let st: State = rt.get_state();
        let store = rt.store();
        let mut allocs = st.load_allocs(&store).unwrap();
        // allocs deleted
        let client_id = CLIENT.id().unwrap();
        assert!(allocs.get(client_id, 1).unwrap().is_none());
        assert!(allocs.get(client_id, 2).unwrap().is_none());
        assert!(allocs.get(client_id, 3).unwrap().is_none());

        // claims inserted
        let mut claims = st.load_claims(&store).unwrap();
        let provider_id = provider.id().unwrap();
        let claim1 = claims.get(provider_id, 1).unwrap().unwrap().clone();
        let claim2 = claims.get(provider_id, 2).unwrap().unwrap().clone();
        let claim3 = claims.get(provider_id, 3).unwrap().unwrap().clone();
        assert_eq!(claim1.client, client_id);
        assert_eq!(claim2.client, client_id);
        assert_eq!(claim3.client, client_id);

        // get claims 
        //successfully
        let succ_gc = h.get_claims(& mut rt, provider_id, vec![1, 2, 3]).unwrap();
        assert_eq!(3, succ_gc.batch_info.success_count);
        assert_eq!(claim2, succ_gc.claims[1]);
        
        // bad provider
        let fail_gc = h.get_claims(& mut rt, provider_id+42, vec![1, 2, 3]).unwrap();
        assert_eq!(0, fail_gc.batch_info.success_count);

        // mixed bag
        let mix_gc = h.get_claims(& mut rt, provider_id, vec![1, 4, 5]).unwrap();
        assert_eq!(1, mix_gc.batch_info.success_count);
        assert_eq!(claim1, succ_gc.claims[0]);

    }
}

mod datacap {
    use fil_fungible_token::receiver::types::{UniversalReceiverParams, FRC46_TOKEN_TYPE};
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::address::Address;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{ActorID, MethodNum};

    use fil_actor_verifreg::{Actor as VerifregActor, Method, RestoreBytesParams, State};
    use fil_actors_runtime::cbor::serialize;
    use fil_actors_runtime::runtime::policy_constants::{
        MAXIMUM_VERIFIED_ALLOCATION_EXPIRATION, MAXIMUM_VERIFIED_ALLOCATION_TERM,
        MINIMUM_VERIFIED_ALLOCATION_SIZE, MINIMUM_VERIFIED_ALLOCATION_TERM,
    };
    use fil_actors_runtime::runtime::Runtime;
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::{
        DATACAP_TOKEN_ACTOR_ADDR, EPOCHS_IN_YEAR, STORAGE_MARKET_ACTOR_ADDR,
        STORAGE_POWER_ACTOR_ADDR,
    };
    use harness::*;
    use util::*;

    use crate::*;

    const CLIENT1: ActorID = 101;
    const CLIENT2: ActorID = 102;
    const PROVIDER1: ActorID = 301;
    const PROVIDER2: ActorID = 302;
    const ALLOC_SIZE: u64 = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;

    #[test]
    fn receive_tokens_make_allocs() {
        let (h, mut rt) = new_harness();
        add_miner(&mut rt, PROVIDER1);
        add_miner(&mut rt, PROVIDER2);

        {
            let reqs = vec![
                make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE),
                make_alloc_req(&rt, PROVIDER2, ALLOC_SIZE * 2),
            ];
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs.clone());
            h.receive_tokens(&mut rt, payload).unwrap();

            // Verify allocations in state.
            let st: State = rt.get_state();
            let store = &rt.store();
            let mut allocs = st.load_allocs(store).unwrap();

            assert_eq!(
                &alloc_from_req(CLIENT1, &reqs[0]),
                allocs.get(CLIENT1, 1).unwrap().unwrap()
            );
            assert_eq!(
                &alloc_from_req(CLIENT1, &reqs[1]),
                allocs.get(CLIENT1, 2).unwrap().unwrap()
            );
            assert_eq!(3, st.next_allocation_id);
        }
        {
            // Make another allocation from a different client
            let reqs = vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)];
            let payload = make_receiver_hook_token_payload(CLIENT2, reqs.clone());
            h.receive_tokens(&mut rt, payload).unwrap();

            // Verify allocations in state.
            let st: State = rt.get_state();
            let store = &rt.store();
            let mut allocs = st.load_allocs(store).unwrap();
            assert_eq!(
                &alloc_from_req(CLIENT2, &reqs[0]),
                allocs.get(CLIENT2, 3).unwrap().unwrap()
            );
            assert_eq!(4, st.next_allocation_id);
        }
        h.check_state(&rt);
    }

    #[test]
    fn receive_requires_datacap_caller() {
        let (h, mut rt) = new_harness();
        add_miner(&mut rt, PROVIDER1);

        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(
                &make_receiver_hook_token_payload(
                    CLIENT1,
                    vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)],
                ),
                "payload",
            )
            .unwrap(),
        };

        rt.set_caller(*MARKET_ACTOR_CODE_ID, *STORAGE_MARKET_ACTOR_ADDR); // Wrong caller
        rt.expect_validate_caller_addr(vec![*DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "caller address",
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                &RawBytes::serialize(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_requires_to_self() {
        let (h, mut rt) = new_harness();
        add_miner(&mut rt, PROVIDER1);

        let mut payload = make_receiver_hook_token_payload(
            CLIENT1,
            vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)],
        );
        // Set invalid receiver hook "to" address (should be the verified registry itself).
        payload.to = PROVIDER1;
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };

        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, *DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![*DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "token receiver expected to",
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                &RawBytes::serialize(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_requires_miner_actor() {
        let (h, mut rt) = new_harness();
        let provider1 = Address::new_id(PROVIDER1);
        rt.set_address_actor_type(provider1, *ACCOUNT_ACTOR_CODE_ID);

        let reqs = vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)];
        let payload = make_receiver_hook_token_payload(CLIENT1, reqs);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            format!("allocation provider {} must be a miner actor", provider1).as_str(),
            h.receive_tokens(&mut rt, payload),
        );
        h.check_state(&rt);
    }

    #[test]
    fn receive_invalid_reqs() {
        let (h, mut rt) = new_harness();
        add_miner(&mut rt, PROVIDER1);

        // Alloc too small
        {
            let reqs = vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE - 1)];
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation size 1048575 below minimum 1048576",
                h.receive_tokens(&mut rt, payload),
            );
        }
        // Min term too short
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)];
            reqs[0].term_min = MINIMUM_VERIFIED_ALLOCATION_TERM - 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation term min 518399 below limit 518400",
                h.receive_tokens(&mut rt, payload),
            );
        }
        // Max term too long
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)];
            reqs[0].term_max = MAXIMUM_VERIFIED_ALLOCATION_TERM + 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation term max 5259486 above limit 5259485",
                h.receive_tokens(&mut rt, payload),
            );
        }
        // Term minimum greater than maximum
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)];
            reqs[0].term_max = 2 * EPOCHS_IN_YEAR;
            reqs[0].term_min = reqs[0].term_max + 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation term min 2103795 exceeds term max 2103794",
                h.receive_tokens(&mut rt, payload),
            );
        }
        // Allocation expires too late
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE)];
            reqs[0].expiration = rt.epoch + MAXIMUM_VERIFIED_ALLOCATION_EXPIRATION + 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation expiration 86401 exceeds maximum 86400",
                h.receive_tokens(&mut rt, payload),
            );
        }
        // Tokens received doesn't match sum of allocation sizes
        {
            let reqs = vec![
                make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE),
                make_alloc_req(&rt, PROVIDER2, ALLOC_SIZE),
            ];
            let mut payload = make_receiver_hook_token_payload(CLIENT1, reqs);
            payload.amount = TokenAmount::from_whole((ALLOC_SIZE * 2 + 1) as i64);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "total allocation size 2097152 must match data cap amount received 2097153",
                h.receive_tokens(&mut rt, payload),
            );
        }
        // One bad request fails the lot
        {
            let reqs = vec![
                make_alloc_req(&rt, PROVIDER1, ALLOC_SIZE),
                make_alloc_req(&rt, PROVIDER2, ALLOC_SIZE - 1),
            ];
            let mut payload = make_receiver_hook_token_payload(CLIENT1, reqs);
            payload.amount = TokenAmount::from_whole((ALLOC_SIZE * 2 - 1) as i64);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation size 1048575 below minimum 1048576",
                h.receive_tokens(&mut rt, payload),
            );
        }
        h.check_state(&rt);
    }

    #[test]
    fn restore() {
        let (h, mut rt) = new_harness();
        let deal_size = &rt.policy.minimum_verified_allocation_size.clone();
        h.restore_bytes(&mut rt, &CLIENT, deal_size).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn restore_resolves_client_address() {
        let (h, mut rt) = new_harness();
        let client_pubkey = Address::new_secp256k1(&[3u8; 65]).unwrap();
        rt.id_addresses.insert(client_pubkey, *CLIENT);

        // Restore to pubkey address.
        let deal_size = rt.policy.minimum_verified_allocation_size.clone();
        h.restore_bytes(&mut rt, &client_pubkey, &deal_size).unwrap();
        h.check_state(&rt)
    }

    #[test]
    fn restore_requires_market_actor_caller() {
        let (h, mut rt) = new_harness();
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*POWER_ACTOR_CODE_ID, *STORAGE_POWER_ACTOR_ADDR);
        let params = RestoreBytesParams {
            address: *CLIENT,
            deal_size: rt.policy.minimum_verified_allocation_size.clone(),
        };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::RestoreBytes as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state(&rt)
    }

    #[test]
    fn restore_requires_minimum_deal_size() {
        let (h, mut rt) = new_harness();

        let deal_size = rt.policy.minimum_verified_allocation_size.clone() - 1;
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.restore_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state(&rt)
    }

    #[test]
    fn restore_rejects_root() {
        let (h, mut rt) = new_harness();
        let deal_size = rt.policy.minimum_verified_allocation_size.clone();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.restore_bytes(&mut rt, &ROOT_ADDR, &deal_size),
        );
        h.check_state(&rt)
    }

    #[test]
    fn restore_rejects_verifier() {
        let (h, mut rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance).unwrap();
        let deal_size = rt.policy.minimum_verified_allocation_size.clone();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.restore_bytes(&mut rt, &VERIFIER, &deal_size),
        );
        h.check_state(&rt)
    }
}

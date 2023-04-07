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
    static ref PROVIDER2: Address = Address::new_id(306);
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
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actor_verifreg::{Actor as VerifregActor, Method};
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use harness::*;

    use crate::*;

    #[test]
    fn construct_with_root_id() {
        let rt = new_runtime();
        let h = Harness { root: ROOT_ADDR };
        h.construct_and_verify(&rt, &h.root);
        h.check_state(&rt);
    }

    #[test]
    fn construct_resolves_non_id() {
        let rt = new_runtime();
        let h = Harness { root: ROOT_ADDR };
        let root_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.borrow_mut().insert(root_pubkey, h.root);
        h.construct_and_verify(&rt, &root_pubkey);
        h.check_state(&rt);
    }

    #[test]
    fn construct_fails_if_root_unresolved() {
        let rt = new_runtime();
        let root_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();

        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<VerifregActor>(
                Method::Constructor as MethodNum,
                IpldBlock::serialize_cbor(&root_pubkey).unwrap(),
            ),
        );
    }
}

mod verifiers {
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{MethodNum, METHOD_SEND};
    use std::ops::Deref;

    use fil_actor_verifreg::{Actor as VerifregActor, AddVerifierParams, DataCap, Method};
    use fil_actors_runtime::test_utils::*;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use harness::*;
    use util::*;

    use crate::*;

    #[test]
    fn add_verifier_requires_root_caller() {
        let (h, rt) = new_harness();
        rt.expect_validate_caller_addr(vec![h.root]);
        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, Address::new_id(501));
        let params =
            AddVerifierParams { address: Address::new_id(201), allowance: verifier_allowance(&rt) };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifier as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_enforces_min_size() {
        let (h, rt) = new_harness();
        let allowance: DataCap = rt.policy.minimum_verified_allocation_size.clone() - 1;

        let params = AddVerifierParams { address: *VERIFIER, allowance };
        let result = rt.call::<VerifregActor>(
            Method::AddVerifier as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        );
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_rejects_root() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.add_verifier(&rt, &ROOT_ADDR, &allowance));
        rt.reset();
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_rejects_client() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_verifier_with_existing_cap(&rt, &VERIFIER, &allowance, &DataCap::from(1)),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_rejects_unresolved_address() {
        let (h, rt) = new_harness();
        let verifier_key_address = Address::new_secp256k1(&[3u8; 65]).unwrap();
        let allowance = verifier_allowance(&rt);
        // Expect runtime to attempt to create the actor, but don't add it to the mock's
        // address resolution table.
        rt.expect_send_simple(
            verifier_key_address,
            METHOD_SEND,
            None,
            TokenAmount::default(),
            None,
            ExitCode::OK,
        );

        let params = AddVerifierParams { address: verifier_key_address, allowance };
        let result = rt.call::<VerifregActor>(
            Method::AddVerifier as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        );

        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_id_address() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn add_verifier_resolves_address() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        let pubkey_addr = Address::new_secp256k1(&[0u8; 65]).unwrap();
        rt.id_addresses.borrow_mut().insert(pubkey_addr, *VERIFIER);
        h.add_verifier(&rt, &pubkey_addr, &allowance).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn remove_requires_root() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance).unwrap();

        let caller = Address::new_id(501);
        rt.expect_validate_caller_addr(vec![h.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        assert_ne!(h.root, caller);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::RemoveVerifier as MethodNum,
                IpldBlock::serialize_cbor(VERIFIER.deref()).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn remove_requires_verifier_exists() {
        let (h, rt) = new_harness();
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.remove_verifier(&rt, &VERIFIER));
        h.check_state(&rt);
    }

    #[test]
    fn remove_verifier() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance).unwrap();
        h.remove_verifier(&rt, &VERIFIER).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn remove_verifier_id_address() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        let verifier_pubkey = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.borrow_mut().insert(verifier_pubkey, *VERIFIER);
        // Add using pubkey address.
        h.add_verifier(&rt, &VERIFIER, &allowance).unwrap();
        // Remove using ID address.
        h.remove_verifier(&rt, &VERIFIER).unwrap();
        h.check_state(&rt);
    }
}

mod clients {
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{MethodNum, METHOD_SEND};
    use num_traits::Zero;

    use fil_actor_verifreg::{
        ext, Actor as VerifregActor, AddVerifiedClientParams, DataCap, Method,
    };
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::{DATACAP_TOKEN_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR};

    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use harness::*;
    use num_traits::ToPrimitive;
    use util::*;

    use crate::*;

    #[test]
    fn many_verifiers_and_clients() {
        let (h, rt) = new_harness();
        // Each verifier has enough allowance for two clients.
        let allowance_client = client_allowance(&rt);
        let allowance_verifier = allowance_client.clone() + allowance_client.clone();
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();
        h.add_verifier(&rt, &VERIFIER2, &allowance_verifier).unwrap();

        h.add_client(&rt, &VERIFIER, &CLIENT, &allowance_client).unwrap();
        h.add_client(&rt, &VERIFIER, &CLIENT2, &allowance_client).unwrap();

        h.add_client(&rt, &VERIFIER2, &CLIENT3, &allowance_client).unwrap();
        h.add_client(&rt, &VERIFIER2, &CLIENT4, &allowance_client).unwrap();

        // No more allowance left
        h.assert_verifier_allowance(&rt, &VERIFIER, &DataCap::from(0));
        h.assert_verifier_allowance(&rt, &VERIFIER2, &DataCap::from(0));
        h.check_state(&rt);
    }

    #[test]
    fn verifier_allowance_exhausted() {
        let (h, rt) = new_harness();
        let allowance = client_allowance(&rt);
        // Verifier only has allowance for one client.
        h.add_verifier(&rt, &VERIFIER, &allowance).unwrap();

        h.add_client(&rt, &VERIFIER, &CLIENT, &allowance).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&rt, &VERIFIER, &CLIENT2, &allowance),
        );
        rt.reset();
        h.assert_verifier_allowance(&rt, &VERIFIER, &DataCap::zero());
        h.check_state(&rt);
    }

    #[test]
    fn resolves_client_address() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);

        let client_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.borrow_mut().insert(client_pubkey, *CLIENT);

        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();
        h.add_client(&rt, &VERIFIER, &client_pubkey, &allowance_client).unwrap();

        // Adding another client with the same address increments
        // the data cap which has already been granted.
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();
        h.add_client(&rt, &VERIFIER, &CLIENT, &allowance_client).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn minimum_allowance_ok() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = rt.policy.minimum_verified_allocation_size.clone();
        h.add_client(&rt, &VERIFIER, &CLIENT, &allowance).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn rejects_unresolved_address() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();

        let client = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        // Expect runtime to attempt to create the actor, but don't add it to the mock's
        // address resolution table.
        rt.expect_send_simple(
            client,
            METHOD_SEND,
            None,
            TokenAmount::default(),
            None,
            ExitCode::OK,
        );

        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&rt, &VERIFIER, &client, &allowance_client),
        );
        rt.reset();
        h.check_state(&rt);
    }

    #[test]
    fn rejects_allowance_below_minimum() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = rt.policy.minimum_verified_allocation_size.clone() - 1;
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&rt, &VERIFIER, &CLIENT, &allowance),
        );
        rt.reset();
        h.check_state(&rt);
    }

    #[test]
    fn rejects_non_verifier_caller() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();

        let caller = Address::new_id(209);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_NOT_FOUND,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verified_client_restricted_correctly() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();

        let params =
            AddVerifiedClientParams { address: *CLIENT, allowance: allowance_client.clone() };

        // set caller to not-builtin
        rt.set_caller(*EVM_ACTOR_CODE_ID, *VERIFIER);

        // cannot call the unexported method num
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "must be built-in",
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );

        rt.verify();

        // can call the exported method num

        let mint_params = ext::datacap::MintParams {
            to: *CLIENT,
            amount: TokenAmount::from_whole(allowance_client.to_i64().unwrap()),
            operators: vec![STORAGE_MARKET_ACTOR_ADDR],
        };
        rt.expect_send_simple(
            DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Mint as MethodNum,
            IpldBlock::serialize_cbor(&mint_params).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );

        rt.expect_validate_caller_any();
        rt.call::<VerifregActor>(
            Method::AddVerifiedClientExported as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap();

        rt.verify();

        h.check_state(&rt);
    }

    #[test]
    fn rejects_allowance_greater_than_verifier_cap() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = allowance_verifier + 1;
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&rt, &VERIFIER, &h.root, &allowance),
        );
        rt.reset();
        h.check_state(&rt);
    }

    #[test]
    fn rejects_root_as_client() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&rt, &VERIFIER, &h.root, &allowance_client),
        );
        rt.reset();
        h.check_state(&rt);
    }

    #[test]
    fn rejects_verifier_as_client() {
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier(&rt, &VERIFIER, &allowance_verifier).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&rt, &VERIFIER, &VERIFIER, &allowance_client),
        );
        rt.reset();

        h.add_verifier(&rt, &VERIFIER2, &allowance_verifier).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&rt, &VERIFIER, &VERIFIER2, &allowance_client),
        );
        rt.reset();
        h.check_state(&rt);
    }
}

mod allocs_claims {
    use cid::Cid;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::bigint::BigInt;
    use fvm_shared::error::ExitCode;
    use fvm_shared::piece::PaddedPieceSize;
    use fvm_shared::{ActorID, MethodNum};
    use num_traits::Zero;
    use std::str::FromStr;

    use fil_actor_verifreg::{
        Actor, AllocationID, ClaimTerm, DataCap, ExtendClaimTermsParams, GetClaimsParams,
        GetClaimsReturn, Method, State,
    };
    use fil_actor_verifreg::{Claim, ExtendClaimTermsReturn};
    use fil_actors_runtime::runtime::policy_constants::{
        MAXIMUM_VERIFIED_ALLOCATION_TERM, MINIMUM_VERIFIED_ALLOCATION_SIZE,
        MINIMUM_VERIFIED_ALLOCATION_TERM,
    };
    use fil_actors_runtime::test_utils::{
        expect_abort_contains_message, ACCOUNT_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID,
    };
    use fil_actors_runtime::FailCode;
    use harness::*;

    use crate::*;

    const CLIENT1: ActorID = 101;
    const CLIENT2: ActorID = 102;
    const PROVIDER1: ActorID = 301;
    const PROVIDER2: ActorID = 302;
    const ALLOC_SIZE: u64 = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;

    #[test]
    fn expire_allocs() {
        let (h, rt) = new_harness();

        let mut alloc1 = make_alloc("1", CLIENT1, PROVIDER1, ALLOC_SIZE);
        alloc1.expiration = 100;
        let mut alloc2 = make_alloc("2", CLIENT1, PROVIDER1, ALLOC_SIZE * 2);
        alloc2.expiration = 200;
        let total_size = alloc1.size.0 + alloc2.size.0;

        let id1 = h.create_alloc(&rt, &alloc1).unwrap();
        let id2 = h.create_alloc(&rt, &alloc2).unwrap();
        let state_with_allocs: State = rt.get_state();

        // Can't remove allocations that aren't expired
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], 0).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_FORBIDDEN, ExitCode::USR_FORBIDDEN], ret.results.codes());
        assert_eq!(DataCap::zero(), ret.datacap_recovered);

        // Can't remove with wrong client ID
        rt.set_epoch(200);
        let ret = h.remove_expired_allocations(&rt, CLIENT2, vec![id1, id2], 0).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_NOT_FOUND, ExitCode::USR_NOT_FOUND], ret.results.codes());
        assert_eq!(DataCap::zero(), ret.datacap_recovered);

        // Remove the first alloc, which expired.
        rt.set_epoch(100);
        let ret =
            h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], alloc1.size.0).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::USR_FORBIDDEN], ret.results.codes());
        assert_eq!(DataCap::from(alloc1.size.0), ret.datacap_recovered);

        // Remove the second alloc (the first is no longer found).
        rt.set_epoch(200);
        let ret =
            h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], alloc2.size.0).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_NOT_FOUND, ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc2.size.0), ret.datacap_recovered);

        // Reset state and show we can remove two at once.
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], total_size).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(total_size), ret.datacap_recovered);

        // Reset state and show that only what was asked for is removed.
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![id1], alloc1.size.0).unwrap();
        assert_eq!(vec![1], ret.considered);
        assert_eq!(vec![ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc1.size.0), ret.datacap_recovered);

        // Reset state and show that specifying none removes only expired allocations
        rt.set_epoch(0);
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![], 0).unwrap();
        assert_eq!(Vec::<AllocationID>::new(), ret.considered);
        assert_eq!(Vec::<ExitCode>::new(), ret.results.codes());
        assert_eq!(DataCap::zero(), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_some());
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_some());

        rt.set_epoch(100);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![], alloc1.size.0).unwrap();
        assert_eq!(vec![1], ret.considered);
        assert_eq!(vec![ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc1.size.0), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_none()); // removed
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_some());

        rt.set_epoch(200);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![], alloc2.size.0).unwrap();
        assert_eq!(vec![2], ret.considered);
        assert_eq!(vec![ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc2.size.0), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_none()); // removed
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_none()); // removed

        // Reset state and show that specifying none removes *all* expired allocations
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![], total_size).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(total_size), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_none()); // removed
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_none()); // removed
        h.check_state(&rt);
    }

    #[test]
    fn claim_allocs() {
        let (h, rt) = new_harness();

        let size = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;
        let alloc1 = make_alloc("1", CLIENT1, PROVIDER1, size);
        let alloc2 = make_alloc("2", CLIENT2, PROVIDER1, size); // Distinct client
        let alloc3 = make_alloc("3", CLIENT1, PROVIDER2, size); // Distinct provider

        h.create_alloc(&rt, &alloc1).unwrap();
        h.create_alloc(&rt, &alloc2).unwrap();
        h.create_alloc(&rt, &alloc3).unwrap();
        h.check_state(&rt);

        let sector = 1000;
        let expiry = MINIMUM_VERIFIED_ALLOCATION_TERM;

        let prior_state: State = rt.get_state();
        {
            // Claim two for PROVIDER1
            let reqs = vec![
                make_claim_req(1, &alloc1, sector, expiry),
                make_claim_req(2, &alloc2, sector, expiry),
            ];
            let ret = h.claim_allocations(&rt, PROVIDER1, reqs, size * 2, false).unwrap();

            assert_eq!(ret.batch_info.codes(), vec![ExitCode::OK, ExitCode::OK]);
            assert_eq!(ret.claimed_space, BigInt::from(2 * size));
            assert_alloc_claimed(&rt, CLIENT1, PROVIDER1, 1, &alloc1, 0, sector);
            assert_alloc_claimed(&rt, CLIENT2, PROVIDER1, 2, &alloc2, 0, sector);
            h.check_state(&rt);
        }
        {
            // Can't find claim for wrong client
            rt.replace_state(&prior_state);
            let mut reqs = vec![
                make_claim_req(1, &alloc1, sector, expiry),
                make_claim_req(2, &alloc2, sector, expiry),
            ];
            reqs[1].client = CLIENT1;
            let ret = h.claim_allocations(&rt, PROVIDER1, reqs, size, false).unwrap();
            assert_eq!(ret.batch_info.codes(), vec![ExitCode::OK, ExitCode::USR_NOT_FOUND]);
            assert_eq!(ret.claimed_space, BigInt::from(size));
            assert_alloc_claimed(&rt, CLIENT1, PROVIDER1, 1, &alloc1, 0, sector);
            assert_allocation(&rt, CLIENT2, 2, &alloc2);
            h.check_state(&rt);
        }
        {
            // Can't claim for other provider
            rt.replace_state(&prior_state);
            let reqs = vec![
                make_claim_req(2, &alloc2, sector, expiry),
                make_claim_req(3, &alloc3, sector, expiry), // Different provider
            ];
            let ret = h.claim_allocations(&rt, PROVIDER1, reqs, size, false).unwrap();
            assert_eq!(ret.batch_info.codes(), vec![ExitCode::OK, ExitCode::USR_FORBIDDEN]);
            assert_eq!(ret.claimed_space, BigInt::from(size));
            assert_alloc_claimed(&rt, CLIENT1, PROVIDER1, 2, &alloc2, 0, sector);
            assert_allocation(&rt, CLIENT1, 3, &alloc3);
            h.check_state(&rt);
        }
        {
            // Mismatched data / size
            rt.replace_state(&prior_state);
            let mut reqs = vec![
                make_claim_req(1, &alloc1, sector, expiry),
                make_claim_req(2, &alloc2, sector, expiry),
            ];
            reqs[0].data =
                Cid::from_str("bafyreibjo4xmgaevkgud7mbifn3dzp4v4lyaui4yvqp3f2bqwtxcjrdqg4")
                    .unwrap();
            reqs[1].size = PaddedPieceSize(size + 1);
            let ret = h.claim_allocations(&rt, PROVIDER1, reqs, 0, false).unwrap();
            assert_eq!(
                ret.batch_info.codes(),
                vec![ExitCode::USR_FORBIDDEN, ExitCode::USR_FORBIDDEN]
            );
            assert_eq!(ret.claimed_space, BigInt::zero());
            h.check_state(&rt);
        }
        {
            // Expired allocation
            rt.replace_state(&prior_state);
            let reqs = vec![make_claim_req(1, &alloc1, sector, expiry)];
            rt.set_epoch(alloc1.expiration + 1);
            let ret = h.claim_allocations(&rt, PROVIDER1, reqs, 0, false).unwrap();
            assert_eq!(ret.batch_info.codes(), vec![ExitCode::USR_FORBIDDEN]);
            assert_eq!(ret.claimed_space, BigInt::zero());
            h.check_state(&rt);
        }
        {
            // Sector expiration too soon
            rt.replace_state(&prior_state);
            let reqs = vec![make_claim_req(1, &alloc1, sector, alloc1.term_min - 1)];
            let ret = h.claim_allocations(&rt, PROVIDER1, reqs, 0, false).unwrap();
            assert_eq!(ret.batch_info.codes(), vec![ExitCode::USR_FORBIDDEN]);
            // Sector expiration too late
            let reqs = vec![make_claim_req(1, &alloc1, sector, alloc1.term_max + 1)];
            let ret = h.claim_allocations(&rt, PROVIDER1, reqs, 0, false).unwrap();
            assert_eq!(ret.batch_info.codes(), vec![ExitCode::USR_FORBIDDEN]);
            assert_eq!(ret.claimed_space, BigInt::zero());
            h.check_state(&rt);
        }
    }

    #[test]
    fn get_claims() {
        let (h, rt) = new_harness();
        let size = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;
        let sector = 0;
        let start = 0;
        let min_term = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let max_term = min_term + 1000;

        let claim1 = make_claim("1", CLIENT1, PROVIDER1, size, min_term, max_term, start, sector);
        let claim2 = make_claim("2", CLIENT1, PROVIDER1, size, min_term, max_term, start, sector);
        let claim3 = make_claim("3", CLIENT1, PROVIDER2, size, min_term, max_term, start, sector);
        let id1 = h.create_claim(&rt, &claim1).unwrap();
        let id2 = h.create_claim(&rt, &claim2).unwrap();
        let id3 = h.create_claim(&rt, &claim3).unwrap();

        {
            // Get multiple
            let ret = h.get_claims(&rt, PROVIDER1, vec![id1, id2]).unwrap();
            assert_eq!(2, ret.batch_info.success_count);
            assert_eq!(claim1, ret.claims[0]);
            assert_eq!(claim2, ret.claims[1]);
        }
        {
            // Wrong provider
            let ret = h.get_claims(&rt, PROVIDER1, vec![id3]).unwrap();
            assert_eq!(0, ret.batch_info.success_count);
        }
        {
            // Mixed bag
            let ret = h.get_claims(&rt, PROVIDER1, vec![id1, id3, id2]).unwrap();
            assert_eq!(2, ret.batch_info.success_count);
            assert_eq!(claim1, ret.claims[0]);
            assert_eq!(claim2, ret.claims[1]);
            assert_eq!(
                vec![FailCode { idx: 1, code: ExitCode::USR_NOT_FOUND }],
                ret.batch_info.fail_codes
            );
        }
        h.check_state(&rt);
    }

    #[test]
    fn extend_claims_basic() {
        let (h, rt) = new_harness();
        let size = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;
        let sector = 0;
        let start = 0;
        let min_term = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let max_term = min_term + 1000;

        let claim1 = make_claim("1", CLIENT1, PROVIDER1, size, min_term, max_term, start, sector);
        let claim2 = make_claim("2", CLIENT1, PROVIDER1, size, min_term, max_term, start, sector);
        let claim3 = make_claim("3", CLIENT1, PROVIDER2, size, min_term, max_term, start, sector);

        let id1 = h.create_claim(&rt, &claim1).unwrap();
        let id2 = h.create_claim(&rt, &claim2).unwrap();
        let id3 = h.create_claim(&rt, &claim3).unwrap();

        // Extend claim terms and verify return value.
        let params = ExtendClaimTermsParams {
            terms: vec![
                ClaimTerm { provider: PROVIDER1, claim_id: id1, term_max: max_term + 1 },
                ClaimTerm { provider: PROVIDER1, claim_id: id2, term_max: max_term + 2 },
                ClaimTerm { provider: PROVIDER2, claim_id: id3, term_max: max_term + 3 },
            ],
        };
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
        let ret = h.extend_claim_terms(&rt, &params).unwrap();
        assert_eq!(ret.codes(), vec![ExitCode::OK, ExitCode::OK, ExitCode::OK]);

        // Verify state directly.
        assert_claim(&rt, PROVIDER1, id1, &Claim { term_max: max_term + 1, ..claim1 });
        assert_claim(&rt, PROVIDER1, id2, &Claim { term_max: max_term + 2, ..claim2 });
        assert_claim(&rt, PROVIDER2, id3, &Claim { term_max: max_term + 3, ..claim3 });
        h.check_state(&rt);
    }

    #[test]
    fn extend_claims_edge_cases() {
        let (h, rt) = new_harness();
        let size = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;
        let sector = 0;
        let start = 0;
        let min_term = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let max_term = min_term + 1000;

        let claim = make_claim("1", CLIENT1, PROVIDER1, size, min_term, max_term, start, sector);

        // Basic success case with no-op extension
        {
            let claim_id = h.create_claim(&rt, &claim).unwrap();
            let params = ExtendClaimTermsParams {
                terms: vec![ClaimTerm { provider: PROVIDER1, claim_id, term_max: max_term }],
            };
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
            let ret = h.extend_claim_terms(&rt, &params).unwrap();
            assert_eq!(ret.codes(), vec![ExitCode::OK]);
            rt.verify()
        }
        // Mismatched client is forbidden
        {
            let claim_id = h.create_claim(&rt, &claim).unwrap();
            let params = ExtendClaimTermsParams {
                terms: vec![ClaimTerm { provider: PROVIDER1, claim_id, term_max: max_term }],
            };
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT2));
            let ret = h.extend_claim_terms(&rt, &params).unwrap();
            assert_eq!(ret.codes(), vec![ExitCode::USR_FORBIDDEN]);
            rt.verify()
        }
        // Mismatched provider is not found
        {
            let claim_id = h.create_claim(&rt, &claim).unwrap();
            let params = ExtendClaimTermsParams {
                terms: vec![ClaimTerm { provider: PROVIDER2, claim_id, term_max: max_term }],
            };
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
            let ret = h.extend_claim_terms(&rt, &params).unwrap();
            assert_eq!(ret.codes(), vec![ExitCode::USR_NOT_FOUND]);
            rt.verify()
        }
        // Term in excess of limit is denied
        {
            let claim_id = h.create_claim(&rt, &claim).unwrap();
            let params = ExtendClaimTermsParams {
                terms: vec![ClaimTerm {
                    provider: PROVIDER1,
                    claim_id,
                    term_max: MAXIMUM_VERIFIED_ALLOCATION_TERM + 1,
                }],
            };
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
            let ret = h.extend_claim_terms(&rt, &params).unwrap();
            assert_eq!(ret.codes(), vec![ExitCode::USR_ILLEGAL_ARGUMENT]);
            rt.verify()
        }
        // Reducing term is denied.
        {
            let claim_id = h.create_claim(&rt, &claim).unwrap();
            let params = ExtendClaimTermsParams {
                terms: vec![ClaimTerm { provider: PROVIDER1, claim_id, term_max: max_term - 1 }],
            };
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
            let ret = h.extend_claim_terms(&rt, &params).unwrap();
            assert_eq!(ret.codes(), vec![ExitCode::USR_ILLEGAL_ARGUMENT]);
            rt.verify()
        }
        // Extending an already-expired claim is ok
        {
            let claim_id = h.create_claim(&rt, &claim).unwrap();
            let params = ExtendClaimTermsParams {
                terms: vec![ClaimTerm {
                    provider: PROVIDER1,
                    claim_id,
                    term_max: MAXIMUM_VERIFIED_ALLOCATION_TERM,
                }],
            };
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
            rt.set_epoch(max_term + 1);
            let ret = h.extend_claim_terms(&rt, &params).unwrap();
            assert_eq!(ret.codes(), vec![ExitCode::OK]);
            rt.verify()
        }
        h.check_state(&rt);
    }

    #[test]
    fn expire_claims() {
        let (h, rt) = new_harness();
        let term_start = 0;
        let term_min = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let sector = 0;

        // expires at term_start + term_min + 100
        let claim1 = make_claim(
            "1",
            CLIENT1,
            PROVIDER1,
            ALLOC_SIZE,
            term_min,
            term_min + 100,
            term_start,
            sector,
        );
        // expires at term_start + 200 + term_min (i.e. 100 epochs later)
        let claim2 = make_claim(
            "2",
            CLIENT1,
            PROVIDER1,
            ALLOC_SIZE * 2,
            term_min,
            term_min,
            term_start + 200,
            sector,
        );

        let id1 = h.create_claim(&rt, &claim1).unwrap();
        let id2 = h.create_claim(&rt, &claim2).unwrap();
        let state_with_allocs: State = rt.get_state();

        // Removal of expired claims shares most of its implementation with removing expired allocations.
        // The full test suite is not duplicated here,   simple ones to ensure that the expiration
        // is correctly computed.

        // None expired yet
        rt.set_epoch(term_start + term_min + 99);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![id1, id2]).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_FORBIDDEN, ExitCode::USR_FORBIDDEN], ret.results.codes());

        // One expired
        rt.set_epoch(term_start + term_min + 100);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![id1, id2]).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::USR_FORBIDDEN], ret.results.codes());

        // Both now expired
        rt.set_epoch(term_start + term_min + 200);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![id1, id2]).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_NOT_FOUND, ExitCode::OK], ret.results.codes());

        // Reset state, and show that specifying none removes only expired allocations
        rt.set_epoch(term_start + term_min);
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![]).unwrap();
        assert_eq!(Vec::<AllocationID>::new(), ret.considered);
        assert_eq!(Vec::<ExitCode>::new(), ret.results.codes());
        assert!(h.load_claim(&rt, PROVIDER1, id1).is_some());
        assert!(h.load_claim(&rt, PROVIDER1, id2).is_some());

        rt.set_epoch(term_start + term_min + 200);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![]).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::OK], ret.results.codes());
        assert!(h.load_claim(&rt, PROVIDER1, id1).is_none()); // removed
        assert!(h.load_claim(&rt, PROVIDER1, id2).is_none()); // removed
        h.check_state(&rt);
    }

    #[test]
    fn claims_restricted_correctly() {
        let (h, rt) = new_harness();
        let size = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;
        let sector = 0;
        let start = 0;
        let min_term = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let max_term = min_term + 1000;

        let claim1 = make_claim("1", CLIENT1, PROVIDER1, size, min_term, max_term, start, sector);

        let id1 = h.create_claim(&rt, &claim1).unwrap();

        // First, let's extend some claims

        let params = ExtendClaimTermsParams {
            terms: vec![ClaimTerm { provider: PROVIDER1, claim_id: id1, term_max: max_term + 1 }],
        };

        // set caller to not-builtin
        rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(CLIENT1));

        // cannot call the unexported extend method num
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "must be built-in",
            h.extend_claim_terms(&rt, &params),
        );

        // can call the exported method num

        rt.expect_validate_caller_any();
        let ret: ExtendClaimTermsReturn = rt
            .call::<Actor>(
                Method::ExtendClaimTermsExported as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )
            .unwrap()
            .unwrap()
            .deserialize()
            .expect("failed to deserialize extend claim terms return");

        rt.verify();

        assert_eq!(ret.codes(), vec![ExitCode::OK]);

        // Now let's Get those Claims, and check them

        let params = GetClaimsParams { claim_ids: vec![id1], provider: PROVIDER1 };

        // cannot call the unexported extend method num
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "must be built-in",
            rt.call::<Actor>(
                Method::GetClaims as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );

        rt.verify();

        // can call the exported method num
        rt.expect_validate_caller_any();
        let ret: GetClaimsReturn = rt
            .call::<Actor>(
                Method::GetClaimsExported as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )
            .unwrap()
            .unwrap()
            .deserialize()
            .expect("failed to deserialize get claims return");

        rt.verify();

        assert_eq!(ret.batch_info.codes(), vec![ExitCode::OK]);
        assert_eq!(ret.claims, vec![Claim { term_max: max_term + 1, ..claim1 }]);

        h.check_state(&rt);
    }
}

mod datacap {
    use frc46_token::receiver::FRC46_TOKEN_TYPE;
    use fvm_actor_utils::receiver::UniversalReceiverParams;
    use fvm_shared::address::Address;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{ActorID, MethodNum};

    use fil_actor_verifreg::{Actor as VerifregActor, Claim, Method, State};
    use fil_actors_runtime::cbor::serialize;
    use fil_actors_runtime::runtime::policy_constants::{
        MAXIMUM_VERIFIED_ALLOCATION_EXPIRATION, MAXIMUM_VERIFIED_ALLOCATION_TERM,
        MINIMUM_VERIFIED_ALLOCATION_SIZE, MINIMUM_VERIFIED_ALLOCATION_TERM,
    };
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::{
        BatchReturn, DATACAP_TOKEN_ACTOR_ADDR, EPOCHS_IN_YEAR, STORAGE_MARKET_ACTOR_ADDR,
    };
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use harness::*;

    use crate::*;

    const CLIENT1: ActorID = 101;
    const CLIENT2: ActorID = 102;
    const PROVIDER1: ActorID = 301;
    const PROVIDER2: ActorID = 302;
    const SIZE: u64 = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;
    const BATCH_EMPTY: BatchReturn = BatchReturn::empty();

    #[test]
    fn receive_tokens_make_allocs() {
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);
        add_miner(&rt, PROVIDER2);

        {
            let reqs = vec![
                make_alloc_req(&rt, PROVIDER1, SIZE),
                make_alloc_req(&rt, PROVIDER2, SIZE * 2),
            ];
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs.clone(), vec![], SIZE * 3);
            h.receive_tokens(&rt, payload, BatchReturn::ok(2), BATCH_EMPTY, vec![1, 2], 0).unwrap();

            // Verify allocations in state.
            assert_allocation(&rt, CLIENT1, 1, &alloc_from_req(CLIENT1, &reqs[0]));
            assert_allocation(&rt, CLIENT1, 2, &alloc_from_req(CLIENT1, &reqs[1]));
            let st: State = rt.get_state();
            assert_eq!(3, st.next_allocation_id);
        }
        {
            // Make another allocation from a different client
            let reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
            let payload = make_receiver_hook_token_payload(CLIENT2, reqs.clone(), vec![], SIZE);
            h.receive_tokens(&rt, payload, BatchReturn::ok(1), BATCH_EMPTY, vec![3], 0).unwrap();

            // Verify allocations in state.
            assert_allocation(&rt, CLIENT2, 3, &alloc_from_req(CLIENT2, &reqs[0]));
            let st: State = rt.get_state();
            assert_eq!(4, st.next_allocation_id);
        }
        h.check_state(&rt);
    }

    #[test]
    fn receive_tokens_extend_claims() {
        let (h, rt) = new_harness();

        let term_min = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let term_max = term_min + 100;
        let term_start = 100;
        let sector = 1234;
        rt.set_epoch(term_start);
        let claim1 =
            make_claim("1", CLIENT1, PROVIDER1, SIZE, term_min, term_max, term_start, sector);
        let claim2 =
            make_claim("2", CLIENT2, PROVIDER2, SIZE * 2, term_min, term_max, term_start, sector);

        let cid1 = h.create_claim(&rt, &claim1).unwrap();
        let cid2 = h.create_claim(&rt, &claim2).unwrap();

        let reqs = vec![
            make_extension_req(PROVIDER1, cid1, term_max + 1000),
            make_extension_req(PROVIDER2, cid2, term_max + 2000),
        ];
        // Client1 extends both claims
        let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE * 3);
        h.receive_tokens(&rt, payload, BATCH_EMPTY, BatchReturn::ok(2), vec![], SIZE * 3).unwrap();

        // Verify claims in state.
        assert_claim(&rt, PROVIDER1, cid1, &Claim { term_max: term_max + 1000, ..claim1 });
        assert_claim(&rt, PROVIDER2, cid2, &Claim { term_max: term_max + 2000, ..claim2 });
        h.check_state(&rt);
    }

    #[test]
    fn receive_tokens_make_alloc_and_extend_claims() {
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);
        add_miner(&rt, PROVIDER2);

        let alloc_reqs =
            vec![make_alloc_req(&rt, PROVIDER1, SIZE), make_alloc_req(&rt, PROVIDER2, SIZE * 2)];

        let term_min = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let term_max = term_min + 100;
        let term_start = 100;
        let sector = 1234;
        rt.set_epoch(term_start);
        let claim1 =
            make_claim("1", CLIENT1, PROVIDER1, SIZE, term_min, term_max, term_start, sector);
        let claim2 =
            make_claim("2", CLIENT2, PROVIDER2, SIZE * 2, term_min, term_max, term_start, sector);
        let cid1 = h.create_claim(&rt, &claim1).unwrap();
        let cid2 = h.create_claim(&rt, &claim2).unwrap();

        let ext_reqs = vec![
            make_extension_req(PROVIDER1, cid1, term_max + 1000),
            make_extension_req(PROVIDER2, cid2, term_max + 2000),
        ];

        // CLIENT1 makes two new allocations and extends two existing claims.
        let payload =
            make_receiver_hook_token_payload(CLIENT1, alloc_reqs.clone(), ext_reqs, SIZE * 6);
        h.receive_tokens(
            &rt,
            payload,
            BatchReturn::ok(2),
            BatchReturn::ok(2),
            vec![3, 4],
            claim1.size.0 + claim2.size.0,
        )
        .unwrap();

        // Verify state.
        assert_allocation(&rt, CLIENT1, 3, &alloc_from_req(CLIENT1, &alloc_reqs[0]));
        assert_allocation(&rt, CLIENT1, 4, &alloc_from_req(CLIENT1, &alloc_reqs[1]));
        assert_claim(&rt, PROVIDER1, cid1, &Claim { term_max: term_max + 1000, ..claim1 });
        assert_claim(&rt, PROVIDER2, cid2, &Claim { term_max: term_max + 2000, ..claim2 });

        let st: State = rt.get_state();
        assert_eq!(5, st.next_allocation_id);
        h.check_state(&rt);
    }

    #[test]
    fn receive_requires_datacap_caller() {
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);

        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(
                &make_receiver_hook_token_payload(
                    CLIENT1,
                    vec![make_alloc_req(&rt, PROVIDER1, SIZE)],
                    vec![],
                    SIZE,
                ),
                "payload",
            )
            .unwrap(),
        };

        rt.set_caller(*MARKET_ACTOR_CODE_ID, STORAGE_MARKET_ACTOR_ADDR); // Wrong caller
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "caller address",
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_requires_to_self() {
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);

        let mut payload = make_receiver_hook_token_payload(
            CLIENT1,
            vec![make_alloc_req(&rt, PROVIDER1, SIZE)],
            vec![],
            SIZE,
        );
        // Set invalid receiver hook "to" address (should be the verified registry itself).
        payload.to = PROVIDER1;
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };

        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "token receiver expected to",
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_alloc_requires_miner_actor() {
        let (h, rt) = new_harness();
        let provider1 = Address::new_id(PROVIDER1);
        rt.set_address_actor_type(provider1, *ACCOUNT_ACTOR_CODE_ID);

        let reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
        let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            format!("allocation provider {} must be a miner actor", provider1.id().unwrap())
                .as_str(),
            h.receive_tokens(&rt, payload, BatchReturn::ok(1), BATCH_EMPTY, vec![1], 0),
        );
        h.check_state(&rt);
    }

    #[test]
    fn receive_invalid_alloc_reqs() {
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);
        add_miner(&rt, PROVIDER2);

        // Alloc too small
        {
            let reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE - 1)];
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE - 1);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation size 1048575 below minimum 1048576",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
        }
        // Min term too short
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
            reqs[0].term_min = MINIMUM_VERIFIED_ALLOCATION_TERM - 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation term min 518399 below limit 518400",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
        }
        // Max term too long
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
            reqs[0].term_max = MAXIMUM_VERIFIED_ALLOCATION_TERM + 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation term max 5259486 above limit 5259485",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
        }
        // Term minimum greater than maximum
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
            reqs[0].term_max = 2 * EPOCHS_IN_YEAR;
            reqs[0].term_min = reqs[0].term_max + 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation term min 2103795 exceeds term max 2103794",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
        }
        // Allocation expires too late
        {
            let mut reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
            reqs[0].expiration = *rt.epoch.borrow() + MAXIMUM_VERIFIED_ALLOCATION_EXPIRATION + 1;
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation expiration 172801 exceeds maximum 172800",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
        }
        // Tokens received doesn't match sum of allocation sizes
        {
            let reqs =
                vec![make_alloc_req(&rt, PROVIDER1, SIZE), make_alloc_req(&rt, PROVIDER2, SIZE)];
            let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE * 2 + 1);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "total allocation size 2097152 must match data cap amount received 2097153",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
        }
        // One bad request fails the lot
        {
            let reqs = vec![
                make_alloc_req(&rt, PROVIDER1, SIZE),
                make_alloc_req(&rt, PROVIDER2, SIZE - 1),
            ];
            let mut payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE * 2 - 1);
            payload.amount = TokenAmount::from_whole((SIZE * 2 - 1) as i64);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "allocation size 1048575 below minimum 1048576",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
        }
        h.check_state(&rt);
    }

    #[test]
    fn receive_invalid_extension_reqs() {
        let (h, rt) = new_harness();

        let term_min = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let term_max = term_min + 100;
        let term_start = 100;
        let sector = 1234;
        let claim1 =
            make_claim("1", CLIENT1, PROVIDER1, SIZE, term_min, term_max, term_start, sector);

        let cid1 = h.create_claim(&rt, &claim1).unwrap();
        let st: State = rt.get_state();

        // Extension too long
        {
            rt.replace_state(&st);
            let epoch = term_start + 1000;
            rt.set_epoch(epoch);
            let max_allowed_term = epoch - term_start + MAXIMUM_VERIFIED_ALLOCATION_TERM;
            let reqs = vec![make_extension_req(PROVIDER1, cid1, max_allowed_term + 1)];
            let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "term_max 5260486 for claim 1 exceeds maximum 5260485 at current epoch 1100",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
            // But just on the limit is allowed
            let reqs = vec![make_extension_req(PROVIDER1, cid1, max_allowed_term)];
            let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
            h.receive_tokens(&rt, payload, BATCH_EMPTY, BatchReturn::ok(1), vec![], SIZE).unwrap();
            h.check_state(&rt);
        }
        {
            // Claim already expired
            rt.replace_state(&st);
            let epoch = term_start + term_max + 1;
            let new_term = epoch - term_start + MINIMUM_VERIFIED_ALLOCATION_TERM;
            rt.set_epoch(epoch);
            let reqs = vec![make_extension_req(PROVIDER1, cid1, new_term)];
            let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
            expect_abort_contains_message(
                ExitCode::USR_FORBIDDEN,
                "claim 1 expired at 518600, current epoch 518601",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
            // But just at expiration is allowed
            let epoch = term_start + term_max;
            let new_term = epoch - term_start + MAXIMUM_VERIFIED_ALLOCATION_TERM; // Can get full max term now
            rt.set_epoch(epoch);
            let reqs = vec![make_extension_req(PROVIDER1, cid1, new_term)];
            let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
            h.receive_tokens(&rt, payload, BATCH_EMPTY, BatchReturn::ok(1), vec![], SIZE).unwrap();
            h.check_state(&rt);
        }
        {
            // Extension is zero
            rt.replace_state(&st);
            rt.set_epoch(term_start + 100);
            let reqs = vec![make_extension_req(PROVIDER1, cid1, term_max)];
            let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "term_max 518500 for claim 1 is not larger than existing term max 518500",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
            // Extension is negative
            let reqs = vec![make_extension_req(PROVIDER1, cid1, term_max - 1)];
            let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "term_max 518499 for claim 1 is not larger than existing term max 518500",
                h.receive_tokens(&rt, payload, BATCH_EMPTY, BATCH_EMPTY, vec![], 0),
            );
            // But extension by just 1 epoch is allowed
            let reqs = vec![make_extension_req(PROVIDER1, cid1, term_max + 1)];
            let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
            h.receive_tokens(&rt, payload, BATCH_EMPTY, BatchReturn::ok(1), vec![], SIZE).unwrap();
            h.check_state(&rt);
        }
    }
}

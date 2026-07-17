use fvm_shared::address::Address;
use lazy_static::lazy_static;

#[allow(dead_code)]
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
    use fil_actors_runtime::test_utils::MockRuntime;
    use fvm_shared::sector::StoragePower;

    pub fn verifier_allowance(rt: &MockRuntime) -> StoragePower {
        rt.policy.minimum_verified_allocation_size.clone() + 42
    }

    pub fn client_allowance(rt: &MockRuntime) -> StoragePower {
        verifier_allowance(rt) - 1
    }
}

mod construction {
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::MethodNum;
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::error::ExitCode;

    use fil_actor_verifreg::{Actor as VerifregActor, Method};
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
    use fil_actors_runtime::test_utils::*;
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
    use std::ops::Deref;

    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::MethodNum;
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::error::ExitCode;

    use fil_actor_verifreg::{Actor as VerifregActor, AddVerifierParams, Method};
    use fil_actors_runtime::test_utils::*;
    use harness::*;
    use util::*;

    use crate::*;

    // FIP-1249: AddVerifier is now deprecated and always returns USR_FORBIDDEN.
    // These tests verify the method is properly disabled.

    #[test]
    fn add_verifier_requires_root_caller() {
        // FIP-1249: AddVerifier always returns forbidden regardless of caller
        let (h, rt) = new_harness();
        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, Address::new_id(501));
        rt.expect_validate_caller_any();
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
        // FIP-1249: AddVerifier always returns forbidden, even for invalid params
        let (h, rt) = new_harness();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ROOT_ADDR);
        rt.expect_validate_caller_any();
        let allowance = rt.policy.minimum_verified_allocation_size.clone() - 1;
        let params = AddVerifierParams { address: *VERIFIER, allowance };
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
    fn add_verifier_rejects_root() {
        // FIP-1249: AddVerifier always returns forbidden
        let (h, rt) = new_harness();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ROOT_ADDR);
        rt.expect_validate_caller_any();
        let allowance = verifier_allowance(&rt);
        let params = AddVerifierParams { address: ROOT_ADDR, allowance };
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
    fn add_verifier_rejects_client() {
        // FIP-1249: AddVerifier always returns forbidden
        let (h, rt) = new_harness();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ROOT_ADDR);
        rt.expect_validate_caller_any();
        let allowance = verifier_allowance(&rt);
        let params = AddVerifierParams { address: *VERIFIER, allowance };
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
    fn add_verifier_rejects_unresolved_address() {
        // FIP-1249: AddVerifier always returns forbidden
        let (h, rt) = new_harness();
        let verifier_key_address = Address::new_secp256k1(&[3u8; 65]).unwrap();
        let allowance = verifier_allowance(&rt);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ROOT_ADDR);
        rt.expect_validate_caller_any();
        let params = AddVerifierParams { address: verifier_key_address, allowance };
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
    fn add_verifier_id_address() {
        // FIP-1249: AddVerifier is deprecated, always returns forbidden
        let (h, rt) = new_harness();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ROOT_ADDR);
        rt.expect_validate_caller_any();
        let allowance = verifier_allowance(&rt);
        let params = AddVerifierParams { address: *VERIFIER, allowance };
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
    fn add_verifier_resolves_address() {
        // FIP-1249: AddVerifier is deprecated, always returns forbidden
        let (h, rt) = new_harness();
        let pubkey_addr = Address::new_secp256k1(&[0u8; 65]).unwrap();
        rt.id_addresses.borrow_mut().insert(pubkey_addr, *VERIFIER);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ROOT_ADDR);
        rt.expect_validate_caller_any();
        let allowance = verifier_allowance(&rt);
        let params = AddVerifierParams { address: pubkey_addr, allowance };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifier as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    // FIP-1249: RemoveVerifier is now deprecated and always returns USR_FORBIDDEN.
    // These tests verify the method is properly disabled, regardless of caller or state.

    #[test]
    fn remove_verifier_disabled_for_non_root_caller() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        // FIP-1249: use direct state insertion instead of deprecated add_verifier
        h.add_verifier_directly(&rt, &VERIFIER, &allowance);

        let caller = Address::new_id(501);
        rt.expect_validate_caller_any();
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
    fn remove_verifier_disabled_even_if_verifier_does_not_exist() {
        let (h, rt) = new_harness();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.root);
        rt.expect_validate_caller_any();
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
    fn remove_verifier_disabled_for_root_caller() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        // FIP-1249: use direct state insertion instead of deprecated add_verifier
        h.add_verifier_directly(&rt, &VERIFIER, &allowance);

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.root);
        rt.expect_validate_caller_any();
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::RemoveVerifier as MethodNum,
                IpldBlock::serialize_cbor(VERIFIER.deref()).unwrap(),
            ),
        );
        h.check_state(&rt);
        // The verifier is untouched, since removal is disabled.
        h.assert_verifier_allowance(&rt, &VERIFIER, &allowance);
    }

    #[test]
    fn remove_verifier_disabled_id_address() {
        let (h, rt) = new_harness();
        let allowance = verifier_allowance(&rt);
        let verifier_pubkey = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.borrow_mut().insert(verifier_pubkey, *VERIFIER);
        // FIP-1249: use direct state insertion instead of deprecated add_verifier
        h.add_verifier_directly(&rt, &VERIFIER, &allowance);

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.root);
        rt.expect_validate_caller_any();
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::RemoveVerifier as MethodNum,
                IpldBlock::serialize_cbor(&verifier_pubkey).unwrap(),
            ),
        );
        h.check_state(&rt);
    }
}

mod clients {
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::MethodNum;
    use fvm_shared::error::ExitCode;

    use fil_actor_verifreg::{Actor as VerifregActor, AddVerifiedClientParams, Method};
    use fil_actors_runtime::test_utils::*;
    use harness::*;
    use util::*;

    use crate::*;

    // FIP-1249: AddVerifiedClient is now deprecated and always returns USR_FORBIDDEN.
    // All tests that previously exercised AddVerifiedClient behavior now verify it's properly disabled.

    #[test]
    fn many_verifiers_and_clients() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance_client = client_allowance(&rt);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn verifier_allowance_exhausted() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance = client_allowance(&rt);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn resolves_client_address() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance_client = client_allowance(&rt);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn minimum_allowance_ok() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance = rt.policy.minimum_verified_allocation_size.clone();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_unresolved_address() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance_client = client_allowance(&rt);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_allowance_below_minimum() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance = rt.policy.minimum_verified_allocation_size.clone() - 1;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_non_verifier_caller() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance_client = client_allowance(&rt);
        let caller = Address::new_id(209);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *CLIENT, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn add_verified_client_restricted_correctly() {
        // FIP-1249: Both exported and unexported AddVerifiedClient methods return forbidden
        let (h, rt) = new_harness();
        let allowance_client = client_allowance(&rt);
        let params = AddVerifiedClientParams { address: *CLIENT, allowance: allowance_client };

        // set caller to not-builtin
        rt.set_caller(*EVM_ACTOR_CODE_ID, *VERIFIER);

        // cannot call the unexported method num (still forbidden due to built-in check)
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "must be built-in",
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.reset();

        // exported method num also returns forbidden due to FIP-1249
        rt.set_caller(*EVM_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClientExported as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );

        h.check_state(&rt);
    }

    #[test]
    fn rejects_allowance_greater_than_verifier_cap() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance = &allowance_verifier + 1;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: ROOT_ADDR, allowance };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_root_as_client() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance_client = client_allowance(&rt);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: ROOT_ADDR, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_verifier_as_client() {
        // FIP-1249: AddVerifiedClient is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        let allowance_client = client_allowance(&rt);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *VERIFIER);
        rt.expect_validate_caller_any();
        let params = AddVerifiedClientParams { address: *VERIFIER, allowance: allowance_client };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::AddVerifiedClient as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }
}

mod allocs_claims {
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{ActorID, MethodNum};
    use num_traits::Zero;

    use fil_actor_verifreg::{
        Actor, AllocationID, ClaimTerm, DataCap, ExtendClaimTermsParams, GetClaimsParams, Method,
        State,
    };
    use fil_actors_runtime::FailCode;
    use fil_actors_runtime::runtime::policy_constants::{
        MINIMUM_VERIFIED_ALLOCATION_SIZE, MINIMUM_VERIFIED_ALLOCATION_TERM,
    };
    use fil_actors_runtime::test_utils::{
        ACCOUNT_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID, expect_abort, expect_abort_contains_message,
    };
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

        let expect_1 = vec![(id1, alloc1.clone())];
        let expect_2 = vec![(id2, alloc2.clone())];
        let expect_both = vec![(id1, alloc1.clone()), (id2, alloc2.clone())];

        // Can't remove allocations that aren't expired
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], vec![]).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_FORBIDDEN, ExitCode::USR_FORBIDDEN], ret.results.codes());
        assert_eq!(DataCap::zero(), ret.datacap_recovered);

        // Can't remove with wrong client ID
        rt.set_epoch(200);
        let ret = h.remove_expired_allocations(&rt, CLIENT2, vec![id1, id2], vec![]).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_NOT_FOUND, ExitCode::USR_NOT_FOUND], ret.results.codes());
        assert_eq!(DataCap::zero(), ret.datacap_recovered);

        // Remove the first alloc, which expired.
        rt.set_epoch(100);
        let ret =
            h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], expect_1.clone()).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::USR_FORBIDDEN], ret.results.codes());
        assert_eq!(DataCap::from(alloc1.size.0), ret.datacap_recovered);

        // Remove the second alloc (the first is no longer found).
        rt.set_epoch(200);
        let ret =
            h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], expect_2.clone()).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_NOT_FOUND, ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc2.size.0), ret.datacap_recovered);

        // Reset state and show we can remove two at once.
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![id1, id2], expect_both).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(total_size), ret.datacap_recovered);

        // Reset state and show that only what was asked for is removed.
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![id1], expect_1.clone()).unwrap();
        assert_eq!(vec![1], ret.considered);
        assert_eq!(vec![ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc1.size.0), ret.datacap_recovered);

        // Reset state and show that specifying none removes only expired allocations
        rt.set_epoch(0);
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![], vec![]).unwrap();
        assert_eq!(Vec::<AllocationID>::new(), ret.considered);
        assert_eq!(Vec::<ExitCode>::new(), ret.results.codes());
        assert_eq!(DataCap::zero(), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_some());
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_some());

        rt.set_epoch(100);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![], expect_1).unwrap();
        assert_eq!(vec![1], ret.considered);
        assert_eq!(vec![ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc1.size.0), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_none()); // removed
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_some());

        rt.set_epoch(200);
        let ret = h.remove_expired_allocations(&rt, CLIENT1, vec![], expect_2).unwrap();
        assert_eq!(vec![2], ret.considered);
        assert_eq!(vec![ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(alloc2.size.0), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_none()); // removed
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_none()); // removed

        // Reset state and show that specifying none removes *all* expired allocations
        rt.replace_state(&state_with_allocs);
        let ret = h
            .remove_expired_allocations(&rt, CLIENT1, vec![], vec![(id1, alloc1), (id2, alloc2)])
            .unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::OK], ret.results.codes());
        assert_eq!(DataCap::from(total_size), ret.datacap_recovered);
        assert!(h.load_alloc(&rt, CLIENT1, id1).is_none()); // removed
        assert!(h.load_alloc(&rt, CLIENT1, id2).is_none()); // removed
        h.check_state(&rt);
    }

    #[test]
    fn claim_allocs() {
        // FIP-1249: ClaimAllocations is deprecated and always returns forbidden.
        let (h, rt) = new_harness();

        let size = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;
        let alloc1 = make_alloc("1", CLIENT1, PROVIDER1, size);

        let id1 = h.create_alloc(&rt, &alloc1).unwrap();

        let sector = 1000;
        let expiry = MINIMUM_VERIFIED_ALLOCATION_TERM;

        // ClaimAllocations now returns forbidden
        let reqs = vec![make_claim_reqs(sector, expiry, &[(id1, &alloc1)])];
        rt.expect_validate_caller_type(vec![fil_actors_runtime::runtime::builtins::Type::Miner]);
        rt.set_caller(
            *fil_actors_runtime::test_utils::MINER_ACTOR_CODE_ID,
            Address::new_id(PROVIDER1),
        );
        let params =
            fil_actor_verifreg::ClaimAllocationsParams { sectors: reqs, all_or_nothing: false };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<Actor>(
                Method::ClaimAllocations as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
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
        // FIP-1249: ExtendClaimTerms is deprecated and always returns forbidden.
        let (h, rt) = new_harness();
        let min_term = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let max_term = min_term + 1000;

        let params = ExtendClaimTermsParams {
            terms: vec![ClaimTerm { provider: PROVIDER1, claim_id: 1, term_max: max_term + 1 }],
        };

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
        rt.expect_validate_caller_any();
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<Actor>(
                Method::ExtendClaimTerms as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }

    #[test]
    fn extend_claims_edge_cases() {
        // FIP-1249: ExtendClaimTerms is deprecated and always returns forbidden.
        let (h, rt) = new_harness();
        let min_term = MINIMUM_VERIFIED_ALLOCATION_TERM;
        let max_term = min_term + 1000;

        let params = ExtendClaimTermsParams {
            terms: vec![ClaimTerm { provider: PROVIDER1, claim_id: 1, term_max: max_term }],
        };
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(CLIENT1));
        rt.expect_validate_caller_any();
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<Actor>(
                Method::ExtendClaimTerms as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
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

        let expect_1 = vec![(id1, claim1.clone())];
        let expect_2 = vec![(id2, claim2.clone())];
        let expect_both = vec![(id1, claim1), (id2, claim2)];

        // None expired yet
        rt.set_epoch(term_start + term_min + 99);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![id1, id2], vec![]).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_FORBIDDEN, ExitCode::USR_FORBIDDEN], ret.results.codes());

        // One expired
        rt.set_epoch(term_start + term_min + 100);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![id1, id2], expect_1).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::USR_FORBIDDEN], ret.results.codes());

        // Both now expired
        rt.set_epoch(term_start + term_min + 200);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![id1, id2], expect_2).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::USR_NOT_FOUND, ExitCode::OK], ret.results.codes());

        // Reset state, and show that specifying none removes only expired allocations
        rt.set_epoch(term_start + term_min);
        rt.replace_state(&state_with_allocs);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![], vec![]).unwrap();
        assert_eq!(Vec::<AllocationID>::new(), ret.considered);
        assert_eq!(Vec::<ExitCode>::new(), ret.results.codes());
        assert!(h.load_claim(&rt, PROVIDER1, id1).is_some());
        assert!(h.load_claim(&rt, PROVIDER1, id2).is_some());

        rt.set_epoch(term_start + term_min + 200);
        let ret = h.remove_expired_claims(&rt, PROVIDER1, vec![], expect_both).unwrap();
        assert_eq!(vec![1, 2], ret.considered);
        assert_eq!(vec![ExitCode::OK, ExitCode::OK], ret.results.codes());
        assert!(h.load_claim(&rt, PROVIDER1, id1).is_none()); // removed
        assert!(h.load_claim(&rt, PROVIDER1, id2).is_none()); // removed
        h.check_state(&rt);
    }

    #[test]
    fn claims_restricted_correctly() {
        let (h, rt) = new_harness();

        // FIP-1249: ExtendClaimTerms is deprecated. Both exported and unexported return forbidden.
        let params = ExtendClaimTermsParams { terms: vec![] };

        // set caller to not-builtin
        rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(CLIENT1));

        // cannot call the unexported extend method num (still forbidden due to built-in check)
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "must be built-in",
            rt.call::<Actor>(
                Method::ExtendClaimTerms as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.reset();

        // exported method num also returns forbidden due to FIP-1249
        rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(CLIENT1));
        rt.expect_validate_caller_any();
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<Actor>(
                Method::ExtendClaimTermsExported as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );

        // GetClaims still works (not deprecated)

        let params = GetClaimsParams { claim_ids: vec![], provider: PROVIDER1 };
        // cannot call the unexported get claims method num
        rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(CLIENT1));
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "must be built-in",
            h.get_claims(&rt, PROVIDER1, vec![]),
        );

        rt.reset();

        // can call the exported method num
        rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(CLIENT1));
        rt.expect_validate_caller_any();
        rt.call::<Actor>(
            Method::GetClaimsExported as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap();

        rt.verify();

        h.check_state(&rt);
    }
}

mod datacap {
    use frc46_token::receiver::FRC46_TOKEN_TYPE;
    use fvm_actor_utils::receiver::UniversalReceiverParams;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{ActorID, MethodNum};

    use fil_actor_verifreg::{Actor as VerifregActor, Method};
    use fil_actors_runtime::cbor::serialize;
    use fil_actors_runtime::runtime::policy_constants::MINIMUM_VERIFIED_ALLOCATION_SIZE;
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::{DATACAP_TOKEN_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR};
    use harness::*;

    use crate::*;

    const CLIENT1: ActorID = 101;
    const PROVIDER1: ActorID = 301;
    const PROVIDER2: ActorID = 302;
    const SIZE: u64 = MINIMUM_VERIFIED_ALLOCATION_SIZE as u64;

    // FIP-1249: UniversalReceiverHook is deprecated and always returns forbidden.
    // Tests that previously exercised allocation creation and claim extension via tokens
    // now verify the method is properly disabled.

    #[test]
    fn receive_tokens_make_allocs() {
        // FIP-1249: UniversalReceiverHook is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);
        add_miner(&rt, PROVIDER2);

        let reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
        let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_tokens_extend_claims() {
        // FIP-1249: UniversalReceiverHook is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();

        let reqs = vec![make_extension_req(PROVIDER1, 1, 1000)];
        let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_tokens_make_alloc_and_extend_claims() {
        // FIP-1249: UniversalReceiverHook is deprecated, verify it returns forbidden
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);
        add_miner(&rt, PROVIDER2);

        let alloc_reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
        let payload = make_receiver_hook_token_payload(CLIENT1, alloc_reqs, vec![], SIZE);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
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
        // FIP-1249: UniversalReceiverHook is deprecated.
        // Even with a wrong "to" address, the method returns forbidden after caller validation.
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);

        let mut payload = make_receiver_hook_token_payload(
            CLIENT1,
            vec![make_alloc_req(&rt, PROVIDER1, SIZE)],
            vec![],
            SIZE,
        );
        payload.to = PROVIDER1;
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };

        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
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
        // FIP-1249: UniversalReceiverHook is deprecated, returns forbidden regardless of provider type
        let (h, rt) = new_harness();
        let provider1 = Address::new_id(PROVIDER1);
        rt.set_address_actor_type(provider1, *ACCOUNT_ACTOR_CODE_ID);

        let reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE)];
        let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_invalid_alloc_reqs() {
        // FIP-1249: UniversalReceiverHook is deprecated, returns forbidden for all requests
        let (h, rt) = new_harness();
        add_miner(&rt, PROVIDER1);

        let reqs = vec![make_alloc_req(&rt, PROVIDER1, SIZE - 1)];
        let payload = make_receiver_hook_token_payload(CLIENT1, reqs, vec![], SIZE - 1);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }

    #[test]
    fn receive_invalid_extension_reqs() {
        // FIP-1249: UniversalReceiverHook is deprecated, returns forbidden for all requests
        let (h, rt) = new_harness();

        let reqs = vec![make_extension_req(PROVIDER1, 1, 1000)];
        let payload = make_receiver_hook_token_payload(CLIENT1, vec![], reqs, SIZE);
        let params = UniversalReceiverParams {
            type_: FRC46_TOKEN_TYPE,
            payload: serialize(&payload, "payload").unwrap(),
        };
        rt.set_caller(*DATACAP_TOKEN_ACTOR_CODE_ID, DATACAP_TOKEN_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![DATACAP_TOKEN_ACTOR_ADDR]);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::UniversalReceiverHook as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        rt.verify();
        h.check_state(&rt);
    }
}

// Tests to match with Go github.com/filecoin-project/go-state-types/builtin/*/verifreg
mod serialization {
    use std::str::FromStr;

    use cid::Cid;
    use hex_literal::hex;

    use fil_actor_verifreg::{AllocationClaim, ClaimAllocationsParams, SectorAllocationClaims};
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::piece::PaddedPieceSize;

    #[test]
    fn claim_allocations_params() {
        let test_cases = vec![
            (
                ClaimAllocationsParams { sectors: vec![], all_or_nothing: false },
                // [[],false]
                &hex!("8280f4")[..],
            ),
            (
                ClaimAllocationsParams {
                    sectors: vec![SectorAllocationClaims {
                        sector: 101,
                        expiry: 202,
                        claims: vec![],
                    }],
                    all_or_nothing: true,
                },
                // [[[101,202,[]]],true]
                &hex!("828183186518ca80f5"),
            ),
            (
                ClaimAllocationsParams {
                    sectors: vec![
                        SectorAllocationClaims {
                            sector: 101,
                            expiry: 202,
                            claims: vec![
                                AllocationClaim {
                                    client: 303,
                                    allocation_id: 404,
                                    data: Cid::from_str("baga6ea4seaaqa").unwrap(),
                                    size: PaddedPieceSize(505),
                                },
                                AllocationClaim {
                                    client: 606,
                                    allocation_id: 707,
                                    data: Cid::from_str("baga6ea4seaaqc").unwrap(),
                                    size: PaddedPieceSize(808),
                                },
                            ],
                        },
                        SectorAllocationClaims { sector: 303, expiry: 404, claims: vec![] },
                    ],
                    all_or_nothing: true,
                },
                // [[[101,202,[[303,404,baga6ea4seaaqa,505],[606,707,baga6ea4seaaqc,808]]],[303,404,[]]],true]
                &hex!(
                    "828283186518ca828419012f190194d82a49000181e203922001001901f98419025e1902c3d82a49000181e203922001011903288319012f19019480f5"
                ),
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: ClaimAllocationsParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }
}

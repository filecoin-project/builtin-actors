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
    use fil_actors_runtime::test_utils::MockRuntime;
    use fvm_shared::sector::StoragePower;

    pub fn verifier_allowance(rt: &MockRuntime) -> StoragePower {
        rt.policy.minimum_verified_deal_size.clone() + 42
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

    use crate::*;
    use harness::*;

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

    use fil_actor_verifreg::{Actor as VerifregActor, AddVerifierParams, Method};
    use fil_actors_runtime::test_utils::*;

    use crate::*;
    use harness::*;
    use util::*;

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
        let allowance = rt.policy.minimum_verified_deal_size.clone() - 1;
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
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.add_verifier(&mut rt, &CLIENT, &allowance));
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

    use fil_actor_verifreg::{Actor as VerifregActor, AddVerifierClientParams, DataCap, Method};
    use fil_actors_runtime::test_utils::*;

    use crate::*;
    use harness::*;
    use util::*;

    #[test]
    fn many_verifiers_and_clients() {
        let (h, mut rt) = new_harness();
        // Each verifier has enough allowance for two clients.
        let allowance_client = client_allowance(&rt);
        let allowance_verifier = allowance_client.clone() + allowance_client.clone();
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();
        h.add_verifier(&mut rt, &VERIFIER2, &allowance_verifier).unwrap();

        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance_client, &allowance_client).unwrap();
        h.add_client(&mut rt, &VERIFIER, &CLIENT2, &allowance_client, &allowance_client).unwrap();

        h.add_client(&mut rt, &VERIFIER2, &CLIENT3, &allowance_client, &allowance_client).unwrap();
        h.add_client(&mut rt, &VERIFIER2, &CLIENT4, &allowance_client, &allowance_client).unwrap();

        // all clients should exist and verifiers should have no more allowance left
        h.assert_client_allowance(&rt, &CLIENT, &allowance_client);
        h.assert_client_allowance(&rt, &CLIENT2, &allowance_client);
        h.assert_client_allowance(&rt, &CLIENT3, &allowance_client);
        h.assert_client_allowance(&rt, &CLIENT4, &allowance_client);
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

        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &CLIENT2, &allowance, &allowance),
        );

        // One client should exist and verifier should have no more allowance left.
        h.assert_client_allowance(&rt, &CLIENT, &allowance);
        h.assert_verifier_allowance(&rt, &VERIFIER, &DataCap::from(0));
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
        h.add_client(&mut rt, &VERIFIER, &client_pubkey, &allowance_client, &allowance_client)
            .unwrap();

        // Adding another verified client with the same ID address increments
        // the data cap which has already been granted.
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();
        let expected_allowance = allowance_client.clone() + allowance_client.clone();
        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance_client, &expected_allowance).unwrap();
        h.check_state(&rt);
    }

    #[test]
    fn minimum_allowance_ok() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = rt.policy.minimum_verified_deal_size.clone();
        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance).unwrap();
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
            h.add_client(&mut rt, &VERIFIER, &client, &allowance_client, &allowance_client),
        );
        h.check_state(&rt);
    }

    #[test]
    fn rejects_allowance_below_minimum() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        h.add_verifier(&mut rt, &VERIFIER, &allowance_verifier).unwrap();

        let allowance = rt.policy.minimum_verified_deal_size.clone() - 1;
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance),
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
            h.add_client(&mut rt, &VERIFIER, &h.root, &allowance, &allowance),
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
            h.add_client(&mut rt, &VERIFIER, &h.root, &allowance_client, &allowance_client),
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
            h.add_client(&mut rt, &VERIFIER, &VERIFIER, &allowance_client, &allowance_client),
        );

        h.add_verifier(&mut rt, &VERIFIER2, &allowance_verifier).unwrap();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.add_client(&mut rt, &VERIFIER, &VERIFIER2, &allowance_client, &allowance_client),
        );

        h.check_state(&rt);
    }
}

mod claims {
    use crate::*;
    use harness::*;
    use util::*;
    use fil_actors_runtime::test_utils::make_piece_cid;
    use fil_actors_runtime::{MapMap};
    use fil_actors_runtime::runtime::Runtime;
    use fil_actor_verifreg::{Actor as VerifregActor, Method, State, Allocation, Claim, AllocationID, SectorAllocationClaim};
    use fvm_shared::piece::PaddedPieceSize;
    use fvm_shared::sector::SectorID;
    use fvm_shared::clock::ChainEpoch;
    use fvm_shared::{MethodNum, HAMT_BIT_WIDTH};
    use fvm_shared::error::ExitCode;


    fn make_alloc(expected_id: AllocationID, provider: Address) -> Allocation {
        Allocation{
            client: *CLIENT,
            provider,
            data: make_piece_cid(format!("{}", expected_id).as_bytes()),
            size: PaddedPieceSize(128),
            term_min: 1000,
            term_max: 2000,
            expiration: 100,
        }
    }

    fn make_claim(id: AllocationID, alloc: Allocation, sector_id: SectorID, sector_expiry: ChainEpoch) -> SectorAllocationClaim {
        SectorAllocationClaim { client: alloc.client, allocation_id: id, piece_cid: alloc.data, piece_size: alloc.size, sector_id, sector_expiry }
    }

    fn sector_id(provider: Address, number: u64) -> SectorID {
        SectorID {
            miner: provider.id().unwrap(),
            number,
        }
    }

    #[test]
    fn claim_allocs() {
        let (h, mut rt) = new_harness(); 
        let provider = *PROVIDER;
        
        let alloc1  = make_alloc(1, provider);
        let alloc2 = make_alloc(2, provider);
        let alloc3 = make_alloc(3, provider);

        h.create_alloc(&mut rt, alloc1.clone()).unwrap();
        h.create_alloc(&mut rt, alloc2.clone()).unwrap();
        h.create_alloc(&mut rt, alloc3.clone()).unwrap();

        let ret = h.claim_allocations(&mut rt, provider, vec![
            make_claim(1, alloc1, sector_id(provider, 1000), 1500),
            make_claim(2, alloc2, sector_id(provider,1000), 1500),
            make_claim(3, alloc3, sector_id(provider, 1000), 1500)
        ]).unwrap();

        assert_eq!(ret.codes(), vec![ExitCode::OK, ExitCode::OK, ExitCode::OK]);

         // check that state is as expected
         let st: State = rt.get_state();
         let mut allocs = MapMap::<_, Allocation>::from_root(rt.store(), &st.allocations, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH).unwrap();
         // allocs deleted
         assert!(allocs.get(*CLIENT, 1).unwrap().is_none());
         assert!(allocs.get(*CLIENT, 2).unwrap().is_none());
         assert!(allocs.get(*CLIENT, 3).unwrap().is_none());

        // claims inserted
        let mut claims = MapMap::<_, Claim>::from_root(rt.store(), &st.claims, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH).unwrap();
        assert_eq!(claims.get(provider, 1).unwrap().unwrap().client, *CLIENT);
        assert_eq!(claims.get(provider, 2).unwrap().unwrap().client, *CLIENT);
        assert_eq!(claims.get(provider, 3).unwrap().unwrap().client, *CLIENT);
         
    }
}

mod datacap {
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::address::Address;
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actor_verifreg::{Actor as VerifregActor, Method, RestoreBytesParams, UseBytesParams};
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::{STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};

    use crate::*;
    use harness::*;
    use util::*;

    #[test]
    fn consume_multiple_clients() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_deal_size.clone() * 10;

        let ca1 = rt.policy.minimum_verified_deal_size.clone() * 3;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &ca1);
        let ca2 = rt.policy.minimum_verified_deal_size.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT2, &allowance, &ca2); // FIXME redundant verifier
        let ca3 = rt.policy.minimum_verified_deal_size.clone() + 1;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT3, &allowance, &ca3);

        let deal_size = rt.policy.minimum_verified_deal_size.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &(ca1.clone() - &deal_size));

        h.use_bytes(&mut rt, &CLIENT2, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT2, &(ca2 - &deal_size));

        // Client 3 had less than minimum balance remaining.
        h.use_bytes(&mut rt, &CLIENT3, &deal_size).unwrap();
        h.assert_client_removed(&rt, &CLIENT3);

        // Client 1 uses more bytes.
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &(ca1.clone() - &deal_size - &deal_size));

        // Client 2 uses more bytes, exhausting allocation
        h.use_bytes(&mut rt, &CLIENT2, &deal_size).unwrap();
        h.assert_client_removed(&rt, &CLIENT2);
        h.check_state(&rt);
    }

    #[test]
    fn consume_then_fail_exhausted() {
        let (h, mut rt) = new_harness();
        let ve_allowance = rt.policy.minimum_verified_deal_size.clone() * 10;
        let cl_allowance = rt.policy.minimum_verified_deal_size.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &ve_allowance, &cl_allowance);

        // Use some allowance.
        let deal_size = rt.policy.minimum_verified_deal_size.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();

        // Attempt to use more than remaining.
        let deal_size = rt.policy.minimum_verified_deal_size.clone() + 2;
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.use_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state(&rt)
    }

    #[test]
    fn consume_resolves_client_address() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_deal_size.clone();

        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        let client_pubkey = Address::new_secp256k1(&[3u8; 65]).unwrap();
        rt.id_addresses.insert(client_pubkey, *CLIENT);
        h.use_bytes(&mut rt, &client_pubkey, &allowance).unwrap();
        h.check_state(&rt)
    }

    #[test]
    fn consume_then_fail_removed() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_deal_size.clone();
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use full allowance.
        h.use_bytes(&mut rt, &CLIENT, &allowance).unwrap();
        // Fail to use any more because client was removed.
        expect_abort(ExitCode::USR_NOT_FOUND, h.use_bytes(&mut rt, &CLIENT, &allowance));
        h.check_state(&rt)
    }

    #[test]
    fn consume_requires_market_actor_caller() {
        let (h, mut rt) = new_harness();
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*POWER_ACTOR_CODE_ID, *STORAGE_POWER_ACTOR_ADDR);
        let params = UseBytesParams {
            address: *CLIENT,
            deal_size: rt.policy.minimum_verified_deal_size.clone(),
        };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<VerifregActor>(
                Method::UseBytes as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state(&rt)
    }

    #[test]
    fn consume_requires_minimum_deal_size() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier_and_client(
            &mut rt,
            &VERIFIER,
            &CLIENT,
            &allowance_verifier,
            &allowance_client,
        );

        let deal_size = rt.policy.minimum_verified_deal_size.clone() - 1;
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.use_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state(&rt)
    }

    #[test]
    fn consume_requires_client_exists() {
        let (h, mut rt) = new_harness();
        let min_deal_size = rt.policy.minimum_verified_deal_size.clone();
        expect_abort(ExitCode::USR_NOT_FOUND, h.use_bytes(&mut rt, &CLIENT, &min_deal_size));
        h.check_state(&rt)
    }

    #[test]
    fn consume_requires_deal_size_below_allowance() {
        let (h, mut rt) = new_harness();
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier_and_client(
            &mut rt,
            &VERIFIER,
            &CLIENT,
            &allowance_verifier,
            &allowance_client,
        );

        let deal_size = allowance_client.clone() + 1;
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.use_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state(&rt)
    }

    #[test]
    fn restore_multiple_clients() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_deal_size.clone() * 10;

        let ca1 = rt.policy.minimum_verified_deal_size.clone() * 3;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &ca1);
        let ca2 = rt.policy.minimum_verified_deal_size.clone() * 2;
        h.add_client(&mut rt, &VERIFIER, &CLIENT2, &ca2, &ca2).unwrap();
        let ca3 = rt.policy.minimum_verified_deal_size.clone() + 1;
        h.add_client(&mut rt, &VERIFIER, &CLIENT3, &ca3, &ca3).unwrap();

        let deal_size = rt.policy.minimum_verified_deal_size.clone();
        h.restore_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &(ca1.clone() + &deal_size));

        h.restore_bytes(&mut rt, &CLIENT2, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT2, &(ca2.clone() + &deal_size));

        h.restore_bytes(&mut rt, &CLIENT3, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT3, &(ca3.clone() + &deal_size));

        // Clients 1 and 2 now use bytes.
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &ca1);

        h.use_bytes(&mut rt, &CLIENT2, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT2, &ca2);

        // Restore bytes back to all clients
        h.restore_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &(ca1.clone() + &deal_size));

        h.restore_bytes(&mut rt, &CLIENT2, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT2, &(ca2.clone() + &deal_size));

        h.restore_bytes(&mut rt, &CLIENT3, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT3, &(ca3.clone() + &deal_size + &deal_size));
        h.check_state(&rt);
    }

    #[test]
    fn restore_after_reducing_client_cap() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_deal_size.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use half allowance.
        let deal_size = rt.policy.minimum_verified_deal_size.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &rt.policy.minimum_verified_deal_size);

        // Restore it.
        h.restore_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &allowance);
        h.check_state(&rt)
    }

    #[test]
    fn restore_resolves_client_address() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_deal_size.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use half allowance.
        let deal_size = rt.policy.minimum_verified_deal_size.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &rt.policy.minimum_verified_deal_size);

        let client_pubkey = Address::new_secp256k1(&[3u8; 65]).unwrap();
        rt.id_addresses.insert(client_pubkey, *CLIENT);

        // Restore to pubkey address.
        h.restore_bytes(&mut rt, &client_pubkey, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &allowance);
        h.check_state(&rt)
    }

    #[test]
    fn restore_after_removing_client() {
        let (h, mut rt) = new_harness();
        let allowance = rt.policy.minimum_verified_deal_size.clone() + 1;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use allowance.
        let deal_size = rt.policy.minimum_verified_deal_size.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_removed(&rt, &CLIENT);

        // Restore it. Client has only the restored bytes (lost the +1 in original allowance).
        h.restore_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &deal_size);
        h.check_state(&rt)
    }

    #[test]
    fn restore_requires_market_actor_caller() {
        let (h, mut rt) = new_harness();
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*POWER_ACTOR_CODE_ID, *STORAGE_POWER_ACTOR_ADDR);
        let params = RestoreBytesParams {
            address: *CLIENT,
            deal_size: rt.policy.minimum_verified_deal_size.clone(),
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
        let allowance_verifier = verifier_allowance(&rt);
        let allowance_client = client_allowance(&rt);
        h.add_verifier_and_client(
            &mut rt,
            &VERIFIER,
            &CLIENT,
            &allowance_verifier,
            &allowance_client,
        );

        let deal_size = rt.policy.minimum_verified_deal_size.clone() - 1;
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.restore_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state(&rt)
    }

    #[test]
    fn restore_rejects_root() {
        let (h, mut rt) = new_harness();
        let deal_size = rt.policy.minimum_verified_deal_size.clone();
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
        let deal_size = rt.policy.minimum_verified_deal_size.clone();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.restore_bytes(&mut rt, &VERIFIER, &deal_size),
        );
        h.check_state(&rt)
    }
}

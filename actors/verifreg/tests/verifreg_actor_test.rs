use fvm_shared::address::Address;
use lazy_static::lazy_static;

use fil_actor_verifreg::{DataCap, MINIMUM_VERIFIED_DEAL_SIZE};

mod harness;

lazy_static! {
    static ref VERIFIER_ALLOWANCE: DataCap = MINIMUM_VERIFIED_DEAL_SIZE.clone() + 42;
    static ref CLIENT_ALLOWANCE: DataCap = VERIFIER_ALLOWANCE.clone() - 1;
    static ref VERIFIER: Address = Address::new_id(201);
    static ref VERIFIER2: Address = Address::new_id(202);
    static ref CLIENT: Address = Address::new_id(301);
    static ref CLIENT2: Address = Address::new_id(302);
    static ref CLIENT3: Address = Address::new_id(303);
    static ref CLIENT4: Address = Address::new_id(304);
}

mod construction {
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::encoding::RawBytes;
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
        h.check_state();
    }

    #[test]
    fn construct_resolves_non_id() {
        let mut rt = new_runtime();
        let h = Harness { root: *ROOT_ADDR };
        let root_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.insert(root_pubkey, h.root);
        h.construct_and_verify(&mut rt, &root_pubkey);
        h.check_state();
    }

    #[test]
    fn construct_fails_if_root_unresolved() {
        let mut rt = new_runtime();
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
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::encoding::RawBytes;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{MethodNum, METHOD_SEND};

    use fil_actor_verifreg::{
        Actor as VerifregActor, AddVerifierParams, Method, MINIMUM_VERIFIED_DEAL_SIZE,
    };
    use fil_actors_runtime::test_utils::*;

    use crate::*;
    use harness::*;

    #[test]
    fn add_verifier_requires_root_caller() {
        let (h, mut rt) = new_harness();
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
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() - 1;
        expect_abort(ExitCode::ErrIllegalArgument, h.add_verifier(&mut rt, &VERIFIER, &allowance));
        h.check_state();
    }

    #[test]
    fn add_verifier_rejects_root() {
        let (h, mut rt) = new_harness();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_verifier(&mut rt, &ROOT_ADDR, &VERIFIER_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn add_verifier_rejects_client() {
        let (h, mut rt) = new_harness();
        h.add_verifier_and_client(
            &mut rt,
            &VERIFIER,
            &CLIENT,
            &VERIFIER_ALLOWANCE,
            &VERIFIER_ALLOWANCE,
        );
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_verifier(&mut rt, &CLIENT, &VERIFIER_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn add_verifier_rejects_unresolved_address() {
        let (h, mut rt) = new_harness();
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
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        h.check_state();
    }

    #[test]
    fn add_verifier_resolves_address() {
        let (h, mut rt) = new_harness();
        let pubkey_addr = Address::new_secp256k1(&[0u8; 65]).unwrap();
        rt.id_addresses.insert(pubkey_addr, *VERIFIER);

        h.add_verifier(&mut rt, &pubkey_addr, &VERIFIER_ALLOWANCE).unwrap();
        h.check_state();
    }

    #[test]
    fn remove_requires_root() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();

        let caller = Address::new_id(501);
        rt.expect_validate_caller_addr(vec![h.root]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        assert_ne!(h.root, caller);
        expect_abort(
            ExitCode::SysErrForbidden,
            rt.call::<VerifregActor>(
                Method::RemoveVerifier as MethodNum,
                &RawBytes::serialize(*VERIFIER).unwrap(),
            ),
        );
        h.check_state();
    }

    #[test]
    fn remove_requires_verifier_exists() {
        let (h, mut rt) = new_harness();
        expect_abort(ExitCode::ErrIllegalArgument, h.remove_verifier(&mut rt, &VERIFIER));
        h.check_state();
    }

    #[test]
    fn remove_verifier() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        h.remove_verifier(&mut rt, &VERIFIER).unwrap();
        h.check_state();
    }

    #[test]
    fn remove_verifier_id_address() {
        let (h, mut rt) = new_harness();
        let verifier_pubkey = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.insert(verifier_pubkey, *VERIFIER);
        // Add using pubkey address.
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        // Remove using ID address.
        h.remove_verifier(&mut rt, &VERIFIER).unwrap();
        h.check_state();
    }
}

mod clients {
    use fvm_shared::address::{Address, BLS_PUB_LEN};
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::encoding::RawBytes;
    use fvm_shared::error::ExitCode;
    use fvm_shared::{MethodNum, METHOD_SEND};

    use fil_actor_verifreg::{
        Actor as VerifregActor, AddVerifierClientParams, DataCap, Method,
        MINIMUM_VERIFIED_DEAL_SIZE,
    };
    use fil_actors_runtime::test_utils::*;

    use crate::*;
    use harness::*;

    #[test]
    fn many_verifiers_and_clients() {
        let (h, mut rt) = new_harness();
        // Each verifier has enough allowance for two clients.
        let verifier_allowance = CLIENT_ALLOWANCE.clone() + CLIENT_ALLOWANCE.clone();
        h.add_verifier(&mut rt, &VERIFIER, &verifier_allowance).unwrap();
        h.add_verifier(&mut rt, &VERIFIER2, &verifier_allowance).unwrap();

        h.add_client(&mut rt, &VERIFIER, &CLIENT, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();
        h.add_client(&mut rt, &VERIFIER, &CLIENT2, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();

        h.add_client(&mut rt, &VERIFIER2, &CLIENT3, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();
        h.add_client(&mut rt, &VERIFIER2, &CLIENT4, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();

        // all clients should exist and verifiers should have no more allowance left
        h.assert_client_allowance(&rt, &CLIENT, &CLIENT_ALLOWANCE);
        h.assert_client_allowance(&rt, &CLIENT2, &CLIENT_ALLOWANCE);
        h.assert_client_allowance(&rt, &CLIENT3, &CLIENT_ALLOWANCE);
        h.assert_client_allowance(&rt, &CLIENT4, &CLIENT_ALLOWANCE);
        h.assert_verifier_allowance(&rt, &VERIFIER, &DataCap::from(0));
        h.assert_verifier_allowance(&rt, &VERIFIER2, &DataCap::from(0));
        h.check_state();
    }

    #[test]
    fn verifier_allowance_exhausted() {
        let (h, mut rt) = new_harness();
        // Verifier only has allowance for one client.
        h.add_verifier(&mut rt, &VERIFIER, &CLIENT_ALLOWANCE).unwrap();

        h.add_client(&mut rt, &VERIFIER, &CLIENT, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE).unwrap();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &VERIFIER, &CLIENT2, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );

        // One client should exist and verifier should have no more allowance left.
        h.assert_client_allowance(&rt, &CLIENT, &CLIENT_ALLOWANCE);
        h.assert_verifier_allowance(&rt, &VERIFIER, &DataCap::from(0));
        h.check_state();
    }

    #[test]
    fn resolves_client_address() {
        let (h, mut rt) = new_harness();

        let client_pubkey = Address::new_bls(&[7u8; BLS_PUB_LEN]).unwrap();
        rt.id_addresses.insert(client_pubkey, *CLIENT);

        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        h.add_client(&mut rt, &VERIFIER, &client_pubkey, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE)
            .unwrap();

        // Adding another verified client with the same ID address increments
        // the data cap which has already been granted.
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        let expected_allowance = CLIENT_ALLOWANCE.clone() + CLIENT_ALLOWANCE.clone();
        h.add_client(&mut rt, &VERIFIER, &CLIENT, &CLIENT_ALLOWANCE, &expected_allowance).unwrap();
        h.check_state();
    }

    #[test]
    fn minimum_allowance_ok() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();

        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance).unwrap();
        h.check_state();
    }

    #[test]
    fn rejects_unresolved_address() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();

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
            h.add_client(&mut rt, &VERIFIER, &client, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn rejects_allowance_below_minimum() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();

        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() - 1;
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance),
        );
        h.check_state();
    }

    #[test]
    fn rejects_non_verifier_caller() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();

        let caller = Address::new_id(209);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, caller);
        rt.expect_validate_caller_any();
        let params =
            AddVerifierClientParams { address: *CLIENT, allowance: CLIENT_ALLOWANCE.clone() };
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
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();

        let allowance = VERIFIER_ALLOWANCE.clone() + 1;
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &VERIFIER, &h.root, &allowance, &allowance),
        );
        h.check_state();
    }

    #[test]
    fn rejects_root_as_client() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &VERIFIER, &h.root, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );
        h.check_state();
    }

    #[test]
    fn rejects_verifier_as_client() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &VERIFIER, &VERIFIER, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );

        h.add_verifier(&mut rt, &VERIFIER2, &VERIFIER_ALLOWANCE).unwrap();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.add_client(&mut rt, &VERIFIER, &VERIFIER2, &CLIENT_ALLOWANCE, &CLIENT_ALLOWANCE),
        );

        h.check_state();
    }
}

mod datacap {
    use fvm_shared::address::Address;
    use fvm_shared::encoding::RawBytes;
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actor_verifreg::{
        Actor as VerifregActor, Method, RestoreBytesParams, UseBytesParams,
        MINIMUM_VERIFIED_DEAL_SIZE,
    };
    use fil_actors_runtime::test_utils::*;
    use fil_actors_runtime::{STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};

    use crate::*;
    use harness::*;

    #[test]
    fn consume_multiple_clients() {
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 10;

        let ca1 = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 3;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &ca1);
        let ca2 = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT2, &allowance, &ca2); // FIXME redundant verifier
        let ca3 = MINIMUM_VERIFIED_DEAL_SIZE.clone() + 1;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT3, &allowance, &ca3);

        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
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
        h.check_state();
    }

    #[test]
    fn consume_then_fail_exhausted() {
        let (h, mut rt) = new_harness();
        let ve_allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 10;
        let cl_allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &ve_allowance, &cl_allowance);

        // Use some allowance.
        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();

        // Attempt to use more than remaining.
        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone() + 2;
        expect_abort(ExitCode::ErrIllegalArgument, h.use_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state()
    }

    #[test]
    fn consume_resolves_client_address() {
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone();

        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        let client_pubkey = Address::new_secp256k1(&[3u8; 65]).unwrap();
        rt.id_addresses.insert(client_pubkey, *CLIENT);
        h.use_bytes(&mut rt, &client_pubkey, &allowance).unwrap();
        h.check_state()
    }

    #[test]
    fn consume_then_fail_removed() {
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use full allowance.
        h.use_bytes(&mut rt, &CLIENT, &allowance).unwrap();
        // Fail to use any more because client was removed.
        expect_abort(ExitCode::ErrNotFound, h.use_bytes(&mut rt, &CLIENT, &allowance));
        h.check_state()
    }

    #[test]
    fn consume_requires_market_actor_caller() {
        let (h, mut rt) = new_harness();
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*POWER_ACTOR_CODE_ID, *STORAGE_POWER_ACTOR_ADDR);
        let params =
            UseBytesParams { address: *CLIENT, deal_size: MINIMUM_VERIFIED_DEAL_SIZE.clone() };
        expect_abort(
            ExitCode::SysErrForbidden,
            rt.call::<VerifregActor>(
                Method::UseBytes as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state()
    }

    #[test]
    fn consume_requires_minimum_deal_size() {
        let (h, mut rt) = new_harness();
        h.add_verifier_and_client(
            &mut rt,
            &VERIFIER,
            &CLIENT,
            &VERIFIER_ALLOWANCE,
            &CLIENT_ALLOWANCE,
        );

        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone() - 1;
        expect_abort(ExitCode::ErrIllegalArgument, h.use_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state()
    }

    #[test]
    fn consume_requires_client_exists() {
        let (h, mut rt) = new_harness();
        expect_abort(
            ExitCode::ErrNotFound,
            h.use_bytes(&mut rt, &CLIENT, &MINIMUM_VERIFIED_DEAL_SIZE),
        );
        h.check_state()
    }

    #[test]
    fn consume_requires_deal_size_below_allowance() {
        let (h, mut rt) = new_harness();
        h.add_verifier_and_client(
            &mut rt,
            &VERIFIER,
            &CLIENT,
            &VERIFIER_ALLOWANCE,
            &CLIENT_ALLOWANCE,
        );

        let deal_size = CLIENT_ALLOWANCE.clone() + 1;
        expect_abort(ExitCode::ErrIllegalArgument, h.use_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state()
    }

    #[test]
    fn restore_multiple_clients() {
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 10;

        let ca1 = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 3;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &ca1);
        let ca2 = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 2;
        h.add_client(&mut rt, &VERIFIER, &CLIENT2, &ca2, &ca2).unwrap();
        let ca3 = MINIMUM_VERIFIED_DEAL_SIZE.clone() + 1;
        h.add_client(&mut rt, &VERIFIER, &CLIENT3, &ca3, &ca3).unwrap();

        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
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
        h.check_state();
    }

    #[test]
    fn restore_after_reducing_client_cap() {
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use half allowance.
        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &MINIMUM_VERIFIED_DEAL_SIZE);

        // Restore it.
        h.restore_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &allowance);
        h.check_state()
    }

    #[test]
    fn restore_resolves_client_address() {
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() * 2;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use half allowance.
        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &MINIMUM_VERIFIED_DEAL_SIZE);

        let client_pubkey = Address::new_secp256k1(&[3u8; 65]).unwrap();
        rt.id_addresses.insert(client_pubkey, *CLIENT);

        // Restore to pubkey address.
        h.restore_bytes(&mut rt, &client_pubkey, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &allowance);
        h.check_state()
    }

    #[test]
    fn restore_after_removing_client() {
        let (h, mut rt) = new_harness();
        let allowance = MINIMUM_VERIFIED_DEAL_SIZE.clone() + 1;
        h.add_verifier_and_client(&mut rt, &VERIFIER, &CLIENT, &allowance, &allowance);

        // Use allowance.
        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        h.use_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_removed(&rt, &CLIENT);

        // Restore it. Client has only the restored bytes (lost the +1 in original allowance).
        h.restore_bytes(&mut rt, &CLIENT, &deal_size).unwrap();
        h.assert_client_allowance(&rt, &CLIENT, &deal_size);
        h.check_state()
    }

    #[test]
    fn restore_requires_market_actor_caller() {
        let (h, mut rt) = new_harness();
        rt.expect_validate_caller_addr(vec![*STORAGE_MARKET_ACTOR_ADDR]);
        rt.set_caller(*POWER_ACTOR_CODE_ID, *STORAGE_POWER_ACTOR_ADDR);
        let params =
            RestoreBytesParams { address: *CLIENT, deal_size: MINIMUM_VERIFIED_DEAL_SIZE.clone() };
        expect_abort(
            ExitCode::SysErrForbidden,
            rt.call::<VerifregActor>(
                Method::RestoreBytes as MethodNum,
                &RawBytes::serialize(params).unwrap(),
            ),
        );
        h.check_state()
    }

    #[test]
    fn restore_requires_minimum_deal_size() {
        let (h, mut rt) = new_harness();
        h.add_verifier_and_client(
            &mut rt,
            &VERIFIER,
            &CLIENT,
            &VERIFIER_ALLOWANCE,
            &CLIENT_ALLOWANCE,
        );

        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone() - 1;
        expect_abort(ExitCode::ErrIllegalArgument, h.restore_bytes(&mut rt, &CLIENT, &deal_size));
        h.check_state()
    }

    #[test]
    fn restore_rejects_root() {
        let (h, mut rt) = new_harness();
        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        expect_abort(
            ExitCode::ErrIllegalArgument,
            h.restore_bytes(&mut rt, &ROOT_ADDR, &deal_size),
        );
        h.check_state()
    }

    #[test]
    fn restore_rejects_verifier() {
        let (h, mut rt) = new_harness();
        h.add_verifier(&mut rt, &VERIFIER, &VERIFIER_ALLOWANCE).unwrap();
        let deal_size = MINIMUM_VERIFIED_DEAL_SIZE.clone();
        expect_abort(ExitCode::ErrIllegalArgument, h.restore_bytes(&mut rt, &VERIFIER, &deal_size));
        h.check_state()
    }
}

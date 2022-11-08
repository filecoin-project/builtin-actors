use fvm_shared::address::Address;
use lazy_static::lazy_static;

use fil_actors_runtime::test_utils::MockRuntime;
use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;

use crate::harness::{new_runtime, Harness};

mod harness;

lazy_static! {
    static ref ALICE: Address = Address::new_id(101);
    static ref BOB: Address = Address::new_id(102);
    static ref CARLA: Address = Address::new_id(103);
}

mod construction {
    use crate::*;
    use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;

    #[test]
    fn construct_with_verified() {
        let mut rt = new_runtime();
        let h = Harness { governor: VERIFIED_REGISTRY_ACTOR_ADDR };
        h.construct_and_verify(&mut rt, &h.governor);
        h.check_state(&rt);
    }
}

mod mint {
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actor_datacap::{Actor, Method, MintParams, INFINITE_ALLOWANCE};
    use fil_actors_runtime::cbor::serialize;
    use fil_actors_runtime::test_utils::{expect_abort_contains_message, MARKET_ACTOR_CODE_ID};
    use fil_actors_runtime::{STORAGE_MARKET_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};
    use fvm_ipld_encoding::RawBytes;
    use std::ops::Sub;

    use crate::*;

    #[test]
    fn mint_balances() {
        // The token library has far more extensive tests, this is just a sanity check.
        let (mut rt, h) = make_harness();

        let amt = TokenAmount::from_whole(1);
        let ret = h.mint(&mut rt, &*ALICE, &amt, vec![]).unwrap();
        assert_eq!(amt, ret.supply);
        assert_eq!(amt, ret.balance);
        assert_eq!(amt, h.get_supply(&rt));
        assert_eq!(amt, h.get_balance(&rt, &*ALICE));

        let ret = h.mint(&mut rt, &*BOB, &amt, vec![]).unwrap();
        assert_eq!(&amt * 2, ret.supply);
        assert_eq!(amt, ret.balance);
        assert_eq!(&amt * 2, h.get_supply(&rt));
        assert_eq!(amt, h.get_balance(&rt, &*BOB));

        h.check_state(&rt);
    }

    #[test]
    fn requires_verifreg_caller() {
        let (mut rt, h) = make_harness();
        let amt = TokenAmount::from_whole(1);
        let params = MintParams { to: *ALICE, amount: amt, operators: vec![] };

        rt.expect_validate_caller_addr(vec![VERIFIED_REGISTRY_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, STORAGE_MARKET_ACTOR_ADDR);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "caller address",
            rt.call::<Actor>(Method::Mint as MethodNum, &serialize(&params, "params").unwrap()),
        );
        h.check_state(&rt);
    }

    #[test]
    fn requires_whole_tokens() {
        let (mut rt, h) = make_harness();
        let amt = TokenAmount::from_atto(100);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "must be a multiple of 1000000000000000000",
            h.mint(&mut rt, &*ALICE, &amt, vec![]),
        );
        rt.reset();
        h.check_state(&rt);
    }

    #[test]
    fn auto_allowance_on_mint() {
        let (mut rt, h) = make_harness();
        let amt = TokenAmount::from_whole(42);
        h.mint(&mut rt, &*ALICE, &amt, vec![*BOB]).unwrap();
        let allowance = h.get_allowance_between(&rt, &*ALICE, &*BOB);
        assert!(allowance.eq(&INFINITE_ALLOWANCE));

        // mint again
        h.mint(&mut rt, &*ALICE, &amt, vec![*BOB]).unwrap();
        let allowance2 = h.get_allowance_between(&rt, &*ALICE, &*BOB);
        assert!(allowance2.eq(&INFINITE_ALLOWANCE));

        // transfer of an allowance *does* deduct allowance even though it is too small to matter in practice
        let operator_data = RawBytes::new(vec![1, 2, 3, 4]);
        h.transfer_from(&mut rt, &*BOB, &*ALICE, &h.governor, &(2 * amt.clone()), operator_data)
            .unwrap();
        let allowance3 = h.get_allowance_between(&rt, &*ALICE, &*BOB);
        assert!(allowance3.eq(&INFINITE_ALLOWANCE.clone().sub(2 * amt.clone())));

        // minting any amount to this address at the same operator resets at infinite
        h.mint(&mut rt, &*ALICE, &TokenAmount::from_whole(1), vec![*BOB]).unwrap();
        let allowance = h.get_allowance_between(&rt, &*ALICE, &*BOB);
        assert!(allowance.eq(&INFINITE_ALLOWANCE));

        h.check_state(&rt);
    }
}

mod transfer {
    // Tests for the specific transfer restrictions of the datacap token.

    use crate::{make_harness, ALICE, BOB, CARLA};
    use fil_actors_runtime::test_utils::expect_abort_contains_message;
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;

    #[test]
    fn only_governor_allowed() {
        let (mut rt, h) = make_harness();
        let operator_data = RawBytes::new(vec![1, 2, 3, 4]);

        let amt = TokenAmount::from_whole(1);
        h.mint(&mut rt, &*ALICE, &amt, vec![]).unwrap();

        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer(&mut rt, &*ALICE, &*BOB, &amt, operator_data.clone()),
        );
        rt.reset();

        // Transfer to governor is allowed.
        h.transfer(&mut rt, &*ALICE, &h.governor, &amt, operator_data.clone()).unwrap();

        // The governor can transfer out.
        h.transfer(&mut rt, &h.governor, &*BOB, &amt, operator_data).unwrap();
    }

    #[test]
    fn transfer_from_restricted() {
        let (mut rt, h) = make_harness();
        let operator_data = RawBytes::new(vec![1, 2, 3, 4]);

        let amt = TokenAmount::from_whole(1);
        h.mint(&mut rt, &*ALICE, &amt, vec![*BOB]).unwrap();

        // operator can't transfer out to third address
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer_from(&mut rt, &*BOB, &*ALICE, &*CARLA, &amt, operator_data.clone()),
        );
        rt.reset();

        // operator can't transfer out to self
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer_from(&mut rt, &*BOB, &*ALICE, &*BOB, &amt, operator_data.clone()),
        );
        rt.reset();
        // even if governor has a delegate operator and enough tokens, delegated transfer
        // cannot send to non governor
        h.mint(&mut rt, &h.governor, &amt, vec![*BOB]).unwrap();
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer_from(&mut rt, &*BOB, &h.governor, &*ALICE, &amt, operator_data),
        );
        rt.reset();
    }
}

mod destroy {
    use crate::{make_harness, ALICE, BOB};
    use fil_actor_datacap::DestroyParams;
    use fil_actors_runtime::test_utils::{expect_abort_contains_message, ACCOUNT_ACTOR_CODE_ID};
    use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::MethodNum;

    use fil_actor_datacap::{Actor, Method};
    use fil_actors_runtime::cbor::serialize;
    use fvm_shared::error::ExitCode;

    #[test]
    fn only_governor_allowed() {
        let (mut rt, h) = make_harness();

        let amt = TokenAmount::from_whole(1);
        h.mint(&mut rt, &*ALICE, &(2 * amt.clone()), vec![*BOB]).unwrap();

        // destroying from operator does not work
        let params = DestroyParams { owner: *ALICE, amount: amt.clone() };

        rt.expect_validate_caller_addr(vec![VERIFIED_REGISTRY_ACTOR_ADDR]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *BOB);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "caller address",
            rt.call::<Actor>(Method::Destroy as MethodNum, &serialize(&params, "params").unwrap()),
        );

        // Destroying from 0 allowance having governor works
        assert!(h.get_allowance_between(&rt, &*ALICE, &h.governor).is_zero());
        let ret = h.destroy(&mut rt, &*ALICE, &amt).unwrap();
        assert_eq!(ret.balance, amt); // burned 2 amt - amt = amt
        h.check_state(&rt)
    }
}

fn make_harness() -> (MockRuntime, Harness) {
    let mut rt = new_runtime();
    let h = Harness { governor: VERIFIED_REGISTRY_ACTOR_ADDR };
    h.construct_and_verify(&mut rt, &h.governor);
    (rt, h)
}

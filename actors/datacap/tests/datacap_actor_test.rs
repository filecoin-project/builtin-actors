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
        let h = Harness { registry: *VERIFIED_REGISTRY_ACTOR_ADDR };
        h.construct_and_verify(&mut rt, &h.registry);
        h.check_state(&rt);
    }
}

mod mint {
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actor_datacap::{Actor, Method, MintParams};
    use fil_actors_runtime::cbor::serialize;
    use fil_actors_runtime::test_utils::{expect_abort_contains_message, MARKET_ACTOR_CODE_ID};
    use fil_actors_runtime::{STORAGE_MARKET_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};

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
        let (mut rt, _) = make_harness();
        let amt = TokenAmount::from_whole(1);
        let params = MintParams { to: *ALICE, amount: amt, operators: vec![] };

        rt.expect_validate_caller_addr(vec![*VERIFIED_REGISTRY_ACTOR_ADDR]);
        rt.set_caller(*MARKET_ACTOR_CODE_ID, *STORAGE_MARKET_ACTOR_ADDR);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "caller address",
            rt.call::<Actor>(Method::Mint as MethodNum, &serialize(&params, "params").unwrap()),
        );
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
    }

    #[test]
    fn adds_allowance_for_operator() {
        let (mut rt, h) = make_harness();

        let amt = TokenAmount::from_whole(1);
        let allowance = TokenAmount::from_whole(2_000_000_000);
        h.mint(&mut rt, &*ALICE, &amt, vec![*BOB]).unwrap();
        assert_eq!(allowance, h.get_allowance(&rt, &*ALICE, &BOB));

        h.mint(&mut rt, &*ALICE, &amt, vec![*BOB]).unwrap();
        // Note: the doubling of allowance here is unfortunate, should be resolved when
        // the token library offers a set_allowance method.
        assert_eq!(&allowance * 2, h.get_allowance(&rt, &*ALICE, &BOB));

        // Different operator
        h.mint(&mut rt, &*ALICE, &amt, vec![*CARLA]).unwrap();
        assert_eq!(&allowance * 2, h.get_allowance(&rt, &*ALICE, &BOB));
        assert_eq!(allowance, h.get_allowance(&rt, &*ALICE, &CARLA));

        h.check_state(&rt);
    }
}

fn make_harness() -> (MockRuntime, Harness) {
    let mut rt = new_runtime();
    let h = Harness { registry: *VERIFIED_REGISTRY_ACTOR_ADDR };
    h.construct_and_verify(&mut rt, &h.registry);
    (rt, h)
}

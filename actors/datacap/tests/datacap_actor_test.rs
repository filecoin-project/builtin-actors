use fvm_shared::address::Address;
use lazy_static::lazy_static;

use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
use fil_actors_runtime::test_utils::MockRuntime;

use crate::harness::{Harness, new_runtime};

mod harness;

lazy_static! {
    static ref ALICE: Address = Address::new_id(101);
    static ref BOB: Address = Address::new_id(102);
    static ref CARLA: Address = Address::new_id(103);
}

mod construction {
    use crate::*;
    use fil_actor_datacap::{Actor, DATACAP_GRANULARITY, GranularityReturn, Method};
    use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
    use fvm_shared::MethodNum;

    #[test]
    fn construct_with_verified() {
        let rt = new_runtime();
        let h = Harness { governor: VERIFIED_REGISTRY_ACTOR_ADDR };
        h.construct_and_verify(&rt, &h.governor);
        h.check_state(&rt);

        rt.expect_validate_caller_any();
        let ret: GranularityReturn = rt
            .call::<Actor>(Method::GranularityExported as MethodNum, None)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();
        rt.verify();
        assert_eq!(ret.granularity, DATACAP_GRANULARITY);

        rt.expect_validate_caller_any();
        let ret: String = rt
            .call::<Actor>(Method::NameExported as MethodNum, None)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();
        rt.verify();
        assert_eq!(ret, "DataCap");

        rt.expect_validate_caller_any();
        let ret: String = rt
            .call::<Actor>(Method::SymbolExported as MethodNum, None)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();
        rt.verify();
        assert_eq!(ret, "DCAP")
    }
}

// FIP-1249: Mint is now deprecated and always returns USR_FORBIDDEN.
// These tests verify the method is properly disabled, regardless of caller or params.
mod mint {
    use fvm_shared::MethodNum;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;

    use fil_actor_datacap::{Actor, Method, MintParams};
    use fil_actors_runtime::test_utils::{
        MARKET_ACTOR_CODE_ID, VERIFREG_ACTOR_CODE_ID, expect_abort_contains_message,
    };
    use fil_actors_runtime::{STORAGE_MARKET_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};
    use fvm_ipld_encoding::ipld_block::IpldBlock;

    use crate::*;

    #[test]
    fn mint_disabled_for_governor_caller() {
        let (rt, h) = make_harness();
        let amt = TokenAmount::from_whole(1);
        let params = MintParams { to: *ALICE, amount: amt, operators: vec![] };

        rt.expect_validate_caller_any();
        rt.set_caller(*VERIFREG_ACTOR_CODE_ID, VERIFIED_REGISTRY_ACTOR_ADDR);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "datacap is deprecated",
            rt.call::<Actor>(
                Method::MintExported as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
        assert!(h.get_balance(&rt, &ALICE).is_zero());
    }

    #[test]
    fn mint_disabled_for_arbitrary_caller() {
        let (rt, h) = make_harness();
        let amt = TokenAmount::from_whole(1);
        let params = MintParams { to: *ALICE, amount: amt, operators: vec![] };

        rt.expect_validate_caller_any();
        rt.set_caller(*MARKET_ACTOR_CODE_ID, STORAGE_MARKET_ACTOR_ADDR);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "datacap is deprecated",
            rt.call::<Actor>(
                Method::MintExported as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );
        h.check_state(&rt);
    }
}

mod transfer {
    // Tests for the specific transfer restrictions of the datacap token.

    use crate::{ALICE, BOB, CARLA, make_harness};
    use fil_actors_runtime::test_utils::expect_abort_contains_message;
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::error::ExitCode;

    #[test]
    fn only_governor_allowed() {
        let (rt, h) = make_harness();
        let operator_data = RawBytes::new(vec![1, 2, 3, 4]);

        let amt = TokenAmount::from_whole(1);
        h.mint_directly(&rt, &ALICE, &amt);

        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer(&rt, &ALICE, &BOB, &amt, operator_data.clone()),
        );
        rt.reset();

        // Transfer to governor is allowed.
        h.transfer(&rt, &ALICE, &h.governor, &amt, operator_data.clone()).unwrap();

        // The governor can transfer out.
        h.transfer(&rt, &h.governor, &BOB, &amt, operator_data).unwrap();
    }

    #[test]
    fn transfer_from_restricted() {
        let (rt, h) = make_harness();
        let operator_data = RawBytes::new(vec![1, 2, 3, 4]);

        let amt = TokenAmount::from_whole(1);
        h.mint_directly(&rt, &ALICE, &amt);
        h.allow_directly(&rt, &ALICE, &BOB, &amt);

        // operator can't transfer out to third address
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer_from(&rt, &BOB, &ALICE, &CARLA, &amt, operator_data.clone()),
        );
        rt.reset();

        // operator can't transfer out to self
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer_from(&rt, &BOB, &ALICE, &BOB, &amt, operator_data.clone()),
        );
        rt.reset();
        // even if governor has a delegate operator and enough tokens, delegated transfer
        // cannot send to non governor
        h.mint_directly(&rt, &h.governor, &amt);
        h.allow_directly(&rt, &h.governor, &BOB, &amt);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "transfer not allowed",
            h.transfer_from(&rt, &BOB, &h.governor, &ALICE, &amt, operator_data),
        );
        rt.reset();
    }
}

mod destroy {
    use crate::{ALICE, BOB, make_harness};
    use fil_actor_datacap::DestroyParams;
    use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
    use fil_actors_runtime::test_utils::{ACCOUNT_ACTOR_CODE_ID, expect_abort_contains_message};
    use fvm_shared::MethodNum;
    use fvm_shared::econ::TokenAmount;

    use fil_actor_datacap::{Actor, Method};
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::error::ExitCode;

    #[test]
    fn only_governor_allowed() {
        let (rt, h) = make_harness();

        let amt = TokenAmount::from_whole(1);
        h.mint_directly(&rt, &ALICE, &(2 * amt.clone()));

        // destroying from operator does not work
        let params = DestroyParams { owner: *ALICE, amount: amt.clone() };

        rt.expect_validate_caller_addr(vec![VERIFIED_REGISTRY_ACTOR_ADDR]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *BOB);
        expect_abort_contains_message(
            ExitCode::USR_FORBIDDEN,
            "caller address",
            rt.call::<Actor>(
                Method::DestroyExported as MethodNum,
                IpldBlock::serialize_cbor(&params).unwrap(),
            ),
        );

        // Destroying from 0 allowance having governor works
        assert!(h.get_allowance_between(&rt, &ALICE, &h.governor).is_zero());
        let ret = h.destroy(&rt, &ALICE, &amt).unwrap();
        assert_eq!(ret.balance, amt); // burned 2 amt - amt = amt
        h.check_state(&rt)
    }
}

fn make_harness() -> (MockRuntime, Harness) {
    let rt = new_runtime();
    let h = Harness { governor: VERIFIED_REGISTRY_ACTOR_ADDR };
    h.construct_and_verify(&rt, &h.governor);
    (rt, h)
}

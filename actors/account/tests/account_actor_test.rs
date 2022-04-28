// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_account::{Actor as AccountActor, State};
use fil_actors_runtime::builtin::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;

macro_rules! account_tests {
    ($($name:ident: $value:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let (addr, exit_code) = $value;

                let mut rt = MockRuntime {
                    receiver: fvm_shared::address::Address::new_id(100),
                    caller: SYSTEM_ACTOR_ADDR.clone(),
                    caller_type: SYSTEM_ACTOR_CODE_ID.clone(),
                    ..Default::default()
                };
                rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);

                if exit_code.is_success() {
                    rt.call::<AccountActor>(1, &RawBytes::serialize(addr).unwrap()).unwrap();

                    let state: State = rt.get_state();
                    assert_eq!(state.address, addr);
                    rt.expect_validate_caller_any();

                    let pk: Address = rt
                        .call::<AccountActor>(2, &RawBytes::default())
                        .unwrap()
                        .deserialize()
                        .unwrap();
                    assert_eq!(pk, addr);
                } else {
                    expect_abort(
                        exit_code,
                        rt.call::<AccountActor>(1,&RawBytes::serialize(addr).unwrap())
                    )
                }
                rt.verify();
            }
        )*
    }
}

account_tests! {
    happy_construct_secp256k1_address: (
        Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap(),
        ExitCode::OK
    ),
    happy_construct_bls_address: (
        Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap(),
        ExitCode::OK
    ),
    fail_construct_id_address: (
        Address::new_id(1),
        ExitCode::USR_ILLEGAL_ARGUMENT
    ),
    fail_construct_actor_address: (
        Address::new_actor(&[1, 2, 3]),
        ExitCode::USR_ILLEGAL_ARGUMENT
    ),
}

// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::anyhow;
use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::{testing::check_state_invariants, Actor as AccountActor, State};
use fil_actors_runtime::builtin::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

use fil_actor_account::{testing::check_state_invariants, Actor as AccountActor, Method, State};
use fil_actors_runtime::builtin::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::test_utils::*;

#[test]
fn construction() {
    fn construct(addr: Address, exit_code: ExitCode) {
        let mut rt = MockRuntime {
            receiver: Address::new_id(100),
            caller: *SYSTEM_ACTOR_ADDR,
            caller_type: *SYSTEM_ACTOR_CODE_ID,
            ..Default::default()
        };
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);

        if exit_code.is_success() {
            rt.call::<AccountActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(addr).unwrap(),
            )
            .unwrap();

            let state: State = rt.get_state();
            assert_eq!(state.address, addr);
            rt.expect_validate_caller_any();

            let pk: Address = rt
                .call::<AccountActor>(Method::PubkeyAddress as MethodNum, &RawBytes::default())
                .unwrap()
                .deserialize()
                .unwrap();
            assert_eq!(pk, addr);
            check_state(&rt);
        } else {
            expect_abort(exit_code, rt.call::<AccountActor>(1, &RawBytes::serialize(addr).unwrap()))
        }
        rt.verify();
    }

    construct(
        Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap(),
        ExitCode::OK,
    );
    construct(Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap(), ExitCode::OK);
    construct(Address::new_id(1), ExitCode::USR_ILLEGAL_ARGUMENT);
    construct(Address::new_actor(&[1, 2, 3]), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn token_receiver() {
    let mut rt = MockRuntime {
        receiver: Address::new_id(100),
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    };
    rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);

    let param = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.call::<AccountActor>(
        Method::Constructor as MethodNum,
        &RawBytes::serialize(&param).unwrap(),
    )
    .unwrap();

    rt.expect_validate_caller_any();
    let ret = rt.call::<AccountActor>(
        Method::FungibleTokenReceiverHook as MethodNum,
        &RawBytes::new(vec![1, 2, 3]),
    );
    assert!(ret.is_ok());
    assert_eq!(RawBytes::default(), ret.unwrap());
}

fn check_state(rt: &MockRuntime) {
    let test_address = Address::new_id(1000);
    let (_, acc) = check_state_invariants(&rt.get_state(), &test_address);
    acc.assert_empty();
}

macro_rules! account_constructor_tests {
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
                rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

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

                    check_state(&rt);
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

account_constructor_tests! {
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

#[test]
fn authenticate_message() {
    let mut rt = MockRuntime {
        receiver: Address::new_id(100),
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    };

    let addr = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

    rt.call::<AccountActor>(1, &RawBytes::serialize(addr).unwrap()).unwrap();

    let state: State = rt.get_state();
    assert_eq!(state.address, addr);

    let params =
        RawBytes::serialize(AuthenticateMessageParams { signature: vec![], message: vec![] })
            .unwrap();

    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Ok(()),
    });
    assert_eq!(RawBytes::default(), rt.call::<AccountActor>(3, &params).unwrap());

    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Err(anyhow!("bad signature")),
    });
    assert_eq!(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<AccountActor>(3, &params).unwrap_err().exit_code()
    );

    rt.verify();
}

// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::anyhow;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::{testing::check_state_invariants, Actor as AccountActor, Method, State};
use fil_actors_runtime::builtin::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::test_utils::*;

#[test]
fn construction() {
    fn construct(addr: Address, exit_code: ExitCode) {
        let mut rt = MockRuntime { receiver: Address::new_id(100), ..Default::default() };
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

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
    let mut rt = MockRuntime { receiver: Address::new_id(100), ..Default::default() };
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

    let param = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.call::<AccountActor>(
        Method::Constructor as MethodNum,
        &RawBytes::serialize(&param).unwrap(),
    )
    .unwrap();

    rt.set_caller(make_identity_cid(b"1234"), Address::new_id(1000));
    rt.expect_validate_caller_any();
    let ret = rt.call::<AccountActor>(
        Method::UniversalReceiverHook as MethodNum,
        &RawBytes::new(vec![1, 2, 3]),
    );
    assert!(ret.is_ok());
    assert_eq!(RawBytes::default(), ret.unwrap());
}

#[test]
fn authenticate_message() {
    let mut rt = MockRuntime { receiver: Address::new_id(100), ..Default::default() };
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);

    let addr = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    rt.call::<AccountActor>(Method::Constructor as MethodNum, &RawBytes::serialize(addr).unwrap())
        .unwrap();

    let state: State = rt.get_state();
    assert_eq!(state.address, addr);

    let params =
        RawBytes::serialize(AuthenticateMessageParams { signature: vec![], message: vec![] })
            .unwrap();

    // Valid signature
    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Ok(()),
    });
    assert_eq!(
        RawBytes::default(),
        rt.call::<AccountActor>(Method::AuthenticateMessageExported as MethodNum, &params).unwrap()
    );
    rt.verify();

    // Invalid signature
    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Err(anyhow!("bad signature")),
    });
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "bad signature",
        rt.call::<AccountActor>(Method::AuthenticateMessageExported as MethodNum, &params),
    );
    rt.verify();

    // Ok to call exported method number
    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Ok(()),
    });
    rt.call::<AccountActor>(Method::AuthenticateMessageExported as MethodNum, &params).unwrap();
}

fn check_state(rt: &MockRuntime) {
    let test_address = Address::new_id(1000);
    let (_, acc) = check_state_invariants(&rt.get_state(), &test_address);
    acc.assert_empty();
}

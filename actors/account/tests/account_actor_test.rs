// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::anyhow;
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::{testing::check_state_invariants, Actor as AccountActor, Method, State};
use fil_actors_runtime::builtin::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::FIRST_EXPORTED_METHOD_NUMBER;

#[test]
fn construction() {
    fn construct(addr: Address, exit_code: ExitCode) {
        let rt = MockRuntime { receiver: Address::new_id(100), ..Default::default() };
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

        if exit_code.is_success() {
            rt.call::<AccountActor>(
                Method::Constructor as MethodNum,
                IpldBlock::serialize_cbor(&addr).unwrap(),
            )
            .unwrap();

            let state: State = rt.get_state();
            assert_eq!(state.address, addr);
            rt.expect_validate_caller_any();

            let pk: Address = rt
                .call::<AccountActor>(Method::PubkeyAddress as MethodNum, None)
                .unwrap()
                .unwrap()
                .deserialize()
                .unwrap();
            assert_eq!(pk, addr);
            check_state(&rt);
        } else {
            expect_abort(
                exit_code,
                rt.call::<AccountActor>(1, IpldBlock::serialize_cbor(&addr).unwrap()),
            )
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
    let rt = MockRuntime { receiver: Address::new_id(100), ..Default::default() };
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

    let param = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.call::<AccountActor>(
        Method::Constructor as MethodNum,
        IpldBlock::serialize_cbor(&param).unwrap(),
    )
    .unwrap();

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(1000));
    rt.expect_validate_caller_any();
    let ret = rt
        .call::<AccountActor>(
            frc42_dispatch::method_hash!("Receive"),
            IpldBlock::serialize_cbor(&UniversalReceiverParams {
                type_: 0,
                payload: RawBytes::new(vec![1, 2, 3]),
            })
            .unwrap(),
        )
        .unwrap();
    assert!(ret.is_none());
}

#[test]
fn authenticate_message() {
    let rt = MockRuntime { receiver: Address::new_id(100), ..Default::default() };
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);

    let addr = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    rt.call::<AccountActor>(
        Method::Constructor as MethodNum,
        IpldBlock::serialize_cbor(&addr).unwrap(),
    )
    .unwrap();

    let state: State = rt.get_state();
    assert_eq!(state.address, addr);

    let params = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: vec![],
        message: vec![],
    })
    .unwrap();

    // Valid signature
    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Ok(()),
    });

    assert!(rt
        .call::<AccountActor>(Method::AuthenticateMessageExported as MethodNum, params.clone())
        .unwrap()
        .unwrap()
        .deserialize::<bool>()
        .unwrap());

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
        rt.call::<AccountActor>(Method::AuthenticateMessageExported as MethodNum, params.clone()),
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
    assert!(rt
        .call::<AccountActor>(Method::AuthenticateMessageExported as MethodNum, params)
        .unwrap()
        .unwrap()
        .deserialize::<bool>()
        .unwrap());
}

#[test]
fn test_fallback() {
    let rt = MockRuntime { receiver: Address::new_id(100), ..Default::default() };
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);

    let addr = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    rt.call::<AccountActor>(
        Method::Constructor as MethodNum,
        IpldBlock::serialize_cbor(&addr).unwrap(),
    )
    .unwrap();

    let state: State = rt.get_state();
    assert_eq!(state.address, addr);

    // this is arbitrary
    let params = IpldBlock::serialize_cbor(&vec![1u8, 2u8, 3u8]).unwrap();

    // accept >= 2<<24
    rt.expect_validate_caller_any();
    let result = rt.call::<AccountActor>(FIRST_EXPORTED_METHOD_NUMBER, params.clone()).unwrap();
    assert!(result.is_none());

    rt.expect_validate_caller_any();
    let result = rt.call::<AccountActor>(FIRST_EXPORTED_METHOD_NUMBER + 1, params.clone()).unwrap();
    assert!(result.is_none());

    // reject < 1<<24
    rt.expect_validate_caller_any();
    let result = rt.call::<AccountActor>(FIRST_EXPORTED_METHOD_NUMBER - 1, params);
    assert!(result.is_err());

    rt.verify();
}

fn check_state(rt: &MockRuntime) {
    let test_address = Address::new_id(1000);
    let (_, acc) = check_state_invariants(&rt.get_state(), &test_address);
    acc.assert_empty();
}

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

#[test]
fn construction() {
    fn construct(addr: Address, exit_code: ExitCode) {
        let mut rt = MockRuntime {
            receiver: Address::new_id(100),
            caller: SYSTEM_ACTOR_ADDR,
            caller_type: *SYSTEM_ACTOR_CODE_ID,
            ..Default::default()
        };
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
    let mut rt = MockRuntime {
        receiver: Address::new_id(100),
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    };
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

    let param = Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap();
    rt.call::<AccountActor>(
        Method::Constructor as MethodNum,
        IpldBlock::serialize_cbor(&param).unwrap(),
    )
    .unwrap();

    rt.expect_validate_caller_any();
    let ret = rt.call::<AccountActor>(
        Method::UniversalReceiverHook as MethodNum,
        IpldBlock::serialize_cbor(&UniversalReceiverParams {
            type_: 0,
            payload: RawBytes::new(vec![1, 2, 3]),
        })
        .unwrap(),
    );
    assert!(ret.is_ok());
    assert_eq!(RawBytes::default(), ret.unwrap());
}

fn check_state(rt: &MockRuntime) {
    let test_address = Address::new_id(1000);
    let (_, acc) = check_state_invariants(&rt.get_state(), &test_address);
    acc.assert_empty();
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

    rt.call::<AccountActor>(1, IpldBlock::serialize_cbor(&addr).unwrap()).unwrap();

    let state: State = rt.get_state();
    assert_eq!(state.address, addr);

    let params = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: vec![],
        message: vec![],
    })
    .unwrap();

    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Ok(()),
    });
    assert_eq!(RawBytes::default(), rt.call::<AccountActor>(3, params.clone()).unwrap());

    rt.expect_validate_caller_any();
    rt.expect_verify_signature(ExpectedVerifySig {
        sig: Signature::new_secp256k1(vec![]),
        signer: addr,
        plaintext: vec![],
        result: Err(anyhow!("bad signature")),
    });
    assert_eq!(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<AccountActor>(3, params).unwrap_err().exit_code()
    );

    rt.verify();
}

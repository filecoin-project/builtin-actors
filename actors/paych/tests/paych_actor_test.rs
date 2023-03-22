// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cell::RefCell;
use std::collections::HashMap;

use cid::Cid;
use derive_builder::Builder;
use fil_actor_paych::ext::account::{AuthenticateMessageParams, AUTHENTICATE_MESSAGE_METHOD};
use fil_actor_paych::testing::check_state_invariants;
use fil_actor_paych::{
    Actor as PaychActor, ConstructorParams, LaneState, Merge, Method, ModVerifyParams,
    SignedVoucher, State as PState, UpdateChannelStateParams, MAX_LANE, SETTLE_DELAY,
};

use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_amt::Amt;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::METHOD_CONSTRUCTOR;
use num_traits::Zero;

const PAYCH_ID: u64 = 100;
const PAYER_ID: u64 = 102;
const PAYEE_ID: u64 = 103;

struct LaneParams {
    epoch_num: ChainEpoch,
    from: Address,
    to: Address,
    amt: TokenAmount,
    lane: u64,
    nonce: u64,
}

lazy_static::lazy_static! {
    pub static ref AUTHENTICATE_MESSAGE_RESPONSE: Option<IpldBlock> =
        IpldBlock::serialize_cbor(&true).unwrap();
}

fn call(rt: &MockRuntime, method_num: u64, ser: Option<IpldBlock>) -> Option<IpldBlock> {
    rt.call::<PaychActor>(method_num, ser).unwrap()
}

fn expect_abort(rt: &MockRuntime, method_num: u64, ser: Option<IpldBlock>, exp: ExitCode) {
    let err = rt.call::<PaychActor>(method_num, ser).unwrap_err();
    assert_eq!(exp, err.exit_code());
}

fn construct_lane_state_amt(rt: &MockRuntime, lss: Vec<LaneState>) -> Cid {
    let mut arr = Amt::new(&rt.store);
    for (i, ls) in (0..).zip(lss.into_iter()) {
        arr.set(i, ls).unwrap();
    }
    arr.flush().unwrap()
}

fn get_lane_state(rt: &MockRuntime, cid: &Cid, lane: u64) -> LaneState {
    let arr: Amt<LaneState, _> = Amt::load(cid, &rt.store).unwrap();

    arr.get(lane).unwrap().unwrap().clone()
}

fn check_state(rt: &MockRuntime) {
    let (_, acc) = check_state_invariants(&rt.get_state(), rt.store(), &rt.get_balance());
    acc.assert_empty();
}

mod paych_constructor {
    use fil_actors_runtime::runtime::builtins::Type;
    use fvm_shared::{METHOD_CONSTRUCTOR, METHOD_SEND};

    use super::*;

    const TEST_PAYCH_ADDR: u64 = 100;
    const TEST_PAYER_ADDR: u64 = 101;
    const TEST_CALLER_ADDR: u64 = 102;

    fn construct_runtime() -> MockRuntime {
        let paych_addr = Address::new_id(TEST_PAYCH_ADDR);
        let payer_addr = Address::new_id(TEST_PAYER_ADDR);
        let caller_addr = Address::new_id(TEST_CALLER_ADDR);
        let mut actor_code_cids = HashMap::default();
        actor_code_cids.insert(payer_addr, *ACCOUNT_ACTOR_CODE_ID);

        MockRuntime {
            receiver: paych_addr,
            caller: RefCell::new(caller_addr),
            caller_type: RefCell::new(*INIT_ACTOR_CODE_ID),
            actor_code_cids: RefCell::new(actor_code_cids),
            ..Default::default()
        }
    }

    #[test]
    fn create_paych_actor_test() {
        let caller_addr = Address::new_id(TEST_CALLER_ADDR);
        let rt = construct_runtime();
        rt.actor_code_cids.borrow_mut().insert(caller_addr, *ACCOUNT_ACTOR_CODE_ID);
        construct_and_verify(&rt, Address::new_id(TEST_PAYER_ADDR), caller_addr);
        check_state(&rt);
    }

    #[test]
    fn actor_doesnt_exist_test() {
        let rt = construct_runtime();
        rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
        rt.expect_validate_caller_type(vec![Type::Init]);
        let params = ConstructorParams {
            to: Address::new_id(TEST_PAYCH_ADDR),
            from: Address::new_secp256k1(&[2; fvm_shared::address::SECP_PUB_LEN]).unwrap(),
        };
        expect_abort(
            &rt,
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&params).unwrap(),
            ExitCode::USR_NOT_FOUND,
        );
    }

    #[test]
    fn create_paych_actor_after_resolving_to_id_address() {
        let payer_addr = Address::new_id(TEST_PAYER_ADDR);
        let payer_non_id = Address::new_bls(&[102; fvm_shared::address::BLS_PUB_LEN]).unwrap();
        let payee_addr = Address::new_id(103_u64);
        let payee_non_id = Address::new_bls(&[104; fvm_shared::address::BLS_PUB_LEN]).unwrap();

        let rt = construct_runtime();

        rt.actor_code_cids.borrow_mut().insert(payee_addr, *ACCOUNT_ACTOR_CODE_ID);

        rt.id_addresses.borrow_mut().insert(payer_non_id, payer_addr);
        rt.id_addresses.borrow_mut().insert(payee_non_id, payee_addr);

        construct_and_verify(&rt, payer_non_id, payee_non_id);
        check_state(&rt);
    }

    #[test]
    fn sendr_addr_not_resolvable_to_id_addr() {
        const TO_ADDR: u64 = 101;
        let to_addr = Address::new_id(TO_ADDR);
        let paych_addr = Address::new_id(TEST_PAYCH_ADDR);
        let caller_addr = Address::new_id(TEST_CALLER_ADDR);
        let non_id_addr = Address::new_bls(&[111; fvm_shared::address::BLS_PUB_LEN]).unwrap();

        let mut actor_code_cids = HashMap::default();
        actor_code_cids.insert(to_addr, *ACCOUNT_ACTOR_CODE_ID);

        let rt = MockRuntime {
            receiver: paych_addr,
            caller: RefCell::new(caller_addr),
            caller_type: RefCell::new(*INIT_ACTOR_CODE_ID),
            actor_code_cids: RefCell::new(actor_code_cids),
            ..Default::default()
        };

        rt.expect_send_simple(
            non_id_addr,
            METHOD_SEND,
            Default::default(),
            TokenAmount::zero(),
            Default::default(),
            ExitCode::OK,
        );

        rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
        rt.expect_validate_caller_type(vec![Type::Init]);
        let params = ConstructorParams { from: non_id_addr, to: to_addr };
        expect_abort(
            &rt,
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&params).unwrap(),
            ExitCode::USR_ILLEGAL_ARGUMENT,
        );
    }

    #[test]
    fn target_addr_not_resolvable_to_id_addr() {
        let from_addr = Address::new_id(5555_u64);
        let paych_addr = Address::new_id(TEST_PAYCH_ADDR);
        let caller_addr = Address::new_id(TEST_CALLER_ADDR);
        let non_id_addr = Address::new_bls(&[111; fvm_shared::address::BLS_PUB_LEN]).unwrap();

        let mut actor_code_cids = HashMap::default();
        actor_code_cids.insert(from_addr, *ACCOUNT_ACTOR_CODE_ID);

        let rt = MockRuntime {
            receiver: paych_addr,
            caller: RefCell::new(caller_addr),
            caller_type: RefCell::new(*INIT_ACTOR_CODE_ID),
            actor_code_cids: RefCell::new(actor_code_cids),
            ..Default::default()
        };

        rt.expect_send_simple(
            non_id_addr,
            METHOD_SEND,
            Default::default(),
            TokenAmount::zero(),
            Default::default(),
            ExitCode::OK,
        );

        rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
        rt.expect_validate_caller_type(vec![Type::Init]);
        let params = ConstructorParams { from: from_addr, to: non_id_addr };
        expect_abort(
            &rt,
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&params).unwrap(),
            ExitCode::USR_ILLEGAL_ARGUMENT,
        );
    }
}

mod create_lane_tests {
    use fvm_shared::crypto::signature::Signature;

    use super::*;

    const TEST_INIT_ACTOR_ADDR: u64 = 100;
    const PAYCH_ADDR: u64 = 101;
    const PAYER_ADDR: u64 = 102;
    const PAYEE_ADDR: u64 = 103;
    const PAYCH_BALANCE: u64 = 9;

    #[derive(Builder, Debug)]
    #[builder(name = "TestCaseBuilder")]
    struct TestCase {
        #[builder(default = "Address::new_id(PAYCH_ADDR)")]
        payment_channel: Address,
        desc: String,
        #[builder(default = "ACCOUNT_ACTOR_CODE_ID.clone()")]
        target_code: Cid,
        #[builder(default = "1")]
        epoch: ChainEpoch,
        #[builder(default = "1")]
        tl_min: ChainEpoch,
        #[builder(default = "0")]
        tl_max: ChainEpoch,
        #[builder(default)]
        lane: u64,
        #[builder(default)]
        nonce: u64,
        #[builder(default = "1")]
        amt: i64,
        #[builder(default)]
        secret_preimage: Vec<u8>,
        #[builder(default)]
        sig: Option<Signature>,
        #[builder(default = "true")]
        verify_sig: bool,
        #[builder(default = "ExitCode::USR_ILLEGAL_ARGUMENT")]
        exp_exit_code: ExitCode,
    }

    impl TestCase {
        pub fn builder() -> TestCaseBuilder {
            TestCaseBuilder::default()
        }
    }

    #[test]
    fn create_lane_test() {
        let init_actor_addr = Address::new_id(TEST_INIT_ACTOR_ADDR);
        let paych_addr = Address::new_id(PAYCH_ADDR);
        let payer_addr = Address::new_id(PAYER_ADDR);
        let payee_addr = Address::new_id(PAYEE_ADDR);
        let paych_balance = TokenAmount::from_atto(PAYCH_BALANCE);
        let paych_non_id = Address::new_bls(&[201; fvm_shared::address::BLS_PUB_LEN]).unwrap();
        let sig = Some(Signature::new_bls("doesn't matter".as_bytes().to_vec()));

        let test_cases: Vec<TestCase> = vec![
            TestCase::builder()
                .desc("succeeds".to_string())
                .sig(sig.clone())
                .exp_exit_code(ExitCode::OK)
                .build()
                .unwrap(),
            TestCase::builder()
                .desc(
                    "fails if channel address does not match address on the signed voucher"
                        .to_string(),
                )
                .payment_channel(Address::new_id(210))
                .sig(sig.clone())
                .build()
                .unwrap(),
            TestCase::builder()
                .desc(
                    "fails if address on the signed voucher cannot be resolved to ID address"
                        .to_string(),
                )
                .payment_channel(Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap())
                .sig(sig.clone())
                .build()
                .unwrap(),
            TestCase::builder()
                .desc(
                    "succeeds if address on the signed voucher can be resolved to channel ID address"
                        .to_string(),
                )
                .payment_channel(paych_non_id)
                .exp_exit_code(ExitCode::OK)
                .sig(sig.clone())
                .build()
                .unwrap(),
            TestCase::builder()
                .desc("fails if new send balance is negative".to_string())
                .amt(-1)
                .sig(sig.clone())
                .build()
                .unwrap(),
            TestCase::builder()
                .desc("fails if balance too low".to_string())
                .amt(10)
                .sig(sig.clone())
                .build()
                .unwrap(),
            TestCase::builder()
                .desc("fails is signature is not valid".to_string())
                .sig(None)
                .build()
                .unwrap(),
            TestCase::builder()
                .desc("fails if too early for a voucher".to_string())
                .tl_min(10)
                .sig(sig.clone())
                .build()
                .unwrap(),
            TestCase::builder()
                .desc("fails is beyond timelockmax".to_string())
                .epoch(10)
                .tl_max(5)
                .sig(sig.clone())
                .build()
                .unwrap(),
            TestCase::builder()
                .desc("fails if signature is not verified".to_string())
                .sig(sig)
                .verify_sig(false)
                .build()
                .unwrap(),
        ];

        for test_case in test_cases {
            println!("Test Description {}", test_case.desc);

            let mut actor_code_cids = HashMap::default();
            actor_code_cids.insert(payee_addr, *ACCOUNT_ACTOR_CODE_ID);
            actor_code_cids.insert(payer_addr, *ACCOUNT_ACTOR_CODE_ID);

            let rt = MockRuntime {
                receiver: paych_addr,
                caller: RefCell::new(init_actor_addr),
                caller_type: RefCell::new(*INIT_ACTOR_CODE_ID),
                actor_code_cids: RefCell::new(actor_code_cids),
                epoch: RefCell::new(test_case.epoch),
                balance: RefCell::new(paych_balance.clone()),
                ..Default::default()
            };

            rt.id_addresses.borrow_mut().insert(paych_non_id, paych_addr);

            construct_and_verify(&rt, payer_addr, payee_addr);

            let sv = SignedVoucher {
                time_lock_min: test_case.tl_min,
                time_lock_max: test_case.tl_max,
                secret_pre_image: test_case.secret_preimage.clone(),
                lane: test_case.lane,
                nonce: test_case.nonce,
                amount: TokenAmount::from_atto(test_case.amt),
                signature: test_case.sig.clone(),
                channel_addr: test_case.payment_channel,
                extra: Default::default(),
                min_settle_height: Default::default(),
                merges: Default::default(),
            };

            let ucp = UpdateChannelStateParams::from(sv.clone());
            rt.set_caller(test_case.target_code, payee_addr);
            rt.expect_validate_caller_addr(vec![payer_addr, payee_addr]);

            if test_case.sig.is_some() && test_case.secret_preimage.is_empty() {
                let exp_exit_code = if !test_case.verify_sig {
                    ExitCode::USR_ILLEGAL_ARGUMENT
                } else {
                    ExitCode::OK
                };
                expect_authenticate_message(&rt, payer_addr, sv.clone(), exp_exit_code);
            }

            if test_case.exp_exit_code.is_success() {
                call(
                    &rt,
                    Method::UpdateChannelState as u64,
                    IpldBlock::serialize_cbor(&ucp).unwrap(),
                );

                let st: PState = rt.get_state();
                let l_states = Amt::<LaneState, _>::load(&st.lane_states, &rt.store).unwrap();
                assert_eq!(l_states.count(), 1);

                let ls = l_states.get(sv.lane).unwrap().unwrap();
                assert_eq!(sv.amount, ls.redeemed);
                assert_eq!(sv.nonce, ls.nonce);
                check_state(&rt);
            } else {
                expect_abort(
                    &rt,
                    Method::UpdateChannelState as u64,
                    IpldBlock::serialize_cbor(&ucp).unwrap(),
                    test_case.exp_exit_code,
                );
                verify_initial_state(&rt, payer_addr, payee_addr);
            }
            rt.verify();
        }
    }
}

mod update_channel_state_redeem {
    use super::*;

    #[test]
    fn redeem_voucher_one_lane() {
        let (rt, mut sv) = require_create_channel_with_lanes(1);
        let state: PState = rt.get_state();
        let payee_addr = Address::new_id(PAYEE_ID);

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, payee_addr);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        sv.amount = TokenAmount::from_atto(9);

        let payer_addr = Address::new_id(PAYER_ID);

        expect_authenticate_message(&rt, payer_addr, sv.clone(), ExitCode::OK);

        call(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv)).unwrap(),
        );

        rt.verify();
        let exp_ls = LaneState { redeemed: TokenAmount::from_atto(9), nonce: 2 };
        let exp_state = PState {
            from: state.from,
            to: state.to,
            to_send: TokenAmount::from_atto(9),
            settling_at: state.settling_at,
            min_settle_height: state.min_settle_height,
            lane_states: construct_lane_state_amt(&rt, vec![exp_ls]),
        };
        verify_state(&rt, Some(1), exp_state);
    }

    #[test]
    fn redeem_voucher_correct_lane() {
        let (rt, mut sv) = require_create_channel_with_lanes(3);
        let state: PState = rt.get_state();
        let payee_addr = Address::new_id(PAYEE_ID);

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, payee_addr);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        let initial_amount = state.to_send;
        sv.amount = TokenAmount::from_atto(9);
        sv.lane = 1;

        let ls_to_update: LaneState = get_lane_state(&rt, &state.lane_states, sv.lane);
        sv.nonce = ls_to_update.nonce + 1;
        let payer_addr = Address::new_id(PAYER_ID);

        expect_authenticate_message(&rt, payer_addr, sv.clone(), ExitCode::OK);

        call(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv.clone())).unwrap(),
        );

        rt.verify();

        let state: PState = rt.get_state();
        let ls_updated: LaneState = get_lane_state(&rt, &state.lane_states, sv.lane);
        let big_delta = &sv.amount - &ls_to_update.redeemed;

        let exp_send = big_delta + &initial_amount;
        assert_eq!(exp_send, state.to_send);
        assert_eq!(sv.amount, ls_updated.redeemed);
        assert_eq!(sv.nonce, ls_updated.nonce);
        check_state(&rt);
    }

    #[test]
    fn redeem_voucher_nonce_reuse() {
        let (rt, mut sv) = require_create_channel_with_lanes(3);
        let state: PState = rt.get_state();
        let payee_addr = Address::new_id(PAYEE_ID);

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, payee_addr);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        sv.amount = TokenAmount::from_atto(9);
        sv.nonce = 1;

        let payer_addr = Address::new_id(PAYER_ID);

        expect_authenticate_message(&rt, payer_addr, sv.clone(), ExitCode::OK);

        expect_abort(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv)).unwrap(),
            ExitCode::USR_ILLEGAL_ARGUMENT,
        );

        rt.verify();
        check_state(&rt);
    }
}

mod merge_tests {
    use super::*;

    fn construct_runtime(num_lanes: u64) -> (MockRuntime, SignedVoucher, PState) {
        let (rt, sv) = require_create_channel_with_lanes(num_lanes);
        let state: PState = rt.get_state();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);
        (rt, sv, state)
    }

    fn failure_end(rt: &MockRuntime, sv: SignedVoucher, exp_exit_code: ExitCode) {
        let payee_addr = Address::new_id(PAYEE_ID);
        expect_authenticate_message(rt, payee_addr, sv.clone(), ExitCode::OK);

        expect_abort(
            rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv)).unwrap(),
            exp_exit_code,
        );
        rt.verify();
    }

    #[test]
    fn merge_success() {
        let num_lanes = 3;
        let (rt, mut sv, mut state) = construct_runtime(num_lanes);

        let merge_to: LaneState = get_lane_state(&rt, &state.lane_states, 0);
        let merge_from: LaneState = get_lane_state(&rt, &state.lane_states, 1);

        sv.lane = 0;
        let merge_nonce = merge_to.nonce + 10;

        sv.merges = vec![Merge { lane: 1, nonce: merge_nonce }];
        let payee_addr = Address::new_id(PAYEE_ID);
        expect_authenticate_message(&rt, payee_addr, sv.clone(), ExitCode::OK);

        call(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv.clone())).unwrap(),
        );
        rt.verify();
        let exp_merge_to = LaneState { redeemed: sv.amount.clone(), nonce: sv.nonce };
        let exp_merge_from =
            LaneState { redeemed: merge_from.redeemed.clone(), nonce: merge_nonce };
        let redeemed = &merge_from.redeemed + &merge_to.redeemed;
        let exp_delta = &sv.amount - &redeemed;
        state.to_send = exp_delta + &state.to_send;

        state.lane_states = construct_lane_state_amt(
            &rt,
            vec![exp_merge_to, exp_merge_from, get_lane_state(&rt, &state.lane_states, 2)],
        );

        verify_state(&rt, Some(num_lanes), state);
    }

    #[test]
    fn merge_failure() {
        struct TestCase {
            lane: u64,
            voucher: u64,
            balance: i32,
            merge: u64,
            exit: ExitCode,
        }
        let test_cases = vec![
            TestCase {
                lane: 1,
                voucher: 10,
                balance: 0,
                merge: 1,
                exit: ExitCode::USR_ILLEGAL_ARGUMENT,
            },
            TestCase {
                lane: 1,
                voucher: 0,
                balance: 0,
                merge: 10,
                exit: ExitCode::USR_ILLEGAL_ARGUMENT,
            },
            TestCase {
                lane: 1,
                voucher: 10,
                balance: 1,
                merge: 10,
                exit: ExitCode::USR_ILLEGAL_ARGUMENT,
            },
            TestCase {
                lane: 0,
                voucher: 10,
                balance: 0,
                merge: 10,
                exit: ExitCode::USR_ILLEGAL_ARGUMENT,
            },
        ];

        for tc in test_cases {
            let num_lanes = 2;
            let (rt, mut sv, state) = construct_runtime(num_lanes);

            rt.set_balance(TokenAmount::from_atto(tc.balance));

            sv.lane = 0;
            sv.nonce = tc.voucher;
            sv.merges = vec![Merge { lane: tc.lane, nonce: tc.merge }];
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
            failure_end(&rt, sv, tc.exit);
        }
    }

    #[test]
    fn invalid_merge_lane_999() {
        let num_lanes = 2;
        let (rt, mut sv) = require_create_channel_with_lanes(num_lanes);
        let state: PState = rt.get_state();

        sv.lane = 0;
        sv.nonce = 10;
        sv.merges = vec![Merge { lane: 999, nonce: sv.nonce }];
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);
        failure_end(&rt, sv, ExitCode::USR_ILLEGAL_ARGUMENT);
    }

    #[test]
    fn lane_limit_exceeded() {
        let (rt, mut sv, _) = construct_runtime(1);

        sv.lane = MAX_LANE + 1;
        sv.nonce += 1;
        sv.amount = TokenAmount::from_atto(100);
        failure_end(&rt, sv, ExitCode::USR_ILLEGAL_ARGUMENT);
    }
}

mod update_channel_state_extra {
    use super::*;
    use fvm_ipld_encoding::CBOR;

    const OTHER_ADDR: u64 = 104;

    fn construct_runtime(exit_code: ExitCode) -> (MockRuntime, SignedVoucher) {
        let (rt, mut sv) = require_create_channel_with_lanes(1);
        let state: PState = rt.get_state();
        let other_addr = Address::new_id(OTHER_ADDR);
        let fake_params = RawBytes::new(vec![1, 2, 3, 4]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        sv.extra = Some(ModVerifyParams {
            actor: other_addr,
            method: Method::UpdateChannelState as u64,
            data: fake_params.clone(),
        });
        expect_authenticate_message(&rt, state.to, sv.clone(), ExitCode::OK);

        rt.expect_send_simple(
            other_addr,
            Method::UpdateChannelState as u64,
            Some(IpldBlock { codec: CBOR, data: fake_params.to_vec() }),
            TokenAmount::zero(),
            None,
            exit_code,
        );
        (rt, sv)
    }

    #[test]
    fn extra_call_succeed() {
        let (rt, sv) = construct_runtime(ExitCode::OK);
        call(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv)).unwrap(),
        );
        rt.verify();
        check_state(&rt);
    }

    #[test]
    fn extra_call_fail() {
        let (rt, sv) = construct_runtime(ExitCode::USR_UNSPECIFIED);
        expect_abort(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv)).unwrap(),
            ExitCode::USR_UNSPECIFIED,
        );
        rt.verify();
        check_state(&rt);
    }
}

#[test]
fn update_channel_settling() {
    let (rt, sv) = require_create_channel_with_lanes(1);
    rt.epoch.replace(10);
    let state: PState = rt.get_state();
    rt.expect_validate_caller_addr(vec![state.from, state.to]);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
    call(&rt, Method::Settle as u64, None);

    let exp_settling_at = SETTLE_DELAY + 10;
    let state: PState = rt.get_state();
    assert_eq!(exp_settling_at, state.settling_at);
    assert_eq!(state.min_settle_height, 0);

    struct TestCase {
        min_settle: i64,
        exp_min_settle_height: i64,
        exp_settling_at: i64,
    }
    let test_cases = vec![
        TestCase {
            min_settle: 0,
            exp_min_settle_height: state.min_settle_height,
            exp_settling_at: state.settling_at,
        },
        TestCase { min_settle: 2, exp_min_settle_height: 2, exp_settling_at: state.settling_at },
        TestCase { min_settle: 12, exp_min_settle_height: 12, exp_settling_at: state.settling_at },
        TestCase {
            min_settle: state.settling_at + 1,
            exp_min_settle_height: state.settling_at + 1,
            exp_settling_at: state.settling_at + 1,
        },
    ];

    let mut ucp = UpdateChannelStateParams::from(sv);
    for tc in test_cases {
        ucp.sv.min_settle_height = tc.min_settle;
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        expect_authenticate_message(&rt, state.to, ucp.sv.clone(), ExitCode::OK);

        call(&rt, Method::UpdateChannelState as u64, IpldBlock::serialize_cbor(&ucp).unwrap());
        let new_state: PState = rt.get_state();
        assert_eq!(tc.exp_settling_at, new_state.settling_at);
        assert_eq!(tc.exp_min_settle_height, new_state.min_settle_height);
        ucp.sv.nonce += 1;
        check_state(&rt);
    }
}

mod secret_preimage {
    use super::*;

    #[test]
    fn succeed_correct_secret() {
        let (rt, sv) = require_create_channel_with_lanes(1);
        let state: PState = rt.get_state();
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        let ucp = UpdateChannelStateParams::from(sv.clone());

        expect_authenticate_message(&rt, state.to, sv, ExitCode::OK);

        call(&rt, Method::UpdateChannelState as u64, IpldBlock::serialize_cbor(&ucp).unwrap());

        rt.verify();
        check_state(&rt);
    }

    #[test]
    fn incorrect_secret() {
        let (rt, sv) = require_create_channel_with_lanes(1);

        let state: PState = rt.get_state();

        let mut ucp = UpdateChannelStateParams { secret: b"Profesr".to_vec(), sv };
        let mut mag = b"Magneto".to_vec();
        mag.append(&mut vec![0; 25]);
        ucp.sv.secret_pre_image = mag;

        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        expect_authenticate_message(&rt, state.to, ucp.sv.clone(), ExitCode::OK);

        expect_abort(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&ucp).unwrap(),
            ExitCode::USR_ILLEGAL_ARGUMENT,
        );

        rt.verify();
        check_state(&rt);
    }
}

mod actor_settle {
    use super::*;

    const EP: i64 = 10;

    #[test]
    fn adjust_settling_at() {
        let (rt, _sv) = require_create_channel_with_lanes(1);
        rt.epoch.replace(EP);
        let mut state: PState = rt.get_state();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        call(&rt, Method::Settle as u64, None);

        let exp_settling_at = EP + SETTLE_DELAY;
        state = rt.get_state();
        assert_eq!(state.settling_at, exp_settling_at);
        assert_eq!(state.min_settle_height, 0);
        check_state(&rt);
    }

    #[test]
    fn call_twice() {
        let (rt, _sv) = require_create_channel_with_lanes(1);
        rt.epoch.replace(EP);
        let state: PState = rt.get_state();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);
        call(&rt, Method::Settle as u64, None);

        rt.expect_validate_caller_addr(vec![state.from, state.to]);
        expect_abort(&rt, Method::Settle as u64, None, ExitCode::USR_ILLEGAL_STATE);
    }

    #[test]
    fn settle_if_height_less() {
        let (rt, mut sv) = require_create_channel_with_lanes(1);
        rt.epoch.replace(EP);
        let mut state: PState = rt.get_state();

        sv.min_settle_height = (EP + SETTLE_DELAY) + 1;
        let ucp = UpdateChannelStateParams::from(sv.clone());

        rt.expect_validate_caller_addr(vec![state.from, state.to]);
        expect_authenticate_message(&rt, state.to, sv, ExitCode::OK);

        call(&rt, Method::UpdateChannelState as u64, IpldBlock::serialize_cbor(&ucp).unwrap());

        state = rt.get_state();
        assert_eq!(state.settling_at, 0);
        assert_eq!(state.min_settle_height, ucp.sv.min_settle_height);

        // Settle.
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);
        call(&rt, Method::Settle as u64, None);

        state = rt.get_state();
        assert_eq!(state.settling_at, ucp.sv.min_settle_height);
        check_state(&rt);
    }

    #[test]
    fn voucher_invalid_after_settling() {
        const ERR_CHANNEL_STATE_UPDATE_AFTER_SETTLED: ExitCode = ExitCode::new(32);

        let (rt, sv) = require_create_channel_with_lanes(1);
        rt.epoch.replace(EP);
        let mut state: PState = rt.get_state();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);

        call(&rt, Method::Settle as u64, None);

        state = rt.get_state();
        rt.epoch.replace(state.settling_at + 40);
        rt.expect_validate_caller_addr(vec![state.from, state.to]);
        expect_abort(
            &rt,
            Method::UpdateChannelState as u64,
            IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv)).unwrap(),
            ERR_CHANNEL_STATE_UPDATE_AFTER_SETTLED,
        );
    }
}

mod actor_collect {
    use fvm_shared::METHOD_SEND;

    use super::*;

    #[test]
    fn happy_path() {
        let (rt, _sv) = require_create_channel_with_lanes(1);
        let curr_epoch: ChainEpoch = 10;
        rt.epoch.replace(curr_epoch);
        let st: PState = rt.get_state();

        // Settle.
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, st.from);
        rt.expect_validate_caller_addr(vec![st.from, st.to]);
        call(&rt, Method::Settle as u64, None);

        let st: PState = rt.get_state();
        assert_eq!(st.settling_at, SETTLE_DELAY + curr_epoch);
        rt.expect_validate_caller_addr(vec![st.from, st.to]);

        // wait for settlingat epoch
        rt.epoch.replace(st.settling_at + 1);

        rt.expect_send_simple(
            st.to,
            METHOD_SEND,
            Default::default(),
            st.to_send.clone(),
            Default::default(),
            ExitCode::OK,
        );

        // Collect.
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, st.to);
        rt.expect_validate_caller_addr(vec![st.from, st.to]);
        rt.expect_delete_actor(st.from);
        let res = call(&rt, Method::Collect as u64, None);
        assert!(res.is_none());
        check_state(&rt);
    }

    #[test]
    fn actor_collect() {
        struct TestCase {
            dont_settle: bool,
            exp_send_to: ExitCode,
            exp_collect_exit: ExitCode,
        }

        let test_cases = vec![
            // fails if not settling with: payment channel not settling or settled
            TestCase {
                dont_settle: true,
                exp_send_to: ExitCode::OK,
                exp_collect_exit: ExitCode::USR_FORBIDDEN,
            },
            // fails if Failed to send funds to `To`
            TestCase {
                dont_settle: false,
                exp_send_to: ExitCode::USR_UNSPECIFIED,
                exp_collect_exit: ExitCode::USR_UNSPECIFIED,
            },
        ];

        for tc in test_cases {
            let (rt, _sv) = require_create_channel_with_lanes(1);
            rt.epoch.replace(10);
            let mut state: PState = rt.get_state();

            if !tc.dont_settle {
                rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
                rt.expect_validate_caller_addr(vec![state.from, state.to]);
                call(&rt, Method::Settle as u64, None);
                state = rt.get_state();
                assert_eq!(state.settling_at, SETTLE_DELAY + *rt.epoch.borrow());
            }

            // "wait" for SettlingAt epoch
            rt.epoch.replace(state.settling_at + 1);

            if !tc.dont_settle {
                rt.expect_send_simple(
                    state.to,
                    METHOD_SEND,
                    Default::default(),
                    state.to_send.clone(),
                    Default::default(),
                    tc.exp_send_to,
                );
            }

            // Collect.
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, state.from);
            rt.expect_validate_caller_addr(vec![state.from, state.to]);
            expect_abort(&rt, Method::Collect as u64, None, tc.exp_collect_exit);
            check_state(&rt);
        }
    }
}

fn require_create_channel_with_lanes(num_lanes: u64) -> (MockRuntime, SignedVoucher) {
    let paych_addr = Address::new_id(100);
    let payer_addr = Address::new_id(PAYER_ID);
    let payee_addr = Address::new_id(PAYEE_ID);
    let balance = TokenAmount::from_atto(100_000);
    let received = TokenAmount::zero();
    let curr_epoch = 2;

    let mut actor_code_cids = HashMap::default();
    actor_code_cids.insert(payee_addr, *ACCOUNT_ACTOR_CODE_ID);
    actor_code_cids.insert(payer_addr, *ACCOUNT_ACTOR_CODE_ID);

    let rt = MockRuntime {
        receiver: paych_addr,
        caller: RefCell::new(INIT_ACTOR_ADDR),
        caller_type: RefCell::new(*INIT_ACTOR_CODE_ID),
        actor_code_cids: RefCell::new(actor_code_cids),
        value_received: RefCell::new(received),
        balance: RefCell::new(balance),
        epoch: RefCell::new(curr_epoch),
        ..Default::default()
    };

    construct_and_verify(&rt, payer_addr, payee_addr);

    let mut last_sv = None;
    for i in 0..num_lanes {
        let lane_param = LaneParams {
            epoch_num: curr_epoch,
            from: payer_addr,
            to: payee_addr,
            amt: (TokenAmount::from_atto(i + 1)),
            lane: i as u64,
            nonce: i + 1,
        };

        last_sv = Some(require_add_new_lane(&rt, lane_param));
    }

    (rt, last_sv.unwrap())
}

fn require_add_new_lane(rt: &MockRuntime, param: LaneParams) -> SignedVoucher {
    let payee_addr = Address::new_id(103_u64);
    let sig = Signature::new_bls(vec![0, 1, 2, 3, 4, 5, 6, 7]);
    let mut sv = SignedVoucher {
        time_lock_min: param.epoch_num,
        time_lock_max: i64::MAX,
        lane: param.lane,
        nonce: param.nonce,
        amount: param.amt.clone(),
        signature: Some(sig),
        secret_pre_image: Default::default(),
        channel_addr: Address::new_id(PAYCH_ID),
        extra: Default::default(),
        min_settle_height: Default::default(),
        merges: Default::default(),
    };
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, param.from);
    rt.expect_validate_caller_addr(vec![param.from, param.to]);

    expect_authenticate_message(rt, payee_addr, sv.clone(), ExitCode::OK);

    call(
        rt,
        Method::UpdateChannelState as u64,
        IpldBlock::serialize_cbor(&UpdateChannelStateParams::from(sv.clone())).unwrap(),
    );
    rt.verify();
    sv.nonce += 1;
    sv
}

fn construct_and_verify(rt: &MockRuntime, sender: Address, receiver: Address) {
    let params = ConstructorParams { from: sender, to: receiver };
    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    rt.expect_validate_caller_type(vec![Type::Init]);
    call(rt, METHOD_CONSTRUCTOR, IpldBlock::serialize_cbor(&params).unwrap());
    rt.verify();
    let sender_id = *rt.id_addresses.borrow().get(&sender).unwrap_or(&sender);
    let receiver_id = *rt.id_addresses.borrow().get(&receiver).unwrap_or(&receiver);
    verify_initial_state(rt, sender_id, receiver_id);
}

fn verify_initial_state(rt: &MockRuntime, sender: Address, receiver: Address) {
    let _state: PState = rt.get_state();
    let empt_arr_cid = Amt::<(), _>::new(&rt.store).flush().unwrap();
    let expected_state = PState::new(sender, receiver, empt_arr_cid);
    verify_state(rt, None, expected_state)
}

fn verify_state(rt: &MockRuntime, exp_lanes: Option<u64>, expected_state: PState) {
    let state: PState = rt.get_state();

    assert_eq!(expected_state.to, state.to);
    assert_eq!(expected_state.from, state.from);
    assert_eq!(expected_state.min_settle_height, state.min_settle_height);
    assert_eq!(expected_state.settling_at, state.settling_at);
    assert_eq!(expected_state.to_send, state.to_send);
    if let Some(exp_lanes) = exp_lanes {
        assert_lane_states_length(rt, &state.lane_states, exp_lanes);
        assert_eq!(expected_state.lane_states, state.lane_states);
    } else {
        assert_lane_states_length(rt, &state.lane_states, 0);
    }
    check_state(rt);
}

fn assert_lane_states_length(rt: &MockRuntime, cid: &Cid, l: u64) {
    let arr = Amt::<LaneState, _>::load(cid, &rt.store).unwrap();
    assert_eq!(arr.count(), l);
}

fn expect_authenticate_message(
    rt: &MockRuntime,
    payer_addr: Address,
    sv: SignedVoucher,
    exp_exit_code: ExitCode,
) {
    rt.expect_send(
        payer_addr,
        AUTHENTICATE_MESSAGE_METHOD,
        IpldBlock::serialize_cbor(&AuthenticateMessageParams {
            signature: sv.clone().signature.unwrap().bytes,
            message: sv.signing_bytes().unwrap(),
        })
        .unwrap(),
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        exp_exit_code.is_success().then(|| AUTHENTICATE_MESSAGE_RESPONSE.clone()).flatten(),
        exp_exit_code,
        None,
    )
}

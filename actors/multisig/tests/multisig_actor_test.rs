use fil_actor_multisig::{
    compute_proposal_hash, Actor as MultisigActor, ConstructorParams, Method, ProposeReturn, State,
    Transaction, TxnID, TxnIDParams, SIGNERS_MAX,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::{Address, BLS_PUB_LEN};
use fvm_shared::bigint::bigint_ser;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod util;

fn construct_runtime(receiver: Address) -> MockRuntime {
    MockRuntime {
        receiver,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

// Constructor
#[cfg(test)]
mod constructor_tests {
    use super::*;

    const MSIG: Address = Address::new_id(1000);
    const ANNE: Address = Address::new_id(101);
    const BOB: Address = Address::new_id(102);
    const CHARLIE: Address = Address::new_id(103);

    #[test]
    fn test_simple_construction() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();
        let params = ConstructorParams {
            signers: vec![ANNE, BOB, CHARLIE],
            num_approvals_threshold: 2,
            unlock_duration: 200,
            start_epoch: 100,
        };

        rt.set_received(TokenAmount::from(100u8));
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        let ret = rt.call::<MultisigActor>(
            Method::Constructor as u64,
            &RawBytes::serialize(&params).unwrap(),
        );
        assert_eq!(RawBytes::default(), ret.unwrap());
        rt.verify();

        let st: State = rt.get_state();
        assert_eq!(params.signers, st.signers);
        assert_eq!(params.num_approvals_threshold, st.num_approvals_threshold);
        assert_eq!(TokenAmount::from(100u8), st.initial_balance);
        assert_eq!(200, st.unlock_duration);
        assert_eq!(100, st.start_epoch);
        h.assert_transactions(&rt, vec![]);
    }

    #[test]
    fn test_construction_by_resolving_signers_to_id_addresses() {
        let anne_non_id = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
        let bob_non_id = Address::new_bls(&[2u8; BLS_PUB_LEN]).unwrap();
        let charlie_non_id = Address::new_bls(&[3u8; BLS_PUB_LEN]).unwrap();

        let mut rt = construct_runtime(MSIG);
        rt.id_addresses.insert(anne_non_id, ANNE);
        rt.id_addresses.insert(bob_non_id, BOB);
        rt.id_addresses.insert(charlie_non_id, CHARLIE);
        let params = ConstructorParams {
            signers: vec![anne_non_id, bob_non_id, charlie_non_id],
            num_approvals_threshold: 2,
            unlock_duration: 0,
            start_epoch: 0,
        };

        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        let ret = rt
            .call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&params).unwrap(),
            )
            .unwrap();
        assert_eq!(ret, RawBytes::default());
    }

    #[test]
    fn test_construction_with_vesting() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();
        rt.set_epoch(1234);
        let params = ConstructorParams {
            signers: vec![ANNE, BOB, CHARLIE],
            num_approvals_threshold: 3,
            unlock_duration: 100,
            start_epoch: 1234,
        };
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        assert_eq!(
            RawBytes::default(),
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&params).unwrap()
            )
            .unwrap()
        );

        let st: State = rt.get_state();
        assert_eq!(params.signers, st.signers);
        assert_eq!(params.num_approvals_threshold, st.num_approvals_threshold);
        assert_eq!(TokenAmount::zero(), st.initial_balance);
        assert_eq!(100, st.unlock_duration);
        assert_eq!(1234, st.start_epoch);
        h.assert_transactions(&rt, vec![]);
    }

    #[test]
    fn test_construction_fail_to_construct_multisig_actor_with_0_signers() {
        let mut rt = construct_runtime(MSIG);
        let zero_signer_params = ConstructorParams {
            signers: Vec::new(),
            num_approvals_threshold: 1,
            unlock_duration: 1,
            start_epoch: 0,
        };
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);

        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&zero_signer_params).unwrap(),
            ),
        );
        rt.verify();
    }

    #[test]
    fn test_construction_fail_to_construct_multisig_with_more_than_max_signers() {
        let mut rt = construct_runtime(MSIG);
        let mut signers = Vec::new();
        let mut i: u64 = 0;
        while i <= SIGNERS_MAX as u64 {
            signers.push(Address::new_id(i + 1000));
            i += 1;
        }
        let over_max_signers_params = ConstructorParams {
            signers,
            num_approvals_threshold: 1,
            unlock_duration: 1,
            start_epoch: 0,
        };
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&over_max_signers_params).unwrap(),
            ),
        );
        rt.verify();
    }

    #[test]
    fn fail_to_construct_multisig_with_more_approvals_than_signers() {
        let mut rt = construct_runtime(MSIG);
        let params = ConstructorParams {
            signers: vec![ANNE],
            num_approvals_threshold: 2,
            unlock_duration: 0,
            start_epoch: 0,
        };
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&params).unwrap(),
            ),
        );
        rt.verify();
    }

    #[test]
    fn fail_to_contruct_multisig_if_a_signer_is_not_resolvable_to_id_address() {
        let mut rt = construct_runtime(MSIG);
        let anne_non_id = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
        // no mapping to ANNE added to runtime
        let params = ConstructorParams {
            signers: vec![anne_non_id, BOB, CHARLIE],
            num_approvals_threshold: 2,
            unlock_duration: 1,
            start_epoch: 0,
        };
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.expect_send(
            anne_non_id,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        expect_abort(
            ExitCode::USR_ILLEGAL_STATE,
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&params).unwrap(),
            ),
        );
        rt.verify();
    }

    #[test]
    fn fail_to_construct_msig_with_duplicate_signers_all_id() {
        let mut rt = construct_runtime(MSIG);
        let params = ConstructorParams {
            signers: vec![ANNE, BOB, BOB],
            num_approvals_threshold: 2,
            unlock_duration: 0,
            start_epoch: 0,
        };
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&params).unwrap(),
            ),
        );
        rt.verify();
    }

    #[test]
    fn fail_to_construct_msig_with_duplicate_signers_id_and_non_id() {
        let bob_non_id = Address::new_bls(&[2u8; BLS_PUB_LEN]).unwrap();
        let mut rt = construct_runtime(MSIG);
        rt.id_addresses.insert(bob_non_id, BOB);
        let params = ConstructorParams {
            signers: vec![ANNE, bob_non_id, BOB],
            num_approvals_threshold: 2,
            unlock_duration: 0,
            start_epoch: 0,
        };
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&params).unwrap(),
            ),
        );
        rt.verify();
    }
}

#[cfg(test)]
mod vesting_tests {
    use super::*;

    const MSIG: Address = Address::new_id(1000);
    const ANNE: Address = Address::new_id(101);
    const BOB: Address = Address::new_id(102);
    const CHARLIE: Address = Address::new_id(103);
    const DARLENE: Address = Address::new_id(104);

    const UNLOCK_DURATION: ChainEpoch = 10;
    const START_EPOCH: ChainEpoch = 0;
    const MSIG_INITIAL_BALANCE: u8 = 100;

    #[test]
    fn happy_path_full_vesting() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();

        rt.set_balance(TokenAmount::from(MSIG_INITIAL_BALANCE));
        rt.set_received(TokenAmount::from(MSIG_INITIAL_BALANCE));
        h.construct_and_verify(&mut rt, 2, UNLOCK_DURATION, START_EPOCH, vec![ANNE, BOB, CHARLIE]);
        rt.set_received(TokenAmount::zero());

        // anne proposes that darlene receive inital balance
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ANNE);
        let proposal_hash = h.propose_ok(
            &mut rt,
            DARLENE,
            TokenAmount::from(MSIG_INITIAL_BALANCE),
            METHOD_SEND,
            RawBytes::default(),
        );

        // bob approves anne's tx too soon
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, BOB);
        expect_abort(ExitCode::USR_INSUFFICIENT_FUNDS, h.approve(&mut rt, TxnID(0), proposal_hash));
        rt.reset();

        // advance the epoch s.t. all funds are unlocked
        rt.set_epoch(START_EPOCH + UNLOCK_DURATION);
        rt.expect_send(
            DARLENE,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::from(MSIG_INITIAL_BALANCE),
            RawBytes::default(),
            ExitCode::OK,
        );
        assert_eq!(RawBytes::default(), h.approve_ok(&mut rt, TxnID(0), proposal_hash))

        // h.check_state()
    }

    #[test]
    fn partial_vesting_propose_to_send_half_the_actor_balance_when_the_epoch_is_half_the_unlock_duration(
    ) {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();

        rt.set_balance(TokenAmount::from(MSIG_INITIAL_BALANCE));
        rt.set_received(TokenAmount::from(MSIG_INITIAL_BALANCE));
        h.construct_and_verify(&mut rt, 2, UNLOCK_DURATION, START_EPOCH, vec![ANNE, BOB, CHARLIE]);
        rt.set_received(TokenAmount::zero());

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ANNE);
        let proposal_hash = h.propose_ok(
            &mut rt,
            DARLENE,
            TokenAmount::from(MSIG_INITIAL_BALANCE / 2),
            METHOD_SEND,
            RawBytes::default(),
        );
        rt.set_epoch(START_EPOCH + UNLOCK_DURATION / 2);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, BOB);
        rt.expect_send(
            DARLENE,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::from(MSIG_INITIAL_BALANCE / 2),
            RawBytes::default(),
            ExitCode::OK,
        );
        h.approve_ok(&mut rt, TxnID(0), proposal_hash);

        // h.check_state()
    }

    #[test]
    fn propose_and_autoapprove_tx_above_locked_amount_fails() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();

        rt.set_balance(TokenAmount::from(MSIG_INITIAL_BALANCE));
        rt.set_received(TokenAmount::from(MSIG_INITIAL_BALANCE));
        h.construct_and_verify(&mut rt, 1, UNLOCK_DURATION, START_EPOCH, vec![ANNE, BOB, CHARLIE]);
        rt.set_received(TokenAmount::zero());

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ANNE);
        expect_abort(
            ExitCode::USR_INSUFFICIENT_FUNDS,
            h.propose(
                &mut rt,
                DARLENE,
                TokenAmount::from(MSIG_INITIAL_BALANCE),
                METHOD_SEND,
                RawBytes::default(),
            ),
        );
        rt.reset();
        rt.set_epoch(START_EPOCH + UNLOCK_DURATION / 10);
        let amount_out = TokenAmount::from(MSIG_INITIAL_BALANCE / 10);
        rt.expect_send(
            DARLENE,
            METHOD_SEND,
            RawBytes::default(),
            amount_out.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        h.propose_ok(&mut rt, DARLENE, amount_out, METHOD_SEND, RawBytes::default());

        // h.check_state()
    }

    #[test]
    fn fail_to_vest_more_than_locked_amount() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();

        rt.set_balance(TokenAmount::from(MSIG_INITIAL_BALANCE));
        rt.set_received(TokenAmount::from(MSIG_INITIAL_BALANCE));
        h.construct_and_verify(&mut rt, 2, UNLOCK_DURATION, START_EPOCH, vec![ANNE, BOB, CHARLIE]);
        rt.set_received(TokenAmount::zero());

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ANNE);
        let proposal_hash = h.propose_ok(
            &mut rt,
            DARLENE,
            TokenAmount::from(MSIG_INITIAL_BALANCE / 2),
            METHOD_SEND,
            RawBytes::default(),
        );
        rt.set_epoch(START_EPOCH + UNLOCK_DURATION / 10);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, BOB);
        expect_abort(ExitCode::USR_INSUFFICIENT_FUNDS, h.approve(&mut rt, TxnID(0), proposal_hash));
    }

    #[test]
    fn avoid_truncating_division() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();

        let locked_balance = TokenAmount::from(UNLOCK_DURATION - 1); // balance < duration
        let one = TokenAmount::from(1u8);
        rt.set_balance(locked_balance.clone());
        rt.set_received(locked_balance.clone());
        h.construct_and_verify(&mut rt, 1, UNLOCK_DURATION, START_EPOCH, vec![ANNE, BOB, CHARLIE]);
        rt.set_received(TokenAmount::zero());

        // expect nothing vested yet
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ANNE);
        expect_abort(
            ExitCode::USR_INSUFFICIENT_FUNDS,
            h.propose(&mut rt, ANNE, one.clone(), METHOD_SEND, RawBytes::default()),
        );
        rt.reset();

        // expect nothing ( (x-1/x) <1 unit) vested after 1 epoch
        rt.set_epoch(START_EPOCH + 1);
        expect_abort(
            ExitCode::USR_INSUFFICIENT_FUNDS,
            h.propose(&mut rt, ANNE, one.clone(), METHOD_SEND, RawBytes::default()),
        );
        rt.reset();

        // expect 1 unit available after 2 epochs
        rt.set_epoch(START_EPOCH + 2);
        rt.expect_send(
            ANNE,
            METHOD_SEND,
            RawBytes::default(),
            one.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        h.propose_ok(&mut rt, ANNE, one.clone(), METHOD_SEND, RawBytes::default());
        rt.set_balance(locked_balance.clone());

        // do not expect full vesting before full duration elapsed
        rt.set_epoch(START_EPOCH + UNLOCK_DURATION - 1);
        expect_abort(
            ExitCode::USR_INSUFFICIENT_FUNDS,
            h.propose(&mut rt, ANNE, locked_balance.clone(), METHOD_SEND, RawBytes::default()),
        );
        rt.reset();

        // expect all but one unit available after all but one epochs
        rt.expect_send(
            ANNE,
            METHOD_SEND,
            RawBytes::default(),
            locked_balance.clone() - one.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        h.propose_ok(&mut rt, ANNE, locked_balance.clone() - one, METHOD_SEND, RawBytes::default());
        rt.set_balance(locked_balance.clone());

        // expect everything after exactly lock duration
        rt.set_epoch(START_EPOCH + UNLOCK_DURATION);
        rt.expect_send(
            ANNE,
            METHOD_SEND,
            RawBytes::default(),
            locked_balance.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        h.propose_ok(&mut rt, ANNE, locked_balance, METHOD_SEND, RawBytes::default());
    }

    #[test]
    fn sending_zero_ok_when_nothing_vests() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();

        rt.set_balance(TokenAmount::from(MSIG_INITIAL_BALANCE));
        rt.set_received(TokenAmount::from(MSIG_INITIAL_BALANCE));
        h.construct_and_verify(&mut rt, 2, UNLOCK_DURATION, START_EPOCH, vec![ANNE, BOB, CHARLIE]);
        rt.set_received(TokenAmount::zero());

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ANNE);
        rt.expect_send(
            BOB,
            METHOD_SEND,
            RawBytes::default(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );
    }

    #[test]
    fn sending_zero_when_lockup_exceeds_balance() {
        let mut rt = construct_runtime(MSIG);
        let h = util::ActorHarness::new();

        h.construct_and_verify(&mut rt, 1, 0, START_EPOCH, vec![ANNE]);
        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, MSIG);
        rt.set_balance(TokenAmount::from(10u8));
        rt.set_received(TokenAmount::from(10u8));

        // lock up funds the actor doesn't have yet
        h.lock_balance(&mut rt, START_EPOCH, UNLOCK_DURATION, TokenAmount::from(10u8)).unwrap();

        // make a tx that transfers no value
        let send_amount = TokenAmount::zero();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, ANNE);
        rt.expect_send(
            BOB,
            METHOD_SEND,
            RawBytes::default(),
            send_amount.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        h.propose_ok(&mut rt, BOB, send_amount, METHOD_SEND, RawBytes::default());

        // verify that sending any value is prevented
        let send_amount = TokenAmount::from(1u8);
        expect_abort(
            ExitCode::USR_INSUFFICIENT_FUNDS,
            h.propose(&mut rt, BOB, send_amount, METHOD_SEND, RawBytes::default()),
        )
    }
}

// Propose

#[test]
fn test_simple_propose() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];

    let send_value = TokenAmount::from(10u8);
    h.construct_and_verify(&mut rt, 2, no_unlock_duration, start_epoch, signers);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    h.propose_ok(&mut rt, chuck, send_value.clone(), METHOD_SEND, RawBytes::default());
    let txn0 = Transaction {
        to: chuck,
        value: send_value,
        method: METHOD_SEND,
        params: RawBytes::default(),
        approved: vec![anne],
    };
    let expect_txns = vec![(TxnID(0), txn0)];
    h.assert_transactions(&rt, expect_txns);
}

#[test]
fn test_propose_with_threshold_met() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    let num_approvals = 1;
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let fake_params = RawBytes::from([99u8; 3].to_vec());
    let send_value = TokenAmount::from(10u8);

    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];
    rt.set_balance(TokenAmount::from(10u8));
    rt.set_received(TokenAmount::zero());
    h.construct_and_verify(&mut rt, num_approvals, no_unlock_duration, start_epoch, signers);

    rt.expect_send(
        chuck,
        METHOD_SEND,
        fake_params.clone(),
        send_value.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    h.propose_ok(&mut rt, chuck, send_value, METHOD_SEND, fake_params);
    h.assert_transactions(&rt, vec![]);
}

#[test]
fn test_propose_with_threshold_and_non_empty_return_value() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    let num_approvals = 1;
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let fake_params = RawBytes::from([99u8; 3].to_vec());
    let send_value = TokenAmount::from(10u8);
    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];

    rt.set_balance(TokenAmount::from(20u8));
    rt.set_received(TokenAmount::zero());
    h.construct_and_verify(&mut rt, num_approvals, no_unlock_duration, start_epoch, signers);

    #[derive(Serialize_tuple, Deserialize_tuple)]
    struct FakeReturn {
        addr1: Address,
        addr2: Address,
        #[serde(with = "bigint_ser")]
        tokens: TokenAmount,
    }

    let propose_ret = FakeReturn {
        addr1: Address::new_id(1),
        addr2: Address::new_id(2),
        tokens: TokenAmount::from(77u8),
    };
    let inner_ret_bytes = serialize(&propose_ret, "fake proposal return value").unwrap();
    let fake_method = 42u64;
    rt.expect_send(
        chuck,
        fake_method,
        fake_params.clone(),
        send_value.clone(),
        inner_ret_bytes.clone(),
        ExitCode::OK,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    let ret = h
        .propose(&mut rt, chuck, send_value, fake_method, fake_params)
        .unwrap()
        .deserialize::<ProposeReturn>()
        .unwrap();
    assert!(ret.applied);
    assert_eq!(TxnID(0), ret.txn_id);
    assert_eq!(ExitCode::OK, ret.code);
    assert_eq!(inner_ret_bytes, ret.ret);
}

#[test]
fn test_fail_propose_with_threshold_met_and_insufficient_balance() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    let num_approvals = 1;
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let fake_params = RawBytes::from([99u8; 3].to_vec());
    let send_value = TokenAmount::from(10u8);
    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];

    rt.set_balance(TokenAmount::zero());
    rt.set_received(TokenAmount::zero());
    h.construct_and_verify(&mut rt, num_approvals, no_unlock_duration, start_epoch, signers);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    expect_abort(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        h.propose(&mut rt, chuck, send_value, METHOD_SEND, fake_params),
    );
    rt.reset();
    h.assert_transactions(&mut rt, vec![]);
}

#[test]
fn test_fail_propose_from_non_signer() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    let num_approvals = 1;
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let fake_params = RawBytes::from([99u8; 3].to_vec());
    let send_value = TokenAmount::from(10u8);
    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];

    rt.set_balance(TokenAmount::zero());
    rt.set_received(TokenAmount::zero());
    h.construct_and_verify(&mut rt, num_approvals, no_unlock_duration, start_epoch, signers);

    // non signer
    let richard = Address::new_id(105);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, richard);
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        h.propose(&mut rt, chuck, send_value, METHOD_SEND, fake_params),
    );

    rt.reset();
    h.assert_transactions(&mut rt, vec![]);
}

// AddSigner
#[test]
fn test_add_signer() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let chuck_pubkey = Address::new_bls(&[3u8; BLS_PUB_LEN]).unwrap();

    struct TestCase<'a> {
        #[allow(dead_code)]
        desc: &'a str,

        id_addr_mapping: Vec<(Address, Address)>, // non-id to id
        initial_signers: Vec<Address>,
        initial_approvals: u64,

        add_signer: Address,
        increase: bool,

        expect_signers: Vec<Address>,
        expect_approvals: u64,
        code: ExitCode,
    }

    let test_cases = vec![
        TestCase{
            desc: "happy path add signer",
            id_addr_mapping: Vec::new(),
            initial_signers: vec![anne, bob],
            initial_approvals: 2,
            add_signer: chuck,
            increase: false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 2,
            code: ExitCode::OK,
        },
        TestCase{
            desc: "add signer and increase threshold",
            id_addr_mapping: Vec::new(),
            initial_signers: vec![anne, bob],
            initial_approvals: 2,
            add_signer: chuck,
            increase: true,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::OK,
        },
        TestCase{
            desc: "fail to add signer that already exists",
            id_addr_mapping: Vec::new(),
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            add_signer: chuck,
            increase: false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase{
            desc: "fail to add signer with ID address that already exists even thugh we only have non ID address as approver",
            id_addr_mapping: vec![(chuck_pubkey, chuck)],
            initial_signers: vec![anne, bob, chuck_pubkey],
            initial_approvals: 3,
            add_signer: chuck,
            increase:false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase{
            desc: "fail to add signer with ID address that already exists even thugh we only have non ID address as approver",
            id_addr_mapping: vec![(chuck_pubkey, chuck)],
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 3,
            add_signer: chuck_pubkey,
            increase:false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::USR_FORBIDDEN,
        }
    ];

    for tc in test_cases {
        let mut rt = construct_runtime(msig);
        let h = util::ActorHarness::new();
        for (src, target) in tc.id_addr_mapping {
            rt.id_addresses.insert(src, target);
        }

        h.construct_and_verify(&mut rt, tc.initial_approvals, 0, 0, tc.initial_signers);

        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        match tc.code {
            ExitCode::OK => {
                let ret = h.add_signer(&mut rt, tc.add_signer, tc.increase).unwrap();
                assert_eq!(RawBytes::default(), ret);
                let st: State = rt.get_state();
                assert_eq!(tc.expect_signers, st.signers);
                assert_eq!(tc.expect_approvals, st.num_approvals_threshold);
            }
            _ => expect_abort(tc.code, h.add_signer(&mut rt, tc.add_signer, tc.increase)),
        }
    }
}

// RemoveSigner

#[test]
fn test_remove_signer() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let anne_non_id = Address::new_bls(&[3u8; BLS_PUB_LEN]).unwrap();
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let richard = Address::new_id(104);

    struct TestCase<'a> {
        #[allow(dead_code)]
        desc: &'a str,

        initial_signers: Vec<Address>,
        initial_approvals: u64,

        remove_signer: Address,
        decrease: bool,

        expect_signers: Vec<Address>,
        expect_approvals: u64,
        code: ExitCode,
    }

    let test_cases = vec![
        TestCase {
            desc: "happy path remove signer",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: chuck,
            decrease: false,
            expect_signers: vec![anne, bob],
            expect_approvals: 2,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer and decrease threshold",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: chuck,
            decrease: true,
            expect_signers: vec![anne, bob],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer when msig is created with an id addr and removed with pk addr",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: anne_non_id,
            decrease: true,
            expect_signers: vec![bob, chuck],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer when msig created with pk addr and removed with id addr",
            initial_signers: vec![anne_non_id, bob, chuck],
            initial_approvals: 2,
            remove_signer: anne,
            decrease: true,
            expect_signers: vec![bob, chuck],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer when msig is created and removed with pk addr",
            initial_signers: vec![anne_non_id, bob, chuck],
            initial_approvals: 2,
            remove_signer: anne_non_id,
            decrease: true,
            expect_signers: vec![bob, chuck],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "fail signer if decrease is set to false and number of signers below threshold",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 3,
            remove_signer: chuck,
            decrease: false,
            expect_signers: vec![],
            expect_approvals: 0,
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
        TestCase {
            desc: "remove signer from single signer list",
            initial_signers: vec![anne],
            initial_approvals: 1,
            remove_signer: anne,
            decrease: false,
            expect_signers: vec![],
            expect_approvals: 0,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase {
            desc: "fail to remove non-signer",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: richard,
            decrease: false,
            expect_signers: vec![],
            expect_approvals: 0,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase {
            desc: "fail to remove a signer and decrease approvals below 1",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 1,
            remove_signer: anne,
            decrease: true,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 1,
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
    ];

    for tc in test_cases {
        let mut rt = construct_runtime(msig);
        rt.id_addresses.insert(anne_non_id, anne);
        let h = util::ActorHarness::new();
        h.construct_and_verify(&mut rt, tc.initial_approvals, 0, 0, tc.initial_signers);

        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        let ret = h.remove_signer(&mut rt, tc.remove_signer, tc.decrease);

        match tc.code {
            ExitCode::OK => {
                assert_eq!(RawBytes::default(), ret.unwrap());
                let st: State = rt.get_state();
                assert_eq!(tc.expect_signers, st.signers);
                assert_eq!(tc.expect_approvals, st.num_approvals_threshold);
            }
            _ => assert_eq!(
                tc.code,
                ret.expect_err("remove signer return expected to be actor error").exit_code()
            ),
        }
        rt.verify();
    }
}

// SwapSigner
#[test]
fn test_signer_swap() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let bob_non_id = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
    let chuck = Address::new_id(103);
    let darlene = Address::new_id(104);
    let num_approvals: u64 = 1;

    struct TestCase<'a> {
        #[allow(dead_code)]
        desc: &'a str,

        initial_signers: Vec<Address>,
        swap_to: Address,
        swap_from: Address,
        expect_signers: Vec<Address>,
        code: ExitCode,
    }

    let test_cases = vec![
        TestCase {
            desc: "happy path remove signer",
            initial_signers: vec![anne, bob],
            swap_to: chuck,
            swap_from: bob,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "swap signer when multi-sig is created with it's ID address but we ask for a swap with it's non-ID address",
            initial_signers: vec![anne, bob],
            swap_to: chuck,
            swap_from: bob_non_id,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "swap signer when multi-sig is created with it's non-ID address but we ask for a swap with it's ID address",
            initial_signers: vec![anne, bob_non_id],
            swap_to: chuck,
            swap_from: bob,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "swap signer when multi-sig is created with it's non-ID address and we ask for a swap with it's non-ID address",
            initial_signers: vec![anne, bob_non_id],
            swap_to: chuck,
            swap_from: bob_non_id,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "fail to swap when from signer not found",
            initial_signers: vec![anne, bob],
            swap_to: chuck,
            swap_from: darlene,
            expect_signers: vec![],
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase {
            desc: "fail to swap when to signer already present",
            initial_signers: vec![anne, bob],
            swap_to: bob,
            swap_from: anne,
            expect_signers: vec![],
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
        TestCase {
            desc: "fail to swap when to signer ID address already present(even though we have the non-ID address)",
            initial_signers: vec![anne, bob_non_id],
            swap_to: bob,
            swap_from: anne,
            expect_signers: vec![],
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
        TestCase {
            desc: "fail to swap when to signer non-ID address already present(even though we have the ID address)",
            initial_signers: vec![anne, bob],
            swap_to: bob_non_id,
            swap_from: anne,
            expect_signers: vec![],
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        }
    ];

    for tc in test_cases {
        let mut rt = construct_runtime(msig);
        rt.id_addresses.insert(bob_non_id, bob);
        let h = util::ActorHarness::new();
        h.construct_and_verify(&mut rt, num_approvals, 0, 0, tc.initial_signers);

        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        let ret = h.swap_signers(&mut rt, tc.swap_from, tc.swap_to);
        match tc.code {
            ExitCode::OK => {
                assert_eq!(RawBytes::default(), ret.unwrap());
                let st: State = rt.get_state();
                assert_eq!(tc.expect_signers, st.signers);
            }
            _ => assert_eq!(
                tc.code,
                ret.expect_err("swap signer return expected to be actor error").exit_code()
            ),
        };
    }
}

// Approve
mod approval_tests {
    use super::*;

    #[test]
    fn test_approve_simple_propose_and_approval() {
        // setup rt
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let h = util::ActorHarness::new();
        // construct msig
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        let fake_ret = RawBytes::from(vec![4, 3, 2, 1]);
        let send_value = TokenAmount::from(10u8);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        // assert txn
        let expect_txn = Transaction {
            to: chuck,
            value: send_value.clone(),
            method: fake_method,
            params: fake_params.clone(),
            approved: vec![anne],
        };
        h.assert_transactions(&rt, vec![(TxnID(0), expect_txn)]);

        // approval
        rt.set_balance(send_value.clone());
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        rt.expect_send(chuck, fake_method, fake_params, send_value, fake_ret, ExitCode::OK);
        h.approve_ok(&mut rt, TxnID(0), proposal_hash);
        h.assert_transactions(&rt, vec![]);
    }

    #[test]
    fn test_approve_with_non_empty_ret_value() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        let fake_ret = RawBytes::from(vec![4, 3, 2, 1]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        rt.expect_send(chuck, fake_method, fake_params, send_value, fake_ret.clone(), ExitCode::OK);
        let ret = h.approve_ok(&mut rt, TxnID(0), proposal_hash);
        assert_eq!(fake_ret, ret);
        h.assert_transactions(&rt, vec![]);
    }

    #[test]
    fn test_approval_works_if_enough_funds_have_been_unlocked_for_the_tx() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(20u8);
        let unlock_duration = 20;
        let start_epoch = 10;
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(send_value.clone());
        h.construct_and_verify(&mut rt, 2, unlock_duration, start_epoch, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());
        h.assert_transactions(
            &rt,
            vec![(
                TxnID(0),
                Transaction {
                    to: chuck,
                    value: send_value.clone(),
                    method: fake_method,
                    params: fake_params.clone(),
                    approved: vec![anne],
                },
            )],
        );
        rt.set_epoch(start_epoch + unlock_duration);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        rt.expect_send(
            chuck,
            fake_method,
            fake_params,
            send_value,
            RawBytes::default(),
            ExitCode::OK,
        );

        h.approve_ok(&mut rt, TxnID(0), proposal_hash);
    }

    #[test]
    fn test_fail_approval_if_current_balance_less_than_tx_value() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone() - 1);
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        expect_abort(ExitCode::USR_INSUFFICIENT_FUNDS, h.approve(&mut rt, TxnID(0), proposal_hash));
        h.assert_transactions(
            &rt,
            vec![(
                TxnID(0),
                Transaction {
                    to: chuck,
                    value: send_value.clone(),
                    method: fake_method,
                    params: fake_params.clone(),
                    approved: vec![anne],
                },
            )],
        );
    }
    #[test]
    fn fail_approval_if_not_enough_unlocked_balance_available() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(20u8);
        let unlock_duration = 20;
        let start_epoch = 10;
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(send_value.clone());
        h.construct_and_verify(&mut rt, 2, unlock_duration, start_epoch, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());
        h.assert_transactions(
            &rt,
            vec![(
                TxnID(0),
                Transaction {
                    to: chuck,
                    value: send_value.clone(),
                    method: fake_method,
                    params: fake_params.clone(),
                    approved: vec![anne],
                },
            )],
        );
        rt.set_epoch(start_epoch + unlock_duration / 2);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        expect_abort(ExitCode::USR_INSUFFICIENT_FUNDS, h.approve(&mut rt, TxnID(0), proposal_hash))
    }

    #[test]
    fn fail_approval_with_bad_proposal_hash() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());
        let bad_hash = compute_proposal_hash(
            &Transaction {
                to: chuck,
                value: send_value.clone(),
                method: fake_method,
                params: fake_params.clone(),
                approved: vec![bob], //mismatch
            },
            &rt,
        )
        .unwrap();
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, h.approve(&mut rt, TxnID(0), bad_hash));
    }

    #[test]
    fn accept_approval_with_no_proposal_hash() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        rt.expect_send(
            chuck,
            fake_method,
            fake_params,
            send_value,
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
        let params = TxnIDParams { id: TxnID(0), proposal_hash: Vec::<u8>::new() };
        rt.call::<MultisigActor>(Method::Approve as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        rt.verify();
    }

    #[test]
    fn fail_approve_tx_more_than_once() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        // anne tries to approve a tx she proposed and fails
        expect_abort(ExitCode::USR_FORBIDDEN, h.approve(&mut rt, TxnID(0), proposal_hash));
        rt.reset();
        h.assert_transactions(
            &rt,
            vec![(
                TxnID(0),
                Transaction {
                    to: chuck,
                    value: send_value,
                    method: fake_method,
                    params: fake_params,
                    approved: vec![anne],
                },
            )],
        );
    }

    #[test]
    fn fail_approve_tx_that_does_not_exist() {
        let dne_tx_id = TxnID(1);
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 1, 0, 0, signers);

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
        let params = TxnIDParams { id: dne_tx_id, proposal_hash: Vec::<u8>::new() };
        rt.call::<MultisigActor>(Method::Approve as u64, &RawBytes::serialize(params).unwrap())
            .expect_err("should fail on approve of non existent tx id");
        rt.verify();
    }

    #[test]
    fn fail_to_approve_tx_by_non_signer() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        let richard = Address::new_id(105);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, richard);
        expect_abort(ExitCode::USR_FORBIDDEN, h.approve(&mut rt, TxnID(0), proposal_hash));
        rt.reset();
        h.assert_transactions(
            &rt,
            vec![(
                TxnID(0),
                Transaction {
                    to: chuck,
                    value: send_value,
                    method: fake_method,
                    params: fake_params,
                    approved: vec![anne],
                },
            )],
        )
    }

    #[test]
    fn proposed_tx_is_approved_if_number_approvers_has_crossed_threshold() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        // reduce threshold so tx is already approved
        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        let new_threshold = 1;
        h.change_num_approvals_threshold(&mut rt, new_threshold).unwrap();

        // self approval executes tx because the msig is across the threshold
        rt.expect_send(
            chuck,
            fake_method,
            fake_params,
            send_value,
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        h.approve_ok(&mut rt, TxnID(0), proposal_hash);
        h.assert_transactions(&rt, vec![]);
    }

    #[test]
    fn approve_tx_if_num_approvers_has_crossed_threshold_even_if_duplicate_approval() {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob, chuck];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 3, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        // bob approves tx
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        h.approve_ok(&mut rt, TxnID(0), proposal_hash);

        // reduce threshold so tx is already approved
        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        let new_threshold = 2;
        h.change_num_approvals_threshold(&mut rt, new_threshold).unwrap();

        // duplicate approval executes tx because the msig is across the threshold
        rt.expect_send(
            chuck,
            fake_method,
            fake_params,
            send_value,
            RawBytes::default(),
            ExitCode::OK,
        );
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
        h.approve_ok(&mut rt, TxnID(0), proposal_hash);
        h.assert_transactions(&rt, vec![]);
    }

    #[test]
    fn approve_tx_if_num_approvers_has_already_crossed_threshold_but_non_signatory_cannot_approve_tx(
    ) {
        let msig = Address::new_id(100);
        let anne = Address::new_id(101);
        let bob = Address::new_id(102);
        let chuck = Address::new_id(103);
        let signers = vec![anne, bob];
        let mut rt = construct_runtime(msig);
        let send_value = TokenAmount::from(10u8);
        let h = util::ActorHarness::new();
        rt.set_balance(send_value.clone());
        rt.set_received(TokenAmount::zero());
        h.construct_and_verify(&mut rt, 2, 0, 0, signers);

        let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
        let fake_method = 42;
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        let proposal_hash =
            h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

        // reduce threshold so tx is already approved
        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        let new_threshold = 1;
        h.change_num_approvals_threshold(&mut rt, new_threshold).unwrap();

        // non-signer alice cannot approve the tx
        let alice = Address::new_id(104);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, alice);
        expect_abort(ExitCode::USR_FORBIDDEN, h.approve(&mut rt, TxnID(0), proposal_hash));
        rt.reset();

        // anne can self approve with lower threshold
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
        rt.expect_send(
            chuck,
            fake_method,
            fake_params,
            send_value,
            RawBytes::default(),
            ExitCode::OK,
        );
        h.approve_ok(&mut rt, TxnID(0), proposal_hash);

        h.assert_transactions(&rt, vec![]);
    }
}

// Cancel
#[test]
fn test_simple_propose_and_cancel() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();
    let signers = vec![anne, bob];

    h.construct_and_verify(&mut rt, 2, 0, 0, signers);

    let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
    let fake_method = 42;
    let send_value = TokenAmount::from(10u8);
    // anne proposes tx
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    let proposal_hash = h.propose_ok(&mut rt, chuck, send_value, fake_method, fake_params);

    // anne cancels the tx
    let ret = h.cancel(&mut rt, TxnID(0), proposal_hash).unwrap();
    assert_eq!(RawBytes::default(), ret);

    // tx should be removed from actor state
    h.assert_transactions(&rt, vec![]);
}

// LockBalance
#[test]
fn test_lock_balance_checks_preconditions() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    h.construct_and_verify(&mut rt, 1, 0, 0, vec![anne]);

    let vest_start = 0_i64;
    let lock_amount = TokenAmount::from(100_000u32);
    let vest_duration = 1000_i64;

    // Disallow negative duration but allow negative start epoch
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.lock_balance(&mut rt, vest_start, -1_i64, lock_amount),
    );

    // Disallow negative amount
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.lock_balance(&mut rt, vest_start, vest_duration, TokenAmount::from(-1i32)),
    );
}

// ChangeNumApprovalsThreshold
#[test]
fn test_change_threshold_happy_path_decrease_threshold() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();
    let signers = vec![anne, bob, chuck];
    let initial_threshold = 2;

    h.construct_and_verify(&mut rt, initial_threshold, 0, 0, signers);

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
    let ret = h.change_num_approvals_threshold(&mut rt, 1).unwrap();
    assert_eq!(RawBytes::default(), ret);
    let st: State = rt.get_state();
    assert_eq!(1, st.num_approvals_threshold);
}

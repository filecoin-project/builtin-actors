use fil_actor_multisig::{
    compute_proposal_hash, Actor, AddSignerParams, ConstructorParams, Method, ProposeParams, State,
    Transaction, TxnID,
};
//use fil_actor_multisig::types::{TxnID, BytesKey};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fil_actors_runtime::{make_map_with_root, ActorError};
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::RawBytes;
use fvm_shared::MethodNum;
use std::collections::HashMap;
pub struct ActorHarness {}

impl ActorHarness {
    pub fn construct_and_verify(
        self: &Self,
        rt: &mut MockRuntime,
        initial_approvals: u64,
        unlock_duration: ChainEpoch,
        start_epoch: ChainEpoch,
        initial_signers: Vec<Address>,
    ) {
        let params = ConstructorParams {
            signers: initial_signers,
            num_approvals_threshold: initial_approvals,
            unlock_duration: unlock_duration,
            start_epoch: start_epoch,
        };
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        let result = rt
            .call::<Actor>(Method::Constructor as u64, &RawBytes::serialize(params).unwrap())
            .unwrap();
        assert_eq!(result.bytes().len(), 0);
        rt.verify();
    }

    pub fn add_signer(
        self: &Self,
        rt: &mut MockRuntime,
        signer: Address,
        increase: bool,
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_addr(vec![rt.receiver]);
        let params = AddSignerParams { signer: signer, increase: increase };
        let ret = rt.call::<Actor>(Method::AddSigner as u64, &RawBytes::serialize(params).unwrap());
        rt.verify();
        ret
    }

    pub fn propose_ok(
        self: &Self,
        rt: &mut MockRuntime,
        to: Address,
        value: TokenAmount,
        method: MethodNum,
        params: RawBytes,
    ) -> [u8; 32] {
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
        let propose_params =
            ProposeParams { to: to, value: value.clone(), method: method, params: params.clone() };
        expect_ok(
            rt.call::<Actor>(Method::Propose as u64, &RawBytes::serialize(propose_params).unwrap()),
        );
        // compute proposal hash
        let txn = Transaction {
            to: to,
            value: value,
            method: method,
            params: params,
            approved: vec![rt.caller],
        };
        compute_proposal_hash(&txn, rt).unwrap()
    }

    pub fn assert_transactions(
        self: &Self,
        rt: &MockRuntime,
        expect_txns: HashMap<TxnID, Transaction>,
    ) {
        let st = rt.get_state::<State>().unwrap();
        let ptx = make_map_with_root::<_, Transaction>(&st.pending_txs, &rt.store).unwrap();
        let expect_count = expect_txns.len();
        // check that all expected txns exist in state
        for (txn_id, expect_v) in expect_txns {
            let v = ptx.get(&txn_id.key()).unwrap().unwrap();
            assert_eq!(expect_v, *v);
        }
        // check that there are no more txns in state than in expected
        let mut count = 0;
        ptx.for_each(|_tx_id, _txn: &Transaction| {
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(expect_count, count)
    }
}

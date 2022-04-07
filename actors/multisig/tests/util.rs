use fil_actor_multisig::TxnIDParams;
use fil_actor_multisig::{
    compute_proposal_hash, Actor, AddSignerParams, ApproveReturn, ConstructorParams, Method,
    ProposeParams, State, Transaction, TxnID,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fil_actors_runtime::{make_map_with_root, parse_uint_key, ActorError};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;
pub struct ActorHarness {}

impl ActorHarness {
    pub fn new() -> ActorHarness {
        ActorHarness {}
    }

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
        rt.verify();
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

    // requires that the approval finishes the transaction and that the resulting invocation succeeds.
    // returns the (raw) output of the successful invocation.
    pub fn approve_ok(
        self: &Self,
        rt: &mut MockRuntime,
        txn_id: TxnID,
        proposal_hash: [u8; 32],
    ) -> RawBytes {
        let ret = self.approve(rt, txn_id, proposal_hash).unwrap();
        let approve_ret = ret.deserialize::<ApproveReturn>().unwrap();
        assert_eq!(ExitCode::Ok, approve_ret.code);
        approve_ret.ret
    }

    pub fn approve(
        self: &Self,
        rt: &mut MockRuntime,
        txn_id: TxnID,
        proposal_hash: [u8; 32],
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
        let approve_params =
            TxnIDParams { id: txn_id, proposal_hash: Vec::<u8>::from(proposal_hash) };
        let ret =
            rt.call::<Actor>(Method::Approve as u64, &RawBytes::serialize(approve_params).unwrap());
        rt.verify();
        ret
    }

    pub fn assert_transactions(
        self: &Self,
        rt: &MockRuntime,
        mut expect_txns: Vec<(TxnID, Transaction)>,
    ) {
        let st = rt.get_state::<State>().unwrap();
        let ptx = make_map_with_root::<_, Transaction>(&st.pending_txs, &rt.store).unwrap();
        let mut actual_txns = Vec::new();
        ptx.for_each(|k, txn: &Transaction| {
            actual_txns.push((TxnID(parse_uint_key(k)? as i64), txn.clone()));
            Ok(())
        })
        .unwrap();
        expect_txns.sort_by_key(|(TxnID(id), _txn)| (*id));
        actual_txns.sort_by_key(|(TxnID(id), _txn)| (*id));
        assert_eq!(expect_txns, actual_txns);
    }
}

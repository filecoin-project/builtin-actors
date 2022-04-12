use fil_actor_multisig::{
    compute_proposal_hash, Actor, AddSignerParams, ApproveReturn, ConstructorParams, Method,
    ProposeParams, RemoveSignerParams, State, SwapSignerParams, Transaction, TxnID, TxnIDParams,
};
use fil_actor_multisig::{ChangeNumApprovalsThresholdParams, LockBalanceParams};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fil_actors_runtime::{make_map_with_root, parse_uint_key, ActorError};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

#[derive(Default)]
pub struct ActorHarness {}

impl ActorHarness {
    pub fn new() -> ActorHarness {
        Default::default()
    }

    pub fn construct_and_verify(
        &self,
        rt: &mut MockRuntime,
        initial_approvals: u64,
        unlock_duration: ChainEpoch,
        start_epoch: ChainEpoch,
        initial_signers: Vec<Address>,
    ) {
        let params = ConstructorParams {
            signers: initial_signers,
            num_approvals_threshold: initial_approvals,
            unlock_duration,
            start_epoch,
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
        &self,
        rt: &mut MockRuntime,
        signer: Address,
        increase: bool,
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_addr(vec![rt.receiver]);
        let params = AddSignerParams { signer, increase };
        let ret = rt.call::<Actor>(Method::AddSigner as u64, &RawBytes::serialize(params).unwrap());
        rt.verify();
        ret
    }

    pub fn remove_signer(
        &self,
        rt: &mut MockRuntime,
        signer: Address,
        decrease: bool,
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_addr(vec![rt.receiver]);
        let params = RemoveSignerParams { signer, decrease };
        let ret =
            rt.call::<Actor>(Method::RemoveSigner as u64, &RawBytes::serialize(params).unwrap());
        rt.verify();
        ret
    }

    pub fn swap_signers(
        &self,
        rt: &mut MockRuntime,
        old_signer: Address,
        new_signer: Address,
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_addr(vec![rt.receiver]);
        let params = SwapSignerParams { from: old_signer, to: new_signer };
        let ret =
            rt.call::<Actor>(Method::SwapSigner as u64, &RawBytes::serialize(params).unwrap());
        rt.verify();
        ret
    }

    pub fn propose_ok(
        &self,
        rt: &mut MockRuntime,
        to: Address,
        value: TokenAmount,
        method: MethodNum,
        params: RawBytes,
    ) -> [u8; 32] {
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
        let propose_params =
            ProposeParams { to, value: value.clone(), method, params: params.clone() };
        rt.call::<Actor>(Method::Propose as u64, &RawBytes::serialize(propose_params).unwrap())
            .unwrap();
        rt.verify();
        // compute proposal hash
        let txn = Transaction { to, value, method, params, approved: vec![rt.caller] };
        compute_proposal_hash(&txn, rt).unwrap()
    }

    // requires that the approval finishes the transaction and that the resulting invocation succeeds.
    // returns the (raw) output of the successful invocation.
    pub fn approve_ok(
        &self,
        rt: &mut MockRuntime,
        txn_id: TxnID,
        proposal_hash: [u8; 32],
    ) -> RawBytes {
        let ret = self.approve(rt, txn_id, proposal_hash).unwrap();
        let approve_ret = ret.deserialize::<ApproveReturn>().unwrap();
        assert_eq!(ExitCode::OK, approve_ret.code);
        approve_ret.ret
    }

    pub fn approve(
        &self,
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

    pub fn cancel(
        &self,
        rt: &mut MockRuntime,
        txn_id: TxnID,
        proposal_hash: [u8; 32],
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
        let cancel_params =
            TxnIDParams { id: txn_id, proposal_hash: Vec::<u8>::from(proposal_hash) };
        let ret =
            rt.call::<Actor>(Method::Cancel as u64, &RawBytes::serialize(cancel_params).unwrap());
        rt.verify();
        ret
    }

    pub fn lock_balance(
        &self,
        rt: &mut MockRuntime,
        start: ChainEpoch,
        duration: ChainEpoch,
        amount: TokenAmount,
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_addr(vec![rt.receiver]);
        let lock_balance_params =
            LockBalanceParams { start_epoch: start, unlock_duration: duration, amount };
        let ret = rt.call::<Actor>(
            Method::LockBalance as u64,
            &RawBytes::serialize(lock_balance_params).unwrap(),
        );
        rt.verify();
        ret
    }

    pub fn change_num_approvals_threshold(
        &self,
        rt: &mut MockRuntime,
        new_threshold: u64,
    ) -> Result<RawBytes, ActorError> {
        rt.expect_validate_caller_addr(vec![rt.receiver]);
        let change_threshold_params = ChangeNumApprovalsThresholdParams { new_threshold };
        let ret = rt.call::<Actor>(
            Method::ChangeNumApprovalsThreshold as u64,
            &RawBytes::serialize(change_threshold_params).unwrap(),
        );
        rt.verify();
        ret
    }

    pub fn assert_transactions(
        &self,
        rt: &MockRuntime,
        mut expect_txns: Vec<(TxnID, Transaction)>,
    ) {
        let st: State = rt.get_state();
        let ptx = make_map_with_root::<_, Transaction>(&st.pending_txs, &rt.store).unwrap();
        let mut actual_txns = Vec::new();
        ptx.for_each(|k, txn: &Transaction| {
            actual_txns.push((TxnID(parse_uint_key(k).unwrap() as i64), txn.clone()));
        })
        .unwrap();
        expect_txns.sort_by_key(|(TxnID(id), _txn)| (*id));
        actual_txns.sort_by_key(|(TxnID(id), _txn)| (*id));
        assert_eq!(expect_txns, actual_txns);
    }
}

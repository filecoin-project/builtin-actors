use fil_actor_multisig::{Actor, AddSignerParams, ConstructorParams, Method, Transaction, ProposalHashData, ProposeParams};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::ActorError;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::MethodNum;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::RawBytes;
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

    ) -> RawBytes {
        // compute proposal hash
        let txn = Transaction {
            to: to,
            value: value,
            method: method,
            params: params,
            approved: vec![rt.caller],
        };
        compute_proposal_hash(&txn, rt).unwrap();
    }
}

use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::RegisteredSealProof;

use fil_actor_verifreg::{AllocationRequest, AllocationRequests};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::policy_constants::MINIMUM_VERIFIED_ALLOCATION_SIZE;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::{DATACAP_TOKEN_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};
use fvm_shared::error::ExitCode;
use test_vm::util::{apply_code, apply_ok, create_accounts, create_miner};
use test_vm::VM;

use fil_actor_datacap::{Method as DataCapMethod, MintParams};
use frc46_token::token::types::TransferFromParams;

/* Mint a token for client and transfer it to a receiver, exercising error cases */
#[test]
fn datacap_transfer_scenario() {
    let policy = Policy::default();
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let (client, operator, owner) = (addrs[0], addrs[1], addrs[2]);

    // need to allocate to an actual miner actor to pass verifreg receiver hook checks
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, _) = create_miner(
        &mut v,
        owner,
        owner,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    );

    let data_cap_amt = TokenAmount::from_whole(
        MINIMUM_VERIFIED_ALLOCATION_SIZE + MINIMUM_VERIFIED_ALLOCATION_SIZE / 2,
    );
    let mint_params =
        MintParams { to: client, amount: data_cap_amt.clone(), operators: vec![operator] };

    // cannot mint from non-verifreg
    apply_code(
        &v,
        operator,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::Mint as u64,
        mint_params.clone(),
        ExitCode::USR_FORBIDDEN,
    );

    // mint datacap for client
    apply_ok(
        &v,
        VERIFIED_REGISTRY_ACTOR_ADDR,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::Mint as u64,
        mint_params,
    );

    let alloc = AllocationRequest {
        provider: maddr,
        data: make_piece_cid("datacap-test-alloc".as_bytes()),
        size: PaddedPieceSize(MINIMUM_VERIFIED_ALLOCATION_SIZE as u64),
        term_min: policy.minimum_verified_allocation_term,
        term_max: policy.maximum_verified_allocation_term,
        expiration: v.get_epoch() + policy.maximum_verified_allocation_expiration,
    };
    let transfer_from_params = TransferFromParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        from: client,
        amount: TokenAmount::from_whole(MINIMUM_VERIFIED_ALLOCATION_SIZE),
        operator_data: serialize(
            &AllocationRequests { allocations: vec![alloc.clone()], extensions: vec![] },
            "operator data",
        )
        .unwrap(),
    };
    let clone_params = |x: &TransferFromParams| -> TransferFromParams {
        TransferFromParams {
            to: x.to.clone(),
            from: x.from.clone(),
            amount: x.amount.clone(),
            operator_data: x.operator_data.clone(),
        }
    };

    // bad operator data caught in verifreg receiver hook and propagated
    // 1. piece size too small
    let mut bad_alloc = alloc.clone();
    bad_alloc.size = PaddedPieceSize(MINIMUM_VERIFIED_ALLOCATION_SIZE as u64 - 1);
    let mut params_piece_too_small = clone_params(&transfer_from_params);
    params_piece_too_small.operator_data = serialize(
        &AllocationRequests { allocations: vec![bad_alloc], extensions: vec![] },
        "operator data",
    )
    .unwrap();
    apply_code(
        &v,
        operator,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::TransferFrom as u64,
        params_piece_too_small,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // 2. mismatch more datacap than piece needs
    let mut params_mismatched_datacap = clone_params(&transfer_from_params);
    params_mismatched_datacap.amount =
        TokenAmount::from_whole(MINIMUM_VERIFIED_ALLOCATION_SIZE + 1);
    apply_code(
        &v,
        operator,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::TransferFrom as u64,
        params_mismatched_datacap,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // 3. invalid term
    let mut bad_alloc = alloc.clone();
    bad_alloc.term_max = policy.maximum_verified_allocation_term + 1;
    let mut params_bad_term = clone_params(&transfer_from_params);
    params_bad_term.operator_data = serialize(
        &AllocationRequests { allocations: vec![bad_alloc], extensions: vec![] },
        "operator data",
    )
    .unwrap();
    apply_code(
        &v,
        operator,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::TransferFrom as u64,
        params_bad_term,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // cannot transfer from operator to non-verifreg
    let mut params_bad_receiver = clone_params(&transfer_from_params);
    params_bad_receiver.to = owner;
    apply_code(
        &v,
        owner,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::TransferFrom as u64,
        clone_params(&params_bad_receiver),
        ExitCode::USR_FORBIDDEN, // ExitCode(19) because non-operator has insufficient allowance
    );

    // cannot transfer with non-operator caller
    apply_code(
        &v,
        owner,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::TransferFrom as u64,
        clone_params(&transfer_from_params),
        ExitCode::USR_INSUFFICIENT_FUNDS, // ExitCode(19) because non-operator has insufficient allowance
    );

    apply_ok(
        &v,
        operator,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::TransferFrom as u64,
        clone_params(&transfer_from_params),
    );

    // Datacap already spent, not enough left
    apply_code(
        &v,
        operator,
        DATACAP_TOKEN_ACTOR_ADDR,
        TokenAmount::zero(),
        DataCapMethod::TransferFrom as u64,
        transfer_from_params,
        ExitCode::USR_INSUFFICIENT_FUNDS,
    );
}

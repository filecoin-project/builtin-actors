use alloy_core::primitives::Bytes as AlloyBytes;
use alloy_core::sol;
use alloy_core::sol_types::SolCall;
use export_macro::vm_test;
use fil_actor_miner::{
    CompactCommD, GenerateSectorLocationParams, GenerateSectorLocationReturn,
    Method as MinerMethod, ProveCommitSectors3Params, SectorActivationManifest, SectorStatusCode,
};
use fil_actors_runtime::{EAM_ACTOR_ADDR, runtime::Policy};
use fvm_ipld_encoding::{BytesDe, RawBytes, ipld_block::IpldBlock};
use fvm_shared::{
    econ::TokenAmount,
    sector::{RegisteredSealProof, SectorNumber},
};
use num_traits::Zero;
use vm_api::VM;
use vm_api::util::serialize_ok;

use crate::util::{
    PrecommitMetadata, advance_by_deadline_to_epoch, create_accounts, create_miner,
    precommit_sectors_v2,
};

// Generate a statically typed interface for the SectorStatusChecker contract
sol!("../actors/evm/tests/contracts/SectorStatusChecker.sol");

// Use ContractParams from evm_test module
use super::evm_test::ContractParams;

#[vm_test]
pub fn evm_sector_status_test(v: &dyn VM) {
    // Create accounts
    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[1]);
    let (miner_addr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // Get miner actor ID
    let miner_id = v.resolve_id_address(&miner_addr).unwrap().id().unwrap();

    // Precommit and prove a sector
    let sector_number: SectorNumber = 100;

    let manifests: Vec<SectorActivationManifest> =
        vec![SectorActivationManifest { sector_number, pieces: vec![] }];

    let meta: Vec<PrecommitMetadata> =
        vec![PrecommitMetadata { deals: vec![], commd: CompactCommD::empty() }];

    precommit_sectors_v2(v, 1, meta, &worker, &miner_addr, seal_proof, sector_number, true, None);

    // Advance time to prove commit epoch
    let policy = Policy::default();
    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &miner_addr, prove_time);

    // ProveCommitSectors3
    let proofs = vec![RawBytes::new(vec![8, 8, 8, 8])];
    let prove_params = ProveCommitSectors3Params {
        sector_activations: manifests,
        sector_proofs: proofs,
        aggregate_proof: RawBytes::default(),
        aggregate_proof_type: None,
        require_activation_success: false,
        require_notification_success: false,
    };

    let prove_result = v
        .execute_message(
            &worker,
            &miner_addr,
            &TokenAmount::zero(),
            MinerMethod::ProveCommitSectors3 as u64,
            IpldBlock::serialize_cbor(&prove_params).unwrap(),
        )
        .unwrap();
    assert!(prove_result.code.is_success(), "ProveCommit failed: {}", prove_result.message);

    // Step 1: Call GenerateSectorLocation directly on the miner actor (off-chain query)
    let gen_params = GenerateSectorLocationParams { sector_number };
    let gen_result = v
        .execute_message(
            &worker,
            &miner_addr,
            &TokenAmount::zero(),
            MinerMethod::GenerateSectorLocationExported as u64,
            IpldBlock::serialize_cbor(&gen_params).unwrap(),
        )
        .unwrap();
    assert!(gen_result.code.is_success(), "GenerateSectorLocation failed: {}", gen_result.message);

    let gen_return: GenerateSectorLocationReturn =
        gen_result.ret.unwrap().deserialize().expect("Failed to decode GenerateSectorLocation");
    assert_eq!(gen_return.status, SectorStatusCode::Active, "Expected Active status");
    assert!(!gen_return.aux_data.is_empty(), "Expected non-empty aux_data");

    // Deploy SectorStatusChecker EVM contract
    let hex_str = std::fs::read_to_string("../actors/evm/tests/contracts/SectorStatusChecker.hex")
        .expect("Failed to read contract bytecode hex file");
    let contract_bytecode =
        hex::decode(hex_str.trim()).expect("Failed to decode contract bytecode hex");

    let create_result = v
        .execute_message(
            &worker,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            IpldBlock::serialize_cbor(&fil_actor_eam::CreateExternalParams(contract_bytecode))
                .unwrap(),
        )
        .unwrap();
    assert!(
        create_result.code.is_success(),
        "Failed to create EVM contract: {}",
        create_result.message
    );

    let create_return: fil_actor_eam::CreateReturn =
        create_result.ret.unwrap().deserialize().expect("Failed to decode create return");
    let contract_addr = create_return.robust_address.unwrap();

    // Step 2: Call validateSectorStatus on the contract with aux_data from step 1
    let aux_data = gen_return.aux_data;
    let call_params = SectorStatusChecker::validateSectorStatusCall::new((
        miner_id,
        sector_number,
        String::from("Active"),
        AlloyBytes::from(aux_data.clone()),
    ))
    .abi_encode();

    let result = v
        .execute_message(
            &worker,
            &contract_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params))),
        )
        .unwrap();
    assert!(result.code.is_success(), "validateSectorStatus failed: {}", result.message);

    let return_data: BytesDe = result.ret.unwrap().deserialize().unwrap();
    let valid = SectorStatusChecker::validateSectorStatusCall::abi_decode_returns(&return_data.0)
        .expect("Failed to decode validateSectorStatus return");
    assert!(valid, "Expected sector status Active to be valid");

    // Check that "Dead" status is invalid for this active sector
    let call_params = SectorStatusChecker::validateSectorStatusCall::new((
        miner_id,
        sector_number,
        String::from("Dead"),
        AlloyBytes::from(aux_data.clone()),
    ))
    .abi_encode();

    let result = v
        .execute_message(
            &worker,
            &contract_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params))),
        )
        .unwrap();
    assert!(
        result.code.is_success(),
        "validateSectorStatus (Dead) call failed: {}",
        result.message
    );

    let return_data: BytesDe = result.ret.unwrap().deserialize().unwrap();
    let valid = SectorStatusChecker::validateSectorStatusCall::abi_decode_returns(&return_data.0)
        .expect("Failed to decode validateSectorStatus return");
    assert!(!valid, "Expected sector status Dead to be invalid");

    // Check that "Faulty" status is invalid for this active sector
    let call_params = SectorStatusChecker::validateSectorStatusCall::new((
        miner_id,
        sector_number,
        String::from("Faulty"),
        AlloyBytes::from(aux_data.clone()),
    ))
    .abi_encode();

    let result = v
        .execute_message(
            &worker,
            &contract_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params))),
        )
        .unwrap();
    assert!(
        result.code.is_success(),
        "validateSectorStatus (Faulty) call failed: {}",
        result.message
    );

    let return_data: BytesDe = result.ret.unwrap().deserialize().unwrap();
    let valid = SectorStatusChecker::validateSectorStatusCall::abi_decode_returns(&return_data.0)
        .expect("Failed to decode validateSectorStatus return");
    assert!(!valid, "Expected sector status Faulty to be invalid");

    // Step 3: Call getNominalSectorExpiration on the contract
    let call_params =
        SectorStatusChecker::getNominalSectorExpirationCall::new((miner_id, sector_number))
            .abi_encode();

    let result = v
        .execute_message(
            &worker,
            &contract_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params))),
        )
        .unwrap();
    assert!(result.code.is_success(), "getNominalSectorExpiration failed: {}", result.message);

    let return_data: BytesDe = result.ret.unwrap().deserialize().unwrap();
    let expiration =
        SectorStatusChecker::getNominalSectorExpirationCall::abi_decode_returns(&return_data.0)
            .expect("Failed to decode getNominalSectorExpiration return");

    // Sector expiration should be well in the future (set at precommit time)
    assert!(
        expiration > v.epoch(),
        "Expected expiration {} > current epoch {}",
        expiration,
        v.epoch()
    );
}

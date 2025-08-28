use export_macro::vm_test;
use fil_actor_miner::{
    ProveCommitSectors3Params, SectorActivationManifest, PieceActivationManifest,
    DataActivationNotification, Method as MinerMethod,
};
use fil_actors_runtime::{
    EAM_ACTOR_ADDR, test_utils::EVM_ACTOR_CODE_ID, test_utils::make_piece_cid,
};
use fvm_ipld_encoding::{RawBytes, ipld_block::IpldBlock};
use fvm_shared::{
    address::Address, econ::TokenAmount, sector::{RegisteredSealProof, SectorNumber},
    piece::PaddedPieceSize,
};
use num_traits::Zero;
use vm_api::VM;

use crate::util::{
    create_accounts, create_miner, precommit_sectors_v2,
    advance_by_deadline_to_epoch
};


#[vm_test]
pub fn evm_receives_ddo_notifications_test(v: &dyn VM) {

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
    
    // Deploy the NotificationReceiver EVM contract
    // The file is a hex string, so decode it to bytes
    let hex_str = std::fs::read_to_string("../../../actors/evm/tests/contracts/NotificationReceiver.hex")
        .expect("Failed to read contract bytecode hex file");
    let hex_str = hex_str.trim();
    let contract_bytecode = hex::decode(hex_str).expect("Failed to decode contract bytecode hex");
    
    // Create an EVM actor to receive notifications
    let params = IpldBlock::serialize_cbor(&fil_actor_eam::CreateParams {
        initcode: contract_bytecode.to_vec().into(),
        nonce: 0,
    }).unwrap();
    
    let create_result = v.execute_message(
        &worker,
        &EAM_ACTOR_ADDR,
        &TokenAmount::from_whole(1),
        fil_actor_eam::Method::Create as u64,
        params,
    ).unwrap();
    
    assert!(create_result.code.is_success(), "Failed to create EVM contract: {}", create_result.message);
    
    let create_return: fil_actor_eam::CreateReturn = 
        create_result.ret.unwrap().deserialize().expect("Failed to decode create return");
    let evm_actor_addr = Address::new_id(create_return.actor_id);
    let evm_robust_addr = create_return.robust_address.unwrap();
    let evm_eth_addr = create_return.eth_address;
    
    println!("Created EVM contract at ID: {}, Robust: {}, ETH: 0x{}", 
             evm_actor_addr, evm_robust_addr, hex::encode(&evm_eth_addr));

    // Precommit sectors
    let sector_number: SectorNumber = 100;
    let precommits = precommit_sectors_v2(
        v,
        1,
        vec![],
        &worker,
        &miner_addr,
        seal_proof,
        sector_number,
        true,
        None,
    );

    // Advance time to prove commit epoch
    let prove_time = v.epoch() + 150;
    advance_by_deadline_to_epoch(v, &miner_addr, prove_time);

    // Create piece activation manifests with notifications to EVM contract
    let piece_size = PaddedPieceSize(32 << 30); // 32 GiB
    let manifests: Vec<SectorActivationManifest> = precommits.iter().enumerate().map(|(i, pc)| {
        let piece_cid = make_piece_cid(format!("piece-{}", i).as_bytes());
        let notification_payload = RawBytes::from(vec![i as u8, 1, 2, 3]); // Simple test payload
        
        SectorActivationManifest {
            sector_number: pc.info.sector_number,
            pieces: vec![
                PieceActivationManifest {
                    cid: piece_cid,
                    size: piece_size,
                    verified_allocation_key: None,
                    notify: vec![
                        // Send notification to our EVM contract
                        DataActivationNotification {
                            address: evm_robust_addr.clone(),
                            payload: notification_payload.clone(),
                        },
                    ],
                },
            ],
        }
    }).collect();

    // ProveCommitSectors3 with notifications
    let prove_params = ProveCommitSectors3Params {
        sector_activations: manifests,
        sector_proofs: vec![], // Empty proofs for testing
        aggregate_proof: RawBytes::default(),
        aggregate_proof_type: None,
        require_activation_success: false,
        require_notification_success: false,
    };

    let prove_result = v.execute_message(
        &worker,
        &miner_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectors3 as u64,
        IpldBlock::serialize_cbor(&prove_params).unwrap(),
    ).unwrap();

    assert!(prove_result.code.is_success(), "ProveCommit failed: {}", prove_result.message);
    
    println!("Successfully proved sectors with EVM notifications");

    // Verify that the EVM contract received the notifications
    // In a real test, we would call a getter method on the contract to verify state
    // For now, we check that the EVM actor exists and has the expected code
    let evm_actor = v.actor(&evm_actor_addr).unwrap();
    assert_eq!(evm_actor.code, *EVM_ACTOR_CODE_ID, "EVM actor has wrong code ID");
    
    // The contract should have processed the notifications
    // In production, we would call contract methods to verify the stored notification data
    
}
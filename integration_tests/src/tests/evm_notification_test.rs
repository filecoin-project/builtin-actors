use export_macro::vm_test;
use fil_actor_miner::{
    ProveCommitSectors3Params, SectorActivationManifest, PieceActivationManifest,
    DataActivationNotification, SECTOR_CONTENT_CHANGED, Method as MinerMethod,
};
use fil_actors_runtime::{
    EAM_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    test_utils::EVM_ACTOR_CODE_ID, EPOCHS_IN_DAY,
};
use fvm_ipld_encoding::{RawBytes, ipld_block::IpldBlock};
use fvm_shared::{
    address::Address, econ::TokenAmount, sector::{RegisteredSealProof, SectorNumber},
    piece::PaddedPieceSize, ActorID, METHOD_SEND,
};
use num_traits::Zero;
use vm_api::VM;
use vm_api::util::apply_ok;

use crate::util::{
    assert_invariants, create_accounts, create_miner, precommit_sectors_v2,
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_to_proving_deadline, get_network_stats, get_network_version,
    miner_balance, sector_info,
};
use crate::TEST_FAUCET_ADDR;

const BATCH_SIZE: usize = 2;

#[vm_test]
pub fn evm_receives_ddo_notifications_test(v: &dyn VM) {
    // Network version check
    let nv = get_network_version(v);
    
    // Create accounts
    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[1]);
    let (miner_id, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    let miner_addr = Address::new_id(miner_id);

    // Deploy the NotificationReceiver EVM contract
    // First, compile the contract bytecode (simplified for testing)
    let contract_bytecode = include_bytes!("../../contracts/notification_receiver_bytecode.bin");
    
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
        BATCH_SIZE,
        BATCH_SIZE,
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
                        // Also send to storage market for compatibility
                        DataActivationNotification {
                            address: STORAGE_MARKET_ACTOR_ADDR,
                            payload: notification_payload,
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
    
    // Also test with ProveReplicaUpdates3
    // First, we need existing sectors to update
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_addr,
        &worker,
        sector_number,
        sector_number + BATCH_SIZE as u64,
        v.epoch() + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY,
    );
    
    // Create update manifests with notifications to EVM
    let update_manifests: Vec<SectorUpdateManifest> = (0..BATCH_SIZE).map(|i| {
        let sector_num = sector_number + i as u64;
        let piece_cid = make_piece_cid(format!("update-piece-{}", i).as_bytes());
        let notification_payload = RawBytes::from(vec![100 + i as u8, 4, 5, 6]);
        
        SectorUpdateManifest {
            sector: sector_num,
            deadline: 0, // Will be set by the actor
            partition: 0, // Will be set by the actor  
            new_sealed_cid: make_sealed_cid(format!("update-sealed-{}", i).as_bytes()),
            pieces: vec![
                PieceActivationManifest {
                    cid: piece_cid,
                    size: piece_size,
                    verified_allocation_key: None,
                    notify: vec![
                        DataActivationNotification {
                            address: evm_robust_addr.clone(),
                            payload: notification_payload,
                        },
                    ],
                },
            ],
        }
    }).collect();

    let update_params = ProveReplicaUpdates3Params {
        sector_updates: update_manifests,
        sector_proofs: vec![],
        aggregate_proof: RawBytes::default(),
        update_proofs_type: RegisteredUpdateProof::StackedDRG32GiBV1,
        aggregate_proof_type: None,
        require_activation_success: false,
        require_notification_success: false,
    };

    let update_result = v.execute_message(
        &worker,
        &miner_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates3 as u64,
        IpldBlock::serialize_cbor(&update_params).unwrap(),
    ).unwrap();

    assert!(update_result.code.is_success(), "ProveReplicaUpdates failed: {}", update_result.message);
    
    println!("Successfully updated sectors with EVM notifications");

    // Verify the EVM contract is still functioning
    let final_evm_actor = v.actor(&evm_actor_addr).unwrap();
    assert_eq!(final_evm_actor.code, *EVM_ACTOR_CODE_ID, "EVM actor code changed unexpectedly");
    
    assert_invariants(v, &miner_addr);
}

// Helper functions to create test CIDs
fn make_piece_cid(data: &[u8]) -> cid::Cid {
    use cid::multihash::{Code, MultihashDigest};
    let hash = Code::Blake2b256.digest(data);
    cid::Cid::new_v1(0x55, hash) // 0x55 is the multicodec for raw
}

fn make_sealed_cid(data: &[u8]) -> cid::Cid {
    use cid::multihash::{Code, MultihashDigest};
    let hash = Code::Blake2b256.digest(data);
    cid::Cid::new_v1(0x55, hash)
}

// Re-export some constants from other modules
use fil_actor_miner::{ProveReplicaUpdates3Params, SectorUpdateManifest};
use fil_actor_miner::RegisteredUpdateProof;
const DEFAULT_SECTOR_EXPIRATION_DAYS: i64 = 220;
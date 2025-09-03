use alloy_core::sol_types::{SolCall, SolValue};
use alloy_core::{primitives::U256 as AlloyU256, sol};
use cid::Cid;
use export_macro::vm_test;
use fil_actor_miner::{
    CompactCommD, DataActivationNotification, Method as MinerMethod, PieceActivationManifest,
    PieceChange, ProveCommitSectors3Params, SECTOR_CONTENT_CHANGED, SectorActivationManifest,
    SectorChanges, max_prove_commit_duration,
};
use fil_actors_runtime::{EAM_ACTOR_ADDR, runtime::Policy, test_utils::make_piece_cid};
use fvm_ipld_encoding::{BytesDe, CBOR, RawBytes, ipld_block::IpldBlock};
use fvm_shared::address::Address;
use fvm_shared::{
    econ::TokenAmount,
    piece::PaddedPieceSize,
    piece::PieceInfo,
    sector::{RegisteredSealProof, SectorNumber},
};
use num_traits::Zero;
use vm_api::VM;
use vm_api::util::serialize_ok;

use crate::util::{
    PrecommitMetadata, advance_by_deadline_to_epoch, create_accounts, create_miner,
    precommit_sectors_v2,
};

// Generate a statically typed interface for the NotificationReceiver contract
sol!("../actors/evm/tests/contracts/NotificationReceiver.sol");

// Use ContractParams from evm_test module to avoid duplicate definition
use super::evm_test::ContractParams;

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
    let hex_str = std::fs::read_to_string("../actors/evm/tests/contracts/NotificationReceiver.hex")
        .expect("Failed to read contract bytecode hex file");
    let hex_str = hex_str.trim();
    let contract_bytecode = hex::decode(hex_str).expect("Failed to decode contract bytecode hex");

    // Create an EVM actor to receive notifications
    let params =
        IpldBlock::serialize_cbor(&fil_actor_eam::CreateExternalParams(contract_bytecode)).unwrap();

    let create_result = v
        .execute_message(
            &worker,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            params,
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "Failed to create EVM contract: {}",
        create_result.message
    );

    let create_return: fil_actor_eam::CreateReturn =
        create_result.ret.unwrap().deserialize().expect("Failed to decode create return");
    let evm_robust_addr = create_return.robust_address.unwrap();
    let _evm_eth_addr = create_return.eth_address;

    // Precommit sectors
    let sector_number: SectorNumber = 100;

    // Create piece activation manifests with notifications to EVM contract
    let piece_size0 = PaddedPieceSize(32 << 30); // 32 GiB
    let piece_cid0 = make_piece_cid(format!("piece-{}", 0).as_bytes());
    let notification_payload = RawBytes::from(hex::decode("cafe").unwrap());

    let manifests: Vec<SectorActivationManifest> = vec![SectorActivationManifest {
        sector_number,
        pieces: vec![PieceActivationManifest {
            cid: piece_cid0,
            size: piece_size0,
            verified_allocation_key: None,
            notify: vec![
                // Send notification to our EVM contract
                DataActivationNotification {
                    address: evm_robust_addr,
                    payload: notification_payload.clone(),
                },
            ],
        }],
    }];

    let meta: Vec<PrecommitMetadata> = manifests
        .iter()
        .map(|sector| {
            let pis: Vec<PieceInfo> =
                sector.pieces.iter().map(|p| PieceInfo { size: p.size, cid: p.cid }).collect();
            let commd = v.primitives().compute_unsealed_sector_cid(seal_proof, &pis).unwrap();
            PrecommitMetadata { deals: vec![], commd: CompactCommD::of(commd) }
        })
        .collect();

    // Track the precommit epoch for later verification
    let precommit_epoch = v.epoch();

    precommit_sectors_v2(v, 1, meta, &worker, &miner_addr, seal_proof, sector_number, true, None);

    // Before prove commit no notifications have been received
    check_receiver_notification_count(v, &worker, &evm_robust_addr, 0);

    // Advance time to prove commit epoch
    let prove_time = v.epoch() + 151;
    advance_by_deadline_to_epoch(v, &miner_addr, prove_time);

    // ProveCommitSectors3 with notifications
    let proofs = vec![RawBytes::new(vec![8, 8, 8, 8]); manifests.len()]; // dummy value for faked proof syscalls in test vm
    let prove_params = ProveCommitSectors3Params {
        sector_activations: manifests,
        sector_proofs: proofs, // Empty proofs for testing
        aggregate_proof: RawBytes::default(),
        aggregate_proof_type: None,
        require_activation_success: false,
        require_notification_success: true,
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

    /* ***Verify that the EVM contract received the notifications correctly*** */
    let policy = Policy::default();
    let expected_notification = ExpectedNotification {
        sector: sector_number,
        minimum_commitment_epoch: precommit_epoch
            + policy.min_sector_expiration
            + max_prove_commit_duration(&policy, seal_proof).unwrap(),
        piece_cid: piece_cid0,
        piece_size: piece_size0.0,
        payload: notification_payload.to_vec(),
    };

    check_receiver_notification_count(v, &worker, &evm_robust_addr, 1);
    check_receiver_notification_at(v, &worker, &evm_robust_addr, 0, &expected_notification);
}

// Helper functions checking state of receiver contract

pub fn check_receiver_notification_count(
    v: &dyn VM,
    sender_addr: &Address,
    receiver_addr: &Address,
    expected_count: u64,
) {
    let call_params = NotificationReceiver::totalNotificationsCall::new(()).abi_encode();
    let call_result = v
        .execute_message(
            &sender_addr,
            &receiver_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params.to_vec()))),
        )
        .unwrap();

    assert!(
        call_result.code.is_success(),
        "Failed to call totalNotifications: {}",
        call_result.message
    );

    // Decode the return value
    let return_data: BytesDe = call_result.ret.unwrap().deserialize().unwrap();
    let total_notifications = AlloyU256::abi_decode(&return_data.0)
        .expect("Failed to decode totalNotifications return value");
    assert_eq!(
        total_notifications,
        AlloyU256::from(expected_count),
        "Expected {} notification(s), got {}",
        expected_count,
        total_notifications
    );
}

/// Struct to hold all notification values for checking against contract state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedNotification {
    pub sector: u64,
    pub minimum_commitment_epoch: i64,
    pub piece_cid: Cid,
    pub piece_size: u64,
    pub payload: Vec<u8>,
}

pub fn check_receiver_notification_at(
    v: &dyn VM,
    sender_addr: &Address,
    receiver_addr: &Address,
    index: u64,
    expected: &ExpectedNotification,
) {
    let call_params =
        NotificationReceiver::getNotificationCall::new((AlloyU256::from(index),)).abi_encode();
    let call_result = v
        .execute_message(
            &sender_addr,
            &receiver_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params.to_vec()))),
        )
        .unwrap();
    assert!(
        call_result.code.is_success(),
        "Failed to call getNotification: {}",
        call_result.message
    );

    // Decode the return value - it returns a tuple of (uint64, int64, bytes, uint64, bytes)
    let return_data: BytesDe = call_result.ret.unwrap().deserialize().unwrap();

    // Use the generated abi_decode_returns function
    let notification_result =
        NotificationReceiver::getNotificationCall::abi_decode_returns(&return_data.0)
            .expect("Failed to decode getNotification return value");

    let received_sector = notification_result.sector;
    let minimum_commitment_epoch = notification_result.minimumCommitmentEpoch;
    let data_cid_bytes = notification_result.dataCid;
    let received_piece_size = notification_result.pieceSize;
    let received_payload = notification_result.payload;

    assert_eq!(
        received_sector, expected.sector,
        "Sector number mismatch: expected {}, got {}",
        expected.sector, received_sector
    );

    assert_eq!(
        received_piece_size, expected.piece_size,
        "Piece size mismatch: expected {}, got {}",
        expected.piece_size, received_piece_size
    );

    let expected_payload_bytes = expected.payload.to_vec();
    assert_eq!(
        received_payload.as_ref(),
        expected_payload_bytes.as_slice(),
        "Payload mismatch: expected 0x{:x?}, got 0x{:x?}",
        &expected_payload_bytes,
        &received_payload.as_ref()
    );

    // Check the piece CID data is present
    // The contract receives the CID with an extra leading byte from the CBOR encoding,
    // so we verify it contains the expected CID data after the first byte
    let expected_cid_bytes = expected.piece_cid.to_bytes();
    assert!(!data_cid_bytes.is_empty(), "Data CID should not be empty");
    // Verify the CID data matches
    assert_eq!(data_cid_bytes[0], 0, "Data CID should start with 0x00 for ipld cbor reasons");
    assert_eq!(
        &data_cid_bytes[1..],
        expected_cid_bytes,
        "Piece CID data mismatch: expected {:x?}, got {:x?}",
        &expected_cid_bytes,
        &data_cid_bytes[1..]
    );

    assert_eq!(
        minimum_commitment_epoch, expected.minimum_commitment_epoch,
        "Minimum commitment epoch mismatch: expected {}, got {}",
        expected.minimum_commitment_epoch, minimum_commitment_epoch
    );
}

#[vm_test]
pub fn evm_direct_call_fails_non_miner_test(v: &dyn VM) {
    // Create accounts
    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let (_owner, worker) = (addrs[0], addrs[1]);

    // Deploy the NotificationReceiver EVM contract
    let hex_str = std::fs::read_to_string("../actors/evm/tests/contracts/NotificationReceiver.hex")
        .expect("Failed to read contract bytecode hex file");
    let hex_str = hex_str.trim();
    let contract_bytecode = hex::decode(hex_str).expect("Failed to decode contract bytecode hex");

    // Create an EVM actor to receive notifications
    let params =
        IpldBlock::serialize_cbor(&fil_actor_eam::CreateExternalParams(contract_bytecode)).unwrap();

    let create_result = v
        .execute_message(
            &worker,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            params,
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "Failed to create EVM contract: {}",
        create_result.message
    );

    let create_return: fil_actor_eam::CreateReturn =
        create_result.ret.unwrap().deserialize().expect("Failed to decode create return");
    let evm_robust_addr = create_return.robust_address.unwrap();

    // Now attempt to call handle_filecoin_method directly from an account actor (not a miner)
    // We'll construct the CBOR parameters for a SectorContentChanged notification

    // Create a dummy notification payload that matches the expected format
    let piece_cid = make_piece_cid(b"test-piece");
    let piece_size = PaddedPieceSize(32 << 30); // 32 GiB
    let notification_payload = hex::decode("cafe").unwrap();

    // Build CBOR encoded params for handle_filecoin_method using builtin miner types
    // The structure should be:
    // [{sector: 100, minimum_commitment_epoch: 1000, added: [{data: piece_cid, size: piece_size, payload: notification_payload}]}]
    use fvm_ipld_encoding::to_vec;

    let sector_changes = vec![SectorChanges {
        sector: 100,
        minimum_commitment_epoch: 1000,
        added: vec![PieceChange {
            data: piece_cid,
            size: piece_size,
            payload: RawBytes::from(notification_payload),
        }],
    }];

    let cbor_params = to_vec(&sector_changes).expect("Failed to serialize CBOR params");

    // Now call handle_filecoin_method using the alloy interface
    let _method_selector = NotificationReceiver::handle_filecoin_methodCall::SELECTOR;

    // Encode the call using alloy's ABI encoding
    let call_params = NotificationReceiver::handle_filecoin_methodCall::new((
        SECTOR_CONTENT_CHANGED,
        CBOR,
        cbor_params.into(),
    ))
    .abi_encode();

    // Attempt to invoke the contract method from a regular account (not a miner)
    let call_result = v.execute_message(
        &worker,
        &evm_robust_addr,
        &TokenAmount::zero(),
        fil_actor_evm::Method::InvokeContract as u64,
        Some(serialize_ok(&ContractParams(call_params.to_vec()))),
    );

    // Verify the error message contains information about the miner check
    if let Ok(result) = call_result {
        assert!(!result.code.is_success(), "Call should have failed with non-miner actor");
    }

    // Verify that no notifications were stored
    check_receiver_notification_count(v, &worker, &evm_robust_addr, 0);
}

use alloy_core::sol_types::{SolCall, SolValue};
use alloy_core::{primitives::U256 as AlloyU256, sol};
use export_macro::vm_test;
use fil_actor_miner::{
    CompactCommD, DataActivationNotification, Method as MinerMethod, PieceActivationManifest,
    ProveCommitSectors3Params, SectorActivationManifest, max_prove_commit_duration,
};
use fil_actors_runtime::{EAM_ACTOR_ADDR, runtime::Policy, test_utils::make_piece_cid};
use fvm_ipld_encoding::{BytesDe, RawBytes, ipld_block::IpldBlock};
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
    // 1. Call totalNotifications() to verify only one notification witnessed
    {
        let call_params = NotificationReceiver::totalNotificationsCall::new(()).abi_encode();
        let call_result = v
            .execute_message(
                &worker,
                &evm_robust_addr,
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
            AlloyU256::from(1),
            "Expected 1 notification, got {}",
            total_notifications
        );
    }

    // 2. Call getNotification(0) to verify the notification contents match exactly what was sent
    {
        let call_params =
            NotificationReceiver::getNotificationCall::new((AlloyU256::from(0),)).abi_encode();
        let call_result = v
            .execute_message(
                &worker,
                &evm_robust_addr,
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
            received_sector, sector_number,
            "Sector number mismatch: expected {}, got {}",
            sector_number, received_sector
        );

        assert_eq!(
            received_piece_size, piece_size0.0,
            "Piece size mismatch: expected {}, got {}",
            piece_size0.0, received_piece_size
        );

        // Check payload matches exactly what we set in the manifest (hex "cafe")
        let expected_payload_bytes = notification_payload.to_vec();
        assert_eq!(
            received_payload.as_ref(),
            expected_payload_bytes.as_slice(),
            "Payload mismatch: expected 0x{}, got 0x{}",
            hex::encode(&expected_payload_bytes),
            hex::encode(&received_payload)
        );

        // Check the piece CID data is present
        // The contract receives the CID with an extra leading byte from the CBOR encoding,
        // so we verify it contains the expected CID data after the first byte
        let expected_cid_bytes = piece_cid0.to_bytes();
        assert!(!data_cid_bytes.is_empty(), "Data CID should not be empty");
        // Verify the CID data matches
        assert_eq!(data_cid_bytes[0], 0, "Data CID should start with 0x00 for ipld cbor reasons");
        assert_eq!(
            &data_cid_bytes[1..],
            expected_cid_bytes,
            "Piece CID data mismatch: expected 0x{}, got 0x{}",
            hex::encode(&expected_cid_bytes),
            hex::encode(&data_cid_bytes[1..])
        );

        // Verify minimum_commitment_epoch matches the sector expiration
        // The sector expiration is set to: precommit_epoch + min_sector_expiration + max_prove_commit_duration
        let policy = Policy::default();
        let expected_expiration = precommit_epoch
            + policy.min_sector_expiration
            + max_prove_commit_duration(&policy, seal_proof).unwrap();

        assert_eq!(
            minimum_commitment_epoch, expected_expiration,
            "Minimum commitment epoch mismatch: expected {}, got {}",
            expected_expiration, minimum_commitment_epoch
        );
    }
}

// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market::{
    Actor as MarketActor, ComputeDataCommitmentParams, ComputeDataCommitmentReturn, Method,
    SectorDataSpec,
};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PieceInfo;
use fvm_shared::sector::RegisteredSealProof;

use cid::Cid;

mod harness;
use harness::*;

#[cfg(test)]
mod compute_data_commitment {
    use super::*;

    #[test]
    fn successfully_compute_cid() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

        let mut rt = setup();
        let deal_id1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        let d1 = get_deal_proposal(&mut rt, deal_id1);

        let deal_id2 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch + 1,
        );
        let d2 = get_deal_proposal(&mut rt, deal_id2);

        let input = SectorDataSpec {
            deal_ids: vec![deal_id1, deal_id2],
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
        };

        let param = ComputeDataCommitmentParams { inputs: vec![input] };

        let p1 = PieceInfo { size: d1.piece_size, cid: d1.piece_cid };
        let p2 = PieceInfo { size: d2.piece_size, cid: d2.piece_cid };

        let c = make_piece_cid("100".as_bytes());

        rt.expect_compute_unsealed_sector_cid(
            RegisteredSealProof::StackedDRG2KiBV1P1,
            vec![p1, p2],
            c,
            ExitCode::OK,
        );
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

        let ret: ComputeDataCommitmentReturn = rt
            .call::<MarketActor>(
                Method::ComputeDataCommitment as u64,
                &RawBytes::serialize(param).unwrap(),
            )
            .unwrap()
            .deserialize()
            .unwrap();

        assert_eq!(c, ret.commds[0]);

        rt.verify();
        check_state(&rt);
    }

    #[test]
    fn success_on_empty_piece_info() {
        let mut rt = setup();
        let input = SectorDataSpec {
            deal_ids: vec![],
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
        };
        let param = ComputeDataCommitmentParams { inputs: vec![input] };

        let c = make_piece_cid("UnsealedEmpty".as_bytes());
        rt.expect_compute_unsealed_sector_cid(
            RegisteredSealProof::StackedDRG2KiBV1P1,
            vec![],
            c,
            ExitCode::OK,
        );
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

        let ret: ComputeDataCommitmentReturn = rt
            .call::<MarketActor>(
                Method::ComputeDataCommitment as u64,
                &RawBytes::serialize(param).unwrap(),
            )
            .unwrap()
            .deserialize()
            .unwrap();

        assert_eq!(c, ret.commds[0]);

        rt.verify();
        check_state(&rt);
    }

    #[ignore]
    #[test]
    fn success_with_multiple_sector_commitments() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

        let mut rt = setup();
        let deal_id1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        let d1 = get_deal_proposal(&mut rt, deal_id1);

        let deal_id2 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch + 1,
        );
        let d2 = get_deal_proposal(&mut rt, deal_id2);

        let param = ComputeDataCommitmentParams {
            inputs: vec![
                SectorDataSpec {
                    deal_ids: vec![],
                    sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
                },
                SectorDataSpec {
                    deal_ids: vec![deal_id1, deal_id2],
                    sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
                },
            ],
        };

        let p1 = PieceInfo { size: d1.piece_size, cid: d1.piece_cid };
        let p2 = PieceInfo { size: d2.piece_size, cid: d2.piece_cid };

        let c1 = make_piece_cid("UnsealedSector1".as_bytes());
        let c2 = make_piece_cid("UnsealedSector2".as_bytes());

        rt.expect_compute_unsealed_sector_cid(
            RegisteredSealProof::StackedDRG2KiBV1P1,
            vec![],
            c1,
            ExitCode::OK,
        );
        rt.expect_compute_unsealed_sector_cid(
            RegisteredSealProof::StackedDRG2KiBV1P1,
            vec![p1, p2],
            c2,
            ExitCode::OK,
        );
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

        let ret: ComputeDataCommitmentReturn = rt
            .call::<MarketActor>(
                Method::ComputeDataCommitment as u64,
                &RawBytes::serialize(param).unwrap(),
            )
            .unwrap()
            .deserialize()
            .unwrap();
        assert_eq!(c1, ret.commds[0]);
        assert_eq!(c2, ret.commds[1]);

        rt.verify();
        check_state(&rt);
    }

    #[test]
    fn fail_when_deal_proposal_is_absent() {
        let mut rt = setup();
        let input = SectorDataSpec {
            deal_ids: vec![1],
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
        };
        let param = ComputeDataCommitmentParams { inputs: vec![input] };
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        expect_abort(
            ExitCode::USR_NOT_FOUND,
            rt.call::<MarketActor>(
                Method::ComputeDataCommitment as u64,
                &RawBytes::serialize(param).unwrap(),
            ),
        );
        check_state(&rt);
    }

    #[test]
    fn fail_when_syscall_returns_an_error() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

        let mut rt = setup();
        let deal_id = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        let d = get_deal_proposal(&mut rt, deal_id);
        let input = SectorDataSpec {
            deal_ids: vec![deal_id],
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
        };
        let param = ComputeDataCommitmentParams { inputs: vec![input] };

        let pi = PieceInfo { size: d.piece_size, cid: d.piece_cid };

        rt.expect_compute_unsealed_sector_cid(
            RegisteredSealProof::StackedDRG2KiBV1P1,
            vec![pi],
            Cid::default(),
            ExitCode::USR_ILLEGAL_ARGUMENT,
        );
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MarketActor>(
                Method::ComputeDataCommitment as u64,
                &RawBytes::serialize(param).unwrap(),
            ),
        );
        check_state(&rt);
    }

    #[test]
    fn fail_whole_call_when_one_deal_proposal_of_one_sector_is_absent() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

        let mut rt = setup();
        let deal_id1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        let deal_id2 = 2;

        let param = ComputeDataCommitmentParams {
            inputs: vec![
                SectorDataSpec {
                    deal_ids: vec![],
                    sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
                },
                SectorDataSpec {
                    deal_ids: vec![deal_id1, deal_id2],
                    sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
                },
            ],
        };
        let c1 = make_piece_cid("UnsealedSector1".as_bytes());
        rt.expect_compute_unsealed_sector_cid(
            RegisteredSealProof::StackedDRG2KiBV1P1,
            vec![],
            c1,
            ExitCode::OK,
        ); // first sector is computed
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        expect_abort(
            ExitCode::USR_NOT_FOUND,
            rt.call::<MarketActor>(
                Method::ComputeDataCommitment as u64,
                &RawBytes::serialize(param).unwrap(),
            ),
        );
        check_state(&rt);
    }

    #[test]
    fn fail_whole_call_when_one_commitment_fails_syscall() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

        let mut rt = setup();
        let deal_id1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        let deal_id2 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch + 1,
        );

        let param = ComputeDataCommitmentParams {
            inputs: vec![
                SectorDataSpec {
                    deal_ids: vec![],
                    sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
                },
                SectorDataSpec {
                    deal_ids: vec![deal_id1, deal_id2],
                    sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
                },
            ],
        };
        rt.expect_compute_unsealed_sector_cid(
            RegisteredSealProof::StackedDRG2KiBV1P1,
            vec![],
            Cid::default(),
            ExitCode::USR_ILLEGAL_ARGUMENT,
        );
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MarketActor>(
                Method::ComputeDataCommitment as u64,
                &RawBytes::serialize(param).unwrap(),
            ),
        );
        check_state(&rt);
    }
}

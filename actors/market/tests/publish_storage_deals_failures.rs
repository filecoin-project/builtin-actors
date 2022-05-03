// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market::policy::deal_provider_collateral_bounds;
use fil_actor_market::DealProposal;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::StoragePower;
use fvm_shared::TOTAL_FILECOIN;

use anyhow::anyhow;
use cid::Cid;
use num_traits::FromPrimitive;

mod harness;
use harness::*;

#[cfg(test)]
mod publish_storage_deals_failures {
    use super::*;

    #[test]
    fn deal_end_after_deal_start() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.start_epoch = 10;
            d.end_epoch = 9;
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn current_epoch_greater_than_start_epoch() {
        let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
            d.start_epoch = rt.epoch - 1;
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn deal_duration_greater_than_max_deal_duration() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.start_epoch = ChainEpoch::from(10);
            d.end_epoch = d.start_epoch + (540 * EPOCHS_IN_DAY) + 1
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn negative_price_per_epoch() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.storage_price_per_epoch = TokenAmount::from(-1);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn price_per_epoch_greater_than_total_filecoin() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.storage_price_per_epoch = TOTAL_FILECOIN.clone() + 1;
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn negative_provider_collateral() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.provider_collateral = TokenAmount::from(-1);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn provider_collateral_greater_than_max_collateral() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.provider_collateral = TOTAL_FILECOIN.clone() + 1;
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn provider_collateral_less_than_bound() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            let power = StoragePower::from_i128(1 << 50).unwrap();
            let (provider_min, _) = deal_provider_collateral_bounds(
                &Policy::default(),
                PaddedPieceSize(2048),
                &BigInt::from(0u8),
                &BigInt::from(0u8),
                &power,
            );
            d.provider_collateral = provider_min - 1;
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn negative_client_collateral() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.client_collateral = TokenAmount::from(-1);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn client_collateral_greater_than_max_collateral() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.client_collateral = TOTAL_FILECOIN.clone() + 1;
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn client_does_not_have_enough_balance_for_collateral() {
        let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
            add_participant_funds(rt, CLIENT_ADDR, d.client_balance_requirement() - 1);
            add_provider_funds(rt, d.provider_collateral.clone(), &MinerAddresses::default());
        };
        assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn provider_does_not_have_enough_balance_for_collateral() {
        let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
            add_participant_funds(rt, CLIENT_ADDR, d.client_balance_requirement());
            add_provider_funds(rt, d.provider_collateral.clone() - 1, &MinerAddresses::default());
        };
        assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn client_address_does_not_exist() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.client = Address::new_id(1);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn unable_to_resolve_client_address() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.client = new_bls_addr(1);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn signature_is_invalid() {
        let f = |_rt: &mut MockRuntime, _d: &mut DealProposal| {};
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Err(anyhow!("error")));
    }

    #[test]
    fn no_entry_for_client_in_locked_balance_table() {
        let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
            add_provider_funds(rt, d.provider_collateral.clone(), &MinerAddresses::default());
        };
        assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn no_entry_for_provider_in_locked_balance_table() {
        let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
            add_participant_funds(rt, CLIENT_ADDR, d.client_balance_requirement());
        };
        assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn bad_piece_cid() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.piece_cid = Cid::default();
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn zero_piece_size() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.piece_size = PaddedPieceSize(0u64);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn piece_size_less_than_128_bytes() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.piece_size = PaddedPieceSize(64u64);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }

    #[test]
    fn piece_size_is_not_a_power_of_2() {
        let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
            d.piece_size = PaddedPieceSize(254u64);
        };
        assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, Ok(()));
    }
}

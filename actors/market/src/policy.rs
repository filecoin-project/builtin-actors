// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cmp::max;

use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::DealWeight;
use fvm_shared::bigint::Integer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::StoragePower;
use fvm_shared::TOTAL_FILECOIN;
use num_traits::Zero;

use super::deal::DealProposal;

/// Maximum length of a deal label.
pub(super) const DEAL_MAX_LABEL_SIZE: usize = 256;

/// Bounds (inclusive) on deal duration.
pub(super) fn deal_duration_bounds(_size: PaddedPieceSize) -> (ChainEpoch, ChainEpoch) {
    (180 * EPOCHS_IN_DAY, 540 * EPOCHS_IN_DAY)
}

pub(super) fn deal_price_per_epoch_bounds(
    _size: PaddedPieceSize,
    _duration: ChainEpoch,
) -> (TokenAmount, &'static TokenAmount) {
    (0.into(), &TOTAL_FILECOIN)
}

pub(super) fn deal_provider_collateral_bounds(
    policy: &Policy,
    size: PaddedPieceSize,
    network_raw_power: &StoragePower,
    baseline_power: &StoragePower,
    network_circulating_supply: &TokenAmount,
) -> (TokenAmount, TokenAmount) {
    // minimumProviderCollateral = ProviderCollateralSupplyTarget * normalizedCirculatingSupply
    // normalizedCirculatingSupply = networkCirculatingSupply * dealPowerShare
    // dealPowerShare = dealRawPower / max(BaselinePower(t), NetworkRawPower(t), dealRawPower)

    let lock_target_num = network_circulating_supply * policy.prov_collateral_percent_supply_num;
    let power_share_num = TokenAmount::from(size.0);
    let power_share_denom = max(max(network_raw_power, baseline_power), &power_share_num).clone();

    let num: TokenAmount = power_share_num * lock_target_num;
    let denom: TokenAmount = power_share_denom * policy.prov_collateral_percent_supply_denom;
    ((num.div_floor(&denom)), TOTAL_FILECOIN.clone())
}

pub(super) fn deal_client_collateral_bounds(
    _: PaddedPieceSize,
    _: ChainEpoch,
) -> (TokenAmount, TokenAmount) {
    (TokenAmount::zero(), TOTAL_FILECOIN.clone()) // PARAM_FINISH
}

/// Penalty to provider deal collateral if the deadline expires before sector commitment.
pub(super) fn collateral_penalty_for_deal_activation_missed(
    provider_collateral: TokenAmount,
) -> TokenAmount {
    provider_collateral
}

/// Computes the weight for a deal proposal, which is a function of its size and duration.
pub(super) fn deal_weight(proposal: &DealProposal) -> DealWeight {
    let deal_duration = DealWeight::from(proposal.duration());
    deal_duration * proposal.piece_size.0
}

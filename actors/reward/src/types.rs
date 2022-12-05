// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

// #[derive(Debug, PartialEq, Eq, Clone, Default)]
// #[serde(transparent)]
// pub struct ConstructorParams {
//     #[serde(with = "bigint_ser")]
//     pub curr_realized_power: Option<StoragePower>,
// }
//
// #[derive(Debug, PartialEq, Eq, Clone, Default)]
// #[serde(transparent)]
// pub struct UpdateNetworkKPIParams {
//     #[serde(with = "bigint_ser")]
//     pub curr_realized_power: Option<StoragePower>,
// }

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct AwardBlockRewardParams {
    pub miner: Address,
    pub penalty: TokenAmount,
    pub gas_reward: TokenAmount,
    pub win_count: i64,
}

pub use fvm_shared::reward::ThisEpochRewardReturn;

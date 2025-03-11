// Copyright 2021-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::tuple::*;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use num_traits::Zero;

pub mod math;
pub mod smooth;

pub use smooth::FilterEstimate;

use crate::runtime::Runtime;
use crate::{deserialize_block, ActorError};

use super::{extract_send_result, REWARD_ACTOR_ADDR};

pub const THIS_EPOCH_REWARD_METHOD: u64 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ThisEpochRewardReturn {
    // * Removed this_epoch_reward in v2
    pub this_epoch_reward_smoothed: FilterEstimate,
    #[serde(with = "bigint_ser")]
    pub this_epoch_baseline_power: StoragePower,
}

/// Requests the current epoch target block reward from the reward actor.
/// return value includes reward, smoothed estimate of reward, and baseline power
pub fn request_current_epoch_block_reward(
    rt: &impl Runtime,
) -> Result<ThisEpochRewardReturn, ActorError> {
    deserialize_block(
        extract_send_result(rt.send_simple(
            &REWARD_ACTOR_ADDR,
            THIS_EPOCH_REWARD_METHOD,
            Default::default(),
            TokenAmount::zero(),
        ))
        .map_err(|e| e.wrap("failed to check epoch baseline power"))?,
    )
}

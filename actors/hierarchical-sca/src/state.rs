// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_hamt::BytesKey;
use fvm_shared::blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::bigint::{bigint_ser, BigInt};
use fvm_shared::encoding::tuple::*;
use fvm_shared::encoding::{Cbor};
use lazy_static::lazy_static;
use integer_encoding::VarInt;

/// Storage power actor state
#[derive(Default, Serialize_tuple, Deserialize_tuple)]
pub struct State {
    pub first_cron_epoch: ChainEpoch,
}

pub enum Status {
    Active,
    Inactive,
    Killed,
}

lazy_static! {
    /// TODO: Comment
    static ref MIN_SUBNET_STAKE: BigInt = BigInt::from(10_i64.pow(18));
}

impl Cbor for State {}

impl State {
    pub fn new<BS: Blockstore>(store: &BS) -> anyhow::Result<State> {
        Ok(State {first_cron_epoch: 0})
    }

}

pub(super) fn epoch_key(e: ChainEpoch) -> BytesKey {
    let bz = e.encode_var_vec();
    bz.into()
}


#[cfg(test)]
mod test {
}

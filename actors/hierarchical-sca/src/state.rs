// Copyright: ConsensusLab
//
use cid::Cid;
use anyhow::{anyhow, /*Context*/};
use fil_actors_runtime::{
    // actor_error,
    Array, make_empty_map, 
    // make_map_with_root, make_map_with_root_and_bitwidth,
    // ActorDowncast, ActorError, Map, Multimap,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{Cbor, /*RawBytes*/};
use fvm_shared::econ::TokenAmount;
use fvm_ipld_hamt::BytesKey;
use fvm_shared::bigint::{bigint_ser, BigInt};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::HAMT_BIT_WIDTH;
use lazy_static::lazy_static;
use integer_encoding::VarInt;

use super::types::*;

/// Storage power actor state
#[derive(Default, Serialize_tuple, Deserialize_tuple)]
pub struct State {
    pub network_name: String, 
    pub total_subnets: u64,
    #[serde(with = "bigint_ser")]
    pub min_stake: TokenAmount, 
    pub subnets: Cid, // HAMT[cid.Cid]Subnet
    pub check_period: ChainEpoch,
    pub checkpoints: Cid, // HAMT[epoch]Checkpoint
    pub check_msg_registry: Cid, // HAMT[cid]CrossMsgs
    pub nonce: u64,
    pub bottom_up_nonce: u64,
    pub bottom_up_msg_meta: Cid,// AMT[schema.CrossMsgs] from child subnets to apply.
    pub applied_bottomup_nonce: u64,
    pub applied_topdown_nonce: u64,
    pub atomic_exec_registry: Cid, // HAMT[cid]AtomicExec
}

pub enum Status {
    Active,
    Inactive,
    Killed,
}

lazy_static! {
    /// TODO: Comment
    // static ref MIN_SUBNET_STAKE: BigInt = BigInt::from(10_i64.pow(18));
    static ref MIN_SUBNET_STAKE: BigInt = TokenAmount::from(10_i64.pow(18));
}

impl Cbor for State {}

impl State {
    pub fn new<BS: Blockstore>(store: &BS, params: ConstructorParams) -> anyhow::Result<State> {
        let empty_sn_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| anyhow!("Failed to create empty map: {}", e))?;
        let empty_checkpoint_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| anyhow!("Failed to create empty map: {}", e))?;
        let empty_meta_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| anyhow!("Failed to create empty map: {}", e))?;
        let empty_atomic_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| anyhow!("Failed to create empty map: {}", e))?;
        let empty_bottomup_array =
            Array::<(), BS>::new_with_bit_width(store, CROSSMSG_AMT_BITWIDTH)
                .flush()
                .map_err(|e| anyhow!("Failed to create empty proposals array: {}", e))?;
        Ok(State {
            network_name: params.network_name,
            min_stake: MIN_SUBNET_STAKE.clone(),
            check_period: match params.checkpoint_period > DEFAULT_CHECKPOINT_PERIOD {
                true => params.checkpoint_period, 
                false => DEFAULT_CHECKPOINT_PERIOD,
            },
            subnets: empty_sn_map,
            checkpoints: empty_checkpoint_map,
            check_msg_registry: empty_meta_map,
            bottom_up_msg_meta: empty_bottomup_array,
            applied_bottomup_nonce: MAX_NONCE,
            atomic_exec_registry: empty_atomic_map,
            ..Default::default()
        })
    }

}

#[cfg(test)]
mod test {
}

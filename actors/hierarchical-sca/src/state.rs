// Copyright: ConsensusLab
//
use anyhow::anyhow;
use cid::Cid;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{
    make_empty_map, make_map_with_root_and_bitwidth, ActorDowncast, ActorError, Array, Map,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::bigint::{bigint_ser, BigInt};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::HAMT_BIT_WIDTH;
use lazy_static::lazy_static;
use num_traits::Zero;
use std::str::FromStr;

use super::checkpoint::*;
use super::subnet::*;
use super::types::*;

/// Storage power actor state
#[derive(Default, Serialize_tuple, Deserialize_tuple)]
pub struct State {
    pub network_name: SubnetID,
    pub total_subnets: u64,
    #[serde(with = "bigint_ser")]
    pub min_stake: TokenAmount,
    pub subnets: Cid, // HAMT[cid.Cid]Subnet
    pub check_period: ChainEpoch,
    pub checkpoints: Cid,        // HAMT[epoch]Checkpoint
    pub check_msg_registry: Cid, // HAMT[cid]CrossMsgs
    pub nonce: u64,
    pub bottom_up_nonce: u64,
    pub bottom_up_msg_meta: Cid, // AMT[schema.CrossMsgs] from child subnets to apply.
    pub applied_bottomup_nonce: u64,
    pub applied_topdown_nonce: u64,
    pub atomic_exec_registry: Cid, // HAMT[cid]AtomicExec
}

lazy_static! {
    static ref MIN_SUBNET_COLLATERAL: BigInt = TokenAmount::from(MIN_COLLATERAL_AMOUNT);
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
                .map_err(|e| anyhow!("Failed to create empty messages array: {}", e))?;
        Ok(State {
            network_name: SubnetID::from_str(&params.network_name)?,
            min_stake: MIN_SUBNET_COLLATERAL.clone(),
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

    pub fn get_subnet<BS: Blockstore>(
        &self,
        store: &BS,
        id: &SubnetID,
    ) -> anyhow::Result<Option<Subnet>> {
        let subnets =
            make_map_with_root_and_bitwidth::<_, Subnet>(&self.subnets, store, HAMT_BIT_WIDTH)
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnets")
                })?;

        let subnet = get_subnet(&subnets, id)?;
        Ok(subnet.cloned())
    }

    pub fn register_subnet<BS, RT>(&mut self, rt: &RT, id: &SubnetID) -> anyhow::Result<()>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let val = rt.message().value_received();
        if val < self.min_stake {
            return Err(anyhow!("call to register doesn't include enough funds"));
        }
        let mut subnets =
            make_map_with_root_and_bitwidth::<_, Subnet>(&self.subnets, rt.store(), HAMT_BIT_WIDTH)
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnets")
                })?;

        let empty_topdown_array =
            Array::<(), BS>::new_with_bit_width(rt.store(), CROSSMSG_AMT_BITWIDTH)
                .flush()
                .map_err(|e| anyhow!("Failed to create empty messages array: {}", e))?;

        let subnet = Subnet {
            id: id.clone(),
            stake: val,
            top_down_msgs: empty_topdown_array,
            circ_supply: TokenAmount::zero(),
            status: Status::Active,
            nonce: 0,
            prev_checkpoint: Checkpoint::default(),
        };
        set_subnet(&mut subnets, &id, subnet)?;
        self.subnets = subnets.flush().map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush subnets")
        })?;
        self.total_subnets += 1;
        Ok(())
    }
}

fn get_subnet<'m, BS: Blockstore>(
    claims: &'m Map<BS, Subnet>,
    id: &SubnetID,
) -> anyhow::Result<Option<&'m Subnet>> {
    claims
        .get(&id.to_bytes())
        .map_err(|e| e.downcast_wrap(format!("failed to get subnet for id {}", id)))
}

pub fn set_subnet<BS: Blockstore>(
    subnets: &mut Map<BS, Subnet>,
    id: &SubnetID,
    subnet: Subnet,
) -> anyhow::Result<()> {
    subnets
        .set(id.to_bytes().into(), subnet)
        .map_err(|e| e.downcast_wrap(format!("failed to set subnet for id {}", id)))?;
    Ok(())
}

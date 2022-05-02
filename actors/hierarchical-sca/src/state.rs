// Copyright: ConsensusLab
//
use anyhow::anyhow;
use cid::Cid;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{
    make_empty_map, make_map_with_root_and_bitwidth, ActorDowncast, Array, Map,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_ipld_hamt::BytesKey;
use fvm_shared::bigint::{bigint_ser, BigInt};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::HAMT_BIT_WIDTH;
use lazy_static::lazy_static;
use num_traits::Zero;
use std::collections::HashMap;
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
    pub bottomup_nonce: u64,
    pub bottomup_msg_meta: Cid, // AMT[CrossMsgMeta] from child subnets to apply.
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
            bottomup_msg_meta: empty_bottomup_array,
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

    pub(crate) fn register_subnet<BS, RT>(&mut self, rt: &RT, id: &SubnetID) -> anyhow::Result<()>
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

    pub(crate) fn rm_subnet<BS: Blockstore>(
        &mut self,
        store: &BS,
        id: &SubnetID,
    ) -> anyhow::Result<()> {
        let mut subnets =
            make_map_with_root_and_bitwidth::<_, Subnet>(&self.subnets, store, HAMT_BIT_WIDTH)
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnets")
                })?;
        subnets
            .delete(&id.to_bytes())
            .map_err(|e| e.downcast_wrap(format!("failed to delete subnet for id {}", id)))?;
        self.subnets = subnets.flush().map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush subnets")
        })?;
        self.total_subnets -= 1;
        Ok(())
    }

    pub(crate) fn flush_subnet<BS: Blockstore>(
        &mut self,
        store: &BS,
        sub: &Subnet,
    ) -> anyhow::Result<()> {
        let mut subnets =
            make_map_with_root_and_bitwidth::<_, Subnet>(&self.subnets, store, HAMT_BIT_WIDTH)
                .map_err(|e| anyhow!("error loading subnets: {}", e))?;
        set_subnet(&mut subnets, &sub.id, sub.clone())?;
        self.subnets = subnets.flush().map_err(|e| anyhow!("error flushing subnets: {}", e))?;
        Ok(())
    }

    pub(crate) fn flush_checkpoint<BS: Blockstore>(
        &mut self,
        store: &BS,
        ch: &Checkpoint,
    ) -> anyhow::Result<()> {
        let mut checkpoints = make_map_with_root_and_bitwidth::<_, Checkpoint>(
            &self.checkpoints,
            store,
            HAMT_BIT_WIDTH,
        )
        .map_err(|e| anyhow!("error loading checkpoints: {}", e))?;
        set_checkpoint(&mut checkpoints, ch.clone())?;
        self.checkpoints =
            checkpoints.flush().map_err(|e| anyhow!("error flushing checkpoints: {}", e))?;
        Ok(())
    }

    pub fn get_window_checkpoint<'m, BS: Blockstore>(
        &self,
        store: &'m BS,
        epoch: ChainEpoch,
    ) -> anyhow::Result<Checkpoint> {
        if epoch < 0 {
            return Err(anyhow!("epoch can't be negative"));
        }
        let ch_epoch = checkpoint_epoch(epoch, self.check_period);
        let checkpoints = make_map_with_root_and_bitwidth::<_, Checkpoint>(
            &self.checkpoints,
            store,
            HAMT_BIT_WIDTH,
        )
        .map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load checkpoints")
        })?;

        let out_ch = match get_checkpoint(&checkpoints, &ch_epoch)? {
            Some(ch) => ch.clone(),
            None => Checkpoint::new(self.network_name.clone(), ch_epoch),
        };

        Ok(out_ch)
    }

    pub(crate) fn apply_check_msgs<'m, BS: Blockstore>(
        &mut self,
        store: &'m BS,
        sub: &mut Subnet,
        commit: &'m Checkpoint,
    ) -> anyhow::Result<(TokenAmount, HashMap<SubnetID, Vec<&'m CrossMsgMeta>>)> {
        let mut burn_val = TokenAmount::zero();
        let mut aux: HashMap<SubnetID, Vec<&CrossMsgMeta>> = HashMap::new();

        // if cross-msgs directed to current network
        for mm in commit.cross_msgs() {
            if mm.to == self.network_name {
                self.store_bottomup_msg(&store, mm)
                    .map_err(|e| anyhow!("error storing bottomup msg: {}", e))?;
            } else {
                // if we are not the parent, someone is trying to forge messages
                if mm.from.parent().unwrap_or_else(|| SubnetID::default()) != self.network_name {
                    continue;
                }
                let meta = aux.entry(mm.to.clone()).or_insert(vec![mm]);
                (*meta).push(mm);
            }
            burn_val += mm.value.clone();
            self.release_circ_supply(store, sub, &mm.from, &mm.value)?;
        }

        Ok((burn_val, aux))
    }

    pub(crate) fn agg_child_msgmeta<BS: Blockstore>(
        &mut self,
        store: &BS,
        ch: &mut Checkpoint,
        aux: HashMap<SubnetID, Vec<&CrossMsgMeta>>,
    ) -> anyhow::Result<()> {
        for (to, mm) in aux.into_iter() {
            // aggregate values inside msgmeta
            let value = mm.iter().fold(TokenAmount::zero(), |acc, x| acc + x.value.clone());
            let metas = mm.into_iter().cloned().collect();

            match ch.crossmsg_meta_index(&self.network_name, &to) {
                Some(index) => {
                    let msgmeta = &mut ch.data.cross_msgs[index];
                    let prev_cid = msgmeta.msgs_cid;
                    let m_cid = self.append_metas_to_meta(store, &prev_cid, metas)?;
                    msgmeta.msgs_cid = m_cid;
                    msgmeta.value += value;
                }
                None => {
                    let mut msgmeta = CrossMsgMeta::new(&self.network_name, &to);
                    let mut n_mt = CrossMsgs::new();
                    n_mt.metas = metas;
                    let mut cross_reg = make_map_with_root_and_bitwidth::<_, CrossMsgs>(
                        &self.check_msg_registry,
                        store,
                        HAMT_BIT_WIDTH,
                    )?;

                    let meta_cid = put_msgmeta(&mut cross_reg, n_mt);
                    msgmeta.value += value.clone();
                    msgmeta.msgs_cid = meta_cid?;
                    ch.append_msgmeta(msgmeta)?;
                }
            };
        }

        Ok(())
    }

    pub(crate) fn append_metas_to_meta<BS: Blockstore>(
        &mut self,
        store: &BS,
        meta_cid: &Cid,
        metas: Vec<CrossMsgMeta>,
    ) -> anyhow::Result<Cid> {
        let mut cross_reg = make_map_with_root_and_bitwidth::<_, CrossMsgs>(
            &self.check_msg_registry,
            store,
            HAMT_BIT_WIDTH,
        )?;

        // get previous meta stored
        let mut prev_meta = match cross_reg.get(&meta_cid.to_bytes())? {
            Some(m) => m.clone(),
            None => return Err(anyhow!("no msgmeta found for cid")),
        };

        prev_meta.add_metas(metas)?;

        // if the cid hasn't changed
        let cid = prev_meta.cid()?;
        if &cid == meta_cid {
            return Ok(cid);
        }
        // else we persist the new msgmeta
        self.put_delete_flush_meta(&mut cross_reg, meta_cid, prev_meta)
    }

    pub(crate) fn put_delete_flush_meta<BS: Blockstore>(
        &mut self,
        registry: &mut Map<BS, CrossMsgs>,
        prev_cid: &Cid,
        meta: CrossMsgs,
    ) -> anyhow::Result<Cid> {
        // add new meta
        let m_cid = put_msgmeta(registry, meta)?;
        // remove the previous one
        registry.delete(&prev_cid.to_bytes())?;
        // flush
        self.check_msg_registry =
            registry.flush().map_err(|e| anyhow!("error flushing crossmsg registry: {}", e))?;

        Ok(m_cid)
    }

    pub(crate) fn release_circ_supply<BS: Blockstore>(
        &mut self,
        store: &BS,
        curr: &mut Subnet,
        id: &SubnetID,
        val: &TokenAmount,
    ) -> anyhow::Result<()> {
        // if current subnet, we don't need to get the
        // subnet again
        if curr.id == *id {
            curr.release_supply(val)?;
            return Ok(());
        }

        let sub =
            self.get_subnet(store, id).map_err(|e| anyhow!("failed to load subnet: {}", e))?;
        match sub {
            Some(mut sub) => {
                sub.release_supply(val)?;
                self.flush_subnet(store, &sub)
            }
            None => return Err(anyhow!("subnet with id {} not registered", id)),
        }?;
        Ok(())
    }

    pub(crate) fn store_bottomup_msg<BS: Blockstore>(
        &mut self,
        store: &BS,
        meta: &CrossMsgMeta,
    ) -> anyhow::Result<()> {
        let mut crossmsgs = CrossMsgMetaArray::load(&self.bottomup_msg_meta, store)
            .map_err(|e| anyhow!("failed to load crossmsg meta array: {}", e))?;

        let mut new_meta = meta.clone();
        new_meta.nonce = self.bottomup_nonce;
        crossmsgs
            .set(new_meta.nonce, new_meta)
            .map_err(|e| anyhow!("failed to load crossmsg meta array: {}", e))?;

        self.bottomup_nonce += 1;
        Ok(())
    }
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

fn get_subnet<'m, BS: Blockstore>(
    subnets: &'m Map<BS, Subnet>,
    id: &SubnetID,
) -> anyhow::Result<Option<&'m Subnet>> {
    subnets
        .get(&id.to_bytes())
        .map_err(|e| e.downcast_wrap(format!("failed to get subnet for id {}", id)))
}

pub fn set_checkpoint<BS: Blockstore>(
    checkpoints: &mut Map<BS, Checkpoint>,
    ch: Checkpoint,
) -> anyhow::Result<()> {
    let epoch = ch.epoch();
    checkpoints
        .set(BytesKey::from(epoch.to_ne_bytes().to_vec()), ch)
        .map_err(|e| e.downcast_wrap(format!("failed to set checkpoint for epoch {}", epoch)))?;
    Ok(())
}

fn get_checkpoint<'m, BS: Blockstore>(
    checkpoints: &'m Map<BS, Checkpoint>,
    epoch: &ChainEpoch,
) -> anyhow::Result<Option<&'m Checkpoint>> {
    checkpoints
        .get(&BytesKey::from(epoch.to_ne_bytes().to_vec()))
        .map_err(|e| e.downcast_wrap(format!("failed to get checkpoint for id {}", epoch)))
}

fn put_msgmeta<BS: Blockstore>(
    registry: &mut Map<BS, CrossMsgs>,
    metas: CrossMsgs,
) -> anyhow::Result<Cid> {
    let m_cid = metas.cid()?;
    registry
        .set(m_cid.to_bytes().into(), metas)
        .map_err(|e| e.downcast_wrap(format!("failed to set crossmsg meta for cid {}", m_cid)))?;
    Ok(m_cid)
}

fn get_msgmeta<'m, BS: Blockstore>(
    registry: &'m Map<BS, CrossMsgs>,
    cid: &Cid,
) -> anyhow::Result<Option<&'m CrossMsgs>> {
    registry
        .get(&cid.to_bytes())
        .map_err(|e| e.downcast_wrap(format!("failed to get crossmsgs for cid {}", cid)))
}

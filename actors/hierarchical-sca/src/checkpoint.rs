use super::subnet::SubnetID;
use crate::StorableMsg;
use anyhow::anyhow;
use cid::multihash::Code;
use cid::Cid;
use fil_actors_runtime::Array;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_encoding::{serde_bytes, Cbor};
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

#[derive(Default, PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct Checkpoint {
    pub data: CheckData,
    #[serde(with = "serde_bytes")]
    sig: Vec<u8>,
}
impl Cbor for Checkpoint {}

impl Checkpoint {
    pub fn new(id: SubnetID, epoch: ChainEpoch) -> Self {
        Self {
            data: CheckData { source: id, epoch: epoch, ..Default::default() },
            ..Default::default()
        }
    }
    pub fn source(&self) -> &SubnetID {
        &self.data.source
    }

    pub fn prev_check(&self) -> Cid {
        self.data.prev_check
    }

    pub fn cross_msgs(&mut self) -> &Vec<CrossMsgMeta> {
        &self.data.cross_msgs
    }

    pub fn crossmsg_meta(&self, from: &SubnetID, to: &SubnetID) -> Option<&mut CrossMsgMeta> {
        // Some(self.data.cross_msgs.iter().find(|m| from == &m.from && to == &m.to)?.clone())
        let out = self.data.cross_msgs.iter().find(|m| from == &m.from && to == &m.to)?;
        Some(&mut out)
    }
}

#[derive(Default, PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct CheckData {
    source: SubnetID,
    #[serde(with = "serde_bytes")]
    tip_set: Vec<u8>,
    epoch: ChainEpoch,
    prev_check: Cid,
    childs: Vec<ChildCheck>,
    cross_msgs: Vec<CrossMsgMeta>,
}
impl Cbor for CheckData {}

#[derive(PartialEq, Eq, Clone, Debug, Default, Serialize_tuple, Deserialize_tuple)]
pub struct CrossMsgMeta {
    pub from: SubnetID,
    pub to: SubnetID,
    pub msgs_cid: Cid,
    pub nonce: u64,
    #[serde(with = "bigint_ser")]
    pub value: TokenAmount,
}
impl Cbor for CrossMsgMeta {}

impl CrossMsgMeta {
    pub fn new(from: &SubnetID, to: &SubnetID) -> Self {
        Self { from: from.clone(), to: to.clone(), ..Default::default() }
    }

    pub fn set_nonce(&mut self, nonce: u64) {
        self.nonce = nonce;
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ChildCheck {
    source: SubnetID,
    checks: Vec<Cid>,
}
impl Cbor for ChildCheck {}

#[derive(PartialEq, Eq, Clone, Debug, Default, Serialize_tuple, Deserialize_tuple)]
pub struct CrossMsgs {
    pub msgs: Vec<StorableMsg>,
    pub metas: Vec<CrossMsgMeta>,
}
impl Cbor for CrossMsgs {}

#[derive(PartialEq, Eq, Clone, Debug, Default, Serialize_tuple, Deserialize_tuple)]
pub struct MetaTag {
    pub msgs_cid: Cid,
    pub meta_cid: Cid,
}
impl Cbor for MetaTag {}

impl CrossMsgs {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn cid(&self) -> anyhow::Result<Cid> {
        let store = MemoryBlockstore::new();
        let mut msgs_array = Array::new(&store);
        msgs_array.batch_set(self.msgs.clone())?;
        let msgs_cid = msgs_array
            .flush()
            .map_err(|e| anyhow!("Failed to create empty messages array: {}", e))?;

        let mut meta_array = Array::new(&store);
        meta_array.batch_set(self.msgs.clone())?;
        let meta_cid = meta_array
            .flush()
            .map_err(|e| anyhow!("Failed to create empty messages array: {}", e))?;

        Ok(store.put_cbor(&MetaTag { msgs_cid: msgs_cid, meta_cid: meta_cid }, Code::Blake2b256)?)
    }
}

/// CheckpointEpoch returns the epoch of the next checkpoint
/// that needs to be signed
///
/// Return the template of the checkpoint template that has been
/// frozen and that is ready for signing and commitment in the
/// current window.
pub fn checkpoint_epoch(epoch: ChainEpoch, period: ChainEpoch) -> ChainEpoch {
    (epoch / period) * period
}

/// WindowEpoch returns the epoch of the active checkpoint window
///
/// Determines the epoch to which new checkpoints and xshard transactions need
/// to be assigned.
pub fn window_epoch(epoch: ChainEpoch, period: ChainEpoch) -> ChainEpoch {
    let ind = epoch / period;
    period * (ind + 1)
}

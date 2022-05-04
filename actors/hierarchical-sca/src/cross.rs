use anyhow::anyhow;
use cid::multihash::Code;
use cid::Cid;
use fil_actors_runtime::Array;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_ipld_encoding::{CborStore, RawBytes};
use fvm_shared::address::{Address, SubnetID};
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;

use crate::checkpoint::CrossMsgMeta;

/// StorableMsg stores all the relevant information required
/// to execute cross-messages.
///
/// We follow this approach because we can't directly store types.Message
/// as we did in the actor's Go counter-part. Instead we just persist the
/// information required to create the cross-messages and execute in the
/// corresponding node implementation.
#[derive(PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct StorableMsg {
    pub from: Address,
    pub to: Address,
    pub method: MethodNum,
    pub params: RawBytes,
    #[serde(with = "bigint_ser")]
    pub value: TokenAmount,
    pub nonce: u64,
}
impl Cbor for StorableMsg {}

impl StorableMsg {
    pub fn new_fund_msg(
        sub_id: &SubnetID,
        sig_addr: &Address,
        value: TokenAmount,
    ) -> anyhow::Result<Self> {
        let from = Address::new_hierarchical(
            &match sub_id.parent() {
                Some(s) => s,
                None => return Err(anyhow!("error getting parent for subnet addr")),
            },
            sig_addr,
        )?;
        let to = Address::new_hierarchical(sub_id, sig_addr)?;
        Ok(Self {
            from: from,
            to: to,
            method: METHOD_SEND,
            params: RawBytes::default(),
            value: value,
            nonce: 0,
        })
    }
}

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

    pub(crate) fn add_metas(&mut self, metas: Vec<CrossMsgMeta>) -> anyhow::Result<()> {
        for m in metas.iter() {
            if self.metas.iter().any(|ms| ms == m) {
                continue;
            }
            self.metas.push(m.clone());
        }

        Ok(())
    }
}

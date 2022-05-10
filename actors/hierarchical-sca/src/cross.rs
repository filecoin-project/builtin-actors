use anyhow::anyhow;
use cid::multihash::Code;
use cid::Cid;
use fil_actors_runtime::{Array, BURNT_FUNDS_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_ipld_encoding::{CborStore, RawBytes};
use fvm_shared::address::{Address, SubnetID};
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;
use std::path::Path;

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

#[derive(PartialEq, Eq)]
pub enum HCMsgType {
    Unknown = 0,
    BottomUp,
    TopDown,
}

impl StorableMsg {
    pub fn new_release_msg(
        sub_id: &SubnetID,
        sig_addr: &Address,
        value: TokenAmount,
        nonce: u64,
    ) -> anyhow::Result<Self> {
        let to = Address::new_hierarchical(
            &match sub_id.parent() {
                Some(s) => s,
                None => return Err(anyhow!("error getting parent for subnet addr")),
            },
            sig_addr,
        )?;
        let from = Address::new_hierarchical(sub_id, &BURNT_FUNDS_ACTOR_ADDR)?;
        Ok(Self {
            from: from,
            to: to,
            method: METHOD_SEND,
            params: RawBytes::default(),
            value: value,
            nonce: nonce,
        })
    }

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

    pub fn hc_type(&self) -> anyhow::Result<HCMsgType> {
        let sto = self.to.subnet()?;
        let sfrom = self.from.subnet()?;
        if is_bottomup(&sfrom, &sto) {
            return Ok(HCMsgType::BottomUp);
        }
        Ok(HCMsgType::TopDown)
    }
}

pub fn is_bottomup(from: &SubnetID, to: &SubnetID) -> bool {
    let index = match from.common_parent(&to) {
        Some((ind, _)) => ind,
        None => return false,
    };
    let a = from.to_string();
    Path::new(&a).components().count() - 1 > index
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

    pub(crate) fn add_msg(&mut self, msg: &StorableMsg) -> anyhow::Result<()> {
        // TODO: Check if the message has already been added.
        self.msgs.push(msg.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::cross::*;
    use std::str::FromStr;

    #[test]
    fn test_is_bottomup() {
        bottom_up("/root/f01", "/root/f01/f02", false);
        bottom_up("/root/f01", "/root", true);
        bottom_up("/root/f01", "/root/f01/f02", false);
        bottom_up("/root/f01", "/root/f02/f02", true);
        bottom_up("/root/f01/f02", "/root/f01/f02", false);
        bottom_up("/root/f01/f02", "/root/f01/f02/f03", false);
    }
    fn bottom_up(a: &str, b: &str, res: bool) {
        assert_eq!(
            is_bottomup(&SubnetID::from_str(a).unwrap(), &SubnetID::from_str(b).unwrap()),
            res
        );
    }
}

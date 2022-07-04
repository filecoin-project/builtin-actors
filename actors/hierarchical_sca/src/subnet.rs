use anyhow::anyhow;
use fil_actors_runtime::runtime::Runtime;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::repr::*;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::SubnetID;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;

use crate::tcid::{TAmt, TCid};
use crate::CROSSMSG_AMT_BITWIDTH;

use super::checkpoint::*;
use super::cross::StorableMsg;
use super::state::State;

#[derive(PartialEq, Eq, Clone, Copy, Debug, Deserialize_repr, Serialize_repr)]
#[repr(i32)]
pub enum Status {
    Active,
    Inactive,
    Killed,
}

#[derive(Clone, Debug, Serialize_tuple, Deserialize_tuple, PartialEq)]
pub struct Subnet {
    pub id: SubnetID,
    #[serde(with = "bigint_ser")]
    pub stake: TokenAmount,
    pub top_down_msgs: TCid<TAmt<StorableMsg, CROSSMSG_AMT_BITWIDTH>>,
    pub nonce: u64,
    #[serde(with = "bigint_ser")]
    pub circ_supply: TokenAmount,
    pub status: Status,
    pub prev_checkpoint: Option<Checkpoint>,
}

impl Cbor for Subnet {}

impl Subnet {
    pub(crate) fn add_stake<BS, RT>(
        &mut self,
        rt: &RT,
        st: &mut State,
        value: &TokenAmount,
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        self.stake += value;
        if self.stake < st.min_stake {
            self.status = Status::Inactive;
        }
        st.flush_subnet(rt.store(), self)?;
        Ok(())
    }

    /// store topdown messages for their execution in the subnet
    pub(crate) fn store_topdown_msg<BS: Blockstore>(
        &mut self,
        store: &BS,
        msg: &StorableMsg,
    ) -> anyhow::Result<()> {
        self.top_down_msgs.update(store, |crossmsgs| {
            crossmsgs
                .set(msg.nonce, msg.clone())
                .map_err(|e| anyhow!("failed to set crossmsg meta array: {}", e))
        })
    }

    pub(crate) fn release_supply(&mut self, value: &TokenAmount) -> anyhow::Result<()> {
        if &self.circ_supply < value {
            return Err(anyhow!(
                "wtf! we can't release funds below circ, supply. something went really wrong"
            ));
        }
        self.circ_supply -= value;
        Ok(())
    }
}

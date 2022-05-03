use anyhow::anyhow;
use cid::Cid;
use fil_actors_runtime::runtime::Runtime;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;
use lazy_static::lazy_static;
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

use super::checkpoint::*;
use super::state::State;

#[derive(PartialEq, Eq, Hash, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct SubnetID {
    parent: String,
    actor: Address,
}
impl Cbor for SubnetID {}

lazy_static! {
    pub static ref ROOTNET_ID: SubnetID =
        SubnetID { parent: String::from("/root"), actor: Address::new_id(0) };
}

#[derive(Debug, PartialEq, Error)]
pub enum Error {
    #[error("invalid subnet id")]
    InvalidID,
}

impl SubnetID {
    pub fn new(parent: &SubnetID, subnet_act: Address) -> SubnetID {
        let parent_str = parent.to_string();

        return SubnetID { parent: parent_str, actor: subnet_act };
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let str_id = self.to_string();
        str_id.into_bytes()
    }

    pub fn subnet_actor(&self) -> Address {
        self.actor
    }

    pub fn parent(&self) -> Option<SubnetID> {
        if *self == *ROOTNET_ID {
            return None;
        }
        match SubnetID::from_str(&self.parent) {
            Ok(id) => Some(id),
            Err(_) => None,
        }
    }

    // pub fn common_parent(other: &SubnetID) -> Result<SubnetID, Error> {
    //     panic!("not implemented")
    // }
    // pub fn down(other: &SubnetID) -> Result<SubnetID, Error> {
    //     panic!("not implemented")
    // }
    // pub fn up(other: &SubnetID) -> Result<SubnetID, Error> {
    //     panic!("not implemented")
    // }
}

impl fmt::Display for SubnetID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.parent == "/root" && self.actor == Address::new_id(0) {
            return write!(f, "{}", self.parent);
        }
        let act_str = self.actor.to_string();
        match Path::join(Path::new(&self.parent), Path::new(&act_str)).to_str() {
            Some(r) => write!(f, "{}", r),
            None => Err(fmt::Error),
        }
    }
}

impl Default for SubnetID {
    fn default() -> Self {
        Self { parent: String::from(""), actor: Address::new_id(0) }
    }
}

impl FromStr for SubnetID {
    type Err = Error;
    fn from_str(addr: &str) -> Result<Self, Error> {
        if addr == ROOTNET_ID.to_string() {
            return Ok(ROOTNET_ID.clone());
        }

        let id = Path::new(addr);
        let act = match Path::file_name(id) {
            Some(act_str) => Address::from_str(act_str.to_str().unwrap_or("")),
            None => return Err(Error::InvalidID),
        };

        let mut anc = id.ancestors();
        _ = anc.next();
        let par = match anc.next() {
            Some(par_str) => par_str.to_str(),
            None => return Err(Error::InvalidID),
        }
        .ok_or(Error::InvalidID)
        .unwrap();

        Ok(Self {
            parent: String::from(par),
            actor: match act {
                Ok(addr) => addr,
                Err(_) => return Err(Error::InvalidID),
            },
        })
    }
}

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
    pub top_down_msgs: Cid, // AMT[type.Messages] from child subnets to apply.
    pub nonce: u64,
    #[serde(with = "bigint_ser")]
    pub circ_supply: TokenAmount,
    pub status: Status,
    pub prev_checkpoint: Checkpoint,
}

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
        st.flush_subnet(rt.store(), self)
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

#[cfg(test)]
mod tests {
    use crate::subnet::*;
    use fvm_shared::address::Address;

    #[test]
    fn test_subnet_id() {
        let act = Address::new_id(1001);
        let sub_id = SubnetID::new(&ROOTNET_ID.clone(), act);
        let sub_id_str = sub_id.to_string();
        assert_eq!(sub_id_str, "/root/f01001");

        let rtt_id = SubnetID::from_str(&sub_id_str).unwrap();
        assert_eq!(sub_id, rtt_id);

        let rootnet = ROOTNET_ID.clone();
        assert_eq!(rootnet.to_string(), "/root");
        let root_sub = SubnetID::from_str(&rootnet.to_string()).unwrap();
        assert_eq!(root_sub, rootnet);
    }
}

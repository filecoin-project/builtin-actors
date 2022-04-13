use anyhow::anyhow;
use cid::Cid;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{serde_bytes, Cbor, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::{bigint_ser, BigInt};
use fvm_shared::econ::TokenAmount;
use lazy_static::lazy_static;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::borrow::Cow;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

#[derive(PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct SubnetID {
    parent: String,
    sub_act: Address,
}
impl Cbor for SubnetID {}

lazy_static! {
    static ref ROOTNET_ID: SubnetID =
        SubnetID { parent: String::from("/root"), sub_act: Address::new_id(0) };
}

#[derive(Debug, PartialEq, Error)]
pub enum Error {
    #[error("invalid subnet id")]
    InvalidID,
}

impl SubnetID {
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

pub fn new_id(parent: SubnetID, subnet_act: Address) -> SubnetID {
    let parent_str = parent.to_string();

    return SubnetID { parent: parent_str, sub_act: subnet_act };
}

impl fmt::Display for SubnetID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.parent == "/root" && self.sub_act == Address::new_id(0) {
            return write!(f, "{}", self.parent);
        }
        let act_str = self.sub_act.to_string();
        match Path::join(Path::new(&self.parent), Path::new(&act_str)).to_str() {
            Some(r) => write!(f, "{}", r),
            None => Err(fmt::Error),
        }
    }
}

impl FromStr for SubnetID {
    type Err = Error;
    fn from_str(addr: &str) -> Result<Self, Error> {
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
            sub_act: match act {
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

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct Subnet {
    pub id: SubnetID,
    #[serde(with = "bigint_ser")]
    pub stake: TokenAmount,
    pub top_down_msgs: Cid, // AMT[type.Messages] from child subnets to apply.
    pub nonce: u64,
    #[serde(with = "bigint_ser")]
    pub circ_supply: TokenAmount,
    pub status: Status,
}

#[cfg(test)]
mod tests {
    use crate::subnet::*;
    use fvm_shared::address::Address;

    #[test]
    fn test_subnet_id() {
        let act = Address::new_id(1001);
        let sub_id = new_id(ROOTNET_ID.clone(), act);
        let sub_id_str = sub_id.to_string();
        assert_eq!(sub_id_str, "/root/f01001");

        let rtt_id = SubnetID::from_str(&sub_id_str).unwrap();
        assert_eq!(sub_id, rtt_id);

        let rootnet = ROOTNET_ID.clone();
        assert_eq!(rootnet.to_string(), "/root");
    }
}

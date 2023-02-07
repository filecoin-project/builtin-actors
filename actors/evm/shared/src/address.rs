use fvm_ipld_encoding::{serde, strict_bytes};

/// A Filecoin address as represented in the FEVM runtime (also called EVM-form).
#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq, Clone, Copy)]
pub struct EthAddress(#[serde(with = "strict_bytes")] pub [u8; 20]);

impl std::fmt::Debug for EthAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}
impl EthAddress {
    /// Returns a "null" address.
    pub const fn null() -> Self {
        Self([0u8; 20])
    }

    /// Returns an EVM-form ID address from actor ID.
    pub fn from_id(id: u64) -> EthAddress {
        let mut bytes = [0u8; 20];
        bytes[0] = 0xff;
        bytes[12..].copy_from_slice(&id.to_be_bytes());
        EthAddress(bytes)
    }

    /// Returns true if this is the null/zero EthAddress.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.0 == [0; 20]
    }

    /// Returns true if the EthAddress is an actor ID embedded in an eth address.
    #[inline]
    pub fn is_id(&self) -> bool {
        self.0[0] == 0xff && self.0[1..12].iter().all(|&i| i == 0)
    }
}

use crate::U256;
use fil_actors_runtime::EAM_ACTOR_ID;
use fvm_ipld_encoding::{serde, strict_bytes};
use fvm_shared::address::Address;
use fvm_shared::ActorID;

/// A Filecoin address as represented in the FEVM runtime (also called EVM-form).
///
/// TODO this type will eventually handle f4 address detection.
#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq, Clone, Copy)]
pub struct EthAddress(#[serde(with = "strict_bytes")] pub [u8; 20]);

/// Converts a U256 to an EthAddress by taking the lower 20 bytes.
///
/// Per the EVM spec, this simply discards the high bytes.
impl From<U256> for EthAddress {
    fn from(v: U256) -> Self {
        let mut bytes = [0u8; 32];
        v.to_big_endian(&mut bytes);
        Self(bytes[12..].try_into().unwrap())
    }
}

impl std::fmt::Debug for EthAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl From<EthAddress> for Address {
    fn from(addr: EthAddress) -> Self {
        From::from(&addr)
    }
}

impl From<&EthAddress> for Address {
    fn from(addr: &EthAddress) -> Self {
        if let Some(id) = addr.as_id() {
            Address::new_id(id)
        } else {
            Address::new_delegated(EAM_ACTOR_ID, addr.as_ref()).unwrap()
        }
    }
}

impl EthAddress {
    /// Returns an EVM-form ID address from actor ID.
    pub fn from_id(id: u64) -> EthAddress {
        let mut bytes = [0u8; 20];
        bytes[0] = 0xff;
        bytes[12..].copy_from_slice(&id.to_be_bytes());
        EthAddress(bytes)
    }

    /// Interpret the EVM word as an ID address in EVM-form, and return a Filecoin ID address if
    /// that's the case.
    ///
    /// An ID address starts with 0xff (msb), and contains the u64 in the last 8 bytes.
    /// We assert that everything in between are 0x00, otherwise we've gotten an illegal address.
    ///
    /// 0    1-11       12
    /// 0xff \[0x00...] [id address...]
    pub fn as_id(&self) -> Option<ActorID> {
        if (self.0[0] != 0xff) || !self.0[1..12].iter().all(|&byte| byte == 0) {
            return None;
        }
        Some(u64::from_be_bytes(self.0[12..].try_into().unwrap()))
    }

    /// Returns this Address as an EVM word.
    pub fn as_evm_word(&self) -> U256 {
        U256::from_big_endian(&self.0)
    }

    /// Returns true if this is the null/zero EthAddress.
    pub fn is_null(&self) -> bool {
        self.0 == [0; 20]
    }
}

impl AsRef<[u8]> for EthAddress {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use crate::interpreter::address::EthAddress;
    use crate::U256;

    const TYPE_PADDING: &[u8] = &[0; 12]; // padding (12 bytes)
    const ID_ADDRESS_MARKER: &[u8] = &[0xff]; // ID address marker (1 byte)
    const GOOD_ADDRESS_PADDING: &[u8] =
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // padding for inner u64 (11 bytes)

    macro_rules! id_address_test {
        ($($name:ident: $input:expr => $expectation:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let evm_bytes = $input.concat();
                let evm_addr = EthAddress::from(U256::from(evm_bytes.as_slice()));
                assert_eq!(
                    evm_addr.as_id(),
                    $expectation
                );

                // test inverse conversion, if a valid ID address was supplied
                if let Some(fil_id) = $expectation {
                    assert_eq!(EthAddress::from_id(fil_id), evm_addr);
                }
            }
        )*
        };
    }

    id_address_test! {
        good_address_1: [
            TYPE_PADDING,
            ID_ADDRESS_MARKER,
            GOOD_ADDRESS_PADDING,
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01].as_slice() // ID address (u64 big endian) (8 bytes)
        ] => Some(1),

        good_address_2: [
            TYPE_PADDING,
            ID_ADDRESS_MARKER,
            GOOD_ADDRESS_PADDING,
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff].as_slice() // ID address (u64 big endian) (8 bytes)
        ] => Some(u16::MAX as u64),

        bad_marker: [
            TYPE_PADDING,
            &[0xfa],
            GOOD_ADDRESS_PADDING,
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01].as_slice() // ID address (u64 big endian) (8 bytes)
        ] => None,

        bad_padding: [
            TYPE_PADDING,
            ID_ADDRESS_MARKER,
            &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01], // bad padding
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01].as_slice() // ID address (u64 big endian) (8 bytes)
        ] => None,

        bad_marker_and_padding: [
            TYPE_PADDING,
            &[0xfa],
            &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01], // bad padding
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01].as_slice() // ID address (u64 big endian) (8 bytes)
        ] => None,
    }
}

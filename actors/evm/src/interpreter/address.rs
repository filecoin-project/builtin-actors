use crate::interpreter::{H160, H256};
use crate::U256;
use fvm_shared::address::Address as FilecoinAddress;

/// A Filecoin ad   dress as represented in the FEVM runtime.
/// This type is a little bit silly now, but it will eventually handle f4 address detection.
pub struct Address(H160);

impl From<U256> for Address {
    fn from(v: U256) -> Self {
        Self(H256(v.into()).into())
    }
}

impl Address {
    /// Expect an ID address and return it if that's the case.
    /// An ID address starts with 0xff (msb), and contains the u64 in the last 8 bytes.
    /// We assert that everything in between are 0x00, otherwise we've gotten an illegal address.
    ///
    /// 0    1-11       12
    /// 0xff \[0x00...] [id address...]
    ///
    /// TODO worth replacing the Option with a Result, once we figure out error handling in this actor.
    pub fn as_id_address(&self) -> Option<FilecoinAddress> {
        let val = &self.0 .0;
        if (val[0] != 0xff) || !val[1..12].iter().all(|&byte| byte == 0) {
            return None;
        }
        Some(FilecoinAddress::new_id(u64::from_be_bytes(val[12..].try_into().unwrap())))
    }
}

#[cfg(test)]
mod tests {
    use crate::interpreter::address::Address;
    use crate::U256;
    use fvm_shared::address::Address as FilecoinAddress;

    const TYPE_PADDING: &[u8] = &[0; 12]; // padding preceding H160 (12 bytes)
    const ID_ADDRESS_MARKER: &[u8] = &[0xff]; // ID address marker (1 byte)
    const GOOD_ADDRESS_PADDING: &[u8] =
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // padding for inner u64 (11 bytes)

    macro_rules! id_address_test {
        ($($name:ident: $input:expr => $expectation:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let bytes = $input.concat();
                assert_eq!(
                    Address::from(U256::from(bytes.as_slice())).as_id_address(),
                    $expectation
                )
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
        ] => Some(FilecoinAddress::new_id(1)),

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

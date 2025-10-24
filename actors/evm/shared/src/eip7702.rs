use crate::address::EthAddress;

/// EIP-7702 bytecode magic prefix and version.
pub const EIP7702_MAGIC: [u8; 2] = [0xEF, 0x01];
pub const EIP7702_VERSION: u8 = 0x00;

/// Returns true if code is an EIP-7702 delegation indicator: 0xEF 0x01 0x00 || 20-byte address.
pub fn is_eip7702_code(code: &[u8]) -> bool {
    code.len() == 23 && code[0..2] == EIP7702_MAGIC && code[2] == EIP7702_VERSION
}

/// Attempts to parse an EIP-7702 delegation indicator and return the embedded 20-byte address.
pub fn eip7702_delegate_address(code: &[u8]) -> Option<EthAddress> {
    if !is_eip7702_code(code) { return None; }
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&code[3..23]);
    Some(EthAddress(addr))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detect_and_parse() {
        let mut raw = vec![0u8; 23];
        raw[0] = 0xEF; raw[1] = 0x01; raw[2] = 0x00;
        for i in 0..20 { raw[3+i] = 0xAB; }
        assert!(is_eip7702_code(&raw));
        let d = eip7702_delegate_address(&raw).unwrap();
        assert_eq!(d, EthAddress([0xAB; 20]));
        assert!(!is_eip7702_code(&raw[..10]));
        let mut bad = raw.clone();
        bad[1] = 0x00; assert!(!is_eip7702_code(&bad));
    }
}


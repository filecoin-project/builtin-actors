use fil_actors_evm_shared::uints::U256;

/// EIP-7939 `CLZ` test vectors from https://eips.ethereum.org/EIPS/eip-7939
#[allow(dead_code)]
pub fn clz_eip7939_test_vectors() -> [(U256, U256); 6] {
    [
        (U256::ZERO, U256::from(256)),
        (U256::ONE << 255, U256::ZERO),
        (U256::MAX, U256::ZERO),
        (U256::ONE << 254, U256::ONE),
        ((U256::ONE << 255) - U256::ONE, U256::ONE),
        (U256::ONE, U256::from(255)),
    ]
}

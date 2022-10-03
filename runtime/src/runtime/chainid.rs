#[cfg(feature = "mainnet")]
pub const CHAINID: u64 = 314;

#[cfg(feature = "wallaby")]
pub const CHAINID: u64 = 31415;

#[cfg(feature = "calibrationnet")]
pub const CHAINID: u64 = 314159;

#[cfg(any(feature = "catterpillarnet", feature = "butterflynet"))]
pub const CHAINID: u64 = 3141592;

#[cfg(any(
    feature = "devnet",
    feature = "devnet-wasm",
    feature = "testing",
    feature = "testing-fake-proofs",
))]
pub const CHAINID: u64 = 31415926;

// default build is same as mainnet
#[cfg(not(any(
    feature = "mainnet",
    feature = "wallaby",
    feature = "calibrationnet",
    feature = "catterpillarnet",
    feature = "butterflynet",
    feature = "devnet",
    feature = "devnet-wasm",
    feature = "testing",
    feature = "testing-fake-proofs",
)))]
pub const CHAINID: u64 = 314;

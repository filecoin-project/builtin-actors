use fvm_shared::address::Address;
use rand::prelude::*;

pub fn new_bls_addr(s: u8) -> Address {
    let seed = [s; 32];
    let mut rng : StdRng = SeedableRng::from_seed(seed);
    let mut key = [0u8; 48];
    rng.fill_bytes(&mut key);
    Address::new_bls(&key).unwrap()
}

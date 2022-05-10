
use fvm_shared::address::{Address, BLS_PUB_LEN};
use rand_chacha::ChaCha8Rng;
use rand::prelude::*;

// Generate count addresses by seeding an rng with seed
pub fn pk_addrs_from(seed: u64, count: u64) -> Vec<Address> {
    let mut seed_arr  = [0u8; 32];
    for (i, b) in seed.to_ne_bytes().iter().enumerate() {
        seed_arr[i] = *b;
    }
    let mut rng = ChaCha8Rng::from_seed(seed_arr);
    let mut ret = vec![];
    for _ in 0..count {
        ret.push(new_bls_from_rng(&mut rng));
    }
    ret
}

// Generate nice 32 byte arrays sampled uniformly at random based off of a u64 seed
fn new_bls_from_rng(rng: & mut ChaCha8Rng) -> Address {
    let mut bytes = [0u8; BLS_PUB_LEN];
    rng.fill_bytes(&mut bytes);
    Address::new_bls(&bytes).unwrap()
}

use fvm_shared::address::{Address, BLS_PUB_LEN};
use rand_chacha::ChaCha8Rng;
use rand::prelude::*;
use crate::*;
use fvm_shared::econ::TokenAmount;
use fvm_ipld_encoding::{Cbor, CborStore, RawBytes};
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use fvm_shared::error::ExitCode;


// Generate count addresses by seeding an rng
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

const ACCOUNT_SEED: u64 = 93837778;

pub fn create_accounts<'bs>(mut v: &VM<'bs>, count: u64, balance: TokenAmount) -> Vec<Address> {
    let pk_addrs = pk_addrs_from(ACCOUNT_SEED, count);
    // Send funds from faucet to pk address, creating account actor
    for pk_addr in pk_addrs.clone() {
        apply_ok(& mut v, TEST_FAUCET_ADDR, pk_addr, balance.clone(), METHOD_SEND, RawBytes::default());
    }
    // Normalize pk address to return id address of account actor
    let mut addrs = Vec::<Address>::new();
    for pk_addr in pk_addrs {
        addrs.push(v.normalize_address(&pk_addr).unwrap())
    }
    addrs
}

pub fn apply_ok<'bs, C:Cbor>(mut v: &VM<'bs>, from: Address, to: Address, value: TokenAmount, method: MethodNum, params: C ) ->  RawBytes {
    let res = v.apply_message(from, to, value, method, params).unwrap();
    assert_eq!(ExitCode::OK, res.code);
    res.ret
}
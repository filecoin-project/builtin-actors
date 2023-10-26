use cid::multihash::Code;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{CborStore, RawBytes};
use fvm_shared::address::{Address, BLS_PUB_LEN};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

mod blockstore;
pub use blockstore::*;
use serde::de::DeserializeOwned;

use crate::VM;

/// Generate count addresses by seeding an rng
pub fn pk_addrs_from(seed: u64, count: u64) -> Vec<Address> {
    let mut seed_arr = [0u8; 32];
    for (i, b) in seed.to_ne_bytes().iter().enumerate() {
        seed_arr[i] = *b;
    }
    let mut rng = ChaCha8Rng::from_seed(seed_arr);
    (0..count).map(|_| new_bls_from_rng(&mut rng)).collect()
}

/// Generate nice 32 byte arrays sampled uniformly at random based off of a u64 seed
fn new_bls_from_rng(rng: &mut ChaCha8Rng) -> Address {
    let mut bytes = [0u8; BLS_PUB_LEN];
    rng.fill_bytes(&mut bytes);
    Address::new_bls(&bytes).unwrap()
}

pub fn apply_ok<S: Serialize>(
    v: &dyn VM,
    from: &Address,
    to: &Address,
    value: &TokenAmount,
    method: MethodNum,
    params: Option<S>,
) -> RawBytes {
    apply_code(v, from, to, value, method, params, ExitCode::OK)
}

pub fn apply_code<S: Serialize>(
    v: &dyn VM,
    from: &Address,
    to: &Address,
    value: &TokenAmount,
    method: MethodNum,
    params: Option<S>,
    code: ExitCode,
) -> RawBytes {
    let params = params.map(|p| IpldBlock::serialize_cbor(&p).unwrap().unwrap());
    let res = v.execute_message(from, to, value, method, params).unwrap();
    assert_eq!(code, res.code, "expected code {}, got {} ({})", code, res.code, res.message);
    res.ret.map_or(RawBytes::default(), |b| RawBytes::new(b.data))
}

pub fn apply_ok_implicit<S: Serialize>(
    v: &dyn VM,
    from: &Address,
    to: &Address,
    value: &TokenAmount,
    method: MethodNum,
    params: Option<S>,
) -> RawBytes {
    let code = ExitCode::OK;
    let params = params.map(|p| IpldBlock::serialize_cbor(&p).unwrap().unwrap());
    let res = v.execute_message_implicit(from, to, value, method, params).unwrap();
    assert_eq!(code, res.code, "expected code {}, got {} ({})", code, res.code, res.message);
    res.ret.map_or(RawBytes::default(), |b| RawBytes::new(b.data))
}
pub fn get_state<T: DeserializeOwned>(v: &dyn VM, a: &Address) -> Option<T> {
    let cid = v.actor(a).unwrap().state;
    v.blockstore().get(&cid).unwrap().map(|slice| fvm_ipld_encoding::from_slice(&slice).unwrap())
}

/// Convenience function to create an IpldBlock from a serializable object
pub fn serialize_ok<S: Serialize>(s: &S) -> IpldBlock {
    IpldBlock::serialize_cbor(s).unwrap().unwrap()
}

/// Update the state of a given actor in place
pub fn mutate_state<S, F>(v: &dyn VM, addr: &Address, f: F)
where
    S: Serialize + DeserializeOwned,
    F: FnOnce(&mut S),
{
    let mut a = v.actor(addr).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let mut st = store.get_cbor::<S>(&a.state).unwrap().unwrap();
    f(&mut st);
    a.state = store.put_cbor(&st, Code::Blake2b256).unwrap();
    v.set_actor(addr, a);
}

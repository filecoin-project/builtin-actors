use fil_actor_init::Method as InitMethod;
use fil_actor_miner::{Method as MinerMethod, MinerConstructorParams};
use fil_actor_power::{CreateMinerParams, Method as PowerMethod};
use fil_actors_runtime::cbor::serialize;

use fil_actors_runtime::{INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::METHOD_SEND;
use test_vm::{ExpectInvocation, FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR, VM};

#[test]
fn test_proposal_hash() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

}

use cid::{multihash, Cid};
use fvm_shared::econ::TokenAmount;
use fil_actors_runtime::{
    //actor_error, make_empty_map, make_map_with_root, resolve_to_id_addr, wasm_trampoline,
    Map,
};
use fvm_shared::encoding::tuple::*;
use fvm_shared::encoding::de::Deserialize;
use fvm_shared::bigint::{bigint_ser, Integer};
use fvm_ipld_hamt::{Hamt, BytesKey};
use std::cell::RefCell;
use std::collections::HashMap;
use fvm_shared::blockstore::{MemoryBlockstore};
use anyhow;


use core::fmt::Error;


pub struct VM<'bs> {
    store: &'bs MemoryBlockstore,

    state_root: Cid,
    actors_dirty: bool,
    actors: Hamt<MemoryBlockstore, Actor, BytesKey>,
} 


impl<'bs> VM<'bs> {
    fn apply_message_internal() {

        // get from_id

        // grab from actor from state

        // send
        // 1. update call seq num
        // 2. build invoc context
        // 3. call method
    }

    pub fn normalize_address() {

    }

    pub fn get_actor() {

    }

    pub fn set_actor() {

    }

    fn checkpoint(&mut self) ->
     Result<Cid, fvm_ipld_hamt::Error> {
        self.actors.flush()
    }
}


#[derive(Serialize_tuple, Deserialize_tuple, Clone)]
pub struct Actor{
    code: Cid,  // Might want to mock this out to avoid dealing with the annoying bundler
    head: Cid, 
    call_seq_num: u64,
    #[serde(with = "bigint_ser")]
    balance: TokenAmount,
}

// struct invocation_context{
//     msg: internal_message,
//     from_actor: Actor,
//     to_actor: Actor,
// }

// impl invocation_context{

// }
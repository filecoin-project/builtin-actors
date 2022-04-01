
use cid::{multihash, Cid};
use fil_actors_runtime::builtin::HAMT_BIT_WIDTH;
use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::tuple::*;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::encoding::de::Deserialize;
use fvm_shared::bigint::{bigint_ser, Integer};
use fvm_ipld_hamt::{Hamt, BytesKey, Sha256};
use fvm_shared::blockstore::{MemoryBlockstore, Blockstore};
use std::error::Error;
use std::fmt;


pub struct VM<'bs>{
    store: &'bs MemoryBlockstore,
    state_root: Cid,
    actors_dirty: bool,
    actors: Hamt<&'bs MemoryBlockstore, Actor, BytesKey>,
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

    // pub fn normalize_address(addr: Address) -> Result<(Address, bool), Error> {
    //     match addr.protocol() {
    //         Protocol::ID => Ok((addr, true)),
    //         _ => {

    //         }

    //     }
    // }

    pub fn get_actor(&self, addr: Address) -> Result<Actor, TestVMError> {
        match self.actors.get(&addr.to_bytes())? {
            None => Err(vm_err("failed to get addr")),
            Some(a)=> Ok(a.clone()),
        }
    }

    // blindly overwrite the actor at this address whether it previously existed or not
    pub fn set_actor(&mut self, key: Address, a: Actor) -> Result<(), fvm_ipld_hamt::Error> {
        let _ = self.actors.set(key.to_bytes().into(), a)?;
        Ok(())
    }

    fn checkpoint(&mut self) ->
     Result<Cid, fvm_ipld_hamt::Error> {
        self.state_root = self.actors.flush()?;
        self.actors_dirty = false;
        Ok(self.state_root)
    }

    fn rollback(&mut self, root: Cid) -> Result<(), fvm_ipld_hamt::Error> {
        self.actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::load(&root, &self.store)?;
        self.state_root = root;
        self.actors_dirty = false;
        Ok(())
    } 
}


#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq)]
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
#[derive(Debug)]
pub struct TestVMError {
    msg: String
}

impl fmt::Display for TestVMError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,"{}",self.msg)
    }
}

impl Error for TestVMError {
    fn description(&self) -> &str {
        &self.msg
    }
}

impl From<fvm_ipld_hamt::Error> for TestVMError {
    fn from(h_err: fvm_ipld_hamt::Error) -> Self {
        vm_err(h_err.to_string().as_str())
    }
}

pub fn vm_err(msg: &str) -> TestVMError {
    TestVMError{msg: msg.to_string()}
}

const Cid::
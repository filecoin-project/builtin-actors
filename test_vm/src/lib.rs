use cid::Cid;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_hamt::{BytesKey, Hamt, Sha256};
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;
use fvm_shared::MethodNum;
use fvm_shared::error::ExitCode;
use std::error::Error;
use std::fmt;

use fvm_ipld_encoding::{RawBytes};

pub struct VM<'bs> {
    store: &'bs MemoryBlockstore,
    state_root: Cid,
    actors_dirty: bool,
    actors: Hamt<&'bs MemoryBlockstore, Actor, BytesKey>,
}

impl<'bs> VM<'bs> {
    pub fn new(store: &'bs MemoryBlockstore) -> VM<'bs> {
        let mut actors = Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::new(store);
        VM { store, state_root: actors.flush().unwrap(), actors_dirty: false, actors }
    }

    pub fn get_actor(&self, addr: &Address) -> Option<&Actor> {
        self.actors.get(&addr.to_bytes()).unwrap()
    }

    // blindly overwrite the actor at this address whether it previously existed or not
    pub fn set_actor(&mut self, key: &Address, a: Actor) {
        let _ = self.actors.set(key.to_bytes().into(), a).unwrap();
    }

    pub fn checkpoint(&mut self) -> Cid {
        self.state_root = self.actors.flush().unwrap();
        self.actors_dirty = false;
        self.state_root
    }

    pub fn rollback(&mut self, root: &Cid) {
        self.actors =
            Hamt::<&'bs MemoryBlockstore, Actor, BytesKey, Sha256>::load(root, self.store).unwrap();
        self.state_root = *root;
        self.actors_dirty = false;
    }

    pub fn normalize_address(&self, addr: Address) -> Option<Address> {
        match addr.protocol() {
            Protocol::ID => return Some(addr),
            _ => (),
        }

        let init_actor = self.get_actor(&INIT_ACTOR_ADDR).unwrap();
        self.store.get();

        Some(addr)
    }

    pub fn get_state(&self) Result<>{
        Ok(self.store.get_cbor(self.state.as_ref().unwrap()).unwrap().unwrap())
    }

    pub fn apply_message(&mut self, from: &Address, to: &Address, value: &TokenAmount, method: MethodNum, params: &RawBytes) -> Result<MessageResult, TestVMError> {
        // XXX normalize from address to id
        let mut a = self.get_actor(from).unwrap().clone();
        a.call_seq_num += 1; 
        self.set_actor(from, a);

        let prior_root = self.checkpoint();

        // make top level context with internal context
        // let ret, exitcode = ctx.invoke()
        let ret = RawBytes::default();
        let code = ExitCode::Ok;

        if code != ExitCode::Ok { // if exitcode != ok
            self.rollback(&prior_root);
        } else {
            self.checkpoint();
        }

        Ok(MessageResult{code: code, ret: ret})
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MessageResult {
    pub code: ExitCode,
    pub ret: RawBytes,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Debug)]
pub struct Actor {
    pub code: Cid,
    pub head: Cid,
    pub call_seq_num: u64,
    #[serde(with = "bigint_ser")]
    pub balance: TokenAmount,
}

pub fn actor(code: Cid, head: Cid, seq: u64, bal: TokenAmount) -> Actor {
    Actor { code, head, call_seq_num: seq, balance: bal }
}

#[derive(Debug)]
pub struct TestVMError {
    msg: String,
}

impl fmt::Display for TestVMError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
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
    TestVMError { msg: msg.to_string() }
}

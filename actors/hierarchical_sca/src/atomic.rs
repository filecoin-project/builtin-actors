use crate::{
    tcid::{TCid, THamt},
    StorableMsg,
};
use cid::Cid;
use fvm_ipld_encoding::repr::*;
use fvm_ipld_encoding::{serde_bytes, tuple::*, Cbor, RawBytes};
use fvm_shared::MethodNum;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use std::collections::HashMap;

pub const METHOD_LOCK: MethodNum = 2;
pub const METHOD_MERGE: MethodNum = 3;
pub const METHOD_ABORT: MethodNum = 4;
pub const METHOD_UNLOCK: MethodNum = 5;

#[derive(PartialEq, Eq, Clone, Copy, Debug, Deserialize_repr, Serialize_repr)]
#[repr(u64)]
pub enum ExecStatus {
    UndefState,
    Initialized,
    Success,
    Aborted,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct SerializedState {
    #[serde(with = "serde_bytes")]
    ser: Vec<u8>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct AtomicExec {
    params: AtomicExecParams,
    submitted: HashMap<String, Cid>,
    status: ExecStatus,
}
impl Cbor for AtomicExec {}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct SubmitExecParams {
    cid: Cid,
    abort: bool,
    output: SerializedState, // TODO: LockedState
}
impl Cbor for SubmitExecParams {}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct AtomicExecParams {
    messages: Vec<StorableMsg>,
    inputs: HashMap<String, SerializedState>, // TODO: String/LockedState
}
impl Cbor for AtomicExecParams {}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct LockParams {
    pub method: MethodNum,
    #[serde(with = "serde_bytes")]
    pub params: Vec<u8>,
}
impl Cbor for LockParams {}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct MergeParams<T: Serialize + DeserializeOwned> {
    state: T,
}
impl<T: Serialize + DeserializeOwned> Cbor for MergeParams<T> {}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct UnlockParams {
    pub dummy: u64,
}
impl Cbor for UnlockParams {}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct LockedState<T: Serialize + DeserializeOwned> {
    lock: bool,
    state: T,
}
impl<T: Serialize + DeserializeOwned> Cbor for LockedState<T> {}

pub trait LockableState<S: Serialize + DeserializeOwned> {
    fn merge(other: Self) -> anyhow::Result<()>;
    fn merge_output(other: Self) -> anyhow::Result<()>;
}

pub trait LockableActorState<T: Serialize + DeserializeOwned> {
    fn locked_map_cid() -> TCid<THamt<Cid, LockedState<T>>>;
    fn output(params: LockParams) -> LockedState<T>;
}

pub trait LockableActor<S: Serialize + DeserializeOwned + LockableActorState<S>> {
    fn lock(params: LockParams) -> anyhow::Result<Option<RawBytes>>;
    fn merge(params: MergeParams<S>) -> anyhow::Result<Option<RawBytes>>;
    fn unlock(params: UnlockParams) -> anyhow::Result<Option<RawBytes>>;
    fn abort(params: LockParams) -> anyhow::Result<Option<RawBytes>>;
    fn state(params: LockParams) -> S;
}

#[cfg(test)]
mod test {
    #[test]
    fn test_e2e_lock() {}
}

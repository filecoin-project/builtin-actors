use crate::interpreter::address::EthAddress;
use crate::interpreter::instructions::memory::copy_to_memory;
use crate::U256;
use cid::Cid;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::ActorError;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::{address::Address, econ::TokenAmount};
use num_traits::Zero;
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn extcodesize(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
    addr: U256,
) -> Result<U256, StatusCode> {
    // TODO (M2.2) we're fetching the entire block here just to get its size. We should instead use
    //  the ipld::block_stat syscall, but the Runtime nor the Blockstore expose it.
    //  Tracked in https://github.com/filecoin-project/ref-fvm/issues/867
    let len = match get_evm_bytecode_cid(system.rt, addr)? {
        // evm cid
        Ok(cid) =>
        // TODO this is part of account abstraction hack where EOAs are Embryos
        {
            if cid == system.rt.get_code_cid_for_type(Type::Embryo) {
                Ok(0)
            } else {
                get_evm_bytecode(system.rt, &cid).map(|bytecode| bytecode.len())
            }
        }
        // native cid
        Err(cid) => {
            if cid == system.rt.get_code_cid_for_type(Type::Account) {
                // system account has no code (and we want solidity isContract to return false)
                Ok(0)
            } else {
                // native actor code
                // TODO bikeshed this, needs to be at least non-zero though for solidity isContract.
                // https://github.com/filecoin-project/ref-fvm/issues/1134
                Ok(1)
            }
        }
    }?;

    Ok(len.into())
}

pub fn extcodehash(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
    addr: U256,
) -> Result<U256, StatusCode> {
    let cid = get_evm_bytecode_cid(system.rt, addr)?;
    let digest = if let Ok(cid) = cid {
        cid.hash().digest()
    } else {
        // REMOVEME: instead we could return the hash of the actor, but this may be confusing for EVM contracts
        return Err(StatusCode::InvalidArgument(
            "cannot invoke EXTCODEHASH for non-EVM actor".to_string(),
        ));
    };
    // Take the first 32 bytes of the Multihash
    let digest_len = digest.len().min(32);
    Ok(digest[..digest_len].into())
}

pub fn extcodecopy(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
    addr: U256,
    dest_offset: U256,
    data_offset: U256,
    size: U256,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, .. } = state;

    // TODO err trying to copy native code
    let bytecode = get_evm_bytecode_cid(system.rt, addr).map(|cid| {
        cid.map(|evm_cid| get_evm_bytecode(system.rt, &evm_cid))
            // calling EXTCODECOPY on native actors results with a single byte 0xFE which solidtiy uses for its `assert`/`throw` methods
            // and in general invalid EVM bytecode
            .unwrap_or(Ok(vec![0xFE]))
    })??;

    copy_to_memory(&mut state.memory, dest_offset, size, data_offset, bytecode.as_slice(), true)
}

/// Attempts to get bytecode CID of an evm contract, returning either an error or the CID of the native actor as the error  
pub fn get_evm_bytecode_cid(rt: &impl Runtime, addr: U256) -> Result<Result<Cid, Cid>, StatusCode> {
    let addr: EthAddress = addr.into();
    let addr: Address = addr.try_into()?;
    let actor_id = rt.resolve_address(&addr).ok_or_else(|| {
        StatusCode::InvalidArgument("failed to resolve address".to_string())
        // TODO better error code
    })?;

    let evm_cid = rt.get_code_cid_for_type(Type::EVM);
    let target_cid = rt.get_actor_code_cid(&actor_id);

    match target_cid {
        Some(cid) => {
            if cid != evm_cid {
                return Ok(Err(cid));
            }
        }
        None => (),
    }

    let cid = rt
        .send(&addr, crate::Method::GetBytecode as u64, Default::default(), TokenAmount::zero())?
        .deserialize()?;
    Ok(cid)
}

pub fn get_evm_bytecode(rt: &impl Runtime, cid: &Cid) -> Result<Vec<u8>, StatusCode> {
    let raw_bytecode = rt
        .store()
        .get(cid) // TODO this is inefficient; should call stat here.
        .map_err(|e| StatusCode::InternalError(format!("failed to get bytecode block: {}", &e)))?
        .ok_or_else(|| ActorError::not_found("bytecode block not found".to_string()))?;
    Ok(raw_bytecode)
}

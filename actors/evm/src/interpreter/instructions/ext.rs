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

pub fn extcodesize(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
    addr: U256,
) -> Result<U256, StatusCode> {
    // TODO (M2.2) we're fetching the entire block here just to get its size. We should instead use
    //  the ipld::block_stat syscall, but the Runtime nor the Blockstore expose it.
    //  Tracked in https://github.com/filecoin-project/ref-fvm/issues/867
    let cid = get_cid_type(system.rt, addr)?;

    let len = match cid {
        CodeCid::EVM(addr) => get_evm_bytecode(system.rt, &addr).map(|bytecode| bytecode.len())?,
        CodeCid::Native(_) => 1,
        // account and not found are flattened to 0 size
        _ => 0,
    };

    Ok(len.into())
}

pub fn extcodehash(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
    addr: U256,
) -> Result<U256, StatusCode> {
    let addr = get_cid_type(system.rt, addr)?.unwrap_evm(StatusCode::InvalidArgument(
        "Cannot invoke EXTCODEHASH for non-EVM actor.".to_string(),
    ))?;
    let bytecode_cid: Cid = system
        .rt
        .send(&addr, crate::Method::GetBytecode as u64, Default::default(), TokenAmount::zero())?
        .deserialize()?;

    println!("{}", bytecode_cid);
    let digest = bytecode_cid.hash().digest();

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
    let bytecode = get_cid_type(system.rt, addr).map(|cid| {
        cid.unwrap_evm(())
            .map(|addr| get_evm_bytecode(system.rt, &addr))
            // calling EXTCODECOPY on native actors results with a single byte 0xFE which solidtiy uses for its `assert`/`throw` methods
            // and in general invalid EVM bytecode
            .unwrap_or_else(|_| Ok(vec![0xFE]))
    })??;

    copy_to_memory(&mut state.memory, dest_offset, size, data_offset, bytecode.as_slice(), true)
}

#[derive(Debug)]
pub enum CodeCid {
    /// EVM Address and the CID of the actor (not the bytecode)
    EVM(Address),
    Native(Cid),
    Account,
    NotFound,
}

impl CodeCid {
    pub fn unwrap_evm<E>(&self, err: E) -> Result<Address, E> {
        if let CodeCid::EVM(ret) = self {
            Ok(*ret)
        } else {
            Err(err)
        }
    }
}

/// Resolves an address to the address type
pub fn get_cid_type(rt: &impl Runtime, addr: U256) -> Result<CodeCid, StatusCode> {
    let addr: EthAddress = addr.into();
    let addr: Address = addr.try_into()?;

    rt.resolve_address(&addr)
        .and_then(|id| {
            rt.get_actor_code_cid(&id).map(|cid| {
                let code_cid = rt
                    .resolve_builtin_actor_type(&cid)
                    .map(|t| {
                        match t {
                            Type::Account => CodeCid::Account,
                            // TODO part of current account abstraction hack where emryos are accounts
                            Type::Embryo => CodeCid::Account,
                            Type::EVM => CodeCid::EVM(addr),
                            // remaining builtin actors are native
                            _ => CodeCid::Native(cid),
                        }
                        // not a builtin actor, so it is probably a native actor
                    })
                    .unwrap_or(CodeCid::Native(cid));
                Ok(code_cid)
            })
        })
        .unwrap_or(Ok(CodeCid::NotFound))
}

pub fn get_evm_bytecode(rt: &impl Runtime, addr: &Address) -> Result<Vec<u8>, StatusCode> {
    let cid = rt
        .send(addr, crate::Method::GetBytecode as u64, Default::default(), TokenAmount::zero())?
        .deserialize()?;
    let raw_bytecode = rt
        .store()
        .get(&cid) // TODO this is inefficient; should call stat here.
        .map_err(|e| StatusCode::InternalError(format!("failed to get bytecode block: {}", &e)))?
        .ok_or_else(|| ActorError::not_found("bytecode block not found".to_string()))?;
    Ok(raw_bytecode)
}

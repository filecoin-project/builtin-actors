use crate::interpreter::instructions::memory::copy_to_memory;
use crate::interpreter::{address::EthAddress, precompiles::Precompiles};
use crate::U256;
use cid::Cid;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::ActorError;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::sys::SendFlags;
use fvm_shared::{address::Address, econ::TokenAmount};
use multihash::Multihash;
use num_traits::Zero;
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
};

/// Keccak256 hash of `[0xfe]`, "native bytecode"
const NATIVE_BYTECODE_HASH: [u8; 32] =
    hex_literal::hex!("bcc90f2d6dada5b18e155c17a1c0a55920aae94f39857d39d0d8ed07ae8f228b");

/// Keccak256 hash of `[]`, empty bytecode
const EMPTY_EVM_HASH: [u8; 32] =
    hex_literal::hex!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");

pub fn extcodesize(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    addr: U256,
) -> Result<U256, StatusCode> {
    // TODO (M2.2) we're fetching the entire block here just to get its size. We should instead use
    //  the ipld::block_stat syscall, but the Runtime nor the Blockstore expose it.
    //  Tracked in https://github.com/filecoin-project/ref-fvm/issues/867
    let len = match get_contract_type(system.rt, addr) {
        ContractType::EVM(addr) => {
            get_evm_bytecode(system, &addr).map(|bytecode| bytecode.len())?
        }
        ContractType::Native(_) => 1,
        // account, not found, and precompiles are 0 size
        _ => 0,
    };

    Ok(len.into())
}

pub fn extcodehash(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    addr: U256,
) -> Result<U256, StatusCode> {
    let addr = match get_contract_type(system.rt, addr) {
        ContractType::EVM(a) => a,
        // _Technically_ since we have native "bytecode" set as 0xfe this is valid, though we cant differentiate between different native actors.
        ContractType::Native(_) => return Ok(NATIVE_BYTECODE_HASH.into()),
        // Precompiles "exist" and therefore aren't empty (although spec-wise they can be either 0 or keccak("") ).
        ContractType::Precompile => return Ok(EMPTY_EVM_HASH.into()),
        // NOTE: There may be accounts that in EVM would be considered "empty" (as defined in EIP-161) and give 0, but we will instead return keccak("").
        //      The FVM does not have chain state cleanup so contracts will never end up "empty" and be removed, they will either exist (in any state in the contract lifecycle)
        //      and return keccak(""), or not exist (where nothing has ever been deployed at that address) and return 0.
        // TODO: With account abstraction, this may be something other than an empty hash!
        ContractType::Account => return Ok(EMPTY_EVM_HASH.into()),
        // Not found
        ContractType::NotFound => return Ok(U256::zero()),
    };

    // multihash { keccak256(bytecode) }
    let bytecode_hash: Multihash = system
        .send(
            &addr,
            crate::Method::GetBytecodeHash as u64,
            Default::default(),
            TokenAmount::zero(),
            None,
            SendFlags::READ_ONLY,
        )?
        .deserialize()?;

    let digest = bytecode_hash.digest();

    // Take the first 32 bytes of the Multihash
    let digest_len = digest.len().min(32);
    Ok(digest[..digest_len].into())
}

pub fn extcodecopy(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    addr: U256,
    dest_offset: U256,
    data_offset: U256,
    size: U256,
) -> Result<(), StatusCode> {
    let bytecode = match get_contract_type(system.rt, addr) {
        ContractType::EVM(addr) => get_evm_bytecode(system, &addr)?,
        ContractType::NotFound | ContractType::Account | ContractType::Precompile => Vec::new(),
        // calling EXTCODECOPY on native actors results with a single byte 0xFE which solidtiy uses for its `assert`/`throw` methods
        // and in general invalid EVM bytecode
        _ => vec![0xFE],
    };

    copy_to_memory(&mut state.memory, dest_offset, size, data_offset, bytecode.as_slice(), true)
}

#[derive(Debug)]
pub enum ContractType {
    Precompile,
    /// EVM ID Address and the CID of the actor (not the bytecode)
    EVM(Address),
    Native(Cid),
    Account,
    NotFound,
}

/// Resolves an address to the address type
pub fn get_contract_type<RT: Runtime>(rt: &RT, addr: U256) -> ContractType {
    let addr: EthAddress = addr.into();
    // precompiles cant be resolved by the FVM
    if Precompiles::<RT>::is_precompile(&addr.as_evm_word()) {
        return ContractType::Precompile;
    }

    addr.try_into()
        .ok() // into filecoin address
        .and_then(|addr| rt.resolve_address(&addr)) // resolve actor id
        .and_then(|id| rt.get_actor_code_cid(&id).map(|cid| (id, cid))) // resolve code cid
        .map(|(id, cid)| match rt.resolve_builtin_actor_type(&cid) {
            // TODO part of current account abstraction hack where placeholders are accounts
            Some(Type::Account | Type::Placeholder | Type::EthAccount) => ContractType::Account,
            Some(Type::EVM) => ContractType::EVM(Address::new_id(id)),
            // remaining builtin actors are native
            _ => ContractType::Native(cid),
        })
        .unwrap_or(ContractType::NotFound)
}

pub fn get_evm_bytecode_cid(system: &mut System<impl Runtime>, addr: &Address) -> Result<Cid, ActorError> {
    Ok(system
        .send(addr, crate::Method::GetBytecode as u64, Default::default(), TokenAmount::zero(), None, SendFlags::READ_ONLY)?
        .deserialize()?)
}

pub fn get_evm_bytecode(system: &mut System<impl Runtime>, addr: &Address) -> Result<Vec<u8>, StatusCode> {
    let cid = get_evm_bytecode_cid(system, addr)?;
    let raw_bytecode = system.rt
        .store()
        .get(&cid) // TODO this is inefficient; should call stat here.
        .map_err(|e| StatusCode::InternalError(format!("failed to get bytecode block: {}", &e)))?
        .ok_or_else(|| ActorError::not_found("bytecode block not found".to_string()))?;
    Ok(raw_bytecode)
}

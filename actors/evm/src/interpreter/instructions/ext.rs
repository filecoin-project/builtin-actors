use crate::interpreter::instructions::memory::copy_to_memory;
use crate::interpreter::precompiles::Precompiles;
use crate::BytecodeHash;
use cid::Cid;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::ActorError;
use fil_actors_runtime::{deserialize_block, AsActorError};
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::{address::Address, econ::TokenAmount};
use num_traits::Zero;
use {
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

pub fn extcodesize(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    addr: U256,
) -> Result<U256, ActorError> {
    // TODO (M2.2) we're fetching the entire block here just to get its size. We should instead use
    //  the ipld::block_stat syscall, but the Runtime nor the Blockstore expose it.
    //  Tracked in https://github.com/filecoin-project/ref-fvm/issues/867
    let len = match get_contract_type(system.rt, &addr.into()) {
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
) -> Result<U256, ActorError> {
    let addr = match get_contract_type(system.rt, &addr.into()) {
        ContractType::EVM(a) => a,
        // _Technically_ since we have native "bytecode" set as 0xfe this is valid, though we cant differentiate between different native actors.
        ContractType::Native(_) => return Ok(BytecodeHash::NATIVE_ACTOR.into()),
        // Precompiles "exist" and therefore aren't empty (although spec-wise they can be either 0 or keccak("") ).
        ContractType::Precompile => return Ok(BytecodeHash::EMPTY.into()),
        // NOTE: There may be accounts that in EVM would be considered "empty" (as defined in EIP-161) and give 0, but we will instead return keccak("").
        //      The FVM does not have chain state cleanup so contracts will never end up "empty" and be removed, they will either exist (in any state in the contract lifecycle)
        //      and return keccak(""), or not exist (where nothing has ever been deployed at that address) and return 0.
        // TODO: With account abstraction, this may be something other than an empty hash!
        ContractType::Account => return Ok(BytecodeHash::EMPTY.into()),
        // Not found
        ContractType::NotFound => return Ok(U256::zero()),
    };

    // multihash { keccak256(bytecode) }
    let bytecode_hash: BytecodeHash = deserialize_block(system.send(
        &addr,
        crate::Method::GetBytecodeHash as u64,
        Default::default(),
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
    )?)?;
    Ok(bytecode_hash.into())
}

pub fn extcodecopy(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    addr: U256,
    dest_offset: U256,
    data_offset: U256,
    size: U256,
) -> Result<(), ActorError> {
    let bytecode = match get_contract_type(system.rt, &addr.into()) {
        ContractType::EVM(addr) => get_evm_bytecode(system, &addr)?,
        ContractType::NotFound | ContractType::Account | ContractType::Precompile => Vec::new(),
        // calling EXTCODECOPY on native actors results with a single byte 0xFE which solidtiy uses for its `assert`/`throw` methods
        // and in general invalid EVM bytecode
        _ => vec![0xFE],
    };

    copy_to_memory(&mut state.memory, dest_offset, size, data_offset, bytecode.as_slice(), true)
}

#[derive(Debug)]
#[allow(clippy::upper_case_acronyms)]
pub enum ContractType {
    Precompile,
    /// EVM ID Address and the CID of the actor (not the bytecode)
    EVM(Address),
    Native(Cid),
    Account,
    NotFound,
}

/// Resolves an address to the address type
pub fn get_contract_type<RT: Runtime>(rt: &RT, addr: &EthAddress) -> ContractType {
    // precompiles cant be resolved by the FVM
    // addresses passed in precompile range will be returned as NotFound; EAM asserts that no actors can be deployed in the precompile reserved range
    if Precompiles::<RT>::is_precompile(addr) {
        return ContractType::Precompile;
    }

    let addr: Address = addr.into();
    rt.resolve_address(&addr) // resolve actor id
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

pub fn get_evm_bytecode_cid(
    system: &mut System<impl Runtime>,
    addr: &Address,
) -> Result<Option<Cid>, ActorError> {
    deserialize_block(system.send(
        addr,
        crate::Method::GetBytecode as u64,
        Default::default(),
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
    )?)
}

pub fn get_evm_bytecode(
    system: &mut System<impl Runtime>,
    addr: &Address,
) -> Result<Vec<u8>, ActorError> {
    if let Some(cid) = get_evm_bytecode_cid(system, addr)? {
        let raw_bytecode = system
            .rt
            .store()
            .get(&cid) // TODO this is inefficient; should call stat here.
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get bytecode block")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "bytecode block not found")?;
        Ok(raw_bytecode)
    } else {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use crate::evm_unit_test;
    use crate::BytecodeHash;
    use cid::Cid;
    use fil_actors_evm_shared::uints::U256;
    use fil_actors_runtime::runtime::Primitives;
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    use fvm_ipld_blockstore::Blockstore;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::address::Address as FilAddress;
    use fvm_shared::crypto::hash::SupportedHashes;
    use fvm_shared::error::ExitCode;
    use fvm_shared::sys::SendFlags;
    use num_traits::Zero;

    #[test]
    fn test_extcodesize() {
        evm_unit_test! {
            (rt) {
                rt.in_call = true;

                let addr = FilAddress::new_id(1001);
                rt.set_address_actor_type(addr, *EVM_ACTOR_CODE_ID);

                let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
                let bytecode = vec![0x01, 0x02, 0x03, 0x04];
                rt.store.put_keyed(&bytecode_cid, bytecode.as_slice()).unwrap();

                rt.expect_send(
                    addr,
                    crate::Method::GetBytecode as u64,
                    Default::default(),
                    TokenAmount::zero(),
                    None,
                    SendFlags::READ_ONLY,
                    IpldBlock::serialize_cbor(&bytecode_cid).unwrap(),
                    ExitCode::OK,
                    None,
                );
            }
            (m) {
                EXTCODESIZE;
            }
            m.state.stack.push(EthAddress::from_id(1001).as_evm_word()).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(4));
        };
    }

    #[test]
    fn test_extcodesize_nonexist() {
        evm_unit_test! {
            (rt) {
                rt.in_call = true;
            }
            (m) {
                EXTCODESIZE;
            }
            m.state.stack.push(EthAddress::from_id(1001).as_evm_word()).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0));
        };
    }

    #[test]
    fn test_extcodecopy() {
        let bytecode = vec![0x01, 0x02, 0x03, 0x04];

        evm_unit_test! {
            (rt) {
                rt.in_call = true;

                let addr = FilAddress::new_id(1001);
                rt.set_address_actor_type(addr, *EVM_ACTOR_CODE_ID);
                let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
                rt.store.put_keyed(&bytecode_cid, bytecode.as_slice()).unwrap();

                rt.expect_send(
                    addr,
                    crate::Method::GetBytecode as u64,
                    Default::default(),
                    TokenAmount::zero(),
                    None,
                    SendFlags::READ_ONLY,
                    IpldBlock::serialize_cbor(&bytecode_cid).unwrap(),
                    ExitCode::OK,
                    None,
                );
            }
            (m) {
                EXTCODECOPY;
            }
            m.state.stack.push(U256::from(4)).unwrap();  // length
            m.state.stack.push(U256::from(0)).unwrap(); // offset
            m.state.stack.push(U256::from(0)).unwrap(); // destOffset
            m.state.stack.push(EthAddress::from_id(1001).as_evm_word()).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);
            assert_eq!(&m.state.memory[0..4], &bytecode);
        };
    }

    #[test]
    fn test_extcodecopy_partial() {
        let bytecode = vec![0x01, 0x02, 0x03, 0x04];

        evm_unit_test! {
            (rt) {
                rt.in_call = true;

                let addr = FilAddress::new_id(1001);
                rt.set_address_actor_type(addr, *EVM_ACTOR_CODE_ID);
                let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
                rt.store.put_keyed(&bytecode_cid, bytecode.as_slice()).unwrap();

                rt.expect_send(
                    addr,
                    crate::Method::GetBytecode as u64,
                    Default::default(),
                    TokenAmount::zero(),
                    None,
                    SendFlags::READ_ONLY,
                    IpldBlock::serialize_cbor(&bytecode_cid).unwrap(),
                    ExitCode::OK,
                    None,
                );
            }
            (m) {
                EXTCODECOPY;
            }
            m.state.stack.push(U256::from(3)).unwrap();  // length
            m.state.stack.push(U256::from(1)).unwrap(); // offset
            m.state.stack.push(U256::from(0)).unwrap(); // destOffset
            m.state.stack.push(EthAddress::from_id(1001).as_evm_word()).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);
            assert_eq!(m.state.memory[0..3], bytecode[1..4]);
        };
    }

    #[test]
    fn test_extcodecopy_oob() {
        let bytecode = vec![0x01, 0x02, 0x03, 0x04];

        evm_unit_test! {
            (rt) {
                rt.in_call = true;

                let addr = FilAddress::new_id(1001);
                rt.set_address_actor_type(addr, *EVM_ACTOR_CODE_ID);
                let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
                rt.store.put_keyed(&bytecode_cid, bytecode.as_slice()).unwrap();

                rt.expect_send(
                    addr,
                    crate::Method::GetBytecode as u64,
                    Default::default(),
                    TokenAmount::zero(),
                    None,
                    SendFlags::READ_ONLY,
                    IpldBlock::serialize_cbor(&bytecode_cid).unwrap(),
                    ExitCode::OK,
                    None,
                );
            }
            (m) {
                EXTCODECOPY;
            }
            m.state.stack.push(U256::from(4)).unwrap();  // length
            m.state.stack.push(U256::from(1)).unwrap(); // offset
            m.state.stack.push(U256::from(0)).unwrap(); // destOffset
            m.state.stack.push(EthAddress::from_id(1001).as_evm_word()).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);
            assert_eq!(m.state.memory[0..3], bytecode[1..4]);
        };
    }

    #[test]
    fn test_extcodehash() {
        #[allow(unused_assignments)]
        let mut bytecode_hash = None;

        evm_unit_test! {
            (rt) {
                rt.in_call = true;

                let addr = FilAddress::new_id(1001);
                rt.set_address_actor_type(addr, *EVM_ACTOR_CODE_ID);
                let bytecode = vec![0x01, 0x02, 0x03, 0x04];
                let hash = BytecodeHash::try_from(rt.hash(SupportedHashes::Keccak256, &bytecode).as_slice()).unwrap();
                bytecode_hash = Some(hash);

                rt.expect_send(
                    addr,
                    crate::Method::GetBytecodeHash as u64,
                    Default::default(),
                    TokenAmount::zero(),
                    None,
                    SendFlags::READ_ONLY,
                    IpldBlock::serialize_cbor(&hash).unwrap(),
                    ExitCode::OK,
                    None,
                );
            }
            (m) {
                EXTCODEHASH;
            }
            m.state.stack.push(EthAddress::from_id(1001).as_evm_word()).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(bytecode_hash.unwrap()));
        };
    }

    #[test]
    fn test_extcodehash_nonexist() {
        evm_unit_test! {
            (rt) {
                rt.in_call = true;
            }
            (m) {
                EXTCODEHASH;
            }
            m.state.stack.push(EthAddress::from_id(1001).as_evm_word()).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0));
        };
    }
}

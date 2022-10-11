use fvm_shared::{bigint::{self, BigUint}, address::Address};

use crate::interpreter::{address::EthAddress, U256};

use super::memory::{get_memory_region, MemoryRegion};
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn create<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
    create2: bool,
) -> Result<(), StatusCode> {
    let ExecutionState {stack, memory, ..} = state;
    // readonly things?

    // create2
    if create2 {
        // TODO, endowment can't be implemented till abstract account send funds is avaliable 
        let endowment = stack.pop().into();

        let offset = stack.pop();
        let size = stack.pop();
        let input_region = get_memory_region(memory, offset, size)
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;
    
        let salt = stack.pop();
    
        
        let gas = platform.rt.gas_available();
    
        let stackvalue = size;
    
        // endowment bigint?
        let salt = {
            let mut buf = [0u8; 32];
            // TODO make sure this is the right encoding
            salt.to_little_endian(&mut buf);
            buf
        };

        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            // TODO: ERR
            &[]
        };
        // call into Ethereum Address Manager to make the address
        // call_create2(platform, 0, input_data, 0, endowment, salt).unwrap();
    
        // errs
    } else {
        // create1
    }
    
    todo!()
}


struct Create2Ret {
    out: Vec<u8>,
    // f4 address
    addr: Address,
    // todo gas num type
    leftover_gas: i64,
}

#[inline]
pub fn selfdestruct<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _system: &'r mut System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let beneficiary_addr = EthAddress::try_from(state.stack.pop())?;
    let id_addr = beneficiary_addr.as_id_address().expect("no support for non-ID addresses yet");
    state.selfdestroyed = Some(id_addr);
    Ok(())
}

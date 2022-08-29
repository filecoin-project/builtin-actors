use {
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn extcodesize<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    // TODO
    //  1. call actor::get_actor_code_cid
    //  2. check that it matches our code CID (it's an EVM actor)
    //  3. call GetEvmBytecode method, returns the CID of the EVM bytecode block
    //  4. open the block
    //  5. return the length
    todo!()
}

pub fn extcodehash<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    // TODO

    todo!();
}

pub fn extcodecopy<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!();
}

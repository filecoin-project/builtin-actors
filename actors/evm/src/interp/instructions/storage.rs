use {
    crate::interp::system::{AccessStatus, StorageStatus},
    crate::interp::ExecutionState,
    crate::interp::StatusCode,
    crate::interp::System,
    fvm_ipld_blockstore::Blockstore,
};

pub(crate) const COLD_SLOAD_COST: u16 = 2100;
pub(crate) const _COLD_ACCOUNT_ACCESS_COST: u16 = 2600;
pub(crate) const WARM_STORAGE_READ_COST: u16 = 100;

#[inline(always)]
fn ok_or_out_of_gas(gas_left: i64) -> Result<(), StatusCode> {
    match gas_left >= 0 {
        true => Ok(()),
        false => Err(StatusCode::OutOfGas),
    }
}

#[inline]
pub fn sload<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
) -> Result<(), StatusCode> {
    todo!();
}

#[inline]
pub fn sstore<'r, BS: Blockstore>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS>,
) -> Result<(), StatusCode> {
    if state.message.is_static {
        return Err(StatusCode::StaticModeViolation);
    }

    if state.gas_left <= 2300 {
        return Err(StatusCode::OutOfGas);
    }

    let location = state.stack.pop();
    let value = state.stack.pop();

    let mut cost = 0;
    if platform.access_storage(state.message.recipient, location) == AccessStatus::Cold {
        cost = COLD_SLOAD_COST;
    }

    cost = match platform.set_storage(state.message.recipient, location, value)? {
        StorageStatus::Unchanged | StorageStatus::ModifiedAgain => cost + WARM_STORAGE_READ_COST,
        StorageStatus::Modified | StorageStatus::Deleted => cost + 5000 - COLD_SLOAD_COST,
        StorageStatus::Added => cost + 20000,
    };

    state.gas_left -= i64::from(cost);
    ok_or_out_of_gas(state.gas_left)
}

#[inline]
pub fn balance<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
) -> Result<(), StatusCode> {
    todo!()
}

#[inline]
pub fn selfbalance<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
) -> Result<(), StatusCode> {
    todo!()
}

#[inline]
pub fn extcodesize<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
) -> Result<(), StatusCode> {
    todo!()
}

pub fn extcodehash<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
) -> Result<(), StatusCode> {
    todo!();
}

#[inline]
pub fn create<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
    _create2: bool,
) -> Result<(), StatusCode> {
    todo!()
}

#[inline]
pub fn selfdestruct<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
) -> Result<(), StatusCode> {
    todo!()
}

use std::{marker::PhantomData, num::TryFromIntError};

use fil_actors_runtime::runtime::Runtime;
use fvm_shared::{address::Address, econ::TokenAmount};
use substrate_bn::{CurveError, FieldError, GroupError};

use crate::reader::OverflowError;

use super::{address::EthAddress, instructions::call::CallKind, System, U256};
mod evm;
mod fvm;

use evm::{blake2f, ec_add, ec_mul, ec_pairing, ec_recover, identity, modexp, ripemd160, sha256};
use fvm::{call_actor, call_actor_id, lookup_delegated_address, resolve_address};

type PrecompileFn<RT> = fn(&mut System<RT>, &[u8], PrecompileContext) -> PrecompileResult;
pub type PrecompileResult = Result<Vec<u8>, PrecompileError>;

pub const NATIVE_PRECOMPILE_ADDRESS_PREFIX: u8 = 0xFE;

struct PrecompileTable<RT: Runtime, const N: usize>([Option<PrecompileFn<RT>>; N]);

impl<RT: Runtime, const N: usize> PrecompileTable<RT, N> {
    /// Tries to lookup Precompile, None if empty slot or out of bounds.
    /// Last byte of precompile address - 1 is the index.
    fn get(&self, index: usize) -> Option<PrecompileFn<RT>> {
        self.0.get(index).and_then(|i| i.as_ref()).copied()
    }
}

pub fn is_reserved_precompile_address(addr: &EthAddress) -> bool {
    let [prefix, middle @ .., index] = addr.0;
    (prefix == 0x00 || prefix == NATIVE_PRECOMPILE_ADDRESS_PREFIX)
        && middle == [0u8; 18]
        && index > 0
}

pub struct Precompiles<RT>(PhantomData<RT>);

impl<RT: Runtime> Precompiles<RT> {
    /// FEVM specific precompiles (0xfe prefix)
    const NATIVE_PRECOMPILES: PrecompileTable<RT, 5> = PrecompileTable([
        Some(resolve_address::<RT>),          // 0xfe00..01
        Some(lookup_delegated_address::<RT>), // 0xfe00..02
        Some(call_actor::<RT>),               // 0xfe00..03
        None,                                 // 0xfe00..04 DISABLED
        Some(call_actor_id::<RT>),            // 0xfe00..05
    ]);

    /// EVM specific precompiles
    const EVM_PRECOMPILES: PrecompileTable<RT, 9> = PrecompileTable([
        Some(ec_recover::<RT>), // 0x01 ecrecover
        Some(sha256::<RT>),     // 0x02 SHA2-256
        Some(ripemd160::<RT>),  // 0x03 ripemd160
        Some(identity::<RT>),   // 0x04 identity
        Some(modexp::<RT>),     // 0x05 modexp
        Some(ec_add::<RT>),     // 0x06 ecAdd
        Some(ec_mul::<RT>),     // 0x07 ecMul
        Some(ec_pairing::<RT>), // 0x08 ecPairing
        Some(blake2f::<RT>),    // 0x09 blake2f
    ]);

    fn lookup_precompile(addr: &EthAddress) -> Option<PrecompileFn<RT>> {
        let [prefix, _m @ .., index] = addr.0;
        if is_reserved_precompile_address(addr) {
            let index = index as usize - 1;
            match prefix {
                NATIVE_PRECOMPILE_ADDRESS_PREFIX => Self::NATIVE_PRECOMPILES.get(index),
                0x00 => Self::EVM_PRECOMPILES.get(index),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Call the specified precompile. This will automatically transfer any value (if non-zero) to
    /// the target contract.
    pub fn call_precompile(
        system: &mut System<RT>,
        precompile_addr: &EthAddress,
        input: &[u8],
        context: PrecompileContext,
    ) -> PrecompileResult {
        // First, try to call the precompile, if defined.
        let result = Self::lookup_precompile(precompile_addr)
            .map(|precompile_fn| precompile_fn(system, input, context))
            .transpose()?
            .unwrap_or_default();
        // Then transfer the value. We do this second because we don't want to transfer if the
        // precompile reverts.
        //
        // This shouldn't be observable as the only precompile with side-effects is the call_actor
        // precompile, and that precompile can only be called with delegatecall.
        if !context.value.is_zero() {
            let fil_addr: Address = precompile_addr.into();
            system
                .transfer(&fil_addr, TokenAmount::from(&context.value))
                .map_err(|_| PrecompileError::TransferFailed)?;
        }
        Ok(result)
    }

    /// Checks if word is an existing precompile
    #[inline]
    pub fn is_precompile(addr: &EthAddress) -> bool {
        !addr.is_null() && Self::lookup_precompile(addr).is_some()
    }
}

#[derive(Debug)]
pub enum PrecompileError {
    // EVM precompile errors
    EcErr(CurveError),
    IncorrectInputSize,
    OutOfGas,
    // FVM precompile errors
    InvalidInput,
    CallForbidden,
    TransferFailed,
}

impl From<TryFromIntError> for PrecompileError {
    fn from(_: TryFromIntError) -> Self {
        Self::InvalidInput
    }
}

impl From<OverflowError> for PrecompileError {
    fn from(_: OverflowError) -> Self {
        PrecompileError::InvalidInput
    }
}

impl From<FieldError> for PrecompileError {
    fn from(src: FieldError) -> Self {
        PrecompileError::EcErr(src.into())
    }
}

impl From<CurveError> for PrecompileError {
    fn from(src: CurveError) -> Self {
        PrecompileError::EcErr(src)
    }
}

impl From<GroupError> for PrecompileError {
    fn from(_: GroupError) -> Self {
        PrecompileError::EcErr(CurveError::NotMember)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct PrecompileContext {
    pub call_type: CallKind,
    pub gas_limit: u64,
    pub value: U256,
}

/// Native Type of a given contract
#[repr(u32)]
pub enum NativeType {
    NonExistent = 0,
    // user actors are flattened to "system"
    /// System includes any singletons not otherwise defined.
    System = 1,
    Placeholder = 2,
    Account = 3,
    StorageProvider = 4,
    EVMContract = 5,
    OtherTypes = 6,
}

#[cfg(test)]
mod test {
    use fil_actors_runtime::test_utils::MockRuntime;

    use crate::interpreter::{address::EthAddress, precompiles::is_reserved_precompile_address};

    use super::Precompiles;

    #[test]
    fn is_native_precompile() {
        let addr = EthAddress(hex_literal::hex!("fe00000000000000000000000000000000000001"));
        assert!(Precompiles::<MockRuntime>::is_precompile(&addr));
        assert!(is_reserved_precompile_address(&addr));
    }

    #[test]
    fn is_evm_precompile() {
        let addr = EthAddress(hex_literal::hex!("0000000000000000000000000000000000000001"));
        assert!(Precompiles::<MockRuntime>::is_precompile(&addr));
        assert!(is_reserved_precompile_address(&addr));
    }

    #[test]
    fn is_over_precompile() {
        let addr = EthAddress(hex_literal::hex!("ff00000000000000000000000000000000000001"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&addr));
        assert!(!is_reserved_precompile_address(&addr));
    }

    #[test]
    fn zero_addr_precompile() {
        let eth_addr = EthAddress(hex_literal::hex!("fe00000000000000000000000000000000000000"));
        let native_addr = EthAddress(hex_literal::hex!("0000000000000000000000000000000000000000"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&eth_addr));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&native_addr));
        assert!(!is_reserved_precompile_address(&eth_addr));
        assert!(!is_reserved_precompile_address(&native_addr));
    }

    #[test]
    fn between_precompile() {
        let addr = EthAddress(hex_literal::hex!("a000000000000000000000000000000000000001"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&addr));
        assert!(!is_reserved_precompile_address(&addr));
    }

    #[test]
    fn bad_index() {
        let eth_addr = EthAddress(hex_literal::hex!("fe00000000000000000000000000000000000020"));
        let native_addr = EthAddress(hex_literal::hex!("0000000000000000000000000000000000000020"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&eth_addr));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&native_addr));
        // reserved doesn't check index is within range
        assert!(is_reserved_precompile_address(&eth_addr));
        assert!(is_reserved_precompile_address(&native_addr));
    }
}

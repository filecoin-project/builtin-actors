use std::marker::PhantomData;

use fil_actors_runtime::runtime::Runtime;
use substrate_bn::{CurveError, GroupError};

use super::{instructions::call::CallKind, StatusCode, System, U256};

mod evm;
mod fvm;
pub mod parameter;

use evm::{blake2f, ec_add, ec_mul, ec_pairing, ec_recover, identity, modexp, ripemd160, sha256};
use fvm::{call_actor, get_actor_type, lookup_delegated_address, resolve_address};

// really I'd want to have context as a type parameter, but since the table we generate must have the same types (or dyn) its messy
type PrecompileFn<RT> = unsafe fn(*mut System<RT>, &[u8], PrecompileContext) -> PrecompileResult;
pub type PrecompileResult = Result<Vec<u8>, PrecompileError>;

pub const NATIVE_PRECOMPILE_ADDRESS_PREFIX: u8 = 0xFE;

macro_rules! precompiles {
    ($($precompile:ident,)*) => {
        mod trampolines {
            use fil_actors_runtime::runtime::Runtime;
            use crate::System;
            use super::{PrecompileContext, PrecompileResult};
            $(
                #[inline(always)]
                pub unsafe fn $precompile<RT: Runtime>(s: *mut System<RT>, inp: &[u8], ctx: PrecompileContext) -> PrecompileResult {
                    super::$precompile(&mut *s, inp, ctx)
                }
            )*
        }
        [
            $(trampolines::$precompile,)*
        ]
    }
}

/// Generates a list of precompile smart contracts, index + 1 is the address.
const fn gen_evm_precompiles<RT: Runtime>() -> [PrecompileFn<RT>; 9] {
    precompiles! {
        ec_recover, // 0x01 ecrecover
        sha256,     // 0x02 SHA2-256
        ripemd160,  // 0x03 ripemd160
        identity,   // 0x04 identity
        modexp,     // 0x05 modexp
        ec_add,     // 0x06 ecAdd
        ec_mul,     // 0x07 ecMul
        ec_pairing, // 0x08 ecPairing
        blake2f,    // 0x09 blake2f
    }
}

const fn gen_native_precompiles<RT: Runtime>() -> [PrecompileFn<RT>; 4] {
    precompiles! {
        resolve_address,            // 0xfe00..01 resolve_address
        lookup_delegated_address,   // 0xfe00..02 lookup_delegated_address
        call_actor,                 // 0xfe00..03 call_actor
        get_actor_type,             // 0xfe00..04 get_actor_type
    }
}

pub fn is_reserved_precompile_address(addr: [u8; 20]) -> bool {
    let [prefix, middle @ .., index] = addr;
    (prefix == 0x00 || prefix == NATIVE_PRECOMPILE_ADDRESS_PREFIX)
        && middle == [0u8; 18]
        && index > 0
}

pub struct Precompiles<RT>(PhantomData<RT>);

impl<RT: Runtime> Precompiles<RT> {
    const EVM_PRECOMPILES: [PrecompileFn<RT>; 9] = gen_evm_precompiles();
    const NATIVE_PRECOMPILES: [PrecompileFn<RT>; 4] = gen_native_precompiles();

    fn lookup_precompile(addr: &[u8; 32]) -> Option<PrecompileFn<RT>> {
        // unrwap will never panic, 32 - 12 = 20
        let addr: [u8; 20] = addr[12..].try_into().unwrap();
        let [prefix, _m @ .., index] = addr;
        if is_reserved_precompile_address(addr) {
            let index = index as usize - 1;
            match prefix {
                NATIVE_PRECOMPILE_ADDRESS_PREFIX => Self::NATIVE_PRECOMPILES.get(index),
                0x00 => Self::EVM_PRECOMPILES.get(index),
                _ => None,
            }
            .copied()
        } else {
            None
        }
    }

    /// Precompile Context will be flattened to None if not calling the call_actor precompile.
    /// Panics if address is not a precompile.
    pub fn call_precompile(
        system: &mut System<RT>,
        precompile_addr: U256,
        input: &[u8],
        context: PrecompileContext,
    ) -> PrecompileResult {
        unsafe {
            Self::lookup_precompile(&precompile_addr.to_bytes()).unwrap()(system, input, context)
        }
    }

    /// Checks if word represents
    #[inline]
    pub fn is_precompile(addr: &U256) -> bool {
        !addr.is_zero() && Self::lookup_precompile(&addr.to_bytes()).is_some()
    }
}

#[derive(Debug)]
pub enum PrecompileError {
    EcErr(CurveError),
    EcGroupErr(GroupError),
    InvalidInput,
    CallForbidden,
    IncorrectInputSize,
    OutOfGas,
    CallActorError(StatusCode),
}

impl From<PrecompileError> for StatusCode {
    fn from(src: PrecompileError) -> Self {
        match src {
            PrecompileError::CallActorError(e) => e,
            _ => StatusCode::PrecompileFailure,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PrecompileContext {
    pub call_type: CallKind,
    pub gas_limit: u64,
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

impl NativeType {
    fn word_vec(self) -> Vec<u8> {
        U256::from(self as u32).to_bytes().to_vec()
    }
}

#[cfg(test)]
mod test {
    use fil_actors_runtime::test_utils::MockRuntime;

    use crate::interpreter::{address::EthAddress, precompiles::is_reserved_precompile_address};

    use super::Precompiles;

    #[test]
    fn is_native_precompile() {
        let addr = EthAddress(hex_literal::hex!("fe00000000000000000000000000000000000001"));
        assert!(Precompiles::<MockRuntime>::is_precompile(&addr.as_evm_word()));
        assert!(is_reserved_precompile_address(addr.0));
    }

    #[test]
    fn is_evm_precompile() {
        let addr = EthAddress(hex_literal::hex!("0000000000000000000000000000000000000001"));
        assert!(Precompiles::<MockRuntime>::is_precompile(&addr.as_evm_word()));
        assert!(is_reserved_precompile_address(addr.0));
    }

    #[test]
    fn is_over_precompile() {
        let addr = EthAddress(hex_literal::hex!("ff00000000000000000000000000000000000001"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&addr.as_evm_word()));
        assert!(!is_reserved_precompile_address(addr.0));
    }

    #[test]
    fn zero_addr_precompile() {
        let eth_addr = EthAddress(hex_literal::hex!("fe00000000000000000000000000000000000000"));
        let native_addr = EthAddress(hex_literal::hex!("0000000000000000000000000000000000000000"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&eth_addr.as_evm_word()));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&native_addr.as_evm_word()));
        assert!(!is_reserved_precompile_address(eth_addr.0));
        assert!(!is_reserved_precompile_address(native_addr.0));
    }

    #[test]
    fn between_precompile() {
        let addr = EthAddress(hex_literal::hex!("a000000000000000000000000000000000000001"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&addr.as_evm_word()));
        assert!(!is_reserved_precompile_address(addr.0));
    }

    #[test]
    fn bad_index() {
        let eth_addr = EthAddress(hex_literal::hex!("fe00000000000000000000000000000000000020"));
        let native_addr = EthAddress(hex_literal::hex!("0000000000000000000000000000000000000020"));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&eth_addr.as_evm_word()));
        assert!(!Precompiles::<MockRuntime>::is_precompile(&native_addr.as_evm_word()));
        // reserved doesn't check index is within range
        assert!(is_reserved_precompile_address(eth_addr.0));
        assert!(is_reserved_precompile_address(native_addr.0));
    }
}

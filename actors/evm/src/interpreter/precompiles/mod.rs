use std::marker::PhantomData;

use fil_actors_runtime::runtime::Runtime;
use substrate_bn::{CurveError, GroupError};

use super::{instructions::call::CallKind, StatusCode, System, U256};

mod evm;
mod fvm;
pub mod parameter;

use evm::{blake2f, ec_add, ec_mul, ec_pairing, ec_recover, identity, modexp, ripemd160, sha256};
use fvm::{call_actor, get_actor_type, get_randomness, lookup_delegated_address, resolve_address};

// really I'd want to have context as a type parameter, but since the table we generate must have the same types (or dyn) its messy
type PrecompileFn<RT> = unsafe fn(*mut System<RT>, &[u8], PrecompileContext) -> PrecompileResult;
pub type PrecompileResult = Result<Vec<u8>, PrecompileError>; // TODO i dont like vec

/// Generates a list of precompile smart contracts, index + 1 is the address. (another option is to make an enum)
const fn gen_precompiles<RT: Runtime>() -> [PrecompileFn<RT>; 14] {
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

    precompiles! {
        ec_recover, // ecrecover 0x01
        sha256,     // SHA2-256 0x02
        ripemd160,  // ripemd160 0x03
        identity,   // identity 0x04
        modexp,     // modexp 0x05
        ec_add,     // ecAdd 0x06
        ec_mul,     // ecMul 0x07
        ec_pairing, // ecPairing 0x08
        blake2f,    // blake2f 0x09
        // FIL precompiles
        resolve_address,    // resolve_address 0x0a
        lookup_delegated_address,     // lookup_delegated_address 0x0b
        get_actor_type,     // get actor type 0x0c
        get_randomness,     // rand 0x0d
        call_actor,         // call_actor 0x0e
    }
}

pub struct Precompiles<RT>(PhantomData<RT>);

impl<RT: Runtime> Precompiles<RT> {
    const PRECOMPILES: [PrecompileFn<RT>; 14] = gen_precompiles();
    const MAX_PRECOMPILE: U256 = {
        let mut limbs = [0u64; 4];
        limbs[0] = Self::PRECOMPILES.len() as u64;
        U256(limbs)
    };

    // Precompile Context will be flattened to None if not calling the call_actor precompile
    pub fn call_precompile(
        system: &mut System<RT>,
        precompile_addr: U256,
        input: &[u8],
        context: PrecompileContext,
    ) -> PrecompileResult {
        unsafe { Self::PRECOMPILES[precompile_addr.0[0] as usize - 1](system, input, context) }
    }

    #[inline]
    pub fn is_precompile(addr: &U256) -> bool {
        !addr.is_zero() && addr <= &Self::MAX_PRECOMPILE
    }
}

#[derive(Debug)]
pub enum PrecompileError {
    EcErr(CurveError),
    EcGroupErr(GroupError),
    InvalidInput, // TODO merge with below?
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
    Embryo = 2,
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

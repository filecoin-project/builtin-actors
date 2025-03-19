use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;

/// BLS12_MAP_FP_TO_G1 precompile
/// Implements mapping of field element to G1 point according to EIP-2537
#[allow(dead_code,unused_variables)]
pub fn bls12_map_fp_to_g1<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
}
use std::collections::HashMap;

use fil_actors_runtime::{Map, MessageAccumulator};
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::bigint_ser::BigIntDe;
use num_traits::Signed;

use crate::{DataCap, State};

pub struct StateSummary {
    pub verifiers: HashMap<Address, DataCap>,
}

/// Checks internal invariants of verified registry state.
pub fn check_state_invariants<BS: Blockstore>(
    state: &State,
    store: &BS,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();

    // check verifiers
    let mut all_verifiers = HashMap::new();
    match Map::<_, BigIntDe>::load(&state.verifiers, store) {
        Ok(verifiers) => {
            let ret = verifiers.for_each(|key, cap| {
                let verifier = Address::from_bytes(key)?;
                let cap = &cap.0;

                acc.require(
                    verifier.protocol() == Protocol::ID,
                    format!("verifier {verifier} should have ID protocol"),
                );
                acc.require(
                    !cap.is_negative(),
                    format!("verifier {verifier} cap {cap} is negative"),
                );
                all_verifiers.insert(verifier, cap.clone());
                Ok(())
            });

            acc.require_no_error(ret, "error iterating verifiers");
        }
        Err(e) => acc.add(format!("error loading verifiers {e}")),
    }

    (StateSummary { verifiers: all_verifiers }, acc)
}

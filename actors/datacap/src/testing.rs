use frc46_token::token::state::decode_actor_id;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::address::Protocol;
use fvm_shared::econ::TokenAmount;
use fvm_shared::ActorID;
use std::collections::HashMap;

use fil_actors_runtime::MessageAccumulator;

use crate::State;

pub struct StateSummary {
    pub balances: HashMap<ActorID, TokenAmount>,
    pub allowances: HashMap<ActorID, HashMap<ActorID, TokenAmount>>,
    pub total_supply: TokenAmount,
}

/// Checks internal invariants of data cap token actor state.
pub fn check_state_invariants<BS: Blockstore>(
    state: &State,
    store: &BS,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();
    acc.require(state.governor.protocol() == Protocol::ID, "governor must be ID address");
    let r = state.token.check_invariants(store);

    // TODO: replace this with the state summary returned by the token library
    // after that's implemented (expected in 1.2.0)
    let mut summary = StateSummary {
        balances: HashMap::new(),
        allowances: HashMap::new(),
        total_supply: state.token.supply.clone(),
    };
    match state.token.get_balance_map(store) {
        Ok(balance_map) => {
            balance_map
                .for_each(|owner, v| {
                    // Unwrapping decode here because the token library should have checked it.
                    let owner = decode_actor_id(owner).unwrap();
                    summary.balances.insert(owner, v.clone());
                    Ok(())
                })
                .unwrap();
        }
        Err(e) => acc.add(format!("error loading balances {e}")),
    }
    match state.token.get_allowances_map(store) {
        Ok(allowances_map) => {
            allowances_map
                .for_each(|owner, _| {
                    let owner = decode_actor_id(owner).unwrap();
                    let mut allowances = HashMap::<ActorID, TokenAmount>::new();
                    match state.token.get_owner_allowance_map(store, owner) {
                        Ok(allowance_map) => {
                            if let Some(allowance_map) = allowance_map {
                                allowance_map
                                    .for_each(|operator, v| {
                                        let operator = decode_actor_id(operator).unwrap();
                                        allowances.insert(operator, v.clone());
                                        Ok(())
                                    })
                                    .unwrap();
                            } else {
                                acc.add(format!("missing allowance map for {owner}"));
                            }
                        }
                        Err(e) => acc.add(format!("error loading allowances for {owner} {e}")),
                    }
                    summary.allowances.insert(owner, allowances);
                    Ok(())
                })
                .unwrap();
        }
        Err(e) => acc.add(format!("error loading allowances {e}")),
    }

    if let Err(e) = r {
        acc.add(e.to_string());
    }

    (summary, acc)
}

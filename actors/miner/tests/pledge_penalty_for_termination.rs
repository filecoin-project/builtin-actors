use fil_actor_miner::{
    pledge_penalty_for_termination_lower_bound, INITIAL_PLEDGE_FACTOR, TERMINATION_LIFETIME_CAP,
    TERMINATION_REWARD_FACTOR_DENOM, TERMINATION_REWARD_FACTOR_NUM,
};
use fil_actors_runtime::reward::FilterEstimate;
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;

fn epoch_target_reward() -> TokenAmount {
    TokenAmount::from_atto(1_u128 << 50)
}

// 1 64 GiB sector
fn qa_sector_power() -> StoragePower {
    StoragePower::from(1_u128 << 36)
}

// 1 PiB of network power, no estimated changes
fn network_qa_power() -> StoragePower {
    StoragePower::from(1_u128 << 50)
}

// exactly 1 attoFIL per byte of power, no estimated changes
fn reward_estimate() -> FilterEstimate {
    FilterEstimate::new(epoch_target_reward().atto().clone(), BigInt::zero())
}

fn power_estimate() -> FilterEstimate {
    FilterEstimate::new(network_qa_power(), BigInt::zero())
}

fn undeclared_penalty() -> TokenAmount {
    pledge_penalty_for_termination_lower_bound(
        &reward_estimate(),
        &power_estimate(),
        &qa_sector_power(),
    )
}

// TODO
// what test could we contrive?

use fil_actor_miner::{initial_pledge_for_power, pre_commit_deposit_for_power};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;
use num_traits::zero;

macro_rules! my_const {
    ($name:ident, $ret_type:ty, $value:expr) => {
        fn $name() -> $ret_type {
            $value
        }
    };
}

my_const!(epoch_target_reward, TokenAmount, zero());
my_const!(qa_sector_power, StoragePower, StoragePower::from(1u64 << 36));
my_const!(network_qa_power, StoragePower, StoragePower::from(1u64 << 10));
my_const!(baseline_power, StoragePower, network_qa_power());
my_const!(power_rate_of_change, StoragePower, StoragePower::from(1u64 << 10));
my_const!(reward_estimate, FilterEstimate, FilterEstimate::new(epoch_target_reward(), zero()));
my_const!(
    power_estimate,
    FilterEstimate,
    FilterEstimate::new(network_qa_power(), power_rate_of_change())
);
my_const!(circulating_supply, TokenAmount, zero());

#[test]
fn initial_pledge_clamped_at_one_attofil() {
    let initial_pledge = initial_pledge_for_power(
        &qa_sector_power(),
        &baseline_power(),
        &reward_estimate(),
        &power_estimate(),
        &circulating_supply(),
    );
    assert_eq!(TokenAmount::from(1), initial_pledge);
}

#[test]
fn precommit_deposit_is_clamped_at_one_attofil() {
    let precommit_deposit =
        pre_commit_deposit_for_power(&reward_estimate(), &power_estimate(), &qa_sector_power());
    assert_eq!(TokenAmount::from(1), precommit_deposit);
}

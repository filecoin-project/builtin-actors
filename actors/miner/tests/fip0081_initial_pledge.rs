use fil_actor_miner::initial_pledge_for_power;
use fil_actors_runtime::reward::FilterEstimate;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
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
my_const!(power_rate_of_change, StoragePower, StoragePower::from(1u64 << 10));
my_const!(
    reward_estimate,
    FilterEstimate,
    FilterEstimate::new(epoch_target_reward().atto().clone(), zero())
);
my_const!(
    power_estimate,
    FilterEstimate,
    FilterEstimate::new(network_qa_power(), power_rate_of_change())
);
my_const!(circulating_supply, TokenAmount, TokenAmount::from_whole(1));

// After-ramp where 'baseline power' is at 70% and `simple power` is at 30%.
#[test]
fn initial_pledge_after_ramp() {
    let initial_pledge = initial_pledge_for_power(
        &qa_sector_power(),
        &StoragePower::from(1u64 << 37),
        &reward_estimate(),
        &power_estimate(),
        &circulating_supply(),
    );
    assert_eq!(
        TokenAmount::from_atto(1) + TokenAmount::from_whole(1950).div_floor(10000),
        initial_pledge
    );
}

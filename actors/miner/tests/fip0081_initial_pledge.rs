use fil_actor_miner::initial_pledge_for_power;
use fil_actors_runtime::reward::FilterEstimate;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use num_traits::zero;
use test_case::test_case;

const SECTOR_POWER: u64 = 1 << 36;

// FIP-0081 is fully activated: additional pledge is 70% baseline and 30% simple pledge.
#[test_case(
    2 * SECTOR_POWER, SECTOR_POWER, 0,
    TokenAmount::from_whole(1), TokenAmount::from_atto(195_000_000_000_000_001u64);
    "70 percent baseline and 30 percent simple pledge"
)]
#[test_case(
    2 * SECTOR_POWER, 4 * SECTOR_POWER, 0,
    TokenAmount::from_whole(1), TokenAmount::from_atto(75_000_000_000_000_001u64);
    "network power exceeds baseline"
)]
#[test_case(
    SECTOR_POWER / 2, SECTOR_POWER / 4, 0,
    TokenAmount::from_whole(1), TokenAmount::from_atto(300_000_000_000_000_001u64);
    "sector power bounds both denominators"
)]
// floor(1.68) + floor(1.44) + 1 attoFIL minimum reward.
#[test_case(
    2 * SECTOR_POWER, SECTOR_POWER, 0,
    TokenAmount::from_atto(16), TokenAmount::from_atto(3);
    "weighted pledge terms are rounded separately"
)]
// At full network power, 20 days of reward adds 57,600 attoFIL.
#[test_case(
    2 * SECTOR_POWER, SECTOR_POWER, 1,
    TokenAmount::from_whole(1), TokenAmount::from_atto(195_000_000_000_057_600u64);
    "projected reward is added to consensus pledge"
)]
// The cap is 1 FIL per 32 GiB, rounded down per byte before multiplying by sector power.
#[test_case(
    2 * SECTOR_POWER, SECTOR_POWER, 0,
    TokenAmount::from_whole(100), TokenAmount::from_whole(1).div_floor(32u64 << 30) * SECTOR_POWER;
    "pledge is capped"
)]
fn initial_pledge_for_power_fip0081(
    baseline_power: u64,
    network_qa_power: u64,
    epoch_reward: u64,
    circulating_supply: TokenAmount,
    expected: TokenAmount,
) {
    let initial_pledge = initial_pledge_for_power(
        &StoragePower::from(SECTOR_POWER),
        &StoragePower::from(baseline_power),
        &FilterEstimate::new(epoch_reward.into(), zero()),
        &FilterEstimate::new(network_qa_power.into(), zero()),
        &circulating_supply,
    );
    assert_eq!(expected, initial_pledge);
}

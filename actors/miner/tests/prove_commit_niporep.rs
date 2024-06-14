use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{clock::ChainEpoch, error::ExitCode};

use fil_actor_miner::{Actor, Method};
use util::*;

mod util;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn prove_single_sector_ni_fail() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let expiration = seal_randomness_epoch + 1000;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(seal_randomness_epoch + 400);

    let params = h.make_prove_commit_ni_params(miner, &[100], seal_randomness_epoch, expiration, 0);

    let res = rt.call::<Actor>(
        Method::ProveCommitSectorsNI as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );

    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn prove_two_sectors_aggregate_ni() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let expiration = seal_randomness_epoch + 1000;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(seal_randomness_epoch + 400);

    let sector_nums = (0..2).collect::<Vec<_>>();
    let params =
        h.make_prove_commit_ni_params(miner, &sector_nums, seal_randomness_epoch, expiration, 0);

    let res = h.prove_commit_sectors_ni(&rt, params, true);

    assert!(res.is_ok());

    let deadlines = h.get_state(&rt).load_deadlines(&rt.store).unwrap();
    let deadline = deadlines.load_deadline(&rt.store, 0).unwrap();
    assert_eq!(deadline.live_sectors, 2);
}

#[test]
fn prove_sectors_max_aggregate_ni() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let expiration = seal_randomness_epoch + 1000;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(seal_randomness_epoch + 400);

    for i in 0..3 {
        let sector_nums =
            (i * 1000..i * 1000 + rt.policy.max_aggregated_sectors_ni).collect::<Vec<_>>();
        let params = h.make_prove_commit_ni_params(
            miner,
            &sector_nums,
            seal_randomness_epoch,
            expiration,
            proving_deadline,
        );

        let res = h.prove_commit_sectors_ni(&rt, params, i == 0);

        assert!(res.is_ok());

        let deadlines = h.get_state(&rt).load_deadlines(&rt.store).unwrap();
        let deadline = deadlines.load_deadline(&rt.store, proving_deadline).unwrap();
        let partitions = deadline.partitions_amt(&rt.store).unwrap();
        let partition = partitions.get(0).unwrap().unwrap();
        let partition_sectors: Vec<u64> = partition.sectors.iter().collect();

        assert_eq!(deadline.live_sectors, (i + 1) * rt.policy.max_aggregated_sectors_ni);

        // Check if the last max_aggregated_sectors_ni sectors in partition are the ones we just committed
        assert!(partition_sectors
            .iter()
            .rev()
            .take(rt.policy.max_aggregated_sectors_ni as usize)
            .rev()
            .eq(sector_nums.iter()));
    }
}

#[test]
fn prove_too_much_sector_ni_fail() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let expiration = seal_randomness_epoch + 1000;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(seal_randomness_epoch + 400);

    let sector_nums = (0..rt.policy.max_aggregated_sectors_ni + 1).collect::<Vec<_>>();

    let params =
        h.make_prove_commit_ni_params(miner, &sector_nums, seal_randomness_epoch, expiration, 0);

    let res = rt.call::<Actor>(
        Method::ProveCommitSectorsNI as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );

    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

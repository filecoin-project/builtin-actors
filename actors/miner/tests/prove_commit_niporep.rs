use fil_actors_runtime::cbor::serialize;
use fvm_shared::{
    clock::ChainEpoch,
    commcid::cid_to_replica_commitment_v1,
    randomness::Randomness,
    sector::{AggregateSealVerifyInfo, SealVerifyInfo},
};

use fil_actor_miner::CompactCommD;
use log::info;
use util::*;

mod util;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test_log::test]
fn prove_single_sector_ni() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let expiration = seal_randomness_epoch + 1000;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(seal_randomness_epoch + 400);

    let params =
        h.make_prove_commit_ni_params(miner, &[100], seal_randomness_epoch, expiration, false);

    let interactive_randomness =
        cid_to_replica_commitment_v1(&params.sectors[0].sealed_cid).unwrap();
    let seal_verify_info = SealVerifyInfo {
        registered_proof: params.seal_proof_type,
        sector_id: fvm_shared::sector::SectorID { miner, number: params.sectors[0].sector_number },
        randomness: Randomness(vec![1; 32]),
        proof: params.sector_proofs[0].clone().into(),
        sealed_cid: params.sectors[0].sealed_cid,
        unsealed_cid: CompactCommD::empty().get_cid(params.seal_proof_type).unwrap(),
        deal_ids: Vec::new(),
        interactive_randomness: Randomness(interactive_randomness.to_vec()),
    };

    let entropy = serialize(&rt.receiver, "address for get verify info").unwrap();

    rt.expect_get_randomness_from_tickets(
        fil_actors_runtime::runtime::DomainSeparationTag::SealRandomness,
        seal_randomness_epoch,
        entropy.to_vec(),
        [1; 32],
    );

    rt.expect_batch_verify_seals(vec![seal_verify_info], Ok(vec![true]));
    rt.expect_validate_caller_addr(h.caller_addrs());

    // Call prove commit niporep
    let res = h.prove_commit_sectors_ni(&rt, params);
    info!("prove_commit_ni res: {:?}", res);

    assert!(res.is_ok());
}

#[test_log::test]
fn prove_sectors_aggregate_ni() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let expiration = seal_randomness_epoch + 1000;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(seal_randomness_epoch + 400);

    let sector_nums = [100, 101, 102, 103];
    let params =
        h.make_prove_commit_ni_params(miner, &sector_nums, seal_randomness_epoch, expiration, true);
    let randomness = Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.to_vec());

    let entropy = serialize(&rt.receiver, "address for get verify info").unwrap();

    sector_nums.iter().for_each(|_| {
        rt.expect_get_randomness_from_tickets(
            fil_actors_runtime::runtime::DomainSeparationTag::SealRandomness,
            seal_randomness_epoch,
            entropy.to_vec(),
            TEST_RANDOMNESS_ARRAY_FROM_ONE,
        )
    });

    let seal_verify_info = params
        .sectors
        .iter()
        .map(|sector| AggregateSealVerifyInfo {
            sector_number: sector.sector_number,
            randomness: randomness.clone(),
            interactive_randomness: Randomness(
                cid_to_replica_commitment_v1(&sector.sealed_cid).unwrap().to_vec(),
            ),
            sealed_cid: sector.sealed_cid.clone(),
            unsealed_cid: CompactCommD::empty().get_cid(params.seal_proof_type).unwrap(),
        })
        .collect::<Vec<_>>();

    rt.expect_aggregate_verify_seals(seal_verify_info, params.aggregate_proof.to_vec(), Ok(()));
    rt.expect_validate_caller_addr(h.caller_addrs());

    // Call prove commit niporep
    let res = h.prove_commit_sectors_ni(&rt, params);
    info!("prove_commit_ni res: {:?}", res);

    assert!(res.is_ok());
}

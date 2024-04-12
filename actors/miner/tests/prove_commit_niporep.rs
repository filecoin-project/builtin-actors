use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{
    clock::ChainEpoch,
    randomness::Randomness,
    sector::{InteractiveSealRandomness, NISealVerifyInfo, SealRandomness, SealVerifyInfo},
};

use fil_actor_miner::{Actor, CompactCommD, Method};
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
    info!("miner: {:?}", miner);

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&rt);
    // let dl_info = h.deadline(&rt);

    let params = h.make_prove_commit_ni_params(100);

    let ni_seal_verify_info = NISealVerifyInfo {
        registered_proof: params.seal_proof_type,
        sector_id: fvm_shared::sector::SectorID { miner, number: params.sectors[0].sector_number },
        randomness: Randomness(Vec::new()),
        proof: params.sector_proofs[0].clone().into(),
        sealed_cid: params.sectors[0].sealed_cid,
        unsealed_cid: CompactCommD::empty().get_cid(params.seal_proof_type).unwrap(),
    };

    rt.expect_batch_verify_ni_seals(vec![ni_seal_verify_info], Ok(vec![]));

    // Call prove commit niporep
    let res = rt.call::<Actor>(
        Method::ProveCommitSectorsNI as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );
    info!("prove_commit_ni res: {:?}", res);
}

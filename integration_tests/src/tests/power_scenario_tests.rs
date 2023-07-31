use fil_actor_init::Method as InitMethod;
use fil_actor_miner::{Method as MinerMethod, MinerConstructorParams};
use fil_actor_power::{CreateMinerParams, Method as PowerMethod};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::BytesDe;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::METHOD_SEND;
use vm_api::trace::ExpectInvocation;
use vm_api::util::serialize_ok;
use vm_api::VM;

use crate::util::assert_invariants;
use crate::{FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR};

pub fn power_create_miner_test(v: &dyn VM) {
    let owner = Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap();
    v.execute_message(
        &TEST_FAUCET_ADDR,
        &owner,
        &TokenAmount::from_atto(10_000u32),
        METHOD_SEND,
        None,
    )
    .unwrap();
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];
    let peer_id = "miner".as_bytes().to_vec();
    let post_proof = RegisteredPoStProof::StackedDRGWindow32GiBV1P1;
    let params = CreateMinerParams {
        owner,
        worker: owner,
        window_post_proof_type: post_proof,
        peer: peer_id.clone(),
        multiaddrs: multiaddrs.clone(),
    };

    let res = v
        .execute_message(
            &owner,
            &STORAGE_POWER_ACTOR_ADDR,
            &TokenAmount::from_atto(1000u32),
            PowerMethod::CreateMiner as u64,
            Some(serialize_ok(&params)),
        )
        .unwrap();

    let owner_id = v.resolve_id_address(&owner).unwrap();
    let expect = ExpectInvocation {
        // send to power actor
        from: owner_id,
        to: STORAGE_POWER_ACTOR_ADDR,
        method: PowerMethod::CreateMiner as u64,
        params: Some(IpldBlock::serialize_cbor(&params).unwrap()),
        ret: Some(res.ret),
        subinvocs: Some(vec![
            // request init actor construct miner
            ExpectInvocation {
                from: STORAGE_POWER_ACTOR_ADDR,
                to: INIT_ACTOR_ADDR,
                method: InitMethod::Exec as u64,
                subinvocs: Some(vec![ExpectInvocation {
                    // init then calls miner constructor
                    from: INIT_ACTOR_ADDR,
                    to: Address::new_id(FIRST_TEST_USER_ADDR + 1),
                    method: MinerMethod::Constructor as u64,
                    params: Some(
                        IpldBlock::serialize_cbor(&MinerConstructorParams {
                            owner,
                            worker: owner,
                            window_post_proof_type: post_proof,
                            peer_id,
                            control_addresses: vec![],
                            multi_addresses: multiaddrs,
                        })
                        .unwrap(),
                    ),
                    ..Default::default()
                }]),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    expect.matches(v.take_invocations().last().unwrap());
    assert_invariants(v, &Policy::default());
}

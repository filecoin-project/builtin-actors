use fil_actor_init::Method as InitMethod;
use fil_actor_miner::{Method as MinerMethod, MinerConstructorParams};
use fil_actor_power::{CreateMinerParams, Method as PowerMethod};
use fil_actors_runtime::cbor::serialize;

use fil_actors_runtime::{INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::METHOD_SEND;
use test_vm::{ExpectInvocation, FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR, VM};

#[test]
fn create_miner() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let owner = Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap();
    v.apply_message(
        TEST_FAUCET_ADDR,
        owner,
        TokenAmount::from(10_000u32),
        METHOD_SEND,
        RawBytes::default(),
    )
    .unwrap();
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];
    let peer_id = "miner".as_bytes().to_vec();
    let params = CreateMinerParams {
        owner,
        worker: owner,
        window_post_proof_type: RegisteredPoStProof::StackedDRGWindow32GiBV1,
        peer: peer_id.clone(),
        multiaddrs: multiaddrs.clone(),
    };

    let res = v
        .apply_message(
            owner,
            *STORAGE_POWER_ACTOR_ADDR,
            TokenAmount::from(1000u32),
            PowerMethod::CreateMiner as u64,
            params.clone(),
        )
        .unwrap();

    let expect = ExpectInvocation {
        // send to power actor
        to: *STORAGE_POWER_ACTOR_ADDR,
        method: PowerMethod::CreateMiner as u64,
        params: Some(serialize(&params, "power create miner params").unwrap()),
        code: None,
        from: None,
        ret: Some(res.ret),
        subinvocs: Some(vec![
            // request init actor construct miner
            ExpectInvocation {
                to: *INIT_ACTOR_ADDR,
                method: InitMethod::Exec as u64,
                params: None,
                code: None,
                from: None,
                ret: None,
                subinvocs: Some(vec![ExpectInvocation {
                    // init then calls miner constructor
                    to: Address::new_id(FIRST_TEST_USER_ADDR + 1),
                    method: MinerMethod::Constructor as u64,
                    params: Some(
                        serialize(
                            &MinerConstructorParams {
                                owner,
                                worker: owner,
                                window_post_proof_type:
                                    RegisteredPoStProof::StackedDRGWindow32GiBV1,
                                peer_id,
                                control_addresses: vec![],
                                multi_addresses: multiaddrs,
                            },
                            "miner constructor params",
                        )
                        .unwrap(),
                    ),
                    code: None,
                    from: None,
                    ret: None,
                    subinvocs: None,
                }]),
            },
        ]),
    };
    expect.matches(v.take_invocations().last().unwrap())
}

use fil_actor_init::Method as InitMethod;
use fil_actor_miner::ext::power::EnrollCronEventParams;
use fil_actor_miner::{Method as MinerMethod, MinerConstructorParams, PreCommitSectorParams};
use fil_actor_power::{
    CreateMinerParams, CreateMinerReturn, EnrollCronEventParams, Method as PowerMethod,
};
use fil_actors_runtime::cbor::serialize;

use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::{CRON_ACTOR_ADDR, INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredPoStProof, RegisteredSealProof};
use fvm_shared::METHOD_SEND;
use num_traits::Zero;
use test_vm::util::create_accounts;
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

#[test]
fn test_cron_tick() {
    let store = MemoryBlockstore::new();
    let mut vm = VM::new_with_singletons(&store);

    let addrs = create_accounts(&vm, 1, BigInt::from(10_000u64) * BigInt::from(10u64.pow(18)));

    // create a miner
    let miner_balance = BigInt::from(10_000u64) * BigInt::from(10u64.pow(18));
    let params = CreateMinerParams {
        owner: addrs[0],
        worker: addrs[0],
        window_post_proof_type: RegisteredPoStProof::StackedDRGWindow32GiBV1,
        // todo: not sure if these values are correct, placeholders
        peer: String::from("pid").into_bytes(),
        multiaddrs: vec![],
    };
    let ret = vm
        .apply_message(
            addrs[0],
            addrs[0],
            miner_balance,
            PowerMethod::CreateMiner as u64,
            params.clone(),
        )
        .unwrap();

    // todo: this fails; figure out how to deserialize this
    let miner_addrs: CreateMinerReturn = ret.ret.deserialize().unwrap();

    // create precommit
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1; // p1??
    let sector_number = 100;
    let sealed_cid = make_sealed_cid(b"100");
    let precommit_params = PreCommitSectorParams {
        seal_proof,
        sector_number,
        sealed_cid,
        seal_rand_epoch: vm.get_epoch() - 1,
        deal_ids: vec![],
        expiration: vm.get_epoch(), // todo
        ..Default::default()
    };
    vm.apply_message(
        addrs[0],
        miner_addrs.robust_address,
        TokenAmount::from(0),
        MinerMethod::PreCommitSector as u64,
        RawBytes::serialize(&precommit_params).unwrap(),
    )
    .unwrap();

    // find epoch of miner's next cron task (precommit:1, enrollCron:2)
    let cron_params = vm.params_for_invocation(vec![1, 2]);
    let cron_config: EnrollCronEventParams = cron_params.deserialize().unwrap();

    // create new vm at epoch 1 less than epoch requested by miner
    let v = vm.with_epoch(cron_config.event_epoch - 1);

    vm.apply_message(
        *CRON_ACTOR_ADDR,
        *STORAGE_POWER_ACTOR_ADDR,
        BigInt::zero(),
        PowerMethod::EnrollCronEvent as u64,
        // abi.Empty?
        RawBytes::new(vec![]),
    );
}

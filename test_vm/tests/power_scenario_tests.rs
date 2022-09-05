use fil_actor_init::Method as InitMethod;
use fil_actor_miner::{
    max_prove_commit_duration, Method as MinerMethod, MinerConstructorParams,
    PreCommitSectorParams, MIN_SECTOR_EXPIRATION,
};
use fil_actor_power::{CreateMinerParams, Method as PowerMethod};
use fil_actor_reward::Method as RewardMethod;
use fil_actors_runtime::cbor::serialize;

use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::{
    CRON_ACTOR_ADDR, INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;

use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredPoStProof, RegisteredSealProof};
use fvm_shared::METHOD_SEND;
use num_traits::Zero;
use test_vm::util::{
    apply_ok, create_accounts, create_miner, invariant_failure_patterns, miner_dline_info,
};
use test_vm::{ExpectInvocation, FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR, VM};

#[test]
fn create_miner_test() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let owner = Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap();
    v.apply_message(
        TEST_FAUCET_ADDR,
        owner,
        TokenAmount::from_atto(10_000u32),
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
            TokenAmount::from_atto(1000u32),
            PowerMethod::CreateMiner as u64,
            params.clone(),
        )
        .unwrap();

    let expect = ExpectInvocation {
        // send to power actor
        to: *STORAGE_POWER_ACTOR_ADDR,
        method: PowerMethod::CreateMiner as u64,
        params: Some(serialize(&params, "power create miner params").unwrap()),
        ret: Some(res.ret),
        subinvocs: Some(vec![
            // request init actor construct miner
            ExpectInvocation {
                to: *INIT_ACTOR_ADDR,
                method: InitMethod::Exec as u64,
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
                    ..Default::default()
                }]),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    expect.matches(v.take_invocations().last().unwrap());
    v.assert_state_invariants();
}

#[test]
fn test_cron_tick() {
    let store = MemoryBlockstore::new();
    let mut vm = VM::new_with_singletons(&store);

    let addrs = create_accounts(&vm, 1, TokenAmount::from_whole(10_000));

    // create a miner
    let (id_addr, robust_addr) = create_miner(
        &mut vm,
        addrs[0],
        addrs[0],
        RegisteredPoStProof::StackedDRGWindow32GiBV1,
        TokenAmount::from_whole(10_000),
    );

    // create precommit
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_number = 100;
    let sealed_cid = make_sealed_cid(b"100");
    let precommit_params = PreCommitSectorParams {
        seal_proof,
        sector_number,
        sealed_cid,
        seal_rand_epoch: vm.get_epoch() - 1,
        deal_ids: vec![],
        expiration: vm.get_epoch()
            + MIN_SECTOR_EXPIRATION
            + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap()
            + 100,
        ..Default::default()
    };

    apply_ok(
        &vm,
        addrs[0],
        robust_addr,
        TokenAmount::zero(),
        MinerMethod::PreCommitSector as u64,
        precommit_params,
    );

    // find epoch of miner's next cron task (precommit:1, enrollCron:2)
    let cron_epoch = miner_dline_info(&vm, id_addr).last() - 1;

    // create new vm at epoch 1 less than epoch requested by miner
    let v = vm.with_epoch(cron_epoch);

    // run cron and expect a call to miner and a call to update reward actor params
    apply_ok(
        &v,
        *CRON_ACTOR_ADDR,
        *STORAGE_POWER_ACTOR_ADDR,
        TokenAmount::zero(),
        PowerMethod::OnEpochTickEnd as u64,
        RawBytes::default(),
    );

    // expect miner call to be missing
    ExpectInvocation {
        // original send to storage power actor
        to: *STORAGE_POWER_ACTOR_ADDR,
        method: PowerMethod::OnEpochTickEnd as u64,
        subinvocs: Some(vec![
            // get data from reward actor for any eventual calls to confirmsectorproofsparams
            ExpectInvocation {
                to: *REWARD_ACTOR_ADDR,
                method: RewardMethod::ThisEpochReward as u64,
                ..Default::default()
            },
            // expect call to reward to update kpi
            ExpectInvocation {
                to: *REWARD_ACTOR_ADDR,
                method: RewardMethod::UpdateNetworkKPI as u64,
                from: Some(*STORAGE_POWER_ACTOR_ADDR),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().first().unwrap());

    // create new vm at cron epoch with existing state
    let v = v.with_epoch(cron_epoch + 1);

    // run cron and expect a call to miner and a a call to update reward actor params
    apply_ok(
        &v,
        *CRON_ACTOR_ADDR,
        *STORAGE_POWER_ACTOR_ADDR,
        TokenAmount::zero(),
        PowerMethod::OnEpochTickEnd as u64,
        RawBytes::default(),
    );

    let sub_invocs = vec![
        // get data from reward and power for any eventual calls to confirmsectorproofsvalid
        ExpectInvocation {
            to: *REWARD_ACTOR_ADDR,
            method: RewardMethod::ThisEpochReward as u64,
            ..Default::default()
        },
        // expect call back to miner that was set up in create miner
        ExpectInvocation {
            to: id_addr,
            method: MinerMethod::OnDeferredCronEvent as u64,
            from: Some(*STORAGE_POWER_ACTOR_ADDR),
            value: Some(TokenAmount::zero()),
            ..Default::default()
        },
        // expect call to reward to update kpi
        ExpectInvocation {
            to: *REWARD_ACTOR_ADDR,
            method: RewardMethod::UpdateNetworkKPI as u64,
            from: Some(*STORAGE_POWER_ACTOR_ADDR),
            ..Default::default()
        },
    ];

    // expect call to miner
    ExpectInvocation {
        // original send to storage power actor
        to: *STORAGE_POWER_ACTOR_ADDR,
        method: PowerMethod::OnEpochTickEnd as u64,
        subinvocs: Some(sub_invocs),
        ..Default::default()
    }
    .matches(v.take_invocations().first().unwrap());

    v.expect_state_invariants(
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
    );
}

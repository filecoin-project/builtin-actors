use fil_actor_init::Method as InitMethod;
use fil_actor_miner::{
    max_prove_commit_duration, Method as MinerMethod, MinerConstructorParams,
    PreCommitSectorParams, MIN_SECTOR_EXPIRATION,
};
use fil_actor_power::{CreateMinerParams, Method as PowerMethod};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::{CRON_ACTOR_ADDR, INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredPoStProof, RegisteredSealProof};
use fvm_shared::METHOD_SEND;
use num_traits::Zero;
use test_vm::util::{
    assert_invariants, create_accounts, create_miner, expect_invariants,
    invariant_failure_patterns, miner_dline_info,
};
use test_vm::{TestVM, FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR};
use vm_api::expects::Expect;
use vm_api::trace::ExpectInvocation;
use vm_api::util::{apply_ok, serialize_ok};
use vm_api::VM;

#[test]
fn power_create_miner() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);

    power_create_miner_test(&v);
}

fn power_create_miner_test(v: &dyn VM) {
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

#[test]
fn test_cron_tick() {
    let store = MemoryBlockstore::new();
    let vm = TestVM::<MemoryBlockstore>::new_with_singletons(&store);

    let addrs = create_accounts(&vm, 1, &TokenAmount::from_whole(10_000));

    // create a miner
    let (id_addr, robust_addr) = create_miner(
        &vm,
        &addrs[0],
        &addrs[0],
        RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
        &TokenAmount::from_whole(10_000),
    );

    // create precommit
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_number = 100;
    let sealed_cid = make_sealed_cid(b"100");
    let precommit_params = PreCommitSectorParams {
        seal_proof,
        sector_number,
        sealed_cid,
        seal_rand_epoch: vm.epoch() - 1,
        deal_ids: vec![],
        expiration: vm.epoch()
            + MIN_SECTOR_EXPIRATION
            + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap()
            + 100,
        ..Default::default()
    };

    apply_ok(
        &vm,
        &addrs[0],
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::PreCommitSector as u64,
        Some(precommit_params),
    );

    // find epoch of miner's next cron task (precommit:1, enrollCron:2)
    let cron_epoch = miner_dline_info(&vm, &id_addr).last() - 1;

    // create new vm at epoch 1 less than epoch requested by miner
    let v = vm.with_epoch(cron_epoch);

    // run cron and expect a call to miner and a call to update reward actor params
    apply_ok(
        &v,
        &CRON_ACTOR_ADDR,
        &STORAGE_POWER_ACTOR_ADDR,
        &TokenAmount::zero(),
        PowerMethod::OnEpochTickEnd as u64,
        None::<RawBytes>,
    );

    ExpectInvocation {
        // original send to storage power actor
        from: CRON_ACTOR_ADDR,
        to: STORAGE_POWER_ACTOR_ADDR,
        method: PowerMethod::OnEpochTickEnd as u64,
        subinvocs: Some(vec![
            Expect::reward_this_epoch(STORAGE_POWER_ACTOR_ADDR),
            // expect miner call to be missing
            Expect::reward_update_kpi(),
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().first().unwrap());

    // create new vm at cron epoch with existing state
    let v = v.with_epoch(cron_epoch + 1);

    // run cron and expect a call to miner and a a call to update reward actor params
    apply_ok(
        &v,
        &CRON_ACTOR_ADDR,
        &STORAGE_POWER_ACTOR_ADDR,
        &TokenAmount::zero(),
        PowerMethod::OnEpochTickEnd as u64,
        None::<RawBytes>,
    );

    let sub_invocs = vec![
        Expect::reward_this_epoch(STORAGE_POWER_ACTOR_ADDR),
        // expect call back to miner that was set up in create miner
        ExpectInvocation {
            from: STORAGE_POWER_ACTOR_ADDR,
            to: id_addr,
            method: MinerMethod::OnDeferredCronEvent as u64,
            value: Some(TokenAmount::zero()),
            // Subinvocs unchecked
            ..Default::default()
        },
        Expect::reward_update_kpi(),
    ];

    // expect call to miner
    ExpectInvocation {
        // original send to storage power actor
        from: CRON_ACTOR_ADDR,
        to: STORAGE_POWER_ACTOR_ADDR,
        method: PowerMethod::OnEpochTickEnd as u64,
        subinvocs: Some(sub_invocs),
        ..Default::default()
    }
    .matches(v.take_invocations().first().unwrap());

    expect_invariants(
        &v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
    );
}

use fil_actor_market::Method as MethodsMarket;
use fil_actor_verifreg::{Method as MethodsVerifreg, VerifierParams};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    test_utils::*, BURNT_FUNDS_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{serde_bytes, Cbor, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::{bigint_ser, BigInt, Zero};
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{
    AggregateSealVerifyProofAndInfos, RegisteredSealProof, ReplicaUpdateInfo, SealVerifyInfo,
    StoragePower, WindowPoStVerifyInfo,
};
use num_traits::cast::FromPrimitive;
use test_vm::util::{add_verifier, apply_ok, create_accounts, create_miner, publish_deal};
use test_vm::{ExpectInvocation, TEST_VM_RAND_STRING, VERIFREG_ROOT_KEY, VM};

#[test]
fn terminate_sectors() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 4, TokenAmount::from(10_000e18 as i128));
    let (owner, verifier, unverified_client, verified_client) =
        (addrs[0], addrs[1], addrs[2], addrs[3]);
    let worker = owner.clone();

    let miner_balance = TokenAmount::from(1_000e18 as i128);
    let sector_number = 100;
    let sealed_cid = make_sealed_cid(b"s100");
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;

    let (id_addr, robust_addr) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        miner_balance,
    );

    // publish verified and unverified deals
    add_verifier(&v, verifier, StoragePower::from_i64(32 << 40 as i64).unwrap());

    let add_client_params = VerifierParams {
        address: verified_client,
        allowance: StoragePower::from_i64(32 << 40 as i64).unwrap(),
    };
    apply_ok(
        &v,
        verifier,
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        MethodsVerifreg::AddVerifiedClient as u64,
        add_client_params,
    );

    // add market collateral
    let collateral = TokenAmount::from(3e18 as u64);
    apply_ok(
        &v,
        unverified_client,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral.clone(),
        MethodsMarket::AddBalance as u64,
        unverified_client.clone(),
    );
    apply_ok(
        &v,
        verified_client,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral,
        MethodsMarket::AddBalance as u64,
        verified_client.clone(),
    );

    let miner_collateral = TokenAmount::from(64e18 as u64);
    apply_ok(
        &v,
        worker,
        *STORAGE_MARKET_ACTOR_ADDR,
        miner_collateral,
        MethodsMarket::AddBalance as u64,
        id_addr,
    );

    // create 3 deals, some verified and some not
    let mut deal_ids = vec![];
    let deal_start = v.get_epoch() + &Policy::default().pre_commit_challenge_delay + 1;
    let deals = publish_deal(
        &v,
        worker,
        verified_client,
        id_addr,
        "deal1".to_string(),
        PaddedPieceSize(1 << 30),
        true,
        deal_start,
        181 * EPOCHS_IN_DAY,
    );
    deals.ids.iter().map(|id| {deal_ids.push(id)});
    let deals = publish_deal(&v,worker, verified_client, id_addr, "deal2", 1<<32, true, deal_start, 200 * EPOCHS_IN_DAY);
    deals.ids.iter().map(|id| {deal_ids.push(id)});
    let deals = publish_deal(&v, worker, unverified_client, id_addr, "deal3", 1 << 34, false, deal_start, 210 * EPOCHS_IN_DAY);
    deals.ids.iter().map(|id| {deal_ids.push(id)});

    let res = v.apply_message(*SYSTEM_ACTOR_ADDR, *CRON_ACTOR_ADDR, TokenAmount::zero(), MethodsCron::EpochTick, RawBytes::default()).unwrap();
    assert_eq!(ExitCode::OK, res.code);

}

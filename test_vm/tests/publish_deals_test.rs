use fil_actor_market::{
    ClientDealProposal, DealProposal, Label, Method as MarketMethod, PublishStorageDealsParams,
    PublishStorageDealsReturn,
};
use fvm_shared::crypto::signature::{Signature, SignatureType};

use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::Method as AccountMethod;
use fil_actor_miner::max_prove_commit_duration;
use fil_actor_miner::Method as MinerMethod;
use fil_actor_power::Method as PowerMethod;
use fil_actor_reward::Method as RewardMethod;

use fil_actor_verifreg::{AddVerifierClientParams, Method as VerifregMethod};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    test_utils::*, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};
use test_vm::util::{
    apply_ok, bf_all, create_accounts, create_accounts_seeded, create_miner, verifreg_add_verifier,
};
use test_vm::{ExpectInvocation, VM};

struct Addrs {
    worker: Address,
    client1: Address,
    client2: Address,
    not_miner: Address,
    cheap_client: Address,
    maddr: Address,
    verified_client: Address,
}

const DEAL_LIFETIME: ChainEpoch = 181 * EPOCHS_IN_DAY;

fn token_defaults() -> (TokenAmount, TokenAmount, TokenAmount) {
    let price_per_epoch = TokenAmount::from_atto(1 << 20);
    let provider_collateral = TokenAmount::from_whole(2);
    let client_collateral = TokenAmount::from_whole(1);
    (price_per_epoch, provider_collateral, client_collateral)
}

// create miner and client and add collateral
fn setup(store: &'_ MemoryBlockstore) -> (VM<'_>, Addrs, ChainEpoch) {
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 7, TokenAmount::from_whole(10_000));
    let (worker, client1, client2, not_miner, cheap_client, verifier, verified_client) =
        (addrs[0], addrs[1], addrs[2], addrs[3], addrs[4], addrs[5], addrs[6]);
    let owner = worker;

    // setup provider
    let miner_balance = TokenAmount::from_whole(100);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;

    let maddr = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        miner_balance,
    )
    .0;

    // setup verified client
    verifreg_add_verifier(&v, verifier, StoragePower::from((32_u64 << 40) as u128));
    let add_client_params = AddVerifierClientParams {
        address: verified_client,
        allowance: StoragePower::from((1_u64 << 32) as u64),
    };
    apply_ok(
        &v,
        verifier,
        VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        add_client_params,
    );

    let client_collateral = TokenAmount::from_whole(100);
    apply_ok(
        &v,
        client1,
        STORAGE_MARKET_ACTOR_ADDR,
        client_collateral.clone(),
        MarketMethod::AddBalance as u64,
        client1,
    );
    apply_ok(
        &v,
        client2,
        STORAGE_MARKET_ACTOR_ADDR,
        client_collateral.clone(),
        MarketMethod::AddBalance as u64,
        client2,
    );
    apply_ok(
        &v,
        verified_client,
        STORAGE_MARKET_ACTOR_ADDR,
        client_collateral,
        MarketMethod::AddBalance as u64,
        verified_client,
    );

    let miner_collateral = TokenAmount::from_whole(100);
    apply_ok(
        &v,
        worker,
        STORAGE_MARKET_ACTOR_ADDR,
        miner_collateral,
        MarketMethod::AddBalance as u64,
        maddr,
    );

    let deal_start =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    (
        v,
        Addrs { worker, client1, client2, not_miner, cheap_client, maddr, verified_client },
        deal_start,
    )
}

#[test]
fn psd_mistmatched_provider() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);

    // good deal
    batcher.stage(a.client1, "deal0", DealOptions::default());
    // bad deal, provider doesn't match worker
    batcher.stage(
        a.client1,
        "deal1",
        DealOptions { provider: Some(a.not_miner), ..Default::default() },
    );
    //good deal
    batcher.stage(a.client1, "deal2", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 2], good_inputs);

    v.assert_state_invariants();
}

#[test]
fn psd_bad_piece_size() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    // bad deal piece size too small
    batcher.stage(
        a.client1,
        "deal0",
        DealOptions { piece_size: Some(PaddedPieceSize(0)), ..Default::default() },
    );
    // good deal
    batcher.stage(a.client1, "deal1", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![1], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_start_time_in_past() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    let bad_deal_start = v.get_epoch() - 1;
    batcher.stage(
        a.client1,
        "deal0",
        DealOptions { deal_start: Some(bad_deal_start), ..Default::default() },
    );
    batcher.stage(a.client1, "deal1", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![1], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_client_address_cannot_be_resolved() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    let bad_client = Address::new_id(5_000_000);
    batcher.stage(a.client1, "deal0", DealOptions::default());
    batcher.stage(bad_client, "deal1", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_no_client_lockup() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    batcher.stage(a.cheap_client, "deal0", DealOptions::default());
    batcher.stage(a.client1, "deal1", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![1], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_not_enought_client_lockup_for_batch() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    // Add one lifetime cost to cheap_client's market balance but attempt to make 3 deals
    let (default_price, _, default_client_collateral) = token_defaults();
    let one_lifetime_cost = default_client_collateral + DEAL_LIFETIME * default_price;
    apply_ok(
        &v,
        a.cheap_client,
        STORAGE_MARKET_ACTOR_ADDR,
        one_lifetime_cost,
        MarketMethod::AddBalance as u64,
        a.cheap_client,
    );

    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    // good
    batcher.stage(a.cheap_client, "deal0", DealOptions::default());
    // bad -- insufficient funds
    batcher.stage(a.cheap_client, "deal1", DealOptions::default());
    batcher.stage(a.cheap_client, "deal2", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_not_enough_provider_lockup_for_batch() {
    let store = MemoryBlockstore::new();
    let (mut v, a, deal_start) = setup(&store);

    // note different seed, different address
    let cheap_worker = create_accounts_seeded(&v, 1, TokenAmount::from_whole(10_000), 444)[0];
    let cheap_maddr = create_miner(
        &mut v,
        cheap_worker,
        cheap_worker,
        fvm_shared::sector::RegisteredPoStProof::StackedDRGWindow32GiBV1,
        TokenAmount::from_whole(100),
    )
    .0;
    // add one deal of collateral to provider's market account
    let default_provider_collateral = token_defaults().1;
    apply_ok(
        &v,
        cheap_worker,
        STORAGE_MARKET_ACTOR_ADDR,
        default_provider_collateral,
        MarketMethod::AddBalance as u64,
        cheap_maddr,
    );
    let mut batcher = DealBatcher::new(
        &v,
        cheap_maddr,
        PaddedPieceSize(1 << 30),
        false,
        deal_start,
        DEAL_LIFETIME,
    );
    // good deal
    batcher.stage(a.client1, "deal0", DealOptions::default());
    // bad deal insufficient funds on provider
    batcher.stage(a.client2, "deal1", DealOptions::default());
    let deal_ret = batcher.publish_ok(cheap_worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_duplicate_deal_in_batch() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);

    // good deals
    batcher.stage(a.client1, "deal0", DealOptions::default());
    batcher.stage(a.client1, "deal1", DealOptions::default());

    // bad duplicates
    batcher.stage(a.client1, "deal0", DealOptions::default());
    batcher.stage(a.client1, "deal0", DealOptions::default());

    // good
    batcher.stage(a.client1, "deal2", DealOptions::default());

    // bad
    batcher.stage(a.client1, "deal1", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1, 4], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_duplicate_deal_in_state() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);

    batcher.stage(a.client2, "deal0", DealOptions::default());
    let deal_ret1 = batcher.publish_ok(a.worker);
    let good_inputs1 = bf_all(deal_ret1.valid_deals);
    assert_eq!(vec![0], good_inputs1);

    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    batcher.stage(a.client2, "deal1", DealOptions::default());
    // duplicate in batch
    batcher.stage(a.client2, "deal1", DealOptions::default());
    // duplicate in state
    batcher.stage(a.client2, "deal0", DealOptions::default());

    let deal_ret2 = batcher.publish_ok(a.worker);
    let good_inputs2 = bf_all(deal_ret2.valid_deals);
    assert_eq!(vec![0], good_inputs2);
    v.assert_state_invariants();
}

#[test]
fn psd_verified_deal_fails_getting_datacap() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);

    batcher.stage(a.verified_client, "deal0", DealOptions::default());
    // good verified deal that uses up all datacap
    batcher.stage(
        a.verified_client,
        "deal1",
        DealOptions {
            piece_size: Some(PaddedPieceSize(1_u64 << 32)),
            verified: Some(true),
            ..Default::default()
        },
    );
    // bad verified deal, no data cap left
    batcher.stage(
        a.verified_client,
        "deal2",
        DealOptions {
            piece_size: Some(PaddedPieceSize(1_u64 << 32)),
            verified: Some(true),
            ..Default::default()
        },
    );

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_random_assortment_of_failures() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    // Add one lifetime cost to cheap_client's market balance but attempt to make 3 deals
    let (default_price, _, default_client_collateral) = token_defaults();
    let one_lifetime_cost = default_client_collateral + DEAL_LIFETIME * default_price;
    apply_ok(
        &v,
        a.cheap_client,
        STORAGE_MARKET_ACTOR_ADDR,
        one_lifetime_cost,
        MarketMethod::AddBalance as u64,
        a.cheap_client,
    );
    let broke_client = create_accounts_seeded(&v, 1, TokenAmount::zero(), 555)[0];

    batcher.stage(
        a.verified_client,
        "deal1",
        DealOptions {
            piece_size: Some(PaddedPieceSize(1u64 << 32)),
            verified: Some(true),
            ..Default::default()
        },
    );
    // duplicate
    batcher.stage(
        a.verified_client,
        "deal1",
        DealOptions {
            piece_size: Some(PaddedPieceSize(1u64 << 32)),
            verified: Some(true),
            ..Default::default()
        },
    );
    batcher.stage(a.cheap_client, "deal2", DealOptions::default());
    // no client funds
    batcher.stage(broke_client, "deal3", DealOptions::default());
    // provider addr does not match
    batcher.stage(
        a.client1,
        "deal4",
        DealOptions { provider: Some(a.client2), ..Default::default() },
    );
    // insufficient data cap
    batcher.stage(
        a.verified_client,
        "deal5",
        DealOptions { verified: Some(true), ..Default::default() },
    );
    // cheap client out of funds
    batcher.stage(a.cheap_client, "deal6", DealOptions::default());
    // provider collateral too low
    batcher.stage(
        a.client2,
        "deal7",
        DealOptions { provider_collateral: Some(TokenAmount::zero()), ..Default::default() },
    );
    batcher.stage(a.client1, "deal8", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 2, 8], good_inputs);
    v.assert_state_invariants();
}

#[test]
fn psd_all_deals_are_bad() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    let bad_client = Address::new_id(1000);

    batcher.stage(
        a.client1,
        "deal0",
        DealOptions { provider_collateral: Some(TokenAmount::zero()), ..Default::default() },
    );
    batcher.stage(
        a.client1,
        "deal1",
        DealOptions { provider: Some(a.client2), ..Default::default() },
    );
    batcher.stage(a.client1, "deal2", DealOptions { verified: Some(true), ..Default::default() });
    batcher.stage(bad_client, "deal3", DealOptions::default());
    batcher.stage(
        a.client1,
        "deal4",
        DealOptions { piece_size: Some(PaddedPieceSize(0)), ..Default::default() },
    );

    batcher.publish_fail(a.worker);
    v.assert_state_invariants();
}

#[test]
fn psd_bad_sig() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let (storage_price_per_epoch, provider_collateral, client_collateral) = token_defaults();

    let deal_label = "deal0".to_string();
    let proposal = DealProposal {
        piece_cid: make_piece_cid(deal_label.as_bytes()),
        piece_size: PaddedPieceSize(1 << 30),
        verified_deal: false,
        client: a.client1,
        provider: a.maddr,
        label: Label::String(deal_label),
        start_epoch: deal_start,
        end_epoch: deal_start + DEAL_LIFETIME,
        storage_price_per_epoch,
        provider_collateral,
        client_collateral,
    };

    let invalid_sig_bytes = "very_invalid_sig".as_bytes().to_vec();
    let publish_params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal {
            proposal: proposal.clone(),
            client_signature: Signature {
                sig_type: SignatureType::BLS,
                bytes: invalid_sig_bytes.clone(),
            },
        }],
    };

    let ret = v
        .apply_message(
            a.worker,
            STORAGE_MARKET_ACTOR_ADDR,
            TokenAmount::zero(),
            MarketMethod::PublishStorageDeals as u64,
            publish_params,
        )
        .unwrap();
    assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, ret.code);

    ExpectInvocation {
        to: STORAGE_MARKET_ACTOR_ADDR,
        method: MarketMethod::PublishStorageDeals as u64,
        subinvocs: Some(vec![
            ExpectInvocation {
                to: a.maddr,
                method: MinerMethod::ControlAddresses as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: REWARD_ACTOR_ADDR,
                method: RewardMethod::ThisEpochReward as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::CurrentTotalPower as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: a.client1,
                method: AccountMethod::AuthenticateMessageExported as u64,
                params: Some(
                    serialize(
                        &AuthenticateMessageParams {
                            signature: invalid_sig_bytes,
                            message: serialize(&proposal, "deal proposal").unwrap().to_vec(),
                        },
                        "auth params",
                    )
                    .unwrap(),
                ),
                code: Some(ExitCode::USR_ILLEGAL_ARGUMENT),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    v.assert_state_invariants();
}

#[test]
fn psd_all_deals_are_good() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);

    // good deals
    batcher.stage(a.client1, "deal0", DealOptions::default());
    batcher.stage(a.client1, "deal1", DealOptions::default());
    batcher.stage(a.client1, "deal2", DealOptions::default());
    batcher.stage(a.client1, "deal3", DealOptions::default());
    batcher.stage(a.client1, "deal4", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![0, 1, 2, 3, 4], good_inputs);
    v.assert_state_invariants();
}

#[derive(Clone, Default)]
struct DealOptions {
    provider: Option<Address>,
    piece_size: Option<PaddedPieceSize>,
    verified: Option<bool>,
    deal_start: Option<ChainEpoch>,
    deal_lifetime: Option<ChainEpoch>,
    price_per_epoch: Option<TokenAmount>,
    provider_collateral: Option<TokenAmount>,
    client_collateral: Option<TokenAmount>,
}

struct DealBatcher<'bs> {
    deals: Vec<DealProposal>,
    v: &'bs VM<'bs>,
    default_provider: Address,
    default_piece_size: PaddedPieceSize,
    default_verified: bool,
    default_deal_start: ChainEpoch,
    default_deal_lifetime: ChainEpoch,
    default_price_per_epoch: TokenAmount,
    default_provider_collateral: TokenAmount,
    default_client_collateral: TokenAmount,
}

impl<'bs> DealBatcher<'bs> {
    fn new(
        v: &'bs VM<'bs>,
        default_provider: Address,
        default_piece_size: PaddedPieceSize,
        default_verified: bool,
        default_deal_start: ChainEpoch,
        default_deal_lifetime: ChainEpoch,
    ) -> Self {
        let (default_price_per_epoch, default_provider_collateral, default_client_collateral) =
            token_defaults();
        DealBatcher {
            deals: vec![],
            v,
            default_provider,
            default_piece_size,
            default_verified,
            default_deal_start,
            default_deal_lifetime,
            default_price_per_epoch,
            default_provider_collateral,
            default_client_collateral,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stage(&mut self, client: Address, deal_label: &str, opts: DealOptions) {
        let opts = self.default_opts(opts);
        let label = Label::String(deal_label.to_string());
        let deal = DealProposal {
            piece_cid: make_piece_cid(deal_label.as_bytes()),
            piece_size: opts.piece_size.unwrap(),
            verified_deal: opts.verified.unwrap(),
            client,
            provider: opts.provider.unwrap(),
            label,
            start_epoch: opts.deal_start.unwrap(),
            end_epoch: opts.deal_start.unwrap() + opts.deal_lifetime.unwrap(),
            storage_price_per_epoch: opts.price_per_epoch.unwrap(),
            provider_collateral: opts.provider_collateral.unwrap(),
            client_collateral: opts.client_collateral.unwrap(),
        };
        self.deals.push(deal)
    }

    pub fn default_opts(&self, in_opts: DealOptions) -> DealOptions {
        let mut opts = in_opts.clone();
        if in_opts.provider.is_none() {
            opts.provider = Some(self.default_provider)
        }
        if in_opts.piece_size.is_none() {
            opts.piece_size = Some(self.default_piece_size)
        }
        if in_opts.verified.is_none() {
            opts.verified = Some(self.default_verified)
        }
        if in_opts.deal_start.is_none() {
            opts.deal_start = Some(self.default_deal_start)
        }
        if in_opts.deal_lifetime.is_none() {
            opts.deal_lifetime = Some(self.default_deal_lifetime)
        }
        if in_opts.price_per_epoch.is_none() {
            opts.price_per_epoch = Some(self.default_price_per_epoch.clone())
        }
        if in_opts.provider_collateral.is_none() {
            opts.provider_collateral = Some(self.default_provider_collateral.clone())
        }
        if in_opts.client_collateral.is_none() {
            opts.client_collateral = Some(self.default_client_collateral.clone())
        }
        opts
    }

    pub fn publish_ok(&mut self, sender: Address) -> PublishStorageDealsReturn {
        let params_deals = self
            .deals
            .iter_mut()
            .map(|deal| ClientDealProposal {
                proposal: deal.clone(),
                client_signature: Signature {
                    sig_type: SignatureType::BLS,
                    bytes: serialize(deal, "serializing deal proposal").unwrap().to_vec(),
                },
            })
            .collect();
        let publish_params = PublishStorageDealsParams { deals: params_deals };
        let ret: PublishStorageDealsReturn = apply_ok(
            self.v,
            sender,
            STORAGE_MARKET_ACTOR_ADDR,
            TokenAmount::zero(),
            MarketMethod::PublishStorageDeals as u64,
            publish_params,
        )
        .deserialize()
        .unwrap();
        ret
    }

    pub fn publish_fail(&mut self, sender: Address) {
        let params_deals = self
            .deals
            .iter_mut()
            .map(|deal| ClientDealProposal {
                proposal: deal.clone(),
                client_signature: Signature {
                    sig_type: SignatureType::BLS,
                    bytes: serialize(deal, "serializing deal proposal").unwrap().to_vec(),
                },
            })
            .collect();
        let publish_params = PublishStorageDealsParams { deals: params_deals };
        let ret = self
            .v
            .apply_message(
                sender,
                STORAGE_MARKET_ACTOR_ADDR,
                TokenAmount::zero(),
                MarketMethod::PublishStorageDeals as u64,
                publish_params,
            )
            .unwrap();
        assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, ret.code);
    }
}

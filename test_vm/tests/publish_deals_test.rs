use fil_actor_cron::Method as MethodsCron;
use fil_actor_market::{
    ClientDealProposal, DealMetaArray, DealProposal, Label, Method as MethodsMarket,
    PublishStorageDealsParams, PublishStorageDealsReturn, State as MarketState,
    WithdrawBalanceParams,
};
use fvm_shared::crypto::signature::{Signature, SignatureType};

use fil_actor_miner::{
    max_prove_commit_duration, power_for_sector, Method as MethodsMiner, PreCommitSectorParams,
    ProveCommitSectorParams, State as MinerState, TerminateSectorsParams, TerminationDeclaration,
};
use fil_actor_power::{Method as MethodsPower, State as PowerState};
use fil_actor_reward::Method as MethodsReward;
use fil_actor_verifreg::{Method as MethodsVerifreg, VerifierParams};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    test_utils::*, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, REWARD_ACTOR_ADDR,
    STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};
use fvm_shared::METHOD_SEND;
use num_traits::cast::FromPrimitive;
use test_vm::util::{
    add_verifier, advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_to_proving_deadline, apply_ok, bf_all, create_accounts, create_miner, make_bitfield,
    publish_deal, submit_windowed_post,
};
use test_vm::{ExpectInvocation, VM};

struct Addrs {
    worker: Address,
    client1: Address,
    client2: Address,
    not_miner: Address,
    cheap_client: Address,
    maddr: Address,
}

const DEAL_LIFETIME: ChainEpoch = 181 * EPOCHS_IN_DAY;

fn token_defaults() -> (TokenAmount, TokenAmount, TokenAmount) {
    let price_per_epoch = TokenAmount::from(1 << 20);
    let provider_collateral = TokenAmount::from(2e18 as u128);
    let client_collateral = TokenAmount::from(1e18 as u128);
    (price_per_epoch, provider_collateral, client_collateral)
}

// create miner and client and add collateral
fn setup<'bs>(store: &'bs MemoryBlockstore) -> (VM<'bs>, Addrs, ChainEpoch) {
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 5, TokenAmount::from(10_000e18 as i128));
    let (worker, client1, client2, not_miner, cheap_client) =
        (addrs[0], addrs[1], addrs[2], addrs[3], addrs[4]);
    let owner = worker;

    let miner_balance = TokenAmount::from(100e18 as i128);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;

    let maddr = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        miner_balance,
    )
    .0;
    let client_collateral = TokenAmount::from(100e18 as i128);
    apply_ok(
        &v,
        client1,
        *STORAGE_MARKET_ACTOR_ADDR,
        client_collateral.clone(),
        MethodsMarket::AddBalance as u64,
        client1,
    );
    apply_ok(
        &v,
        client2,
        *STORAGE_MARKET_ACTOR_ADDR,
        client_collateral,
        MethodsMarket::AddBalance as u64,
        client2,
    );
    let miner_collateral = TokenAmount::from(100e18 as i128);
    apply_ok(
        &v,
        worker,
        *STORAGE_MARKET_ACTOR_ADDR,
        miner_collateral,
        MethodsMarket::AddBalance as u64,
        maddr,
    );

    let deal_start =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    (v, Addrs { worker, client1, client2, not_miner, cheap_client, maddr }, deal_start)
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
    assert_eq!(vec![0, 2], good_inputs)
}

#[test]
fn psd_bad_piece_size() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);  
    // bad deal piece size too small
    batcher.stage(a.client1, "deal0", DealOptions{piece_size: Some(PaddedPieceSize(0)), ..Default::default()});
    // good deal
    batcher.stage(a.client1, "deal1", DealOptions::default());

     let deal_ret = batcher.publish_ok(a.worker);
     let good_inputs = bf_all(deal_ret.valid_deals);
     assert_eq!(vec![1], good_inputs);
}

#[test]
fn psd_start_time_in_past() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    let bad_deal_start = v.get_epoch() - 1;
    batcher.stage(a.client1, "deal0", DealOptions{deal_start: Some(bad_deal_start), ..Default::default()});
    batcher.stage(a.client1, "deal1", DealOptions::default());

    let deal_ret = batcher.publish_ok(a.worker);
    let good_inputs = bf_all(deal_ret.valid_deals);
    assert_eq!(vec![1], good_inputs);
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
} 

#[test]
fn psd_no_client_lockup() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME); 
}

#[test]
fn psd_all_deals_are_bad() {
    let store = MemoryBlockstore::new();
    let (v, a, deal_start) = setup(&store);
    let mut batcher =
        DealBatcher::new(&v, a.maddr, PaddedPieceSize(1 << 30), false, deal_start, DEAL_LIFETIME);
    let bad_client = Address::new_id(1000);

    batcher.stage(a.client1, "deal0", DealOptions{provider_collateral: Some(TokenAmount::zero()), ..Default::default()});
    batcher.stage(a.client1, "deal1", DealOptions{provider: Some(a.client2), ..Default::default()});
    batcher.stage(a.client1, "deal2", DealOptions{verified: Some(true), ..Default::default()});
    batcher.stage(bad_client, "deal3", DealOptions::default());
    batcher.stage(a.client1, "deal4", DealOptions{piece_size: Some(PaddedPieceSize(0)), ..Default::default()});

    batcher.publish_fail(a.worker);
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
}


#[derive(Clone)]
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

impl Default for DealOptions {
    fn default() -> Self {
        Self {
            provider: None,
            piece_size: None,
            verified: None,
            deal_start: None,
            deal_lifetime: None,
            price_per_epoch: None,
            provider_collateral: None,
            client_collateral: None,
        }
    }
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

    pub fn default_opts(&self, mut in_opts: DealOptions) -> DealOptions {
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
                client_signature: Signature { sig_type: SignatureType::BLS, bytes: vec![] },
            })
            .collect();
        let publish_params = PublishStorageDealsParams { deals: params_deals };
        let ret: PublishStorageDealsReturn = apply_ok(
            &self.v,
            sender,
            *STORAGE_MARKET_ACTOR_ADDR,
            TokenAmount::zero(),
            MethodsMarket::PublishStorageDeals as u64,
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
                client_signature: Signature { sig_type: SignatureType::BLS, bytes: vec![] },
            })
            .collect();
        let publish_params = PublishStorageDealsParams { deals: params_deals };
        let ret = self.v.apply_message(sender, *STORAGE_MARKET_ACTOR_ADDR, TokenAmount::zero(), MethodsMarket::PublishStorageDeals as u64, publish_params).unwrap();
        assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, ret.code);
    }
}

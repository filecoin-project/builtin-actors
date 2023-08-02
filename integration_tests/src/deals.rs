use fil_actor_market::{
    ClientDealProposal, DealProposal, Label, Method as MarketMethod, PublishStorageDealsParams,
    PublishStorageDealsReturn,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use num_traits::Zero;
use vm_api::util::{apply_ok, serialize_ok};
use vm_api::VM;

#[derive(Clone)]
pub struct DealOptions {
    pub piece_size: PaddedPieceSize,
    pub verified: bool,
    pub deal_start: ChainEpoch,
    pub deal_lifetime: ChainEpoch,
    pub price_per_epoch: TokenAmount,
    pub provider_collateral: TokenAmount,
    pub client_collateral: TokenAmount,
}

impl Default for DealOptions {
    fn default() -> Self {
        DealOptions {
            piece_size: PaddedPieceSize(1 << 30),
            verified: false,
            deal_start: 0,
            deal_lifetime: Policy::default().min_sector_expiration,
            price_per_epoch: TokenAmount::from_atto(1),
            provider_collateral: TokenAmount::from_whole(1),
            client_collateral: TokenAmount::from_whole(1),
        }
    }
}

// A helper for staging and publishing deals.
// Note that this doesn't check trace expectations,
// see https://github.com/filecoin-project/builtin-actors/issues/1302.
pub struct DealBatcher<'vm> {
    v: &'vm dyn VM,
    deals: Vec<DealProposal>,
    default_options: DealOptions,
    published: bool,
}

impl<'vm> DealBatcher<'vm> {
    pub fn new(v: &'vm dyn VM, opts: DealOptions) -> Self {
        DealBatcher { v, deals: vec![], default_options: opts, published: false }
    }

    pub fn proposals(&self) -> &Vec<DealProposal> {
        &self.deals
    }

    pub fn stage(&mut self, client: Address, provider: Address) {
        let label = format!("deal-{}", self.deals.len());
        self.stage_with_opts_label(client, provider, label, self.default_options.clone())
    }

    pub fn stage_with_opts(&mut self, client: Address, provider: Address, opts: DealOptions) {
        let label = format!("deal-{}", self.deals.len());
        self.stage_with_opts_label(client, provider, label, opts)
    }

    pub fn stage_with_label(&mut self, client: Address, provider: Address, label: String) {
        self.stage_with_opts_label(client, provider, label, self.default_options.clone())
    }

    pub fn stage_with_opts_label(
        &mut self,
        client: Address,
        provider: Address,
        label: String,
        opts: DealOptions,
    ) {
        let deal = DealProposal {
            piece_cid: make_piece_cid(label.as_bytes()),
            piece_size: opts.piece_size,
            verified_deal: opts.verified,
            client,
            provider,
            label: Label::String(label),
            start_epoch: opts.deal_start,
            end_epoch: opts.deal_start + opts.deal_lifetime,
            storage_price_per_epoch: opts.price_per_epoch,
            provider_collateral: opts.provider_collateral,
            client_collateral: opts.client_collateral,
        };
        self.deals.push(deal)
    }

    pub fn publish_ok(&mut self, sender: Address) -> PublishStorageDealsReturn {
        assert!(!self.published, "already published");
        let publish_params = publish_deal_params(&self.deals);
        let ret: PublishStorageDealsReturn = apply_ok(
            self.v,
            &sender,
            &STORAGE_MARKET_ACTOR_ADDR,
            &TokenAmount::zero(),
            MarketMethod::PublishStorageDeals as u64,
            Some(publish_params),
        )
        .deserialize()
        .unwrap();
        self.published = true;
        ret
    }

    pub fn publish_fail(&mut self, sender: Address) {
        let publish_params = publish_deal_params(&self.deals);
        let ret = self
            .v
            .execute_message(
                &sender,
                &STORAGE_MARKET_ACTOR_ADDR,
                &TokenAmount::zero(),
                MarketMethod::PublishStorageDeals as u64,
                Some(serialize_ok(&publish_params)),
            )
            .unwrap();
        assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, ret.code);
    }
}

fn publish_deal_params(proposals: &[DealProposal]) -> PublishStorageDealsParams {
    let params_deals = proposals
        .iter()
        .map(|deal| ClientDealProposal {
            proposal: deal.clone(),
            client_signature: Signature {
                sig_type: SignatureType::BLS,
                bytes: serialize(deal, "serializing deal proposal").unwrap().to_vec(),
            },
        })
        .collect();
    PublishStorageDealsParams { deals: params_deals }
}

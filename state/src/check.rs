use std::collections::HashMap;
use std::fmt::Debug;

use anyhow::bail;
use bimap::BiBTreeMap;
use cid::Cid;
use fil_actor_account::State as AccountState;
use fil_actor_cron::State as CronState;
use fil_actor_datacap::State as DataCapState;
use fil_actor_init::State as InitState;
use fil_actor_market::State as MarketState;
use fil_actor_miner::CronEventPayload;
use fil_actor_miner::PowerPair;
use fil_actor_miner::State as MinerState;
use fil_actor_miner::CRON_EVENT_PROCESS_EARLY_TERMINATIONS;
use fil_actor_miner::CRON_EVENT_PROVING_DEADLINE;
use fil_actor_multisig::State as MultisigState;
use fil_actor_paych::State as PaychState;
use fil_actor_power::testing::MinerCronEvent;
use fil_actor_power::State as PowerState;
use fil_actor_reward::State as RewardState;
use fil_actor_verifreg::{DataCap, State as VerifregState};

use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;

use fil_actors_runtime::Map;
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::from_slice;
use fvm_ipld_encoding::CborStore;
use fvm_shared::address::Address;
use fvm_shared::address::Protocol;

use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use anyhow::anyhow;
use fvm_ipld_encoding::tuple::*;

use fil_actor_account::testing as account;
use fil_actor_cron::testing as cron;
use fil_actor_datacap::testing as datacap;
use fil_actor_init::testing as init;
use fil_actor_market::testing as market;
use fil_actor_miner::testing as miner;
use fil_actor_multisig::testing as multisig;
use fil_actor_paych::testing as paych;
use fil_actor_power::testing as power;
use fil_actor_reward::testing as reward;
use fil_actor_verifreg::testing as verifreg;
use fil_actors_runtime::runtime::builtins::Type;

/// Value type of the top level of the state tree.
/// Represents the on-chain state of a single actor.
#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Eq, Debug)]
pub struct Actor {
    /// CID representing the code associated with the actor
    pub code: Cid,
    /// CID of the head state object for the actor
    pub head: Cid,
    /// `call_seq_num` for the next message to be received by the actor (non-zero for accounts only)
    pub call_seq_num: u64,
    /// Token balance of the actor
    pub balance: TokenAmount,
}

/// A specialization of a map of ID-addresses to actor heads.
pub struct Tree<'a, BS>
where
    BS: Blockstore,
{
    pub map: Map<'a, BS, Actor>,
    pub store: &'a BS,
}

impl<'a, BS: Blockstore> Tree<'a, BS> {
    /// Loads a tree from a root CID and store
    pub fn load(store: &'a BS, root: &Cid) -> anyhow::Result<Self> {
        let map = Map::load(root, store)?;

        Ok(Tree { map, store })
    }

    pub fn for_each<F>(&self, mut f: F) -> anyhow::Result<()>
    where
        F: FnMut(&Address, &Actor) -> anyhow::Result<()>,
    {
        self.map
            .for_each(|key, val| {
                let address = Address::from_bytes(key)?;
                f(&address, val)
            })
            .map_err(|e| anyhow!("Failed iterating map: {}", e))
    }
}

macro_rules! get_state {
    ($tree:ident, $actor:ident, $state:ty) => {
        $tree
            .store
            .get_cbor::<$state>(&$actor.head)?
            .ok_or_else(|| anyhow!("{} is empty", stringify!($state)))?
    };
}

// Note: BiBTreeMap is an overly constrained type for what we are doing here, but chosen
// to match the Manifest implementation in the FVM.
// It could be replaced with a custom mapping trait (while Rust doesn't support
// abstract collection traits).
pub fn check_state_invariants<'a, BS: Blockstore + Debug>(
    manifest: &BiBTreeMap<Cid, Type>,
    policy: &Policy,
    tree: Tree<'a, BS>,
    expected_balance_total: &TokenAmount,
    prior_epoch: ChainEpoch,
) -> anyhow::Result<MessageAccumulator> {
    let acc = MessageAccumulator::default();
    let mut total_fil = TokenAmount::zero();

    let mut init_summary: Option<init::StateSummary> = None;
    let mut cron_summary: Option<cron::StateSummary> = None;
    let mut account_summaries = Vec::<account::StateSummary>::new();
    let mut power_summary: Option<power::StateSummary> = None;
    let mut miner_summaries = HashMap::<Address, miner::StateSummary>::new();
    let mut market_summary: Option<market::StateSummary> = None;
    let mut paych_summaries = Vec::<paych::StateSummary>::new();
    let mut multisig_summaries = Vec::<multisig::StateSummary>::new();
    let mut reward_summary: Option<reward::StateSummary> = None;
    let mut verifreg_summary: Option<verifreg::StateSummary> = None;
    let mut datacap_summary: Option<datacap::StateSummary> = None;

    tree.for_each(|key, actor| {
        let acc = acc.with_prefix(format!("{key} "));

        if key.protocol() != Protocol::ID {
            acc.add(format!("unexpected address protocol in state tree root: {key}"));
        }
        total_fil += &actor.balance;

        match manifest.get_by_left(&actor.code) {
            Some(Type::System) => (),
            Some(Type::Init) => {
                let state = get_state!(tree, actor, InitState);
                let (summary, msgs) = init::check_state_invariants(&state, tree.store);
                acc.with_prefix("init: ").add_all(&msgs);
                init_summary = Some(summary);
            }
            Some(Type::Cron) => {
                let state = get_state!(tree, actor, CronState);
                let (summary, msgs) = cron::check_state_invariants(&state);
                acc.with_prefix("cron: ").add_all(&msgs);
                cron_summary = Some(summary);
            }
            Some(Type::Account) => {
                let state = get_state!(tree, actor, AccountState);
                let (summary, msgs) = account::check_state_invariants(&state, key);
                acc.with_prefix("account: ").add_all(&msgs);
                account_summaries.push(summary);
            }
            Some(Type::Power) => {
                let state = get_state!(tree, actor, PowerState);
                let (summary, msgs) = power::check_state_invariants(policy, &state, tree.store);
                acc.with_prefix("power: ").add_all(&msgs);
                power_summary = Some(summary);
            }
            Some(Type::Miner) => {
                let state = get_state!(tree, actor, MinerState);
                let (summary, msgs) =
                    miner::check_state_invariants(policy, &state, tree.store, &actor.balance);
                acc.with_prefix("miner: ").add_all(&msgs);
                miner_summaries.insert(*key, summary);
            }
            Some(Type::Market) => {
                let state = get_state!(tree, actor, MarketState);
                let (summary, msgs) = market::check_state_invariants(
                    &state,
                    tree.store,
                    &actor.balance,
                    prior_epoch + 1,
                );
                acc.with_prefix("market: ").add_all(&msgs);
                market_summary = Some(summary);
            }
            Some(Type::PaymentChannel) => {
                let state = get_state!(tree, actor, PaychState);
                let (summary, msgs) =
                    paych::check_state_invariants(&state, tree.store, &actor.balance);
                acc.with_prefix("paych: ").add_all(&msgs);
                paych_summaries.push(summary);
            }
            Some(Type::Multisig) => {
                let state = get_state!(tree, actor, MultisigState);
                let (summary, msgs) = multisig::check_state_invariants(&state, tree.store);
                acc.with_prefix("multisig: ").add_all(&msgs);
                multisig_summaries.push(summary);
            }
            Some(Type::Reward) => {
                let state = get_state!(tree, actor, RewardState);
                let (summary, msgs) =
                    reward::check_state_invariants(&state, prior_epoch, &actor.balance);
                acc.with_prefix("reward: ").add_all(&msgs);
                reward_summary = Some(summary);
            }
            Some(Type::VerifiedRegistry) => {
                let state = get_state!(tree, actor, VerifregState);
                let (summary, msgs) =
                    verifreg::check_state_invariants(&state, tree.store, prior_epoch);
                acc.with_prefix("verifreg: ").add_all(&msgs);
                verifreg_summary = Some(summary);
            }
            Some(Type::DataCap) => {
                let state = get_state!(tree, actor, DataCapState);
                let (summary, msgs) = datacap::check_state_invariants(&state, tree.store);
                acc.with_prefix("datacap: ").add_all(&msgs);
                datacap_summary = Some(summary);
            }
            None => {
                bail!("unexpected actor code CID {} for address {}", actor.code, key);
            }
        };

        Ok(())
    })?;

    // Perform cross-actor checks from state summaries here.
    if let Some(power_summary) = power_summary {
        check_miner_against_power(&acc, &miner_summaries, &power_summary);
    }

    if let Some(market_summary) = market_summary.clone() {
        check_deal_states_against_sectors(&acc, &miner_summaries, &market_summary);
    }

    if let Some(verifreg_summary) = verifreg_summary {
        if let Some(datacap_summary) = datacap_summary {
            check_verifreg_against_datacap(&acc, &verifreg_summary, &datacap_summary);
        }
        if let Some(market_summary) = market_summary {
            check_market_against_verifreg(&acc, &market_summary, &verifreg_summary);
        }
        check_verifreg_against_miners(&acc, &verifreg_summary, &miner_summaries);
    }

    acc.require(
        &total_fil == expected_balance_total,
        format!("total token balance is {total_fil}, expected {expected_balance_total}"),
    );

    Ok(acc)
}

fn check_miner_against_power(
    acc: &MessageAccumulator,
    miner_summaries: &HashMap<Address, miner::StateSummary>,
    power_summary: &power::StateSummary,
) {
    for (address, miner_summary) in miner_summaries {
        //check claim
        if let Some(claim) = power_summary.claims.get(address) {
            let claim_power =
                PowerPair::new(claim.raw_byte_power.clone(), claim.quality_adj_power.clone());
            acc.require(miner_summary.active_power == claim_power, format!("miner {address} computed active power {:?} does not match claim {claim_power:?}", miner_summary.active_power));
            acc.require(
                miner_summary.window_post_proof_type == claim.window_post_proof_type,
                format!(
                    "miner seal proof type {:?} does not match claim proof type {:?}",
                    miner_summary.window_post_proof_type, claim.window_post_proof_type
                ),
            );
        } else {
            acc.add(format!("miner {address} has no power claim"));
        }

        //check crons
        let mut proving_period_cron: Option<&MinerCronEvent> = None;
        if let Some(crons) = power_summary.crons.get(address) {
            for event in crons {
                match from_slice::<CronEventPayload>(event.payload.bytes()) {
                    Ok(payload) => {
                        acc.require(
                            matches!(
                                payload.event_type,
                                CRON_EVENT_PROCESS_EARLY_TERMINATIONS | CRON_EVENT_PROVING_DEADLINE
                            ),
                            format!(
                                "miner {address} has unexpected cron event type {}",
                                payload.event_type
                            ),
                        );
                        if payload.event_type == CRON_EVENT_PROVING_DEADLINE {
                            if proving_period_cron.is_some() {
                                acc.add(format!("miner {address} has duplicate proving period crons at epoch {} and {}", proving_period_cron.as_ref().unwrap().epoch, event.epoch));
                            }
                            proving_period_cron = Some(event);
                        }
                    }
                    Err(e) => acc.add(format!(
                        "miner {address} registered cron at epoch {} with wrong or corrupt payload: {e}",
                        event.epoch
                    )),
                }
                acc.require(proving_period_cron.is_some() == miner_summary.deadline_cron_active, format!("miner {address} has invalid deadline_cron_active ({}) for proving_period_cron status ({})", miner_summary.deadline_cron_active, proving_period_cron.is_some()));
                acc.require(
                    proving_period_cron.is_some(),
                    format!("miner {address} has no proving period cron"),
                );
            }
        } else {
            // with deferred and discontinued crons it is normal for a miner actor to have no cron
            // events
            acc.require(
                !miner_summary.deadline_cron_active,
                format!("miner {address} has no cron events but the deadline cron is active"),
            );
        }
    }
}

fn check_deal_states_against_sectors(
    acc: &MessageAccumulator,
    miner_summaries: &HashMap<Address, miner::StateSummary>,
    market_summary: &market::StateSummary,
) {
    // Check that all active deals are included within a non-terminated sector.
    // We cannot check that all deals referenced within a sector are in the market, because deals
    // can be terminated independently of the sector in which they are included.
    for (deal_id, deal) in &market_summary.deals {
        if deal.sector_start_epoch == -1 {
            // deal hasn't been activated yet, make no assertions about sector state
            continue;
        }

        let miner_summary = if let Some(miner_summary) = miner_summaries.get(&deal.provider) {
            miner_summary
        } else {
            acc.add(format!(
                "provider {} for deal {} not found among miners",
                deal.provider, &deal_id
            ));
            continue;
        };

        let sector_deal = if let Some(sector_deal) = miner_summary.deals.get(deal_id) {
            sector_deal
        } else {
            acc.require(
                deal.slash_epoch >= 0,
                format!(
                    "un-slashed deal {deal_id} not referenced in active sectors of miner {}",
                    deal.provider
                ),
            );
            continue;
        };

        acc.require(
            deal.sector_start_epoch == sector_deal.sector_start,
            format!(
                "deal state start {} does not match sector start {} for miner {}",
                deal.sector_start_epoch, sector_deal.sector_start, deal.provider
            ),
        );

        acc.require(
            deal.sector_start_epoch <= sector_deal.sector_expiration,
            format!(
                "deal state start {} activated after sector expiration {} for miner {}",
                deal.sector_start_epoch, sector_deal.sector_expiration, deal.provider
            ),
        );

        acc.require(
            deal.last_update_epoch <= sector_deal.sector_expiration,
            format!(
                "deal state update at {} after sector expiration {} for miner {}",
                deal.last_update_epoch, sector_deal.sector_expiration, deal.provider
            ),
        );

        acc.require(
            deal.slash_epoch <= sector_deal.sector_expiration,
            format!(
                "deal state slashed at {} after sector expiration {} for miner {}",
                deal.slash_epoch, sector_deal.sector_expiration, deal.provider
            ),
        );
    }
}

fn check_verifreg_against_datacap(
    acc: &MessageAccumulator,
    verifreg_summary: &verifreg::StateSummary,
    datacap_summary: &datacap::StateSummary,
) {
    // Verifier and datacap token holders are distinct.
    for verifier in verifreg_summary.verifiers.keys() {
        acc.require(
            !datacap_summary.balances.contains_key(&verifier.id().unwrap()),
            format!("verifier {} is also a datacap token holder", verifier),
        );
    }
    // Verifreg token balance matches unclaimed allocations.
    let pending_alloc_total: DataCap =
        verifreg_summary.allocations.iter().map(|(_, alloc)| alloc.size.0).sum();
    let verifreg_balance = datacap_summary
        .balances
        .get(&VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap())
        .cloned()
        .unwrap_or_else(TokenAmount::zero);
    acc.require(
        TokenAmount::from_whole(pending_alloc_total.clone()) == verifreg_balance,
        format!(
            "verifreg datacap balance {} does not match pending allocation size {}",
            verifreg_balance, pending_alloc_total
        ),
    );
}

fn check_market_against_verifreg(
    acc: &MessageAccumulator,
    market_summary: &market::StateSummary,
    verifreg_summary: &verifreg::StateSummary,
) {
    // all activated verified deals with claim ids reference a claim in verifreg state
    // note that it is possible for claims to exist with no matching deal if the deal expires
    for (claim_id, deal_id) in &market_summary.claim_id_to_deal_id {
        // claim is found
        let claim = match verifreg_summary.claims.get(claim_id) {
            None => {
                acc.add(format!("claim {} not found for activated deal {}", claim_id, deal_id));
                continue;
            }
            Some(claim) => claim,
        };

        let info = match market_summary.deals.get(deal_id) {
            None => {
                acc.add(format!(
                    "internal invariant error invalid market state referrences missing deal {}",
                    deal_id
                ));
                continue;
            }
            Some(info) => info,
        };
        // claim and proposal match
        acc.require(
            info.provider.id().unwrap() == claim.provider,
            format!(
                "mismatched providers {} {} on claim {} and deal {}",
                claim.provider,
                info.provider.id().unwrap(),
                claim_id,
                deal_id
            ),
        );
        acc.require(
            info.piece_cid.unwrap() == claim.data,
            format!(
                "mismatched piece cid {} {} on claim {} and deal {}",
                info.piece_cid.unwrap(),
                claim.data,
                claim_id,
                deal_id
            ),
        );
    }

    // all pending deal allocation ids have an associated allocation
    // note that it is possible for allocations to exist that don't match any deal
    // if they are created from a direct DataCap transfer
    for (allocation_id, deal_id) in &market_summary.alloc_id_to_deal_id {
        // allocation is found
        let alloc = match verifreg_summary.allocations.get(allocation_id) {
            None => {
                acc.add(format!(
                    "allocation {} not found for pending deal {}",
                    allocation_id, deal_id
                ));
                continue;
            }
            Some(alloc) => alloc,
        };
        // alloc and proposal match
        let info = match market_summary.deals.get(deal_id) {
            None => {
                acc.add(format!(
                    "internal invariant error invalid market state referrences missing deal {}",
                    deal_id
                ));
                continue;
            }
            Some(info) => info,
        };
        acc.require(
            info.provider.id().unwrap() == alloc.provider,
            format!(
                "mismatched providers {} {} on alloc {} and deal {}",
                alloc.provider,
                info.provider.id().unwrap(),
                allocation_id,
                deal_id
            ),
        );
        acc.require(
            info.piece_cid.unwrap() == alloc.data,
            format!(
                "mismatched piece cid {} {} on alloc {} and deal {}",
                info.piece_cid.unwrap(),
                alloc.data,
                allocation_id,
                deal_id
            ),
        );
    }
}

fn check_verifreg_against_miners(
    acc: &MessageAccumulator,
    verifreg_summary: &verifreg::StateSummary,
    miner_summaries: &HashMap<Address, miner::StateSummary>,
) {
    for claim in verifreg_summary.claims.values() {
        // all claims are indexed by valid providers
        let maddr = Address::new_id(claim.provider);
        let miner_summary = match miner_summaries.get(&maddr) {
            None => {
                acc.add(format!("claim provider {} is not found in miner summaries", maddr));
                continue;
            }
            Some(summary) => summary,
        };

        // all claims are linked to a valid sector number
        acc.require(
            miner_summary.sectors_with_deals.get(&claim.sector).is_some(),
            format!(
                "claim sector number {} not recorded as a sector with deals for miner {}",
                claim.sector, maddr
            ),
        );
    }
}

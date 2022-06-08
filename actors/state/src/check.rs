use std::collections::HashMap;
use std::fmt::Debug;

use anyhow::bail;
use cid::Cid;
use fil_actor_account::State as AccountState;
use fil_actor_cron::State as CronState;
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
use fil_actor_verifreg::State as VerifregState;

use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::Map;
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::from_slice;
use fvm_ipld_encoding::CborStore;
use fvm_shared::actor::builtin::Manifest;
use fvm_shared::actor::builtin::Type;
use fvm_shared::address::Address;
use fvm_shared::address::Protocol;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use anyhow::anyhow;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::bigint::bigint_ser;

use fil_actor_account::testing as account;
use fil_actor_cron::testing as cron;
use fil_actor_init::testing as init;
use fil_actor_market::testing as market;
use fil_actor_miner::testing as miner;
use fil_actor_multisig::testing as multisig;
use fil_actor_paych::testing as paych;
use fil_actor_power::testing as power;
use fil_actor_reward::testing as reward;
use fil_actor_verifreg::testing as verifreg;

/// Value type of the top level of the state tree.
/// Represents the on-chain state of a single actor.
#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Debug)]
pub struct Actor {
    /// CID representing the code associated with the actor
    pub code: Cid,
    /// CID of the head state object for the actor
    pub head: Cid,
    /// `call_seq_num` for the next message to be received by the actor (non-zero for accounts only)
    pub call_seq_num: u64,
    #[serde(with = "bigint_ser")]
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

pub fn check_state_invariants<'a, BS: Blockstore + Debug>(
    manifest: &Manifest,
    policy: &Policy,
    tree: Tree<'a, BS>,
    expected_balance_total: &TokenAmount,
    prior_epoch: ChainEpoch,
) -> anyhow::Result<MessageAccumulator> {
    let acc = MessageAccumulator::default();
    let mut total_fil = BigInt::zero();

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
                let (summary, msgs) =
                    market::check_state_invariants(&state, tree.store, &actor.balance, prior_epoch);
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
                let (summary, msgs) = verifreg::check_state_invariants(&state, tree.store);
                acc.with_prefix("verifreg: ").add_all(&msgs);
                verifreg_summary = Some(summary);
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

    if let Some(market_summary) = market_summary {
        check_deal_states_against_sectors(&acc, &miner_summaries, &market_summary);
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

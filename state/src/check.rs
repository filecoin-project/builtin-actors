use std::collections::BTreeMap;
use std::collections::HashMap;

use anyhow::anyhow;
use anyhow::bail;
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
use fil_actors_runtime::DealWeight;
use fil_actors_runtime::MessageAccumulator;
use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::from_slice;
use fvm_ipld_encoding::CborStore;
use fvm_shared::address::Address;
use fvm_shared::address::Protocol;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorNumber;
use num_traits::Zero;

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
use vm_api::ActorState;

macro_rules! get_state {
    ($store:ident, $actor:ident, $state:ty) => {
        $store
            .get_cbor::<$state>(&$actor.state)?
            .ok_or_else(|| anyhow!("{} is empty", stringify!($state)))?
    };
}

// Note: BiBTreeMap is an overly constrained type for what we are doing here, but chosen
// to match the Manifest implementation in the FVM.
// It could be replaced with a custom mapping trait (while Rust doesn't support
// abstract collection traits).
pub fn check_state_invariants<BS: Blockstore>(
    store: &BS,
    manifest: &BTreeMap<Cid, Type>,
    policy: &Policy,
    tree: &BTreeMap<Address, ActorState>,
    expected_balance_total: Option<TokenAmount>,
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
    let mut datacap_summary: Option<frc46_token::token::state::StateSummary> = None;

    tree.iter().try_for_each(|(key, actor)| -> anyhow::Result<()> {
        let acc = acc.with_prefix(format!("{key} "));

        if key.protocol() != Protocol::ID {
            acc.add(format!("unexpected address protocol in state tree root: {key}"));
        }
        total_fil += &actor.balance;

        match manifest.get(&actor.code) {
            Some(Type::System) => (),
            Some(Type::Init) => {
                let state = get_state!(store, actor, InitState);
                let (summary, msgs) = init::check_state_invariants(&state, store);
                acc.with_prefix("init: ").add_all(&msgs);
                init_summary = Some(summary);
            }
            Some(Type::Cron) => {
                let state = get_state!(store, actor, CronState);
                let (summary, msgs) = cron::check_state_invariants(&state);
                acc.with_prefix("cron: ").add_all(&msgs);
                cron_summary = Some(summary);
            }
            Some(Type::Account) => {
                let state = get_state!(store, actor, AccountState);
                let (summary, msgs) = account::check_state_invariants(&state, key);
                acc.with_prefix("account: ").add_all(&msgs);
                account_summaries.push(summary);
            }
            Some(Type::Power) => {
                let state = get_state!(store, actor, PowerState);
                let (summary, msgs) = power::check_state_invariants(policy, &state, store);
                acc.with_prefix("power: ").add_all(&msgs);
                power_summary = Some(summary);
            }
            Some(Type::Miner) => {
                let state = get_state!(store, actor, MinerState);
                let (summary, msgs) =
                    miner::check_state_invariants(policy, &state, store, &actor.balance);
                acc.with_prefix("miner: ").add_all(&msgs);
                miner_summaries.insert(*key, summary);
            }
            Some(Type::Market) => {
                let state = get_state!(store, actor, MarketState);
                let (summary, msgs) =
                    market::check_state_invariants(&state, store, &actor.balance, prior_epoch + 1);
                acc.with_prefix("market: ").add_all(&msgs);
                market_summary = Some(summary);
            }
            Some(Type::PaymentChannel) => {
                let state = get_state!(store, actor, PaychState);
                let (summary, msgs) = paych::check_state_invariants(&state, store, &actor.balance);
                acc.with_prefix("paych: ").add_all(&msgs);
                paych_summaries.push(summary);
            }
            Some(Type::Multisig) => {
                let state = get_state!(store, actor, MultisigState);
                let (summary, msgs) = multisig::check_state_invariants(&state, store);
                acc.with_prefix("multisig: ").add_all(&msgs);
                multisig_summaries.push(summary);
            }
            Some(Type::Reward) => {
                let state = get_state!(store, actor, RewardState);
                let (summary, msgs) =
                    reward::check_state_invariants(&state, prior_epoch, &actor.balance);
                acc.with_prefix("reward: ").add_all(&msgs);
                reward_summary = Some(summary);
            }
            Some(Type::VerifiedRegistry) => {
                let state = get_state!(store, actor, VerifregState);
                let (summary, msgs) = verifreg::check_state_invariants(&state, store, prior_epoch);
                acc.with_prefix("verifreg: ").add_all(&msgs);
                verifreg_summary = Some(summary);
            }
            Some(Type::DataCap) => {
                let state = get_state!(store, actor, DataCapState);
                let (summary, msgs) = datacap::check_state_invariants(&state, store);
                acc.with_prefix("datacap: ").add_all(&msgs);
                datacap_summary = Some(summary);
            }
            Some(Type::Placeholder) => {}
            Some(Type::EVM) => {}
            Some(Type::EAM) => {}
            Some(Type::EthAccount) => {}
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

    if let Some(expected_balance_total) = expected_balance_total {
        acc.require(
            total_fil == expected_balance_total,
            format!("total token balance is {total_fil}, expected {expected_balance_total}"),
        );
    }

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

        let _miner_summary = if let Some(miner_summary) = miner_summaries.get(&deal.provider) {
            miner_summary
        } else {
            acc.add(format!(
                "provider {} for deal {} not found among miners",
                deal.provider, &deal_id
            ));
            continue;
        };
    }
}

fn check_verifreg_against_datacap(
    acc: &MessageAccumulator,
    verifreg_summary: &verifreg::StateSummary,
    datacap_summary: &frc46_token::token::state::StateSummary,
) {
    // Verifier and datacap token holders are distinct.
    for verifier in verifreg_summary.verifiers.keys() {
        acc.require(
            !datacap_summary.balance_map.as_ref().unwrap().contains_key(&verifier.id().unwrap()),
            format!("verifier {} is also a datacap token holder", verifier),
        );
    }
    // Verifreg token balance matches unclaimed allocations.
    let pending_alloc_total: DataCap =
        verifreg_summary.allocations.values().map(|alloc| alloc.size.0).sum();
    let verifreg_balance = datacap_summary
        .balance_map
        .as_ref()
        .unwrap()
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
                    "internal invariant error invalid market state references missing deal {}",
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
    // Accumulates the weight of claims for each sector.
    let mut sector_claim_verified_weights: BTreeMap<(Address, SectorNumber), DealWeight> =
        BTreeMap::new();

    for (id, claim) in &verifreg_summary.claims {
        // All claims are indexed by valid providers
        let maddr = Address::new_id(claim.provider);
        let miner_summary = match miner_summaries.get(&maddr) {
            None => {
                acc.add(format!("claim provider {} is not found in miner summaries", maddr));
                continue;
            }
            Some(summary) => summary,
        };

        // Find sectors associated with claims.
        // A claim might not have a sector if the sector was terminated and cleaned up.
        if let Some(sector) = miner_summary.live_data_sectors.get(&claim.sector) {
            acc.require(
                sector.sector_start <= claim.term_start,
                format!(
                    "claim {} sector start {} is after claim term start {} for miner {}",
                    id, sector.sector_start, claim.term_start, maddr
                ),
            );
            // Legacy QAP sectors can be extended to expire after the associated claim term,
            // and verified deal weight depends age prior to extension.
            if !sector.legacy_qap {
                acc.require(
                    sector.sector_expiration >= claim.term_start + claim.term_min,
                    format!(
                        "claim {} sector expiration {} is before claim min term {} for miner {}",
                        id,
                        sector.sector_start,
                        claim.term_start + claim.term_min,
                        maddr
                    ),
                );
                acc.require(
                    sector.sector_expiration <= claim.term_start + claim.term_max,
                    format!(
                        "claim {} sector expiration {} is after claim term max {} for miner {}",
                        id,
                        sector.sector_expiration,
                        claim.term_start + claim.term_max,
                        maddr
                    ),
                );
                let expected_duration = sector.sector_expiration - claim.term_start;
                let expected_weight = DealWeight::from(claim.size.0) * expected_duration;
                *sector_claim_verified_weights.entry((maddr, claim.sector)).or_default() +=
                    expected_weight;
            }
        }
    }
    for ((maddr, sector), claim_weight) in &sector_claim_verified_weights {
        let miner_summary = miner_summaries.get(maddr).unwrap();
        let sector = miner_summary.live_data_sectors.get(sector).unwrap();
        acc.require(
            sector.verified_deal_weight == *claim_weight,
            format!(
                "sector verified weight {} does not match claims of {} for miner {}",
                sector.verified_deal_weight, claim_weight, maddr
            ),
        )
    }
}

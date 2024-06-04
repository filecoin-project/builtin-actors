use std::collections::HashMap;
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryFrom,
};

use cid::multihash::{Code, MultihashDigest};
use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::DAG_CBOR;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{
    address::{Address, Protocol},
    clock::{ChainEpoch, EPOCH_UNDEFINED},
    deal::DealID,
    econ::TokenAmount,
    ActorID,
};
use integer_encoding::VarInt;
use num_traits::Zero;

use fil_actors_runtime::builtin::HAMT_BIT_WIDTH;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth, ActorError, AsActorError, MessageAccumulator,
};

use crate::ext::verifreg::AllocationID;
use crate::{
    balance_table::BalanceTable, DealArray, DealMetaArray, DealOpsByEpoch, DealProposal,
    PendingProposalsSet, ProviderSectorsMap, SectorDealsMap, State, DEAL_OPS_BY_EPOCH_CONFIG,
    PENDING_PROPOSALS_CONFIG, PROVIDER_SECTORS_CONFIG, SECTOR_DEALS_CONFIG,
};

#[derive(Clone)]
pub struct DealSummary {
    pub provider: Address,
    pub start_epoch: ChainEpoch,
    pub end_epoch: ChainEpoch,
    pub sector_number: SectorNumber,
    pub sector_start_epoch: ChainEpoch,
    pub last_update_epoch: ChainEpoch,
    pub slash_epoch: ChainEpoch,
    pub piece_cid: Option<Cid>,
}

impl Default for DealSummary {
    fn default() -> Self {
        Self {
            provider: Address::new_id(0),
            start_epoch: 0,
            end_epoch: 0,
            sector_number: 0,
            sector_start_epoch: -1,
            last_update_epoch: -1,
            slash_epoch: -1,
            piece_cid: None,
        }
    }
}

#[derive(Default, Clone)]
pub struct StateSummary {
    pub deals: BTreeMap<DealID, DealSummary>,
    pub provider_sector_deals: HashMap<ActorID, HashMap<SectorNumber, Vec<DealID>>>,
    pub alloc_id_to_deal_id: BTreeMap<u64, DealID>,
    pub pending_proposal_count: u64,
    pub deal_state_count: u64,
    pub lock_table_count: u64,
    pub deal_op_epoch_count: u64,
    pub deal_op_count: u64,
}

/// Checks internal invariants of market state
pub fn check_state_invariants<BS: Blockstore>(
    state: &State,
    store: &BS,
    balance: &TokenAmount,
    current_epoch: ChainEpoch,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();

    acc.require(
        !state.total_client_locked_collateral.is_negative(),
        format!(
            "negative total client locked collateral: {}",
            state.total_client_locked_collateral
        ),
    );
    acc.require(
        !state.total_provider_locked_collateral.is_negative(),
        format!(
            "negative total provider locked collateral: {}",
            state.total_provider_locked_collateral
        ),
    );
    acc.require(
        !state.total_client_storage_fee.is_negative(),
        format!("negative total client storage fee: {}", state.total_client_storage_fee),
    );

    // Proposals
    let mut proposal_cids = BTreeSet::<Cid>::new();
    let mut max_deal_id = -1;
    let mut proposal_stats = BTreeMap::<DealID, DealSummary>::new();
    let mut expected_deal_ops = BTreeSet::<DealID>::new();
    let mut total_proposal_collateral = TokenAmount::zero();

    match DealArray::load(&state.proposals, store) {
        Ok(proposals) => {
            let ret = proposals.for_each(|deal_id, proposal| {
                let proposal_cid = deal_cid(proposal)?;

                if proposal.start_epoch >= current_epoch {
                    expected_deal_ops.insert(deal_id);
                }

                // keep some state
                proposal_cids.insert(proposal_cid);
                max_deal_id = max_deal_id.max(deal_id as i64);

                proposal_stats.insert(
                    deal_id,
                    DealSummary {
                        provider: proposal.provider,
                        start_epoch: proposal.start_epoch,
                        end_epoch: proposal.end_epoch,
                        piece_cid: Some(proposal.piece_cid),
                        ..Default::default()
                    },
                );

                total_proposal_collateral +=
                    &proposal.client_collateral + &proposal.provider_collateral;

                acc.require(
                    proposal.client.protocol() == Protocol::ID,
                    "client address for deal {deal_id} is not an ID address",
                );
                acc.require(
                    proposal.provider.protocol() == Protocol::ID,
                    "provider address for deal {deal_id} is not an ID address",
                );
                Ok(())
            });
            acc.require_no_error(ret, "error iterating proposals");
        }
        Err(e) => acc.add(format!("error loading proposals: {e}")),
    };

    // next id should be higher than any existing deal
    acc.require(
        state.next_id as i64 > max_deal_id,
        format!(
            "next id, {}, is not greater than highest id in proposals, {max_deal_id}",
            state.next_id
        ),
    );

    let mut pending_allocations = BTreeMap::<DealID, AllocationID>::new();
    let mut alloc_id_to_deal_id = BTreeMap::<AllocationID, DealID>::new();
    match make_map_with_root_and_bitwidth(&state.pending_deal_allocation_ids, store, HAMT_BIT_WIDTH)
    {
        Ok(pending_allocations_hamt) => {
            let ret = pending_allocations_hamt.for_each(|key, allocation_id| {
                let deal_id: u64 = u64::decode_var(key.0.as_slice()).unwrap().0;

                acc.require(
                    proposal_stats.get(&deal_id).is_some(),
                    format!("pending deal allocation {} not found in proposals", deal_id),
                );

                pending_allocations.insert(deal_id, *allocation_id);
                alloc_id_to_deal_id.insert(*allocation_id, deal_id);
                Ok(())
            });
            acc.require_no_error(ret, "error iterating pending allocations");
        }
        Err(e) => acc.add(format!("error loading pending allocations: {e}")),
    };

    // deal states
    let mut deal_state_count = 0;
    match DealMetaArray::load(&state.states, store) {
        Ok(deal_states) => {
            let ret = deal_states.for_each(|deal_id, deal_state| {
                acc.require(
                    deal_state.sector_start_epoch >= 0,
                    format!("deal {deal_id} state start epoch undefined: {:?}", deal_state),
                );
                acc.require(
                    deal_state.last_updated_epoch == EPOCH_UNDEFINED
                        || deal_state.last_updated_epoch >= deal_state.sector_start_epoch,
                    format!(
                        "deal {deal_id} state last updated before sector start: {deal_state:?}"
                    ),
                );
                acc.require(
                    deal_state.last_updated_epoch == EPOCH_UNDEFINED
                        || deal_state.last_updated_epoch <= current_epoch,
                    format!(
                        "deal {deal_id} last updated epoch {} after current {current_epoch}",
                        deal_state.last_updated_epoch
                    ),
                );
                acc.require(deal_state.slash_epoch == EPOCH_UNDEFINED || deal_state.slash_epoch >= deal_state.sector_start_epoch, format!("deal {deal_id} state slashed before sector start: {deal_state:?}"));
                acc.require(deal_state.slash_epoch == EPOCH_UNDEFINED || deal_state.slash_epoch <= current_epoch, format!("deal {deal_id} state slashed after current epoch {current_epoch}: {deal_state:?}"));

                if let Some(stats) = proposal_stats.get_mut(&deal_id) {
                    stats.sector_number = deal_state.sector_number;
                    stats.sector_start_epoch = deal_state.sector_start_epoch;
                    stats.last_update_epoch = deal_state.last_updated_epoch;
                    stats.slash_epoch = deal_state.slash_epoch;
                } else {
                    acc.add(format!("no deal proposal for deal state {deal_id}"));
                }
                acc.require(!pending_allocations.contains_key(&deal_id), format!("deal {deal_id} has pending allocation"));

                deal_state_count += 1;
                Ok(())
            });
            acc.require_no_error(ret, "error iterating deal states");
        }
        Err(e) => acc.add(format!("error loading deal states: {e}")),
    };

    // Provider->sector->deal mapping
    // Each entry corresponds to non-terminated deal state.
    // A deal may have expired but remain in the mapping until settlement.
    let mut provider_sector_deals = HashMap::<ActorID, HashMap<SectorNumber, Vec<DealID>>>::new();
    match ProviderSectorsMap::load(
        store,
        &state.provider_sectors,
        PROVIDER_SECTORS_CONFIG,
        "provider sectors",
    ) {
        Ok(provider_sectors) => {
            let ret = provider_sectors.for_each(|provider, sectors_root| {
                match SectorDealsMap::load(store, sectors_root, SECTOR_DEALS_CONFIG, "sector deals") {
                    Ok(sectors_deals) => {
                        let ret = sectors_deals.for_each(|sector, deal_ids| {
                            for deal_id in deal_ids {
                                provider_sector_deals.entry(provider).or_default().entry(sector).or_default().push(*deal_id);
                                if let Some(stats) = proposal_stats.get(deal_id) {
                                    acc.require(
                                        stats.provider == Address::new_id(provider),
                                        format!(
                                            "provider sector deal {deal_id} provider {provider} does not match proposal provider {}", stats.provider
                                        ),
                                    );
                                    acc.require(
                                        stats.sector_number == sector,
                                        format!(
                                            "provider sector deal {deal_id} sector {sector} does not match proposal sector {}", stats.sector_number
                                        ),
                                    );
                                    acc.require(stats.slash_epoch == EPOCH_UNDEFINED, format!("provider sector deal {deal_id} is slashed"));
                                } else {
                                    acc.add(format!("provider sector deal {deal_id} not found in proposals"));
                                }
                            }
                            Ok(())
                        });
                        acc.require_no_error(ret, "error iterating provider sector deals");
                    }
                    Err(e) => acc.add(format!("error loading provider sector deals: {e}")),
                }
                Ok(())
            });
            acc.require_no_error(ret, "error iterating provider sector deals");
        }
        Err(e) => acc.add(format!("error loading provider sector deals: {e}")),
    };
    // Check the reverse direction, which is almost the same.
    // Every non-terminated, *non-expired* deal state should be in provider sector deals.
    // Terminated deals are removed synchronously.
    // Expired deals are be removed from the mapping at settlement (which may not have happened).
    // Note expired deals from terminated sectors are removed from the mapping even before settlement.
    for (id, stats) in &proposal_stats {
        if stats.sector_start_epoch >= 0
            && stats.slash_epoch == EPOCH_UNDEFINED
            && stats.end_epoch > current_epoch
        {
            let sector_deals = provider_sector_deals
                .get(&stats.provider.id().unwrap())
                .and_then(|p| p.get(&stats.sector_number));
            acc.require(
                sector_deals.map(|v| v.contains(id)).unwrap_or(false),
                format!("active deal {id} not found in provider sector deals"),
            );
        }
    }

    // pending proposals
    let mut pending_proposal_count = 0;
    match PendingProposalsSet::load(
        store,
        &state.pending_proposals,
        PENDING_PROPOSALS_CONFIG,
        "pending proposals",
    ) {
        Ok(pending_proposals) => {
            let ret = pending_proposals.for_each(|key| {
                let proposal_cid = Cid::try_from(key.to_owned())
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "not a CID")?;
                acc.require(
                    proposal_cids.contains(&proposal_cid),
                    format!("pending proposal with cid {proposal_cid} not found within proposals"),
                );

                pending_proposal_count += 1;
                Ok(())
            });
            acc.require_no_error(ret, "error iterating pending proposals");
        }
        Err(e) => acc.add(format!("error loading pending proposals: {e}")),
    };

    // escrow table and locked table
    let mut lock_table_count = 0;
    let escrow_table = BalanceTable::from_root(store, &state.escrow_table, "escrow table");
    let lock_table = BalanceTable::from_root(store, &state.locked_table, "locked table");

    match (escrow_table, lock_table) {
        (Ok(escrow_table), Ok(lock_table)) => {
            let mut locked_total = TokenAmount::zero();
            let ret = lock_table.0.for_each(|address, locked_amount| {
                locked_total += locked_amount;

                // every entry in locked table should have a corresponding entry in escrow table that is at least as high
                let escrow_amount = &escrow_table.get(&address)?;
                acc.require(escrow_amount >= locked_amount, format!("locked funds for {address}, {locked_amount}, greater than escrow amount, {escrow_amount}"));

                lock_table_count += 1;

                Ok(())
            });
            acc.require_no_error(ret, "error iterating locked table");

            // lockTable total should be sum of client and provider locked plus client storage fee
            let expected_lock_total = &state.total_provider_locked_collateral
                + &state.total_client_locked_collateral
                + &state.total_client_storage_fee;
            acc.require(locked_total == expected_lock_total, format!("locked total, {locked_total}, does not sum to provider locked, {}, client locked, {}, and client storage fee, {}", state.total_provider_locked_collateral, state.total_client_locked_collateral, state.total_client_storage_fee));

            // assert escrow <= actor balance
            // lock_table item <= escrow item and escrow_total <= balance implies lock_table total <= balance
            match escrow_table.total() {
                Ok(escrow_total) => {
                    acc.require(
                        &escrow_total <= balance,
                        format!(
                            "escrow total, {escrow_total}, greater than actor balance, {balance}"
                        ),
                    );
                    acc.require(escrow_total >= total_proposal_collateral, format!("escrow total, {escrow_total}, less than sum of proposal collateral, {total_proposal_collateral}"));
                }
                Err(e) => acc.add(format!("error calculating escrow total: {e}")),
            }
        }
        (escrow_table, lock_table) => {
            acc.require_no_error(escrow_table, "error loading escrow table");
            acc.require_no_error(lock_table, "error loading locked table");
        }
    };

    // deals ops by epoch
    let (mut deal_op_epoch_count, mut deal_op_count) = (0, 0);
    match DealOpsByEpoch::load(
        store,
        &state.deal_ops_by_epoch,
        DEAL_OPS_BY_EPOCH_CONFIG,
        "deal ops",
    ) {
        Ok(deal_ops) => {
            let ret = deal_ops.for_each(|epoch: ChainEpoch, _| {
                deal_op_epoch_count += 1;
                deal_ops.for_each_in(&epoch, |deal_id: DealID| {
                    expected_deal_ops.remove(&deal_id);
                    deal_op_count += 1;
                    Ok(())
                })
            });
            acc.require_no_error(ret, "error iterating all deal ops");
        }
        Err(e) => acc.add(format!("error loading deal ops: {e}")),
    };

    acc.require(
        expected_deal_ops.is_empty(),
        format!("missing deal ops for proposals: {expected_deal_ops:?}"),
    );

    (
        StateSummary {
            deals: proposal_stats,
            provider_sector_deals,
            pending_proposal_count,
            deal_state_count,
            lock_table_count,
            deal_op_epoch_count,
            deal_op_count,
            alloc_id_to_deal_id,
        },
        acc,
    )
}

/// Compute a deal CID directly (the actor code uses a runtime built-in).
pub(crate) fn deal_cid(proposal: &DealProposal) -> Result<Cid, ActorError> {
    const DIGEST_SIZE: u32 = 32;
    let data = serialize(proposal, "deal proposal")?;
    let hash = Code::Blake2b256.digest(data.bytes());
    debug_assert_eq!(u32::from(hash.size()), DIGEST_SIZE, "expected 32byte digest");
    Ok(Cid::new_v1(DAG_CBOR, hash))
}

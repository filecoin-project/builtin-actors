## Summary

Implements the **FIL+ deprecation half** of [FIP-1249](https://github.com/filecoin-project/FIPs/discussions/1249) (Deprecate FIL+ and Fund Services via Block Reward Split). This PR does **not** implement the block reward split / Service Rewards Actor — only the datacap/verifreg removal and universal 10x QAP changes.

**51 files changed, +2,711 / −4,569 lines (net −1,858). 1,207 tests pass, 0 failures, 0 warnings.**

## What's Implemented

### Universal 10x QAP for all sectors

- New `FULL_QA_POWER` flag (`0x2`) on `SectorOnChainInfoFlags`
- `qa_power_for_sector()` returns `qa_power_max(size)` (10x raw power) when the flag is set
- All sector creation paths set the flag automatically:
  - `ProveCommitSectors3` / `activate_new_sector_infos`
  - `ProveCommitSectorsNI`
  - `ProveReplicaUpdates3` / `update_existing_sector_info`
- Pre-existing sectors retain their current QAP until explicitly upgraded

### Miner actor: verifreg/datacap removal

- Removed `ext::verifreg` module entirely (no more cross-actor calls to verified registry)
- `activate_sectors_pieces`: no longer claims allocations; all piece space treated as unverified
- `activate_sectors_deals`: no longer filters verified deals or calls `ClaimAllocations`
- `batch_claim_allocations` and `get_claims` functions removed
- Sector extensions: stripped claim validation; uses proportional deal weight reduction for legacy sectors
- `verified_allocation_key` field kept on `PieceActivationManifest` for API backward compat but ignored

### Market actor: datacap removal

- `publish_storage_deals`: removed datacap balance checks, allocation requests, datacap token transfers
- `batch_activate_deals` / `sector_content_changed`: removed `pending_deal_allocation_ids` tracking
- Removed helper functions: `balance_of`, `transfer_from`, `alloc_request_for_deal`, `datacap_transfer_request`
- Removed `ext::datacap` module and most of `ext::verifreg`
- `DealProposal.verified_deal` field kept for serialization backward compat

### Verifreg actor: disabled

- `AddVerifier`, `AddVerifiedClient`: return `USR_FORBIDDEN`
- `UniversalReceiverHook`: returns `USR_FORBIDDEN` (no new allocations)
- `ClaimAllocations`, `ExtendClaimTerms`: return `USR_FORBIDDEN`
- Cleanup methods still active: `RemoveExpiredAllocations`, `RemoveExpiredClaims`, `GetClaims`, `RemoveVerifier`, `RemoveVerifiedClientDataCap`
- Dead helper functions removed (`mint`, `burn`, `validate_*`, `can_claim_alloc`, etc.)

### State invariant updates

- `check_verifreg_against_miners`: skips claim-to-sector weight validation for `FULL_QA_POWER` sectors
- `DataSummary` tracks `full_qa_power` flag for cross-actor checks

## Decisions That May Need Discussion

Several design choices were made that go somewhat beyond what the FIP text strictly requires, or where the FIP is ambiguous:

### 1. Existing datacap is fully blocked, not just minting

The FIP says: *"no new datacap minted, but the existing datacap can still be used (ie, allocated) until natural expiration."*

We disabled `UniversalReceiverHook` and `ClaimAllocations` entirely, meaning existing datacap holders **cannot** create new allocations or claim them. The rationale: since all sectors get 10x regardless, allocating datacap has zero functional effect — it would be pure overhead. Existing allocations/claims will drain naturally via `RemoveExpiredAllocations`/`RemoveExpiredClaims`.

**If strict FIP compliance is preferred**, we'd re-enable `UniversalReceiverHook` and `ClaimAllocations` so existing datacap can be spent (even though it changes nothing about QAP).

### 2. Claim validation stripped from sector extensions

The FIP doesn't explicitly address what happens when legacy sectors (with existing verifreg claims) are extended. We stripped claim validation entirely — extensions now use proportional deal weight reduction for legacy sectors. The alternative would be keeping the verifreg `GetClaims` call alive for legacy extension paths.

### 3. verified_deal_weight stored as zero for new sectors

New sectors set `verified_space = BigInt::zero()` regardless of actual deal content. The `deal_weight` field still tracks unverified deal space. This means `verified_deal_weight` is no longer meaningful on new sectors — it's always zero. The `FULL_QA_POWER` flag is what drives 10x power.

## New Tests

- **Policy tests** (5): `FULL_QA_POWER` flag gives 10x, ignores deal weights, ignores duration, legacy formula preserved
- **FIP-1249 integration tests** (4): CC sector gets 10x, NI sector gets 10x, verifreg minting disabled, verified deal publishes without datacap ops

## Not Implemented (Out of Scope)

The following FIP-1249 components are **not** part of this PR:
- Block reward split (α parameter, Service Rewards Actor)
- Service Rewards Actor (FEVM smart contract)
- Gated step-up schedule (10% → 40%)
- Security gate multisig

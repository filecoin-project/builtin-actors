//! Deterministic random sequences of stream mutations and awards.
//!
//! Each step checks ledger validity, the future weight schedule, allocation conservation
//! and explicit liabilities. Awards must succeed with ample funding; rejected mutations
//! discard both their changes and any due writes.
//!
//! Exercises the ledger directly, not actor authorization, transfers or storage failures.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

use super::*;
use crate::streams::invariants::schedule;

const TIMELOCK: ChainEpoch = 20;
const SEEDS: u64 = 32;
const STEPS: usize = 2_000;
/// Small wallet pool to exercise repeated claims and recipient changes.
const WALLETS: u64 = 24;
const FIRST_WALLET: u64 = 10_000;
const FIRST_WRITER: u64 = 20_000;
const FIRST_GENERATED_STREAM: StreamId = 3;

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        // SplitMix64
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0);
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next();
            if value >= threshold {
                return value % bound;
            } // else avoid the modulo bias
        }
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        (!items.is_empty()).then(|| &items[self.below(items.len() as u64) as usize])
    }

    fn wallet(&mut self) -> Address {
        Address::new_id(FIRST_WALLET + self.below(WALLETS))
    }

    /// Positive shares summing to DENOM, optionally including f099.
    fn share_map(&mut self) -> Vec<RecipientShare> {
        let count = 1 + self.below(5) as usize;
        let mut recipients: Vec<Address> = Vec::new();
        while recipients.len() < count {
            let wallet = self.wallet();
            if !recipients.contains(&wallet) {
                recipients.push(wallet);
            }
        }
        if self.chance(4) {
            recipients.push(BURNT_FUNDS_ACTOR_ADDR);
        }
        if self.chance(8) {
            recipients.push(BURNT_FUNDS_ACTOR_ADDR);
        }
        // Cut up a full DENOM share spread randomly, then assign the shares to recipients.
        let mut cuts = Vec::with_capacity(recipients.len() + 1);
        cuts.extend([0, DENOM]);
        while cuts.len() < recipients.len() + 1 {
            let cut = 1 + self.below(DENOM - 1);
            if !cuts.contains(&cut) {
                cuts.push(cut);
            }
        }
        cuts.sort_unstable();
        recipients
            .into_iter()
            .zip(cuts.windows(2))
            .map(|(recipient, interval)| RecipientShare {
                recipient,
                share: interval[1] - interval[0],
            })
            .collect()
    }

    /// Valid weight fields with positive, negative or zero slopes.
    fn weight_record(&mut self, epoch: ChainEpoch) -> WeightRecord {
        let cap = 1 + self.below(pct(45));
        let floor = self.below(cap + 1);
        let v_start = floor + self.below(cap - floor + 1);
        let slope = match self.below(4) {
            0 => 0,
            1 => -(self.below(pct(1)) as i64),
            _ => self.below(pct(1)) as i64,
        };
        let t_start = epoch - 100 + self.below(400) as ChainEpoch;
        WeightRecord { v_start, slope, t_start, floor, cap }
    }
}

/// Aggregate allocations and claims, tracked separately from ledger balances.
#[derive(Default)]
struct Tally {
    minted: TokenAmount,
    miner: TokenAmount,
    explicit: TokenAmount,
    /// Minted burn allocation, including sentinel shares.
    burned: TokenAmount,
    /// Rounding dust from explicit-stream period folds.
    dust: TokenAmount,
    claimed: TokenAmount,
}

impl Tally {
    fn held(&self) -> TokenAmount {
        &self.explicit - &self.claimed - &self.dust
    }
}

/// Every operation and outcome the walk has to reach across the fixed seeds.
const REQUIRED_COVERAGE: [&str; 19] = [
    "awards",
    "accruing awards",
    "accruing awards to registered streams",
    "null gaps",
    "paying claims",
    "paying claims from registered streams",
    "share installs",
    "sentinel share maps",
    "address swaps",
    "registrations",
    "implicit registrations",
    "removals",
    "writer changes",
    "weight writes",
    "cancellations",
    "applied writes",
    "dropped writes",
    "tombstones",
    "rejections",
];

#[derive(Default, Debug)]
struct Coverage(BTreeMap<&'static str, usize>);

impl Coverage {
    fn hit(&mut self, name: &'static str) {
        self.add(name, 1);
    }

    fn hit_if(&mut self, name: &'static str, reached: bool) {
        self.add(name, usize::from(reached));
    }

    fn add(&mut self, name: &'static str, count: usize) {
        *self.0.entry(name).or_default() += count;
    }

    fn merge(&mut self, other: &Coverage) {
        for (name, count) in &other.0 {
            self.add(name, *count);
        }
    }
}

struct Walk {
    streams: StreamsState,
    accrued: Vec<StreamAccrual>,
    epoch: ChainEpoch,
    next_id: StreamId,
    tally: Tally,
    coverage: Coverage,
}

impl Walk {
    fn new() -> Walk {
        let (streams, accrued) = base_state();
        Walk {
            streams,
            accrued,
            epoch: 1_000,
            next_id: FIRST_GENERATED_STREAM,
            tally: Tally::default(),
            coverage: Coverage::default(),
        }
    }

    /// Validate the state retained after the preceding operation.
    fn load(&self) -> Ledger {
        Ledger::checked(self.streams.clone(), self.accrued.clone()).unwrap_or_else(|error| {
            panic!("state stopped loading at epoch {}: {error}", self.epoch)
        })
    }

    fn commit(&mut self, ledger: Ledger, applied: &ApplyResult) {
        self.coverage.add("applied writes", applied.applied.len());
        self.coverage.add("dropped writes", applied.dropped.len());
        self.tally.dust += &applied.fold_dust;
        self.streams = ledger.streams;
        self.accrued = ledger.accrued;
    }

    /// Apply due writes and a mutation, discarding both if the mutation fails.
    fn method<T>(
        &mut self,
        act: impl FnOnce(&mut Ledger, ChainEpoch) -> anyhow::Result<T>,
    ) -> Option<T> {
        let mut ledger = self.load();
        let applied = ledger.apply_due(self.epoch);
        match act(&mut ledger, self.epoch) {
            Ok(value) => {
                self.commit(ledger, &applied);
                Some(value)
            }
            Err(_) => {
                self.coverage.hit("rejections");
                None
            }
        }
    }

    fn award(&mut self, random: &mut Random) {
        let ledger = self.load();
        let block_reward = TokenAmount::from_atto(1 + random.below(10_000_000));
        let gas_reward = TokenAmount::from_atto(random.below(1_000));
        // We're not doing reserve exhaustion testing here, so avoid that case.
        let balance = TokenAmount::from_whole(1_000_000) + self.tally.held() + &gas_reward;
        let (ledger, award) = plan_award(ledger, self.epoch, &balance, &gas_reward, &block_reward)
            .unwrap_or_else(|| panic!("award degraded at epoch {}", self.epoch));
        assert_eq!(block_reward, award.block_reward);
        let explicit: TokenAmount =
            award.allocation.portions.iter().map(|(_, amount)| amount).sum();
        assert_eq!(block_reward, &award.allocation.miner + &explicit + &award.allocation.burn);
        self.tally.minted += &block_reward;
        self.tally.miner += &award.allocation.miner;
        self.tally.explicit += &explicit;
        self.tally.burned += &award.allocation.burn;
        self.coverage.hit("awards");
        self.coverage.hit_if("accruing awards", !explicit.is_zero());
        self.coverage.hit_if(
            "accruing awards to registered streams",
            award
                .allocation
                .portions
                .iter()
                .any(|(id, amount)| *id >= FIRST_GENERATED_STREAM && !amount.is_zero()),
        );
        self.commit(ledger, &award.applied);
    }

    fn explicit_ids(&self) -> Vec<StreamId> {
        self.streams
            .streams
            .iter()
            .filter(|stream| !stream.is_implicit())
            .map(|stream| stream.id)
            .collect()
    }

    fn live_ids(&self) -> Vec<StreamId> {
        self.streams.streams.iter().map(|stream| stream.id).collect()
    }

    /// Include pending registrations so generated writes can depend on them.
    fn addressable_ids(&self) -> Vec<StreamId> {
        let mut ids = self.live_ids();
        ids.extend(
            self.streams
                .pending_writes_queue
                .iter()
                .filter(|write| write.op == PendingWriteOp::RegisterStream)
                .filter_map(|write| write.id),
        );
        ids
    }

    /// Prefer current recipients and unpaid rows; also probe unknown wallets.
    fn claimant(&self, random: &mut Random, id: StreamId) -> Address {
        let mut known = Vec::new();
        if let Some(distribution) = self.streams.explicit(id) {
            known.extend(distribution.shares.iter().map(|row| row.recipient));
            known.extend(distribution.payable.iter().map(|row| row.recipient));
        }
        if let Some(tombstone) = self.streams.tombstones.iter().find(|tombstone| tombstone.id == id)
        {
            known.extend(tombstone.payable.iter().map(|row| row.recipient));
        }
        if random.chance(4) {
            random.wallet()
        } else {
            random.pick(&known).copied().unwrap_or_else(|| random.wallet())
        }
    }

    fn step(&mut self, random: &mut Random) {
        match random.below(16) {
            0..=5 => self.award(random),
            6 => {
                // Null rounds defer due writes.
                self.epoch += random.below(60) as ChainEpoch;
                self.coverage.hit("null gaps");
            }
            7 => {
                // Claim
                let mut ids = self.live_ids();
                // Could claim from a valid or invalid stream and it shouldn't abort.
                ids.extend(self.streams.tombstones.iter().map(|tombstone| tombstone.id));
                ids.push(999_999);
                let id = *random.pick(&ids).unwrap();
                let mut wallets: Vec<Option<Address>> = Vec::new();
                // Wallets could either be valid, unknown or unresolvable and the claim() should
                // still not abort, test all 3 variants.
                for _ in 0..1 + random.below(4) {
                    wallets.push(if random.chance(6) {
                        None
                    } else {
                        Some(self.claimant(random, id))
                    });
                }
                if let Some(paid) = self.method(|ledger, _| ledger.claim(id, &wallets)) {
                    let total: TokenAmount = paid.iter().sum();
                    self.tally.claimed += &total;
                    self.coverage.hit_if("paying claims", !total.is_zero());
                    self.coverage.hit_if(
                        "paying claims from registered streams",
                        id >= FIRST_GENERATED_STREAM && !total.is_zero(),
                    );
                }
            }
            8 => {
                // SetShares
                let Some(id) = random.pick(&self.explicit_ids()).copied() else { return };
                let map = random.share_map();
                let sentinel = map.iter().any(|row| row.recipient == BURNT_FUNDS_ACTOR_ADDR);
                if let Some(dust) = self.method(|ledger, _| ledger.set_shares(id, map)) {
                    self.tally.dust += &dust;
                    self.coverage.hit("share installs");
                    self.coverage.hit_if("sentinel share maps", sentinel);
                }
            }
            9 => {
                // ReplaceAddress
                let Some(id) = random.pick(&self.explicit_ids()).copied() else { return };
                let old = random.wallet();
                let new = if random.chance(5) { BURNT_FUNDS_ACTOR_ADDR } else { random.wallet() };
                if let Some(dust) = self.method(|ledger, _| ledger.replace_address(id, old, new)) {
                    self.tally.dust += &dust;
                    self.coverage.hit("address swaps");
                }
            }
            10 => {
                // RegisterStream
                let id = self.next_id;
                let weight = random.weight_record(self.epoch);
                let implicit = random.chance(6);
                let distribution = (!implicit).then(|| DistributionInit {
                    writer: Address::new_id(FIRST_WRITER + random.below(8)),
                    shares: admit_shares(random.share_map()).unwrap(),
                });
                let activation = self.epoch + TIMELOCK + random.below(200) as ChainEpoch;
                let call = QueuedCall::Register { id, weight, distribution, activation };
                if self
                    .method(|ledger, epoch| ledger.admit(call, epoch, TIMELOCK).map(|_| ()))
                    .is_some()
                {
                    self.next_id += 1;
                    self.coverage.hit("registrations");
                    self.coverage.hit_if("implicit registrations", implicit);
                }
                // Else this wasn't a valid write: the weights may fail projection, this may be a
                // second implicit stream registration, this may encounter a full queue or stream
                // table or this may conflict with an existing write.
            }
            11 => {
                // RemoveStream
                let Some(id) = random.pick(&self.addressable_ids()).copied() else { return };
                let call = QueuedCall::Remove { id };
                if self
                    .method(|ledger, epoch| ledger.admit(call, epoch, TIMELOCK).map(|_| ()))
                    .is_some()
                {
                    self.coverage.hit("removals");
                }
            }
            12 => {
                // SetDistribution
                let Some(id) = random.pick(&self.addressable_ids()).copied() else { return };
                let writer = Address::new_id(FIRST_WRITER + random.below(8));
                let call = QueuedCall::SetDistribution { id, writer };
                if self
                    .method(|ledger, epoch| ledger.admit(call, epoch, TIMELOCK).map(|_| ()))
                    .is_some()
                {
                    self.coverage.hit("writer changes");
                }
            }
            13 | 14 => {
                // {Set,Step}WeightRecords
                let mut updates = Vec::new();
                for id in self.addressable_ids() {
                    if random.chance(2) {
                        updates.push(WeightRecordUpdate {
                            id,
                            weight: random.weight_record(self.epoch),
                        });
                    }
                }
                let op = if random.chance(3) {
                    PendingWriteOp::StepWeightRecords
                } else {
                    PendingWriteOp::SetWeightRecords
                };
                let call = QueuedCall::Weights { op, updates }.canonical();
                if self
                    .method(|ledger, epoch| ledger.admit(call, epoch, TIMELOCK).map(|_| ()))
                    .is_some()
                {
                    self.coverage.hit("weight writes");
                }
            }
            15 => {
                // Cancel
                let keys: Vec<(Option<StreamId>, PendingWriteOp)> = self
                    .streams
                    .pending_writes_queue
                    .iter()
                    .filter(|write| write.op != PendingWriteOp::StepWeightRecords)
                    .map(|write| (write.id, write.op))
                    .collect();
                let Some((id, op)) = random.pick(&keys).copied() else { return };
                let key = WriteKey::for_cancel(id, op).unwrap();
                let cancelled = self.method(|ledger, _| Ok(ledger.cancel(key))).flatten();
                self.coverage.hit_if("cancellations", cancelled.is_some());
            }
            _ => unreachable!(),
        }
        self.epoch += 1;
    }

    fn assert_properties(&mut self) {
        let ledger = self.load();
        schedule(&self.streams.streams, self.epoch)
            .unwrap_or_else(|error| panic!("schedule broke at epoch {}: {error}", self.epoch));
        assert_eq!(self.tally.held(), ledger.liability(), "held funds drift from liability");
        assert_eq!(
            self.tally.minted,
            &self.tally.miner + &self.tally.explicit + &self.tally.burned,
            "minted value is not conserved"
        );
        self.coverage.hit_if("tombstones", !self.streams.tombstones.is_empty());
    }
}

#[test]
fn a_random_walk_never_stops_loading_or_awarding() {
    let mut total = Coverage::default();
    for seed in 0..SEEDS {
        let seed = 0x5eed_0118_0000_0000 | seed;
        let mut random = Random(seed);
        let mut walk = Walk::new();
        let mut step = 0;
        let result = catch_unwind(AssertUnwindSafe(|| {
            for current in 0..STEPS {
                step = current;
                walk.step(&mut random);
                walk.assert_properties();
            }
            // Advance beyond all generated activation epochs before draining the queue.
            step = STEPS;
            walk.epoch += 1_000;
            walk.award(&mut random);
            walk.assert_properties();
            assert!(walk.streams.pending_writes_queue.is_empty());
        }));
        if let Err(error) = result {
            eprintln!("seed={seed:#018x}, step={step}, epoch={}", walk.epoch);
            resume_unwind(error);
        }

        total.merge(&walk.coverage);
    }

    for name in REQUIRED_COVERAGE {
        let count = total.0.get(name).copied().unwrap_or_default();
        assert!(count > 0, "no coverage of {name}: {total:?}");
    }
}

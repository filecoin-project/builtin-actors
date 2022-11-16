#![allow(dead_code)]
use fil_actor_miner::MinerInfo;
use fil_actor_miner::SectorPreCommitOnChainInfo;
use fil_actor_miner::VestSpec;
use fil_actor_miner::VestingFunds;
use fil_actor_miner::{BitFieldQueue, CollisionPolicy, SectorOnChainInfo, State};
use fil_actors_runtime::{runtime::Policy, ActorError};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::BytesDe;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_hamt::Error as HamtError;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{SectorNumber, SectorSize};
use fvm_shared::{clock::ChainEpoch, clock::QuantSpec, sector::RegisteredPoStProof};
use multihash::Code::Blake2b256;

use fil_actors_runtime::test_utils::*;

pub struct StateHarness {
    pub st: State,
    pub store: MemoryBlockstore,
}

impl StateHarness {
    #[allow(dead_code)]
    pub fn new(period_boundary: ChainEpoch) -> Self {
        Self::new_with_policy(&Policy::default(), period_boundary)
    }

    #[allow(dead_code)]
    pub fn new_with_policy(policy: &Policy, period_boundary: ChainEpoch) -> Self {
        // store init
        let store = MemoryBlockstore::default();
        // state field init
        let owner = 1;
        let worker = 2;

        let test_window_post_proof_type = RegisteredPoStProof::StackedDRGWindow2KiBV1;

        let info = MinerInfo::new(
            owner,
            worker,
            vec![],
            "peer".as_bytes().to_vec(),
            vec![BytesDe("foobar".as_bytes().to_vec()), BytesDe("imafilminer".as_bytes().to_vec())],
            test_window_post_proof_type,
        )
        .unwrap();
        let info_cid = store.put_cbor(&info, Blake2b256).unwrap();

        let st = State::new(policy, &store, info_cid, period_boundary, 0).unwrap();
        StateHarness { st, store }
    }

    pub fn new_runtime(&self) -> MockRuntime {
        MockRuntime::default()
    }

    #[allow(dead_code)]
    pub fn put_precommitted_sectors(
        &mut self,
        precommits: Vec<SectorPreCommitOnChainInfo>,
    ) -> anyhow::Result<()> {
        self.st.put_precommitted_sectors(&self.store, precommits)
    }

    #[allow(dead_code)]
    pub fn delete_precommitted_sectors(
        &mut self,
        sector_nums: &[SectorNumber],
    ) -> Result<(), HamtError> {
        self.st.delete_precommitted_sectors(&self.store, sector_nums)
    }

    #[allow(dead_code)]
    pub fn get_precommit(&self, sector_number: SectorNumber) -> SectorPreCommitOnChainInfo {
        self.st.get_precommitted_sector(&self.store, sector_number).unwrap().unwrap()
    }

    #[allow(dead_code)]
    pub fn has_precommit(&self, sector_number: SectorNumber) -> bool {
        self.st.get_precommitted_sector(&self.store, sector_number).unwrap().is_some()
    }

    #[allow(dead_code)]
    pub fn load_pre_commit_clean_ups<'db>(
        &'db self,
        policy: &Policy,
    ) -> BitFieldQueue<'db, MemoryBlockstore> {
        let quant = self.st.quant_spec_every_deadline(policy);
        let queue =
            BitFieldQueue::new(&self.store, &self.st.pre_committed_sectors_cleanup, quant).unwrap();
        queue
    }

    #[allow(dead_code)]
    pub fn add_pre_commit_clean_ups(
        &mut self,
        policy: &Policy,
        cleanup_events: Vec<(ChainEpoch, u64)>,
    ) -> anyhow::Result<()> {
        self.st.add_pre_commit_clean_ups(policy, &self.store, cleanup_events)
    }

    #[allow(dead_code)]
    pub fn add_locked_funds(
        &mut self,
        current_epoch: ChainEpoch,
        vesting_sum: &TokenAmount,
        spec: &VestSpec,
    ) -> anyhow::Result<TokenAmount> {
        self.st.add_locked_funds(&self.store, current_epoch, vesting_sum, spec)
    }

    #[allow(dead_code)]
    pub fn unlock_vested_funds(
        &mut self,
        current_epoch: ChainEpoch,
    ) -> anyhow::Result<TokenAmount> {
        self.st.unlock_vested_funds(&self.store, current_epoch)
    }

    #[allow(dead_code)]
    pub fn unlock_unvested_funds(
        &mut self,
        current_epoch: ChainEpoch,
        target: &TokenAmount,
    ) -> anyhow::Result<TokenAmount> {
        self.st.unlock_unvested_funds(&self.store, current_epoch, target)
    }

    pub fn has_sector_number(&self, sector_no: SectorNumber) -> bool {
        self.st.has_sector_number(&self.store, sector_no).unwrap()
    }

    pub fn put_sector(&mut self, sector: &SectorOnChainInfo) {
        self.st.put_sectors(&self.store, vec![sector.clone()]).unwrap();
    }

    pub fn get_sector(&self, sector_number: SectorNumber) -> SectorOnChainInfo {
        self.st.get_sector(&self.store, sector_number).unwrap().unwrap()
    }

    // makes a bit field from the passed sector numbers
    pub fn delete_sectors(&mut self, sector_numbers: Vec<u64>) {
        let mut bf = BitField::new();
        for b in sector_numbers.iter() {
            bf.set(*b);
        }
        self.st.delete_sectors(&self.store, &bf).unwrap();
    }

    #[allow(dead_code)]
    pub fn vesting_funds_store_empty(&self) -> bool {
        let vesting = self.store.get_cbor::<VestingFunds>(&self.st.vesting_funds).unwrap().unwrap();
        vesting.funds.is_empty()
    }

    pub fn assign_sectors_to_deadlines(
        &mut self,
        policy: &Policy,
        epoch: ChainEpoch,
        sectors: Vec<SectorOnChainInfo>,
        partition_size: u64,
        sector_size: SectorSize,
    ) {
        self.st
            .assign_sectors_to_deadlines(
                policy,
                &self.store,
                epoch,
                sectors,
                partition_size,
                sector_size,
            )
            .unwrap();
    }

    #[allow(dead_code)]
    pub fn quant_spec_every_deadline(&self, policy: &Policy) -> QuantSpec {
        self.st.quant_spec_every_deadline(policy)
    }

    #[allow(dead_code)]
    pub fn quant_spec_for_deadline(&self, policy: &Policy, deadline_idx: u64) -> QuantSpec {
        self.st.quant_spec_for_deadline(policy, deadline_idx)
    }

    #[allow(dead_code)]
    pub fn allocate_sector_numbers(&mut self, sector_numbers: &[u64]) -> Result<(), ActorError> {
        self.st.allocate_sector_numbers(
            &self.store,
            &BitField::try_from_bits(sector_numbers.iter().copied()).unwrap(),
            CollisionPolicy::DenyCollisions,
        )
    }

    #[allow(dead_code)]
    pub fn mask_sector_numbers(&mut self, ns: &BitField) -> Result<(), ActorError> {
        self.st.allocate_sector_numbers(&self.store, ns, CollisionPolicy::AllowCollisions)
    }

    #[allow(dead_code)]
    pub fn expect_allocated_sector_numbers(&mut self, expected: &BitField) {
        let b: BitField = self.store.get_cbor(&self.st.allocated_sectors).unwrap().unwrap();
        assert_eq!(&b, expected);
    }
}

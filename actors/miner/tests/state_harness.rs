#![allow(dead_code)]
use fil_actor_miner::MinerInfo;
use fil_actor_miner::SectorPreCommitOnChainInfo;
use fil_actor_miner::State;
use fil_actor_miner::VestingFunds;
use fil_actors_runtime::runtime::Policy;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::BytesDe;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_hamt::Error as HamtError;
use fvm_shared::address::Address;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{clock::ChainEpoch, sector::RegisteredPoStProof};
use multihash::Code::Blake2b256;

pub struct StateHarness {
    pub st: State,
    pub store: MemoryBlockstore,
}

impl StateHarness {
    pub fn new(period_boundary: ChainEpoch) -> Self {
        Self::new_with_policy(&Policy::default(), period_boundary)
    }

    pub fn new_with_policy(policy: &Policy, period_boundary: ChainEpoch) -> Self {
        // store init
        let store = MemoryBlockstore::default();
        // state field init
        let owner = Address::new_id(1);
        let worker = Address::new_id(2);

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
}

impl StateHarness {
    pub fn put_precommitted_sectors(
        &mut self,
        precommits: Vec<SectorPreCommitOnChainInfo>,
    ) -> anyhow::Result<()> {
        self.st.put_precommitted_sectors(&self.store, precommits)
    }

    pub fn delete_precommitted_sectors(
        &mut self,
        sector_nums: &[SectorNumber],
    ) -> Result<(), HamtError> {
        self.st.delete_precommitted_sectors(&self.store, sector_nums)
    }

    pub fn get_precommit(&self, sector_number: SectorNumber) -> SectorPreCommitOnChainInfo {
        self.st.get_precommitted_sector(&self.store, sector_number).unwrap().unwrap()
    }

    pub fn has_precommit(&self, sector_number: SectorNumber) -> bool {
        self.st.get_precommitted_sector(&self.store, sector_number).unwrap().is_some()
    }
}

impl StateHarness {
    pub fn vesting_funds_store_empty(&self) -> bool {
        let vesting = self.store.get_cbor::<VestingFunds>(&self.st.vesting_funds).unwrap().unwrap();
        vesting.funds.is_empty()
    }
}

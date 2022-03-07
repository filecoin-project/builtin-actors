use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::INIT_ACTOR_ADDR;

use fil_actor_account::Method as AccountMethod;
use fil_actor_miner::{Actor, Method, MinerConstructorParams as ConstructorParams};

use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::RawBytes;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{
    RegisteredPoStProof, RegisteredSealProof, SectorNumber, SectorSize, StoragePower,
};
use fvm_shared::smooth::FilterEstimate;

use rand::prelude::*;

pub fn new_bls_addr(s: u8) -> Address {
    let seed = [s; 32];
    let mut rng: StdRng = SeedableRng::from_seed(seed);
    let mut key = [0u8; 48];
    rng.fill_bytes(&mut key);
    Address::new_bls(&key).unwrap()
}

#[allow(dead_code)]
pub struct ActorHarness {
    pub receiver: Address,
    pub owner: Address,
    pub worker: Address,
    pub worker_key: Address,

    pub control_addrs: Vec<Address>,

    pub seal_proof_type: RegisteredSealProof,
    pub window_post_proof_type: RegisteredPoStProof,
    pub sector_size: SectorSize,
    pub partition_size: u64,
    pub period_offset: ChainEpoch,
    pub next_sector_no: SectorNumber,

    pub network_pledge: TokenAmount,
    pub network_raw_power: StoragePower,
    pub network_qa_power: StoragePower,
    pub baseline_power: StoragePower,

    pub epoch_reward_smooth: FilterEstimate,
    pub epoch_qa_power_smooth: FilterEstimate,
}

#[allow(dead_code)]
impl ActorHarness {
    pub fn new(proving_period_offset: ChainEpoch) -> ActorHarness {
        let owner = Address::new_id(100);
        let worker = Address::new_id(101);
        let control_addrs = vec![
            Address::new_id(999),
            Address::new_id(998),
            Address::new_id(997),
        ];
        let worker_key = new_bls_addr(0);
        let receiver = Address::new_id(1000);
        let rwd = TokenAmount::from(10_000_000_000_000_000_000i128);
        let pwr = StoragePower::from(1i128 << 50);
        let proof_type = RegisteredSealProof::StackedDRG32GiBV1;

        ActorHarness {
            receiver: receiver,
            owner: owner,
            worker: worker,
            worker_key: worker_key,
            control_addrs: control_addrs,

            seal_proof_type: proof_type,
            window_post_proof_type: proof_type.registered_window_post_proof().unwrap(),
            sector_size: proof_type.sector_size().unwrap(),
            partition_size: proof_type.window_post_partitions_sector().unwrap(),

            period_offset: proving_period_offset,
            next_sector_no: 0,

            network_pledge: rwd.clone() * TokenAmount::from(1000),
            network_raw_power: pwr.clone(),
            network_qa_power: pwr.clone(),
            baseline_power: pwr.clone(),

            epoch_reward_smooth: FilterEstimate::new(rwd.clone(), BigInt::from(0)),
            epoch_qa_power_smooth: FilterEstimate::new(pwr.clone(), BigInt::from(0)),
        }
    }

    pub fn construct_and_verify(self: &Self, rt: &mut MockRuntime) {
        let params = ConstructorParams {
            owner: self.owner.clone(),
            worker: self.worker.clone(),
            control_addresses: self.control_addrs.clone(),
            window_post_proof_type: self.window_post_proof_type,
            peer_id: vec![0],
            multi_addresses: vec![],
        };

        rt.actor_code_cids
            .insert(self.owner, *ACCOUNT_ACTOR_CODE_ID);
        rt.actor_code_cids
            .insert(self.worker, *ACCOUNT_ACTOR_CODE_ID);
        for a in self.control_addrs.iter() {
            rt.actor_code_cids.insert(*a, *ACCOUNT_ACTOR_CODE_ID);
        }

        rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
        rt.expect_send(
            self.worker,
            AccountMethod::PubkeyAddress as u64,
            RawBytes::default(),
            TokenAmount::from(0),
            RawBytes::serialize(self.worker_key).unwrap(),
            ExitCode::Ok,
        );
        rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);

        let result = rt
            .call::<Actor>(
                Method::Constructor as u64,
                &RawBytes::serialize(params).unwrap(),
            )
            .unwrap();
        assert_eq!(result.bytes().len(), 0);
        rt.verify();
    }
}

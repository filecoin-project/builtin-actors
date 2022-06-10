use fil_actor_miner::{new_deadline_info_from_offset_and_epoch, DeadlineInfo, State as MinerState};
use fil_actor_power::{CreateMinerParams, CreateMinerReturn, Method as PowerMethod};
use fil_actors_runtime::{runtime::Policy, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_encoding::BytesDe;
use fvm_shared::{address::Address, econ::TokenAmount, sector::RegisteredPoStProof};
use test_vm::VM;

#[allow(dead_code)]
pub fn miner_dline_info(v: &VM, m: Address) -> DeadlineInfo {
    let st = v.get_state::<MinerState>(m).unwrap();
    new_deadline_info_from_offset_and_epoch(
        &Policy::default(),
        st.proving_period_start,
        v.get_epoch(),
    )
}

#[allow(dead_code)]
pub fn create_miner(
    v: &mut VM,
    owner: Address,
    worker: Address,
    post_proof_type: RegisteredPoStProof,
    balance: TokenAmount,
) -> (Address, Address) {
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];
    let peer_id = "miner".as_bytes().to_vec();
    let params = CreateMinerParams {
        owner,
        worker,
        window_post_proof_type: post_proof_type,
        peer: peer_id,
        multiaddrs,
    };

    let res: CreateMinerReturn = v
        .apply_message(
            owner,
            *STORAGE_POWER_ACTOR_ADDR,
            balance,
            PowerMethod::CreateMiner as u64,
            params,
        )
        .unwrap()
        .ret
        .deserialize()
        .unwrap();
    (res.id_address, res.robust_address)
}

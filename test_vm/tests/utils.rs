use fil_actor_miner::{new_deadline_info_from_offset_and_epoch, DeadlineInfo, State as MinerState};
use fil_actors_runtime::runtime::Policy;
use fvm_shared::address::Address;
use test_vm::VM;

pub fn miner_dline_info(v: &VM, m: Address) -> DeadlineInfo {
    let st = v.get_state::<MinerState>(m).unwrap();
    new_deadline_info_from_offset_and_epoch(
        &Policy::default(),
        st.proving_period_start,
        v.get_epoch(),
    )
}

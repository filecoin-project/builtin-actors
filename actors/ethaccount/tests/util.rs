use fil_actor_ethaccount::{EthAccountActor, Method};
use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::MethodNum;

pub const EOA: Address = Address::new_id(1000);

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: EOA,
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn setup() -> MockRuntime {
    let mut rt = new_runtime();
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.add_delegated_address(
        EOA,
        Address::new_delegated(
            EAM_ACTOR_ID,
            &hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000"),
        )
        .unwrap(),
    );
    rt.call::<EthAccountActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();
    rt
}

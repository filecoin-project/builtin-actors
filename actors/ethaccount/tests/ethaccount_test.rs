mod util;

use crate::util::*;
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

use fil_actor_ethaccount::{EthAccountActor, Method};
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

use fil_actors_runtime::test_utils::{
    expect_abort_contains_message, ACCOUNT_ACTOR_CODE_ID, SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;

#[test]
fn no_delegated_cant_deploy() {
    let mut rt = new_runtime();
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "receiver must have a predictable address",
        rt.call::<EthAccountActor>(Method::Constructor as MethodNum, None),
    );
    rt.verify();
}

#[test]
fn token_receiver() {
    let mut rt = setup();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(1234));
    rt.expect_validate_caller_any();
    let ret = rt
        .call::<EthAccountActor>(
            frc42_dispatch::method_hash!("Receive"),
            IpldBlock::serialize_cbor(&UniversalReceiverParams {
                type_: 0,
                payload: RawBytes::new(vec![1, 2, 3]),
            })
            .unwrap(),
        )
        .unwrap();
    assert!(ret.is_none());
}

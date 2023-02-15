mod util;

use crate::util::*;
use fil_actor_ethaccount::types::AuthenticateMessageParams;
use fil_actor_ethaccount::{EthAccountActor, Method};
use fil_actors_runtime::test_utils::expect_abort_contains_message;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

#[test]
fn must_have_params() {
    let mut rt = setup();
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "method expects arguments",
        rt.call::<EthAccountActor>(Method::AuthenticateMessageExported as MethodNum, None),
    );
    rt.verify();
}

#[test]
fn signature_bad_length_fails() {
    let mut rt = setup();
    rt.expect_validate_caller_any();
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "invalid signature length",
        rt.call::<EthAccountActor>(
            Method::AuthenticateMessageExported as MethodNum,
            IpldBlock::serialize_cbor(&AuthenticateMessageParams {
                signature: vec![0xde, 0xad, 0xbe, 0xef],
                message: vec![0xfa, 0xce],
            })
            .unwrap(),
        ),
    );
    rt.verify();
}

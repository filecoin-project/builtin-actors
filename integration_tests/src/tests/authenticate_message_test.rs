use export_macro::vm_test;
use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::Method::AuthenticateMessageExported;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;

use crate::util::{create_accounts, generate_deal_proposal};
use vm_api::util::{apply_code, apply_ok};
use vm_api::VM;

/// Using a deal proposal as a serialized message, we confirm that:
/// - calls to Account::authenticate_message with valid signatures succeed
/// - calls to Account::authenticate_message with invalid signatures fail
#[vm_test]
pub fn account_authenticate_message_test(v: &dyn VM) {
    let addr = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];

    let proposal =
        generate_deal_proposal(&addr, &addr, &TokenAmount::zero(), &TokenAmount::zero(), 0, 0);
    let proposal_ser =
        RawBytes::serialize(proposal).expect("failed to marshal deal proposal").to_vec();

    // With a good sig, message succeeds
    let authenticate_message_params = AuthenticateMessageParams {
        signature: proposal_ser.clone(),
        message: proposal_ser.clone(),
    };
    apply_ok(
        v,
        &addr,
        &addr,
        &TokenAmount::zero(),
        AuthenticateMessageExported as u64,
        Some(authenticate_message_params),
    );

    // Bad, bad sig! message fails
    let authenticate_message_params =
        AuthenticateMessageParams { signature: vec![], message: proposal_ser };
    apply_code(
        v,
        &addr,
        &addr,
        &TokenAmount::zero(),
        AuthenticateMessageExported as u64,
        Some(authenticate_message_params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
}

use export_macro::vm_test;
use fil_actor_datacap::{Method as DataCapMethod, MintParams};
use fil_actors_runtime::{DATACAP_TOKEN_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use vm_api::VM;
use vm_api::util::{apply_code, apply_ok};

use crate::util::create_accounts;

/// FIP-0118: Mint is now deprecated and always returns USR_FORBIDDEN, regardless of
/// caller - including a message purporting to be from the verifreg actor (the only
/// address that was ever allowed to call it). Since Mint can no longer succeed, the
/// downstream flow this test used to exercise (transferring newly minted datacap into
/// verifreg to create an allocation) is no longer reachable via any live path; that
/// UniversalReceiverHook-disabled behavior is covered directly in verifreg's own unit
/// tests (see `datacap` module in verifreg_actor_test.rs), which inject balances
/// directly into state rather than relying on Mint.
#[vm_test]
pub fn datacap_mint_disabled_test(v: &dyn VM) {
    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let (client, operator) = (addrs[0], addrs[1]);

    let mint_params =
        MintParams { to: client, amount: TokenAmount::from_whole(1), operators: vec![] };

    // cannot mint from an arbitrary account
    apply_code(
        v,
        &operator,
        &DATACAP_TOKEN_ACTOR_ADDR,
        &TokenAmount::zero(),
        DataCapMethod::MintExported as u64,
        Some(mint_params.clone()),
        ExitCode::USR_FORBIDDEN,
    );

    // cannot mint even from a message purporting to be sent by the verifreg actor,
    // the only address that was ever allowed to call Mint
    apply_code(
        v,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &DATACAP_TOKEN_ACTOR_ADDR,
        &TokenAmount::zero(),
        DataCapMethod::MintExported as u64,
        Some(mint_params),
        ExitCode::USR_FORBIDDEN,
    );
}

#[vm_test]
pub fn call_name_symbol_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let sender = addrs[0];

    let mut ret: String = apply_ok(
        v,
        &sender,
        &DATACAP_TOKEN_ACTOR_ADDR,
        &TokenAmount::zero(),
        DataCapMethod::NameExported as u64,
        None::<RawBytes>,
    )
    .deserialize()
    .unwrap();
    assert_eq!("DataCap", ret, "expected name DataCap, got {}", ret);

    ret = apply_ok(
        v,
        &sender,
        &DATACAP_TOKEN_ACTOR_ADDR,
        &TokenAmount::zero(),
        DataCapMethod::SymbolExported as u64,
        None::<RawBytes>,
    )
    .deserialize()
    .unwrap();
    assert_eq!("DCAP", ret, "expected name DataCap, got {}", ret);
}

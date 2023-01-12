#[cfg(feature = "fil-actor")]
#[no_mangle]
pub extern "C" fn invoke(_: u32) -> u32 {
    fvm_sdk::vm::abort(
        fvm_shared::error::ExitCode::USR_UNHANDLED_MESSAGE.value(),
        Some("there is no contract deployed at this address; placeholder actors may only receive value transfers with method 0"),
    )
}

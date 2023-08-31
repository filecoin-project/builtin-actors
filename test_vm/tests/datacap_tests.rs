use fil_actors_integration_tests::tests::{call_name_symbol_test, datacap_transfer_test};
use test_vm::new_test_vm;

/* Mint a token for client and transfer it to a receiver, exercising error cases */
#[test]
fn datacap_transfer() {
    let v = new_test_vm();
    datacap_transfer_test(&*v);
}

/* Call name & symbol */
#[test]
fn call_name_symbol() {
    let v = new_test_vm();
    call_name_symbol_test(&*v);
}

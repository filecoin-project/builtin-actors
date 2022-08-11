use fvm_shared::address::Address;
use lazy_static::lazy_static;

mod harness;

lazy_static! {
    static ref VERIFIER: Address = Address::new_id(201);
    static ref VERIFIER2: Address = Address::new_id(202);
    static ref CLIENT: Address = Address::new_id(301);
    static ref CLIENT2: Address = Address::new_id(302);
    static ref CLIENT3: Address = Address::new_id(303);
    static ref CLIENT4: Address = Address::new_id(304);
}

mod construction {
    use harness::*;

    use crate::*;

    #[test]
    fn construct_with_verified() {
        let mut rt = new_runtime();
        let h = Harness { registry: *REGISTRY_ADDR };
        h.construct_and_verify(&mut rt, &h.registry);
        h.check_state(&rt);
    }
}

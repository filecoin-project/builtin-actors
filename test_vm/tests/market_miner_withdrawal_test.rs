#[cfg(test)]
mod market_tests {
    use fil_actors_integration_tests::tests::market_tests::*;
    use test_vm::new_test_vm;

    #[test]
    fn withdraw_all_funds() {
        let v = new_test_vm();
        withdraw_all_funds_test(&*v);
    }

    #[test]
    fn withdraw_as_much_as_possible() {
        let v = new_test_vm();

        withdraw_as_much_as_possible_test(&*v);
    }

    #[test]
    fn withdraw_0() {
        let v = new_test_vm();
        withdraw_0_test(&*v);
    }
}

#[cfg(test)]
mod miner_tests {
    use fil_actors_integration_tests::tests::miner_tests::*;
    use test_vm::new_test_vm;

    #[test]
    fn withdraw_all_funds() {
        let v = new_test_vm();

        withdraw_all_funds_test(&*v);
    }

    #[test]
    fn withdraw_as_much_as_possible() {
        let v = new_test_vm();
        withdraw_as_much_as_possible_test(&*v);
    }

    #[test]
    fn withdraw_0() {
        let v = new_test_vm();
        withdraw_0_test(&*v);
    }

    #[test]
    fn withdraw_from_non_owner_address_fails() {
        let v = new_test_vm();
        withdraw_from_non_owner_address_fails_test(&*v)
    }
}

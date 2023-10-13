#[cfg(test)]
mod market_tests {
    use fil_actors_integration_tests::tests::market_tests::*;
    use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
    use test_vm::TestVM;

    #[test]
    fn withdraw_all_funds() {
        let store = TrackingMemBlockstore::new();
        let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);
        withdraw_all_funds_test(&v);
    }

    #[test]
    fn withdraw_as_much_as_possible() {
        let store = TrackingMemBlockstore::new();
        let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);

        withdraw_as_much_as_possible_test(&v);
    }

    #[test]
    fn withdraw_0() {
        let store = TrackingMemBlockstore::new();
        let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);
        withdraw_0_test(&v);
    }
}

#[cfg(test)]
mod miner_tests {
    use fil_actors_integration_tests::tests::miner_tests::*;
    use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
    use test_vm::TestVM;

    #[test]
    fn withdraw_all_funds() {
        let store = TrackingMemBlockstore::new();
        let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);

        withdraw_all_funds_test(&v);
    }

    #[test]
    fn withdraw_as_much_as_possible() {
        let store = TrackingMemBlockstore::new();
        let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);
        withdraw_as_much_as_possible_test(&v);
    }

    #[test]
    fn withdraw_0() {
        let store = TrackingMemBlockstore::new();
        let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);
        withdraw_0_test(&v);
    }

    #[test]
    fn withdraw_from_non_owner_address_fails() {
        let store = TrackingMemBlockstore::new();
        let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);
        withdraw_from_non_owner_address_fails_test(&v)
    }
}

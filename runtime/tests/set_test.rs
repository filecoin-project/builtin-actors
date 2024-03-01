// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::{Set, DEFAULT_HAMT_CONFIG};

#[test]
fn put() {
    let store = fil_actors_runtime::test_blockstores::MemoryBlockstore::new();
    let mut set = Set::empty(&store, DEFAULT_HAMT_CONFIG, "t");

    let key: Vec<u8> = "test".into();
    assert!(!set.has(&key).unwrap());

    set.put(&key).unwrap();
    assert!(set.has(&key).unwrap());
}

#[test]
fn collect_keys() {
    let store = fil_actors_runtime::test_blockstores::MemoryBlockstore::new();
    let mut set = Set::<_, u64>::empty(&store, DEFAULT_HAMT_CONFIG, "t");

    set.put(&0u64).unwrap();

    assert_eq!(set.collect_keys().unwrap(), [0u64]);

    set.put(&1u64).unwrap();
    set.put(&2u64).unwrap();
    set.put(&3u64).unwrap();

    assert_eq!(set.collect_keys().unwrap().len(), 4);
}

#[test]
fn delete() {
    let store = fil_actors_runtime::test_blockstores::MemoryBlockstore::new();
    let mut set = Set::empty(&store, DEFAULT_HAMT_CONFIG, "t");

    let key = 0u64;

    assert!(!set.has(&key).unwrap());
    set.put(&key).unwrap();
    assert!(set.has(&key).unwrap());
    set.delete(&key).unwrap();
    assert!(!set.has(&key).unwrap());

    // Test delete when doesn't exist doesn't error
    set.delete(&key).unwrap();
}

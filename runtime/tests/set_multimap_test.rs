// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::clock::ChainEpoch;

use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use fil_actors_runtime::{SetMultimap, SetMultimapConfig, DEFAULT_HAMT_CONFIG};

pub const CONFIG: SetMultimapConfig =
    SetMultimapConfig { outer: DEFAULT_HAMT_CONFIG, inner: DEFAULT_HAMT_CONFIG };

#[test]
fn put_remove() {
    let store = MemoryBlockstore::new();
    let mut smm = SetMultimap::<_, ChainEpoch, u64>::empty(&store, CONFIG, "t");

    let epoch: ChainEpoch = 100;
    assert!(smm.get(&epoch).unwrap().is_none());

    smm.put(&epoch, 8).unwrap();
    smm.put(&epoch, 2).unwrap();
    smm.remove(&epoch, 2).unwrap();

    let set = smm.get(&epoch).unwrap().unwrap();
    assert!(set.has(&8).unwrap());
    assert!(!set.has(&2).unwrap());

    smm.remove_all(&epoch).unwrap();
    assert!(smm.get(&epoch).unwrap().is_none());
}

#[test]
fn for_each() {
    let store = MemoryBlockstore::new();
    let mut smm = SetMultimap::<_, ChainEpoch, u64>::empty(&store, CONFIG, "t");

    let epoch: ChainEpoch = 100;
    assert!(smm.get(&epoch).unwrap().is_none());

    smm.put(&epoch, 8).unwrap();
    smm.put(&epoch, 3).unwrap();
    smm.put(&epoch, 2).unwrap();
    smm.put(&epoch, 8).unwrap();

    let mut vals: Vec<u64> = Vec::new();
    smm.for_each_in(&epoch, |i| {
        vals.push(i);
        Ok(())
    })
    .unwrap();

    assert_eq!(vals.len(), 3);
}

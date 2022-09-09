use std::ops::Range;

use fil_actor_miner::DeadlineSectorMap;
use fil_actor_miner::PartitionSectorMap;
use fil_actors_runtime::runtime::Policy;
use fvm_ipld_bitfield::iter::Ranges;
use fvm_ipld_bitfield::BitField;
use itertools::Itertools;

mod util;
use util::*;

fn sector_from_deadline_sector_map(
    map: &mut DeadlineSectorMap,
    deadline_index: u64,
    partition_index: u64,
) -> &mut BitField {
    map.iter()
        .find(|(index, _)| index == &deadline_index)
        .unwrap()
        .1
        .iter()
        .find(|(index, _)| index == &partition_index)
        .unwrap()
        .1
}

fn sector_from_partition_sector_map(
    map: &mut PartitionSectorMap,
    partition_index: u64,
) -> &mut BitField {
    map.iter().find(|(index, _)| index == &partition_index).unwrap().1
}

fn create_deadline_sector_map(
    policy: &Policy,
    deadline_count: u64,
    partition_count: u64,
) -> DeadlineSectorMap {
    let mut deadline_sector_map = DeadlineSectorMap::new();
    (0..deadline_count).for_each(|deadline_index| {
        (0..partition_count).for_each(|partition_index| {
            deadline_sector_map
                .add(
                    policy,
                    deadline_index,
                    partition_index,
                    &make_bitfield(&[deadline_index * partition_count + partition_index]),
                )
                .unwrap();
        })
    });

    deadline_sector_map
}

fn create_bitfield_sequence(start: u64, end: u64) -> BitField {
    let ranges = vec![Range { start, end }];
    let ranges = Ranges::new(ranges.iter().cloned());
    BitField::from_ranges(ranges)
}

#[test]
fn deadline_sector_map() {
    let policy = Policy::default();

    let deadline_count = 10;
    let partition_count = 5;

    let mut deadline_sector_map =
        create_deadline_sector_map(&policy, deadline_count, partition_count);
    // create another one for verifications, iter needs nasty mut which makes rustc mean
    let mut map_copy = create_deadline_sector_map(&policy, deadline_count, partition_count);

    for (deadline_index, partitions_map) in deadline_sector_map.iter() {
        assert!(map_copy
            .iter()
            .find(|(index, _)| index == &deadline_index)
            .unwrap()
            .1
            .partitions()
            .eq(partitions_map.partitions()));
        for (partition_index, sector) in partitions_map.iter() {
            let validated = sector;
            assert_eq!(
                sector_from_deadline_sector_map(&mut map_copy, deadline_index, partition_index),
                validated
            );
            assert_bitfield_equals(
                validated,
                &[deadline_index * partition_count + partition_index],
            );
        }
    }

    // check all counts
    let (partitions, sectors) = deadline_sector_map.count().unwrap();
    assert_eq!(partitions, partition_count * deadline_count);
    assert_eq!(sectors, partition_count * deadline_count);

    assert!(deadline_sector_map.check(1, 1).is_err());
    assert!(deadline_sector_map.check(100, 1).is_err());
    assert!(deadline_sector_map.check(1, 100).is_err());
    assert!(deadline_sector_map
        .check(partition_count * deadline_count, partition_count * deadline_count)
        .is_ok());

    // merge a sector in
    deadline_sector_map.add_values(&policy, 0, 0, &[1000]).unwrap();
    assert_bitfield_equals(
        sector_from_deadline_sector_map(&mut deadline_sector_map, 0, 0),
        &[0, 1000],
    );
    assert!(deadline_sector_map
        .check(partition_count * deadline_count, partition_count * deadline_count)
        .is_err());
    assert!(deadline_sector_map
        .check(partition_count * deadline_count, partition_count * deadline_count + 1)
        .is_ok());
}

#[test]
fn deadline_sector_map_values() {
    let policy = Policy::default();
    let mut deadline_sector_map = DeadlineSectorMap::new();

    deadline_sector_map.add_values(&policy, 0, 1, &[0, 1, 2, 3]).unwrap();
    assert_bitfield_equals(
        sector_from_deadline_sector_map(&mut deadline_sector_map, 0, 1),
        &[0, 1, 2, 3],
    );
}

#[test]
fn partition_sector_map_values() {
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add_values(0, vec![0, 1, 2, 3]).unwrap();
    assert_bitfield_equals(
        sector_from_partition_sector_map(&mut partition_sector_map, 0),
        &[0, 1, 2, 3],
    );
}

#[test]
fn deadline_sector_map_overflow() {
    let policy = Policy::default();
    let deadline_count = 10;

    let mut deadline_sector_map = DeadlineSectorMap::new();
    (0..deadline_count).for_each(|deadline_index| {
        deadline_sector_map
            .add(&policy, deadline_index, 0, create_bitfield_sequence(0, u64::MAX))
            .unwrap();
    });

    assert!(deadline_sector_map.iter().find(|&(index, _)| index == 0).unwrap().1.count().is_ok());
    assert!(deadline_sector_map.count().is_err());
}

#[test]
fn partition_sector_map_overflow() {
    let mut partition_sector_map = PartitionSectorMap::default();
    let partition_count = 2;

    (0..partition_count).for_each(|partition_index| {
        partition_sector_map.add(partition_index, create_bitfield_sequence(0, u64::MAX)).unwrap();
    });

    assert!(partition_sector_map.count().is_err());
}

#[test]
fn deadline_sector_map_empty() {
    let mut deadline_sector_map = DeadlineSectorMap::new();
    let (partitions, sectors) = deadline_sector_map.count().unwrap();
    assert_eq!(0, partitions);
    assert_eq!(0, sectors);

    deadline_sector_map.iter().for_each(|_| {
        panic!("should not iterate over an empty map");
    });

    assert_eq!(0, deadline_sector_map.deadlines().count());
}

#[test]
fn partition_sector_map_empty() {
    let mut partition_sector_map = PartitionSectorMap::default();

    let (partitions, sectors) = partition_sector_map.count().unwrap();
    assert_eq!(0, partitions);
    assert_eq!(0, sectors);

    partition_sector_map.iter().for_each(|_| {
        panic!("should not iterate over an empty map");
    });

    assert_eq!(0, partition_sector_map.partitions().count());
}

#[test]
fn deadline_sector_map_sorted() {
    let policy = Policy::default();
    let mut deadline_sector_map = DeadlineSectorMap::new();
    (1..48).rev().for_each(|i| {
        deadline_sector_map.add_values(&policy, i, 0, &[0]).unwrap();
    });

    assert!(deadline_sector_map.deadlines().sorted().eq(deadline_sector_map.deadlines()));
}

#[test]
fn partition_sector_map_sorted() {
    let mut partition_sector_map = PartitionSectorMap::default();
    (1..=100).rev().for_each(|i| {
        partition_sector_map.add_values(i, vec![0]).unwrap();
    });

    assert!(partition_sector_map.partitions().sorted().eq(partition_sector_map.partitions()));
}

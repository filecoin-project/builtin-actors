use std::collections::BTreeMap;

use fil_actor_miner::TerminationResult;
use fvm_ipld_bitfield::BitField;

#[test]
fn test_termination_result() {
    let mut result = TerminationResult::new();
    assert!(result.is_empty());
    result.iter().for_each(|(_epoch, _sectors)| unreachable!());

    let mut result_a_sectors: BTreeMap<i64, BitField> = BTreeMap::new();

    let mut result_a_sector_1 = BitField::new();
    result_a_sector_1.set(9);

    let mut result_a_sector_2 = BitField::new();
    result_a_sector_2.set(1);
    result_a_sector_2.set(2);
    result_a_sector_2.set(4);

    let mut result_a_sector_3 = BitField::new();
    result_a_sector_3.set(3);
    result_a_sector_3.set(5);
    result_a_sector_3.set(7);

    result_a_sectors.insert(3, result_a_sector_1);
    result_a_sectors.insert(0, result_a_sector_2);
    result_a_sectors.insert(2, result_a_sector_3);

    let result_a = TerminationResult {
        sectors: result_a_sectors,
        sectors_processed: 7,
        partitions_processed: 1,
    };
    assert!(!result_a.is_empty());

    let mut result_b_sectors: BTreeMap<i64, BitField> = BTreeMap::new();

    let mut result_b_sector_1 = BitField::new();
    result_b_sector_1.set(12);

    let mut result_b_sector_2 = BitField::new();
    result_b_sector_2.set(10);

    result_b_sectors.insert(1, result_b_sector_1);
    result_b_sectors.insert(0, result_b_sector_2);

    let result_b = TerminationResult {
        sectors: result_b_sectors,
        partitions_processed: 1,
        sectors_processed: 2,
    };
    assert!(!result_b.is_empty());
    result += result_a;
    result += result_b;
    assert!(!result.is_empty());

    let mut expected_sectors: BTreeMap<i64, BitField> = BTreeMap::new();
    let mut sector_1 = BitField::new();
    sector_1.set(3);
    sector_1.set(5);
    sector_1.set(7);

    let mut sector_2 = BitField::new();
    sector_2.set(1);
    sector_2.set(2);
    sector_2.set(4);
    sector_2.set(10);

    let mut sector_3 = BitField::new();
    sector_3.set(12);

    let mut sector_4 = BitField::new();
    sector_4.set(9);

    expected_sectors.insert(2, sector_1);
    expected_sectors.insert(0, sector_2);
    expected_sectors.insert(1, sector_3);
    expected_sectors.insert(3, sector_4);

    let expected = TerminationResult {
        sectors: expected_sectors,
        partitions_processed: 2,
        sectors_processed: 9,
    };
    assert_eq!(expected.sectors_processed, result.sectors_processed);
    assert_eq!(expected.partitions_processed, result.partitions_processed);
    assert_eq!(expected.sectors.len(), result.sectors.len());
}

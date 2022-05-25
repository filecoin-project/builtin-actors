use std::collections::BTreeMap;

use fil_actor_miner::TerminationResult;
use fvm_ipld_bitfield::BitField;

#[test]
fn test_termination_result() {
    let result = TerminationResult::new();
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
    result_a_sectors.insert(3, result_a_sector_2);
    result_a_sectors.insert(3, result_a_sector_3);

    let result_a = TerminationResult {
        sectors: result_a_sectors,
        sectors_processed: 7,
        partitions_processed: 1,
    };

    assert!(!result_a.is_empty());
}

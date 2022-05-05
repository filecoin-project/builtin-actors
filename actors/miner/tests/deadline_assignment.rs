use fil_actor_miner::assign_deadlines;
use fil_actor_miner::Deadline;
use fil_actor_miner::SectorOnChainInfo;
use fil_actors_runtime::runtime::Policy;

#[test]
fn test_deadline_assignment() {
    const PARTITION_SIZE: u64 = 4;
    const MAX_PARITIONS: u64 = 100;

    #[derive(Clone)]
    struct Spec {
        live_sectors: u64,
        dead_sectors: u64,
        expect_sectors: Vec<u64>,
    }

    struct TestCase {
        sectors: u64,
        deadlines: Vec<Option<Spec>>,
    }
    let test_cases = [
        // Even assignment and striping.
        TestCase {
            sectors: 10,
            deadlines: vec![
                Some(Spec {
                    dead_sectors: 0,
                    live_sectors: 0,
                    expect_sectors: vec![0, 1, 2, 3, 8, 9],
                }),
                Some(Spec { dead_sectors: 0, live_sectors: 0, expect_sectors: vec![4, 5, 6, 7] }),
            ],
        },
        // Fill non-full first
        TestCase {
            sectors: 5,
            deadlines: vec![
                Some(Spec { dead_sectors: 0, live_sectors: 0, expect_sectors: vec![3, 4] }),
                Some(Spec { dead_sectors: 0, live_sectors: 0, expect_sectors: vec![] }),
                None,
                Some(Spec { dead_sectors: 0, live_sectors: 1, expect_sectors: vec![0, 1, 2] }),
            ],
        },
        // Assign to deadline with least number of live partitions.
        TestCase {
            sectors: 1,
            deadlines: vec![
                // 2 live partitions. +1 would add another.
                Some(Spec { dead_sectors: 0, live_sectors: 8, expect_sectors: vec![] }),
                // 2 live partitions. +1 wouldn't add another.
                // 1 dead partition.
                Some(Spec { dead_sectors: 5, live_sectors: 7, expect_sectors: vec![0] }),
            ],
        },
        // Avoid increasing max partitions. Both deadlines have the same
        // number of partitions post-compaction, but deadline 1 has
        // fewer pre-compaction.
        TestCase {
            sectors: 1,
            deadlines: vec![
                Some(Spec { dead_sectors: 4, live_sectors: 4, expect_sectors: vec![] }),
                Some(Spec { dead_sectors: 0, live_sectors: 4, expect_sectors: vec![0] }),
            ],
        },
        // With multiple open partitions, assign to most full first.
        TestCase {
            sectors: 1,
            deadlines: vec![
                Some(Spec { dead_sectors: 0, live_sectors: 1, expect_sectors: vec![] }),
                Some(Spec { dead_sectors: 0, live_sectors: 2, expect_sectors: vec![0] }),
            ],
        },
        // dead sectors also count
        TestCase {
            sectors: 1,
            deadlines: vec![
                Some(Spec { dead_sectors: 0, live_sectors: 1, expect_sectors: vec![] }),
                Some(Spec { dead_sectors: 2, live_sectors: 0, expect_sectors: vec![0] }),
            ],
        },
        // dead sectors really do count.
        TestCase {
            sectors: 1,
            deadlines: vec![
                Some(Spec { dead_sectors: 1, live_sectors: 0, expect_sectors: vec![] }),
                Some(Spec { dead_sectors: 2, live_sectors: 0, expect_sectors: vec![0] }),
            ],
        },
        // when partitions are equally full, assign based on live sectors.
        TestCase {
            sectors: 1,
            deadlines: vec![
                Some(Spec { dead_sectors: 1, live_sectors: 1, expect_sectors: vec![] }),
                Some(Spec { dead_sectors: 2, live_sectors: 0, expect_sectors: vec![0] }),
            ],
        },
    ];

    for (nth_tc, tc) in test_cases.iter().enumerate() {
        let deadlines: Vec<Option<Deadline>> = tc
            .deadlines
            .iter()
            .cloned()
            .map(|maybe_dl| {
                maybe_dl.map(|dl| Deadline {
                    live_sectors: dl.live_sectors,
                    total_sectors: dl.live_sectors + dl.dead_sectors,
                    ..Default::default()
                })
            })
            .collect();

        let sectors: Vec<SectorOnChainInfo> = (0..tc.sectors)
            .map(|i| SectorOnChainInfo { sector_number: i, ..Default::default() })
            .collect();

        let assignment = assign_deadlines(
            &Policy::default(),
            MAX_PARITIONS,
            PARTITION_SIZE,
            &deadlines,
            sectors,
        )
        .unwrap();
        for (i, sectors) in assignment.iter().enumerate() {
            if let Some(Some(dl)) = tc.deadlines.get(i) {
                // require.Equal(t, len(dl.expectSectors), len(sectors), "for deadline %d", i)
                assert_eq!(
                    dl.expect_sectors.len(),
                    sectors.len(),
                    "for deadline {}, case {}",
                    i,
                    nth_tc
                );
                for (i, &expected_sector_no) in dl.expect_sectors.iter().enumerate() {
                    assert_eq!(sectors[i].sector_number, expected_sector_no);
                }
            } else {
                assert!(
                    sectors.is_empty(),
                    "expected no sectors to have been assigned to blacked out deadline"
                );
            }
        }
    }
}

const MAX_PARTITIONS: u64 = 5;
const PARITION_SIZE: u64 = 5;

#[test]
fn fails_if_all_deadlines_hit_their_max_partitions_limit_before_assigning_all_sectors_to_deadlines()
{
    // one deadline can take 5 * 5 = 25 sectors
    // so 48 deadlines can take 48 * 25 = 1200 sectors.
    // Hence, we should fail if we try to assign 1201 sectors.

    let policy = Policy::default();
    let mut deadlines = Vec::new();
    deadlines.resize_with(policy.wpost_period_deadlines as usize, || {
        Some(Deadline { live_sectors: 0, total_sectors: 0, ..Default::default() })
    });

    let sectors: Vec<SectorOnChainInfo> =
        (0..1201).map(|i| SectorOnChainInfo { sector_number: i, ..Default::default() }).collect();

    assert!(assign_deadlines(&policy, MAX_PARTITIONS, PARITION_SIZE, &deadlines, sectors).is_err());
}

#[test]
fn succeeds_if_all_all_deadlines_hit_their_max_partitions_limit_but_assignment_is_complete() {
    // one deadline can take 5 * 5 = 25 sectors
    // so 48 deadlines that can take 48 * 25 = 1200 sectors.

    let policy = Policy::default();
    let mut deadlines = Vec::new();
    deadlines.resize_with(policy.wpost_period_deadlines as usize, || {
        Some(Deadline { live_sectors: 0, total_sectors: 0, ..Default::default() })
    });

    let sectors: Vec<SectorOnChainInfo> =
        (0..1200).map(|i| SectorOnChainInfo { sector_number: i, ..Default::default() }).collect();

    let deadline_to_sectors =
        assign_deadlines(&policy, MAX_PARTITIONS, PARITION_SIZE, &deadlines, sectors).unwrap();
    for sectors in deadline_to_sectors {
        assert_eq!(sectors.len(), 25);
    }
}

#[test]
fn fails_if_some_deadlines_have_sectors_beforehand_and_all_deadlines_hit_their_max_partition_limit()
{
    let policy = Policy::default();
    let mut deadlines = Vec::new();
    deadlines.resize_with(policy.wpost_period_deadlines as usize, || {
        Some(Deadline { live_sectors: 1, total_sectors: 2, ..Default::default() })
    });

    // can only take 1200 - (2 * 48) = 1104 sectors
    let sectors: Vec<SectorOnChainInfo> =
        (0..1105).map(|i| SectorOnChainInfo { sector_number: i, ..Default::default() }).collect();

    assert!(assign_deadlines(&policy, MAX_PARTITIONS, PARITION_SIZE, &deadlines, sectors).is_err());
}

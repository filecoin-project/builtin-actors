use fil_actor_miner_state_v9::{
    deadline_available_for_compaction, deadline_available_for_optimistic_post_dispute,
    new_deadline_info,
};
use fil_actors_runtime_common::runtime::Policy;

#[test]
fn test_compaction_window() {
    let period_start = 1024;
    let policy = Policy::default();
    let dl_info = new_deadline_info(&policy, period_start, 0, 0);

    assert!(
        deadline_available_for_compaction(
            &policy,
            period_start,
            0,
            dl_info.open - policy.wpost_challenge_window - 1
        ),
        "compaction is possible up till the blackout period"
    );
    assert!(
        !deadline_available_for_compaction(
            &policy,
            period_start,
            0,
            dl_info.open - policy.wpost_challenge_window
        ),
        "compaction is not possible during the prior window"
    );

    assert!(
        !deadline_available_for_compaction(&policy, period_start, 0, dl_info.open + 10),
        "compaction is not possible during the window"
    );
    assert!(
        !deadline_available_for_compaction(&policy, period_start, 0, dl_info.close),
        "compaction is not possible immediately after the window"
    );

    assert!(
        !deadline_available_for_compaction(
            &policy,
            period_start,
            0,
            dl_info.last() + policy.wpost_dispute_window
        ),
        "compaction is not possible before the proof challenge period has passed"
    );

    assert!(
        deadline_available_for_compaction(
            &policy,
            period_start,
            0,
            dl_info.close + policy.wpost_dispute_window
        ),
        "compaction is possible after the proof challenge period has passed"
    );

    assert!(
        deadline_available_for_compaction(
            &policy,
            period_start,
            0,
            dl_info.open + policy.wpost_proving_period - policy.wpost_challenge_window - 1
        ),
        "compaction remains possible until the next blackout"
    );

    assert!(
        !deadline_available_for_compaction(
            &policy,
            period_start,
            0,
            dl_info.open + policy.wpost_proving_period - policy.wpost_challenge_window
        ),
        "compaction is not possible during the next blackout"
    );
}

#[test]
fn test_challenge_window() {
    let period_start = 1024;
    let policy = Policy::default();
    let dl_info = new_deadline_info(&policy, period_start, 0, 0);

    assert!(
        !deadline_available_for_optimistic_post_dispute(&policy, period_start, 0, dl_info.open),
        "proof challenge is not possible while the window is open"
    );
    assert!(
        deadline_available_for_optimistic_post_dispute(&policy, period_start, 0, dl_info.close),
        "proof challenge is possible after the window is closes"
    );
    assert!(
        deadline_available_for_optimistic_post_dispute(
            &policy,
            period_start,
            0,
            dl_info.close + policy.wpost_dispute_window - 1
        ),
        "proof challenge is possible until the proof challenge period has passed"
    );
    assert!(
        !deadline_available_for_optimistic_post_dispute(
            &policy,
            period_start,
            0,
            dl_info.close + policy.wpost_dispute_window
        ),
        "proof challenge is not possible after the proof challenge period has passed"
    );
}

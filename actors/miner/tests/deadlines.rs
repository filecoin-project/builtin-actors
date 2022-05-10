use fil_actor_miner::{new_deadline_info, new_deadline_info_from_offset_and_epoch};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::EPOCHS_IN_DAY;

#[test]
fn quantization_spec_rounds_to_the_next_deadline() {
    let policy = Policy::default();
    let period_start = 2;
    let curr = period_start + policy.wpost_proving_period;
    let d = new_deadline_info(&policy, period_start, 10, curr);
    let quant = d.quant_spec();
    assert_eq!(d.next_not_elapsed().last(), quant.quantize_up(curr));
}

// All proving periods equivalent mod WPoStProving period should give equivalent
// dlines for a given epoch. Only the offset property should matter
#[test]
fn offset_and_epoch_invariant_checking() {
    let policy = Policy::default();
    let pp = 1972;
    let pp_three = 1972 + EPOCHS_IN_DAY * 3;
    let pp_million = 1972 + EPOCHS_IN_DAY * 1_000_000;

    for epoch in [4, 2_000, 400_000, 5_000_000] {
        let dline_a = new_deadline_info_from_offset_and_epoch(&policy, pp, epoch);
        let dline_b = new_deadline_info_from_offset_and_epoch(&policy, pp_three, epoch);
        let dline_c = new_deadline_info_from_offset_and_epoch(&policy, pp_million, epoch);

        assert_eq!(dline_a, dline_b);
        assert_eq!(dline_b, dline_c);
    }
}

#[test]
fn sanity_checks() {
    let policy = Policy::default();
    let offset = 7;
    let start = EPOCHS_IN_DAY * 103 + offset;

    // EPOCHS_IN_DAY*103 + offset we are in deadline 0, pp start = EPOCHS_IN_DAY*103 + offset
    let dline = new_deadline_info_from_offset_and_epoch(&policy, offset, start);
    assert_eq!(0, dline.index);
    assert_eq!(start, dline.period_start);

    // EPOCHS_IN_DAY*103 + offset + WPoStChallengeWindow - 1 we are in deadline 0
    let dline = new_deadline_info_from_offset_and_epoch(
        &policy,
        offset,
        start + policy.wpost_challenge_window - 1,
    );
    assert_eq!(0, dline.index);
    assert_eq!(start, dline.period_start);

    // EPOCHS_IN_DAY*103 + offset + WPoStChallengeWindow we are in deadline 1
    let dline = new_deadline_info_from_offset_and_epoch(
        &policy,
        offset,
        start + policy.wpost_challenge_window,
    );
    assert_eq!(1, dline.index);
    assert_eq!(start, dline.period_start);

    // EPOCHS_IN_DAY*103 + offset + 40*WPoStChallengeWindow we are in deadline 40
    let dline = new_deadline_info_from_offset_and_epoch(
        &policy,
        offset,
        start + 40 * policy.wpost_challenge_window,
    );
    assert_eq!(40, dline.index);
    assert_eq!(start, dline.period_start);

    // EPOCHS_IN_DAY*103 + offset + 40*WPoStChallengeWindow - 1 we are in deadline 39
    let dline = new_deadline_info_from_offset_and_epoch(
        &policy,
        offset,
        start + 40 * policy.wpost_challenge_window - 1,
    );
    assert_eq!(39, dline.index);
    assert_eq!(start, dline.period_start);

    // EPOCHS_IN_DAY*103 + offset + 40*WPoStChallengeWindow + 1 we are in deadline 40
    let dline = new_deadline_info_from_offset_and_epoch(
        &policy,
        offset,
        start + 40 * policy.wpost_challenge_window + 1,
    );
    assert_eq!(40, dline.index);
    assert_eq!(start, dline.period_start);

    // EPOCHS_IN_DAY*103 + offset + WPoStPeriodDeadlines*WPoStChallengeWindow -1 we are in deadline WPoStPeriodDeadlines - 1
    let dline = new_deadline_info_from_offset_and_epoch(
        &policy,
        offset,
        start + policy.wpost_period_deadlines as i64 * policy.wpost_challenge_window - 1,
    );
    assert_eq!(policy.wpost_period_deadlines - 1, dline.index);
    assert_eq!(start, dline.period_start);

    // EPOCHS_IN_DAY*103 + offset + WPoStPeriodDeadlines*WPoStChallengeWindow + 1 we are in deadline 0, pp start = 2880*104 + offset;
    let dline = new_deadline_info_from_offset_and_epoch(
        &policy,
        offset,
        start + policy.wpost_period_deadlines as i64 * policy.wpost_challenge_window,
    );
    assert_eq!(0, dline.index);
    assert_eq!(start + policy.wpost_proving_period, dline.period_start);
}

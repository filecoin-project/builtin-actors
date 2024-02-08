// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

/// Assumed epoch duration. If this changes, a large state-migration will need to be run to update
/// expirations, etc.
pub const EPOCH_DURATION_SECONDS: i64 = 30;

pub const SECONDS_IN_HOUR: i64 = 3600;
pub const SECONDS_IN_DAY: i64 = 86400;
pub const SECONDS_IN_YEAR: i64 = 31556925;
pub const EPOCHS_IN_HOUR: i64 = SECONDS_IN_HOUR / EPOCH_DURATION_SECONDS;
pub const EPOCHS_IN_DAY: i64 = SECONDS_IN_DAY / EPOCH_DURATION_SECONDS;
pub const EPOCHS_IN_YEAR: i64 = SECONDS_IN_YEAR / EPOCH_DURATION_SECONDS;

/// This is a protocol constant from Filecoin and depends on expected consensus. Here it is used to
/// determine expected rewards, fault penalties, etc. This will need to be changed if expected
/// consensus ever changes (and, likely, so will pledge, etc.).
pub const EXPECTED_LEADERS_PER_EPOCH: u64 = 5;

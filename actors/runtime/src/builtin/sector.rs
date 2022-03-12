// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::sector::{RegisteredPoStProof, StoragePower};

/// Returns the minimum storage power required for each seal proof types.
pub fn consensus_miner_min_power(p: RegisteredPoStProof) -> anyhow::Result<StoragePower> {
    use RegisteredPoStProof::*;
    match p {
        StackedDRGWinning2KiBV1
        | StackedDRGWinning8MiBV1
        | StackedDRGWinning512MiBV1
        | StackedDRGWinning32GiBV1
        | StackedDRGWinning64GiBV1
        | StackedDRGWindow2KiBV1
        | StackedDRGWindow8MiBV1
        | StackedDRGWindow512MiBV1
        | StackedDRGWindow32GiBV1
        | StackedDRGWindow64GiBV1 => {
            let power: u64 = if cfg!(feature = "min-power-2k") {
                2 << 10
            } else if cfg!(feature = "min-power-2g") {
                2 << 30
            } else {
                10 << 40
            };
            Ok(StoragePower::from(power))
        }
        Invalid(i) => Err(anyhow::anyhow!("unsupported proof type: {}", i)),
    }
}

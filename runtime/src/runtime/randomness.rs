// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::clock::ChainEpoch;
use fvm_shared::randomness::RANDOMNESS_LENGTH;
use num_derive::FromPrimitive;
use serde_repr::*;

/// Specifies a domain for randomness generation.
#[derive(PartialEq, Eq, Copy, Clone, FromPrimitive, Debug, Hash, Deserialize_repr)]
#[repr(i64)]
pub enum DomainSeparationTag {
    TicketProduction = 1,
    ElectionProofProduction = 2,
    WinningPoStChallengeSeed = 3,
    WindowedPoStChallengeSeed = 4,
    SealRandomness = 5,
    InteractiveSealChallengeSeed = 6,
    WindowPoStDeadlineAssignment = 7,
    MarketDealCronSeed = 8,
    PoStChainCommit = 9,
    EvmPrevRandao = 10,
}

#[allow(unused)]
pub fn draw_randomness(
    hasher: impl FnOnce(&[u8]) -> [u8; 32],
    rbase: &[u8; RANDOMNESS_LENGTH],
    pers: DomainSeparationTag,
    round: ChainEpoch,
    entropy: &[u8],
) -> [u8; RANDOMNESS_LENGTH] {
    let mut data = Vec::with_capacity(RANDOMNESS_LENGTH + 8 + 8 + entropy.len());

    // Append the personalization value
    let i64_bytes = (pers as i64).to_be_bytes();
    data.extend_from_slice(&i64_bytes);

    // Append the randomness
    data.extend_from_slice(rbase);

    // Append the round
    let i64_bytes = round.to_be_bytes();
    data.extend_from_slice(&i64_bytes);

    // Append the entropy
    data.extend_from_slice(entropy);

    hasher(&data)
    //
    // fvm::crypto::hash_blake2b(&data)
}

#[cfg(test)]
mod tests {
    use crate::runtime::randomness::draw_randomness;
    use crate::runtime::DomainSeparationTag;
    use crate::test_utils::blake2b_256;
    use base64::Engine;

    #[test]
    fn draw_randomness_test() {
        let expected_randomness = base64::engine::general_purpose::STANDARD
            .decode("3MCqcLHKZ+pil4MqTS9wjsd+yPvTuTrq8PkGjEo3tYQ=")
            .unwrap();

        let digest = base64::engine::general_purpose::STANDARD
            .decode("GOobxkrhS1hiFA1EYUKZM3xsyVfy5Xy3bQ0gLPnecYs=")
            .unwrap();

        let entropy = base64::engine::general_purpose::STANDARD.decode("RACZyzQ=").unwrap();

        assert_eq!(
            expected_randomness,
            draw_randomness(
                blake2b_256,
                <&[u8; 32]>::try_from(digest.as_slice()).unwrap(),
                DomainSeparationTag::SealRandomness,
                2797727,
                entropy.as_slice(),
            )
        );
    }
}

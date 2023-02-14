// randomness from two different contracts

mod asm;
mod util;

use crate::util::dispatch_num_word;
use rand::prelude::*;

#[allow(dead_code)]
pub fn prevrandao_contract() -> Vec<u8> {
    let init = "";
    let body = r#"
%dispatch_begin()
%dispatch(0x00, basic)
%dispatch(0x01, cache)
%dispatch_end()
    
basic: 
    jumpdest
    difficulty
    %return_stack_word()

cache: 
    jumpdest
    # push store first randomness at 0x00
    difficulty
    push1 0x00
    mstore
    # store second randomness at 0x20
    difficulty
    push1 0x20
    mstore

    # return both values 0x00-0x40
    push1 0x40
    push1 0x00
    return
"#;

    asm::new_contract("prevrandao", init, body).unwrap()
}

#[test]
fn test_prevrandao() {
    let mut rt = util::construct_and_verify(prevrandao_contract());
    rt.epoch = 101;

    // simple test
    {
        rt.expect_get_randomness_from_beacon(
            fil_actors_runtime::runtime::DomainSeparationTag::EvmPrevRandao,
            101,
            b"prevrandao".to_vec(),
            [0u8; 32],
        );

        let result = util::invoke_contract(&mut rt, &dispatch_num_word(0));
        rt.verify();
        assert_eq!(result, [0u8; 32], "expected empty randomness");
        rt.reset();
    }

    let mut rand = thread_rng();

    // actual random value
    {
        let expected: [u8; 32] = rand.gen();
        rt.expect_get_randomness_from_beacon(
            fil_actors_runtime::runtime::DomainSeparationTag::EvmPrevRandao,
            101,
            b"prevrandao".to_vec(),
            expected,
        );

        let result = util::invoke_contract(&mut rt, &dispatch_num_word(0));
        rt.verify();
        assert_eq!(result, expected, "expected random value {expected:?}");
        rt.reset();
    }

    // check cache
    {
        let expected: [u8; 32] = rand.gen();
        rt.expect_get_randomness_from_beacon(
            fil_actors_runtime::runtime::DomainSeparationTag::EvmPrevRandao,
            101,
            b"prevrandao".to_vec(),
            expected,
        );
        let expected = [expected, expected].concat();

        let result = util::invoke_contract(&mut rt, &dispatch_num_word(1));
        rt.verify();
        assert_eq!(result, expected, "expected 2 of the same random value {expected:?}");
        rt.reset();
    }
}

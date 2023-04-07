mod asm;

use fil_actors_evm_shared::uints::U256;
use fvm_ipld_encoding::IPLD_RAW;
use fvm_shared::event::{ActorEvent, Entry, Flags};

mod util;

#[allow(dead_code)]
pub fn events_contract() -> Vec<u8> {
    let init = r#"
"#;
    let body = r#"
# method dispatch:
# - 0x00000000 -> log_zero_data
# - 0x00000001 -> log_zero_nodata
# - 0x00000002 -> log_four_data

%dispatch_begin()
%dispatch(0x00, log_zero_data)
%dispatch(0x01, log_zero_nodata)
%dispatch(0x02, log_four_data)
%dispatch_end()

#### log a zero topic event with data
log_zero_data:
jumpdest
push8 0x1122334455667788
push1 0x00
mstore
push1 0x08
push1 0x18 ## index 24 into memory as mstore writes a full word
log0
push1 0x00
push1 0x00
return

#### log a zero topic event with no data
log_zero_nodata:
jumpdest
push1 0x00
push1 0x00
log0
push1 0x00
push1 0x00
return

#### log a four topic event with data
log_four_data:
jumpdest
push8 0x1122334455667788
push1 0x00
mstore
push4 0x4444
push3 0x3333
push2 0x2222
push2 0x1111
push1 0x08
push1 0x18 ## index 24 into memory as mstore writes a full word
log4
push1 0x00
push1 0x00
return

"#;

    asm::new_contract("events", init, body).unwrap()
}

#[test]
fn test_events() {
    let contract = events_contract();

    let rt = util::construct_and_verify(contract);

    // log zero with data
    let mut contract_params = vec![0u8; 32];
    rt.expect_emitted_event(ActorEvent {
        entries: vec![Entry {
            flags: Flags::FLAG_INDEXED_ALL,
            key: "d".to_string(),
            codec: IPLD_RAW,
            value: vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        }],
    });
    util::invoke_contract(&rt, &contract_params);

    // log zero without data
    contract_params[3] = 0x01;
    rt.expect_emitted_event(ActorEvent { entries: vec![] });
    util::invoke_contract(&rt, &contract_params);

    // log four with data
    contract_params[3] = 0x02;
    rt.expect_emitted_event(ActorEvent {
        entries: vec![
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "t1".to_string(),
                codec: IPLD_RAW,
                value: U256::from(0x1111).to_bytes().into(),
            },
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "t2".to_string(),
                codec: IPLD_RAW,
                value: U256::from(0x2222).to_bytes().into(),
            },
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "t3".to_string(),
                codec: IPLD_RAW,
                value: U256::from(0x3333).to_bytes().into(),
            },
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "t4".to_string(),
                codec: IPLD_RAW,
                value: U256::from(0x4444).to_bytes().into(),
            },
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "d".to_string(),
                codec: IPLD_RAW,
                value: vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
            },
        ],
    });
    util::invoke_contract(&rt, &contract_params);

    rt.verify();
}

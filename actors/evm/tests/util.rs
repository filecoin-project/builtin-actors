use cid::Cid;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fil_actor_evm as evm;
use fil_actor_evm::State;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{
    test_utils::{self, *},
    EAM_ACTOR_ID, INIT_ACTOR_ADDR,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{BytesDe, BytesSer};
use fvm_shared::{address::Address, IDENTITY_HASH, IPLD_RAW};
use lazy_static::lazy_static;

use std::fmt::Debug;

#[allow(dead_code)]
pub fn construct_and_verify(initcode: Vec<u8>) -> MockRuntime {
    init_construct_and_verify(initcode, |_| {})
}

pub const CONTRACT_ADDRESS: [u8; 20] =
    hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000");

#[allow(unused)]
pub const CONTRACT_ID: Address = Address::new_id(0);

pub fn init_construct_and_verify<F: FnOnce(&mut MockRuntime)>(
    initcode: Vec<u8>,
    initrt: F,
) -> MockRuntime {
    let mut rt = MockRuntime::default();

    // enable logging to std
    test_utils::init_logging().ok();

    // construct EVM actor
    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);
    initrt(&mut rt);

    // first actor created is 0
    rt.set_delegated_address(0, Address::new_delegated(EAM_ACTOR_ID, &CONTRACT_ADDRESS).unwrap());
    rt.set_address_actor_type(Address::new_id(0), *EVM_ACTOR_CODE_ID);

    let params = evm::ConstructorParams {
        creator: EthAddress::from_id(fil_actors_runtime::EAM_ACTOR_ADDR.id().unwrap()),
        initcode: initcode.into(),
    };

    assert!(rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .is_none());
    let evm_st: State = rt.state().unwrap();
    let evm_code = rt.store.get(&evm_st.bytecode).unwrap().unwrap();
    log::trace!("bytecode constructed: {}", hex::encode(evm_code));
    rt.verify();

    rt
}

#[allow(dead_code)]
pub fn invoke_contract(rt: &mut MockRuntime, input_data: &[u8]) -> Vec<u8> {
    rt.expect_validate_caller_any();
    let BytesDe(res) = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeContract as u64,
            IpldBlock::serialize_cbor(&BytesSer(input_data)).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    res
}

#[allow(dead_code)]
// silly to have the full word for a single byte but...
pub fn dispatch_num_word(method_num: u8) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[3] = method_num;
    word
}

#[allow(dead_code)]
pub fn id_to_vec(src: &Address) -> Vec<u8> {
    U256::from(src.id().unwrap()).to_bytes().to_vec()
}

lazy_static! {
    pub static ref DUMMY_ACTOR_CODE_ID: Cid =
        Cid::new_v1(IPLD_RAW, Multihash::wrap(IDENTITY_HASH, b"foobarboxy").unwrap());
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrecompileExit {
    Reverted = 0,
    Success = 1,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum NativePrecompile {
    ResolveAddress = 1,
    LookupDelegatedAddress = 2,
    CallActor = 3,
    GetActorType = 4,
    CallActorId = 5,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum PrecompileCallOpcode {
    Call(u64),
    DelegateCall,
    StaticCall,
}

impl PrecompileCallOpcode {
    fn dispatch_num(&self) -> u8 {
        match self {
            PrecompileCallOpcode::Call(_) => 0,
            PrecompileCallOpcode::DelegateCall => 1,
            PrecompileCallOpcode::StaticCall => 2,
        }
    }

    fn call_value(&self) -> Option<u64> {
        match self {
            PrecompileCallOpcode::Call(value) => Some(*value),
            PrecompileCallOpcode::DelegateCall | PrecompileCallOpcode::StaticCall => None,
        }
    }
}

#[allow(dead_code)]
pub fn precompile_address(prefix: u8, index: u8) -> EthAddress {
    let mut buf = [0u8; 20];
    buf[0] = prefix;
    buf[19] = index;
    EthAddress(buf)
}

impl NativePrecompile {
    #[allow(dead_code)]
    pub fn eth_address(&self) -> EthAddress {
        precompile_address(0xfe, *self as u8)
    }
}

#[derive(Clone)]
pub struct PrecompileTest {
    pub expected_exit_code: PrecompileExit,
    pub precompile_address: EthAddress,
    pub output_size: u32,
    pub gas_avaliable: u64,
    pub call_op: PrecompileCallOpcode,
    pub input: Vec<u8>,
    pub expected_return: Vec<u8>,
}

impl Debug for PrecompileTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrecompileTest")
            .field("call_opcode", &self.call_op)
            .field("expected_exit_code", &self.expected_exit_code)
            .field("precompile_address", &self.precompile_address)
            .field("output_size", &self.output_size)
            .field("input", &hex::encode(&self.input))
            .field("expected_output", &hex::encode(&self.expected_return))
            .field("gas_avaliable", &self.gas_avaliable)
            .finish()
    }
}

impl PrecompileTest {
    #[allow(dead_code)]
    pub fn run_test(&self, rt: &mut MockRuntime) {
        rt.expect_gas_available(self.gas_avaliable);
        log::trace!("{:#?}", &self);
        // first byte is precompile number, second is output buffer size, rest is input to precompile
        let result = invoke_contract(
            rt,
            &[
                dispatch_num_word(self.call_op.dispatch_num()).to_vec(),
                self.precompile_address.as_evm_word().to_bytes().to_vec(),
                U256::from(self.output_size).to_bytes().to_vec(),
                self.call_op
                    .call_value()
                    .map(|v| U256::from(v).to_bytes().to_vec())
                    .unwrap_or_default(), // empty vec if not call
                self.input.clone(),
            ]
            .concat(),
        );
        log::trace!("exit [{}] returned: {:?}", result[0], hex::encode(&result[1..]));
        rt.verify();

        let returned_exit = match result[0] {
            0 => PrecompileExit::Reverted,
            1 => PrecompileExit::Success,
            _ => panic!("Expected call to give either 1 or 0, this is a bug!"),
        };
        assert_eq!(self.expected_exit_code, returned_exit);
        assert_eq!(self.expected_return, &result[1..]);

        rt.reset();
    }

    #[allow(dead_code)]
    pub fn run_test_expecting<T: Into<Vec<u8>>>(
        &mut self,
        rt: &mut MockRuntime,
        expecting: T,
        call_exit: PrecompileExit,
    ) {
        self.expected_return = expecting.into();
        self.expected_exit_code = call_exit;
        self.run_test(rt);
    }

    #[allow(dead_code)]
    /// returns (initcode, bytecode) asm
    pub fn test_runner_assembly() -> (String, String) {
        let body = r#"
# store entire input to mem 0x00
calldatasize
# first word of input is the dispatch
push1 0x20 # input offset
push1 0x00 # dst offset
calldatacopy

# dispatch to different call opcodes
%dispatch_begin()
%dispatch(0x00, p_call)
%dispatch(0x01, p_delegatecall)
%dispatch(0x02, p_staticcall)
%dispatch_end()

p_call:
    jumpdest

    # out size
    push1 0x20 # second word of input
    mload

    # out off
    push2 0xA000

    # in size
    push1 0x60 # three words
    calldatasize
    sub
    # in off
    push1 0x60 # three words

    # value
    push1 0x40 # third word of input
    mload

    # precompile address
    push1 0x00 # first word of input is precompile
    mload

    # gas
    push1 0x00

    call

    # return

    # write exit code to first byte of memory
    push1 0x00 # offset
    mstore8

    # write precompile return to memory
    returndatasize
    push1 0x00 # input offset
    push1 0x01 # dst offset (plus 1 to accommodate exit code)
    returndatacopy

    # size
    returndatasize
    push1 0x01
    add
    # offset
    push1 0x00
    return

p_delegatecall:
    jumpdest

    # out size
    push1 0x20 # second word of input
    mload

    # out off
    push2 0xA000

    # in size
    push1 0x40 # two words
    calldatasize
    sub
    # in off
    push1 0x40 # two words

    # precompile address
    push1 0x00 # first word of input is precompile
    mload

    # gas
    push1 0x00

    delegatecall

    # return

    # write exit code to first byte of memory
    push1 0x00 # offset
    mstore8

    # write precompile return to memory
    returndatasize
    push1 0x00 # input offset
    push1 0x01 # dst offset (plus 1 to accommodate exit code)
    returndatacopy

    # size
    returndatasize
    push1 0x01
    add
    # offset
    push1 0x00
    return

p_staticcall:
    jumpdest

    # out size
    push1 0x20 # second word of input
    mload

    # out off
    push2 0xA000

    # in size
    push1 0x40 # two words
    calldatasize
    sub
    # in off
    push1 0x40 # two words

    # precompile address
    push1 0x00 # first word of input is precompile
    mload

    # gas
    push1 0x00

    delegatecall

    # return

    # write exit code to first byte of memory
    push1 0x00 # offset
    mstore8

    # write precompile return to memory
    returndatasize
    push1 0x00 # input offset
    push1 0x01 # dst offset (plus 1 to accommodate exit code)
    returndatacopy

    # size
    returndatasize
    push1 0x01
    add
    # offset
    push1 0x00
    return
"#;
        (String::new(), body.to_string())
    }
}

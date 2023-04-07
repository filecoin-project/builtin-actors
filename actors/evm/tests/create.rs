mod asm;

use evm::ext::eam;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;

mod util;

#[allow(dead_code)]
pub fn magic_precompile_contract() -> Vec<u8> {
    let init = "";
    let body = r#"
# magic value, used as initcode
push16 0x666F6F206261722062617A20626F7879 # foo bar baz boxy
push2 0x0100 # offset of input data
mstore # "bytecode"


%dispatch_begin()
%dispatch(0x00, test_create)
%dispatch(0x01, test_create2)
%dispatch_end()

test_create:
    jumpdest
    push1 0x10   # in size (16 bytes)
    push2 0x0110 # in offset
    push1 0x01   # value (attoFil)
    create
    %return_stack_word()

test_create2:
    jumpdest
    # salt
    push9 0x796573206D616E6921 # yes mani!
    push1 0x10   # in size (16 bytes)
    push2 0x0110 # in offset
    push1 0x01   # endowment (attoFil)
    create2
    %return_stack_word()
"#;

    asm::new_contract("magic-precompile", init, body).unwrap()
}

#[test]
fn test_create() {
    const GAS_AVAILABLE: u64 = 64_000_000;
    const GAS_SUBCALL: u64 = 63_000_000;

    let contract = magic_precompile_contract();
    let rt = util::construct_and_verify(contract);

    let fake_eth_addr = EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));
    let fake_ret = eam::CreateReturn {
        actor_id: 12345,
        eth_address: fake_eth_addr,
        robust_address: Some((&fake_eth_addr).try_into().unwrap()),
    };

    let salt =
        hex_literal::hex!("0000000000000000000000000000000000000000000000796573206D616E6921");

    let create2_params = eam::Create2Params {
        code: hex_literal::hex!("666F6F206261722062617A20626F7879").to_vec(),
        salt,
    };

    let mut create_params = eam::CreateParams {
        code: hex_literal::hex!("666F6F206261722062617A20626F7879").to_vec(),
        nonce: 1,
    };

    // byte 3 is method num
    let mut contract_params = [0u8; 32];

    // invoke contract -- create
    {
        rt.add_balance(TokenAmount::from_atto(1));

        rt.expect_gas_available(GAS_AVAILABLE);
        rt.expect_send(
            EAM_ACTOR_ADDR,
            eam::CREATE_METHOD_NUM,
            IpldBlock::serialize_cbor(&create_params).unwrap(),
            TokenAmount::from_atto(1),
            Some(GAS_SUBCALL),
            SendFlags::empty(),
            IpldBlock::serialize_cbor(&fake_ret).unwrap(),
            ExitCode::OK,
            None,
        );

        let result = util::invoke_contract(&rt, &contract_params);
        let result: [u8; 20] = result[12..].try_into().unwrap();
        let result = EthAddress(result);
        // make sure we arent doing weird things to EAM's return value
        assert_eq!(result, fake_eth_addr);
    }

    // invoke contract -- create with new nonce
    {
        create_params.nonce += 1;

        rt.add_balance(TokenAmount::from_atto(1));

        rt.expect_gas_available(GAS_AVAILABLE);
        rt.expect_send(
            EAM_ACTOR_ADDR,
            eam::CREATE_METHOD_NUM,
            IpldBlock::serialize_cbor(&create_params).unwrap(),
            TokenAmount::from_atto(1),
            Some(GAS_SUBCALL),
            SendFlags::empty(),
            IpldBlock::serialize_cbor(&fake_ret).unwrap(),
            ExitCode::OK,
            None,
        );

        let result = util::invoke_contract(&rt, &contract_params);
        let result: [u8; 20] = result[12..].try_into().unwrap();
        let result = EthAddress(result);
        // make sure we arent doing weird things to EAM's return value
        assert_eq!(result, fake_eth_addr);
    }

    contract_params[3] = 0x01;

    // invoke contract -- create2
    {
        rt.add_balance(TokenAmount::from_atto(1));

        rt.expect_gas_available(GAS_AVAILABLE);
        rt.expect_send(
            EAM_ACTOR_ADDR,
            eam::CREATE2_METHOD_NUM,
            IpldBlock::serialize_cbor(&create2_params).unwrap(),
            TokenAmount::from_atto(1),
            Some(GAS_SUBCALL),
            SendFlags::empty(),
            IpldBlock::serialize_cbor(&fake_ret).unwrap(),
            ExitCode::OK,
            None,
        );

        let result = util::invoke_contract(&rt, &contract_params);
        let result: [u8; 20] = result[12..].try_into().unwrap();
        let result = EthAddress(result);
        // make sure we arent doing weird things to EAM's return value
        assert_eq!(result, fake_eth_addr);
    }

    // not enough funds -- create2
    {
        let result = util::invoke_contract(&rt, &contract_params);
        assert_eq!(&result[..], &[0; 32]);
    }

    contract_params[3] = 0x00;

    // not enough funds -- create
    {
        create_params.nonce += 3;

        let result = util::invoke_contract(&rt, &contract_params);
        assert_eq!(&result[..], &[0; 32]);
    }
}

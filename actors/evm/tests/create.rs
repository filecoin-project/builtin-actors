mod asm;

use evm::interpreter::address::EthAddress;
use evm::interpreter::instructions::lifecycle::{
    Create2Params, CreateParams, EamReturn, CREATE2_METHOD_NUM, CREATE_METHOD_NUM,
};
use fil_actor_evm as evm;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;

mod util;

#[allow(dead_code)]
pub fn magic_precompile_contract() -> Vec<u8> {
    let init = "";
    let body = r#"


%macro return_stack_word()
    push1 0x00
    mstore
    push1 0x20 # always return a full word
    push1 0x00
    return
%end

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
    let contract = magic_precompile_contract();
    let mut rt = util::construct_and_verify(contract);

    let fake_eth_addr = EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));
    let fake_ret = EamReturn {
        actor_id: 12345,
        eth_address: fake_eth_addr,
        robust_address: (&fake_eth_addr).try_into().unwrap(),
    };

    let salt =
        hex_literal::hex!("0000000000000000000000000000000000000000000000796573206D616E6921");

    let create2_params = Create2Params {
        code: hex_literal::hex!("666F6F206261722062617A20626F7879").to_vec(),
        salt,
    };

    let mut create_params = CreateParams {
        code: hex_literal::hex!("666F6F206261722062617A20626F7879").to_vec(),
        nonce: 0,
    };

    // byte 3 is method num
    let mut contract_params = [0u8; 32];

    // invoke contract -- create
    {
        rt.add_balance(TokenAmount::from_atto(1));

        rt.expect_send(
            EAM_ACTOR_ADDR,
            CREATE_METHOD_NUM,
            RawBytes::serialize(create_params.clone()).unwrap(),
            TokenAmount::from_atto(1),
            RawBytes::serialize(fake_ret).unwrap(),
            ExitCode::OK,
        );

        let result = util::invoke_contract(&mut rt, &contract_params);
        let result: [u8; 20] = result[12..].try_into().unwrap();
        let result = EthAddress(result);
        // make sure we arent doing weird things to EAM's return value
        assert_eq!(result, fake_eth_addr);
    }

    // invoke contract -- create with new nonce
    {
        create_params.nonce += 1;

        rt.add_balance(TokenAmount::from_atto(1));

        rt.expect_send(
            EAM_ACTOR_ADDR,
            CREATE_METHOD_NUM,
            RawBytes::serialize(create_params.clone()).unwrap(),
            TokenAmount::from_atto(1),
            RawBytes::serialize(fake_ret).unwrap(),
            ExitCode::OK,
        );

        let result = util::invoke_contract(&mut rt, &contract_params);
        let result: [u8; 20] = result[12..].try_into().unwrap();
        let result = EthAddress(result);
        // make sure we arent doing weird things to EAM's return value
        assert_eq!(result, fake_eth_addr);
    }

    contract_params[3] = 0x01;

    // invoke contract -- create2
    {
        rt.add_balance(TokenAmount::from_atto(1));

        rt.expect_send(
            EAM_ACTOR_ADDR,
            CREATE2_METHOD_NUM,
            RawBytes::serialize(create2_params.clone()).unwrap(),
            TokenAmount::from_atto(1),
            RawBytes::serialize(fake_ret).unwrap(),
            ExitCode::OK,
        );

        let result = util::invoke_contract(&mut rt, &contract_params);
        let result: [u8; 20] = result[12..].try_into().unwrap();
        let result = EthAddress(result);
        // make sure we arent doing weird things to EAM's return value
        assert_eq!(result, fake_eth_addr);
    }

    // not enough funds -- create2
    {
        rt.expect_send(
            EAM_ACTOR_ADDR,
            CREATE2_METHOD_NUM,
            RawBytes::serialize(create2_params).unwrap(),
            TokenAmount::from_atto(1),
            RawBytes::serialize(fake_ret).unwrap(),
            ExitCode::OK,
        );

        let result = util::invoke_contract(&mut rt, &contract_params);
        assert_eq!(&result[..], &[0; 32]);
    }

    contract_params[3] = 0x00;

    // not enough funds -- create
    {
        // TODO this nonce is broken
        create_params.nonce += 3;
        rt.expect_send(
            EAM_ACTOR_ADDR,
            CREATE_METHOD_NUM,
            RawBytes::serialize(create_params).unwrap(),
            TokenAmount::from_atto(1),
            RawBytes::serialize(fake_ret).unwrap(),
            ExitCode::OK,
        );

        let result = util::invoke_contract(&mut rt, &contract_params);
        assert_eq!(&result[..], &[0; 32]);
    }
}

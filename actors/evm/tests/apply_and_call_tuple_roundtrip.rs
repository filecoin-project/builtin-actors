use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::{from_slice, to_vec};

#[test]
fn apply_and_call_params_roundtrip() {
    let auth = evm::DelegationParam {
        chain_id: 314,
        address: EthAddress([0xAA; 20]),
        nonce: 7,
        y_parity: 1,
        r: vec![0x11; 32],
        s: vec![0x22; 32],
    };
    let call = evm::ApplyCall { to: EthAddress([0xBB; 20]), value: vec![0x01, 0x02], input: vec![0x03, 0x04] };
    let params = evm::ApplyAndCallParams { list: vec![auth.clone()], call };

    let enc = to_vec(&params).expect("encode");
    let dec: evm::ApplyAndCallParams = from_slice(&enc).expect("decode");

    assert_eq!(dec.list.len(), 1);
    assert_eq!(dec.list[0], auth);
    assert_eq!(dec.call.to, EthAddress([0xBB; 20]));
    assert_eq!(dec.call.value, vec![0x01, 0x02]);
    assert_eq!(dec.call.input, vec![0x03, 0x04]);
}


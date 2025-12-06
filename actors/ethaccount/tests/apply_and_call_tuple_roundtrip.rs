use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::eip7702::{ApplyAndCallParams, ApplyCall, DelegationParam};
use fvm_ipld_encoding::{from_slice, to_vec};

#[test]
fn apply_and_call_params_roundtrip() {
    let auth = DelegationParam {
        chain_id: 314,
        address: EthAddress([0xAA; 20]),
        nonce: 7,
        y_parity: 1,
        r: vec![0x11; 32],
        s: vec![0x22; 32],
    };
    let call =
        ApplyCall { to: EthAddress([0xBB; 20]), value: vec![0x01, 0x02], input: vec![0x03, 0x04] };
    let params = ApplyAndCallParams { list: vec![auth.clone()], call };

    let enc = to_vec(&params).expect("encode");
    let dec: ApplyAndCallParams = from_slice(&enc).expect("decode");

    assert_eq!(dec.list.len(), 1);
    assert_eq!(dec.list[0], auth);
    assert_eq!(dec.call.to, EthAddress([0xBB; 20]));
    assert_eq!(dec.call.value, vec![0x01, 0x02]);
    assert_eq!(dec.call.input, vec![0x03, 0x04]);
}

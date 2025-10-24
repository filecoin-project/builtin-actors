use fil_actor_delegator::{ApplyDelegationsParams, DelegationParam};
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;

#[test]
fn tuple_wrapper_roundtrip_dag_cbor() {
    let params = ApplyDelegationsParams {
        list: vec![
            DelegationParam { chain_id: 0, address: EthAddress::from_id(1), nonce: 0, y_parity: 1, r: [11u8; 32], s: [22u8; 32] },
            DelegationParam { chain_id: 0, address: EthAddress::from_id(2), nonce: 5, y_parity: 0, r: [33u8; 32], s: [44u8; 32] },
        ],
    };

    let blk = IpldBlock::serialize_dag_cbor(&params).expect("encode");
    let decoded: ApplyDelegationsParams = blk.unwrap().deserialize().expect("decode");

    assert_eq!(decoded.list.len(), 2);
    assert_eq!(decoded.list[0].chain_id, 0);
    assert_eq!(decoded.list[0].address, EthAddress::from_id(1));
    assert_eq!(decoded.list[0].nonce, 0);
    assert_eq!(decoded.list[0].y_parity, 1);
    assert_eq!(decoded.list[1].nonce, 5);
}

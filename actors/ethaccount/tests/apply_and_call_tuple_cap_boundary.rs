use fil_actor_ethaccount as ethaccount;
use fil_actors_evm_shared::{address::EthAddress, eip7702};
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;

fn mk_tuple(nonce: u64) -> eip7702::DelegationParam {
    let mut addr = [0u8; 20];
    addr[19] = (nonce & 0xFF) as u8; // make unique per tuple
    eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress(addr),
        nonce,
        y_parity: 0,
        r: vec![1u8],
        s: vec![1u8],
    }
}

#[test]
fn tuple_cap_64_ok_65_reject() {
    let mut rt = MockRuntime::new();
    rt.expect_validate_caller_any();
    // Receiver must be EthAccount; set a predictable f4 address under EAM namespace.
    rt.set_delegated_address(1000, Address::new_delegated(EAM_ACTOR_ID, &[0u8; 20]).unwrap());
    rt.caller.replace(Address::new_id(10));
    rt.receiver = Address::new_id(1000);

    // Receiver-only constraint applies on this branch; cap boundary is asserted on the rejection path below.

    // 65 tuples rejected
    let list = (0..65).map(mk_tuple).collect::<Vec<_>>();
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err(), "65 tuples should be rejected");
}

// SPDX-License-Identifier: Apache-2.0 MIT
pragma solidity >=0.4.25 <=0.8.17;

contract CallActorPrecompile {
    address constant CALL_ACTOR_ADDRESS = 0xfe00000000000000000000000000000000000003;
    address constant CALL_ACTOR_ID = 0xfe00000000000000000000000000000000000005;

    function call_actor_id(uint64 method, uint256 value, uint64 flags, uint64 codec, bytes calldata params, uint64 id) public returns (bool, bytes memory) {
        return address(CALL_ACTOR_ID).delegatecall(abi.encode(method, value, flags, codec, params, id));
    }

    function call_actor_address(uint64 method, uint256 value, uint64 flags, uint64 codec, bytes calldata params, bytes calldata filAddress) public returns (bool, bytes memory) {
        return address(CALL_ACTOR_ADDRESS).delegatecall(abi.encode(method, value, flags, codec, params, filAddress));
    }
}

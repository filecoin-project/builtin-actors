// SPDX-License-Identifier: Apache-2.0 MIT
pragma solidity >=0.4.25 <=0.8.17;

contract CallActorPrecompile {
    address constant CALL_ACTOR_ADDRESS = 0xfe00000000000000000000000000000000000003;
    address constant CALL_ACTOR_ID = 0xfe00000000000000000000000000000000000005;

    function call_actor_id(uint64 method, uint256 value, uint64 flags, uint64 codec, bytes calldata params, uint64 id) public returns (bool, int256, uint64, bytes memory) {
        (bool success, bytes memory data) = address(CALL_ACTOR_ID).delegatecall(abi.encode(method, value, flags, codec, params, id));
        (int256 exit, uint64 return_codec, bytes memory return_value) = abi.decode(data, (int256, uint64, bytes));
        return (success, exit, return_codec, return_value);
    }

    function call_actor_address(uint64 method, uint256 value, uint64 flags, uint64 codec, bytes calldata params, bytes calldata filAddress) public returns (bool, int256, uint64, bytes memory) {
        (bool success, bytes memory data) = address(CALL_ACTOR_ADDRESS).delegatecall(abi.encode(method, value, flags, codec, params, filAddress));
        (int256 exit, uint64 return_codec, bytes memory return_value) = abi.decode(data, (int256, uint64, bytes));
        return (success, exit, return_codec, return_value);
    }
}

// SPDX-License-Identifier: Apache-2.0 MIT
pragma solidity >=0.4.25 <=0.8.17;

contract Factory {
    function create(int32 value) public returns (address) {
        return address(new FactoryChild(value));
    }

    function create2(bytes32 salt, int32 value) public returns (address) {
        return address(new FactoryChild{salt: salt}(value));
    }
}

contract FactoryChild {
    int32 value;
    constructor(int32 arg) {
        value = arg;
    }
    function die() public {
        selfdestruct(payable(msg.sender));
    }
    function get_value() public view returns (int32) {
        return value;
    }
}

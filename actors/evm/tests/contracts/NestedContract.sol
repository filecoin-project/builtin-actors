// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract NestedContract {
    function writeTransientData(uint256 slot, uint256 value) external {
        assembly {
            tstore(slot, value)
        }
    }

    function readTransientData(uint256 slot) external view returns (uint256) {
        uint256 value;
        assembly {
            value := tload(slot)
        }
        return value;
    }
}

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract ReentrantContract {
    event ReentrySuccess(bool success);

    function callReentry(uint256 slot, uint256 expectedValue) external returns (bool) {
        uint256 storedValue;
        assembly {
            storedValue := tload(slot)
        }
        require(storedValue == expectedValue, "Reentrant value mismatch");

        emit ReentrySuccess(true);
        return true;
    }
}

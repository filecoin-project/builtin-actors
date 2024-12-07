// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract ReentrantContract {
    event ReentrySuccess(bool success);

    function callReentry(uint256 slot, uint256 expectedValue) external returns (bool) {
        /* 
         * TODO 
         * Actually test for reentry
         * */

        uint256 storedValue;
        assembly {
            storedValue := tload(slot)
        }
        require(storedValue == expectedValue, "Reentrant value mismatch");
        return true;
    }
}

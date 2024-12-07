// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract TransientStorageTest {

    function runTests() public returns (bool) {
	    _runTests();

    }

    function _runTests() internal {
        testBasicFunctionality();
        testLifecycleValidation();
    }

    // Test 1: Basic Functionality
    function testBasicFunctionality() public {
        uint256 slot = 1;
        uint256 value = 42;

        // Store value using TSTORE
        assembly {
            tstore(slot, value)
        }

        // Retrieve value using TLOAD
        uint256 retrievedValue;
        assembly {
            retrievedValue := tload(slot)
        }

        require(retrievedValue == value, "TLOAD did not retrieve the correct value");

        // Verify TLOAD from uninitialized location
        uint256 uninitializedSlot = 2;
        uint256 uninitializedValue;
        assembly {
            uninitializedValue := tload(uninitializedSlot)
        }

        require(uninitializedValue == 0, "Uninitialized TLOAD did not return zero");
    }

    // Test 2.1: Verify transient storage clears after transaction
    function testLifecycleValidation() public {
        uint256 slot = 3;
        uint256 value = 99;

        // Store value using TSTORE
        assembly {
            tstore(slot, value)
        }

        // Verify it exists within the same transaction
        uint256 retrievedValue;
        assembly {
            retrievedValue := tload(slot)
        }
        require(retrievedValue == value, "TLOAD did not retrieve stored value within transaction");
    }

    function testLifecycleValidationSubsequentTransaction() public {
        // Test clearing by re-calling as a new transaction
        uint256 slot = 3;
        bool cleared = isStorageCleared(slot);
        require(cleared, "Transient storage was not cleared after transaction");
    }

    function isStorageCleared(uint256 slot) public view returns (bool) {
        uint256 retrievedValue;
        assembly {
            retrievedValue := tload(slot)
        }
        return retrievedValue == 0; // True if cleared
    }

}

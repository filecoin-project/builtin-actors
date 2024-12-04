// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./NestedContract.sol";
import "./ReentrantContract.sol";

contract TransientStorageTest {
    event TestResult(bool success, string message);

    NestedContract nested;
    ReentrantContract reentrant;

    constructor(address nestedAddress, address reentrantAddress) {
	nested = NestedContract(nestedAddress);
        reentrant = ReentrantContract(reentrantAddress);
    }

    function runTests() public returns (bool) {
	    _runTests();
    }

    function _runTests() internal {
        testBasicFunctionality();
        testLifecycleValidation();

	return;

	// XXX Currently calling any external methods in the basic evm test framework causes a revert
	// This is unrelated to the transient data code being tested but a factor of the MockRuntime framework
	// It also means that we can't currently properly test nested contracts or reentrancy

	// It may be that the next two tests are not compatible with the MockRuntime framework and will need to run in Lotus

        testNestedContracts();
        testReentry();
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

        //emit TestResult(true, "Basic functionality passed");
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

        //emit TestResult(true, "Lifecycle validation passed");
    }

    function isStorageCleared(uint256 slot) public view returns (bool) {
        uint256 retrievedValue;
        assembly {
            retrievedValue := tload(slot)
        }
        return retrievedValue == 0; // True if cleared
    }

    // Test 2.2: Verify nested contract independence
    function testNestedContracts() public {
        uint256 slot = 4;
        uint256 value = 88;

        // Store in this contract's transient storage
        assembly {
            tstore(slot, value)
        }

        // Call nested contract to write its own transient storage
        nested.writeTransientData(4, 123);

        // Verify this contract's data is unchanged
        uint256 retrievedValue;
        assembly {
            retrievedValue := tload(slot)
        }
        require(retrievedValue == value, "Nested contract interfered with this contract's storage");

        // Verify nested contract's data independently
        uint256 nestedValue = nested.readTransientData(4);
        require(nestedValue == 123, "Nested contract data incorrect");

        //emit TestResult(true, "Nested contracts validation passed");
    }

    // Test 2.3: Verify transient storage during reentry
    function testReentry() public {
        uint256 slot = 5;
        uint256 value = 77;

        // Store value in transient storage
        assembly {
            tstore(slot, value)
        }

        // Call reentrant contract
        bool success = reentrant.callReentry(slot, value);

        require(success, "Reentry failed");
        //emit TestResult(true, "Reentry validation passed");
    }
}

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract TransientStorageTest {
    event TestResult(bool success, string message);

    constructor() {
        // Automatically run tests on deployment
        _runTests();
    }

    function _runTests() internal {
        testBasicFunctionality();
        testLifecycleValidation();
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

        emit TestResult(true, "Basic functionality passed");
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

        // Test clearing by re-calling a new transaction
        bool cleared = isStorageCleared(slot);
        require(cleared, "Transient storage was not cleared after transaction");

        emit TestResult(true, "Lifecycle validation passed");
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
        NestedContract nested = new NestedContract();
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

        emit TestResult(true, "Nested contracts validation passed");
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
        ReentrantContract reentrant = new ReentrantContract();
        bool success = reentrant.callReentry(slot, value);

        require(success, "Reentry failed");
        emit TestResult(true, "Reentry validation passed");
    }
}

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

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract StorageFootprint {
    // Create enough counters that they would not fit in the default HAMT bucket size of 3.
    uint32 counter1;
    uint32 counter2;
    uint32 counter3;
    uint32 counter4;

    // Create two dynamic arrays to demonstrate that their costs are independent of each other because
    // they occupy different parts of the tree, and then that their items are laid out contiguously.
    uint32[] array1;
    uint32[] array2;

    // Create two mappings to demonstrate that their costs are independent of each other because
    // they occupy different parts of the tree, and that their items are also laid out randomly.
    mapping(uint32 => uint32) mapping1;
    mapping(uint32 => uint32) mapping2;

    // Increment a single counter.
    function inc_counter1() public {
        counter1 += 1;
    }

    // Increment all counters to see if there is a cost difference compared to incrementing a single one.
    function inc_counters() public {
        counter1 += 1;
        counter2 += 1;
        counter3 += 1;
        counter4 += 1;
    }

    // Push `n` more elements to `array1`, to measure how much it costs to extend it with varying
    // number of items, depending on how many fit into a node.
    function array1_push(uint32 n) public {
        // Starting from 1 because pushing 0 doesn't increase the storage size.
        for (uint32 i = 1; i <= n; i++) {
            array1.push(i);
        }
    }

    // Push `n` more elements to `array2`, to see if the size of `array1` has any bearing on it.
    function array2_push(uint32 n) public {
        for (uint32 i = 1; i <= n; i++) {
            array2.push(i);
        }
    }

    // Set `n` consecutive keys starting from `k` to the value `v` in the first mapping.
    // Call it with varying number of items to see that for maps batch size doesn't make a difference.
    function mapping1_set(
        uint32 k,
        uint32 n,
        uint32 v
    ) public {
        for (uint32 i = k; i < k + n; i++) {
            mapping1[i] = v;
        }
    }

    // Set `n` consecutive keys starting from `k` to the value `v` in the second mapping.
    // Can be used to demonstrate that maps don't influence each others' cost.
    function mapping2_set(
        uint32 k,
        uint32 n,
        uint32 v
    ) public {
        for (uint32 i = k; i < k + n; i++) {
            mapping2[i] = v;
        }
    }

    // Sum of items in a range of `array1`.
    // Use this to see how much it costs to retrieve varying number of ranges of items from the array.
    function array1_sum(uint32 k, uint32 n) public view returns (uint32 sum) {
        for (uint32 i = k; i < k + n; i++) {
            sum += array1[i];
        }
        return sum;
    }

    // Sum the items in a range of `mapping1`.
    // Can be used to contrast with the cost of retrieving similar ranges of items from the array.
    function mapping1_sum(uint32 k, uint32 n) public view returns (uint32 sum) {
        for (uint32 i = k; i < k + n; i++) {
            sum += mapping1[i];
        }
        return sum;
    }
}

// SPDX-License-Identifier: Apache-2.0 MIT
pragma solidity >=0.4.25 <=0.8.17;

contract Recursive {
    bool a;
    bool b;

    function enter() public returns (uint32) {
        if (a) {
            return 1;
        }
        a = true;
        uint32 result = Recursive(address(this)).recurse();
        if (result != 0) {
            return result;
        }

        if (!a) {
            return 4;
        }

        if (!b) {
            return 5;
        }
        return 0;
    }

    function recurse() public returns (uint32) {
        if (!a) {
            return 2;
        }
        if (b) {
            return 3;
        }
        b = true;
        return 0;
    }
}

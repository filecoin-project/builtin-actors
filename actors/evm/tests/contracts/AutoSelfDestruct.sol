// SPDX-License-Identifier: MIT
pragma solidity >=0.4.2;

contract AutoSelfDestruct {
    constructor() {
        destroy();
    }
    function destroy() public {
        selfdestruct(payable(msg.sender));
    }
}

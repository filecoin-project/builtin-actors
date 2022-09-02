pragma solidity ^0.8.0;

contract Selfdestruct {
    function die() public {
        selfdestruct(payable(address(0xFF000000000000000000000000000000000003E9)));
    }
}
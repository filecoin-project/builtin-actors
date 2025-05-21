
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract BLSPrecompileCheck {
    address constant G1_ADD_PRECOMPILE = address(0x0B);

    /// @notice Asserts that G1 addition precompile at 0x0B correctly computes 2·P
    function testG1Add() public view {
        
        // Encode input as two G1 points
        bytes memory input = hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21";

        bytes memory EXPECTED_OUTPUT = hex"000000000000000000000000000000000a40300ce2dec9888b60690e9a41d3004fda4886854573974fab73b046d3147ba5b7a5bde85279ffede1b45b3918d82d0000000000000000000000000000000006d3d887e9f53b9ec4eb6cedf5607226754b07c01ace7834f57f3e7315faefb739e59018e22c492006190fba4a870025";
        bytes32 expectedHash = keccak256(EXPECTED_OUTPUT);

        // Call precompile
        (bool success, bytes memory output) = G1_ADD_PRECOMPILE.staticcall(input);
        require(success, "Precompile call failed");
        require(output.length == 128, "Invalid output length");
        bytes32 actualHash = keccak256(output);

        require(actualHash == expectedHash, "Unexpected output");
    }
}




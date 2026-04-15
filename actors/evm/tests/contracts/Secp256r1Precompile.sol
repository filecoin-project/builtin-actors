// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract Secp256r1PrecompileCheck {
    address constant SECP256R1_VERIFY_PRECOMPILE = address(0x0100);
    
    /// @notice Test vector ok_1: Valid signature verification
    function testOk1() public view {
        bytes memory input = hex"4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e";
        
        (bool success, bytes memory output) = SECP256R1_VERIFY_PRECOMPILE.staticcall(input);
        
        require(success, "Precompile call failed");
        require(output.length == 32, "Valid signature should return 32 bytes");
        
        bytes32 result;
        assembly {
            result := mload(add(output, 0x20))
        }
        require(result == bytes32(uint256(1)), "Valid signature should return 1");
    }
    
    /// @notice Test vector fail_wrong_msg_1: Invalid signature due to wrong message
    function testFailWrongMsg1() public view {
        bytes memory input = hex"3cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e";
        
        (bool success, bytes memory output) = SECP256R1_VERIFY_PRECOMPILE.staticcall(input);
        
        require(success, "Precompile call failed");
        require(output.length == 0, "Invalid signature should return 0 bytes");
    }
    
    /// @notice Test vector fail_short_input_1: Input too short 
    function testFailShortInput1() public view {
        bytes memory input = hex"4cee90eb86eaa050036147a12d49004b6a";
        
        (bool success, bytes memory output) = SECP256R1_VERIFY_PRECOMPILE.staticcall(input);
        
        require(success, "Precompile call failed");
        require(output.length == 0, "Invalid input should return 0 bytes");
    }
    
    /// @notice Test vector fail_long_input: Input too long
    function testFailLongInput() public view {
        bytes memory input = hex"4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e00";
        
        (bool success, bytes memory output) = SECP256R1_VERIFY_PRECOMPILE.staticcall(input);
        
        require(success, "Precompile call failed");
        require(output.length == 0, "Invalid input should return 0 bytes");
    }
    
    /// @notice Test vector fail_invalid_sig: Invalid signature 
    function testFailInvalidSig() public view {
        bytes memory input = hex"4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e";
        
        (bool success, bytes memory output) = SECP256R1_VERIFY_PRECOMPILE.staticcall(input);
        
        require(success, "Precompile call failed");
        require(output.length == 0, "Invalid signature should return 0 bytes");
    }
    
    /// @notice Test vector fail_invalid_pubkey: Invalid public key
    function testFailInvalidPubkey() public view {
        bytes memory input = hex"4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d6000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
        
        (bool success, bytes memory output) = SECP256R1_VERIFY_PRECOMPILE.staticcall(input);
        
        require(success, "Precompile call failed");
        require(output.length == 0, "Invalid public key should return 0 bytes");
    }
}

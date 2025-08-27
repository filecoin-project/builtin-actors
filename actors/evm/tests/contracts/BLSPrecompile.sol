
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract BLSPrecompileCheck {
    address constant G1_ADD_PRECOMPILE = address(0x0B);
    address constant G1_MSM_PRECOMPILE = address(0x0C); // G1 MSM is at 0x0C
    address constant G2_ADD_PRECOMPILE = address(0x0D);
    address constant G2_MSM_PRECOMPILE = address(0x0E);
    address constant MAP_FP_TO_G1_PRECOMPILE = address(0x10);
    address constant MAP_FP2_TO_G2_PRECOMPILE = address(0x11);
    address constant PAIRING_CHECK_PRECOMPILE = address(0x0F);

    /// @notice Asserts that G1 addition precompile at 0x0B correctly computes 2·P
    function testG1Add() public view {
        // Test name: bls_g1add_g1+p1
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
                                    
    /// @notice Tests G2 addition precompile at 0x0D
    function testG2Add() public view {
        // Encode input as two G2 points 
        // Format: (x1_real, x1_imaginary, y1_real, y1_imaginary, x2_real, x2_imaginary, y2_real, y2_imaginary)
        // Test name: bls_g2add_g2+p2
        bytes memory input = hex"00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451";

        bytes memory EXPECTED_OUTPUT = hex"000000000000000000000000000000000b54a8a7b08bd6827ed9a797de216b8c9057b3a9ca93e2f88e7f04f19accc42da90d883632b9ca4dc38d013f71ede4db00000000000000000000000000000000077eba4eecf0bd764dce8ed5f45040dd8f3b3427cb35230509482c14651713282946306247866dfe39a8e33016fcbe520000000000000000000000000000000014e60a76a29ef85cbd69f251b9f29147b67cfe3ed2823d3f9776b3a0efd2731941d47436dc6d2b58d9e65f8438bad073000000000000000000000000000000001586c3c910d95754fef7a732df78e279c3d37431c6a2b77e67a00c7c130a8fcd4d19f159cbeb997a178108fffffcbd20";
        bytes32 expectedHash = keccak256(EXPECTED_OUTPUT);

        // Call precompile with try/catch to get more error information
        (bool success, bytes memory output) = G2_ADD_PRECOMPILE.staticcall(input);
        
        require(success, "Precompile call failed");
        require(output.length == 256, "Invalid G2 output length");
        bytes32 actualHash = keccak256(output);

        require(actualHash == expectedHash, "Unexpected G2 addition output");
    }
    
    /// @notice Tests G1 multi-scalar multiplication precompile at 0x0C
    function testG1MSM() public view {
        // Format for G1 MSM input (per EIP-2537):
        // - Concatenation of (scalar, point) pairs:
        //   - 32 bytes scalar
        //   - 64 bytes G1 point (x, y)
        // The number of pairs is inferred from the input length.
        // Test name: bls_g1msm_(g1+g1=2*g1)
        bytes memory input = hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000002";
        
        bytes memory EXPECTED_OUTPUT = hex"000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d28";
        bytes32 expectedHash = keccak256(abi.encodePacked(EXPECTED_OUTPUT));
        
        // Call precompile
        (bool success, bytes memory output) = G1_MSM_PRECOMPILE.staticcall(input);
        
        require(success, "G1 MSM precompile call failed");
        require(output.length == 128, "Invalid G1 MSM output length"); // G1 point is 128 bytes
        
        bytes32 actualHash = keccak256(abi.encodePacked(output));
        require(actualHash == expectedHash, "Unexpected G1 MSM output");
    }
    
    /// @notice Tests G2 multi-scalar multiplication precompile at 0x0E
    function testG2MSM() public view {
        // Format for G2 MSM input (per EIP-2537):
        // - Input is a concatenation of (scalar, point) pairs.
        // - Each pair consists of:
        //   - 32 bytes scalar
        //   - 128 bytes G2 point (x.a, x.b, y.a, y.b)
        // - The number of pairs is inferred from the input length.
        // Test name: bls_g2msm_(g2+g2=2*g2)
        bytes memory input = hex"00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000000000000000000000000000000000002";
        
        // Expected output for this specific input
        bytes memory EXPECTED_OUTPUT = hex"000000000000000000000000000000001638533957d540a9d2370f17cc7ed5863bc0b995b8825e0ee1ea1e1e4d00dbae81f14b0bf3611b78c952aacab827a053000000000000000000000000000000000a4edef9c1ed7f729f520e47730a124fd70662a904ba1074728114d1031e1572c6c886f6b57ec72a6178288c47c33577000000000000000000000000000000000468fb440d82b0630aeb8dca2b5256789a66da69bf91009cbfe6bd221e47aa8ae88dece9764bf3bd999d95d71e4c9899000000000000000000000000000000000f6d4552fa65dd2638b361543f887136a43253d9c66c411697003f7a13c308f5422e1aa0a59c8967acdefd8b6e36ccf3";
        bytes32 expectedHash = keccak256(EXPECTED_OUTPUT);
        
        // Call precompile
        (bool success, bytes memory output) = G2_MSM_PRECOMPILE.staticcall(input);
        
        require(success, "G2 MSM precompile call failed");
        require(output.length == 256, "Invalid G2 MSM output length"); // G2 point is 256 bytes
        
        bytes32 actualHash = keccak256(abi.encodePacked(output));
        require(actualHash == expectedHash, "Unexpected G2 MSM output");
    }
    /// @notice Tests mapping a field element to a G1 point
        function testMapFpToG1() public view {
            // Test name: bls_g1map_
            // Input is a single field element (32 bytes)
            bytes memory input = hex"00000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f03";
            
            // Expected G1 point output (x,y) = 128 bytes
            bytes memory EXPECTED_OUTPUT = hex"00000000000000000000000000000000184bb665c37ff561a89ec2122dd343f20e0f4cbcaec84e3c3052ea81d1834e192c426074b02ed3dca4e7676ce4ce48ba0000000000000000000000000000000004407b8d35af4dacc809927071fc0405218f1401a6d15af775810e4e460064bcc9468beeba82fdc751be70476c888bf3";
            bytes32 expectedHash = keccak256(EXPECTED_OUTPUT);
            
            // Call precompile
            (bool success, bytes memory output) = MAP_FP_TO_G1_PRECOMPILE.staticcall(input);

            require(success, "Map Fp to G1 precompile call failed");

            bytes32 actualHash = keccak256(output);
            require(actualHash == expectedHash, "Unexpected Map Fp to G1 output");
        }

        /// @notice Tests mapping a field element in Fp2 to a G2 point
        function testMapFp2ToG2() public view {
            // Input is a single Fp2 element (64 bytes: a + b*i where a,b are each 32 bytes)
            // Test name: bls_g2map_
            bytes memory input = hex"0000000000000000000000000000000007355d25caf6e7f2f0cb2812ca0e513bd026ed09dda65b177500fa31714e09ea0ded3a078b526bed3307f804d4b93b040000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b78c";
            
            // Expected G2 point output (x.a, x.b, y.a, y.b) = 256 bytes
            bytes memory EXPECTED_OUTPUT = hex"0000000000000000000000000000000000e7f4568a82b4b7dc1f14c6aaa055edf51502319c723c4dc2688c7fe5944c213f510328082396515734b6612c4e7bb700000000000000000000000000000000126b855e9e69b1f691f816e48ac6977664d24d99f8724868a184186469ddfd4617367e94527d4b74fc86413483afb35b000000000000000000000000000000000caead0fd7b6176c01436833c79d305c78be307da5f6af6c133c47311def6ff1e0babf57a0fb5539fce7ee12407b0a42000000000000000000000000000000001498aadcf7ae2b345243e281ae076df6de84455d766ab6fcdaad71fab60abb2e8b980a440043cd305db09d283c895e3d";
            bytes32 expectedHash = keccak256(EXPECTED_OUTPUT);
            
            // Call precompile
            (bool success, bytes memory output) = MAP_FP2_TO_G2_PRECOMPILE.staticcall(input);

            require(success, "Map Fp2 to G2 precompile call failed");

            bytes32 actualHash = keccak256(output);
            require(actualHash == expectedHash, "Unexpected Map Fp2 to G2 output");
        }

        /// @notice Tests BLS pairing check operation
        function testPairing() public view {
            // Input consists of a sequence of points (G1, G2) to check e(P1, Q1) * e(P2, Q2) * ... * e(Pn, Qn) = 1
            // Each pair requires 384 bytes: 128 bytes for G1 point + 256 bytes for G2 point
            
            // Example: Single pair that should give a result of 1 (valid pairing)
            // Test name: bls_pairing_e(G1,0)!=e(-G1,G2)
            bytes memory input = hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed";
            
            bytes memory EXPECTED_OUTPUT = hex"0000000000000000000000000000000000000000000000000000000000000000";
            bytes32 expectedHash = keccak256(EXPECTED_OUTPUT);
            
            // Call precompile
            (bool success, bytes memory output) = PAIRING_CHECK_PRECOMPILE.staticcall(input);

            require(success, "Pairing Check precompile call failed");

            bytes32 actualHash = keccak256(output);
            require(actualHash == expectedHash, "Unexpected Pairing Check output");

        }


    /// @notice Tests that G1 addition precompile returns an error for invalid inputs
    function testG1AddFailure() public view {
        // Input with invalid point (not on curve): bls_g1add_point_not_on_curve
        bytes memory invalidInput = hex"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb00000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a2100000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21";
        
        // Call precompile, expecting failure
        (bool success,) = G1_ADD_PRECOMPILE.staticcall(invalidInput);
        
        // The precompile should fail for invalid points
        require(!success, "Precompile should fail for invalid input");
    }    
    
    /// @notice Tests that G2 addition precompile returns an error for invalid inputs
    function testG2AddFailure() public view {
        // Input with invalid G2 point (not on curve): bls_g2add_violate_top_bytes
        bytes memory invalidInput = hex"10000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"; // Last byte changed to make point invalid

        // Call precompile, expecting failure
        (bool success,) = G2_ADD_PRECOMPILE.staticcall(invalidInput);
        
        // The precompile should fail for invalid points
        require(!success, "G2 add precompile should fail for invalid input");
    }

    /// @notice Tests that G1 MSM precompile returns an error for invalid inputs
    function testG1MSMFailure() public view {
        // Invalid input: zero number of points but with point data: bls_g1msm_violate_top_bytes
        bytes memory invalidInput = hex"1000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca942600000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a210000000000000000000000000000000000000000000000000000000000000002";
        
        // Call precompile, expecting failure
        (bool success,) = G1_MSM_PRECOMPILE.staticcall(invalidInput);
        
        // The precompile should fail for invalid inputs
        require(!success, "G1 MSM precompile should fail for invalid input");
    }

    /// @notice Tests that G2 MSM precompile returns an error for invalid inputs
    function testG2MSMFailure() public view {
        // Invalid input: zero number of points but with point data: bls_g2msm_violate_top_bytes
        bytes memory invalidInput = hex"10000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d8784510000000000000000000000000000000000000000000000000000000000000002";
        
        // Call precompile, expecting failure
        (bool success,) = G2_MSM_PRECOMPILE.staticcall(invalidInput);
        
        // The precompile should fail for invalid inputs
        require(!success, "G2 MSM precompile should fail for invalid input");
    }

    /// @notice Tests that Map Fp to G1 precompile returns an error for invalid inputs
    function testMapFpToG1Failure() public view {
        // Input with invalid length (too short): bls_mapg1_top_bytes
        bytes memory invalidInput = hex"1000000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f"; // Only 15 bytes instead of 32
        
        // Call precompile, expecting failure
        (bool success,) = MAP_FP_TO_G1_PRECOMPILE.staticcall(invalidInput);
        
        // The precompile should fail for invalid inputs
        require(!success, "Map Fp to G1 precompile should fail for invalid input length");
    }

    /// @notice Tests that Map Fp2 to G2 precompile returns an error for invalid inputs
    function testMapFp2ToG2Failure() public view {
        // Input with invalid length (too short): bls_mapg2_top_bytes
        bytes memory invalidInput = hex"000000000000000000000000000000000007355d25caf6e7f2f0cb2812ca0e513bd026ed09dda65b177500fa31714e09ea0ded3a078b526bed3307f804d4b93b040000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b7"; // Less than 64 bytes
        
        // Call precompile, expecting failure
        (bool success,) = MAP_FP2_TO_G2_PRECOMPILE.staticcall(invalidInput);
        
        // The precompile should fail for invalid inputs
        require(!success, "Map Fp2 to G2 precompile should fail for invalid input length");
    }

    /// @notice Tests that pairing check precompile returns an error for invalid inputs
    function testPairingFailure() public view {
        // Input with invalid length (not a multiple of 384): bls_pairing_e(G1_not_in_correct_subgroup,-G2)=e(-G1,G2)
        bytes memory invalidInput = hex"000000000000000000000000000000000123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef00000000000000000000000000000000193fb7cedb32b2c3adc06ec11a96bc0d661869316f5e4a577a9f7c179593987beb4fb2ee424dbb2f5dd891e228b46c4a00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed"; // Only 128 bytes (G1 point)
        
        // Call precompile, expecting failure
        (bool success,) = PAIRING_CHECK_PRECOMPILE.staticcall(invalidInput);
        
        // The precompile should fail for invalid inputs
        require(!success, "Pairing check precompile should fail for invalid input");
    }
}

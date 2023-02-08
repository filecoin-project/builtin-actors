// SPDX-License-Identifier: Apache-2.0 MIT
pragma solidity >=0.4.25 <=0.8.17;

contract FilecoinFallback {
    function handle_filecoin_method(uint64 method, uint64 codec, bytes calldata params) pure public returns (uint32, uint64, bytes memory) {
        require((codec == 0) == (params.length == 0));
        if (method == 1024) {
            return ( 0, 0, bytes("") );
        } else if (method == 1025) {
            return ( 0, 0x51, bytes("foobar") );
        } else if (method == 1026) {
            return ( 42, 0, bytes("") );
        } else if (method == 1027) {
            return ( 42, 0x51, bytes("foobar") );
        }
        revert();
    }
}

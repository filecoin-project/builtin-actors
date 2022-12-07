// SPDX-License-Identifier: Apache-2.0 MIT
pragma solidity >=0.4.25 <=0.8.17;

contract FilecoinFallback {
    function handle_filecoin_method(uint64 method, uint64 codec, bytes calldata params) pure public returns (uint64) {
        require((codec == 0) == (params.length == 0));
        return method;
    }
}
